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
#   EXTRA_GROUPS：额外组，格式 "id:name:成员1,成员2,成员3[:bootstrap节点]"，多个组用分号分隔。
#     例：EXTRA_GROUPS="group-laoflchMD-etc:laoflchMD-etc:2,1,5:2" ./deploy_raft_cluster.sh up
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── 可配置项 ──
NETWORK="${NETWORK:-laoflchdb-net}"
IMAGE="${IMAGE:-laoflchdb-raft:0.1.13}"
NODE_COUNT="${NODE_COUNT:-5}"          # 集群节点数（同时也是组数，滑动窗口一组一个）
GROUP_SIZE="${GROUP_SIZE:-3}"          # 每组节点数（滑动窗口宽度，须 <= NODE_COUNT）
BASE_PORT="${BASE_PORT:-29999}"        # node-1 宿主端口，后续节点依次 +1
REST_BASE_PORT="${REST_BASE_PORT:-18080}"   # node-1 REST 宿主端口，后续节点依次 +1
CONFIG_DIR="${CONFIG_DIR:-$SCRIPT_DIR/deploy_raft}"
CONTAINER_PREFIX="raft-node"
DATA_ROOT="/workspace/raft_data"            # Raft 数据宿主目录，每节点一个子目录 raft-node-{i}
CONTAINER_PORT=29999                   # 容器内 Raft gRPC 端口
CONTAINER_REST_PORT=18080              # 容器内 REST 端口（与参考配置 access_protocols rest 项一致）
ADMIN_PY="$SCRIPT_DIR/scripts/raft_admin.py"
# 额外组（滑动窗口之外的组）：持久化在 $CONFIG_DIR/extra_groups.conf，每行一个
#   <group_id>:<name>:<members逗号分隔>:<bootstrap节点号>
# 优先级：环境变量 EXTRA_GROUPS > 配置文件 > 内置默认
EXTRA_GROUPS_FILE="$CONFIG_DIR/extra_groups.conf"
EXTRA_GROUPS_DEFAULT="group-laoflchMD-etc:laoflchMD-etc:2,1,5:2"
if [ -f "$EXTRA_GROUPS_FILE" ]; then
    EXTRA_GROUPS="${EXTRA_GROUPS:-$(grep -v '^\s*$' "$EXTRA_GROUPS_FILE" | paste -sd';' -)}"
else
    EXTRA_GROUPS="${EXTRA_GROUPS-$EXTRA_GROUPS_DEFAULT}"
fi

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

    local ref="$SCRIPT_DIR/config/prod_raft.yaml"
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
    - group_id: "${g}"
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
    # 额外组（EXTRA_GROUPS）：滑动窗口之外的自定义组，仅当本节点是成员时写入。
    # 格式 "id:name:成员1,成员2,成员3[:bootstrap节点]"，bootstrap 缺省为第一个成员。
    if [ -n "$EXTRA_GROUPS" ]; then
        local specs spec ex_id ex_name ex_members ex_bs rest
        IFS=';' read -ra specs <<< "$EXTRA_GROUPS"
        for spec in "${specs[@]}"; do
            [ -z "$spec" ] && continue
            ex_id="${spec%%:*}"
            rest="${spec#*:}"
            ex_name="${rest%%:*}"
            rest="${rest#*:}"
            ex_members="${rest%%:*}"
            ex_bs="${rest#*:}"
            [ "$ex_bs" = "$ex_members" ] && ex_bs="${ex_members%%,*}"
            case ",${ex_members}," in
                *",${id},"*) ;;
                *) continue ;;
            esac
            bs="false"
            [ "$ex_bs" -eq "$id" ] && bs="true"
            cat >> "$tmp" <<EOF
    - group_id: "${ex_id}"
      name: "${ex_name}"
      node_id: ${id}
      self_addr: "$(peer_addr "$id")"
      bootstrap: ${bs}
      peers:
