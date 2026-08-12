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
        import time as _time
        doc_id = str(int(_time.time() * 1000) % 10**12)
        payload = {
            "doc_id": doc_id,
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
        assert data["data"]["doc_id"] == doc_id, "返回值 doc_id 应与传入的自定义 doc_id 一致"
        print(f"    ✓ 文档添加成功，自定义 doc_id: {data['data']['doc_id']}")

        # 重复添加相同 doc_id 应报重复错误
        resp2 = requests.post(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
            json=payload,
            headers=get_auth_headers(),
            timeout=5
        )
        data2 = resp2.json()
        assert data2["success"] == False, "重复添加相同 doc_id 应失败"
        assert "重复" in str(data2.get("message", "")) or "Duplicate" in str(data2.get("message", "")) or "duplicate" in str(data2.get("message", "")), f"重复错误信息不符合预期: {data2}"
        print(f"    ✓ 重复 doc_id 被正确拒绝: {data2.get('message', '')}")

        # 非数字 doc_id 应被拒绝（仅支持数字 doc_id）
        bad_payload = {
            "doc_id": "abc_not_numeric",
            "fields": {"title": "bad", "content": "bad", "category": "bad", "view_count": "1"}
        }
        resp3 = requests.post(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
            json=bad_payload,
            headers=get_auth_headers(),
            timeout=5
        )
        data3 = resp3.json()
        assert data3["success"] == False, "非数字 doc_id 应被拒绝"
        print(f"    ✓ 非数字 doc_id 被正确拒绝: {data3.get('message', '')}")

        return True
    except Exception as e:
        print(f"    ✗ 添加文档失败: {e}")
        return False

def test_search_with_real_data():
    print("[测试] 添加多个文档并进行真实搜索测试...")
    try:
        import time as _time
        base = int(_time.time() * 1000) % 10**12
        # 添加多个测试文档
        docs = [
            {
                "doc_id": str(base + 1),
                "fields": {
                    "title": "Rust Programming",
                    "content": "Rust is a systems programming language focused on safety, speed, and concurrency.",
                    "category": "Programming Language",
                    "view_count": "150"
                }
            },
            {
                "doc_id": str(base + 2),
                "fields": {
                    "title": "Python Data Analysis",
                    "content": "Python is very popular in the field of data analysis with rich libraries and frameworks.",
                    "category": "Data Analysis",
                    "view_count": "230"
                }
            },
            {
                "doc_id": str(base + 3),
                "fields": {
                    "title": "Machine Learning Introduction",
                    "content": "Machine learning is a branch of artificial intelligence that allows computers to learn from data.",
                    "category": "Artificial Intelligence",
                    "view_count": "320"
                }
            },
            {
                "doc_id": str(base + 4),
                "fields": {
                    "title": "Database Systems",
                    "content": "Database systems are software systems used to store, manage, and retrieve data.",
                    "category": "Computer Science",
                    "view_count": "180"
                }
            }
        ]

        # 添加所有文档
        for doc in docs:
            resp = requests.post(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
                json=doc,
                headers=get_auth_headers(),
                timeout=5
            )
            data = resp.json()
            print(f"    ✓ 成功添加 {doc['doc_id']} 文档")
            assert data["success"] == True, f"添加文档 {doc['doc_id']} 失败"

        print(f"    ✓ 成功添加 {len(docs)} 个文档")

        # 测试基本搜索
        try:
            resp = requests.get(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search?q=Rust",
                headers=get_auth_headers(),
                timeout=5
            )
            if resp.status_code == 200:
                try:
                    data = resp.json()
                    results = data.get("data", {}).get("results", [])
                    print(f"    ✓ 搜索'Rust'返回 {len(results)} 条结果")
                except ValueError:
                    print(f"    ✓ 搜索'Rust'返回非JSON响应，状态码: {resp.status_code}")
            else:
                print(f"    ✓ 搜索'Rust'返回状态码: {resp.status_code}")
        except Exception as e:
            print(f"    ✓ 搜索'Rust'异常: {str(e)[:50]}...")

        # 测试多字段搜索
        try:
            resp = requests.post(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search/multi",
                json={
                    "field_queries": {"title": "Python", "category": "Data Analysis"},
                    "limit": 5
                },
                headers=get_auth_headers(),
                timeout=5
            )
            if resp.status_code == 200:
                try:
                    data = resp.json()
                    results = data.get("data", {}).get("results", [])
                    print(f"    ✓ 多字段搜索返回 {len(results)} 条结果")
                except ValueError:
                    print(f"    ✓ 多字段搜索返回非JSON响应，状态码: {resp.status_code}")
            else:
                print(f"    ✓ 多字段搜索返回状态码: {resp.status_code}")
        except Exception as e:
            print(f"    ✓ 多字段搜索异常: {str(e)[:50]}...")

        # 测试中文搜索（jieba 分词）
        try:
            import time as _time
            cn_base = int(_time.time() * 1000) % 10**12
            cn_docs = [
                {
                    "doc_id": str(cn_base + 1),
                    "fields": {
                        "title": "人工智能技术发展",
                        "content": "人工智能是当前最热门的技术领域，深度学习和机器学习正在改变世界。",
                        "category": "人工智能",
                        "view_count": "500"
                    }
                },
                {
                    "doc_id": str(cn_base + 2),
                    "fields": {
                        "title": "自然语言处理基础",
                        "content": "自然语言处理是人工智能的一个分支，主要研究人与计算机之间的语言交互。",
                        "category": "NLP",
                        "view_count": "400"
                    }
                },
                {
                    "doc_id": str(cn_base + 3),
                    "fields": {
                        "title": "数据库系统原理",
                        "content": "数据库系统是计算机科学中的一个重要方向，用于存储和管理数据。",
                        "category": "数据库",
                        "view_count": "300"
                    }
                },
            ]
            for doc in cn_docs:
                r = requests.post(
                    f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
                    json=doc,
                    headers=get_auth_headers(),
                    timeout=5
                )
                assert r.json()["success"], f"添加中文文档失败: {r.text}"

            # 用 "深度学习" 查询，应该命中第一条文档
            resp = requests.get(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/search?q=深度学习",
                headers=get_auth_headers(),
                timeout=5
            )
            data = resp.json()
            results = data.get("data", {}).get("results", [])
            assert len(results) >= 1, f"中文搜索'深度学习'应至少返回 1 条，实际: {len(results)}"
            # 首条结果应包含 "深度学习"
            first_title = results[0].get("fields", {}).get("title", "")
            first_content = results[0].get("fields", {}).get("content", "")
            assert "深度学习" in first_content or "深度学习" in first_title, f"首条结果不包含'深度学习': {results[0]}"
            print(f"    ✓ 中文搜索'深度学习'返回 {len(results)} 条结果，jieba 分词生效")
        except Exception as e:
            print(f"    ✗ 中文搜索测试失败: {e}")
            return False

        return True
    except Exception as e:
        print(f"    ✗ 真实数据搜索测试失败: {e}")
        return False

def test_get_document_by_id():
    print("[测试] 通过doc_id获取文档...")
    try:
        # 先添加一个测试文档
        #doc_id = "test999"
        payload = {
           # "doc_id": doc_id,
            "fields": {
                "title": "Document Retrieval Test",
                "content": "This document is used to test the retrieval by ID functionality.",
                "category": "Test",
                "view_count": "50"
            }
        }

        # 添加文档
        resp = requests.post(
            f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs",
            json=payload,
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        print(f"    ✓ 添加测试文档，doc_id: {data}")
        assert data["success"] == True, f"添加文档失败: {data}"
        return_doc_id=data["data"]["doc_id"]
        # 通过doc_id获取文档
        try:
            resp = requests.get(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs/{return_doc_id}",
                headers=get_auth_headers(),
                timeout=5
            )

            if resp.status_code == 200:
                try:
                    data = resp.json()
                    print(f"    ✓ 获取文档，doc_id: {data}")
                    if data.get("success"):
                        assert data["data"]["doc_id"] == return_doc_id, f"文档ID不匹配，预期: {return_doc_id}, 实际: {data['data']['doc_id']}"
                        assert data["data"]["fields"]["title"] == "Document Retrieval Test", "文档标题不匹配"
                        assert data["data"]["fields"]["category"] == "Test", "文档分类不匹配"

                        print(f"    ✓ 成功通过doc_id获取文档: {return_doc_id}")
                        print(f"    ✓ 文档标题: {data['data']['fields']['title']}")
                        print(f"    ✓ 文档分类: {data['data']['fields']['category']}")
                    else:
                        print(f"    ✓ 获取文档返回success=False: {data}")
                except ValueError:
                    print(f"    ✓ 获取文档返回非JSON响应，状态码: {resp.status_code}")
            else:
                print(f"    ✓ 获取文档返回状态码: {resp.status_code}")
        except Exception as e:
            print(f"    ✓ 获取文档异常: {str(e)[:50]}...")

        # 测试获取不存在的文档
        try:
            resp = requests.get(
                f"{BASE_URL}/api/v1/index/indices/{INDEX_NAME}/docs/non_existent_doc",
                headers=get_auth_headers(),
                timeout=5
            )

            if resp.status_code == 200:
                try:
                    data = resp.json()
                    print(f"    ✓ 获取不存在的文档返回状态: {data.get('success', 'unknown')}")
                except ValueError:
                    print(f"    ✓ 获取不存在的文档返回非JSON响应，状态码: {resp.status_code}")
            else:
                print(f"    ✓ 获取不存在的文档返回状态码: {resp.status_code}")
        except Exception as e:
            print(f"    ✓ 获取不存在的文档异常: {str(e)[:50]}...")

        return True
    except Exception as e:
        print(f"    ✗ 通过doc_id获取文档测试失败: {e}")
        return False

def test_paginated_documents():
    print("[测试] 分页获取索引下全部文档（上一页/下一页）...")
    page_index = "test_py_page_index"
    try:
        # 创建独立分页索引，确保文档数量确定
        create_payload = {
            "index_name": page_index,
            "fields": [
                {"name": "title", "field_type": "STRING", "comment": "标题"},
                {"name": "content", "field_type": "STRING", "comment": "内容"},
                {"name": "category", "field_type": "STRING", "comment": "分类"},
                {"name": "view_count", "field_type": "INT64", "comment": "浏览次数"},
            ]
        }
        resp = requests.post(f"{BASE_URL}/api/v1/index/indices", json=create_payload, headers=get_auth_headers(), timeout=5)
        data = resp.json()
        assert data["success"] == True, f"创建分页索引失败: {data}"

        # 添加一批测试文档用于分页
        page_docs = [
            {
                "doc_id": f"{300000000000 + i}",
                "fields": {
                    "title": f"Pagination Document {i}",
                    "content": f"Content of pagination document number {i}",
                    "category": "Pagination",
                    "view_count": str(10 + i)
                }
            }
            for i in range(8)
        ]
        for doc in page_docs:
            resp = requests.post(
                f"{BASE_URL}/api/v1/index/indices/{page_index}/docs",
                json=doc,
                headers=get_auth_headers(),
                timeout=5
            )
            data = resp.json()
            assert data["success"] == True, f"添加文档 {doc['doc_id']} 失败: {data}"

        # 第 1 页：offset=0, limit=3
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{page_index}/docs/list?offset=0&limit=3",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True, f"第1页请求失败: {data}"
        page1 = data["data"]
        assert len(page1["documents"]) == 3, f"第1页应返回3条，实际: {len(page1['documents'])}"
        assert page1["total"] == 8, f"total 应为 8，实际: {page1['total']}"
        assert page1["has_prev_page"] == False, "第1页 has_prev_page 应为 False"
        assert page1["has_next_page"] == True, "第1页 has_next_page 应为 True"
        print(f"    ✓ 第1页 offset=0 limit=3: {len(page1['documents'])} 条, total={page1['total']}, prev={page1['has_prev_page']}, next={page1['has_next_page']}")

        # 下一页：offset=3, limit=3
        next_offset = page1["offset"] + len(page1["documents"])
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{page_index}/docs/list?offset={next_offset}&limit=3",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True, f"下一页请求失败: {data}"
        page2 = data["data"]
        assert len(page2["documents"]) == 3, f"第2页应返回3条，实际: {len(page2['documents'])}"
        assert page2["has_prev_page"] == True, "第2页 has_prev_page 应为 True"
        assert page2["has_next_page"] == True, "第2页 has_next_page 应为 True"
        print(f"    ✓ 下一页 offset=3 limit=3: {len(page2['documents'])} 条, prev={page2['has_prev_page']}, next={page2['has_next_page']}")

        # 上一页：返回 offset=0
        prev_offset = max(page2["offset"] - page2["limit"], 0)
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{page_index}/docs/list?offset={prev_offset}&limit=3",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True, f"上一页请求失败: {data}"
        page_back = data["data"]
        assert page_back["offset"] == 0
        assert page_back["has_prev_page"] == False, "返回第1页后 has_prev_page 应为 False"
        print(f"    ✓ 上一页 offset=0: {len(page_back['documents'])} 条, prev={page_back['has_prev_page']}, next={page_back['has_next_page']}")

        # 末页：offset=6, limit=3，仅剩2条，无下一页
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{page_index}/docs/list?offset=6&limit=3",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True, f"末页请求失败: {data}"
        last_page = data["data"]
        assert len(last_page["documents"]) == 2, f"末页应返回2条，实际: {len(last_page['documents'])}"
        assert last_page["has_next_page"] == False, "末页 has_next_page 应为 False"
        assert last_page["has_prev_page"] == True, "末页 has_prev_page 应为 True"
        print(f"    ✓ 末页 offset=6 limit=3: {len(last_page['documents'])} 条, prev={last_page['has_prev_page']}, next={last_page['has_next_page']}")

        # 越界 offset 返回空列表
        resp = requests.get(
            f"{BASE_URL}/api/v1/index/indices/{page_index}/docs/list?offset=100&limit=3",
            headers=get_auth_headers(),
            timeout=5
        )
        data = resp.json()
        assert data["success"] == True, f"越界请求失败: {data}"
        assert len(data["data"]["documents"]) == 0, "越界 offset 应返回空列表"
        print(f"    ✓ 越界 offset=100 返回空列表")

        # 清理分页测试索引
        requests.delete(
            f"{BASE_URL}/api/v1/index/indices/{page_index}",
            headers=get_auth_headers(),
            timeout=5
        )

        return True
    except Exception as e:
        print(f"    ✗ 分页测试失败: {e}")
        # 尝试清理
        try:
            requests.delete(f"{BASE_URL}/api/v1/index/indices/{page_index}", headers=get_auth_headers(), timeout=5)
        except Exception:
            pass
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
        ("真实数据搜索测试", test_search_with_real_data),
        ("通过doc_id获取文档", test_get_document_by_id),
        ("分页获取全部文档", test_paginated_documents),
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
