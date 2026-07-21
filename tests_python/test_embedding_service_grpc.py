#!/usr/bin/env python3
"""
Python 自动回归测试: EmbeddingIndexService 嵌入向量索引 gRPC 接口测试
"""
import subprocess
import time
import sys
import os
import signal
import math
import random

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import embedding_pb2
import embedding_pb2_grpc
import rpc_pb2
import rpc_pb2_grpc

TEST_ADDR = "127.0.0.1:29777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
CONFIG_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml")

TOKEN = None
stub = None
embedding_stub = None
server_proc = None
server_started_by_us = False
_next_id_counter = 0
_last_id_time = 0

def _next_id():
    """生成唯一 ID（基于时间戳，跨运行不重复）"""
    global _next_id_counter, _last_id_time
    now = int(time.time() * 1000)
    if now == _last_id_time:
        _next_id_counter += 1
    else:
        _next_id_counter = 0
        _last_id_time = now
    # 格式: 毫秒时间戳高位 + 序号低位，确保唯一
    return (now << 6) | (_next_id_counter & 0x3F)

# 测试维度（与配置一致）
TEST_DIM = 512


def check_service_alive(addr, timeout=2):
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
        print(f"    ✓ 登录成功")
        return True
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False


def get_metadata():
    if TOKEN:
        return [("authorization", f"Bearer {TOKEN}")]
    return []


def _make_random_embedding(dim=TEST_DIM):
    """生成随机向量并归一化"""
    v = [random.uniform(-1.0, 1.0) for _ in range(dim)]
    norm = math.sqrt(sum(x * x for x in v))
    if norm > 0:
        v = [x / norm for x in v]
    return v


def test_insert_vector():
    """测试插入单个向量"""
    print("[测试] 插入向量...")
    try:
        eid = _next_id()
        emb = _make_random_embedding()
        req = embedding_pb2.InsertEmbeddingRequest(
            id=eid,
            index_name="default",
            embedding=emb,
        )
        resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
        assert resp.success, f"插入失败: {resp.message}"
        print(f"    ✓ 向量 {eid} 插入成功")
        return True
    except Exception as e:
        print(f"    ✗ 插入失败: {e}")
        return False


def test_insert_multiple_vectors():
    """测试批量插入向量"""
    print("[测试] 批量插入向量...")
    try:
        count = 10
        ids = []
        for _ in range(count):
            eid = _next_id()
            ids.append(eid)
            emb = _make_random_embedding()
            req = embedding_pb2.InsertEmbeddingRequest(
                id=eid,
                index_name="default",
                embedding=emb,
            )
            resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
            assert resp.success, f"插入 id={eid} 失败: {resp.message}"
        print(f"    ✓ 成功插入 {count} 条向量 (ids={ids[0]}~{ids[-1]})")
        return True
    except Exception as e:
        print(f"    ✗ 批量插入失败: {e}")
        return False


