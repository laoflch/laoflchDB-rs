#!/bin/bash
# =============================================================================
# laoflchdb 分布式对象存储集群部署脚本（access 节点 + data 节点分离）
#
# 角色划分（同一镜像，靠挂载不同配置区分）：
#   - data 节点（5 个）：容器名 raw-store-{i}
#       config/prod_data_node{i}.yaml -> 只启动 raw_store（chunk 直写本地文件，29998）
#   - access 节点（3 个）：容器名 s3-access-{i}
#       config/prod_access_node{i}.yaml -> 只启动分布式 S3 网关（19001）
#
# 共享与发现：
#   - 对象 / 桶 / multipart 元数据：access 节点写入外部 Raft 的 objmeta 组
#   - data 节点注册：raw_store 周期写入外部 Raft 的 raw_registry 组
#   - access 节点用「静态种子 + raw_registry」发现全部 data 节点，可访问所有 data 节点
#
# 用法：
#   ./deploy_distributed_cluster.sh up            启动 5 data + 3 access（data 先就绪）
#   ./deploy_distributed_cluster.sh down          停止并删除全部容器（保留数据目录）
#   ./deploy_distributed_cluster.sh clean         停止容器并删除 data 数据目录
#   ./deploy_distributed_cluster.sh status        查看容器、端口与 S3 匿名访问状态
#   ./deploy_distributed_cluster.sh logs data 2   查看 raw-store-2 日志
#   ./deploy_distributed_cluster.sh logs access 1 查看 s3-access-1 日志
#   ./deploy_distributed_cluster.sh build         仅构建镜像
#
# 可配置环境变量：
#   DATA_NODE_COUNT(默认5) ACCESS_NODE_COUNT(默认3)
#   ACCESS_S3_BASE_PORT(默认19001) DATA_RAW_BASE_PORT(默认29981)
#   IMAGE(默认 laoflchdb-distributed:0.1.14) NETWORK(默认 laoflchdb-net)
#   CONFIG_SRC_DIR(默认 config) DATA_ROOT(默认 /workspace/distributed_data)
#
# 前置条件：
#   1. 已手动 cargo build --release（镜像内不编译）
#   2. Raft 集群已运行，且存在 objmeta / raw_registry 两个组
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── 可配置项 ──
NETWORK="${NETWORK:-laoflchdb-net}"
IMAGE="${IMAGE:-laoflchdb-distributed:0.1.14}"
DATA_NODE_COUNT="${DATA_NODE_COUNT:-5}"
ACCESS_NODE_COUNT="${ACCESS_NODE_COUNT:-3}"
ACCESS_S3_BASE_PORT="${ACCESS_S3_BASE_PORT:-19001}"   # access-1 宿主 S3 端口，后续依次 +1
DATA_RAW_BASE_PORT="${DATA_RAW_BASE_PORT:-29981}"     # data-1 宿主 raw_store 端口，后续依次 +1
CONFIG_SRC_DIR="${CONFIG_SRC_DIR:-$SCRIPT_DIR/config}"
DATA_ROOT="${DATA_ROOT:-/workspace/distributed_data}"
RAFT_NODE_COUNT="${RAFT_NODE_COUNT:-5}"

DATA_PREFIX="raw-store"
ACCESS_PREFIX="s3-access"
DATA_CONTAINER_PORT=29998
ACCESS_CONTAINER_PORT=19001
DOCKERFILE="Dockerfile.distributed.prod"

if ! command -v docker >/dev/null 2>&1; then
    echo "错误：未找到 docker 命令" >&2
    exit 1
fi

if [ "$(id -u)" -eq 0 ]; then
    echo "警告：正在以 root 运行，数据目录将属于 root；建议用普通用户执行（需在 docker 组中）" >&2
fi

