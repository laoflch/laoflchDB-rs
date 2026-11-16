#!/bin/bash
# laoflchDB 部署脚本（生产参数固化版）
#
# 说明：
#   - 本机没有 docker-compose（既无 docker-compose 二进制也无 compose 插件），
#     因此这里全部使用原生 docker 命令，不再依赖 docker-compose.yml。
#   - GPU 通过 nvidia-container-runtime 注入，必须带 --gpus all；
#     否则容器内 CUDA 初始化会报 CUDA_ERROR_NO_DEVICE 并回退到 CPU。

set -e

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$PROJECT_DIR"

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\([^+"]*\).*/\1/')
IMAGE="laoflchdb:${VERSION}"
CONTAINER="laoflchdb-rust-wsl"

# 当前用户若无 docker 权限，自动加 sudo 前缀（编译步骤仍需以普通用户身份执行，避免 target/ 归属变成 root）
if docker info >/dev/null 2>&1; then
    DOCKER="docker"
else
    DOCKER="sudo docker"
fi

# cargo 可能不在 PATH（rustup 默认装到 $HOME/.cargo/bin）
if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:/home/laoflch/.cargo/bin:/usr/local/cargo/bin:$PATH"
fi

build_binary() {
    echo "== 1/3 编译二进制（release + cuda）=="
    export PATH="$HOME/.cargo/bin:$PATH"
    # shellcheck disable=SC1091
    source "$PROJECT_DIR/local.env"
    cargo build --release --features cuda --bin laoflchdb
}

build_image() {
    echo "== 2/3 构建镜像 ${IMAGE} =="
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
        build_binary
        build_image
        run_container
        echo "== 等待服务就绪 =="
        for i in $(seq 1 12); do
            sleep 5
            if curl -sf -m 3 http://127.0.0.1:38080/health >/dev/null; then
                echo "服务已就绪（${i}0s 内）"
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
        echo "  build          - 编译二进制（release + cuda）"
        echo "  docker-build   - 构建镜像 ${IMAGE}"
        echo "  docker-run     - 启动容器（--gpus all，端口 19000/29777/38080）"
        echo "  docker-stop    - 停止并删除容器"
        echo "  docker-logs    - 跟随日志"
        echo "  docker-status  - 查看容器状态"
        echo "  deploy         - 完整部署（编译 + 构建镜像 + 起容器 + 健康检查）"
        ;;
esac