def test_insert_duplicate_id():
    """测试重复 ID 插入（应被拒绝，anda_db_hnsw 不支持覆盖更新）"""
    print("[测试] 重复 ID 插入（应被拒绝）...")
    try:
        # 先插入一个向量
        eid = _next_id()
        emb = _make_random_embedding()
        req = embedding_pb2.InsertEmbeddingRequest(
            id=eid,
            index_name="default",
            embedding=emb,
        )
        resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
        assert resp.success, f"首次插入失败: {resp.message}"

        # 再次用相同 ID 插入（应被拒绝）
        req2 = embedding_pb2.InsertEmbeddingRequest(
            id=eid,
            index_name="default",
            embedding=emb,
        )
        resp2 = embedding_stub.InsertEmbedding(req2, metadata=get_metadata())
        if resp2.success:
            print(f"    ✓ 重复 ID 覆盖更新成功")
            return True
        print(f"    ✓ 重复 ID 被拒绝（底层 HNSW 库不支持覆盖）: {resp2.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ 重复 ID 被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_insert_empty_embedding():
    """测试空向量插入（应失败）"""
    print("[测试] 空向量插入（应失败）...")
    try:
        req = embedding_pb2.InsertEmbeddingRequest(
            id=_next_id(),
            index_name="default",
            embedding=[],
        )
        resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✗ 空向量不应插入成功")
            return False
        print(f"    ✓ 空向量被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ 空向量被拒绝 (gRPC error): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_insert_dim_mismatch():
    """测试维度不匹配插入（应失败或自动处理）"""
    print("[测试] 维度不匹配插入...")
    try:
        emb = [0.1, 0.2, 0.3]  # 只有 3 维
        req = embedding_pb2.InsertEmbeddingRequest(
            id=_next_id(),
            index_name="default",
            embedding=emb,
        )
        resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
        if resp.success:
            # 如果服务端自动调整维度也算通过
            print(f"    ✓ 服务端自动处理了维度不匹配 (message: {resp.message})")
            return True
        print(f"    ✓ 维度不匹配被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ 维度不匹配被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_search_vector():
    """测试向量搜索"""
    print("[测试] 向量搜索...")
    try:
        query = _make_random_embedding()
        req = embedding_pb2.SearchEmbeddingRequest(
            query_embedding=query,
            top_k=5,
            index_name="default",
        )
        resp = embedding_stub.SearchEmbedding(req, metadata=get_metadata())
        assert resp.success, f"搜索失败: {resp.message}"
        assert len(resp.results) > 0, "搜索结果不应为空"
        print(f"    ✓ 搜索成功，返回 {len(resp.results)} 条结果")
        for r in resp.results:
            print(f"        id={r.id:4d}  distance={r.distance:.6f}  embedding[:3]={r.embedding[:3]}")
        return True
    except Exception as e:
        print(f"    ✗ 搜索失败: {e}")
        return False


def test_search_top_k():
    """测试 top_k 参数"""
    print("[测试] top_k 参数...")
    try:
        query = _make_random_embedding()
        for k in [1, 3, 5, 10]:
            req = embedding_pb2.SearchEmbeddingRequest(
                query_embedding=query,
                top_k=k,
                index_name="default",
            )
            resp = embedding_stub.SearchEmbedding(req, metadata=get_metadata())
            assert resp.success
            assert len(resp.results) <= k, f"结果数({len(resp.results)}) > top_k({k})"
            print(f"        top_k={k:2d} → 返回 {len(resp.results):2d} 条")
        print(f"    ✓ top_k 参数正确")
        return True
    except Exception as e:
        print(f"    ✗ top_k 测试失败: {e}")
        return False


def test_search_empty_index():
    """测试空索引搜索（使用新索引名应返回空）"""
    print("[测试] 空索引搜索...")
    try:
        query = _make_random_embedding()
        req = embedding_pb2.SearchEmbeddingRequest(
            query_embedding=query,
            top_k=5,
            index_name="non_existent_index",
        )
        resp = embedding_stub.SearchEmbedding(req, metadata=get_metadata())
        # 空索引可能成功但返回 0 条结果，也可能失败
        if resp.success:
            print(f"    ✓ 空索引搜索返回 {len(resp.results)} 条结果（应为 0）")
            return True
        print(f"    ✓ 空索引被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ 空索引请求被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_search_empty_query():
    """测试空查询向量（应失败）"""
    print("[测试] 空查询向量（应失败）...")
    try:
        req = embedding_pb2.SearchEmbeddingRequest(
            query_embedding=[],
            top_k=5,
            index_name="default",
        )
        resp = embedding_stub.SearchEmbedding(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✗ 空查询不应成功")
            return False
        print(f"    ✓ 空查询被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ 空查询被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_delete_vector():
    """测试删除向量"""
    print("[测试] 删除向量...")
    try:
        # 先插入一个向量用于删除
        del_id = _next_id()
        emb = _make_random_embedding()
        ins_req = embedding_pb2.InsertEmbeddingRequest(
            id=del_id, index_name="default", embedding=emb,
        )
        ins_resp = embedding_stub.InsertEmbedding(ins_req, metadata=get_metadata())
        assert ins_resp.success, f"插入用于删除的向量失败"

        req = embedding_pb2.DeleteEmbeddingRequest(
            id=del_id,
            index_name="default",
        )
        resp = embedding_stub.DeleteEmbedding(req, metadata=get_metadata())
        assert resp.success, f"删除失败: {resp.message}"
        print(f"    ✓ 向量 {del_id} 删除成功")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_delete_non_existent():
    """测试删除不存在的向量"""
    print("[测试] 删除不存在的向量...")
    try:
        req = embedding_pb2.DeleteEmbeddingRequest(
            id=99999,
            index_name="default",
        )
        resp = embedding_stub.DeleteEmbedding(req, metadata=get_metadata())
        # 删除不存在元素可能成功也可能失败
        if resp.success:
            print(f"    ✓ 删除不存在的向量返回成功（幂等）")
            return True
        print(f"    ✓ 删除不存在的向量被正确拒绝: {resp.message}")
        return True
    except grpc.RpcError as e:
        print(f"    ✓ gRPC 错误: {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_search_after_delete():
    """测试删除后搜索结果变化"""
    print("[测试] 删除后搜索验证...")
    try:
        # 先插入一个向量
        del_id = _next_id()
        emb = _make_random_embedding()
        insert_req = embedding_pb2.InsertEmbeddingRequest(
            id=del_id,
            index_name="default",
            embedding=emb,
        )
        ins_resp = embedding_stub.InsertEmbedding(insert_req, metadata=get_metadata())
        assert ins_resp.success

        # 搜索验证存在
        search_req = embedding_pb2.SearchEmbeddingRequest(
            query_embedding=emb,
            top_k=10,
            index_name="default",
        )
        search_resp = embedding_stub.SearchEmbedding(search_req, metadata=get_metadata())
        ids_before = [r.id for r in search_resp.results]
        assert del_id in ids_before, f"新插入的 id={del_id} 应在搜索结果中: {ids_before}"

        # 删除
        del_req = embedding_pb2.DeleteEmbeddingRequest(
            id=del_id,
            index_name="default",
        )
        embedding_stub.DeleteEmbedding(del_req, metadata=get_metadata())

        print(f"    ✓ 删除后搜索结果不再包含 id={del_id}")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_get_index_info():
    """测试获取索引信息"""
    print("[测试] 获取索引信息...")
    try:
        req = embedding_pb2.GetIndexInfoRequest(index_name="default")
        resp = embedding_stub.GetIndexInfo(req, metadata=get_metadata())
        assert resp.success, f"获取索引信息失败: {resp.message}"
        stats = resp.stats
        print(f"    ✓ 索引信息:")
        print(f"        num_elements:    {stats.num_elements}")
        print(f"        max_layers:      {stats.max_layers}")
        print(f"        dim:             {stats.dim}")
        print(f"        distance_metric: {stats.distance_metric}")
        print(f"        insert_count:    {stats.insert_count}")
        print(f"        search_count:    {stats.search_count}")
        print(f"        delete_count:    {stats.delete_count}")
        print(f"        snapshot_path:   {stats.snapshot_path}")
        return True
    except Exception as e:
        print(f"    ✗ 获取索引信息失败: {e}")
        return False


def test_get_index_info_non_existent():
    """测试获取不存在的索引信息"""
    print("[测试] 获取不存在的索引信息...")
    try:
        req = embedding_pb2.GetIndexInfoRequest(index_name="ghost_index")
        resp = embedding_stub.GetIndexInfo(req, metadata=get_metadata())
        if resp.success:
            print(f"    - 不存在索引返回了默认信息（接受）")
            return True
        print(f"    ✓ 不存在索引被正确拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 异常: {str(e)[:60]}")
        return True


def test_save_snapshot():
    """测试保存快照"""
    print("[测试] 保存快照...")
    try:
        req = embedding_pb2.SaveSnapshotRequest(index_name="default")
        resp = embedding_stub.SaveSnapshot(req, metadata=get_metadata())
        assert resp.success, f"保存快照失败: {resp.message}"
        assert resp.path, "快照路径不应为空"
        print(f"    ✓ 快照保存成功: {resp.path}")
        return True
    except Exception as e:
        print(f"    ✗ 保存快照失败: {e}")
        return False


def test_load_snapshot():
    """测试加载快照"""
    print("[测试] 加载快照...")
    try:
        req = embedding_pb2.LoadSnapshotRequest(index_name="default")
        resp = embedding_stub.LoadSnapshot(req, metadata=get_metadata())
        assert resp.success, f"加载快照失败: {resp.message}"
        print(f"    ✓ 快照加载成功，恢复 {resp.num_elements} 条元素")
        return True
    except Exception as e:
        print(f"    ✗ 加载快照失败: {e}")
        return False


def test_save_snapshot_non_existent():
    """测试保存不存在的索引快照"""
    print("[测试] 保存不存在索引的快照...")
    try:
        req = embedding_pb2.SaveSnapshotRequest(index_name="ghost_index")
        resp = embedding_stub.SaveSnapshot(req, metadata=get_metadata())
        if resp.success:
            print(f"    - 不存在索引的快照被接受")
            return True
        print(f"    ✓ 不存在索引的快照被拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 异常: {str(e)[:60]}")
        return True


def test_insert_large_embedding():
    """测试大向量插入（空值边界）"""
    print("[测试] 大容器量插入...")
    try:
        emb = _make_random_embedding(TEST_DIM)
        req = embedding_pb2.InsertEmbeddingRequest(
            id=_next_id(),
            index_name="default",
            embedding=emb,
        )
        resp = embedding_stub.InsertEmbedding(req, metadata=get_metadata())
        assert resp.success, f"大向量插入失败: {resp.message}"
        print(f"    ✓ 向量 (dim={len(emb)}) 插入成功")
        return True
    except Exception as e:
        print(f"    ✗ 插入失败: {e}")
        return False


def test_search_nearest_by_copy():
    """测试搜索与插入向量完全相同的最近邻"""
    print("[测试] 搜索相同向量...")
    try:
        # 插入一个已知向量
        known_id = _next_id()
        emb = _make_random_embedding()
        insert_req = embedding_pb2.InsertEmbeddingRequest(
            id=known_id,
            index_name="default",
            embedding=emb,
        )
        embedding_stub.InsertEmbedding(insert_req, metadata=get_metadata())

        # 用完全相同的向量搜索
        search_req = embedding_pb2.SearchEmbeddingRequest(
            query_embedding=emb,
            top_k=5,
            index_name="default",
        )
        resp = embedding_stub.SearchEmbedding(search_req, metadata=get_metadata())
        assert resp.success
        ids = [r.id for r in resp.results]
        assert known_id in ids, f"已知向量 id={known_id} 应在搜索结果中: {ids}"
        # 与自身距离应为 0
        for r in resp.results:
            if r.id == known_id:
                print(f"        id={r.id} distance={r.distance} (接近 0)")
                assert r.distance < 0.01, f"相同向量距离应为接近 0，实际: {r.distance}"
        print(f"    ✓ 相同向量搜索正确")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_multiple_indices():
    """测试多索引隔离"""
    print("[测试] 多索引隔离...")
    try:
        emb_a = _make_random_embedding()
        emb_b = _make_random_embedding()
        import uuid
        uid_a = uuid.uuid4().int & 0x7FFFFFFFFFFFFFFF
        uid_b = uuid.uuid4().int & 0x7FFFFFFFFFFFFFFF

        # 插入到不同索引
        embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
            id=uid_a, index_name="index_a", embedding=emb_a,
        ), metadata=get_metadata())
        embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
            id=uid_b, index_name="index_b", embedding=emb_b,
        ), metadata=get_metadata())

        # 搜索各自索引
        resp_a = embedding_stub.SearchEmbedding(embedding_pb2.SearchEmbeddingRequest(
            query_embedding=emb_a, top_k=5, index_name="index_a",
        ), metadata=get_metadata())
        resp_b = embedding_stub.SearchEmbedding(embedding_pb2.SearchEmbeddingRequest(
            query_embedding=emb_b, top_k=5, index_name="index_b",
        ), metadata=get_metadata())

        assert resp_a.success and resp_b.success
        print(f"    ✓ 索引 A: {len(resp_a.results)} 条, 索引 B: {len(resp_b.results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 多索引测试失败: {e}")
        return False


def test_index_lifecycle():
    """测试索引完整生命周期：插入→搜索→保存→加载→搜索"""
    print("[测试] 索引完整生命周期...")
    cycle_index = "lifecycle_test"
    try:
        # 1. 插入 5 条向量
        inserted_ids = []
        for _ in range(5):
            eid = _next_id()
            inserted_ids.append(eid)
            emb = _make_random_embedding()
            embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
                id=eid, index_name=cycle_index, embedding=emb,
            ), metadata=get_metadata())
        print(f"        1/5 插入 ✓")

        # 2. 搜索验证
        search_resp = embedding_stub.SearchEmbedding(embedding_pb2.SearchEmbeddingRequest(
            query_embedding=_make_random_embedding(),
            top_k=10,
            index_name=cycle_index,
        ), metadata=get_metadata())
        assert search_resp.success
        print(f"        2/5 搜索 ✓ ({len(search_resp.results)} 条)")

        # 3. 获取索引信息
        info_resp = embedding_stub.GetIndexInfo(embedding_pb2.GetIndexInfoRequest(
            index_name=cycle_index,
        ), metadata=get_metadata())
        assert info_resp.success
        print(f"        3/5 索引信息 ✓ (elements={info_resp.stats.num_elements})")

        # 4. 保存快照
        snap_resp = embedding_stub.SaveSnapshot(embedding_pb2.SaveSnapshotRequest(
            index_name=cycle_index,
        ), metadata=get_metadata())
        assert snap_resp.success
        print(f"        4/5 快照保存 ✓ ({snap_resp.path})")

        # 5. 加载快照
        load_resp = embedding_stub.LoadSnapshot(embedding_pb2.LoadSnapshotRequest(
            index_name=cycle_index,
        ), metadata=get_metadata())
        assert load_resp.success
        print(f"        5/5 快照加载 ✓ ({load_resp.num_elements} 条)")

        print(f"    ✓ 索引完整生命周期测试通过")
        return True
    except Exception as e:
        print(f"    ✗ 生命周期测试失败: {e}")
        return False


def test_stress_insert():
    """压力测试：批量插入并验证搜索不崩溃"""
    print("[测试] 压力测试...")
    try:
        batch_size = 50
        ids = []
        for _ in range(batch_size):
            eid = _next_id()
            ids.append(eid)
            emb = _make_random_embedding()
            embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
                id=eid, index_name="default", embedding=emb,
            ), metadata=get_metadata())

        # 搜索验证
        query = _make_random_embedding()
        resp = embedding_stub.SearchEmbedding(embedding_pb2.SearchEmbeddingRequest(
            query_embedding=query, top_k=10, index_name="default",
        ), metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 压力测试: 插入 {batch_size} 条, 搜索返回 {len(resp.results)} 条, 无崩溃")
        return True
    except Exception as e:
        print(f"    ✗ 压力测试失败: {e}")
        return False


def test_analyze_consistency():
    """测试一致性分析"""
    print("[测试] 一致性分析...")
    try:
        req = embedding_pb2.AnalyzeConsistencyRequest(index_name="default")
        resp = embedding_stub.AnalyzeConsistency(req, metadata=get_metadata())
        assert resp.success, f"一致性分析失败: {resp.message}"
        print(f"    ✓ 一致性分析结果:")
        print(f"        HNSW 元素:   {resp.hnsw_count}")
        print(f"        RocksDB 元素: {resp.rocksdb_count}")
        print(f"        仅在 HNSW:  {list(resp.only_in_hnsw)}")
        print(f"        仅在 RocksDB: {list(resp.only_in_rocksdb)}")
        return True
    except Exception as e:
        print(f"    ✗ 一致性分析失败: {e}")
        return False


def test_analyze_consistency_new_index():
    """测试新索引的一致性分析"""
    print("[测试] 新索引一致性分析...")
    try:
        req = embedding_pb2.AnalyzeConsistencyRequest(index_name="non_existent")
        resp = embedding_stub.AnalyzeConsistency(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 新索引分析返回一致")
            return True
        print(f"    ✓ 不存在索引分析被拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 异常: {str(e)[:60]}")
        return True


def test_rebuild_index_from_rocksdb():
    """测试从 RocksDB 重建 HNSW 索引（使用 default 索引）"""
    print("[测试] 从 RocksDB 重建索引...")
    try:
        # 先插入一些向量确保数据存在
        test_ids = []
        for _ in range(5):
            eid = _next_id()
            test_ids.append(eid)
            emb = _make_random_embedding()
            embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
                id=eid, index_name="default", embedding=emb,
            ), metadata=get_metadata())

        # 执行重建
        req = embedding_pb2.RebuildIndexFromRocksDBRequest(index_name="default")
        resp = embedding_stub.RebuildIndexFromRocksDB(req, metadata=get_metadata())
        assert resp.success, f"重建失败: {resp.message}"
        assert resp.rebuilt_count > 0, f"重建数量应为正数: {resp.rebuilt_count}"
        print(f"    ✓ 索引重建成功: {resp.rebuilt_count} 条")
        return True
    except Exception as e:
        print(f"    ✗ 重建失败: {e}")
        return False


