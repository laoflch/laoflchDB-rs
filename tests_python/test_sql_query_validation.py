#!/usr/bin/env python3
"""
SQL 查询验证测试 - 验证数据插入和查询返回的数据类型正确性
"""
import requests
import json
import sys
import os

PORT = os.environ.get("LAOFLCHDB_REST_PORT", "8080")
BASE_URL = f"http://127.0.0.1:{PORT}"

TABLE_NAME = "test_sql_validation"

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
        assert data["success"] == True, f"登录失败: {data}"
        assert data["data"]["success"] == True, f"登录数据失败: {data}"
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
        assert data["success"] == True, f"健康检查失败: {data}"
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
                {"name": "id", "column_type": "INT64"},
                {"name": "name", "column_type": "STRING"},
                {"name": "age", "column_type": "INT64"},
                {"name": "score", "column_type": "FLOAT"}
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

def test_insert_data():
    print("[测试] 插入测试数据...")
    try:
        # 插入第一条数据
        payload1 = {
            "row_id": 1,
            "row": {
                "row_type": 0,
                "version": 1,
                "data": ["1", "Alice", "30", "95.5"]
            }
        }
        resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}/rows", json=payload1, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"插入数据失败: {data}"
        
        # 插入第二条数据
        payload2 = {
            "row_id": 2,
            "row": {
                "row_type": 0,
                "version": 1,
                "data": ["2", "Bob", "25", "88.0"]
            }
        }
        resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}/rows", json=payload2, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"插入数据失败: {data}"
        
        # 插入第三条数据
        payload3 = {
            "row_id": 3,
            "row": {
                "row_type": 0,
                "version": 1,
                "data": ["3", "Charlie", "35", "92.5"]
            }
        }
        resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}/rows", json=payload3, headers=get_auth_headers())
        data = resp.json()
        assert data["success"] == True, f"插入数据失败: {data}"
        
        print("    ✓ 插入数据成功")
        return True
    except Exception as e:
        print(f"    ✗ 插入数据失败: {e}")
        return False

