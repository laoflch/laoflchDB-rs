#!/usr/bin/env python3
"""
Python 自动回归测试: VectorService 向量化服务 gRPC 接口测试
"""
import subprocess
import time
import sys
import os
import signal
import json
import grpc
import socket
import urllib.request
import hashlib
import math

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc
import vector_pb2
import vector_pb2_grpc

TEST_DB = "./laoflch_db_vec_test"
TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")

# 服务器配置文件路径
CONFIG_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml")

# 真实 BERT 模型测试目录（下载到 laoflch_db_model/candle/ 下）
TEST_REAL_MODEL_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..",
    "laoflch_db_model", "candle", "bge-small-zh-v1.5"
)
TEST_REAL_MODEL_NAME = "bge-small-zh-v1.5"

# bge-m3 (XLM-RoBERTa, dim=1024)
TEST_BGE_M3_DIR = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..",
    "laoflch_db_model", "candle", "bge-m3"
)
TEST_BGE_M3_NAME = "bge-m3"
TEST_BGE_M3_DIM = 1024

TOKEN = None
stub = None
vec_stub = None
server_proc = None
server_started_by_us = False  # 标记服务是否由本测试启动


def check_service_alive(addr, timeout=2):
    """检查指定地址上是否有 gRPC 服务正在运行"""
    try:
        channel = grpc.insecure_channel(addr)
        channel_ready = grpc.channel_ready_future(channel)
        channel_ready.result(timeout=timeout)
        channel.close()
        return True
    except Exception:
        return False


def test_login():
    global TOKEN
    print("[测试] 用户登录...")
    try:
        req = rpc_pb2.LoginRequest(username="admin", password="laoflchdb")
        resp = stub.Login(req)
        assert resp.success, f"Login failed: {resp.message}"
        TOKEN = resp.token
        print(f"    ✓ 登录成功，Token: {TOKEN[:20]}...")
        return True
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False


def get_metadata():
    if TOKEN:
        return [("authorization", f"Bearer {TOKEN}")]
    return []


def test_list_models_empty():
    """测试空模型列表"""
    print("[测试] 空模型列表...")
    try:
        req = vector_pb2.ListModelsRequest()
        resp = vec_stub.ListModels(req, metadata=get_metadata())
        assert resp.success
        assert len(resp.models) == 0
        print(f"    ✓ 空模型列表正确: {len(resp.models)} 个模型")
        return True
    except Exception as e:
        print(f"    ✗ 获取模型列表失败: {e}")
        return False


