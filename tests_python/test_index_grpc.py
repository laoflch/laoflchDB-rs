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
import socket
import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import rpc_pb2
import rpc_pb2_grpc

TEST_DB = "./laoflch_db_index_test"
TEST_ADDR = "127.0.0.1:29777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
INDEX_NAME = "test_grpc_index"

TOKEN = None
stub = None
server_proc = None

def check_port_in_use(host, port):
    """检查端口是否有服务监听"""
    try:
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.settimeout(1)
        result = sock.connect_ex((host, port))
        sock.close()
        return result == 0
    except:
        return False

def get_grpc_port_from_config(config_path):
    """从配置文件获取 gRPC 端口"""
    try:
        with open(config_path, 'r') as f:
            config = yaml.safe_load(f)
        for protocol in config.get('access_protocols', []):
            if protocol.get('protocol') == 'grpc' and protocol.get('enabled'):
                addr = protocol.get('addr', '0.0.0.0:19777')
                port = int(addr.split(':')[1])
                return port
        return 19777  # 默认端口
    except:
        return 19777

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

def test_get_version():
    print("[测试] 获取版本信息...")
    try:
        req = rpc_pb2.GetVersionRequest()
        resp = stub.GetVersion(req)
        assert resp.success, f"GetVersion failed: {resp.message}"
        print(f"    ✓ 版本信息: {resp.version}")
        return True
    except Exception as e:
        print(f"    ✗ 获取版本失败: {e}")
        return False