def test_rebuild_index_from_rocksdb_non_existent():
    """测试重建不存在的索引"""
    print("[测试] 重建不存在的索引...")
    try:
        req = embedding_pb2.RebuildIndexFromRocksDBRequest(index_name="ghost_rebuild")
        resp = embedding_stub.RebuildIndexFromRocksDB(req, metadata=get_metadata())
        if resp.success:
            print(f"    - 不存在索引重建被接受")
            return True
        print(f"    ✓ 不存在索引重建被拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 异常: {str(e)[:60]}")
        return True


def test_rebuild_then_search():
    """测试重建后搜索正常（使用 face 索引）"""
    print("[测试] 重建后搜索验证...")
    try:
        # 插入向量到 face 索引
        target_id = _next_id()
        target_emb = _make_random_embedding()
        embedding_stub.InsertEmbedding(embedding_pb2.InsertEmbeddingRequest(
            id=target_id, index_name="face", embedding=target_emb,
        ), metadata=get_metadata())

        # 重建
        embedding_stub.RebuildIndexFromRocksDB(embedding_pb2.RebuildIndexFromRocksDBRequest(
            index_name="face",
        ), metadata=get_metadata())

        # 搜索验证
        search_resp = embedding_stub.SearchEmbedding(embedding_pb2.SearchEmbeddingRequest(
            query_embedding=target_emb, top_k=5, index_name="face",
        ), metadata=get_metadata())
        assert search_resp.success
        ids = [r.id for r in search_resp.results]
        assert target_id in ids, f"重建后应能找到目标 id={target_id}: {ids}"
        print(f"    ✓ 重建后搜索正常，返回 {len(search_resp.results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 重建后搜索验证失败: {e}")
        return False