EOF
            local em
            IFS=',' read -ra ems <<< "$ex_members"
            for em in "${ems[@]}"; do
                echo "        - { id: ${em}, addr: \"$(peer_addr "$em")\" }" >> "$tmp"
            done
        done
    fi

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
    # 二进制链接了 CUDA 库，但 681MB 的 CUDA 运行库不拷贝进镜像（保持镜像小）：
    # 容器启动时 bind mount 宿主 /usr/local/cuda 提供 libcublas/libcublasLt/libcurand/libcudart
    # 驱动库 libcuda.so.1（28MB）与主镜像 laoflchdb 一致 COPY 进镜像（Dockerfile COPY deploy_raft/.libcuda_libs/）
    local libs_dir="$CONFIG_DIR/.libcuda_libs"
    mkdir -p "$libs_dir"
    cp -L /usr/lib/x86_64-linux-gnu/libcuda.so.1 "$libs_dir/libcuda.so.1"

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
        mkdir -p "${DATA_ROOT}/raft-node-${i}"
        echo "  [node-$i] $name  (gRPC 宿主 $hp -> $CONTAINER_PORT, REST 宿主 $(host_rest_port "$i") -> $CONTAINER_REST_PORT)"
        docker run -d --name "$name" \
            --network "$NETWORK" \
            --security-opt seccomp=unconfined \
            -p "$hp:$CONTAINER_PORT" \
            -p "$(host_rest_port "$i"):$CONTAINER_REST_PORT" \
            -v "${DATA_ROOT}/raft-node-${i}:/app/laoflch_raft_data" \
            -v "$CONFIG_DIR/$(node_name "$i")/prod_raft.yaml:/app/config/prod_raft.yaml" \
            -v /usr/local/cuda:/usr/local/cuda:ro \
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
    # 额外组（EXTRA_GROUPS）成员：用其第一个成员节点查询
    local specs spec ex_id rest ex_members rep
    IFS=';' read -ra specs <<< "$EXTRA_GROUPS"
    for spec in "${specs[@]}"; do
        [ -z "$spec" ] && continue
        ex_id="${spec%%:*}"
        rest="${spec#*:}"
        rest="${rest#*:}"
        ex_members="${rest%%:*}"
        rep="${ex_members%%,*}"
        echo "===== 组 $ex_id 成员 (via node-$rep 宿主端口 $(host_port "$rep")) ====="
        python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$rep")" --group "$ex_id" list-peers
    done
    for ((i = 1; i <= NODE_COUNT; i++)); do
        echo ""
        echo "===== node-$i 所属组状态 (宿主端口 $(host_port "$i")) ====="
        # 交给 raft_admin 自动探测（数字组 + 部署配置中的自定义命名组）
        python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" status
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

