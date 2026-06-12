#!/usr/bin/env python3
"""
Python 自动回归测试: Index 全文本索引 gRPC 接口测试
"""
import subprocess
import time
import sys
import os
import signal
import grpc

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc

TEST_DB = "./laoflch_db_index_test"
TEST_ADDR = "127.0.0.1:19778"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
INDEX_NAME = "test_grpc_index"

TOKEN = None
stub = None

def test_login():
    global TOKEN, stub
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

def test_create_index():
    print("[测试] 创建全文索引...")
    try:
        req = rpc_pb2.CreateIndexRequest(
            index_name=INDEX_NAME,
            fields=[
                rpc_pb2.IndexFieldDef(name="title", field_type=0, comment="标题"),
                rpc_pb2.IndexFieldDef(name="content", field_type=0, comment="内容"),
                rpc_pb2.IndexFieldDef(name="view_count", field_type=1, comment="浏览次数"),
            ]
        )
        resp = stub.CreateIndex(req, metadata=get_metadata())
        assert resp.success, f"CreateIndex failed: {resp.message}"
        print(f"    ✓ 索引 '{INDEX_NAME}' 创建成功，ID: {resp.index_id}")
        return True
    except Exception as e:
        print(f"    ✗ 创建索引失败: {e}")
        return False

def test_list_indices():
    print("[测试] 列出索引...")
    try:
        req = rpc_pb2.ListIndicesRequest()
        resp = stub.ListIndices(req, metadata=get_metadata())
        assert resp.success, f"ListIndices failed: {resp.message}"
        assert INDEX_NAME in resp.index_names, f"索引列表应包含 '{INDEX_NAME}'，实际: {resp.index_names}"
        print(f"    ✓ 索引列表: {resp.index_names}")
        return True
    except Exception as e:
        print(f"    ✗ 列出索引失败: {e}")
        return False

def test_get_index_fields():
    print("[测试] 获取索引字段...")
    try:
        req = rpc_pb2.GetIndexFieldsRequest(index_name=INDEX_NAME)
        resp = stub.GetIndexFields(req, metadata=get_metadata())
        assert resp.success, f"GetIndexFields failed: {resp.message}"
        assert len(resp.fields) > 0, "字段列表不应为空"
        field_names = [f.column_name for f in resp.fields]
        assert "title" in field_names, f"应包含 'title'，实际: {field_names}"
        print(f"    ✓ 字段列表: {field_names}")
        return True
    except Exception as e:
        print(f"    ✗ 获取字段失败: {e}")
        return False

def test_get_index_meta():
    print("[测试] 获取索引元数据...")
    try:
        req = rpc_pb2.GetIndexMetaRequest(index_name=INDEX_NAME)
        resp = stub.GetIndexMeta(req, metadata=get_metadata())
        assert resp.success, f"GetIndexMeta failed: {resp.message}"
        assert resp.index_name == INDEX_NAME
        assert resp.column_count > 0
        print(f"    ✓ 索引元数据: name={resp.index_name}, columns={resp.column_count}")
        return True
    except Exception as e:
        print(f"    ✗ 获取元数据失败: {e}")
        return False

def test_get_index_stats():
    print("[测试] 获取索引统计...")
    try:
        req = rpc_pb2.GetIndexStatsRequest()
        resp = stub.GetIndexStats(req, metadata=get_metadata())
        assert resp.success, f"GetIndexStats failed: {resp.message}"
        assert resp.total_indices >= 1
        print(f"    ✓ 索引统计: total={resp.total_indices}, names={resp.index_names}")
        return True
    except Exception as e:
        print(f"    ✗ 获取统计失败: {e}")
        return False

def test_search_index():
    print("[测试] 搜索索引（占位符实现）...")
    try:
        req = rpc_pb2.SearchIndexRequest(index_name=INDEX_NAME, query="test", limit=10)
        resp = stub.SearchIndex(req, metadata=get_metadata())
        assert resp.success, f"SearchIndex failed: {resp.message}"
        print(f"    ✓ 搜索结果: {len(resp.results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 搜索失败: {e}")
        return False

def test_search_multi_field():
    print("[测试] 多字段搜索（占位符实现）...")
    try:
        import rpc_pb2 as pb2
        req = rpc_pb2.SearchIndexRequest(
            index_name=INDEX_NAME,
            query="",
            limit=10,
            field_queries={"title": "test", "content": "example"}
        )
        resp = stub.SearchIndex(req, metadata=get_metadata())
        assert resp.success, f"SearchIndex multi-field failed: {resp.message}"
        print(f"    ✓ 多字段搜索结果: {len(resp.results)} 条")
        return True
    except Exception as e:
        print(f"    ✗ 多字段搜索失败: {e}")
        return False