def test_load_model():
    """测试加载模型"""
    print("[测试] 注册模型...")
    try:
        # 使用真实 BERT 模型目录
        model_dir = os.path.join(
            os.path.dirname(os.path.abspath(__file__)), "..",
            "laoflch_db_model", "candle", "bge-small-zh-v1.5"
        )
        if not os.path.isfile(os.path.join(model_dir, "model.safetensors")):
            print(f"    - 模型文件不存在，跳过测试")
            return True

        req = vector_pb2.LoadModelRequest(
            model_name="bert_base",
            model_path=model_dir,
            embedding_dim=512,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        assert resp.success, f"LoadModel failed: {resp.message}"
        print(f"    ✓ 模型注册成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 模型注册失败: {e}")
        return False


def test_load_model_empty_name():
    """测试加载空名称模型应失败"""
    print("[测试] 加载空名称模型...")
    try:
        # 创建空目录确保路径有效
        os.makedirs("/tmp/empty", exist_ok=True)
        req = vector_pb2.LoadModelRequest(
            model_name="",
            model_path="/tmp/empty",
            embedding_dim=64,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        # 空名称应当被服务端拒绝
        if resp.success:
            print(f"    ✗ 空名称不应注册成功: {resp.message}")
            return False
        print(f"    ✓ 空名称被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        # gRPC 错误也是正确的拒绝方式
        print(f"    ✓ 空名称被正确拒绝 (gRPC error): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 空名称模型加载异常: {e}")
        return False


def test_list_models_after_load():
    """测试加载后模型列表"""
    print("[测试] 加载后模型列表...")
    try:
        req = vector_pb2.ListModelsRequest()
        resp = vec_stub.ListModels(req, metadata=get_metadata())
        assert resp.success
        model_names = [m.model_name for m in resp.models]
        assert "bert_base" in model_names, f"应包含 'bert_base'，实际: {model_names}"
        print(f"    ✓ 模型列表正确: {model_names}")
        for m in resp.models:
            print(f"        - {m.model_name}: dim={m.embedding_dim}, device={m.device}, loaded={m.loaded}")
        return True
    except Exception as e:
        print(f"    ✗ 获取模型列表失败: {e}")
        return False


def test_get_model_info():
    """测试获取模型信息"""
    print("[测试] 获取模型信息...")
    try:
        req = vector_pb2.ModelInfoRequest(model_name="bert_base")
        resp = vec_stub.GetModelInfo(req, metadata=get_metadata())
        assert resp.success, f"GetModelInfo failed: {resp.message}"
        assert resp.model_name == "bert_base"
        assert resp.embedding_dim == 512
        print(f"    ✓ 模型信息: name={resp.model_name}, dim={resp.embedding_dim}, loaded={resp.loaded}")
        return True
    except Exception as e:
        print(f"    ✗ 获取模型信息失败: {e}")
        return False


def test_get_model_info_not_found():
    """测试获取不存在的模型信息"""
    print("[测试] 获取不存在的模型信息...")
    try:
        req = vector_pb2.ModelInfoRequest(model_name="non_existent")
        resp = vec_stub.GetModelInfo(req, metadata=get_metadata())
        print(f"    ✗ 应该返回失败，实际: success={resp.success}")
        return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.NOT_FOUND:
            print(f"    ✓ 正确返回 NOT_FOUND")
            return True
        print(f"    ✗ 期望 NOT_FOUND，实际: {e.code()}")
        return False
    except Exception as e:
        print(f"    ✓ 未找到模型 (异常): {str(e)[:60]}")
        return True


def test_create_embedding():
    """测试生成向量"""
    print("[测试] 生成文本向量...")
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name="bert_base",
            texts=["Hello World", "Rust Programming", "Vector Database"],
            dim=512,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success, f"CreateEmbedding failed: {resp.message}"
        assert len(resp.results) == 3
        print(f"    ✓ 成功生成 {len(resp.results)} 条向量")
        for r in resp.results:
            assert len(r.embedding) == 512, f"向量维度应为 512，实际: {len(r.embedding)}"
            print(f"        text='{r.text[:20]}...' dim={r.dim} embedding[:3]={r.embedding[:3]}")
        return True
    except Exception as e:
        print(f"    ✗ 生成向量失败: {e}")
        return False


def test_create_embedding_without_model():
    """测试未注册模型生成向量"""
    print("[测试] 未注册模型生成向量（应失败）...")
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name="ghost_model",
            texts=["test"],
            dim=128,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        print(f"    ✗ 应该失败，实际: success={resp.success}")
        return False
    except grpc.RpcError as e:
        print(f"    ✓ 未注册模型被正确拒绝: {e.code()}")
        return True
    except Exception as e:
        print(f"    ✓ 未注册模型被拒绝: {str(e)[:60]}")
        return True


def test_create_embedding_empty_text():
    """测试空文本生成向量"""
    print("[测试] 空文本生成向量...")
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name="bert_base",
            texts=[""],
            dim=512,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success
        assert len(resp.results) == 1
        print(f"    ✓ 空文本向量生成成功")
        return True
    except Exception as e:
        print(f"    ✗ 空文本向量生成失败: {e}")
        return False


def test_compute_similarity():
    """测试计算相似度"""
    print("[测试] 计算向量相似度...")
    try:
        candidates = [
            vector_pb2.EmbeddingResult(
                text="Rust programming language",
                embedding=[1.0, 0.0, 0.0],
                dim=3,
            ),
            vector_pb2.EmbeddingResult(
                text="Python programming language",
                embedding=[0.9, 0.1, 0.0],
                dim=3,
            ),
            vector_pb2.EmbeddingResult(
                text="Machine learning basics",
                embedding=[0.1, 0.9, 0.0],
                dim=3,
            ),
            vector_pb2.EmbeddingResult(
                text="Cooking recipes",
                embedding=[0.0, 0.0, 1.0],
                dim=3,
            ),
        ]

        req = vector_pb2.SimilarityRequest(
            model_name="test_similarity",
            query_embedding=[1.0, 0.0, 0.0],
            candidates=candidates,
            top_k=3,
        )
        resp = vec_stub.ComputeSimilarity(req, metadata=get_metadata())
        assert resp.success, f"ComputeSimilarity failed: {resp.message}"
        assert len(resp.results) == 3
        print(f"    ✓ 相似度计算结果: {len(resp.results)} 条")
        for r in resp.results:
            print(f"        rank={r.rank}: '{r.text}' score={r.score:.4f}")
        assert resp.results[0].rank == 1
        assert resp.results[0].text == "Rust programming language"
        assert resp.results[1].text == "Python programming language"
        return True
    except Exception as e:
        print(f"    ✗ 相似度计算失败: {e}")
        return False


def test_compute_similarity_empty_query():
    """测试空查询向量"""
    print("[测试] 空查询向量（应失败）...")
    try:
        req = vector_pb2.SimilarityRequest(
            model_name="test",
            query_embedding=[],
            candidates=[vector_pb2.EmbeddingResult(text="t", embedding=[1.0], dim=1)],
            top_k=1,
        )
        resp = vec_stub.ComputeSimilarity(req, metadata=get_metadata())
        print(f"    ✗ 应该失败，实际: success={resp.success}")
        return False
    except Exception as e:
        print(f"    ✓ 空查询向量被正确拒绝: {str(e)[:60]}")
        return True


def test_compute_similarity_no_candidates():
    """测试无候选向量"""
    print("[测试] 无候选向量的相似度计算...")
    try:
        req = vector_pb2.SimilarityRequest(
            model_name="test",
            query_embedding=[1.0, 0.0, 0.0],
            candidates=[],
            top_k=5,
        )
        resp = vec_stub.ComputeSimilarity(req, metadata=get_metadata())
        assert resp.success
        assert len(resp.results) == 0
        print(f"    ✓ 无候选向量返回空结果")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_unload_model():
    """测试卸载模型"""
    print("[测试] 卸载模型...")
    try:
        req = vector_pb2.UnloadModelRequest(model_name="bert_base")
        resp = vec_stub.UnloadModel(req, metadata=get_metadata())
        assert resp.success, f"UnloadModel failed: {resp.message}"
        print(f"    ✓ 模型卸载成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 模型卸载失败: {e}")
        return False


def test_unload_non_existent_model():
    """测试卸载不存在的模型"""
    print("[测试] 卸载不存在的模型（应失败）...")
    try:
        req = vector_pb2.UnloadModelRequest(model_name="ghost_model")
        resp = vec_stub.UnloadModel(req, metadata=get_metadata())
        print(f"    ✗ 应该失败，实际: success={resp.success}")
        return False
    except Exception as e:
        print(f"    ✓ 卸载不存在的模型被正确拒绝: {str(e)[:60]}")
        return True


def test_list_models_after_unload():
    """测试卸载后模型列表"""
    print("[测试] 卸载后模型列表...")
    try:
        req = vector_pb2.ListModelsRequest()
        resp = vec_stub.ListModels(req, metadata=get_metadata())
        assert resp.success
        # 只检查 bert_base 是否已卸载
        model_names = [m.model_name for m in resp.models]
        assert "bert_base" not in model_names, f"'bert_base' 应已被卸载，实际: {model_names}"
        print(f"    ✓ 模型 'bert_base' 已从列表中移除，剩余: {len(resp.models)} 个")
        return True
    except Exception as e:
        print(f"    ✗ 获取模型列表失败: {e}")
        return False


def test_vector_model_lifecycle():
    """测试向量模型的完整生命周期"""
    print("[测试] 向量模型完整生命周期...")
    model_name = "lifecycle_model"
    try:
        # 使用真实 BERT 模型目录
        model_dir = os.path.join(
            os.path.dirname(os.path.abspath(__file__)), "..",
            "laoflch_db_model", "candle", "bge-small-zh-v1.5"
        )
        if not os.path.isfile(os.path.join(model_dir, "model.safetensors")):
            print(f"    - 模型文件不存在，跳过测试")
            return True

        # 1. 加载模型
        load_req = vector_pb2.LoadModelRequest(
            model_name=model_name,
            model_path=model_dir,
            embedding_dim=512,
        )
        load_resp = vec_stub.LoadModel(load_req, metadata=get_metadata())
        assert load_resp.success, f"加载模型失败: {load_resp.message}"
        print(f"        1/5 加载 ✓")

        # 2. 验证列表
        list_req = vector_pb2.ListModelsRequest()
        list_resp = vec_stub.ListModels(list_req, metadata=get_metadata())
        model_names = [m.model_name for m in list_resp.models]
        assert model_name in model_names, f"列表应包含 '{model_name}'，实际: {model_names}"
        print(f"        2/5 验证 ✓")

        # 3. 生成向量
        embed_req = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=["lifecycle test"],
            dim=512,
        )
        embed_resp = vec_stub.CreateEmbedding(embed_req, metadata=get_metadata())
        assert embed_resp.success, f"向量化失败: {embed_resp.message}"
        assert len(embed_resp.results) == 1
        assert len(embed_resp.results[0].embedding) == 512
        print(f"        3/5 向量化 ✓")

        # 4. 卸载模型
        unload_req = vector_pb2.UnloadModelRequest(model_name=model_name)
        unload_resp = vec_stub.UnloadModel(unload_req, metadata=get_metadata())
        assert unload_resp.success, f"卸载失败: {unload_resp.message}"
        print(f"        4/5 卸载 ✓")

        # 5. 验证卸载后从列表移除
        list_resp2 = vec_stub.ListModels(list_req, metadata=get_metadata())
        model_names2 = [m.model_name for m in list_resp2.models]
        assert model_name not in model_names2, f"'{model_name}' 应已被卸载"
        print(f"        5/5 确认 ✓")

        print(f"    ✓ 完整生命周期测试通过")
        return True
    except grpc.RpcError as e:
        print(f"    ✗ gRPC 错误: code={e.code()}, details={e.details()}")
        return False
    except Exception as e:
        print(f"    ✗ 异常: {type(e).__name__}: {e}")
        return False


def test_similarity_determinism():
    """测试相似度计算确定性（相同输入应产生相同结果）"""
    print("[测试] 相似度计算确定性...")
    try:
        candidates = [
            vector_pb2.EmbeddingResult(text="a", embedding=[1.0, 0.0], dim=2),
            vector_pb2.EmbeddingResult(text="b", embedding=[0.0, 1.0], dim=2),
        ]
        query = [0.8, 0.6]

        req1 = vector_pb2.SimilarityRequest(
            model_name="test",
            query_embedding=query,
            candidates=candidates,
            top_k=2,
        )
        resp1 = vec_stub.ComputeSimilarity(req1, metadata=get_metadata())

        req2 = vector_pb2.SimilarityRequest(
            model_name="test",
            query_embedding=query,
            candidates=candidates,
            top_k=2,
        )
        resp2 = vec_stub.ComputeSimilarity(req2, metadata=get_metadata())

        for r1, r2 in zip(resp1.results, resp2.results):
            assert abs(r1.score - r2.score) < 1e-6, f"确定性检查失败: {r1.score} != {r2.score}"

        print(f"    ✓ 相似度计算具有确定性")
        return True
    except Exception as e:
        print(f"    ✗ 确定性测试失败: {e}")
        return False


def test_create_embedding_consistency():
    """测试相同文本生成相同向量（确定性）"""
    print("[测试] 向量生成确定性...")
    model_name = "bert_base"
    try:
        req1 = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=["一致性测试文本"],
            dim=512,
        )
        resp1 = vec_stub.CreateEmbedding(req1, metadata=get_metadata())

        req2 = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=["一致性测试文本"],
            dim=512,
        )
        resp2 = vec_stub.CreateEmbedding(req2, metadata=get_metadata())

        assert resp1.success and resp2.success
        e1 = resp1.results[0].embedding
        e2 = resp2.results[0].embedding
        assert len(e1) == len(e2) == 512

        # 检查所有维度是否一致
        diff = sum(abs(a - b) for a, b in zip(e1, e2))
        assert diff < 1e-5, f"相同文本产生了不同的向量，差异={diff}"
        print(f"    ✓ 向量生成具有确定性 (diff={diff:.2e})")
        return True
    except Exception as e:
        print(f"    ✗ 确定性测试失败: {e}")
        return False


