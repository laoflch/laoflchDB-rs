#!/bin/bash
# =============================================================================
# laoflchdb Raft 集群部署脚本（多 Raft 组 · 滑动窗口分组）
#
# 功能：
#   - 启动 NODE_COUNT 个 raft 容器，每个容器同时运行多个独立的 Raft 组，
#     所有组共用一个 gRPC 端口
#   - 每个节点将容器内 gRPC 端口 (29999) 映射到宿主机不同端口：
#       node-1 -> 127.0.0.1:29999
#       node-2 -> 127.0.0.1:30000
#       ...
#   - 分组方式：滑动窗口。组 g 的成员 = {g, g+1, ..., g+GROUP_SIZE-1}（首尾相连），
#     组数 = NODE_COUNT，每组成员错开。例如 NODE_COUNT=5、GROUP_SIZE=3：
#       组 1 = {1,2,3}   组 2 = {2,3,4}   组 3 = {3,4,5}   组 4 = {4,5,1}   组 5 = {5,1,2}
#   - 每个组在 node-g 上 bootstrap（node-g 必为该组成员），初始化时写入该组全部成员，
#     使初始 leader 分布在不同节点
#
# 用法：
#   ./deploy_raft_cluster.sh up      启动集群（构建镜像 -> 起容器 -> 验证）
#   ./deploy_raft_cluster.sh down    停止并删除容器（保留数据卷）
#   ./deploy_raft_cluster.sh clean   停止容器并删除数据卷、配置目录
#   ./deploy_raft_cluster.sh status  查看容器状态与各节点各组的 Raft 状态
#   ./deploy_raft_cluster.sh build   仅构建镜像
#
# 可配置环境变量：NODE_COUNT / GROUP_SIZE / BASE_PORT / REST_BASE_PORT / NETWORK / IMAGE / CONFIG_DIR
# =============================================================================

set -euo pipefail

# ── 可配置项 ──
NETWORK="${NETWORK:-laoflchdb-net}"
IMAGE="${IMAGE:-laoflchdb-raft:0.1.13}"
NODE_COUNT="${NODE_COUNT:-5}"          # 集群节点数（同时也是组数，滑动窗口一组一个）
GROUP_SIZE="${GROUP_SIZE:-3}"          # 每组节点数（滑动窗口宽度，须 <= NODE_COUNT）
BASE_PORT="${BASE_PORT:-29999}"        # node-1 宿主端口，后续节点依次 +1
REST_BASE_PORT="${REST_BASE_PORT:-18080}"   # node-1 REST 宿主端口，后续节点依次 +1
CONFIG_DIR="${CONFIG_DIR:-$(pwd)/deploy_raft}"
CONTAINER_PREFIX="raft-node"
DATA_VOLUME_PREFIX="raft_data"
CONTAINER_PORT=29999                   # 容器内 Raft gRPC 端口
CONTAINER_REST_PORT=18080              # 容器内 REST 端口（与参考配置 access_protocols rest 项一致）
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
host_rest_port() { printf "%d" $((REST_BASE_PORT + $1 - 1)); }
peer_addr() { printf "%s-%d:%d" "$CONTAINER_PREFIX" "$1" "$CONTAINER_PORT"; }

# ── 滑动窗口分组辅助 ──
# 组 g 的成员 = {g, g+1, ..., g+GROUP_SIZE-1}（环状，首尾相连）

# 组 $1 的第 $2 个成员（$2=0..GROUP_SIZE-1）
group_member() {
    printf "%d" $(( (($1 - 1 + $2) % NODE_COUNT) + 1 ))
}

# node $1 是否属于组 $2
node_in_group() {
    local n="$1" g="$2" k m
    for ((k = 0; k < GROUP_SIZE; k++)); do
        m=$(( (($g - 1 + $k) % NODE_COUNT) + 1 ))
        [ "$m" -eq "$n" ] && return 0
    done
    return 1
}

