#!/usr/bin/env python3
"""
lsql 客户端工具 SQL 执行测试 - 使用 lsql 命令行工具测试所有 SQL 功能
"""
import subprocess
import time
import sys
import os
import signal
import tempfile
import re
import requests
import json

TEST_DB = "./laoflch_db_lsql_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
LSQL_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "lsql")
REST_PORT = "8080"
REST_BASE_URL = f"http://127.0.0.1:{REST_PORT}"

TOKEN = None

def login():
    global TOKEN
    print("    [认证] 用户登录...")
    try:
        payload = {
            "username": "admin",
            "password": "laoflchdb"
        }
        resp = requests.post(f"{REST_BASE_URL}/api/v1/login", json=payload)
        data = resp.json()
        if data.get("success") and data["data"].get("success"):
            TOKEN = data["data"]["token"]
            print(f"    ✓ 登录成功，Token: {TOKEN[:20]}...")
            return True
        else:
            print(f"    ✗ 登录失败: {data}")
            return False
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False

def get_auth_headers():
    if TOKEN:
        return {"Authorization": f"Bearer {TOKEN}"}
    return {}

def create_table_via_api(schema, table_name, columns):
    """通过 REST API 创建表"""
    payload = {
        "schema": schema,
        "table_name": table_name,
        "columns": columns
    }
    resp = requests.post(f"{REST_BASE_URL}/api/v1/tables", json=payload, headers=get_auth_headers())
    return resp.json()

def insert_row_via_api(schema, table_name, row_id, data):
    """通过 REST API 插入数据"""
    payload = {
        "row_id": row_id,
        "row": {
            "row_type": 0,
            "version": 1,
            "data": data
        }
    }
    resp = requests.post(f"{REST_BASE_URL}/api/v1/schemas/{schema}/tables/{table_name}/rows", json=payload, headers=get_auth_headers())
    return resp.json()

def run_lsql_command(sql, host=TEST_ADDR, schema="sys"):
    """运行 lsql 命令行工具执行 SQL 并返回结果"""
    cmd = [
        LSQL_BIN,
        "--host", host,
        "--schema", schema,
        "--command", sql
    ]
    
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=os.path.dirname(os.path.abspath(__file__))
    )
    
    return {
        "returncode": result.returncode,
        "stdout": result.stdout,
        "stderr": result.stderr
    }