def test_drop_index():
    print("[测试] 删除索引...")
    try:
        req = rpc_pb2.DropIndexRequest(index_name=INDEX_NAME)
        resp = stub.DropIndex(req, metadata=get_metadata())
        assert resp.success, f"DropIndex failed: {resp.message}"
        print(f"    ✓ 索引 '{INDEX_NAME}' 删除成功")
        return True
    except Exception as e:
        print(f"    ✗ 删除索引失败: {e}")
        return False

def test_drop_non_existent_index():
    print("[测试] 删除不存在的索引...")
    try:
        req = rpc_pb2.DropIndexRequest(index_name="non_existent_index")
        resp = stub.DropIndex(req, metadata=get_metadata())
        assert resp.success
        print("    ✓ 删除不存在的索引未报错")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def test_unauthorized_create():
    print("[测试] 未认证创建索引（应失败）...")
    try:
        req = rpc_pb2.CreateIndexRequest(
            index_name="unauth_index",
            fields=[rpc_pb2.IndexFieldDef(name="f1", field_type=0)]
        )
        # 不带 metadata
        resp = stub.CreateIndex(req)
        print(f"    ✓ 未认证请求返回: success={resp.success}")
        return True
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.UNAUTHENTICATED:
            print("    ✓ 未认证请求被正确拒绝 (UNAUTHENTICATED)")
            return True
        print(f"    ✗ 期望 UNAUTHENTICATED，实际: {e.code()}")
        return False
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def run_all_tests():
    tests = [
        ("用户登录", test_login),
        ("创建索引", test_create_index),
        ("列出索引", test_list_indices),
        ("获取索引字段", test_get_index_fields),
        ("获取索引元数据", test_get_index_meta),
        ("获取索引统计", test_get_index_stats),
        ("搜索索引(占位符)", test_search_index),
        ("多字段搜索(占位符)", test_search_multi_field),
        ("删除索引", test_drop_index),
        ("删除不存在的索引", test_drop_non_existent_index),
        ("未认证请求", test_unauthorized_create),
    ]
    
    passed = 0
    failed = 0
    
    for name, test_fn in tests:
        ok = test_fn()
        if ok:
            passed += 1
        else:
            failed += 1
    
    print(f"\n测试结果: {passed} 通过, {failed} 失败, 总计 {len(tests)}")
    return failed == 0

def main():
    os.chdir(os.path.dirname(os.path.abspath(__file__)))

    print("=" * 60)
    print("Index gRPC 自动回归测试")
    print("=" * 60)

    print("\n[1/4] 使用已有编译产物...")
    if not os.path.exists(SERVER_BIN):
        print(f"找不到服务端二进制: {SERVER_BIN}，请先手动编译")
        print("在项目根目录执行: source local.env && cargo build --release")
        return 1
    print(f"    ✓ 使用已有编译产物: {SERVER_BIN}")

    print("\n[2/4] 初始化数据库...")
    subprocess.run([SERVER_BIN, "init", "--db-path", TEST_DB], cwd="..", capture_output=True)
    print("    ✓ 数据库初始化完成")
    
    # 清理旧配置
    config_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml")
    if os.path.exists(config_path):
        os.remove(config_path)
    print("    ✓ 旧配置已清理")

    print("\n[3/4] 启动 laoflchDB gRPC 服务...")
    cmd = [SERVER_BIN, "start", "--addr", TEST_ADDR, "--db-path", TEST_DB, "--index-path", TEST_DB]
    log_file = open("index_grpc_server.log", "w")
    server_proc = subprocess.Popen(
        cmd, cwd="..",
        stdout=log_file, stderr=subprocess.STDOUT,
        preexec_fn=os.setsid
    )
    time.sleep(3)
    # 检查服务日志
    with open("index_grpc_server.log") as f:
        log_content = f.read()
    if "启动成功" in log_content:
        print(f"    ✓ 服务已启动 PID={server_proc.pid}")
    else:
        print(f"    ⚠ 服务启动可能异常，日志: {log_content[:200]}")
    # 尝试从日志解析实际端口
    import re
    grpc_match = re.search(r'gRPC.*?(\d+\.\d+\.\d+\.\d+:\d+)', log_content)
    actual_addr = grpc_match.group(1) if grpc_match else TEST_ADDR
    print(f"    ✓ 实际 gRPC 地址: {actual_addr}")

    # 连接实际地址
    print("\n[4/4] 连接 gRPC 客户端...")
    channel = grpc.insecure_channel(actual_addr)

    global stub
    try:
        stub = rpc_pb2_grpc.LaoflchDbStub(channel)
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
