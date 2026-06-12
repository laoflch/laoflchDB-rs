#!/usr/bin/env python3
"""
Python 自动回归测试: Index 全文本索引 REST API 测试
"""
import requests
import json
import sys
import os
import time

PORT = os.environ.get("LAOFLCHDB_REST_PORT", "8080")
BASE_URL = f"http://127.0.0.1:{PORT}"

INDEX_NAME = "test_py_index"
TOKEN = None

def test_login():
    global TOKEN
    print("[测试] 用户登录...")
    try:
        payload = {"username": "admin", "password": "laoflchdb"}
        resp = requests.post(f"{BASE_URL}/api/v1/login", json=payload, timeout=5)
        data = resp.json()
        assert data["success"] == True, f"Login failed: {data}"
        assert data["data"]["success"] == True
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
        resp = requests.get(f"{BASE_URL}/health", timeout=5)
        data = resp.json()
        assert data["success"] == True
        print("    ✓ 健康检查通过")
        return True
    except Exception as e:
        print(f"    ✗ 健康检查失败: {e}")
        return False

def test_create_index():
    print("[测试] 创建全文索引...")
    try:
        payload = {
            "index_name": INDEX_NAME,
            "fields": [
                {"name": "title", "field_type": "STRING", "comment": "标题"},
                {"name": "content", "field_type": "STRING", "comment": "内容"},
                {"name": "category", "field_type": "STRING", "comment": "分类"},
                {"name": "view_count", "field_type": "INT64", "comment": "浏览次数"},
            ]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/index/indices", json=payload, headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True, f"创建索引失败: {data}"
        assert data["data"]["index_id"] > 0
        print(f"    ✓ 索引 '{INDEX_NAME}' 创建成功，ID: {data['data']['index_id']}")
        return True
    except Exception as e:
        print(f"    ✗ 创建索引失败: {e}")
        return False

def test_create_duplicate_index():
    print("[测试] 创建同名索引（应成功，ID 不同）...")
    try:
        payload = {
            "index_name": INDEX_NAME,
            "fields": [{"name": "field1", "field_type": "STRING"}]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/index/indices", json=payload, headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        print(f"    ✓ 同名索引创建成功，新 ID: {data['data']['index_id']}")
        return True
    except Exception as e:
        print(f"    ✗ 创建同名索引失败: {e}")
        return False

def test_create_index_with_all_types():
    print("[测试] 创建包含多种字段类型的索引...")
    try:
        payload = {
            "index_name": "test_all_types_index",
            "fields": [
                {"name": "str_field", "field_type": "STRING"},
                {"name": "int_field", "field_type": "INT64"},
                {"name": "float_field", "field_type": "FLOAT"},
                {"name": "bytes_field", "field_type": "BYTES"},
            ]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/index/indices", json=payload, headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        print(f"    ✓ 多类型索引创建成功，ID: {data['data']['index_id']}")
        return True
    except Exception as e:
        print(f"    ✗ 创建多类型索引失败: {e}")
        return False

def test_list_indices():
    print("[测试] 列出所有索引...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/index/indices", headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        indices = data["data"]["indices"]
        assert INDEX_NAME in indices, f"索引列表应包含 '{INDEX_NAME}'，实际: {indices}"
        print(f"    ✓ 索引列表: {indices}")
        return True
    except Exception as e:
        print(f"    ✗ 列出索引失败: {e}")
        return False

def test_get_index_fields():
    print("[测试] 获取索引字段信息...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/fields", headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        fields = data["data"]
        assert len(fields) > 0, "字段列表不应为空"
        field_names = [f["column_name"] for f in fields]
        assert "title" in field_names, f"应包含 'title' 字段，实际: {field_names}"
        print(f"    ✓ 索引字段: {field_names}")
        return True
    except Exception as e:
        print(f"    ✗ 获取字段信息失败: {e}")
        return False

def test_get_index_meta():
    print("[测试] 获取索引元数据...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/meta", headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        meta = data["data"]
        assert meta["table_name"] == INDEX_NAME
        assert meta["column_count"] > 0
        print(f"    ✓ 索引元数据: name={meta['table_name']}, columns={meta['column_count']}, comment={meta['comment']}")
        return True
    except Exception as e:
        print(f"    ✗ 获取元数据失败: {e}")
        return False

def test_get_index_stats(num):
    print("[测试] 获取索引统计信息...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/index/stats", headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True
        stats = data["data"]
        assert stats["total_indices"] >= num  # 至少有两个索引
        assert len(stats["index_names"]) >= num
        print(f"    ✓ 索引统计: total={stats['total_indices']}, names={stats['index_names']}")
        return True
    except Exception as e:
        print(f"    ✗ 获取统计失败: {e}")
        return False

def test_search_no_auth():
    print("[测试] 未认证搜索请求（应返回 403）...")
    try:
        resp = requests.get(f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search?q=test", timeout=5)
        assert resp.status_code == 403, f"应返回 403，实际: {resp.status_code}"
        print(f"    ✓ 未认证请求被拒绝 (HTTP 403)")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def test_search_invalid_token():
    print("[测试] 无效 Token 搜索请求（应返回 403）...")
    try:
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search?q=test",
            headers={"Authorization": "Bearer invalid_token_xyz"},
            timeout=5
        )
        assert resp.status_code == 403, f"应返回 403，实际: {resp.status_code}"
        print(f"    ✓ 无效 Token 请求被拒绝 (HTTP 403)")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def test_search_empty_results():
    print("[测试] 搜索（占位符实现，应返回空结果）...")
    try:
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search?q=test&limit=10",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True
        results = data["data"]["results"]
        print(f"    ✓ 搜索结果: {len(results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 搜索失败: {e}")
        return False

def test_search_multi_field_empty():
    print("[测试] 多字段搜索（占位符实现，应返回空结果）...")
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search/multi",
            json={
                "field_queries": {"title": "test", "content": "example"},
                "limit": 10
            },
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True
        results = data["data"]["results"]
        print(f"    ✓ 多字段搜索结果: {len(results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 多字段搜索失败: {e}")
        return False

def test_add_document():
    print("[测试] 添加文档（占位符实现）...")
    try:
        payload = {
            "doc_id": "doc_001",
            "fields": {
                "title": "Hello World",
                "content": "This is a test document",
                "category": "test",
                "view_count": "100"
            }
        }
        resp = requests.post(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
            json=payload,
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True
        print(f"    ✓ 文档添加成功，doc_id: {data['data']['doc_id']}")
        return True
    except Exception as e:
        print(f"    ✗ 添加文档失败: {e}")
        return False

def test_delete_index():
    print("[测试] 删除索引...")
    try:
        resp = requests.delete(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True
        print(f"    ✓ 索引 '{INDEX_NAME}' 删除成功")
        return True
    except Exception as e:
        print(f"    ✗ 删除索引失败: {e}")
        return False

def test_delete_index_not_found():
    print("[测试] 删除不存在的索引（应成功）...")
    try:
        resp = requests.delete(
            f"{BASE_URL}/api/v1/index/indices/non_existent_index",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True
        print("    ✓ 删除不存在的索引未报错")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def cleanup():
    print("[清理] 清理测试数据...")
    try:
        # 删除测试索引
        for idx in [INDEX_NAME, "test_all_types_index"]:
            resp = requests.delete(
                f"{BASE_URL}/api/v1/index/indices/{idx}",
                headers=get_auth_headers(),
                timeout=5
            )
            print(f"    - 清理索引 '{idx}': {'成功' if resp.json().get('success') else '失败'}")
    except Exception as e:
        print(f"    - 清理异常: {e}")

def run_all_tests():
    tests = [
        ("健康检查", test_health),
        ("用户登录", test_login),
        ("创建全文索引", test_create_index),
        ("列出索引", test_list_indices),
        ("获取索引字段", test_get_index_fields),
        ("获取索引元数据", test_get_index_meta),
        ("获取索引统计", lambda:test_get_index_stats(1)),
        ("创建多类型索引", test_create_index_with_all_types),
        ("统计更新验证", lambda:test_get_index_stats(2)),
        ("添加文档(占位符)", test_add_document),
        ("搜索(占位符)", test_search_empty_results),
        ("多字段搜索(占位符)", test_search_multi_field_empty),
        ("未认证请求", test_search_no_auth),
        ("无效Token", test_search_invalid_token),
        ("创建同名索引", test_create_duplicate_index),
        ("索引列表复查", test_list_indices),
        ("删除索引", test_delete_index),
        ("删除不存在的索引", test_delete_index_not_found),
    ]
    
    passed = 0
    failed = 0
    
    for name, test_fn in tests:
        if test_fn():
            passed += 1
        else:
            failed += 1
    
    print("\n" + "=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败, 总计 {len(tests)}")
    print("=" * 60)
    
    return failed == 0

if __name__ == "__main__":
    print("=" * 60)
    print("Index REST API 自动回归测试")
    print("=" * 60)
    print(f"服务器地址: {BASE_URL}")
    print()
    
    try:
        success = run_all_tests()
        cleanup()
        sys.exit(0 if success else 1)
    except KeyboardInterrupt:
        print("\n\n测试被中断")
        cleanup()
        sys.exit(1)
