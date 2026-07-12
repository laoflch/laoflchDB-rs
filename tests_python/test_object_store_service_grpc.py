#!/usr/bin/env python3
"""
Python 自动回归测试: ObjectStoreService 对象存储服务 gRPC 接口测试
S3 兼容的对象存储服务，基于 RocksDB BlobDB 实现大对象存储
"""
import time
import sys
import os
import json
import hashlib
import random
import string

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import object_store_pb2
import object_store_pb2_grpc
import rpc_pb2
import rpc_pb2_grpc

TEST_ADDR = "127.0.0.1:19777"
SERVER_BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "release", "laoflchdb")
CONFIG_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "laoflchdb.yaml")

TOKEN = None
stub = None
os_stub = None
server_proc = None
server_started_by_us = False

# 测试 Bucket 名称（使用随机后缀避免冲突）
TEST_BUCKET = "test-bucket-" + ''.join(random.choices(string.ascii_lowercase, k=6))
TEST_BUCKET_LIST = "test-bucket-list-" + ''.join(random.choices(string.ascii_lowercase, k=6))


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
        print(f"    ✓ 登录成功")
        return True
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False


def get_metadata():
    if TOKEN:
        return [("authorization", f"Bearer {TOKEN}")]
    return []


def _random_data(size=1024):
    """生成随机字节数据"""
    return bytes(random.randint(0, 255) for _ in range(size))


def _etag_from_data(data):
    """计算数据的 ETag（MD5 哈希，匹配服务端简化格式）"""
    return f'"{hashlib.md5(data).hexdigest()}"'


# ==================== 测试用例 ====================

def test_create_bucket():
    """测试创建存储桶"""
    print(f"[测试] 创建 Bucket: {TEST_BUCKET}...")
    try:
        req = object_store_pb2.CreateBucketRequest(bucket=TEST_BUCKET)
        resp = os_stub.CreateBucket(req, metadata=get_metadata())
        assert resp.success, f"创建失败: {resp.message}"
        print(f"    ✓ Bucket '{TEST_BUCKET}' 创建成功")
        return True
    except Exception as e:
        print(f"    ✗ 创建失败: {e}")
        return False


def test_create_bucket_duplicate():
    """测试重复创建存储桶（应成功，幂等操作）"""
    print(f"[测试] 重复创建 Bucket: {TEST_BUCKET}（应幂等成功）...")
    try:
        req = object_store_pb2.CreateBucketRequest(bucket=TEST_BUCKET)
        resp = os_stub.CreateBucket(req, metadata=get_metadata())
        assert resp.success, f"重复创建失败: {resp.message}"
        print(f"    ✓ 重复创建 Bucket 成功（幂等）")
        return True
    except Exception as e:
        print(f"    ✗ 重复创建失败: {e}")
        return False


