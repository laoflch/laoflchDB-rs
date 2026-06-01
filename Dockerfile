# 使用官方Rust镜像作为构建阶段
FROM rust:1.75-slim-bookworm AS builder

# 设置工作目录
WORKDIR /app

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# 复制Cargo文件并缓存依赖
COPY Cargo.toml Cargo.lock ./
COPY laoflchdb_db_engine/Cargo.toml laoflchdb_db_engine/
COPY multi_table_rocksdb/Cargo.toml multi_table_rocksdb/

# 创建空的lib.rs文件以满足构建需求
RUN mkdir -p laoflchdb_db_engine/src multi_table_rocksdb/src src
RUN echo "pub fn main() {}" > src/main.rs
RUN echo "pub mod pb;" > laoflchdb_db_engine/src/lib.rs
RUN echo "pub struct MultiTableRocksDBEngine;" > multi_table_rocksdb/src/multi_table_rocksdb.rs

# 构建依赖（缓存层）
RUN cargo build --release --features=production 2>&1 || true

# 复制源代码
COPY . .

# 构建发布版本
RUN cargo build --release --features=production

# 运行阶段
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# 创建运行用户
RUN useradd -m appuser
USER appuser

# 设置工作目录
WORKDIR /app

# 从构建阶段复制二进制文件
COPY --from=builder /app/target/release/laoflchDB-rust /app/laoflchDB-rust

# 复制配置文件
COPY laoflchdb.yaml /app/laoflchdb.yaml

# 创建数据目录
RUN mkdir -p /app/data

# 暴露端口
EXPOSE 8080 19777

# 设置环境变量
ENV RUST_LOG=info
ENV DB_PATH=/app/data

# 健康检查
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# 启动命令
CMD ["/app/laoflchDB-rust", "-c", "/app/laoflchdb.yaml", "start"]
