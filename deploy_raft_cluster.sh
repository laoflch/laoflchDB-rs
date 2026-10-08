#!/bin/bash
# =============================================================================
# laoflchdb Raft 集群部署脚本（多 Raft 组）
#
# 功能：
#   - 启动多个 raft 容器（默认 3 个），每个容器同时运行 GROUP_COUNT 个独立
#     Raft 组，所有组共用一个 gRPC 端口
#   - 每个节点将容器内 gRPC 端口 (29999) 映射到宿主机不同端口：
#       node-1 -> 127.0.0.1:29999
#       node-2 -> 127.0.0.1:30000
#       node-3 -> 127.0.0.1:30001
#   - 每个组在"对应节点"上 bootstrap（组 1 在 node-1、组 2 在 node-2、...），
#     初始化时写入该组全部节点成员，使初始 leader 分布在不同节点
#
# 用法：
#   ./deploy_raft_cluster.sh up      启动集群（构建镜像 -> 起容器 -> 验证）
#   ./deploy_raft_cluster.sh down    停止并删除容器（保留数据卷）
#   ./deploy_raft_cluster.sh clean   停止容器并删除数据卷、配置目录
#   ./deploy_raft_cluster.sh status  查看容器状态与各节点各组的 Raft 状态
#   ./deploy_raft_cluster.sh build   仅构建镜像
#
# 可配置环境变量：NODE_COUNT / GROUP_COUNT / BASE_PORT / NETWORK / IMAGE / CONFIG_DIR
# =============================================================================

set -euo pipefail

# ── 可配置项 ──
NETWORK="${NETWORK:-laoflchdb-net}"
IMAGE="${IMAGE:-laoflchdb-raft:0.1.13}"
NODE_COUNT="${NODE_COUNT:-3}"          # 集群节点数
GROUP_COUNT="${GROUP_COUNT:-$NODE_COUNT}" # Raft 组数（每组 leader 初始分布在不同节点，须 <= NODE_COUNT）
BASE_PORT="${BASE_PORT:-29999}"        # node-1 宿主端口，后续节点依次 +1
CONFIG_DIR="${CONFIG_DIR:-$(pwd)/deploy_raft}"
CONTAINER_PREFIX="raft-node"
DATA_VOLUME_PREFIX="raft_data"
CONTAINER_PORT=29999                   # 容器内 Raft gRPC 端口
ADMIN_PY="$(pwd)/scripts/raft_admin.py"

# 依赖检查
if ! command -v docker >/dev/null 2>&1; then
    echo "错误：未找到 docker 命令" >&2
    exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
    echo "错误：未找到 python3" >&2
    exit 1
fi

node_name() { printf "%s-%d" "$CONTAINER_PREFIX" "$1"; }
host_port() { printf "%d" $((BASE_PORT + $1 - 1)); }
peer_addr() { printf "%s-%d:%d" "$CONTAINER_PREFIX" "$1" "$CONTAINER_PORT"; }

# ── 生成单个节点的 prod_raft.yaml（多 Raft 组）──
# node-${id} 上运行 GROUP_COUNT 个组，每组 node_id=${id}（节点编号即组内 ID），
# 组 g 仅当 g==${id} 时 bootstrap=true（该组在对应节点自举，初始 leader 落在此节点）。
gen_config() {
    local id="$1"
    local dir="$CONFIG_DIR/$(node_name "$id")"
    mkdir -p "$dir"

    cat > "$dir/prod_raft.yaml" <<EOF
# 由 deploy_raft_cluster.sh 生成的 Raft 节点配置（node-${id}，${GROUP_COUNT} 个组）
db_path: /app/data
index_path: /app/indexes
model_path: /app/model

vector_service:
  enabled: false
  auto_load: false

raft_service:
  enabled: true
  address: "0.0.0.0:${CONTAINER_PORT}"
  data_path: /app/laoflch_raft_data
  election_timeout: 1000
  heartbeat_interval: 100
  groups:
EOF
    local g j
    for ((g = 1; g <= GROUP_COUNT; g++)); do
        local bs="false"
        [ "$g" -eq "$id" ] && bs="true"
        cat >> "$dir/prod_raft.yaml" <<EOF
    - group_id: ${g}
      name: "group-${g}"
      node_id: ${id}
      bootstrap: ${bs}
      peers:
EOF
        for ((j = 1; j <= NODE_COUNT; j++)); do
            cat >> "$dir/prod_raft.yaml" <<EOF
        - { id: ${j}, addr: "$(peer_addr "$j")" }
EOF
        done
    done

    cat >> "$dir/prod_raft.yaml" <<'EOF'

embedding_index:
  enabled: false

object_store:
  enabled: false

image_service:
  enabled: false

face_service:
  enabled: false

text_summarize:
  enabled: false

reranker:
  enabled: false

log_level: info
default_policy: allow
runtime_mode: multi_thread

access_protocols:
  - protocol: grpc
    enabled: false
    addr: 0.0.0.0:19777
    service_id: grpc_admin
EOF
    echo "已生成配置: $dir/prod_raft.yaml"
}