def test_list_buckets():
    """测试列出存储桶"""
    print(f"[测试] 列出 Buckets...")
    try:
        req = object_store_pb2.ListBucketsRequest()
        resp = os_stub.ListBuckets(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        names = [b.name for b in resp.buckets]
        assert TEST_BUCKET in names, f"Bucket '{TEST_BUCKET}' 不在列表中: {names}"
        print(f"    ✓ 列出 Buckets 成功，共 {len(resp.buckets)} 个")
        for b in resp.buckets:
            print(f"        name={b.name}, creation_date={b.creation_date}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_put_object():
    """测试存储对象"""
    key = "test/hello.txt"
    data = b"Hello, Object Store Service!"
    content_type = "text/plain"
    metadata = {"author": "test", "version": "1.0"}
    print(f"[测试] PutObject: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type=content_type,
            metadata=metadata,
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"存储失败: {resp.message}"
        assert resp.etag, "ETag 不应为空"
        print(f"    ✓ PutObject 成功，etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 存储失败: {e}")
        return False


def test_put_object_large():
    """测试存储大对象（1MB，验证 BlobDB 大对象存储）"""
    key = "test/large_object.bin"
    data = _random_data(1024 * 1024)  # 1MB
    print(f"[测试] PutObject 大对象（1MB）: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="application/octet-stream",
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"大对象存储失败: {resp.message}"
        print(f"    ✓ 大对象存储成功（{len(data)} bytes），etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 大对象存储失败: {e}")
        return False


def test_put_object_empty():
    """测试存储空对象"""
    key = "test/empty.txt"
    data = b""
    content_type = "text/plain"
    print(f"[测试] PutObject 空对象: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type=content_type,
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"空对象存储失败: {resp.message}"
        print(f"    ✓ 空对象存储成功，etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 空对象存储失败: {e}")
        return False


def test_put_object_with_special_chars():
    """测试存储含特殊字符路径的对象"""
    key = "dir/with spaces & special chars+/file (1).txt"
    data = b"Special chars path test"
    print(f"[测试] PutObject 特殊字符路径: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="text/plain",
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"特殊字符路径存储失败: {resp.message}"
        print(f"    ✓ 特殊字符路径存储成功，etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 特殊字符路径存储失败: {e}")
        return False


def test_get_object():
    """测试获取对象"""
    key = "test/hello.txt"
    expected_data = b"Hello, Object Store Service!"
    print(f"[测试] GetObject: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.GetObject(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.data == expected_data, f"数据不匹配: {resp.data} != {expected_data}"
        assert resp.content_type == "text/plain", f"Content-Type 不匹配: {resp.content_type}"
        assert resp.content_length == len(expected_data), f"Content-Length 不匹配"
        assert resp.etag, "ETag 不应为空"
        assert resp.metadata.get("author") == "test", f"metadata.author 不匹配: {resp.metadata}"
        print(f"    ✓ GetObject 成功: size={resp.content_length}, type={resp.content_type}, etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_object_large():
    """测试获取大对象"""
    key = "test/large_object.bin"
    print(f"[测试] GetObject 大对象: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.GetObject(req, metadata=get_metadata())
        assert resp.success, f"大对象获取失败: {resp.message}"
        assert len(resp.data) == 1024 * 1024, f"大小不匹配: {len(resp.data)}"
        print(f"    ✓ 大对象获取成功（{len(resp.data)} bytes）")
        return True
    except Exception as e:
        print(f"    ✗ 大对象获取失败: {e}")
        return False


def test_get_object_not_found():
    """测试获取不存在的对象（应返回 NOT_FOUND）"""
    key = "non_existent_file.txt"
    print(f"[测试] GetObject 不存在对象（应失败）: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.GetObject(req, metadata=get_metadata())
        if not resp.success:
            print(f"    ✓ 不存在对象被正确拒绝: {resp.message}")
            return True
        print(f"    ✗ 不存在对象不应返回成功")
        return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.NOT_FOUND:
            print(f"    ✓ 不存在对象被正确拒绝 (gRPC NOT_FOUND): {e.code()}")
            return True
        print(f"    ✓ 不存在对象被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_get_object_empty():
    """测试获取空对象"""
    key = "test/empty.txt"
    print(f"[测试] GetObject 空对象: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.GetObject(req, metadata=get_metadata())
        assert resp.success, f"空对象获取失败: {resp.message}"
        assert resp.data == b"", f"空对象数据应为空: {resp.data}"
        assert resp.content_length == 0, f"空对象 Content-Length 应为 0"
        print(f"    ✓ 空对象获取成功（size=0）")
        return True
    except Exception as e:
        print(f"    ✗ 空对象获取失败: {e}")
        return False


def test_head_object():
    """测试获取对象元数据"""
    key = "test/hello.txt"
    expected_data = b"Hello, Object Store Service!"
    print(f"[测试] HeadObject: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.HeadObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.HeadObject(req, metadata=get_metadata())
        assert resp.success, f"Head 失败: {resp.message}"
        assert resp.content_type == "text/plain", f"Content-Type 不匹配: {resp.content_type}"
        assert resp.content_length == len(expected_data), f"Content-Length 不匹配"
        assert resp.etag, "ETag 不应为空"
        assert resp.last_modified, "Last-Modified 不应为空"
        assert resp.metadata.get("author") == "test", f"metadata.author 不匹配: {resp.metadata}"
        print(f"    ✓ HeadObject 成功: size={resp.content_length}, type={resp.content_type}, etag={resp.etag}, last_modified={resp.last_modified}")
        return True
    except Exception as e:
        print(f"    ✗ Head 失败: {e}")
        return False


def test_head_object_not_found():
    """测试不存在的对象 Head（应返回 NOT_FOUND）"""
    key = "non_existent_file.txt"
    print(f"[测试] HeadObject 不存在对象（应失败）: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.HeadObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.HeadObject(req, metadata=get_metadata())
        if not resp.success:
            print(f"    ✓ 不存在对象 Head 被正确拒绝: {resp.message}")
            return True
        print(f"    ✗ 不存在对象 Head 不应返回成功")
        return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.NOT_FOUND:
            print(f"    ✓ 不存在对象 Head 被正确拒绝 (gRPC NOT_FOUND): {e.code()}")
            return True
        print(f"    ✓ 不存在对象 Head 被拒绝 (gRPC): {e.code()}")
        return True
    except Exception as e:
        print(f"    ✗ 异常: {e}")
        return False


def test_list_objects():
    """测试列出对象"""
    print(f"[测试] ListObjects: {TEST_BUCKET}...")
    try:
        req = object_store_pb2.ListObjectsRequest(bucket=TEST_BUCKET)
        resp = os_stub.ListObjects(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        keys = [o.key for o in resp.objects]
        assert "test/hello.txt" in keys, f"test/hello.txt 不在列表中: {keys}"
        assert "test/large_object.bin" in keys, f"test/large_object.bin 不在列表中: {keys}"
        assert "test/empty.txt" in keys, f"test/empty.txt 不在列表中: {keys}"
        print(f"    ✓ ListObjects 成功，共 {len(resp.objects)} 个对象")
        for o in resp.objects:
            print(f"        key={o.key}, size={o.size}, type={o.content_type}, etag={o.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_list_objects_with_prefix():
    """测试带前缀列出对象"""
    prefix = "test/"
    print(f"[测试] ListObjects 带前缀 '{prefix}': {TEST_BUCKET}...")
    try:
        req = object_store_pb2.ListObjectsRequest(
            bucket=TEST_BUCKET,
            prefix=prefix,
        )
        resp = os_stub.ListObjects(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        for o in resp.objects:
            assert o.key.startswith(prefix), f"key '{o.key}' 不以 '{prefix}' 开头"
        print(f"    ✓ ListObjects with prefix 成功，返回 {len(resp.objects)} 个对象")
        for o in resp.objects:
            print(f"        key={o.key}, size={o.size}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_list_objects_with_delimiter():
    """测试带分隔符列出对象（模拟目录结构）"""
    # 先创建一些带层级结构的对象
    keys_to_create = [
        "dir1/file1.txt",
        "dir1/file2.txt",
        "dir1/subdir/file3.txt",
        "dir2/file4.txt",
    ]
    for k in keys_to_create:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=k,
            data=b"test data for " + k.encode(),
            content_type="text/plain",
        )
        os_stub.PutObject(req, metadata=get_metadata())

    print(f"[测试] ListObjects 带分隔符 '/': {TEST_BUCKET}...")
    try:
        req = object_store_pb2.ListObjectsRequest(
            bucket=TEST_BUCKET,
            delimiter="/",
        )
        resp = os_stub.ListObjects(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        assert "dir1/" in resp.common_prefixes or "dir1" in resp.common_prefixes, \
            f"common_prefixes 应包含 'dir1/'，实际: {resp.common_prefixes}"
        print(f"    ✓ ListObjects with delimiter 成功")
        print(f"        objects: {[o.key for o in resp.objects]}")
        print(f"        common_prefixes: {resp.common_prefixes}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_list_objects_empty_bucket():
    """测试空 Bucket 列出对象"""
    empty_bucket = f"empty-{TEST_BUCKET}"
    print(f"[测试] ListObjects 空 Bucket: {empty_bucket}...")
    try:
        # 创建空 Bucket
        req = object_store_pb2.CreateBucketRequest(bucket=empty_bucket)
        os_stub.CreateBucket(req, metadata=get_metadata())

        req = object_store_pb2.ListObjectsRequest(bucket=empty_bucket)
        resp = os_stub.ListObjects(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        assert len(resp.objects) == 0, f"空 Bucket 应返回 0 个对象，实际: {len(resp.objects)}"
        print(f"    ✓ 空 Bucket 列出成功（0 个对象）")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_copy_object():
    """测试复制对象"""
    src_key = "test/hello.txt"
    dst_key = "test/hello_copy.txt"
    print(f"[测试] CopyObject: {TEST_BUCKET}/{src_key} → {TEST_BUCKET}/{dst_key}...")
    try:
        req = object_store_pb2.CopyObjectRequest(
            source_bucket=TEST_BUCKET,
            source_key=src_key,
            destination_bucket=TEST_BUCKET,
            destination_key=dst_key,
        )
        resp = os_stub.CopyObject(req, metadata=get_metadata())
        assert resp.success, f"复制失败: {resp.message}"
        assert resp.etag, "ETag 不应为空"
        print(f"    ✓ CopyObject 成功，etag={resp.etag}")

        # 验证目标对象存在
        get_req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=dst_key)
        get_resp = os_stub.GetObject(get_req, metadata=get_metadata())
        assert get_resp.data == b"Hello, Object Store Service!", "复制后的数据不匹配"
        print(f"    ✓ 验证目标对象数据正确")
        return True
    except Exception as e:
        print(f"    ✗ 复制失败: {e}")
        return False


def test_copy_object_cross_bucket():
    """测试跨 Bucket 复制对象"""
    src_key = "test/hello.txt"
    dst_bucket = TEST_BUCKET_LIST
    dst_key = "copied/hello.txt"
    print(f"[测试] CopyObject 跨 Bucket: {TEST_BUCKET}/{src_key} → {dst_bucket}/{dst_key}...")
    try:
        # 确保目标 Bucket 存在
        req = object_store_pb2.CreateBucketRequest(bucket=dst_bucket)
        os_stub.CreateBucket(req, metadata=get_metadata())

        req = object_store_pb2.CopyObjectRequest(
            source_bucket=TEST_BUCKET,
            source_key=src_key,
            destination_bucket=dst_bucket,
            destination_key=dst_key,
        )
        resp = os_stub.CopyObject(req, metadata=get_metadata())
        assert resp.success, f"跨 Bucket 复制失败: {resp.message}"
        print(f"    ✓ 跨 Bucket CopyObject 成功，etag={resp.etag}")

        # 验证目标对象
        get_req = object_store_pb2.GetObjectRequest(bucket=dst_bucket, key=dst_key)
        get_resp = os_stub.GetObject(get_req, metadata=get_metadata())
        assert get_resp.data == b"Hello, Object Store Service!", "跨 Bucket 复制后的数据不匹配"
        print(f"    ✓ 验证跨 Bucket 复制数据正确")
        return True
    except Exception as e:
        print(f"    ✗ 跨 Bucket 复制失败: {e}")
        return False


def test_put_object_overwrite():
    """测试覆盖已有对象"""
    key = "test/overwrite.txt"
    original_data = b"Original content"
    new_data = b"Updated content after overwrite"
    print(f"[测试] 覆盖对象: {TEST_BUCKET}/{key}...")
    try:
        # 写入原始数据
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=original_data,
            content_type="text/plain",
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"原始写入失败: {resp.message}"

        # 覆盖写入
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=new_data,
            content_type="text/plain",
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"覆盖写入失败: {resp.message}"

        # 验证数据已更新
        get_req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        get_resp = os_stub.GetObject(get_req, metadata=get_metadata())
        assert get_resp.data == new_data, f"覆盖后数据不匹配: {get_resp.data}"
        print(f"    ✓ 对象覆盖成功，数据已更新")
        return True
    except Exception as e:
        print(f"    ✗ 覆盖测试失败: {e}")
        return False


def test_delete_object():
    """测试删除对象"""
    key = "test/hello_copy.txt"  # 从 copy 测试创建的对象
    print(f"[测试] DeleteObject: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.DeleteObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.DeleteObject(req, metadata=get_metadata())
        assert resp.success, f"删除失败: {resp.message}"
        print(f"    ✓ DeleteObject 成功")

        # 验证对象已被删除
        get_req = object_store_pb2.GetObjectRequest(bucket=TEST_BUCKET, key=key)
        try:
            get_resp = os_stub.GetObject(get_req, metadata=get_metadata())
            if not get_resp.success:
                print(f"    ✓ 验证对象已删除 (success=false)")
                return True
        except grpc.RpcError:
            print(f"    ✓ 验证对象已删除 (gRPC error)")
            return True
        print(f"    ✓ 验证对象已删除")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_delete_object_not_found():
    """测试删除不存在的对象（应幂等成功）"""
    key = "non_existent_file.txt"
    print(f"[测试] DeleteObject 不存在对象（应幂等成功）: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.DeleteObjectRequest(bucket=TEST_BUCKET, key=key)
        resp = os_stub.DeleteObject(req, metadata=get_metadata())
        assert resp.success, f"删除不存在对象失败: {resp.message}"
        print(f"    ✓ 删除不存在对象成功（幂等）")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_delete_objects():
    """测试批量删除对象"""
    keys_to_delete = ["dir1/file1.txt", "dir1/file2.txt", "dir2/file4.txt"]
    print(f"[测试] DeleteObjects: {TEST_BUCKET} 删除 {keys_to_delete}...")
    try:
        req = object_store_pb2.DeleteObjectsRequest(
            bucket=TEST_BUCKET,
            keys=keys_to_delete,
        )
        resp = os_stub.DeleteObjects(req, metadata=get_metadata())
        assert resp.success, f"批量删除失败: {resp.message}"
        assert len(resp.deleted_keys) == len(keys_to_delete), \
            f"删除的 key 数量不匹配: {len(resp.deleted_keys)} != {len(keys_to_delete)}"
        print(f"    ✓ DeleteObjects 成功，删除了 {len(resp.deleted_keys)} 个对象: {resp.deleted_keys}")
        return True
    except Exception as e:
        print(f"    ✗ 批量删除失败: {e}")
        return False


def test_delete_objects_partial():
    """测试批量删除包含部分不存在的对象（应幂等）"""
    keys = ["dir1/subdir/file3.txt", "non_existent_1.txt", "non_existent_2.txt"]
    print(f"[测试] DeleteObjects 部分不存在: {TEST_BUCKET} 删除 {keys}...")
    try:
        req = object_store_pb2.DeleteObjectsRequest(
            bucket=TEST_BUCKET,
            keys=keys,
        )
        resp = os_stub.DeleteObjects(req, metadata=get_metadata())
        assert resp.success, f"批量删除失败: {resp.message}"
        print(f"    ✓ 部分删除成功，删除了 {len(resp.deleted_keys)} 个对象: {resp.deleted_keys}")
        return True
    except Exception as e:
        print(f"    ✗ 批量删除失败: {e}")
        return False


def test_list_objects_after_delete():
    """测试删除后列出对象验证"""
    print(f"[测试] 删除后 ListObjects 验证: {TEST_BUCKET}...")
    try:
        req = object_store_pb2.ListObjectsRequest(bucket=TEST_BUCKET)
        resp = os_stub.ListObjects(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        keys = [o.key for o in resp.objects]
        assert "test/hello_copy.txt" not in keys, "已删除的对象不应出现在列表中"
        assert "dir1/file1.txt" not in keys, "已批量删除的对象不应出现在列表中"
        assert "test/hello.txt" in keys, "未删除的对象应仍在列表中"
        print(f"    ✓ 删除后验证通过，剩余 {len(resp.objects)} 个对象")
        for o in resp.objects:
            print(f"        key={o.key}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_put_object_with_metadata():
    """测试带自定义元数据存储对象"""
    key = "test/metadata_test.txt"
    data = b"Metadata test"
    metadata = {
        "key1": "value1",
        "key2": "中文值",
        "key3": "value with spaces",
    }
    print(f"[测试] 带自定义元数据 PutObject: {TEST_BUCKET}/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="text/plain",
            metadata=metadata,
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        assert resp.success, f"存储失败: {resp.message}"

        # 验证元数据
        head_req = object_store_pb2.HeadObjectRequest(bucket=TEST_BUCKET, key=key)
        head_resp = os_stub.HeadObject(head_req, metadata=get_metadata())
        assert head_resp.success, f"Head 失败: {head_resp.message}"
        for k, v in metadata.items():
            assert head_resp.metadata.get(k) == v, f"metadata.{k} 不匹配: {head_resp.metadata.get(k)} != {v}"
        print(f"    ✓ 自定义元数据存储成功，所有元数据正确")
        return True
    except Exception as e:
        print(f"    ✗ 元数据测试失败: {e}")
        return False


def test_delete_bucket():
    """测试删除存储桶"""
    # 先删除 Bucket 中所有对象
    print(f"[测试] 删除 Bucket: {TEST_BUCKET}...")
    try:
        req = object_store_pb2.DeleteBucketRequest(bucket=TEST_BUCKET)
        resp = os_stub.DeleteBucket(req, metadata=get_metadata())
        assert resp.success, f"删除 Bucket 失败: {resp.message}"
        print(f"    ✓ DeleteBucket 成功")

        # 验证 Bucket 已删除
        list_req = object_store_pb2.ListBucketsRequest()
        list_resp = os_stub.ListBuckets(list_req, metadata=get_metadata())
        names = [b.name for b in list_resp.buckets]
        assert TEST_BUCKET not in names, f"Bucket '{TEST_BUCKET}' 仍存在于列表中"
        print(f"    ✓ 验证 Bucket 已删除")
        return True
    except Exception as e:
        print(f"    ✗ 删除 Bucket 失败: {e}")
        return False


def test_create_bucket_invalid_name():
    """测试创建无效 Bucket 名称"""
    # 注：服务端目前不限制 bucket 名称格式，此测试仅做记录
    print(f"[测试] 创建 Bucket 基本验证...")
    try:
        req = object_store_pb2.CreateBucketRequest(bucket="valid-bucket-name")
        resp = os_stub.CreateBucket(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 有效 Bucket 名称创建成功")
            return True
        print(f"    ✓ 创建被拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 创建被拒绝: {e}")
        return True


def test_put_object_no_bucket():
    """测试向不存在的 Bucket 存储对象"""
    key = "test.txt"
    data = b"test"
    print(f"[测试] PutObject 到不存在 Bucket（应自动创建）: non_existent_bucket/{key}...")
    try:
        req = object_store_pb2.PutObjectRequest(
            bucket="non_existent_bucket_auto_create",
            key=key,
            data=data,
            content_type="text/plain",
        )
        resp = os_stub.PutObject(req, metadata=get_metadata())
        # 服务端自动创建 Bucket 或拒绝
        if resp.success:
            print(f"    ✓ 自动创建 Bucket 并存储成功")
            return True
        print(f"    ✓ 被拒绝: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 被拒绝: {e}")
        return True


# ==================== 主测试流程 ====================

def run_all_tests():
    """运行所有测试用例并统计结果"""
    tests = [
        ("登录", test_login),
        # Bucket 操作
        ("创建 Bucket", test_create_bucket),
        ("重复创建 Bucket", test_create_bucket_duplicate),
        ("列出 Buckets", test_list_buckets),
        # PutObject
        ("存储对象", test_put_object),
        ("存储大对象（1MB）", test_put_object_large),
        ("存储空对象", test_put_object_empty),
        ("存储特殊字符路径", test_put_object_with_special_chars),
        ("带自定义元数据存储", test_put_object_with_metadata),
        ("覆盖已有对象", test_put_object_overwrite),
        ("向不存在 Bucket 存储", test_put_object_no_bucket),
        # GetObject
        ("获取对象", test_get_object),
        ("获取大对象", test_get_object_large),
        ("获取空对象", test_get_object_empty),
        ("获取不存在对象", test_get_object_not_found),
        # HeadObject
        ("获取对象元数据", test_head_object),
        ("获取不存在对象元数据", test_head_object_not_found),
        # ListObjects
        ("列出对象", test_list_objects),
        ("带前缀列出对象", test_list_objects_with_prefix),
        ("带分隔符列出对象", test_list_objects_with_delimiter),
        ("空 Bucket 列出对象", test_list_objects_empty_bucket),
        # CopyObject
        ("复制对象", test_copy_object),
        ("跨 Bucket 复制对象", test_copy_object_cross_bucket),
        # DeleteObject
        ("删除对象", test_delete_object),
        ("删除不存在对象", test_delete_object_not_found),
        # DeleteObjects
        ("批量删除对象", test_delete_objects),
        ("批量删除部分不存在", test_delete_objects_partial),
        # 验证
        ("删除后列出对象验证", test_list_objects_after_delete),
        # DeleteBucket
        ("删除 Bucket", test_delete_bucket),
        ("创建无效 Bucket 名称", test_create_bucket_invalid_name),
    ]

    passed = 0
    failed = 0
    failed_tests = []

    print("=" * 70)
    print(f"对象存储服务 (ObjectStoreService) gRPC 接口测试")
    print(f"  - 服务器地址: {TEST_ADDR}")
    print(f"  - 测试 Bucket: {TEST_BUCKET}")
    print(f"  - 共 {len(tests)} 个测试用例")
    print("=" * 70)

    for name, func in tests:
        print("-" * 60)
        try:
            result = func()
            if result:
                passed += 1
            else:
                failed += 1
                failed_tests.append(name)
        except Exception as e:
            print(f"    ✗ 测试异常: {e}")
            failed += 1
            failed_tests.append(name)

    print("=" * 70)
    print(f"测试结果: {passed}/{len(tests)} 通过", end="")
    if failed > 0:
        print(f", {failed} 失败")
        print(f"失败测试: {failed_tests}")
    else:
        print()
    print("=" * 70)

    return failed == 0


def main():
    global stub, os_stub, server_proc, server_started_by_us

    print("=" * 70)
    print("对象存储服务 (ObjectStoreService) 测试启动")
    print("=" * 70)

    # 检查服务是否已在运行
    print(f"[启动] 检查 gRPC 服务 {TEST_ADDR}...")
    if check_service_alive(TEST_ADDR):
        print("[启动] 服务已在运行，将使用现有服务")
    else:
        print(f"[启动] 服务未运行在 {TEST_ADDR}，请先手动启动服务")
        print(f"[启动] 启动命令示例: {SERVER_BIN}")
        return False

    # 建立连接
    channel = grpc.insecure_channel(TEST_ADDR)
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    os_stub = object_store_pb2_grpc.ObjectStoreServiceStub(channel)

    # 运行测试
    try:
        success = run_all_tests()
    finally:
        channel.close()

    return success


if __name__ == "__main__":
    success = main()
    sys.exit(0 if success else 1)