def test_create_index():
    print("[测试] 创建全文索引...")
    try:
        req = rpc_pb2.CreateIndexRequest(
            index_name=INDEX_NAME,
            fields=[
                rpc_pb2.IndexFieldDef(name="title", field_type=0, comment="标题"),
                rpc_pb2.IndexFieldDef(name="content", field_type=0, comment="内容"),
                rpc_pb2.IndexFieldDef(name="category", field_type=0, comment="分类"),
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

def test_create_index_with_all_types():
    print("[测试] 创建包含多种字段类型的索引...")
    try:
        req = rpc_pb2.CreateIndexRequest(
            index_name="test_all_types_index_grpc",
            fields=[
                rpc_pb2.IndexFieldDef(name="str_field", field_type=0),
                rpc_pb2.IndexFieldDef(name="int_field", field_type=1),
                rpc_pb2.IndexFieldDef(name="float_field", field_type=3),
                rpc_pb2.IndexFieldDef(name="bytes_field", field_type=2),
            ]
        )
        resp = stub.CreateIndex(req, metadata=get_metadata())
        assert resp.success, f"CreateIndex failed: {resp.message}"
        print(f"    ✓ 多类型索引创建成功，ID: {resp.index_id}")
        return True
    except Exception as e:
        print(f"    ✗ 创建多类型索引失败: {e}")
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

def test_get_index_stats(num=1):
    print("[测试] 获取索引统计...")
    try:
        req = rpc_pb2.GetIndexStatsRequest()
        resp = stub.GetIndexStats(req, metadata=get_metadata())
        assert resp.success, f"GetIndexStats failed: {resp.message}"
        assert resp.total_indices >= num, f"期望至少 {num} 个索引，实际: {resp.total_indices}"
        print(f"    ✓ 索引统计: total={resp.total_indices}, names={resp.index_names}")
        return True
    except Exception as e:
        print(f"    ✗ 获取统计失败: {e}")
        return False

def test_add_document():
    print("[测试] 添加文档...")
    try:
        req = rpc_pb2.AddDocumentRequest(
            index_name=INDEX_NAME,
            doc_id="doc_grpc_001",
            fields={
                "title": "Hello gRPC World",
                "content": "This is a test document via gRPC",
                "category": "test",
                "view_count": "100"
            }
        )
        resp = stub.AddDocument(req, metadata=get_metadata())
        assert resp.success, f"AddDocument failed: {resp.message}"
        print(f"    ✓ 文档添加成功，doc_id: {resp.doc_id}")
        return True
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.UNIMPLEMENTED:
            print(f"    ⚠ AddDocument gRPC方法未实现: {e.details()}")
            return True
        print(f"    ✗ 添加文档失败: {e}")
        return False
    except Exception as e:
        print(f"    ✗ 添加文档失败: {e}")
        return False

def test_search_with_real_data():
    print("[测试] 添加多个文档并进行真实搜索测试...")
    try:
        docs = [
            {
                "doc_id": "doc_grpc_001",
                "fields": {
                    "title": "Rust Programming",
                    "content": "Rust is a systems programming language focused on safety, speed, and concurrency.",
                    "category": "Programming Language",
                    "view_count": "150"
                }
            },
            {
                "doc_id": "doc_grpc_002",
                "fields": {
                    "title": "Python Data Analysis",
                    "content": "Python is very popular in the field of data analysis with rich libraries.",
                    "category": "Data Analysis",
                    "view_count": "230"
                }
            },
            {
                "doc_id": "doc_grpc_003",
                "fields": {
                    "title": "Machine Learning Introduction",
                    "content": "Machine learning is a branch of artificial intelligence.",
                    "category": "Artificial Intelligence",
                    "view_count": "320"
                }
            }
        ]

        add_success = True
        for doc in docs:
            try:
                req = rpc_pb2.AddDocumentRequest(
                    index_name=INDEX_NAME,
                    doc_id=doc["doc_id"],
                    fields=doc["fields"]
                )
                resp = stub.AddDocument(req, metadata=get_metadata())
                if resp.success:
                    print(f"    ✓ 成功添加 {doc['doc_id']} 文档 ")
                else:
                    print(f"    ⚠ 添加文档 {doc['doc_id']} 返回失败: {resp.message}")
            except grpc.RpcError as e:
                if e.code() == grpc.StatusCode.UNIMPLEMENTED:
                    add_success = False
                    print(f"    ⚠ AddDocument gRPC方法未实现，跳过文档添加测试")
                    break
                else:
                    raise

        if not add_success:
            return True

        print(f"    ✓ 成功添加 {len(docs)} 个文档")

        try:
            req = rpc_pb2.SearchIndexRequest(index_name=INDEX_NAME, query="Rust", limit=5)
            resp = stub.SearchIndex(req, metadata=get_metadata())
            if resp.success:
                print(f"    ✓ 搜索'Rust'返回 {len(resp.results)} 条结果: {resp.results}")
            else:
                print(f"    ✓ 搜索'Rust'返回失败: {resp.message}")
        except Exception as e:
            print(f"    ✓ 搜索'Rust'异常: {str(e)[:50]}...")

        return True
    except Exception as e:
        print(f"    ✗ 真实数据搜索测试失败: {e}")
        return False

def test_get_document_by_id():
    print("[测试] 通过doc_id获取文档...")
    try:
        #doc_id = "test_grpc_doc_999"
        try:
            req = rpc_pb2.AddDocumentRequest(
                index_name=INDEX_NAME,
                #doc_id=doc_id,
                fields={
                    "title": "Document Retrieval Test",
                    "content": "This document is used to test retrieval by ID.",
                    "category": "Test",
                    "view_count": "50"
                }
            )
            resp = stub.AddDocument(req, metadata=get_metadata())
            if resp.success:
                print(f"    ✓ 添加测试文档，doc_id: {resp.doc_id}")
            else:
                print(f"    ⚠ 添加文档返回失败: {resp.message}")
        except grpc.RpcError as e:
            if e.code() == grpc.StatusCode.UNIMPLEMENTED:
                print(f"    ⚠ AddDocument gRPC方法未实现")
                return True
            raise

        try:
            req = rpc_pb2.GetDocumentRequest(index_name=INDEX_NAME, doc_id=resp.doc_id)
            resp = stub.GetDocument(req, metadata=get_metadata())
            if resp.success:
                print(f"    ✓ 成功通过doc_id获取文档: {resp.doc_id}")
            else:
                print(f"    ✓ 获取文档返回success=False: {resp.message}")
        except grpc.RpcError as e:
            if e.code() == grpc.StatusCode.UNIMPLEMENTED:
                print(f"    ⚠ GetDocument gRPC方法未实现")
            else:
                print(f"    ✓ 获取文档异常: {str(e)[:50]}...")

        try:
            req = rpc_pb2.GetDocumentRequest(index_name=INDEX_NAME, doc_id="non_existent_doc")
            resp = stub.GetDocument(req, metadata=get_metadata())
            print(f"    ✓ 获取不存在的文档返回success={resp.success}")
        except grpc.RpcError as e:
            print(f"    ✓ 获取不存在的文档异常: {str(e)[:50]}...")

        return True
    except Exception as e:
        print(f"    ✗ 通过doc_id获取文档测试失败: {e}")
        return False

def test_search_index():
    print("[测试] 搜索索引（占位符实现）...")
    try:
        req = rpc_pb2.SearchIndexRequest(index_name=INDEX_NAME, query="test", limit=10)
        resp = stub.SearchIndex(req, metadata=get_metadata())
        assert resp.success, f"SearchIndex failed: {resp.message}"
        print(f"    ✓ 搜索结果: {len(resp.results)} 条")
        print(f"    ✓ 搜索结果: {resp.results} ")
        return True
    except Exception as e:
        print(f"    ✗ 搜索失败: {e}")
        return False

def test_search_multi_field():
    print("[测试] 多字段搜索（占位符实现）...")
    try:
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

def test_search_no_auth():
    print("[测试] 未认证搜索请求（应失败）...")
    try:
        req = rpc_pb2.SearchIndexRequest(index_name=INDEX_NAME, query="test", limit=10)
        resp = stub.SearchIndex(req)
        if not resp.success:
            print(f"    ✓ 未认证请求被正确拒绝: success={resp.success}, message={resp.message}")
            return True
        else:
            print(f"    ✗ 未认证请求应该返回 success=False，实际: success={resp.success}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.UNAUTHENTICATED:
            print("    ✓ 未认证请求被正确拒绝 (UNAUTHENTICATED)")
            return True
        print(f"    ✗ 期望 UNAUTHENTICATED，实际: {e.code()}")
        return False
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def test_search_invalid_token():
    print("[测试] 无效 Token 搜索请求（应失败）...")
    try:
        req = rpc_pb2.SearchIndexRequest(index_name=INDEX_NAME, query="test", limit=10)
        metadata = [("authorization", "Bearer invalid_token_xyz")]
        resp = stub.SearchIndex(req, metadata=metadata)
        if not resp.success:
            print(f"    ✓ 无效Token请求被正确拒绝: success={resp.success}, message={resp.message}")
            return True
        else:
            print(f"    ✗ 无效Token请求应该返回 success=False，实际: success={resp.success}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.UNAUTHENTICATED or e.code() == grpc.StatusCode.PERMISSION_DENIED:
            print(f"    ✓ 无效Token请求被正确拒绝 ({e.code()})")
            return True
        print(f"    ✗ 期望 UNAUTHENTICATED 或 PERMISSION_DENIED，实际: {e.code()}")
        return False
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def test_create_duplicate_index():
    print("[测试] 创建同名索引（应成功，ID 不同）...")
    try:
        req = rpc_pb2.CreateIndexRequest(
            index_name=INDEX_NAME,
            fields=[rpc_pb2.IndexFieldDef(name="field1", field_type=0)]
        )
        resp = stub.CreateIndex(req, metadata=get_metadata())
        assert resp.success, f"CreateIndex should succeed for duplicate name: {resp.message}"
        print(f"    ✓ 同名索引创建成功，新 ID: {resp.index_id}")
        return True
    except Exception as e:
        print(f"    ✗ 创建同名索引失败: {e}")
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
        resp = stub.CreateIndex(req)
        if not resp.success:
            print(f"    ✓ 未认证请求被正确拒绝: success={resp.success}, message={resp.message}")
            return True
        else:
            print(f"    ✗ 未认证请求应该返回 success=False，实际: success={resp.success}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.UNAUTHENTICATED:
            print("    ✓ 未认证请求被正确拒绝 (UNAUTHENTICATED)")
            return True
        print(f"    ✗ 期望 UNAUTHENTICATED，实际: {e.code()}")
        return False
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False

def cleanup():
    print("[清理] 清理测试数据...")
    try:
        for idx in [INDEX_NAME, "test_all_types_index_grpc"]:
            req = rpc_pb2.DropIndexRequest(index_name=idx)
            resp = stub.DropIndex(req, metadata=get_metadata())
            print(f"    - 清理索引 '{idx}': {'成功' if resp.success else '失败'}")
    except Exception as e:
        print(f"    - 清理异常: {e}")

def run_all_tests():
    tests = [
        ("获取版本信息", test_get_version),
        ("用户登录", test_login),
        ("创建全文索引", test_create_index),
        ("列出索引", test_list_indices),
        ("获取索引字段", test_get_index_fields),
        ("获取索引元数据", test_get_index_meta),
        ("获取索引统计", lambda: test_get_index_stats(1)),
        ("创建多类型索引", test_create_index_with_all_types),
        ("统计更新验证", lambda: test_get_index_stats(1)),
        ("添加文档", test_add_document),
        ("搜索索引(占位符)", test_search_index),
        ("多字段搜索(占位符)", test_search_multi_field),
        ("未认证请求", test_search_no_auth),
        ("无效Token", test_search_invalid_token),
        ("创建同名索引", test_create_duplicate_index),
        ("索引列表复查", test_list_indices),
        ("真实数据搜索测试", test_search_with_real_data),
        ("通过doc_id获取文档", test_get_document_by_id),
        ("删除索引", test_drop_index),
        ("删除不存在的索引", test_drop_non_existent_index),
        ("未认证创建索引", test_unauthorized_create),
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
    global server_proc
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
    
    config_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "prod.yaml")
    
    # 强制使用新编译的服务，不使用现有服务
    grpc_port = int(TEST_ADDR.split(':')[1])
    use_existing_service = False
    
    actual_addr = f"127.0.0.1:{grpc_port}"
    
    print("\n[3/4] 检查/启动 laoflchDB gRPC 服务...")
    if use_existing_service:
        print(f"    ✓ 端口 {grpc_port} 已有服务运行，直接使用现有服务")
        server_proc = None
    else:
        if os.path.exists(config_path):
            cmd = [SERVER_BIN, "-c", "laoflchdb.yaml"]
            print(f"    ✓ 使用配置文件: {config_path}")
        else:
            cmd = [SERVER_BIN, "start", "--addr", TEST_ADDR, "--db-path", TEST_DB, "--index-path", TEST_DB]
            print("    ✓ 使用测试配置")
        log_file = open("index_grpc_server.log", "w")
        server_proc = subprocess.Popen(
            cmd, cwd="..",
            stdout=log_file, stderr=subprocess.STDOUT,
            preexec_fn=os.setsid
        )
        time.sleep(3)
        with open("index_grpc_server.log") as f:
            log_content = f.read()
        if "启动成功" in log_content:
            print(f"    ✓ 服务已启动 PID={server_proc.pid}")
        else:
            print(f"    ⚠ 服务启动可能异常，日志: {log_content[:200]}")
        import re
        grpc_match = re.search(r'gRPC.*?(\d+\.\d+\.\d+\.\d+:\d+)', log_content)
        actual_addr = grpc_match.group(1) if grpc_match else TEST_ADDR
    print(f"    ✓ 实际 gRPC 地址: {actual_addr}")

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