ensure_network() {
    if docker network inspect "$NETWORK" >/dev/null 2>&1; then
        echo "使用已有网络: $NETWORK"
    else
        echo "创建 Docker 网络: $NETWORK"
        docker network create "$NETWORK"
    fi
}

build_image() {
    echo "构建镜像 $IMAGE ..."
    if [ ! -f target/release/laoflchdb ]; then
        echo "错误：未找到 target/release/laoflchdb，请先执行 cargo build --release" >&2
        exit 1
    fi
    # 二进制链接了 CUDA 库，将宿主机 CUDA 运行库复制到构建上下文（Dockerfile COPY deploy_raft/.cuda_libs/）
    local libs_dir="$CONFIG_DIR/.cuda_libs"
    mkdir -p "$libs_dir"
    cp -L /usr/lib/x86_64-linux-gnu/libcuda.so.1 "$libs_dir/libcuda.so.1"
    cp -L /usr/local/cuda-12.4/targets/x86_64-linux/lib/libcudart.so.12 "$libs_dir/libcudart.so.12"
    cp -L /usr/local/cuda-12.4/targets/x86_64-linux/lib/libcublas.so.12 "$libs_dir/libcublas.so.12"
    cp -L /usr/local/cuda-12.4/targets/x86_64-linux/lib/libcublasLt.so.12 "$libs_dir/libcublasLt.so.12"
    cp -L /usr/local/cuda-11.8/targets/x86_64-linux/lib/libcurand.so.10 "$libs_dir/libcurand.so.10"
    docker build -t "$IMAGE" -f Dockerfile.raft.prod .
    rm -rf "$libs_dir"
}

ensure_image() {
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        build_image
    else
        echo "使用已有镜像: $IMAGE"
    fi
}

check_ports() {
    for ((i = 1; i <= NODE_COUNT; i++)); do
        local hp
        hp="$(host_port "$i")"
        if (echo > "/dev/tcp/127.0.0.1/$hp") 2>/dev/null; then
            echo "警告：宿主端口 $hp 已被占用，node-$i 可能启动失败" >&2
        fi
    done
}

start_containers() {
    echo "启动 ${NODE_COUNT} 个 Raft 容器..."
    for ((i = 1; i <= NODE_COUNT; i++)); do
        local name hp
        name="$(node_name "$i")"
        hp="$(host_port "$i")"
        docker rm -f "$name" >/dev/null 2>&1 || true
        echo "  [node-$i] $name  (宿主端口 $hp -> $CONTAINER_PORT)"
        docker run -d --name "$name" \
            --network "$NETWORK" \
            --security-opt seccomp=unconfined \
            -p "$hp:$CONTAINER_PORT" \
            -v "${DATA_VOLUME_PREFIX}_${i}:/app/laoflch_raft_data" \
            -v "$CONFIG_DIR/$(node_name "$i")/prod_raft.yaml:/app/config/prod_raft.yaml" \
            "$IMAGE" >/dev/null
    done
}