data_name() { printf "%s-%d" "$DATA_PREFIX" "$1"; }
access_name() { printf "%s-%d" "$ACCESS_PREFIX" "$1"; }
host_s3_port() { printf "%d" $((ACCESS_S3_BASE_PORT + $1 - 1)); }
host_raw_port() { printf "%d" $((DATA_RAW_BASE_PORT + $1 - 1)); }

data_config() { printf "%s/prod_data_node%d.yaml" "$CONFIG_SRC_DIR" "$1"; }
access_config() { printf "%s/prod_access_node%d.yaml" "$CONFIG_SRC_DIR" "$1"; }

ensure_network() {
    if docker network inspect "$NETWORK" >/dev/null 2>&1; then
        echo "使用已有网络: $NETWORK"
    else
        echo "创建 Docker 网络: $NETWORK"
        docker network create "$NETWORK" >/dev/null
    fi
}

build_image() {
    if [ ! -f "$SCRIPT_DIR/$DOCKERFILE" ]; then
        echo "错误：未找到 $DOCKERFILE" >&2
        exit 1
    fi
    if [ ! -f "$SCRIPT_DIR/target/release/laoflchdb" ]; then
        echo "错误：未找到 target/release/laoflchdb，请先执行 cargo build --release" >&2
        exit 1
    fi
    echo "构建镜像 $IMAGE ..."
    docker build -t "$IMAGE" -f "$SCRIPT_DIR/$DOCKERFILE" "$SCRIPT_DIR"
}

ensure_image() {
    if [ ! -f "$SCRIPT_DIR/target/release/laoflchdb" ]; then
        echo "错误：未找到 target/release/laoflchdb，请先执行 cargo build --release" >&2
        exit 1
    fi
    if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
        build_image
        return
    fi
    local bin_mtime dockerfile_mtime img_epoch
    bin_mtime=$(stat -c %Y "$SCRIPT_DIR/target/release/laoflchdb")
    dockerfile_mtime=$(stat -c %Y "$SCRIPT_DIR/$DOCKERFILE" 2>/dev/null || echo 0)
    img_epoch=$(date -d "$(docker image inspect "$IMAGE" --format '{{.Created}}')" +%s 2>/dev/null || echo 0)
    if [ "$bin_mtime" -gt "$img_epoch" ] || [ "$dockerfile_mtime" -gt "$img_epoch" ]; then
        if [ "${ALLOW_IMAGE_OVERWRITE:-0}" != "1" ]; then
            echo "错误：target/release/laoflchdb 或 Dockerfile 比镜像 $IMAGE 新，但 $IMAGE 已存在。" >&2
            echo "      默认不覆盖已发布的 tag（避免丢失回滚点）。请任选其一：" >&2
            echo "      1) 用新 tag：IMAGE=laoflchdb-distributed:0.1.15 ./deploy_distributed_cluster.sh up" >&2
            echo "      2) 明确允许覆盖：ALLOW_IMAGE_OVERWRITE=1 ./deploy_distributed_cluster.sh up" >&2
            exit 1
        fi
        echo "检测到二进制或 Dockerfile 比镜像新，重新构建镜像 $IMAGE ..."
        build_image
    else
        echo "使用已有镜像: $IMAGE"
    fi
}

remove_containers() {
    local i
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        docker rm -f "$(data_name "$i")" >/dev/null 2>&1 || true
    done
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        docker rm -f "$(access_name "$i")" >/dev/null 2>&1 || true
    done
}

check_configs() {
    local i cfg missing=0
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        cfg="$(data_config "$i")"
        if [ ! -f "$cfg" ]; then
            echo "错误：缺少 data 节点 $i 的配置 $cfg" >&2
            missing=1
        fi
    done
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        cfg="$(access_config "$i")"
        if [ ! -f "$cfg" ]; then
            echo "错误：缺少 access 节点 $i 的配置 $cfg" >&2
            missing=1
        fi
    done
    if [ "$missing" -ne 0 ]; then
        exit 1
    fi
}

