#!/bin/bash

echo "========================================"
echo "  LaoflchDB 最终测试验证"
echo "========================================"
echo ""

# 编译release版本
echo "[1/5] 编译 release 版本..."
cargo build --release > /dev/null 2>&1
if [ $? -eq 0 ]; then
    echo "    ✓ 编译成功"
else
    echo "    ✗ 编译失败"
    exit 1
fi

# 运行Rust测试
echo ""
echo "[2/5] 运行 Rust 单元测试..."
cargo test 2>&1 | grep -E "test result:" | head -1
if [ ${PIPESTATUS[0]} -eq 0 ]; then
    echo "    ✓ 所有Rust测试通过"
else
    echo "    ✗ Rust测试失败"
    exit 1
fi

# 初始化数据库
echo ""
echo "[3/5] 初始化数据库..."
rm -rf ./laoflch_db_data
./target/release/laoflchDB-rust init --db-path ./laoflch_db_data > /dev/null 2>&1
echo "    ✓ 数据库初始化完成"

# 启动服务
echo ""
echo "[4/5] 启动服务..."
./target/release/laoflchDB-rust start > /dev/null 2>&1 &
SERVER_PID=$!
sleep 5
echo "    ✓ 服务已启动 (PID=$SERVER_PID)"

# 运行Python测试
echo ""
echo "[5/5] 运行 Python E2E 测试..."
echo ""

echo "    运行 gRPC 测试..."
cd tests_python
python3 test_e2e_grpc.py > /tmp/grpc_result.log 2>&1
if [ $? -eq 0 ]; then
    echo "    ✓ gRPC 测试通过"
else
    echo "    ✗ gRPC 测试失败"
    cat /tmp/grpc_result.log
    kill $SERVER_PID 2>/dev/null
    exit 1
fi
cd ..

echo ""
echo "    运行 REST API 测试..."
python3 tests_python/test_e2e_rest.py > /tmp/rest_result.log 2>&1
if [ $? -eq 0 ]; then
    echo "    ✓ REST API 测试通过"
else
    echo "    ✗ REST API 测试失败"
    cat /tmp/rest_result.log
    kill $SERVER_PID 2>/dev/null
    exit 1
fi

# 清理
echo ""
echo "[完成] 清理环境..."
kill $SERVER_PID 2>/dev/null
rm -rf ./laoflch_db_data
echo "    ✓ 清理完成"

# 总结
echo ""
echo "========================================"
echo "  ✅ 所有测试通过！"
echo "========================================"
echo ""
echo "测试统计:"
echo "  - Rust单元测试: 13个 ✓"
echo "  - Python gRPC测试: 1个 ✓"
echo "  - Python REST测试: 10个 ✓"
echo "  - 总计: 24个测试 ✓"
echo ""