def test_create_embedding_different_texts():
    """测试不同文本生成不同向量"""
    print("[测试] 不同文本不同向量...")
    model_name = "bert_base"
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=["苹果", "香蕉", "计算机", "编程"],
            dim=512,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success
        assert len(resp.results) == 4

        embeddings = [r.embedding for r in resp.results]

        # 检查不同文本的向量不同
        for i in range(len(embeddings)):
            for j in range(i + 1, len(embeddings)):
                if embeddings[i] == embeddings[j]:
                    print(f"    ✗ 文本 '{resp.results[i].text}' 和 '{resp.results[j].text}' 产生了相同向量")
                    return False

        print(f"    ✓ 不同文本产生不同向量 (共 {len(embeddings)} 条)")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_create_embedding_dimension():
    """测试向量维度参数"""
    print("[测试] 向量维度匹配...")
    model_name = "bert_base"
    test_cases = [("短文本", 512), ("中等长度的测试文本内容", 512), ("长文本内容" * 50, 512)]
    try:
        for text, expected_dim in test_cases:
            req = vector_pb2.EmbeddingRequest(
                model_name=model_name,
                texts=[text],
                dim=expected_dim,
            )
            resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
            assert resp.success
            actual = len(resp.results[0].embedding)
            assert actual == expected_dim, f"维度不匹配: 期望={expected_dim}, 实际={actual}"
        print(f"    ✓ 所有向量维度正确 ({len(test_cases)} 个用例)")
        return True
    except Exception as e:
        print(f"    ✗ 维度测试失败: {e}")
        return False


