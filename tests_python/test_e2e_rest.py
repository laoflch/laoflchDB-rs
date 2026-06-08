#!/usr/bin/env python3
import requests
import json
import sys
import os

PORT = os.environ.get("LAOFLCHDB_REST_PORT", "38080")
BASE_URL = f"http://127.0.0.1:{PORT}"

TABLE_NAME = "test_rest_api"

TOKEN = None

def test_login():
    global TOKEN
    print("[测试] 用户登录...")
    try:
        payload = {
            "username": "admin",
            "password": "laoflchdb"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/login", json=payload)
        data = resp.json()
        assert data["success"] == True, f"Login failed: {data}"
        assert data["data"]["success"] == True, f"Login data failed: {data}"
        TOKEN = data["data"]["token"]
        print(f"    ✓ 登录成功，Token: {TOKEN[:20]}...")
        return True
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False

def get_auth_headers():
    if TOKEN:
        return {"Authorization": f"Bearer {TOKEN}"}
    return {}

def test_health():
    print("[测试] 健康检查...")
    try:
        resp = requests.get(f"{BASE_URL}/health")
        data = resp.json()
        assert data["success"] == True, f"Health check failed: {data}"
        print("    ✓ 健康检查通过")
        return True
    except Exception as e:
        print(f"    ✗ 健康检查失败: {e}")
        return False

def cleanup_table():
    print("[清理] 清理旧表...")
    try:
        resp = requests.delete(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}", headers=get_auth_headers())
        print("    ✓ 清理完成")
    except Exception as e:
        print(f"    - 清理失败(可能表不存在): {e}")

def test_create_table():
    print("[测试] 创建表...")
    try:
        payload = {
            "schema": "sys",
            "table_name": TABLE_NAME,
            "columns": [
                {"name": "id", "column_type": "Int64"},
                {"name": "name", "column_type": "String"},
                {"name": "email", "column_type": "String"}
            ]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/tables", json=payload, headers=get_auth_headers())
        data = resp.json()
        if data["success"] == True:
            print("    ✓ 创建表成功")
            return True
        elif "already exists" in data.get("message", ""):
            print("    - 表已存在，跳过创建")
            return True
        else:
            print(f"    ✗ 创建表失败: {data}")
            return False
    except Exception as e:
        print(f"    ✗ 创建表失败: {e}")
        return False

def test_list_tables():
    print("[测试] 列出表...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/schemas/sys/tables", headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"List tables failed: {data}"
        assert TABLE_NAME in data["data"], f"{TABLE_NAME} not found"
        print("    ✓ 列出表成功")
        return True
    except Exception as e:
        print(f"    ✗ 列出表失败: {e}")
        return False

def test_get_table_meta():
    print("[测试] 获取表元数据...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}", headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Get table meta failed: {data}"
        assert data["data"]["table_name"] == TABLE_NAME
        assert data["data"]["column_count"] == 3
        print("    ✓ 获取表元数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 获取表元数据失败: {e}")
        return False

def test_put_data():
    print("[测试] 插入数据...")
    try:
        payload = {
            "schema": "sys",
            "table": TABLE_NAME,
            "key": "user_001",
            "value": '{"id":1,"name":"Alice","email":"alice@example.com"}'
        }
        resp = requests.post(f"{BASE_URL}/api/v1/put", json=payload, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Put data failed: {data}"
        print("    ✓ 插入数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 插入数据失败: {e}")
        return False

def test_get_data():
    print("[测试] 读取数据...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/get",
                           params={"schema": "sys", "table": TABLE_NAME, "key": "user_001"},
                           headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Get data failed: {data}"
        assert data["data"]["value"] is not None
        print("    ✓ 读取数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 读取数据失败: {e}")
        return False

def test_update_data():
    print("[测试] 更新数据...")
    try:
        payload = {
            "schema": "sys",
            "table": TABLE_NAME,
            "key": "user_001",
            "value": '{"id":1,"name":"Alice Updated","email":"alice.updated@example.com"}'
        }
        resp = requests.post(f"{BASE_URL}/api/v1/put", json=payload, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Update data failed: {data}"
        print("    ✓ 更新数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 更新数据失败: {e}")
        return False

def test_delete_data():
    print("[测试] 删除数据...")
    try:
        payload = {
            "schema": "sys",
            "table": TABLE_NAME,
            "key": "user_001"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/delete", json=payload, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Delete data failed: {data}"
        print("    ✓ 删除数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 删除数据失败: {e}")
        return False

def test_verify_delete():
    print("[测试] 验证删除...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/get",
                           params={"schema": "sys", "table": TABLE_NAME, "key": "user_001"},
                           headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Verify delete failed: {data}"
        assert data["data"]["value"] is None, "Data should be null after delete"
        print("    ✓ 验证删除成功")
        return True
    except Exception as e:
        print(f"    ✗ 验证删除失败: {e}")
        return False

def test_error_handling():
    print("[测试] 错误处理...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/get",
                           params={"schema": "sys", "table": "nonexistent", "key": "test"},
                           headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == False, "Should return error for nonexistent table"
        print("    ✓ 错误处理正常")
        return True
    except Exception as e:
        print(f"    ✗ 错误处理异常: {e}")
        return False

def test_sql_query():
    print("[测试] SQL 查询...")
    try:
        payload = {
            "schema": "sys",
            "sql": f"SELECT * FROM {TABLE_NAME}"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"SQL query failed: {data}"
        print("    ✓ SQL 查询成功")
        return True
    except Exception as e:
        print(f"    ✗ SQL 查询失败: {e}")
        return False

def test_logout():
    print("[测试] 用户登出...")
    try:
        payload = {
            "token": TOKEN
        }
        resp = requests.post(f"{BASE_URL}/api/v1/logout", json=payload, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"Logout failed: {data}"
        print("    ✓ 登出成功")
        return True
    except Exception as e:
        print(f"    ✗ 登出失败: {e}")
        return False

def test_unauthenticated_access():
    print("[测试] 未认证访问测试...")
    try:
        global TOKEN
        saved_token = TOKEN
        TOKEN = None
        resp = requests.get(f"{BASE_URL}/api/v1/schemas/sys/tables", headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == False, "Should fail without token"
        print("    ✓ 未认证访问被拒绝")
        TOKEN = saved_token
        return True
    except Exception as e:
        print(f"    ✗ 未认证访问测试异常: {e}")
        return False

def main():
    print("=" * 60)
    print("Python 自动回归测试: REST API 端到端验证")
    print(f"目标端口: {PORT}")
    print("=" * 60)
    print()

    cleanup_table()
    print()

    tests = [
        ("健康检查", test_health),
        ("用户登录", test_login),
        ("未认证访问", test_unauthenticated_access),
        ("创建表", test_create_table),
        ("列出表", test_list_tables),
        ("获取表元数据", test_get_table_meta),
        ("插入数据", test_put_data),
        ("读取数据", test_get_data),
        ("更新数据", test_update_data),
        ("删除数据", test_delete_data),
        ("验证删除", test_verify_delete),
        ("SQL 查询", test_sql_query),
        ("错误处理", test_error_handling),
        ("用户登出", test_logout),
    ]

    passed = 0
    failed = 0

    for name, test_func in tests:
        try:
            if test_func():
                passed += 1
            else:
                failed += 1
        except Exception as e:
            print(f"    ✗ 测试异常: {e}")
            failed += 1
        print()

    cleanup_table()

    print("=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败")
    print("=" * 60)

    if failed > 0:
        sys.exit(1)
    else:
        print("✓ 所有 REST API 测试通过！")
        sys.exit(0)

if __name__ == "__main__":
    main()
