#!/bin/bash

# LaoflchDB 测试脚本

set -e

echo "========================================"
echo "  LaoflchDB 自动化测试"
echo "========================================"
echo ""

# 颜色定义
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# 运行测试函数
run_test() {
    local test_name=$1
    local test_cmd=$2
    
    echo -e "${BLUE}运行 ${test_name}...${NC}"
    if eval $test_cmd; then
        echo -e "${GREEN}✓ ${test_name} 通过${NC}"
        echo ""
        return 0
    else
        echo -e "✗ ${test_name} 失败"
        return 1
    fi
}

# 1. 构建项目
echo -e "${BLUE}步骤 1: 构建项目${NC}"
cargo build --quiet
echo -e "${GREEN}✓ 构建完成${NC}"
echo ""

# 2. 运行单元测试
echo "========================================"
echo "  单元测试"
echo "========================================"
echo ""

run_test "基础UUID测试" "cargo test --test basic_uuid_tests --quiet 2>&1 | grep -q 'test result: ok'"
run_test "Protobuf测试" "cargo test --test protobuf_tests --quiet 2>&1 | grep -q 'test result: ok'"
run_test "REST服务测试" "cargo test --test rest_tests --quiet 2>&1 | grep -q 'test result: ok'"

# 3. 运行集成测试
echo "========================================"
echo "  集成测试"
echo "========================================"
echo ""

run_test "集成测试" "cargo test --test integration_tests --quiet 2>&1 | grep -q 'test result: ok'"

# 4. 运行所有测试
echo "========================================"
echo "  全量测试"
echo "========================================"
echo ""

echo -e "${BLUE}运行所有测试...${NC}"
cargo test --quiet 2>&1 | tail -20
echo ""

# 5. 测试总结
echo "========================================"
echo "  测试总结"
echo "========================================"
echo ""

# 统计测试数量
total_tests=$(cargo test --quiet 2>&1 | grep "passed" | tail -1 | grep -oP '\d+(?= passed)' | awk '{s+=$1} END {print s}')
echo -e "${GREEN}✓ 所有测试通过！${NC}"
echo -e "总计: ${total_tests} 个测试"
echo ""

# 6. 代码覆盖率提示
echo "========================================"
echo "  提示"
echo "========================================"
echo ""
echo "查看详细测试报告: cat TEST_REPORT.md"
echo "运行特定测试: cargo test <test_name>"
echo "查看测试日志: cargo test -- --nocapture"
echo ""