check_ports() {
    local i hp busy=0
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        hp="$(host_raw_port "$i")"
        if (echo > "/dev/tcp/127.0.0.1/$hp") 2>/dev/null; then
            echo "错误：宿主端口 $hp 已被占用（$(data_name "$i")）" >&2
            busy=1
        fi
    done
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        hp="$(host_s3_port "$i")"
        if (echo > "/dev/tcp/127.0.0.1/$hp") 2>/dev/null; then
            echo "错误：宿主端口 $hp 已被占用（$(access_name "$i")）" >&2
            busy=1
        fi
    done
    if [ "$busy" -ne 0 ]; then
        echo "请先释放端口，或用 DATA_RAW_BASE_PORT / ACCESS_S3_BASE_PORT 指定其它端口段" >&2
        exit 1
    fi
}

warn_raft() {
    local i missing=()
    for ((i = 1; i <= RAFT_NODE_COUNT; i++)); do
        if [ "$(docker inspect -f '{{.State.Running}}' "raft-node-$i" 2>/dev/null)" != "true" ]; then
            missing+=("raft-node-$i")
        fi
    done
    if [ "${#missing[@]}" -gt 0 ]; then
        echo "警告：以下 Raft 容器未运行：${missing[*]}" >&2
        echo "      元数据与注册依赖 objmeta / raw_registry 组，请先 ./deploy_raft_cluster.sh up" >&2
    fi
}

start_data_nodes() {
    echo "启动 ${DATA_NODE_COUNT} 个 data 节点（raw_store）..."
    local i name cfg hp
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        name="$(data_name "$i")"
        cfg="$(data_config "$i")"
        hp="$(host_raw_port "$i")"
        mkdir -p "$DATA_ROOT/$name"
        echo "  [data-$i] $name  (raw_store 宿主 $hp -> $DATA_CONTAINER_PORT)"
        docker run -d --name "$name" \
            --network "$NETWORK" \
            --security-opt seccomp=unconfined \
            -p "$hp:$DATA_CONTAINER_PORT" \
            -v "$DATA_ROOT/$name:/app/raw_store" \
            -v /usr/local/cuda:/usr/local/cuda:ro \
            -v /usr/lib/x86_64-linux-gnu/libcuda.so.1:/usr/lib/x86_64-linux-gnu/libcuda.so.1:ro \
            -v "$cfg:/app/config/active.yaml:ro" \
            "$IMAGE" /app/laoflchdb -c /app/config/active.yaml start >/dev/null
    done
}

start_access_nodes() {
    echo "启动 ${ACCESS_NODE_COUNT} 个 access 节点（S3 网关）..."
    local i name cfg hp
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        name="$(access_name "$i")"
        cfg="$(access_config "$i")"
        hp="$(host_s3_port "$i")"
        echo "  [access-$i] $name  (S3 宿主 $hp -> $ACCESS_CONTAINER_PORT)"
        docker run -d --name "$name" \
            --network "$NETWORK" \
            --security-opt seccomp=unconfined \
            -p "$hp:$ACCESS_CONTAINER_PORT" \
            -v /usr/local/cuda:/usr/local/cuda:ro \
            -v /usr/lib/x86_64-linux-gnu/libcuda.so.1:/usr/lib/x86_64-linux-gnu/libcuda.so.1:ro \
            -v "$cfg:/app/config/active.yaml:ro" \
            "$IMAGE" /app/laoflchdb -c /app/config/active.yaml start >/dev/null
    done
}

# 容器内端口可连接即视为就绪（ubuntu 基础镜像自带 bash 的 /dev/tcp）
container_ready() {
    local name="$1" port="$2"
    [ "$(docker inspect -f '{{.State.Running}}' "$name" 2>/dev/null)" = "true" ] || return 1
    docker exec "$name" bash -c "exec 3<>/dev/tcp/127.0.0.1/$port" >/dev/null 2>&1
}

