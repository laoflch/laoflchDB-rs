#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

echo "========================================"
echo "Python 自动化回归测试"
echo "========================================"
echo ""

# 检查是否跳过编译
if [ "$1" != "--skip-build" ]; then
    # 编译 release 版本
    echo -e "编译..."
    cargo build --release 2>&1 | head -30
else
    echo -e "跳过编译..."
fi

# 清理并初始化数据库
echo -e "\n初始化数据库..."
rm -rf ./laoflch_db_data
./target/release/laoflchDB-rust init --db-path ./laoflch_db_data

# 启动服务
echo -e "\n启动服务..."
LOG_FILE=/tmp/server_python_test.log
./target/release/laoflchDB-rust start --addr 127.0.0.1:19777 > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null; pkill -f laoflchDB-rust 2>/dev/null' EXIT

# 等待服务启动
sleep 3

# 验证服务在运行
echo "服务 PID: $SERVER_PID"
ps aux | grep laoflchDB-rust | grep -v grep

# 检查端口监听
sleep 2
echo -e "\n检查服务状态..."
if ss -tuln | grep -q 8080; then
  echo "✓ REST 服务正在监听端口 8080"
else
  echo "✗ REST 服务未正常监听"
  cat "$LOG_FILE"
  exit 1
fi

if ss -tuln | grep -q 19777; then
  echo "✓ gRPC 服务正在监听端口 19777"
else
  echo "✗ gRPC 服务未正常监听"
  cat "$LOG_FILE"
  exit 1
fi

# 运行 REST 测试
echo -e "\n========================================"
echo "运行 REST API 测试..."
cd tests_python
python3 test_e2e_rest.py
REST_EXIT=$?

# 运行 gRPC 测试
echo -e "\n========================================"
echo "运行 gRPC 测试..."
python3 test_e2e_grpc.py
GRPC_EXIT=$?

cd ..

if [ $REST_EXIT -eq 0 ] && [ $GRPC_EXIT -eq 0 ]; then
  echo -e "\n✅ 所有 Python 测试通过！"
  exit 0
else
  echo -e "\n✗ 部分 Python 测试失败！"
  echo "REST exit: $REST_EXIT, gRPC exit: $GRPC_EXIT"
  exit 1
fi