wait_ready() {
    echo "等待所有节点各组的 gRPC 就绪（超时 120s）..."
    local deadline=$(( $(date +%s) + 120 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local ready=1
        for ((i = 1; i <= NODE_COUNT; i++)); do
            for ((g = 1; g <= GROUP_COUNT; g++)); do
                if ! python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$g" get-status >/dev/null 2>&1; then
                    ready=0
                    break
                fi
            done
        done
        if [ "$ready" -eq 1 ]; then
            echo "所有节点已就绪"
            return 0
        fi
        sleep 3
    done
    echo "错误：节点未在 120 秒内就绪，请检查: docker logs raft-node-1" >&2
    docker ps --filter "name=$CONTAINER_PREFIX" --format "table {{.Names}}\t{{.Status}}"
    exit 1
}

verify() {
    echo ""
    echo "==================== 集群状态 ===================="
    echo ""
    for ((g = 1; g <= GROUP_COUNT; g++)); do
        echo "===== 组 $g 成员 (via node-$g 宿主端口 $(host_port "$g")) ====="
        python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$g")" --group "$g" list-peers
    done
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo ""
        echo "===== node-$i 各组状态 (宿主端口 $(host_port "$i")) ====="
        for ((g = 1; g <= GROUP_COUNT; g++)); do
            echo "----- group-$g -----"
            python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$g" get-status
        done
    done
    echo ""
    echo "==================== 访问入口 ===================="
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo "  node-$i: 127.0.0.1:$(host_port "$i")"
    done
    echo ""
    echo "管理接口示例（--group 指定 Raft 组）："
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 get-status"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 2 list-peers"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 put <table> <key> <value>"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 get <table> <key>"
}

up() {
    if [ "$GROUP_COUNT" -gt "$NODE_COUNT" ]; then
        echo "错误：GROUP_COUNT($GROUP_COUNT) 不能大于 NODE_COUNT($NODE_COUNT)" >&2
        exit 1
    fi
    check_ports
    ensure_network
    ensure_image
    for ((i = 1; i <= NODE_COUNT; i++)); do
        gen_config "$i"
    done
    start_containers
    wait_ready
    verify
}

down() {
    echo "停止并删除 Raft 容器（保留数据卷）..."
    for ((i = 1; i <= NODE_COUNT; i++)); do
        docker rm -f "$(node_name "$i")" >/dev/null 2>&1 || true
    done
    echo "完成"
}

clean() {
    down
    echo "删除数据卷与配置目录..."
    for ((i = 1; i <= NODE_COUNT; i++)); do
        docker volume rm "${DATA_VOLUME_PREFIX}_${i}" >/dev/null 2>&1 || true
    done
    rm -rf "$CONFIG_DIR"
    echo "完成"
}

status() {
    echo "===== 容器状态 ====="
    docker ps --filter "name=$CONTAINER_PREFIX" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
    echo ""
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo "===== node-$i 各组状态 (宿主端口 $(host_port "$i")) ====="
        for ((g = 1; g <= GROUP_COUNT; g++)); do
            echo "----- group-$g -----"
            if python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$g" get-status 2>/dev/null; then
                :
            else
                echo "(节点未就绪)"
            fi
        done
        echo ""
    done
}

usage() {
    echo "用法: $0 <command>"
    echo ""
    echo "命令:"
    echo "  up       启动 Raft 集群（构建镜像 -> 起容器 -> 验证）"
    echo "  down     停止并删除容器（保留数据卷）"
    echo "  clean    停止容器并删除数据卷、配置目录"
    echo "  status   查看容器与节点各组 Raft 状态"
    echo "  build    仅构建镜像"
    echo ""
    echo "环境变量: NODE_COUNT(默认3) GROUP_COUNT(默认=NODE_COUNT) BASE_PORT(默认29999) NETWORK(默认laoflchdb-net) IMAGE CONFIG_DIR"
}

cmd="${1:-up}"
case "$cmd" in
    up)          up ;;
    down)        down ;;
    clean)       clean ;;
    status)      status ;;
    build)       build_image ;;
    help|-h|--help) usage ;;
    *)           usage ;;
esac
