#!/usr/bin/env python3
"""
Python 自动化测试: RerankerService 精排服务 REST 接口测试（OpenAI 兼容 /v1/rerank）

测试范围：
- 服务可达性与健康检查（/v1/health）
- 基础精排（query + documents，默认 top_n）
- 指定 top_n
- return_documents 返回原文
- 参数校验（空 query / 空 documents）
- 排序正确性（相关文档应排在前）

依赖：requests（pip install requests）
"""
import sys
import os
import json
import requests

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

BASE_URL = "http://127.0.0.1:8080"
MODEL = "bge-reranker-v2-m3"

QUERY = "什么是机器学习"

DOCUMENTS = [
    "机器学习是人工智能的一个分支，通过数据训练模型进行预测和决策。",
    "今天天气很好，适合外出散步和运动。",
    "深度学习是机器学习的一个子领域，使用多层神经网络进行特征学习。",
    "苹果是一种常见的水果，富含维生素。",
    "神经网络由大量的神经元组成，是深度学习的基础结构。",
]

# 预期：与 query 相关的文档索引
# "什么是机器学习" → 相关：0（机器学习）、2（深度学习/机器学习子领域）、4（神经网络）
EXPECTED_RELEVANT_INDICES = {0, 2, 4}
IRRELEVANT_INDICES = {1, 3}


