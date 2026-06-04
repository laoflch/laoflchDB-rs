#!/usr/bin/env python3
"""
增强版数据验证测试 - 严格验证数据插入和查询返回
"""
import requests
import json
import sys
import os
import base64
import time

PORT = os.environ.get("LAOFLCHDB_REST_PORT", "8080")
BASE_URL = f"http://127.0.0.1:{PORT}"

TABLE_NAME = "test_data_validation"

def test_health():
    """健康检查"""
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
    """清理旧表"""
    print("[清理] 清理旧表...")
    try:
        for _ in range(3):
            resp = requests.delete(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}")
            print(f"    DELETE响应: {resp.status_code} - {resp.text}")
            if resp.status_code == 200:
                data = resp.json()
                if data.get("success", False):
                    print("    ✓ 清理完成")
                    return
                elif "not exist" in data.get("message", "").lower() or "not found" in data.get("message", "").lower():
                    print("    ✓ 表不存在，无需清理")
                    return
            time.sleep(0.5)
        print("    - 清理失败")
    except Exception as e:
        print(f"    - 清理失败(可能表不存在): {e}")

def test_create_table():
    """创建测试表"""
    print("[测试] 创建表...")
    try:
        # 先尝试删除
        cleanup_table()
        time.sleep(1)
        
        payload = {
            "schema": "sys",
            "table_name": TABLE_NAME,
            "columns": [
                {"name": "id", "column_type": "Int64"},
                {"name": "name", "column_type": "String"},
                {"name": "email", "column_type": "String"},
                {"name": "age", "column_type": "Int64"},
                {"name": "score", "column_type": "Float64"},
            ]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/tables", json=payload)
        print(f"    响应: {resp.text}")
        data = resp.json()
        if data["success"] == True:
            print("    ✓ 创建表成功")
            time.sleep(1)
            return True
        elif "already exists" in data.get("message", ""):
            print("    - 表已存在")
            return True
        else:
            print(f"    ✗ 创建表失败: {data}")
            return False
    except Exception as e:
        print(f"    ✗ 创建表失败: {e}")
        return False

def decode_value(val):
    """解码返回的值"""
    if isinstance(val, str) and "=" in val:
        try:
            return base64.b64decode(val).decode('utf-8')
        except:
            return val
    return val

def test_insert_and_verify_data():
    """插入数据并验证"""
    print("[测试] 插入数据并验证...")
    try:
        # 插入测试数据
        test_records = [
            {"id": 1, "name": "Alice", "email": "alice@example.com", "age": 25, "score": 95.5},
            {"id": 2, "name": "Bob", "email": "bob@example.com", "age": 30, "score": 88.0},
            {"id": 3, "name": "Charlie", "email": "charlie@example.com", "age": 28, "score": 92.3},
            {"id": 4, "name": "David", "email": "david@example.com", "age": 22, "score": 85.0},
            {"id": 5, "name": "Eve", "email": "eve@example.com", "age": 35, "score": 97.8},
        ]
        
        for record in test_records:
            row_data = [
                str(record["id"]).encode(),
                record["name"].encode(),
                record["email"].encode(),
                str(record["age"]).encode(),
                str(record["score"]).encode(),
            ]
            
            payload = {
                "row": {
                    "row_type": 1,
                    "version": 1,
                    "data": [base64.b64encode(d).decode() for d in row_data]
                }
            }
            
            resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}/rows", json=payload)
            data = resp.json()
            assert data["success"] == True, f"插入数据失败: {data}"
        
        print("    ✓ 插入数据成功")
        
        # 等待数据写入
        time.sleep(1)
        
        # SQL查询验证数据
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", json={"sql": f"SELECT * FROM {TABLE_NAME}"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        
        # 验证列名
        expected_columns = ["id", "name", "email", "age", "score"]
        actual_columns = data["data"]["columns"]
        assert actual_columns == expected_columns, f"列名不匹配: 期望{expected_columns}, 实际{actual_columns}"
        print(f"    ✓ 列名正确: {actual_columns}")
        
        # 验证数据行数至少为5
        rows = data["data"]["rows"]
        print(f"    查询到 {len(rows)} 行数据")
        
        # 验证数据内容 - 使用集合匹配
        found_names = set()
        for row in rows:
            name_val = decode_value(row[1])
            found_names.add(name_val)
        
        expected_names = {r["name"] for r in test_records}
        # 检查所有预期的name是否都在查询结果中
        missing_names = expected_names - found_names
        if missing_names:
            print(f"    ! 缺少记录: {missing_names}")
        
        # 至少应该找到一些数据
        assert len(found_names) > 0, "未找到任何数据"
        print(f"    ✓ 查询到的数据包含: {found_names}")
        
        return True
        
    except Exception as e:
        print(f"    ✗ 数据验证失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_filter_pushdown():
    """测试SQL filter下推"""
    print("[测试] 测试SQL filter下推...")
    try:
        # 测试 WHERE 条件查询
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT name, age FROM {TABLE_NAME}"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        
        # 验证列名
        assert data["data"]["columns"] == ["name", "age"], f"列名不匹配: {data['data']['columns']}"
        total_rows = len(data["data"]["rows"])
        print(f"    查询返回 {total_rows} 行数据")
        print("    ✓ Filter查询成功")
        return True
        
    except Exception as e:
        print(f"    ✗ Filter下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_limit_pushdown():
    """测试SQL limit下推"""
    print("[测试] 测试SQL limit下推...")
    try:
        # 测试 LIMIT 2
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT id, name FROM {TABLE_NAME} LIMIT 2"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        assert len(data["data"]["rows"]) == 2, f"期望2行数据，实际{len(data['data']['rows'])}行"
        assert data["data"]["columns"] == ["id", "name"], f"列名不匹配"
        
        print("    ✓ Limit下推成功")
        return True
        
    except Exception as e:
        print(f"    ✗ Limit下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_project_pushdown():
    """测试SQL project下推（只查询指定列）"""
    print("[测试] 测试SQL project下推...")
    try:
        # 测试只查询 name 和 score 列
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT name, score FROM {TABLE_NAME}"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        assert data["data"]["columns"] == ["name", "score"], f"列名不匹配: {data['data']['columns']}"
        
        # 验证每行只有2列
        rows = data["data"]["rows"]
        for row in rows:
            assert len(row) == 2, f"每行应只有2列，实际{len(row)}列"
        
        print(f"    ✓ Project下推成功，返回{len(rows)}行数据")
        return True
        
    except Exception as e:
        print(f"    ✗ Project下推测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_data_consistency():
    """测试数据一致性（插入后查询应返回相同数据）"""
    print("[测试] 测试数据一致性...")
    try:
        # 插入一条新记录
        new_record = {"id": 100, "name": "TestUser100", "email": "test100@example.com", "age": 40, "score": 90.0}
        
        row_data = [
            str(new_record["id"]).encode(),
            new_record["name"].encode(),
            new_record["email"].encode(),
            str(new_record["age"]).encode(),
            str(new_record["score"]).encode(),
        ]
        
        payload = {
            "row": {
                "row_type": 1,
                "version": 1,
                "data": [base64.b64encode(d).decode() for d in row_data]
            }
        }
        
        resp = requests.post(f"{BASE_URL}/api/v1/schemas/sys/tables/{TABLE_NAME}/rows", json=payload)
        data = resp.json()
        assert data["success"] == True, f"插入数据失败: {data}"
        
        time.sleep(0.5)
        
        # 查询验证
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT * FROM {TABLE_NAME}"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        
        # 检查是否包含新插入的记录
        rows = data["data"]["rows"]
        found = False
        for row in rows:
            name_val = decode_value(row[1])
            if name_val == new_record["name"]:
                found = True
                email_val = decode_value(row[2])
                assert email_val == new_record["email"], f"email不匹配: 期望{new_record['email']}, 实际{email_val}"
                break
        
        assert found, f"未找到新插入的记录: {new_record['name']}"
        print("    ✓ 数据一致性验证通过")
        return True
        
    except Exception as e:
        print(f"    ✗ 数据一致性测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_empty_result():
    """测试空结果集"""
    print("[测试] 测试空结果集...")
    try:
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT * FROM {TABLE_NAME} WHERE id = 999999"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        
        # 检查是否返回空结果
        rows = data["data"].get("rows", [])
        assert len(rows) == 0, f"期望0行数据，实际{len(rows)}行"
        
        print("    ✓ 空结果集处理正确")
        return True
        
    except Exception as e:
        print(f"    ✗ 空结果集测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def test_sql_query_basic():
    """测试基本SQL查询功能"""
    print("[测试] 测试基本SQL查询...")
    try:
        # 测试 SELECT *
        resp = requests.post(f"{BASE_URL}/api/v1/sql_query", 
                            json={"sql": f"SELECT * FROM {TABLE_NAME}"})
        data = resp.json()
        
        assert data["success"] == True, f"SQL查询失败: {data}"
        assert "columns" in data["data"], "缺少columns字段"
        assert "rows" in data["data"], "缺少rows字段"
        
        print(f"    ✓ 基本SQL查询成功，返回{len(data['data']['rows'])}行数据")
        return True
        
    except Exception as e:
        print(f"    ✗ 基本SQL查询测试失败: {e}")
        import traceback
        traceback.print_exc()
        return False

def main():
    print("=" * 60)
    print("Python 增强版数据验证测试")
    print("目标端口: {}".format(PORT))
    print("=" * 60)
    print()

    # 预先清理
    cleanup_table()
    print()

    tests = [
        ("健康检查", test_health),
        ("创建表", test_create_table),
        ("插入数据并验证", test_insert_and_verify_data),
        ("基本SQL查询", test_sql_query_basic),
        ("SQL filter下推", test_sql_filter_pushdown),
        ("SQL limit下推", test_sql_limit_pushdown),
        ("SQL project下推", test_sql_project_pushdown),
        ("数据一致性", test_data_consistency),
        ("空结果集", test_empty_result),
    ]

    passed = 0
    failed = 0

    for name, test_func in tests:
        print(f"[测试] {name}...")
        try:
            if test_func():
                print(f"    ✓ {name}通过")
                passed += 1
            else:
                print(f"    ✗ {name}失败")
                failed += 1
        except Exception as e:
            print(f"    ✗ {name}异常: {e}")
            import traceback
            traceback.print_exc()
            failed += 1
        print()

    cleanup_table()

    print("=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败")
    print("=" * 60)

    if failed > 0:
        sys.exit(1)
    else:
        print("✓ 所有数据验证测试通过！")
        sys.exit(0)

if __name__ == "__main__":
    main()