def test_sql_query_full():
    print("[测试] SQL 查询 - 全表查询...")
    try:
        payload = {
            "sql": f"SELECT * FROM {TABLE_NAME}"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        assert "columns" in data["data"], "缺少 columns 字段"
        assert "rows" in data["data"], "缺少 rows 字段"
        
        columns = data["data"]["columns"]
        rows = data["data"]["rows"]
        
        # 验证列名
        expected_columns = ["id", "name", "age", "score"]
        assert columns == expected_columns, f"列名不匹配: {columns} != {expected_columns}"
        
        # 验证行数
        assert len(rows) == 3, f"行数不匹配: {len(rows)} != 3"
        
        # 验证数据类型
        for row in rows:
            assert isinstance(row[0], int), f"id 应为整数，实际为 {type(row[0])}: {row[0]}"
            assert isinstance(row[1], str), f"name 应为字符串，实际为 {type(row[1])}: {row[1]}"
            assert isinstance(row[2], int), f"age 应为整数，实际为 {type(row[2])}: {row[2]}"
            assert isinstance(row[3], (int, float)), f"score 应为数字，实际为 {type(row[3])}: {row[3]}"
        
        print(f"    ✓ 全表查询成功，返回 {len(rows)} 条记录")
        print(f"    ✓ 列名: {columns}")
        print(f"    ✓ 数据类型正确")
        return True
    except Exception as e:
        print(f"    ✗ SQL 查询失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_projection():
    print("[测试] SQL 查询 - 投影下推...")
    try:
        payload = {
            "sql": f"SELECT id, name FROM {TABLE_NAME}"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        columns = data["data"]["columns"]
        rows = data["data"]["rows"]
        
        # 验证投影只返回指定列
        expected_columns = ["id", "name"]
        assert columns == expected_columns, f"列名不匹配: {columns} != {expected_columns}"
        
        # 验证每行只有 2 列
        for row in rows:
            assert len(row) == 2, f"每行应只有 2 列，实际有 {len(row)} 列"
            assert isinstance(row[0], int), f"id 应为整数"
            assert isinstance(row[1], str), f"name 应为字符串"
        
        print(f"    ✓ 投影下推成功")
        return True
    except Exception as e:
        print(f"    ✗ 投影下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_filter():
    print("[测试] SQL 查询 - 谓词下推...")
    try:
        # 测试 age > 30
        payload = {
            "sql": f"SELECT name, age FROM {TABLE_NAME} WHERE age > 30"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 1, f"应返回 1 条记录 (Charlie, age=35)，实际返回 {len(rows)} 条"
        
        # 验证返回的数据都满足 age > 30
        for row in rows:
            name, age = row[0], row[1]
            assert age > 30, f"age 应大于 30，实际为 {age}"
            assert isinstance(name, str), f"name 应为字符串"
            assert isinstance(age, int), f"age 应为整数"
        
        print(f"    ✓ 谓词下推 (age > 30) 成功，返回 {len(rows)} 条记录")
        
        # 测试 age >= 30
        payload = {
            "sql": f"SELECT name, age FROM {TABLE_NAME} WHERE age >= 30"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 2, f"应返回 2 条记录 (Alice:30, Charlie:35)，实际返回 {len(rows)} 条"
        
        print(f"    ✓ 谓词下推 (age >= 30) 成功，返回 {len(rows)} 条记录")
        
        # 测试 age = 30
        payload = {
            "sql": f"SELECT name, age FROM {TABLE_NAME} WHERE age = 30"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 1, f"应返回 1 条记录，实际返回 {len(rows)} 条"
        assert rows[0][0] == "Alice", f"name 应为 Alice，实际为 {rows[0][0]}"
        assert rows[0][1] == 30, f"age 应为 30，实际为 {rows[0][1]}"
        
        print(f"    ✓ 谓词下推 (age = 30) 成功")
        
        return True
    except Exception as e:
        print(f"    ✗ 谓词下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_filter_or():
    print("[测试] SQL 查询 - 同一列 OR 条件...")
    try:
        # 测试同一列的 OR 条件 (age < 30 OR age > 40)
        payload = {
            "sql": f"SELECT name, age FROM {TABLE_NAME} WHERE age < 30 OR age > 40"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 1, f"应返回 1 条记录 (Bob, age=25)，实际返回 {len(rows)} 条"
        assert rows[0][0] == "Bob", f"name 应为 Bob，实际为 {rows[0][0]}"
        assert rows[0][1] == 25, f"age 应为 25，实际为 {rows[0][1]}"
        
        print(f"    ✓ 同一列 OR (age < 30 OR age > 40) 成功")
        
        # 测试同一列的多个 OR 条件 (age = 25 OR age = 30 OR age = 35)
        payload = {
            "sql": f"SELECT name, age FROM {TABLE_NAME} WHERE age = 25 OR age = 30 OR age = 35"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 3, f"应返回 3 条记录，实际返回 {len(rows)} 条"
        
        # 验证所有返回的记录都满足条件
        ages = {row[1] for row in rows}
        assert ages == {25, 30, 35}, f"age 值不正确: {ages}"
        
        print(f"    ✓ 同一列多个 OR (age = 25 OR age = 30 OR age = 35) 成功")
        
        return True
    except Exception as e:
        print(f"    ✗ OR 条件测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_filter_and():
    print("[测试] SQL 查询 - AND 条件...")
    try:
        # 测试 AND 条件 (age > 25 AND score > 90)
        payload = {
            "sql": f"SELECT name, age, score FROM {TABLE_NAME} WHERE age > 25 AND score > 90"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 2, f"应返回 2 条记录 (Alice:95.5, Charlie:92.5)，实际返回 {len(rows)} 条"
        
        # 验证所有返回的记录都满足条件
        for row in rows:
            name, age, score = row[0], row[1], row[2]
            assert age > 25, f"age 应大于 25，实际为 {age}"
            assert score > 90, f"score 应大于 90，实际为 {score}"
        
        print(f"    ✓ AND 条件 (age > 25 AND score > 90) 成功")
        
        return True
    except Exception as e:
        print(f"    ✗ AND 条件测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_filter_combined_logic():
    print("[测试] SQL 查询 - 组合逻辑表达式...")
    try:
        # 测试组合逻辑 ((age > 25 AND age < 40) OR score > 92)
        payload = {
            "sql": f"SELECT name, age, score FROM {TABLE_NAME} WHERE (age > 25 AND age < 40) OR score > 92"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 2, f"应返回 2 条记录，实际返回 {len(rows)} 条"
        
        print(f"    ✓ 组合逻辑 ((age > 25 AND age < 40) OR score > 92) 成功")
        
        return True
    except Exception as e:
        print(f"    ✗ 组合逻辑测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_limit():
    print("[测试] SQL 查询 - Limit 下推...")
    try:
        payload = {
            "sql": f"SELECT * FROM {TABLE_NAME} LIMIT 2"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 2, f"应返回 2 条记录，实际返回 {len(rows)} 条"
        
        print(f"    ✓ Limit 下推成功")
        return True
    except Exception as e:
        print(f"    ✗ Limit 下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_combined():
    print("[测试] SQL 查询 - 组合查询...")
    try:
        payload = {
            "sql": f"SELECT name, score FROM {TABLE_NAME} WHERE age > 25 ORDER BY score DESC LIMIT 2"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        columns = data["data"]["columns"]
        
        # 验证列名
        assert columns == ["name", "score"], f"列名不匹配: {columns}"
        
        # 验证数据类型
        for row in rows:
            assert isinstance(row[0], str), f"name 应为字符串"
            assert isinstance(row[1], (int, float)), f"score 应为数字"
        
        print(f"    ✓ 组合查询成功")
        return True
    except Exception as e:
        print(f"    ✗ 组合查询测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_data_type_validation():
    print("[测试] 数据类型精确验证...")
    try:
        payload = {
            "sql": f"SELECT * FROM {TABLE_NAME} WHERE id = 1"
        }
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json=payload, headers=get_auth_headers())
        data = resp.json()
        
        assert data["success"] == True, f"查询失败: {data}"
        
        rows = data["data"]["rows"]
        assert len(rows) == 1, f"应返回 1 条记录"
        
        row = rows[0]
        id_val, name_val, age_val, score_val = row
        
        # 精确数据类型验证
        assert type(id_val) == int, f"id 应为 int，实际为 {type(id_val)}"
        assert type(name_val) == str, f"name 应为 str，实际为 {type(name_val)}"
        assert type(age_val) == int, f"age 应为 int，实际为 {type(age_val)}"
        assert type(score_val) == float or type(score_val) == int, f"score 应为 float 或 int，实际为 {type(score_val)}"
        
        # 值验证
        assert id_val == 1, f"id 应为 1，实际为 {id_val}"
        assert name_val == "Alice", f"name 应为 Alice，实际为 {name_val}"
        assert age_val == 30, f"age 应为 30，实际为 {age_val}"
        assert score_val == 95.5, f"score 应为 95.5，实际为 {score_val}"
        
        print(f"    ✓ 数据类型精确验证通过")
        print(f"    ✓ id={id_val} (type={type(id_val).__name__})")
        print(f"    ✓ name={name_val} (type={type(name_val).__name__})")
        print(f"    ✓ age={age_val} (type={type(age_val).__name__})")
        print(f"    ✓ score={score_val} (type={type(score_val).__name__})")
        return True
    except Exception as e:
        print(f"    ✗ 数据类型验证失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def main():
    print("=" * 70)
    print("Python 自动回归测试: SQL 查询数据类型验证")
    print(f"目标端口: {PORT}")
    print("=" * 70)
    print()

    tests = [
        ("健康检查", test_health),
        ("用户登录", test_login),
        ("创建表", test_create_table),
        ("插入数据", test_insert_data),
        ("全表查询", test_sql_query_full),
        ("投影下推", test_sql_query_projection),
        ("谓词下推", test_sql_query_filter),
        ("OR 条件", test_sql_query_filter_or),
        ("AND 条件", test_sql_query_filter_and),
        ("组合逻辑", test_sql_query_filter_combined_logic),
        ("Limit 下推", test_sql_query_limit),
        ("组合查询", test_sql_query_combined),
        ("数据类型验证", test_data_type_validation),
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
            import traceback
            traceback.print_exc()
            failed += 1
        print()

    cleanup_table()

    print("=" * 70)
    print(f"测试结果: {passed} 通过, {failed} 失败")
    print("=" * 70)

    if failed > 0:
        sys.exit(1)
    else:
        print("✓ 所有 SQL 查询测试通过！")
        sys.exit(0)

if __name__ == "__main__":
    main()
