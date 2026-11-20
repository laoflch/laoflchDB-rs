#!/usr/bin/env bash
# 用新镜像重建 5 个 raft 容器，保持原有配置/数据/端口/网络不变
# 用法：IMAGE=laoflchdb-raft:0.1.14 bash deploy_raft_recreate.sh
set -euo pipefail

IMAGE="${IMAGE:-laoflchdb-raft:0.1.14}"
CONFIG_DIR="/workspace/rust_space/laoflchDB-rust/deploy_raft"
DATA_ROOT="/workspace/raft_data"
NETWORK="laoflchdb-net"

for i in 1 2 3 4 5; do
  name="raft-node-$i"
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker run -d --name "$name" \
    --network "$NETWORK" \
    --security-opt seccomp=unconfined \
    -p "$((29998 + i))":29999 \
    -p "$((18079 + i))":18080 \
    -v "$DATA_ROOT/$name:/app/laoflch_raft_data" \
    -v "$CONFIG_DIR/$name/prod_raft.yaml:/app/config/prod_raft.yaml" \
    -v /usr/local/cuda:/usr/local/cuda:ro \
    -v /usr/lib/x86_64-linux-gnu/libcuda.so.1:/usr/lib/x86_64-linux-gnu/libcuda.so.1:ro \
    "$IMAGE" >/dev/null
  echo "已启动 $name：gRPC $((29998 + i)) / REST $((18079 + i))"
done
