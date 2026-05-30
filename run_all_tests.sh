#!/bin/bash

set -e

echo "========================================"
echo "  LaoflchDB 全量自动化测试"
echo "========================================"
echo ""

# 颜色定义
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

# 测试计数器
total_tests=0
passed_tests=0
failed_tests=0

# 测试函数
run_test() {
    local test_name=$1
    local test_cmd=$2

    echo -e "${BLUE}[测试]${NC} ${test_name}"
    if eval $test_cmd > /dev/null 2>&1; then
        echo -e "${GREEN}✓ 通过${NC}"
        ((passed_tests++))
    else
        echo -e "${RED}✗ 失败${NC}"
        ((failed_tests++))
        return 1
    fi
    ((total_tests++))
}

# 清理函数
cleanup() {
    echo ""
    echo "清理测试环境..."
    pkill -f laoflchDB-rust || true
    rm -rf ./laoflch_db_data
}

# 初始化数据库
init_db() {
    echo -e "${BLUE}[初始化]${NC} 初始化数据库..."
    ./target/release/laoflchDB-rust init --db-path ./laoflch_db_data > /dev/null 2>&1
    echo -e "${GREEN}✓ 数据库初始化完成${NC}"
}

# 1. Rust 单元测试
echo "========================================"
echo "  步骤 1: Rust 单元测试"
echo "========================================"
echo ""

run_test "基础UUID测试" "cargo test --test basic_uuid_tests --quiet"
run_test "Protobuf测试" "cargo test --test protobuf_tests --quiet"
run_test "REST服务测试" "cargo test --test rest_tests --quiet"
run_test "集成测试" "cargo test --test integration_tests --quiet"

# 2. Python 自动化测试
echo ""
echo "========================================"
echo "  步骤 2: Python 自动化测试"
echo "========================================"
echo ""

# 检查Python环境
if ! command -v python3 &> /dev/null; then
    echo -e "${YELLOW}⚠ Python3 未安装，跳过Python测试${NC}"
else
    echo -e "${BLUE}[准备]${NC} 编译 release 版本..."
    cargo build --release > /dev/null 2>&1
    echo -e "${GREEN}✓ 编译完成${NC}"
    echo ""

    # 清理并初始化数据库
    cleanup
    init_db

    # 启动服务（gRPC + REST）
    echo -e "${BLUE}[启动]${NC} 启动服务..."
    ./target/release/laoflchDB-rust start \
        --addr 127.0.0.1:19777 > /tmp/server.log 2>&1 &
    SERVER_PID=$!
    sleep 3
    echo -e "${GREEN}✓ 服务已启动 (PID=$SERVER_PID)${NC}"
    echo ""

    # gRPC 测试
    echo -e "${BLUE}[测试]${NC} gRPC 端到端测试"
    cd tests_python
    if python3 test_e2e_grpc.py > /tmp/grpc_test.log 2>&1; then
        echo -e "${GREEN}✓ gRPC 测试通过${NC}"
        ((passed_tests++))
    else
        echo -e "${RED}✗ gRPC 测试失败${NC}"
        cat /tmp/grpc_test.log
        ((failed_tests++))
    fi
    ((total_tests++))
    cd ..

    echo ""

    # REST API 测试
    echo -e "${BLUE}[测试]${NC} REST API 端到端测试"
    cd tests_python
    if python3 test_e2e_rest.py > /tmp/rest_test.log 2>&1; then
        echo -e "${GREEN}✓ REST API 测试通过${NC}"
        ((passed_tests++))
    else
        echo -e "${RED}✗ REST API 测试失败${NC}"
        cat /tmp/rest_test.log
        ((failed_tests++))
    fi
    ((total_tests++))
    cd ..

    # 停止服务
    echo ""
    echo -e "${BLUE}[清理]${NC} 停止服务..."
    kill $SERVER_PID 2>/dev/null || true
    cleanup
fi

# 3. 测试总结
echo ""
echo "========================================"
echo "  测试总结"
echo "========================================"
echo ""

echo -e "总测试数: ${total_tests}"
echo -e "${GREEN}通过: ${passed_tests}${NC}"
if [ $failed_tests -gt 0 ]; then
    echo -e "${RED}失败: ${failed_tests}${NC}"
else
    echo -e "失败: ${failed_tests}"
fi
echo ""

if [ $failed_tests -eq 0 ]; then
    echo -e "${GREEN}========================================"
    echo "  ✅ 所有测试通过！"
    echo "========================================${NC}"
    exit 0
else
    echo -e "${RED}========================================"
    echo "  ❌ 部分测试失败"
    echo "========================================${NC}"
    exit 1
fi