# 新增一个额外组：登记到 extra_groups.conf -> 重新生成成员节点配置 -> 重启成员容器 -> 等待就绪
# 用法：./deploy_raft_cluster.sh add-group <group_id> [--name 名称] [--members 2,1,5] [--bootstrap 2]
add_group() {
    local id="" name="" members="" bs=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --name)      name="${2:-}"; shift 2 ;;
            --members)   members="${2:-}"; shift 2 ;;
            --bootstrap) bs="${2:-}"; shift 2 ;;
            -*)          echo "错误：未知参数 $1" >&2; exit 1 ;;
            *)
                if [ -z "$id" ]; then id="$1"; else echo "错误：多余参数 $1" >&2; exit 1; fi
                shift ;;
        esac
    done
    if [ -z "$id" ]; then
        echo "用法: $0 add-group <group_id> [--name 名称] [--members 2,1,5] [--bootstrap 2]" >&2
        exit 1
    fi
    if ! [[ "$id" =~ ^[A-Za-z0-9_-]{1,64}$ ]]; then
        echo "错误：group_id 仅允许字母/数字/下划线/连字符，长度 1..64：$id" >&2
        exit 1
    fi

    # 重名检查（数字滑动窗口组 + 已登记的额外组）
    local g existing
    for ((g = 1; g <= NODE_COUNT; g++)); do
        if [ "$id" = "$g" ]; then
            echo "错误：组 $id 已存在（滑动窗口组）" >&2
            exit 1
        fi
    done
    IFS=';' read -ra existing <<< "$EXTRA_GROUPS"
    for spec in "${existing[@]}"; do
        [ -z "$spec" ] && continue
        if [ "${spec%%:*}" = "$id" ]; then
            echo "错误：组 $id 已存在（$(echo "$spec" | cut -d: -f3-4)）" >&2
            exit 1
        fi
    done

    # 默认：组名 = id；成员随机取 GROUP_SIZE 个不同节点；自举节点 = 第一个成员
    [ -z "$name" ] && name="$id"
    if [ -z "$members" ]; then
        members=$(shuf -i "1-$NODE_COUNT" -n "$GROUP_SIZE" | paste -sd, -)
    fi
    local m mn
    IFS=',' read -ra m <<< "$members"
    if [ "${#m[@]}" -lt 1 ]; then
        echo "错误：--members 不能为空" >&2
        exit 1
    fi
    for mn in "${m[@]}"; do
        if ! [[ "$mn" =~ ^[0-9]+$ ]] || [ "$mn" -lt 1 ] || [ "$mn" -gt "$NODE_COUNT" ]; then
            echo "错误：成员节点号非法：$mn（有效范围 1..$NODE_COUNT）" >&2
            exit 1
        fi
    done
    if [ -z "$bs" ]; then
        bs="${members%%,*}"
    fi
    if [[ ",$members," != *",$bs,"* ]]; then
        echo "错误：--bootstrap $bs 不在成员列表 $members 中" >&2
        exit 1
    fi

    local spec="$id:$name:$members:$bs"
    mkdir -p "$CONFIG_DIR"
    # 首次登记时，把当前生效的额外组一并落盘（避免丢失内置默认组）
    if [ ! -f "$EXTRA_GROUPS_FILE" ]; then
        local s
        IFS=';' read -ra existing <<< "$EXTRA_GROUPS"
        for s in "${existing[@]}"; do
            [ -n "$s" ] && echo "$s" >> "$EXTRA_GROUPS_FILE"
        done
    fi
    echo "$spec" >> "$EXTRA_GROUPS_FILE"
    EXTRA_GROUPS="${EXTRA_GROUPS:+$EXTRA_GROUPS;}$spec"
    echo "已登记新组：$id（名称 $name，成员 $members，自举 node-$bs）"

    # 重新生成配置（全部节点幂等），再重启成员容器使其加载新组
    local i
    for ((i = 1; i <= NODE_COUNT; i++)); do
        gen_config "$i"
    done
    echo "重启成员容器：${members//,/ }"
    IFS=',' read -ra m <<< "$members"
    for i in "${m[@]}"; do
        docker restart "$(node_name "$i")" >/dev/null
    done

    echo "等待组 $id 就绪（最长 60s；重启会触发其余组重新选举，属正常现象）..."
    local deadline=$(( $(date +%s) + 60 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$bs")" --group "$id" status >/dev/null 2>&1; then
            break
        fi
        sleep 2
    done
    echo ""
    python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$bs")" --group "$id" list-peers || true
    python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$bs")" --group "$id" status || true
}

# 删除一个额外组：从 extra_groups.conf 移除 -> 重生成配置 -> 重启原成员容器
# 用法：./deploy_raft_cluster.sh remove-group <group_id> [--purge]
#   --purge 同时清理该组的持久化数据（group-{id} 目录与业务库 CF），需短暂停止成员容器
remove_group() {
    local id="" purge=0
    while [ $# -gt 0 ]; do
        case "$1" in
            --purge) purge=1; shift ;;
            -*)      echo "错误：未知参数 $1" >&2; exit 1 ;;
            *)
                if [ -z "$id" ]; then id="$1"; else echo "错误：多余参数 $1" >&2; exit 1; fi
                shift ;;
        esac
    done
    if [ -z "$id" ]; then
        echo "用法: $0 remove-group <group_id> [--purge]" >&2
        exit 1
    fi

    # 滑动窗口数字组由编排决定，不支持单独删除
    local g
    for ((g = 1; g <= NODE_COUNT; g++)); do
        if [ "$id" = "$g" ]; then
            echo "错误：组 $id 是滑动窗口组（由 NODE_COUNT/GROUP_SIZE 决定），不支持删除" >&2
            exit 1
        fi
    done

    # 查找登记项并取出成员列表
    local specs spec members="" found=0
    IFS=';' read -ra specs <<< "$EXTRA_GROUPS"
    for spec in "${specs[@]}"; do
        [ -z "$spec" ] && continue
        if [ "${spec%%:*}" = "$id" ]; then
            members="$(echo "$spec" | cut -d: -f3)"
            found=1
        fi
    done
    if [ "$found" -ne 1 ]; then
        echo "错误：额外组 $id 不存在（当前额外组：$(grep -v '^\s*$' "$EXTRA_GROUPS_FILE" 2>/dev/null | cut -d: -f1 | paste -sd' ' -)）" >&2
        exit 1
    fi

    # 从持久化文件与内存列表移除
    if [ -f "$EXTRA_GROUPS_FILE" ]; then
        local tmp
        tmp="$(mktemp)"
        grep -v "^${id}:" "$EXTRA_GROUPS_FILE" > "$tmp" || true
        mv "$tmp" "$EXTRA_GROUPS_FILE"
    fi
    local remain="" s
    IFS=';' read -ra specs <<< "$EXTRA_GROUPS"
    for s in "${specs[@]}"; do
        [ -z "$s" ] && continue
        [ "${s%%:*}" = "$id" ] && continue
        remain="${remain:+$remain;}$s"
    done
    EXTRA_GROUPS="$remain"
    echo "已移除组 $id 的配置（成员节点 $members）"

    # 重新生成配置（组已不在 EXTRA_GROUPS，不会被写入），再重启原成员容器使其卸载该组
    local i
    for ((i = 1; i <= NODE_COUNT; i++)); do
        gen_config "$i"
    done
    local m
    IFS=',' read -ra m <<< "$members"
    for i in "${m[@]}"; do
        docker restart "$(node_name "$i")" >/dev/null
    done
    echo "等待组 $id 下线（最长 60s）..."
    local deadline=$(( $(date +%s) + 60 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if ! python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "${m[0]}")" --group "$id" status >/dev/null 2>&1; then
            break
        fi
        sleep 2
    done
    echo "组 $id 已下线"

    # 可选：清理持久化数据（停止成员容器后删除数据目录并 drop 业务 CF）
    if [ "$purge" -eq 1 ]; then
        echo "清理组 $id 的持久化数据..."
        for i in "${m[@]}"; do
            docker stop "$(node_name "$i")" >/dev/null
            docker run --rm --security-opt seccomp=unconfined --entrypoint /bin/sh \
                -v "${DATA_ROOT}/raft-node-${i}:/data" \
                "$IMAGE" -c "rm -rf /data/group-${id}; ldb --db=/data/business.db drop_column_family group_${id} >/dev/null 2>&1; ldb --db=/data/business_backup.db drop_column_family group_${id} >/dev/null 2>&1 || true" >/dev/null 2>&1 || true
            docker start "$(node_name "$i")" >/dev/null
        done
        echo "已清理 group-${id} 数据目录与业务 CF group_${id}"
    fi
}

