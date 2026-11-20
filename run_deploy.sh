#!/bin/bash
# laoflchDB 部署脚本（生产参数固化版）
#
# 关键约定：
#   1. 二进制必须在 ubuntu:24.04 容器内编译（glibc 2.39），与运行时镜像基础一致。
#      宿主是 Ubuntu 26.04（glibc 2.43），直接在宿主 cargo build 出的产物在镜像里起不来。
#   2. 本机没有 docker-compose（既无 docker-compose 二进制也无 compose 插件），全部用原生 docker。
#   3. GPU 通过 nvidia-container-runtime 注入，必须带 --gpus all；否则报 CUDA_ERROR_NO_DEVICE 回退 CPU。
#   4. 容器需接入 laoflchdb-net 并带别名 laoflchdb，各 MCP 靠该主机名访问 gRPC/REST/S3。

set -e

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$PROJECT_DIR"

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\([^+"]*\).*/\1/')
IMAGE="laoflchdb:${VERSION}"
CONTAINER="laoflchdb-rust-wsl"
BUILD_CONTAINER="laoflchdb-build24"

# 当前用户若无 docker 权限，自动加 sudo 前缀
if docker info >/dev/null 2>&1; then
    DOCKER="docker"
else
    DOCKER="sudo docker"
fi

# ---------------------------------------------------------------------------
# 24.04 构建容器
# ---------------------------------------------------------------------------
prepare_build_container() {
    if $DOCKER inspect "$BUILD_CONTAINER" >/dev/null 2>&1; then
        $DOCKER start "$BUILD_CONTAINER" >/dev/null 2>&1 || true
        return 0
    fi

    echo "== 创建 24.04 构建容器（首次需要装依赖，约 1-2 分钟）=="
    $DOCKER run -d --name "$BUILD_CONTAINER" \
      -v /home/laoflch:/home/laoflch \
      -v /workspace:/workspace \
      -v /usr/local/cuda:/usr/local/cuda:ro \
      -v /usr/lib/x86_64-linux-gnu/libssl.so.1.1:/usr/lib/x86_64-linux-gnu/libssl.so.1.1:ro \
      -v /usr/lib/x86_64-linux-gnu/libcrypto.so.1.1:/usr/lib/x86_64-linux-gnu/libcrypto.so.1.1:ro \
      ubuntu:24.04 sleep infinity

    $DOCKER exec "$BUILD_CONTAINER" bash -c '
set -e
sed -i "s|http://archive.ubuntu.com/ubuntu|http://mirrors.tuna.tsinghua.edu.cn/ubuntu|g; s|http://security.ubuntu.com/ubuntu|http://mirrors.tuna.tsinghua.edu.cn/ubuntu|g" /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null || true
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config perl protobuf-compiler libssl-dev ca-certificates libclang-dev clang
# cudarc 的 build script 会执行 nvcc --version 做版本探测；本仓库不含需要 nvcc 实际编译的 kernel，给个版本回显即可
printf "%s\n" "#!/bin/sh" "cat <<EOF" "nvcc: NVIDIA (R) Cuda compiler driver" "Copyright (c) 2005-2025 NVIDIA Corporation" "Built on Thu_Jun__5_16:22:59_PDT_2025" "Cuda compilation tools, release 12.9, V12.9.86" "EOF" > /usr/local/bin/nvcc
chmod +x /usr/local/bin/nvcc
'

    # 链接期需要 -lcuda（宿主 /usr/lib/x86_64-linux-gnu 有，容器默认没有）
    LIBCUDA_REAL=$(readlink -f /usr/lib/x86_64-linux-gnu/libcuda.so.1)
    if [ -n "$LIBCUDA_REAL" ] && [ -f "$LIBCUDA_REAL" ]; then
        $DOCKER cp "$LIBCUDA_REAL" "$BUILD_CONTAINER:/usr/lib/x86_64-linux-gnu/$(basename "$LIBCUDA_REAL")"
        $DOCKER exec "$BUILD_CONTAINER" bash -c "cd /usr/lib/x86_64-linux-gnu && ln -sf $(basename \"$LIBCUDA_REAL\") libcuda.so.1 && ln -sf libcuda.so.1 libcuda.so && ldconfig"
    fi
}

build_binary_2404() {
    echo "== 1/3 在 ubuntu:24.04 容器内编译（release + cuda）=="
    prepare_build_container
    local llvm_lib
    llvm_lib=$($DOCKER exec "$BUILD_CONTAINER" bash -c 'ls -d /usr/lib/llvm-*/lib 2>/dev/null | tail -1')
    $DOCKER exec -u 1000:1000 \
      -e HOME=/home/laoflch \
      -e PATH=/home/laoflch/.cargo/bin:/usr/local/bin:/usr/bin:/bin \
      -w "$PROJECT_DIR" "$BUILD_CONTAINER" bash -lc "
source ./local.env
export LIBCLANG_PATH=${llvm_lib:-/usr/lib/llvm-18/lib}
export CLANG_PATH=/usr/bin/clang
export BINDGEN_EXTRA_CLANG_ARGS=
cargo build --release --features cuda --bin laoflchdb
"
}

# 宿主直编（不用于生产镜像：宿主 glibc 2.43 > 镜像 2.39）
build_binary() {
    echo "== 编译二进制（宿主直编，仅用于本地调试）=="
    export PATH="$HOME/.cargo/bin:$PATH"
    source "$PROJECT_DIR/local.env"
    cargo build --release --features cuda --bin laoflchdb
}

# ---------------------------------------------------------------------------
# 镜像 / 容器
# ---------------------------------------------------------------------------
build_image() {
    echo "== 2/3 构建镜像 ${IMAGE}（基础镜像 ubuntu:24.04）=="
    $DOCKER build -t "$IMAGE" -f Dockerfile.prod .
}

run_container() {
    echo "== 3/3 启动容器 ${CONTAINER} =="
    $DOCKER rm -f "$CONTAINER" >/dev/null 2>&1 || true
    $DOCKER run -d --name "$CONTAINER" \
      --gpus all \
      -p 19000:19000 \
      -p 29777:29777 \
      -p 38080:38080 \
      -e LD_LIBRARY_PATH=/usr/local/cuda/lib64 \
      -e RUST_LOG=info \
      -e DB_PATH=/app/data \
      -v "$PROJECT_DIR/laoflch_db_data_prod:/app/data" \
      -v "$PROJECT_DIR/laoflch_db_index:/app/indexes" \
      -v "$PROJECT_DIR/laoflch_db_graph:/app/laoflch_db_graph" \
      -v "$PROJECT_DIR/laoflch_db_oss:/app/laoflch_db_oss" \
      -v "$PROJECT_DIR/laoflch_hnsw_data:/app/laoflch_hnsw_data" \
      -v "$PROJECT_DIR/laoflch_db_model:/app/model" \
      -v /usr/local/cuda/lib64:/usr/local/cuda/lib64:ro \
      "$IMAGE"

    # 接入业务网：各 MCP 容器在 laoflchdb-net 上用主机名 laoflchdb 访问 gRPC/REST/S3，
    # 不接这张网会报 "name resolver error: produced zero addresses"
    $DOCKER network inspect laoflchdb-net >/dev/null 2>&1 || $DOCKER network create laoflchdb-net
    $DOCKER network connect --alias laoflchdb laoflchdb-net "$CONTAINER"
    echo "已接入 laoflchdb-net（别名 laoflchdb）"
}

case "${1:-}" in
    build)
        build_binary
        ;;
    build-2404)
        build_binary_2404
        ;;
    docker-build)
        build_image
        ;;
    docker-run)
        run_container
        sleep 25
        $DOCKER ps --filter "name=$CONTAINER" --format "{{.Names}} {{.Status}}"
        ;;
    docker-stop)
        $DOCKER rm -f "$CONTAINER"
        ;;
    docker-logs)
        $DOCKER logs -f "$CONTAINER"
        ;;
    docker-status)
        $DOCKER ps -a --filter "name=$CONTAINER"
        ;;
    deploy)
        build_binary_2404
        build_image
        run_container
        echo "== 等待服务就绪 =="
        for i in $(seq 1 12); do
            sleep 5
            if curl -sf -m 3 http://127.0.0.1:38080/health >/dev/null; then
                echo "服务已就绪（约 $((i*5))s）"
                break
            fi
            echo "  ...等待启动 ($((i*5))s)"
        done
        echo "== 健康检查 =="
        curl -s -o /dev/null -w "health: %{http_code} %{time_total}s\n" -m 5 http://127.0.0.1:38080/health || true
        echo "== GPU / 限流日志 =="
        $DOCKER logs "$CONTAINER" 2>&1 | grep -iE "CUDA GPU 可用|CUDA 设备初始化失败|上传并发上限" | head -5 || true
        ;;
    *)
        echo "Usage: $0 <command>"
        echo ""
        echo "Commands:"
        echo "  build-2404     - 在 ubuntu:24.04 容器内编译（生产产物，glibc 2.39）"
        echo "  build          - 宿主直编（仅本地调试，产物不能进 24.04 镜像）"
        echo "  docker-build   - 构建镜像 ${IMAGE}"
        echo "  docker-run     - 启动容器（--gpus all，端口 19000/29777/38080，接入 laoflchdb-net）"
        echo "  docker-stop    - 停止并删除容器"
        echo "  docker-logs    - 跟随日志"
        echo "  docker-status  - 查看容器状态"
        echo "  deploy         - 完整部署（24.04 内编译 + 构建镜像 + 起容器 + 健康检查）"
        ;;
esac