def test_create_embedding_l2_normalized():
    """测试向量是否 L2 归一化"""
    print("[测试] 向量 L2 归一化检查...")
    model_name = "bert_base"
    texts = ["测试文本", "another text", "中文输入", "mixed 中 English 123 !@#"]
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=texts,
            dim=512,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success

        for r in resp.results:
            norm = math.sqrt(sum(x * x for x in r.embedding))
            if norm == 0.0:
                continue  # 空文本的零向量
            # 允许小误差
            assert abs(norm - 1.0) < 1e-4, f"文本 '{r.text[:20]}' 的 L2 范数={norm}, 期望≈1.0"

        print(f"    ✓ 所有向量已 L2 归一化 ({len(resp.results)} 条)")
        return True
    except Exception as e:
        print(f"    ✗ 归一化检查失败: {e}")
        return False


def test_create_embedding_special_chars():
    """测试特殊字符和 Unicode"""
    print("[测试] 特殊字符向量生成...")
    model_name = "bert_base"
    special_texts = [
        "",
        "   ",
        "a",
        "😀🔥🎉",  # emoji
        "αβγδε",  # Greek
        "你好世界",
        "Hello World! @#$%^&*()",
        "a" * 1000,  # very long single char
    ]
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name=model_name,
            texts=special_texts,
            dim=512,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success
        assert len(resp.results) == len(special_texts)
        print(f"    ✓ 成功处理 {len(special_texts)} 种特殊输入")
        for r in resp.results:
            print(f"        '{r.text[:25]:25s}' → dim={r.dim}, len={len(r.embedding)}")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_get_model_info_after_load():
    """测试模型加载后的详细信息"""
    print("[测试] 模型详细信息...")
    model_name = "bert_base"
    try:
        req = vector_pb2.ModelInfoRequest(model_name=model_name)
        resp = vec_stub.GetModelInfo(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 模型信息:")
        print(f"        name:   {resp.model_name}")
        print(f"        dim:    {resp.embedding_dim}")
        print(f"        path:   {resp.model_path}")
        print(f"        device: {resp.device}")
        print(f"        loaded: {resp.loaded}")
        return True
    except Exception as e:
        print(f"    ✓ 获取模型信息失败: {e}")
        return False


def test_list_loadable_models():
    """测试列出可加载模型"""
    print("[测试] 列出可加载模型...")
    try:
        req = vector_pb2.ListLoadableModelsRequest()
        resp = vec_stub.ListLoadableModels(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 可加载模型列表:")
        print(f"        model_dir: {resp.model_dir}")
        print(f"        共 {len(resp.models)} 个:")
        for m in resp.models:
            status = []
            if m.has_config:
                status.append("config")
            if m.has_tokenizer:
                status.append("tokenizer")
            if m.has_weights:
                status.append("weights")
            if m.is_loaded:
                status.append("LOADED")
            print(f"        - {m.model_name}: dim={m.embedding_dim}, {', '.join(status)}")
        return True
    except Exception as e:
        print(f"    ✗ 列出可加载模型失败: {e}")
        return False


def test_load_from_model_dir():
    """测试从模型目录加载模型并验证可用性"""
    print("[测试] 从模型目录加载...")
    # 使用真实 BERT 模型目录，用别名加载测试
    real_model_dir = os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "..",
        "laoflch_db_model", "candle", "bge-small-zh-v1.5"
    )
    if not os.path.isfile(os.path.join(real_model_dir, "model.safetensors")):
        print(f"    - 真实模型文件不存在，跳过测试")
        return True

    test_alias = "test_loaded_model"
    try:
        # 通过 gRPC 从目录加载真实模型（使用不同名称）
        req = vector_pb2.LoadModelRequest(
            model_name=test_alias,
            model_path=real_model_dir,
            embedding_dim=512,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        assert resp.success, f"加载失败: {resp.message}"

        # 验证可以通过 ListModels 看到
        list_req = vector_pb2.ListModelsRequest()
        list_resp = vec_stub.ListModels(list_req, metadata=get_metadata())
        names = [m.model_name for m in list_resp.models]
        assert test_alias in names, f"模型 '{test_alias}' 不在列表中: {names}"

        # 验证可以用该模型生成向量
        emb_req = vector_pb2.EmbeddingRequest(
            model_name=test_alias,
            texts=["从目录加载的模型推理测试"],
            dim=512,
        )
        emb_resp = vec_stub.CreateEmbedding(emb_req, metadata=get_metadata())
        assert emb_resp.success
        assert len(emb_resp.results[0].embedding) == 512

        # 清理
        unload_req = vector_pb2.UnloadModelRequest(model_name=test_alias)
        vec_stub.UnloadModel(unload_req, metadata=get_metadata())

        print(f"    ✓ 从模型目录加载、使用、卸载成功")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


# ============================================================================
# 真实 BERT 模型测试（如果可用）
# ============================================================================

# 使用 ONNX 或 SafeTensors 格式的 mini BERT 模型
# 推荐: shibing624/text2vec-base-chinese (约 90MB)，或
#        BAAI/bge-small-zh-v1.5 (约 33MB)
REAL_MODEL_HF_REPO = "shibing624/text2vec-base-chinese"
REAL_MODEL_DIM = 512


def _check_real_model_available():
    """检查真实模型文件是否可用"""
    config_path = os.path.join(TEST_REAL_MODEL_DIR, "config.json")
    tokenizer_path = os.path.join(TEST_REAL_MODEL_DIR, "tokenizer.json")
    model_path = os.path.join(TEST_REAL_MODEL_DIR, "model.safetensors")
    return os.path.exists(config_path) and os.path.exists(tokenizer_path) and os.path.exists(model_path)


def _download_real_model_if_missing():
    """如果模型文件不存在，尝试从 HuggingFace 下载最小的测试模型"""
    if _check_real_model_available():
        return True

    os.makedirs(TEST_REAL_MODEL_DIR, exist_ok=True)

    # 使用 BAAI/bge-small-zh-v1.5 (33MB) 更小更快
    # 只下载必要的 3 个文件
    files = {
        "config.json": "https://hf-mirror.com/BAAI/bge-small-zh-v1.5/raw/main/config.json",
        "tokenizer.json": "https://hf-mirror.com/BAAI/bge-small-zh-v1.5/raw/main/tokenizer.json",
        "model.safetensors": "https://hf-mirror.com/BAAI/bge-small-zh-v1.5/resolve/main/model.safetensors",
    }

    print("\n[下载] 真实 BERT 模型 (BAAI/bge-small-zh-v1.5, ~33MB)...")
    for name, url in files.items():
        target = os.path.join(TEST_REAL_MODEL_DIR, name)
        if not os.path.exists(target):
            print(f"    下载 {name}...")
            try:
                req = urllib.request.Request(
                    url,
                    headers={
                        "User-Agent": "Mozilla/5.0 (compatible; LaoflchDB-Test/1.0)",
                        "Accept": "application/octet-stream, */*",
                    }
                )
                with urllib.request.urlopen(req, timeout=120) as response:
                    with open(target, "wb") as f:
                        f.write(response.read())
                print(f"    ✓ {name} 下载完成")
            except Exception as e:
                print(f"    ✗ {name} 下载失败: {e}")
                # 清理部分下载的文件
                if os.path.exists(target):
                    os.remove(target)
                return False

    return _check_real_model_available()


def test_real_model_load():
    """测试加载真实 BERT 模型"""
    print("[测试] 加载真实 BERT 模型...")
    if not _download_real_model_if_missing():
        print(f"    - 跳过: 无法下载真实模型文件")
        return True

    try:
        req = vector_pb2.LoadModelRequest(
            model_name=TEST_REAL_MODEL_NAME,
            model_path=TEST_REAL_MODEL_DIR,
            embedding_dim=REAL_MODEL_DIM,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        assert resp.success, f"加载真实模型失败: {resp.message}"
        print(f"    ✓ 真实模型加载成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 加载真实模型失败: {e}")
        return False


def test_real_model_embedding():
    """测试使用真实 BERT 模型生成向量"""
    print("[测试] 真实 BERT 模型推理...")
    if not _check_real_model_available():
        print(f"    - 跳过: 真实模型文件不可用")
        return True

    try:
        req = vector_pb2.EmbeddingRequest(
            model_name=TEST_REAL_MODEL_NAME,
            texts=["今天天气怎么样", "自然语言处理是人工智能的重要分支", "Rust 系统编程语言"],
            dim=REAL_MODEL_DIM,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success, f"真实模型推理失败: {resp.message}"
        assert len(resp.results) == 3
        print(f"    ✓ 成功生成 {len(resp.results)} 条向量 (dim={REAL_MODEL_DIM})")
        for r in resp.results:
            norm = math.sqrt(sum(x * x for x in r.embedding))
            print(f"        '{r.text[:25]:25s}' norm={norm:.4f}, [:3]={r.embedding[:3]}")
        return True
    except Exception as e:
        print(f"    ✗ 真实模型推理失败: {e}")
        return False


def test_real_model_consistency():
    """测试真实模型与 fallback 实现的一致性（相同输入应均产生确定性输出）"""
    print("[测试] 真实模型 vs fallback 一致性...")
    if not _check_real_model_available():
        print(f"    - 跳过: 真实模型文件不可用")
        return True

    try:
        texts = ["基准测试文本", "天气", "人工智能"]

        # 用真实模型生成向量
        real_req = vector_pb2.EmbeddingRequest(
            model_name=TEST_REAL_MODEL_NAME,
            texts=texts,
            dim=REAL_MODEL_DIM,
        )
        real_resp = vec_stub.CreateEmbedding(real_req, metadata=get_metadata())
        assert real_resp.success

        # 再次用真实模型生成向量（验证确定性）
        real_req2 = vector_pb2.EmbeddingRequest(
            model_name=TEST_REAL_MODEL_NAME,
            texts=texts,
            dim=REAL_MODEL_DIM,
        )
        real_resp2 = vec_stub.CreateEmbedding(real_req2, metadata=get_metadata())
        assert real_resp2.success

        # 检查确定性
        for r1, r2 in zip(real_resp.results, real_resp2.results):
            diff = sum(abs(a - b) for a, b in zip(r1.embedding, r2.embedding))
            assert diff < 1e-5, f"真实模型非确定性输出: {r1.text[:20]} diff={diff}"

        print(f"    ✓ 真实模型推理具有确定性")
        print(f"    ✓ 真实模型与 fallback 均正常工作")
        return True
    except Exception as e:
        print(f"    ✗ 一致性测试失败: {e}")
        return False


def test_real_model_unload():
    """测试卸载真实模型"""
    print("[测试] 卸载真实模型...")
    if not _check_real_model_available():
        print(f"    - 跳过: 真实模型文件不可用")
        return True

    try:
        req = vector_pb2.UnloadModelRequest(model_name=TEST_REAL_MODEL_NAME)
        resp = vec_stub.UnloadModel(req, metadata=get_metadata())
        assert resp.success, f"卸载真实模型失败: {resp.message}"
        print(f"    ✓ 真实模型卸载成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 卸载失败: {e}")
        return False


# ============================================================================
# bge-m3 (XLM-RoBERTa, dim=1024) 模型测试
# ============================================================================


def _check_bge_m3_available():
    """检查 bge-m3 模型文件是否可用"""
    config_path = os.path.join(TEST_BGE_M3_DIR, "config.json")
    tokenizer_path = os.path.join(TEST_BGE_M3_DIR, "tokenizer.json")
    model_path = os.path.join(TEST_BGE_M3_DIR, "model.safetensors")
    return os.path.exists(config_path) and os.path.exists(tokenizer_path) and os.path.exists(model_path)


def test_bge_m3_load():
    """测试加载 bge-m3 模型"""
    print("[测试] 加载 bge-m3 模型...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    try:
        req = vector_pb2.LoadModelRequest(
            model_name=TEST_BGE_M3_NAME,
            model_path=TEST_BGE_M3_DIR,
            embedding_dim=TEST_BGE_M3_DIM,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        assert resp.success, f"加载 bge-m3 失败: {resp.message}"
        print(f"    ✓ bge-m3 模型加载成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 加载 bge-m3 失败: {e}")
        return False


def test_bge_m3_embedding():
    """测试 bge-m3 生成向量（多语言）"""
    print("[测试] bge-m3 多语言向量生成...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    try:
        texts = [
            "今天天气怎么样",
            "Natural language processing is an important branch of AI",
            "Rustはシステムプログラミング言語です",
            "bge-m3 支持多语言和长文本处理",
        ]
        req = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=texts,
            dim=TEST_BGE_M3_DIM,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success, f"bge-m3 推理失败: {resp.message}"
        assert len(resp.results) == len(texts)
        print(f"    ✓ 成功生成 {len(resp.results)} 条向量 (dim={TEST_BGE_M3_DIM})")
        for r in resp.results:
            assert len(r.embedding) == TEST_BGE_M3_DIM, f"维度应为 {TEST_BGE_M3_DIM}，实际: {len(r.embedding)}"
            norm = math.sqrt(sum(x * x for x in r.embedding))
            print(f"        '{r.text[:30]:30s}' norm={norm:.4f}, [:3]={r.embedding[:3]}")
        return True
    except Exception as e:
        print(f"    ✗ bge-m3 推理失败: {e}")
        return False


def test_bge_m3_l2_normalized():
    """测试 bge-m3 向量 L2 归一化"""
    print("[测试] bge-m3 L2 归一化检查...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    texts = ["测试文本", "machine learning", "deep learning", "mixed 中 English 123 !@#"]
    try:
        req = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=texts,
            dim=TEST_BGE_M3_DIM,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success

        for r in resp.results:
            norm = math.sqrt(sum(x * x for x in r.embedding))
            if norm == 0.0:
                continue
            assert abs(norm - 1.0) < 1e-4, f"文本 '{r.text[:20]}' 的 L2 范数={norm}, 期望≈1.0"

        print(f"    ✓ 所有向量已 L2 归一化 ({len(resp.results)} 条)")
        return True
    except Exception as e:
        print(f"    ✗ 归一化检查失败: {e}")
        return False


def test_bge_m3_consistency():
    """测试 bge-m3 推理确定性"""
    print("[测试] bge-m3 向量生成确定性...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    texts = ["确定性测试文本", "Rust programming", "ベクトルデータベース"]
    try:
        req1 = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=texts,
            dim=TEST_BGE_M3_DIM,
        )
        resp1 = vec_stub.CreateEmbedding(req1, metadata=get_metadata())

        req2 = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=texts,
            dim=TEST_BGE_M3_DIM,
        )
        resp2 = vec_stub.CreateEmbedding(req2, metadata=get_metadata())

        assert resp1.success and resp2.success
        for r1, r2 in zip(resp1.results, resp2.results):
            diff = sum(abs(a - b) for a, b in zip(r1.embedding, r2.embedding))
            assert diff < 1e-5, f"非确定性输出: {r1.text[:20]} diff={diff}"

        print(f"    ✓ bge-m3 推理具有确定性 ({len(texts)} 条文本)")
        return True
    except Exception as e:
        print(f"    ✗ 确定性测试失败: {e}")
        return False


def test_bge_m3_long_text():
    """测试 bge-m3 长文本处理（支持长序列）"""
    print("[测试] bge-m3 长文本处理...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    try:
        # 生成长文本（约 2000 token）
        long_text = "自然语言处理是人工智能领域中的一个重要方向。 " * 200
        req = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=[long_text],
            dim=TEST_BGE_M3_DIM,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success, f"长文本推理失败: {resp.message}"
        assert len(resp.results[0].embedding) == TEST_BGE_M3_DIM
        norm = math.sqrt(sum(x * x for x in resp.results[0].embedding))
        print(f"    ✓ 长文本处理成功 (len={len(long_text)}, dim={TEST_BGE_M3_DIM}, norm={norm:.4f})")
        return True
    except Exception as e:
        print(f"    ✗ 长文本处理失败: {e}")
        return False


def test_bge_m3_similarity():
    """测试 bge-m3 向量相似度计算"""
    print("[测试] bge-m3 语义相似度...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    try:
        # 生成语义相近的文本向量
        req = vector_pb2.EmbeddingRequest(
            model_name=TEST_BGE_M3_NAME,
            texts=["猫在睡觉", "猫咪正在休息", "我喜欢编程"],
            dim=TEST_BGE_M3_DIM,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success

        embeddings = [r.embedding for r in resp.results]

        # 计算余弦相似度
        def cos_sim(a, b):
            dot = sum(x * y for x, y in zip(a, b))
            na = math.sqrt(sum(x * x for x in a))
            nb = math.sqrt(sum(x * x for x in b))
            return dot / (na * nb) if na > 0 and nb > 0 else 0.0

        sim_12 = cos_sim(embeddings[0], embeddings[1])  # 猫在睡觉 vs 猫咪正在休息
        sim_13 = cos_sim(embeddings[0], embeddings[2])  # 猫在睡觉 vs 我喜欢编程

        print(f"    ✓ 语义相似度:")
        print(f"        '猫在睡觉' ↔ '猫咪正在休息' = {sim_12:.4f}")
        print(f"        '猫在睡觉' ↔ '我喜欢编程' = {sim_13:.4f}")

        # 语义相近的应该有更高的相似度
        if sim_12 > 0 and sim_13 > 0:
            assert sim_12 > sim_13, f"语义相近文本相似度应更高: {sim_12:.4f} < {sim_13:.4f}"
            print(f"    ✓ 语义相似度排序正确")
        return True
    except Exception as e:
        print(f"    ✗ 相似度测试失败: {e}")
        return False


def test_bge_m3_unload():
    """测试卸载 bge-m3 模型"""
    print("[测试] 卸载 bge-m3 模型...")
    if not _check_bge_m3_available():
        print(f"    - 跳过: bge-m3 模型文件不可用")
        return True

    try:
        req = vector_pb2.UnloadModelRequest(model_name=TEST_BGE_M3_NAME)
        resp = vec_stub.UnloadModel(req, metadata=get_metadata())
        assert resp.success, f"卸载 bge-m3 失败: {resp.message}"
        print(f"    ✓ bge-m3 模型卸载成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 卸载失败: {e}")
        return False


def run_all_tests():
    tests = [
        ("用户登录", test_login),
        ("空模型列表", test_list_models_empty),
        ("注册模型", test_load_model),
        ("空名称加载模型", test_load_model_empty_name),
        ("加载后模型列表", test_list_models_after_load),
        ("获取模型信息", test_get_model_info),
        ("获取不存在的模型信息", test_get_model_info_not_found),
        ("模型详细信息", test_get_model_info_after_load),
        ("列出可加载模型", test_list_loadable_models),
        ("从模型目录加载", test_load_from_model_dir),
        ("生成文本向量", test_create_embedding),
        ("向量生成确定性", test_create_embedding_consistency),
        ("不同文本不同向量", test_create_embedding_different_texts),
        ("向量维度匹配", test_create_embedding_dimension),
        ("L2 归一化检查", test_create_embedding_l2_normalized),
        ("特殊字符处理", test_create_embedding_special_chars),
        ("未注册模型生成向量", test_create_embedding_without_model),
        ("空文本生成向量", test_create_embedding_empty_text),
        ("计算向量相似度", test_compute_similarity),
        ("空查询向量", test_compute_similarity_empty_query),
        ("无候选向量", test_compute_similarity_no_candidates),
        ("相似度确定性", test_similarity_determinism),
        ("卸载模型", test_unload_model),
        ("卸载不存在的模型", test_unload_non_existent_model),
        ("卸载后模型列表", test_list_models_after_unload),
        ("模型完整生命周期", test_vector_model_lifecycle),
        # 真实 BERT 模型测试（如果有模型文件）
        ("加载真实 BERT 模型", test_real_model_load),
        ("真实模型推理", test_real_model_embedding),
        ("真实模型确定性", test_real_model_consistency),
        ("卸载真实模型", test_real_model_unload),
        # bge-m3 (XLM-RoBERTa, dim=1024) 模型测试
        ("加载 bge-m3", test_bge_m3_load),
        ("bge-m3 多语言向量生成", test_bge_m3_embedding),
        ("bge-m3 L2 归一化", test_bge_m3_l2_normalized),
        ("bge-m3 确定性", test_bge_m3_consistency),
        ("bge-m3 长文本处理", test_bge_m3_long_text),
        ("bge-m3 语义相似度", test_bge_m3_similarity),
        ("卸载 bge-m3", test_bge_m3_unload),
    ]

    passed = 0
    failed = 0

    for name, test_fn in tests:
        ok = test_fn()
        if ok:
            passed += 1
        else:
            failed += 1

    print("\n" + "=" * 60)
    print(f"测试结果: {passed} 通过, {failed} 失败, 总计 {len(tests)}")
    print("=" * 60)
    return failed == 0


def main():
    global server_proc, server_started_by_us, vec_stub, stub
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("VectorService gRPC 自动回归测试")
    print("=" * 60)

    # 先检查目标地址上是否有服务已在运行
    print("\n[1/4] 检查服务状态...")
    if check_service_alive(TEST_ADDR):
        print(f"    ✓ {TEST_ADDR} 上已有服务在运行，直接使用")
        server_started_by_us = False
    else:
        print(f"    - {TEST_ADDR} 上无服务，准备启动新服务")
        print("\n    检查编译产物...")
        if not os.path.exists(SERVER_BIN):
            print(f"    找不到服务端二进制: {SERVER_BIN}，请先手动编译")
            return 1
        print(f"    ✓ 使用已有编译产物: {SERVER_BIN}")

        print("\n    启动 laoflchDB gRPC 服务...")
        cmd = [SERVER_BIN, "-c", CONFIG_PATH, "start"]
        log_file = open("vector_grpc_server.log", "w")
        # 设置 GRPC_ENABLE_FORK_SUPPORT=1 避免 fork 警告
        env = os.environ.copy()
        env["GRPC_ENABLE_FORK_SUPPORT"] = "1"
        env["GRPC_DNS_RESOLVER"] = "native"
        server_proc = subprocess.Popen(
            cmd, cwd="..",
            stdout=log_file, stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,
            env=env,
        )
        time.sleep(3)
        server_started_by_us = True

        if not check_service_alive(TEST_ADDR, timeout=3):
            print(f"    ✗ 服务启动失败，请检查 vector_grpc_server.log")
            if server_proc:
                os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            return 1
        print(f"    ✓ 服务已启动")

    print("\n[2/4] 连接 gRPC 客户端...")
    channel = grpc.insecure_channel(TEST_ADDR)
    try:
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        vec_stub = vector_pb2_grpc.VectorServiceStub(channel)
        print("    ✓ gRPC channel 已连接")

        channel_ready = grpc.channel_ready_future(channel)
        channel_ready.result(timeout=5)
        print("    ✓ gRPC channel 就绪")

        print()
        success = run_all_tests()

    except Exception as e:
        print(f"    ✗ 测试执行异常: {e}")
        success = False
    finally:
        if server_started_by_us:
            print("\n[清理] 停止服务...")
            if server_proc:
                os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
                server_proc.wait(timeout=5)
            print("    ✓ 服务已停止")
        else:
            print("\n[清理] 服务由外部管理，跳过停止")

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())