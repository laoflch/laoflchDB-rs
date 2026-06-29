#!/usr/bin/env python3
"""
Python 自动回归测试: VectorService 向量化服务 gRPC 接口测试
"""
import subprocess
import time
import sys
import os
import signal
import grpc
import socket

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc
import vector_pb2
import vector_pb2_grpc

TEST_DB = "./laoflch_db_vec_test"
TEST_ADDR = "127.0.0.1:29888"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")

TOKEN = None
stub = None
vec_stub = None
server_proc = None


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
    """测试加载模型注册"""
    print("[测试] 注册模型...")
    try:
        req = vector_pb2.LoadModelRequest(
            model_name="bert_base",
            model_path="/tmp/models/bert_base",
            embedding_dim=768,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        assert resp.success, f"LoadModel failed: {resp.message}"
        print(f"    ✓ 模型注册成功: {resp.model_name}")
        return True
    except Exception as e:
        print(f"    ✗ 模型注册失败: {e}")
        return False


def test_load_model_empty_name():
    """测试加载空名称模型"""
    print("[测试] 加载空名称模型...")
    try:
        req = vector_pb2.LoadModelRequest(
            model_name="",
            model_path="/tmp/empty",
            embedding_dim=64,
        )
        resp = vec_stub.LoadModel(req, metadata=get_metadata())
        print(f"    ✓ 空名称模型加载结果: {resp.message}")
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
        assert resp.embedding_dim == 768
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
            dim=768,
        )
        resp = vec_stub.CreateEmbedding(req, metadata=get_metadata())
        assert resp.success, f"CreateEmbedding failed: {resp.message}"
        assert len(resp.results) == 3
        print(f"    ✓ 成功生成 {len(resp.results)} 条向量")
        for r in resp.results:
            assert len(r.embedding) == 768, f"向量维度应为 768，实际: {len(r.embedding)}"
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
            dim=768,
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
        assert len(resp.models) == 0, f"模型列表应为空，实际: {len(resp.models)}"
        print(f"    ✓ 卸载后模型列表为空: {len(resp.models)} 个")
        return True
    except Exception as e:
        print(f"    ✗ 获取模型列表失败: {e}")
        return False


def test_vector_model_lifecycle():
    """测试向量模型的完整生命周期"""
    print("[测试] 向量模型完整生命周期...")
    try:
        # 1. 加载模型
        load_req = vector_pb2.LoadModelRequest(
            model_name="lifecycle_model",
            model_path="/tmp/lifecycle",
            embedding_dim=128,
        )
        load_resp = vec_stub.LoadModel(load_req, metadata=get_metadata())
        assert load_resp.success

        # 2. 验证列表
        list_req = vector_pb2.ListModelsRequest()
        list_resp = vec_stub.ListModels(list_req, metadata=get_metadata())
        assert len(list_resp.models) == 1
        assert list_resp.models[0].model_name == "lifecycle_model"

        # 3. 生成向量
        embed_req = vector_pb2.EmbeddingRequest(
            model_name="lifecycle_model",
            texts=["lifecycle test"],
            dim=128,
        )
        embed_resp = vec_stub.CreateEmbedding(embed_req, metadata=get_metadata())
        assert embed_resp.success
        assert len(embed_resp.results) == 1
        assert len(embed_resp.results[0].embedding) == 128

        # 4. 卸载模型
        unload_req = vector_pb2.UnloadModelRequest(model_name="lifecycle_model")
        unload_resp = vec_stub.UnloadModel(unload_req, metadata=get_metadata())
        assert unload_resp.success

        # 5. 验证卸载后列表为空
        list_resp2 = vec_stub.ListModels(list_req, metadata=get_metadata())
        assert len(list_resp2.models) == 0

        print(f"    ✓ 完整生命周期测试通过: 加载 → 验证 → 向量化 → 卸载 → 确认")
        return True
    except Exception as e:
        print(f"    ✗ 生命周期测试失败: {e}")
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


def run_all_tests():
    tests = [
        ("用户登录", test_login),
        ("空模型列表", test_list_models_empty),
        ("注册模型", test_load_model),
        ("空名称加载模型", test_load_model_empty_name),
        ("加载后模型列表", test_list_models_after_load),
        ("获取模型信息", test_get_model_info),
        ("获取不存在的模型信息", test_get_model_info_not_found),
        ("生成文本向量", test_create_embedding),
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
    global server_proc, vec_stub, stub
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("VectorService gRPC 自动回归测试")
    print("=" * 60)

    print("\n[1/4] 使用已有编译产物...")
    if not os.path.exists(SERVER_BIN):
        print(f"找不到服务端二进制: {SERVER_BIN}，请先手动编译")
        return 1
    print(f"    ✓ 使用已有编译产物: {SERVER_BIN}")

    print("\n[2/4] 初始化数据库...")
    subprocess.run([SERVER_BIN, "init", "--db-path", TEST_DB], cwd="..", capture_output=True)
    print("    ✓ 数据库初始化完成")

    grpc_port = int(TEST_ADDR.split(':')[1])
    actual_addr = f"127.0.0.1:{grpc_port}"

    print("\n[3/4] 启动 laoflchDB gRPC 服务...")
    cmd = [SERVER_BIN, "start", "--addr", TEST_ADDR, "--db-path", TEST_DB]
    log_file = open("vector_grpc_server.log", "w")
    server_proc = subprocess.Popen(
        cmd, cwd="..",
        stdout=log_file, stderr=subprocess.STDOUT,
        preexec_fn=os.setsid
    )
    time.sleep(3)

    print("\n[4/4] 连接 gRPC 客户端...")
    channel = grpc.insecure_channel(actual_addr)
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
        print("\n[清理] 停止服务...")
        if server_proc:
            os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            server_proc.wait(timeout=5)
        print("    ✓ 服务已停止")

    return 0 if success else 1


if __name__ == "__main__":
    sys.exit(main())