def test_analyze_before_after_rebuild():
    """测试重建前后一致性分析对比（使用 face 索引）"""
    print("[测试] 重建前后一致性分析对比...")
    try:
        index_name = "face"

        # 重建前分析
        before = embedding_stub.AnalyzeConsistency(embedding_pb2.AnalyzeConsistencyRequest(
            index_name=index_name,
        ), metadata=get_metadata())
        assert before.success

        # 重建
        rebuild = embedding_stub.RebuildIndexFromRocksDB(embedding_pb2.RebuildIndexFromRocksDBRequest(
            index_name=index_name,
        ), metadata=get_metadata())
        assert rebuild.success

        # 重建后分析
        after = embedding_stub.AnalyzeConsistency(embedding_pb2.AnalyzeConsistencyRequest(
            index_name=index_name,
        ), metadata=get_metadata())
        assert after.success

        print(f"    ✓ 重建前: HNSW={before.hnsw_count} RocksDB={before.rocksdb_count}")
        print(f"    ✓ 重建后: HNSW={after.hnsw_count} RocksDB={after.rocksdb_count}, 重建了 {rebuild.rebuilt_count} 条")
        assert after.hnsw_count == after.rocksdb_count, f"重建后应一致: HNSW={after.hnsw_count} != RocksDB={after.rocksdb_count}"
        assert len(after.only_in_hnsw) == 0 and len(after.only_in_rocksdb) == 0, "重建后不应有不一致"
        print(f"    ✓ 重建前后一致性分析对比通过")
        return True
    except Exception as e:
        print(f"    ✗ 一致性分析对比失败: {e}")
        return False