# ── 生成单个节点的 prod_raft.yaml ──
# 以仓库 config/prod_raft.yaml 为基底（存储引擎 / 双写 / 各服务开关 / 日志等原样继承，
# 能不改的不改），仅替换 raft_service.groups 段为该节点参与的组（组 g 在 node-g 自举）。
gen_config() {
    local id="$1"
    local dir="$CONFIG_DIR/$(node_name "$id")"
    mkdir -p "$dir"
    local total=0 g
    for ((g = 1; g <= NODE_COUNT; g++)); do
        if node_in_group "$id" "$g"; then total=$((total + 1)); fi
    done

    local ref="$(dirname "$0")/config/prod_raft.yaml"
    if [ ! -f "$ref" ]; then
        echo "错误：未找到参考配置 $ref" >&2
        exit 1
    fi

    # 剥离参考配置的 raft_service.groups 段（保留空的 groups: 行，组项由下面按节点生成后插入），
    # 其余字段原样继承。注意：groups: 与第一个组项之间不能有注释行（serde_yaml 会报错）。
    awk '
        /^  groups:/ { print; skip=1; next }
        skip && /^[a-zA-Z#]/ { skip=0 }
        skip { next }
        { print }
    ' "$ref" > "$dir/prod_raft.yaml"

    # 可选环境变量覆盖（未设置时完全沿用参考配置）：
    #   STORAGE_TYPE=memory / rocksdb   覆盖 storage.storage_type
    #   DUAL_WRITE_ENABLED=0/1          覆盖 dual_write.enabled
    if [ -n "${STORAGE_TYPE:-}" ]; then
        sed -i "0,/storage_type: rocksdb/s/storage_type: rocksdb/storage_type: ${STORAGE_TYPE}/" "$dir/prod_raft.yaml"
    fi
    if [ -n "${DUAL_WRITE_ENABLED:-}" ]; then
        sed -i "0,/dual_write:/,/groups:/s/enabled: true/enabled: ${DUAL_WRITE_ENABLED}/" "$dir/prod_raft.yaml"
    fi

    # 生成该节点参与的组项（node-${id} 参与 ${total} 个组；组 g 在 node-g 自举）
    local tmp="$dir/.groups_items.tmp"
    : > "$tmp"
    local k bs m
    for ((g = 1; g <= NODE_COUNT; g++)); do
        if ! node_in_group "$id" "$g"; then
            continue
        fi
        bs="false"
        [ "$g" -eq "$id" ] && bs="true"
        cat >> "$tmp" <<EOF
    - group_id: ${g}
      name: "group-${g}"
      node_id: ${id}
      self_addr: "$(peer_addr "$id")"
      bootstrap: ${bs}
      peers:
EOF
        for ((k = 0; k < GROUP_SIZE; k++)); do
            m="$(group_member "$g" "$k")"
            echo "        - { id: ${m}, addr: \"$(peer_addr "$m")\" }" >> "$tmp"
        done
    done
    sed -i "/^  groups:$/r $tmp" "$dir/prod_raft.yaml"
    rm -f "$tmp"

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
    if [ ! -f target/release/ldb ]; then
        echo "错误：未找到 target/release/ldb（RocksDB ldb 工具），请先执行: source local.env && cargo-xtask ldb" >&2
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

    # ldb 工具链接的 RocksDB 运行时动态库（Dockerfile COPY deploy_raft/.runtime_libs/ 到容器系统库路径）
    local rl_dir="$CONFIG_DIR/.runtime_libs"
    mkdir -p "$rl_dir"
    cp -L /usr/lib/x86_64-linux-gnu/libsnappy.so.1 "$rl_dir/libsnappy.so.1"
    cp -L /usr/lib/x86_64-linux-gnu/libgflags.so.2.2 "$rl_dir/libgflags.so.2.2"
    cp -L /usr/lib/x86_64-linux-gnu/libbz2.so.1.0 "$rl_dir/libbz2.so.1.0"
    cp -L /usr/lib/x86_64-linux-gnu/liblz4.so.1 "$rl_dir/liblz4.so.1"
    cp -L /usr/lib/x86_64-linux-gnu/libzstd.so.1 "$rl_dir/libzstd.so.1"
    cp -L /usr/lib/x86_64-linux-gnu/libz.so.1 "$rl_dir/libz.so.1"
    cp -L /usr/local/lib/libjemalloc.so.2 "$rl_dir/libjemalloc.so.2"

    docker build -t "$IMAGE" -f Dockerfile.raft.prod .
    rm -rf "$libs_dir" "$rl_dir"
}

ensure_image() {
    if [ ! -f target/release/laoflchdb ]; then
        echo "错误：未找到 target/release/laoflchdb，请先执行 cargo build --release" >&2
        exit 1
    fi
    if [ ! -f target/release/ldb ]; then
        echo "错误：未找到 target/release/ldb，请先执行: source local.env && cargo-xtask ldb" >&2
        exit 1
    fi
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        build_image
        return
    fi
    # 二进制或 ldb 比镜像新 → 重建，避免复用含旧二进制的镜像
    local bin_mtime ldb_mtime img_epoch
    bin_mtime=$(stat -c %Y target/release/laoflchdb)
    ldb_mtime=$(stat -c %Y target/release/ldb)
    img_epoch=$(date -d "$(docker image inspect "$IMAGE" --format '{{.Created}}')" +%s 2>/dev/null || echo 0)
    if [ "$bin_mtime" -gt "$img_epoch" ] || [ "$ldb_mtime" -gt "$img_epoch" ]; then
        echo "检测到二进制/ldb 比镜像新，重新构建镜像 $IMAGE ..."
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
        echo "  [node-$i] $name  (gRPC 宿主 $hp -> $CONTAINER_PORT, REST 宿主 $(host_rest_port "$i") -> $CONTAINER_REST_PORT)"
        docker run -d --name "$name" \
            --network "$NETWORK" \
            --security-opt seccomp=unconfined \
            -p "$hp:$CONTAINER_PORT" \
            -p "$(host_rest_port "$i"):$CONTAINER_REST_PORT" \
            -v "${DATA_VOLUME_PREFIX}_${i}:/app/laoflch_raft_data" \
            -v "$CONFIG_DIR/$(node_name "$i")/prod_raft.yaml:/app/config/prod_raft.yaml" \
            "$IMAGE" >/dev/null
    done
}

wait_ready() {
    echo "等待所有节点 gRPC 就绪（超时 120s）..."
    # 所有组共用一个 gRPC 端口，节点只要任一所属组（组 i 必然属于 node-i）可访问即就绪
    local deadline=$(( $(date +%s) + 120 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local ready=1
        for ((i = 1; i <= NODE_COUNT; i++)); do
            if ! python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$i" status >/dev/null 2>&1; then
                ready=0
                break
            fi
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
    # 组 g 的 leader 预期在 node-g（bootstrap 节点），via node-g 查看各组
    for ((g = 1; g <= NODE_COUNT; g++)); do
        echo "===== 组 $g 成员 (via node-$g 宿主端口 $(host_port "$g")) ====="
        python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$g")" --group "$g" list-peers
    done
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo ""
        echo "===== node-$i 所属组状态 (宿主端口 $(host_port "$i")) ====="
        for ((g = 1; g <= NODE_COUNT; g++)); do
            node_in_group "$i" "$g" || continue
            echo "----- group-$g -----"
            python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$g" status
        done
    done
    echo ""
    echo "==================== 访问入口 ===================="
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo "  node-$i: 127.0.0.1:$(host_port "$i")"
    done
    echo ""
    echo "管理接口示例（--group 指定 Raft 组）："
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 status"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 2 list-peers"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 put <table> <key> <value>"
    echo "  python3 scripts/raft_admin.py --addr 127.0.0.1:$(host_port 1) --group 1 get <table> <key>"
}

up() {
    if [ "$GROUP_SIZE" -gt "$NODE_COUNT" ]; then
        echo "错误：GROUP_SIZE($GROUP_SIZE) 不能大于 NODE_COUNT($NODE_COUNT)" >&2
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
        echo "===== node-$i 所属组状态 (宿主端口 $(host_port "$i")) ====="
        for ((g = 1; g <= NODE_COUNT; g++)); do
            node_in_group "$i" "$g" || continue
            echo "----- group-$g -----"
            if python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" --group "$g" status 2>/dev/null; then
                :
            else
                echo "(节点未就绪)"
            fi
        done
        echo ""
    done
}

check_data() {
    local group_arg=()
    if [ -n "${CHECK_GROUP:-}" ]; then
        group_arg=(--group "$CHECK_GROUP")
    fi
    python3 "$ADMIN_PY" "${group_arg[@]}" --nodes "$NODE_COUNT" check-data
}

usage() {
    echo "用法: $0 <command>"
    echo ""
    echo "命令:"
    echo "  up         启动 Raft 集群（构建镜像 -> 起容器 -> 验证）"
    echo "  down       停止并删除容器（保留数据卷）"
    echo "  clean      停止容器并删除数据卷、配置目录"
    echo "  status     查看容器与节点各组 Raft 状态"
    echo "  build      仅构建镜像"
    echo "  check-data 校验各组各节点业务数据落盘是否一致（CHECK_GROUP=<组> 可只查某组）"
    echo ""
    echo "环境变量: NODE_COUNT(默认5) GROUP_SIZE(默认3) BASE_PORT(默认29999) REST_BASE_PORT(默认18080) NETWORK(默认laoflchdb-net) IMAGE CONFIG_DIR CHECK_GROUP"
}

cmd="${1:-up}"
case "$cmd" in
    up)          up ;;
    down)        down ;;
    clean)       clean ;;
    status)      status ;;
    build)       build_image ;;
    check-data)  check_data ;;
    help|-h|--help) usage ;;
    *)           usage ;;
esac