wait_ready() {
    local role="$1" count="$2" port="$3"
    echo "等待所有 ${role} 节点端口就绪（超时 120s）..."
    local deadline=$(( $(date +%s) + 120 )) i name ready
    while [ "$(date +%s)" -lt "$deadline" ]; do
        ready=1
        for ((i = 1; i <= count; i++)); do
            if [ "$role" = "data" ]; then
                name="$(data_name "$i")"
            else
                name="$(access_name "$i")"
            fi
            if ! container_ready "$name" "$port"; then
                ready=0
                break
            fi
        done
        if [ "$ready" -eq 1 ]; then
            echo "所有 ${role} 节点已就绪"
            return 0
        fi
        sleep 3
    done
    echo "错误：${role} 节点未在 120 秒内就绪" >&2
    if [ "$role" = "data" ]; then
        docker ps -a --filter "name=$DATA_PREFIX" --format "table {{.Names}}\t{{.Status}}"
    else
        docker ps -a --filter "name=$ACCESS_PREFIX" --format "table {{.Names}}\t{{.Status}}"
    fi
    for ((i = 1; i <= count; i++)); do
        if [ "$role" = "data" ]; then name="$(data_name "$i")"; else name="$(access_name "$i")"; fi
        echo "----- $name 最近日志 -----" >&2
        docker logs --tail 20 "$name" 2>&1 >&2 || true
    done
    exit 1
}

verify() {
    local i hp raw code
    echo ""
    echo "==================== access 节点（S3 入口） ===================="
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        hp="$(host_s3_port "$i")"
        code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "http://127.0.0.1:$hp/" 2>/dev/null || true)"
        echo "  $(access_name "$i"): http://127.0.0.1:$hp  (匿名访问 HTTP ${code:-无响应}，403 属正常)"
    done
    echo ""
    echo "==================== data 节点（raw_store 内部地址） ===================="
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        raw="$(host_raw_port "$i")"
        echo "  $(data_name "$i"): 127.0.0.1:$raw -> 容器内 $DATA_CONTAINER_PORT（容器间用 $(data_name "$i"):$DATA_CONTAINER_PORT）"
    done
    echo ""
    echo "S3 客户端示例（access key: admin / secret: laoflchdb，三个入口等价）："
    echo "  aws --endpoint-url http://127.0.0.1:$(host_s3_port 1) s3 mb s3://demo"
    echo "  aws --endpoint-url http://127.0.0.1:$(host_s3_port 2) s3 ls s3://demo"
    echo ""
    echo "查看日志： $0 logs data <节点号> | $0 logs access <节点号>"
}

up() {
    check_configs
    ensure_network
    ensure_image
    warn_raft
    # 先移除旧容器再检查端口，避免自身占用导致误报
    remove_containers
    check_ports
    if ! mkdir -p "$DATA_ROOT" 2>/dev/null || [ ! -w "$DATA_ROOT" ]; then
        echo "错误：数据目录 $DATA_ROOT 不可写（可能由 root 创建），请检查属主或改用 DATA_ROOT" >&2
        exit 1
    fi
    # 先起 data，再起 access（access 的静态种子要求至少一个 data 可达）
    start_data_nodes
    wait_ready data "$DATA_NODE_COUNT" "$DATA_CONTAINER_PORT"
    start_access_nodes
    wait_ready access "$ACCESS_NODE_COUNT" "$ACCESS_CONTAINER_PORT"
    verify
}

down() {
    echo "停止并删除分布式对象存储容器（保留数据目录）..."
    remove_containers
    echo "完成"
}

clean() {
    down
    echo "删除 data 节点数据目录 $DATA_ROOT ..."
    local i
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        rm -rf "$DATA_ROOT/$(data_name "$i")"
    done
    echo "完成"
}