clean() {
    down
    echo "删除数据目录与配置目录..."
    for ((i = 1; i <= NODE_COUNT; i++)); do
        rm -rf "${DATA_ROOT}/raft-node-${i}"
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
        # 交给 raft_admin 自动探测（数字组 + 部署配置中的自定义命名组）
        python3 "$ADMIN_PY" --addr "127.0.0.1:$(host_port "$i")" status 2>/dev/null || echo "(节点未就绪)"
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
    echo "  add-group  新增额外组（登记配置并重启成员容器）"
    echo "             $0 add-group <group_id> [--name 名称] [--members 2,1,5] [--bootstrap 2]"
    echo "  remove-group 删除额外组（移除配置并重启成员容器；--purge 同时清理数据）"
    echo "             $0 remove-group <group_id> [--purge]"
    echo "  build      仅构建镜像"
    echo "  check-data 校验各组各节点业务数据落盘是否一致（CHECK_GROUP=<组> 可只查某组）"
    echo ""
    echo "环境变量: NODE_COUNT(默认5) GROUP_SIZE(默认3) BASE_PORT(默认29999) REST_BASE_PORT(默认18080) NETWORK(默认laoflchdb-net) IMAGE CONFIG_DIR CHECK_GROUP EXTRA_GROUPS"
}

cmd="${1:-up}"
case "$cmd" in
    up)          up ;;
    down)        down ;;
    clean)       clean ;;
    status)      status ;;
    add-group|add_group) add_group "${@:2}" ;;
    remove-group|remove_group) remove_group "${@:2}" ;;
    build)       build_image ;;
    check-data)  check_data ;;
    help|-h|--help) usage ;;
    *)           usage ;;
esac