def run_lsql_test():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))
    
    print("=" * 70)
    print("Python 自动回归测试: lsql 客户端 SQL 执行")
    print("=" * 70)
    
    # 检查二进制文件是否存在
    print("\n[0/10] 检查二进制文件...")
    if not os.path.exists(SERVER_BIN):
        print(f"    ✗ 服务端二进制文件不存在: {SERVER_BIN}")
        return 1
    if not os.path.exists(LSQL_BIN):
        print(f"    ✗ lsql 二进制文件不存在: {LSQL_BIN}")
        return 1
    print("    ✓ 二进制文件检查通过")
    
    print("\n[1/10] 清理旧测试数据...")
    subprocess.run(["rm", "-rf", TEST_DB], capture_output=True)
    print("    ✓ 清理完成")
    
    print("\n[2/10] 初始化数据库...")
    init_cmd = [SERVER_BIN, "init", "--db-path", TEST_DB]
    result = subprocess.run(init_cmd, capture_output=True, text=True, cwd="..")
    if result.returncode != 0:
        print(f"    ✗ 初始化失败: {result.stderr}")
        return 1
    print("    ✓ 数据库初始化完成")
    
    print("\n[3/10] 启动 laoflchDB gRPC 服务后台进程...")
    cmd = [
        SERVER_BIN,
        "start",
        "--addr", TEST_ADDR,
        "--db-path", TEST_DB
    ]
    server_proc = subprocess.Popen(
        cmd,
        cwd="..",
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        preexec_fn=os.setsid
    )
    time.sleep(3)
    print(f"    ✓ 服务已启动 PID={server_proc.pid} 监听 {TEST_ADDR}")
    
    try:
        print("\n[4/10] 等待服务就绪...")
        time.sleep(2)
        print("    ✓ 服务就绪")
        
        print("\n[5/10] 用户登录...")
        if not login():
            return 1
        print("    ✓ 用户登录成功")
        
        print("\n[6/10] 测试 lsql 基本连接...")
        result = run_lsql_command("SELECT 1")
        if result["returncode"] != 0:
            print(f"    ✗ 连接失败: {result['stderr']}")
            return 1
        print("    ✓ 基本连接测试通过")
        
        print("\n[7/10] 通过 API 创建表和插入数据...")
        # 通过 API 创建第一个表
        columns1 = [
            {"name": "id", "column_type": "INT64"},
            {"name": "name", "column_type": "STRING"},
            {"name": "age", "column_type": "INT64"},
            {"name": "score", "column_type": "FLOAT"}
        ]
        create_result = create_table_via_api("sys", "test_users", columns1)
        assert create_result.get("success", False) or "already exists" in create_result.get("message", ""), f"创建表失败: {create_result}"
        print("    ✓ 创建表 test_users 成功")
        
        # 通过 API 插入数据
        test_data = [
            ["1", "Alice", "25", "95.5"],
            ["2", "Bob", "30", "88.0"],
            ["3", "Charlie", "28", "92.3"]
        ]
        
        for i, data in enumerate(test_data, 1):
            insert_result = insert_row_via_api("sys", "test_users", i, data)
            assert insert_result.get("success", False), f"插入数据失败: {insert_result}"
        
        time.sleep(1)
        print("    ✓ 数据插入成功")
        
        print("\n[7/10] 测试基本 SELECT 查询...")
        select_tests = [
            ("SELECT * FROM test_users", True, "全表查询"),
            ("SELECT name, age FROM test_users", True, "指定列查询"),
            ("SELECT * FROM test_users WHERE age > 25", True, "WHERE 条件查询"),
            ("SELECT * FROM test_users ORDER BY age", True, "ORDER BY 排序"),
            ("SELECT * FROM test_users LIMIT 2", True, "LIMIT 限制"),
        ]
        
        for sql, should_succeed, desc in select_tests:
            result = run_lsql_command(sql)
            if should_succeed:
                assert result["returncode"] == 0, f"{desc} 失败: {result['stderr']}"
                print(f"    ✓ {desc} 成功")
            else:
                pass
        
        print("\n[8/10] 测试多表 JOIN（通过 API 创建第二个表）...")
        # 通过 API 创建第二个表
        columns2 = [
            {"name": "order_id", "column_type": "INT64"},
            {"name": "user_id", "column_type": "INT64"},
            {"name": "amount", "column_type": "FLOAT"}
        ]
        create_result2 = create_table_via_api("sys", "test_orders", columns2)
        assert create_result2.get("success", False) or "already exists" in create_result2.get("message", ""), f"创建表失败: {create_result2}"
        
        # 插入订单数据
        order_data = [
            ["101", "1", "100.5"],
            ["102", "1", "200.75"],
            ["103", "2", "150.0"]
        ]
        
        for i, data in enumerate(order_data, 1):
            insert_row_via_api("sys", "test_orders", i, data)
        
        time.sleep(1)
        
        join_tests = [
            (
                "SELECT u.name, o.amount FROM test_users u INNER JOIN test_orders o ON u.id = o.user_id",
                "INNER JOIN"
            ),
            (
                "SELECT u.name, o.amount FROM test_users u LEFT JOIN test_orders o ON u.id = o.user_id",
                "LEFT JOIN"
            ),
        ]
        
        for sql, desc in join_tests:
            result = run_lsql_command(sql)
            assert result["returncode"] == 0, f"{desc} 失败: {result['stderr']}"
            print(f"    ✓ {desc} 成功")
        
        print("\n[9/9] 测试更多 SQL 功能...")
        more_tests = [
            ("SELECT * FROM user", "查询 sys.user 表"),
            ("SELECT name AS username FROM test_users", "列别名"),
            ("SELECT id, name FROM test_users WHERE id IN (1, 2)", "IN 条件"),
        ]
        
        for sql, desc in more_tests:
            result = run_lsql_command(sql)
            assert result["returncode"] == 0, f"{desc} 失败: {result['stderr']}"
            print(f"    ✓ {desc} 成功")
        
        print("\n" + "=" * 70)
        print("SUCCESS! lsql 客户端 SQL 执行测试全部通过")
        print("=" * 70)
        print(f"数据保留在: {TEST_DB}")
        
        return 0
    
    except Exception as e:
        print(f"\n    ✗ 测试失败: {type(e).__name__}: {e}")
        import traceback
        traceback.print_exc()
        return 1
    finally:
        print("\n--- 终止服务进程 ---")
        try:
            os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            server_proc.wait(timeout=3)
        except:
            try:
                server_proc.kill()
            except:
                pass
        print(f"数据保留在: {TEST_DB}")

if __name__ == "__main__":
    sys.exit(run_lsql_test())