status() {
    echo "===== 容器状态 ====="
    docker ps -a --filter "name=$DATA_PREFIX" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
    docker ps -a --filter "name=$ACCESS_PREFIX" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
    echo ""
    echo "===== access 节点 S3 响应 ====="
    local i hp code
    for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
        hp="$(host_s3_port "$i")"
        code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "http://127.0.0.1:$hp/" 2>/dev/null || true)"
        echo "  $(access_name "$i")  端口 $hp  HTTP ${code:-无响应}"
    done
    echo ""
    echo "===== data 节点端口 ====="
    local raw
    for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
        raw="$(host_raw_port "$i")"
        if (echo > "/dev/tcp/127.0.0.1/$raw") 2>/dev/null; then
            echo "  $(data_name "$i")  端口 $raw  可连接"
        else
            echo "  $(data_name "$i")  端口 $raw  不可连接"
        fi
    done
}

show_logs() {
    local role="${1:-}" target="${2:-}" i name
    if [ "$role" != "data" ] && [ "$role" != "access" ]; then
        echo "用法: $0 logs {data|access} [节点号]" >&2
        exit 1
    fi
    if [ -n "$target" ]; then
        if [ "$role" = "data" ]; then
            if ! [[ "$target" =~ ^[0-9]+$ ]] || [ "$target" -lt 1 ] || [ "$target" -gt "$DATA_NODE_COUNT" ]; then
                echo "错误：data 节点号须在 1..$DATA_NODE_COUNT 之间" >&2
                exit 1
            fi
            name="$(data_name "$target")"
        else
            if ! [[ "$target" =~ ^[0-9]+$ ]] || [ "$target" -lt 1 ] || [ "$target" -gt "$ACCESS_NODE_COUNT" ]; then
                echo "错误：access 节点号须在 1..$ACCESS_NODE_COUNT 之间" >&2
                exit 1
            fi
            name="$(access_name "$target")"
        fi
        docker logs --tail 200 -f "$name"
        return
    fi
    if [ "$role" = "data" ]; then
        for ((i = 1; i <= DATA_NODE_COUNT; i++)); do
            name="$(data_name "$i")"
            echo "===== $name ====="
            docker logs --tail 100 "$name" 2>&1 || true
            echo ""
        done
    else
        for ((i = 1; i <= ACCESS_NODE_COUNT; i++)); do
            name="$(access_name "$i")"
            echo "===== $name ====="
            docker logs --tail 100 "$name" 2>&1 || true
            echo ""
        done
    fi
}

usage() {
    echo "用法: $0 {up|down|clean|status|logs {data|access} [节点号]|build|help}"
    echo ""
    echo "命令:"
    echo "  up                启动 5 个 data 节点 + 3 个 access 节点（data 先就绪）"
    echo "  down              停止并删除全部容器（保留数据目录）"
    echo "  clean             停止容器并删除 data 数据目录"
    echo "  status            查看容器、端口与 S3 匿名访问状态"
    echo "  logs data [i]     查看 data 节点日志（不带节点号时输出全部）"
    echo "  logs access [i]   查看 access 节点日志（不带节点号时输出全部）"
    echo "  build             仅构建镜像"
    echo ""
    echo "环境变量: DATA_NODE_COUNT(默认5) ACCESS_NODE_COUNT(默认3)"
    echo "          ACCESS_S3_BASE_PORT(默认19001) DATA_RAW_BASE_PORT(默认29981)"
    echo "          IMAGE(默认laoflchdb-distributed:0.1.14) NETWORK(默认laoflchdb-net)"
    echo "          CONFIG_SRC_DIR(默认config) DATA_ROOT(默认/workspace/distributed_data)"
}

cmd="${1:-up}"
case "$cmd" in
    up)             up ;;
    down)           down ;;
    clean)          clean ;;
    status)         status ;;
    logs)           show_logs "${2:-}" "${3:-}" ;;
    build)          build_image ;;
    help|-h|--help) usage ;;
    *)              usage ;;
esac