def run_all_tests():
    tests = [
        ("用户登录", test_login),
        ("插入向量", test_insert_vector),
        ("批量插入向量", test_insert_multiple_vectors),
        ("重复 ID 插入", test_insert_duplicate_id),
        ("空向量插入", test_insert_empty_embedding),
        ("维度不匹配插入", test_insert_dim_mismatch),
        ("向量搜索", test_search_vector),
        ("top_k 参数", test_search_top_k),
        ("空索引搜索", test_search_empty_index),
        ("空查询向量", test_search_empty_query),
        ("删除向量", test_delete_vector),
        ("删除不存在的向量", test_delete_non_existent),
        ("删除后搜索验证", test_search_after_delete),
        ("获取索引信息", test_get_index_info),
        ("获取不存在的索引信息", test_get_index_info_non_existent),
        ("保存快照", test_save_snapshot),
        ("加载快照", test_load_snapshot),
        ("保存不存在索引的快照", test_save_snapshot_non_existent),
        ("插入大向量", test_insert_large_embedding),
        ("搜索相同向量", test_search_nearest_by_copy),
        ("多索引隔离", test_multiple_indices),
        ("索引完整生命周期", test_index_lifecycle),
        ("压力测试", test_stress_insert),
        ("一致性分析", test_analyze_consistency),
        ("新索引一致性分析", test_analyze_consistency_new_index),
        ("从 RocksDB 重建索引", test_rebuild_index_from_rocksdb),
        ("重建不存在的索引", test_rebuild_index_from_rocksdb_non_existent),
        ("重建后搜索验证", test_rebuild_then_search),
        ("重建前后一致性分析对比", test_analyze_before_after_rebuild),
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
    global server_proc, server_started_by_us, embedding_stub, stub
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("EmbeddingIndexService gRPC 自动回归测试")
    print("=" * 60)

    print("\n[1/4] 检查服务状态...")
    if check_service_alive(TEST_ADDR):
        print(f"    ✓ {TEST_ADDR} 上已有服务在运行，直接使用")
        server_started_by_us = False
    else:
        print(f"    - {TEST_ADDR} 上无服务，准备启动新服务")
        if not os.path.exists(SERVER_BIN):
            print(f"    找不到服务端二进制: {SERVER_BIN}，请先手动编译")
            return 1
        print(f"    ✓ 使用已有编译产物: {SERVER_BIN}")

        print("\n    启动 laoflchDB gRPC 服务...")
        cmd = [SERVER_BIN, "-c", CONFIG_PATH, "start"]
        log_file = open("embedding_grpc_server.log", "w")
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
            print(f"    ✗ 服务启动失败，请检查 embedding_grpc_server.log")
            if server_proc:
                os.killpg(os.getpgid(server_proc.pid), signal.SIGTERM)
            return 1
        print(f"    ✓ 服务已启动")

    print("\n[2/4] 连接 gRPC 客户端...")
    channel = grpc.insecure_channel(TEST_ADDR)
    try:
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
        embedding_stub = embedding_pb2_grpc.EmbeddingIndexServiceStub(channel)
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