def check_health():
    """测试健康检查端点"""
    print("[测试] 健康检查 /v1/health...")
    try:
        resp = requests.get(f"{BASE_URL}/v1/health", timeout=10)
        if resp.status_code != 200:
            print(f"    ✗ HTTP {resp.status_code}: {resp.text}")
            return False
        data = resp.json()
        print(f"    ✓ ready={data.get('ready')}, model={data.get('model')}, status={data.get('status')}")
        assert data.get("ready") is True, "模型未就绪"
        assert data.get("model"), "model 不应为空"
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_basic():
    """测试基础精排（默认 top_n）"""
    print("[测试] 基础精排（默认 top_n）...")
    payload = {
        "model": MODEL,
        "query": QUERY,
        "documents": DOCUMENTS,
    }
    try:
        resp = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=60)
        if resp.status_code != 200:
            print(f"    ✗ HTTP {resp.status_code}: {resp.text}")
            return False
        data = resp.json()
        results = data.get("results", [])
        print(f"    ✓ 返回 {len(results)} 条结果")
        for r in results:
            print(f"      index={r['index']}, score={r['relevance_score']:.4f}")
        assert data.get("object") == "list", f"object 应为 list，实际 {data.get('object')}"
        assert data.get("model"), "model 不应为空"
        assert len(results) > 0, "结果不应为空"
        assert "usage" in data and "total_tokens" in data["usage"], "应包含 usage.total_tokens"
        # 验证排序正确性：最高分应来自相关文档
        top = results[0]["index"]
        assert top in EXPECTED_RELEVANT_INDICES, (
            f"最高分文档 index={top} 应为相关文档 {sorted(EXPECTED_RELEVANT_INDICES)}"
        )
        # 相关文档整体应排在无关文档之前
        min_irrelevant_rank = min(
            (i for i, r in enumerate(results) if r["index"] in IRRELEVANT_INDICES),
            default=len(results),
        )
        max_relevant_rank = max(
            (i for i, r in enumerate(results) if r["index"] in EXPECTED_RELEVANT_INDICES),
            default=-1,
        )
        assert max_relevant_rank < min_irrelevant_rank, (
            f"相关文档应在无关文档之前，相关最靠后={max_relevant_rank}，无关最靠前={min_irrelevant_rank}"
        )
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_top_n():
    """测试指定 top_n"""
    print("[测试] 指定 top_n=2...")
    payload = {
        "model": MODEL,
        "query": QUERY,
        "documents": DOCUMENTS,
        "top_n": 2,
    }
    try:
        resp = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=60)
        if resp.status_code != 200:
            print(f"    ✗ HTTP {resp.status_code}: {resp.text}")
            return False
        data = resp.json()
        results = data.get("results", [])
        print(f"    ✓ 返回 {len(results)} 条结果")
        for r in results:
            print(f"      index={r['index']}, score={r['relevance_score']:.4f}")
        assert len(results) == 2, f"top_n=2 应返回 2 条，实际 {len(results)}"
        # 分数应降序
        scores = [r["relevance_score"] for r in results]
        assert scores == sorted(scores, reverse=True), "结果应按分数降序"
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_return_documents():
    """测试 return_documents 返回原文"""
    print("[测试] return_documents=true 返回原文...")
    payload = {
        "model": MODEL,
        "query": QUERY,
        "documents": DOCUMENTS,
        "top_n": 3,
        "return_documents": True,
    }
    try:
        resp = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=60)
        if resp.status_code != 200:
            print(f"    ✗ HTTP {resp.status_code}: {resp.text}")
            return False
        data = resp.json()
        results = data.get("results", [])
        for r in results:
            doc = r.get("document")
            assert doc is not None, "return_documents=true 时应返回 document"
            expected = DOCUMENTS[r["index"]]
            assert doc["text"] == expected, (
                f"document 应匹配 index={r['index']} 的原文"
            )
        print(f"    ✓ {len(results)} 条结果的 document 均正确匹配原文")
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_empty_query():
    """测试空 query 应返回 400"""
    print("[测试] 空 query 参数校验...")
    payload = {"model": MODEL, "query": "", "documents": DOCUMENTS}
    try:
        resp = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=10)
        print(f"    ✓ HTTP {resp.status_code}: {resp.text}")
        assert resp.status_code == 400, f"空 query 应返回 400，实际 {resp.status_code}"
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_empty_documents():
    """测试空 documents 应返回 400"""
    print("[测试] 空 documents 参数校验...")
    payload = {"model": MODEL, "query": QUERY, "documents": []}
    try:
        resp = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=10)
        print(f"    ✓ HTTP {resp.status_code}: {resp.text}")
        assert resp.status_code == 400, f"空 documents 应返回 400，实际 {resp.status_code}"
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_rerank_order_stability():
    """测试排序稳定性：重复调用结果一致"""
    print("[测试] 排序稳定性（两次调用结果应一致）...")
    payload = {"model": MODEL, "query": QUERY, "documents": DOCUMENTS}
    try:
        r1 = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=60).json()
        r2 = requests.post(f"{BASE_URL}/v1/rerank", json=payload, timeout=60).json()
        idx1 = [x["index"] for x in r1["results"]]
        idx2 = [x["index"] for x in r2["results"]]
        assert idx1 == idx2, f"两次排序结果不一致: {idx1} vs {idx2}"
        print(f"    ✓ 两次调用排序一致: {idx1}")
        return True
    except AssertionError as e:
        print(f"    ✗ 断言失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def run_tests():
    tests = [
        ("check_health", check_health),
        ("test_rerank_basic", test_rerank_basic),
        ("test_rerank_top_n", test_rerank_top_n),
        ("test_rerank_return_documents", test_rerank_return_documents),
        ("test_rerank_empty_query", test_rerank_empty_query),
        ("test_rerank_empty_documents", test_rerank_empty_documents),
        ("test_rerank_order_stability", test_rerank_order_stability),
    ]
    passed = 0
    failed = 0
    for name, func in tests:
        try:
            if func():
                passed += 1
            else:
                failed += 1
        except Exception as e:
            print(f"    ✗ 异常: {e}")
            failed += 1
        print()

    print("=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败, 共 {passed + failed} 项")
    return failed == 0


if __name__ == "__main__":
    # 预检服务可达性
    try:
        requests.get(f"{BASE_URL}/v1/health", timeout=5)
    except Exception as e:
        print(f"服务 {BASE_URL} 不可达，请先启动服务: {e}")
        sys.exit(1)

    success = run_tests()
    sys.exit(0 if success else 1)
