#!/bin/bash
# lsql 客户端测试脚本

set -e

LSQL_PATH="./target/debug/lsql"

echo "======================================"
echo "测试 lsql 客户端"
echo "======================================"
echo ""

echo "1. 测试帮助信息..."
"$LSQL_PATH" --help
echo ""

echo "2. 测试列出所有 Schema..."
echo -e "\\dn\\n\\q" | "$LSQL_PATH" 2>&1
echo ""

echo "3. 测试切换到 example schema 并列出表..."
echo -e "\\dt\\n\\q" | "$LSQL_PATH" 2>&1
echo ""

echo "4. 测试执行简单 SQL 查询 (SELECT * FROM users)..."
echo -e "SELECT * FROM users LIMIT 3;\\n\\q" | "$LSQL_PATH" 2>&1
echo ""

echo "5. 测试切换到 sys schema..."
echo -e "\\c sys\\n\\dn\\n\\q" | "$LSQL_PATH" 2>&1
echo ""

echo "======================================"
echo "lsql 测试完成！"
echo "======================================"
