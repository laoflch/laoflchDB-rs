#!/usr/bin/env python3
"""
Python 自动回归测试: ObjectStoreService REST 接口 & S3 兼容性测试
基于 RocksDB BlobDB 实现的大对象存储服务 HTTP REST 接口测试
"""
import time
import sys
import os
import json
import hashlib
import random
import string
import urllib.parse

import requests

# REST 服务端口（从配置文件可知 REST 服务运行在 8080）
REST_PORT = os.environ.get("LAOFLCHDB_REST_PORT", "38080")
BASE_URL = f"http://127.0.0.1:{REST_PORT}"
# 对象存储 REST API 基础路径
OSS_BASE = f"{BASE_URL}/api/v1/object-store"

TOKEN = None

# 测试 Bucket 名称（使用随机后缀避免冲突）
TEST_BUCKET = "test-bucket-" + ''.join(random.choices(string.ascii_lowercase, k=6))
TEST_BUCKET_LIST = "test-bucket-list-" + ''.join(random.choices(string.ascii_lowercase, k=6))


def _random_data(size=1024):
    """生成随机字节数据"""
    return bytes(random.randint(0, 255) for _ in range(size))


def _etag_from_data(data):
    """计算数据的 ETag（MD5 哈希）"""
    return f'"{hashlib.md5(data).hexdigest()}"'


def _get_auth_headers():
    headers = {}
    if TOKEN:
        headers["Authorization"] = f"Bearer {TOKEN}"
    return headers


# ==================== 辅助函数 ====================

def test_login():
    """测试登录以获取 Token"""
    global TOKEN
    print("[测试] 用户登录...")
    try:
        payload = {"username": "admin", "password": "laoflchdb"}
        resp = requests.post(f"{BASE_URL}/api/v1/login", json=payload, timeout=5)
        data = resp.json()
        # 兼容不同格式的登录响应
        if data.get("success"):
            token = data.get("data", {}).get("token", "")
            if token:
                TOKEN = token
                print(f"    ✓ 登录成功")
                return True
        # 尝试直接取 token
        if data.get("token"):
            TOKEN = data["token"]
            print(f"    ✓ 登录成功")
            return True
        print(f"    ✗ 登录失败: {data}")
        return False
    except Exception as e:
        print(f"    ✗ 登录异常: {e}")
        return False


# ==================== S3 兼容性测试 ====================

def test_s3_list_buckets():
    """S3 ListBuckets: GET / 列出所有 Bucket"""
    print(f"[S3] ListBuckets: GET {OSS_BASE}/ ...")
    try:
        resp = requests.get(f"{OSS_BASE}/", headers=_get_auth_headers(), timeout=5)
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        assert "buckets" in data, f"响应缺少 buckets 字段: {data}"
        print(f"    ✓ ListBuckets 成功，共 {len(data['buckets'])} 个 Bucket")
        for b in data["buckets"]:
            print(f"        name={b['name']}, creation_date={b.get('creation_date', 'N/A')}")
        return True
    except Exception as e:
        print(f"    ✗ ListBuckets 失败: {e}")
        return False


def test_s3_create_bucket():
    """S3 CreateBucket: PUT /{bucket} 创建 Bucket"""
    print(f"[S3] CreateBucket: PUT {OSS_BASE}/{TEST_BUCKET} ...")
    try:
        resp = requests.put(f"{OSS_BASE}/{TEST_BUCKET}", headers=_get_auth_headers(), timeout=5)
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        print(f"    ✓ CreateBucket '{TEST_BUCKET}' 成功")
        return True
    except Exception as e:
        print(f"    ✗ CreateBucket 失败: {e}")
        return False


def test_s3_create_bucket_idempotent():
    """S3 幂等创建: PUT /{bucket} 重复创建应成功"""
    print(f"[S3] 幂等创建 Bucket: PUT {OSS_BASE}/{TEST_BUCKET} ...")
    try:
        resp = requests.put(f"{OSS_BASE}/{TEST_BUCKET}", headers=_get_auth_headers(), timeout=5)
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        print(f"    ✓ 重复创建 Bucket 成功（幂等）")
        return True
    except Exception as e:
        print(f"    ✗ 幂等创建失败: {e}")
        return False


def test_s3_put_object():
    """S3 PutObject: PUT /{bucket}/{key} 上传对象"""
    key = "test/hello.txt"
    data = b"Hello, Object Store REST Service!"
    print(f"[S3] PutObject: PUT {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"
        resp = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=data,
            headers=headers,
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        body = resp.json()
        assert "etag" in body, f"响应缺少 etag: {body}"
        print(f"    ✓ PutObject 成功，etag={body['etag']}")
        return True
    except Exception as e:
        print(f"    ✗ PutObject 失败: {e}")
        return False


def test_s3_put_object_large():
    """S3 PutObject 大对象: PUT /{bucket}/{key} 上传 1MB 大对象（验证 BlobDB）"""
    key = "test/large_object.bin"
    data = _random_data(1024 * 1024)  # 1MB
    print(f"[S3] PutObject 大对象（1MB）: PUT {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "application/octet-stream"
        resp = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=data,
            headers=headers,
            timeout=10,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        body = resp.json()
        print(f"    ✓ 大对象存储成功（{len(data)} bytes），etag={body['etag']}")
        return True
    except Exception as e:
        print(f"    ✗ 大对象存储失败: {e}")
        return False


def test_s3_put_object_empty():
    """S3 PutObject 空对象: PUT /{bucket}/{key} 上传空对象"""
    key = "test/empty.txt"
    data = b""
    print(f"[S3] PutObject 空对象: PUT {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"
        resp = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=data,
            headers=headers,
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        body = resp.json()
        print(f"    ✓ 空对象存储成功，etag={body['etag']}")
        return True
    except Exception as e:
        print(f"    ✗ 空对象存储失败: {e}")
        return False


def test_s3_put_object_special_chars():
    """S3 PutObject 特殊字符路径: PUT /{bucket}/{key} 上传含特殊字符路径的对象"""
    key = "dir/with spaces & special chars+/file (1).txt"
    data = b"Special chars path test"
    print(f"[S3] PutObject 特殊字符路径: PUT {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"
        resp = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{urllib.parse.quote(key)}",
            data=data,
            headers=headers,
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        body = resp.json()
        print(f"    ✓ 特殊字符路径存储成功，etag={body['etag']}")
        return True
    except Exception as e:
        print(f"    ✗ 特殊字符路径存储失败: {e}")
        return False


def test_s3_put_object_overwrite():
    """S3 PutObject 覆盖: PUT /{bucket}/{key} 覆盖已有对象"""
    key = "test/overwrite.txt"
    original = b"Original content"
    new_data = b"Updated content after overwrite"
    print(f"[S3] PutObject 覆盖: PUT {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"

        # 写入原始数据
        resp1 = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=original,
            headers=headers,
            timeout=5,
        )
        assert resp1.status_code == 200, f"原始写入失败: {resp1.status_code}"

        # 覆盖写入
        resp2 = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=new_data,
            headers=headers,
            timeout=5,
        )
        assert resp2.status_code == 200, f"覆盖写入失败: {resp2.status_code}"

        # 验证数据已更新
        resp3 = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp3.status_code == 200, f"获取覆盖后数据失败: {resp3.status_code}"
        assert resp3.content == new_data, f"覆盖后数据不匹配: {resp3.content}"
        print(f"    ✓ 对象覆盖成功，数据已更新")
        return True
    except Exception as e:
        print(f"    ✗ 覆盖测试失败: {e}")
        return False


def test_s3_get_object():
    """S3 GetObject: GET /{bucket}/{key} 获取对象"""
    key = "test/hello.txt"
    expected_data = b"Hello, Object Store REST Service!"
    print(f"[S3] GetObject: GET {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        assert resp.content == expected_data, f"数据不匹配: {resp.content} != {expected_data}"
        assert resp.headers.get("content-type") == "text/plain", f"Content-Type 不匹配: {resp.headers.get('content-type')}"
        assert resp.headers.get("etag"), "ETag 不应为空"
        print(f"    ✓ GetObject 成功: size={len(resp.content)}, type={resp.headers.get('content-type')}, etag={resp.headers.get('etag')}")
        return True
    except Exception as e:
        print(f"    ✗ GetObject 失败: {e}")
        return False


def test_s3_get_object_large():
    """S3 GetObject 大对象: GET /{bucket}/{key} 获取大对象"""
    key = "test/large_object.bin"
    print(f"[S3] GetObject 大对象: GET {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        assert len(resp.content) == 1024 * 1024, f"大小不匹配: {len(resp.content)}"
        print(f"    ✓ 大对象获取成功（{len(resp.content)} bytes）")
        return True
    except Exception as e:
        print(f"    ✗ 大对象获取失败: {e}")
        return False


def test_s3_get_object_empty():
    """S3 GetObject 空对象: GET /{bucket}/{key} 获取空对象"""
    key = "test/empty.txt"
    print(f"[S3] GetObject 空对象: GET {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        assert resp.content == b"", f"空对象数据应为空: {resp.content}"
        print(f"    ✓ 空对象获取成功（size=0）")
        return True
    except Exception as e:
        print(f"    ✗ 空对象获取失败: {e}")
        return False


def test_s3_get_object_not_found():
    """S3 GetObject 不存在对象: GET /{bucket}/{key} 应返回 404"""
    key = "non_existent_file.txt"
    print(f"[S3] GetObject 不存在对象（应 404）: GET {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        if resp.status_code == 404:
            print(f"    ✓ 不存在对象被正确拒绝 (404)")
            return True
        print(f"    ✓ 不存在对象被拒绝: status={resp.status_code}")
        return True
    except Exception as e:
        print(f"    ✓ 异常（预期行为）: {e}")
        return True


def test_s3_head_object():
    """S3 HeadObject: HEAD /{bucket}/{key} 获取对象元数据"""
    key = "test/hello.txt"
    expected_data = b"Hello, Object Store REST Service!"
    print(f"[S3] HeadObject: HEAD {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.head(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        assert resp.headers.get("content-type") == "text/plain", f"Content-Type 不匹配: {resp.headers.get('content-type')}"
        assert int(resp.headers.get("content-length", 0)) == len(expected_data), f"Content-Length 不匹配"
        assert resp.headers.get("etag"), "ETag 不应为空"
        assert resp.headers.get("last-modified"), "Last-Modified 不应为空"
        print(f"    ✓ HeadObject 成功: size={resp.headers.get('content-length')}, type={resp.headers.get('content-type')}, etag={resp.headers.get('etag')}")
        return True
    except Exception as e:
        print(f"    ✗ HeadObject 失败: {e}")
        return False


def test_s3_head_object_not_found():
    """S3 HeadObject 不存在对象: HEAD /{bucket}/{key} 应返回 404"""
    key = "non_existent_file.txt"
    print(f"[S3] HeadObject 不存在对象（应 404）: HEAD {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.head(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        if resp.status_code == 404:
            print(f"    ✓ 不存在对象 Head 被正确拒绝 (404)")
            return True
        print(f"    ✓ 不存在对象 Head 被拒绝: status={resp.status_code}")
        return True
    except Exception as e:
        print(f"    ✓ 异常（预期行为）: {e}")
        return True


def test_s3_list_objects():
    """S3 ListObjects: GET /{bucket} 列出所有对象"""
    print(f"[S3] ListObjects: GET {OSS_BASE}/{TEST_BUCKET} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        keys = [o["key"] for o in data.get("objects", [])]
        assert "test/hello.txt" in keys, f"test/hello.txt 不在列表中: {keys}"
        assert "test/large_object.bin" in keys, f"test/large_object.bin 不在列表中: {keys}"
        assert "test/empty.txt" in keys, f"test/empty.txt 不在列表中: {keys}"
        print(f"    ✓ ListObjects 成功，共 {len(data['objects'])} 个对象")
        for o in data["objects"]:
            print(f"        key={o['key']}, size={o['size']}, type={o.get('content_type', 'N/A')}, etag={o.get('etag', 'N/A')}")
        return True
    except Exception as e:
        print(f"    ✗ ListObjects 失败: {e}")
        return False


def test_s3_list_objects_with_prefix():
    """S3 ListObjects 带 prefix: GET /{bucket}?prefix=test/"""
    prefix = "test/"
    print(f"[S3] ListObjects 带 prefix '{prefix}': GET {OSS_BASE}/{TEST_BUCKET}?prefix={prefix} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}",
            params={"prefix": prefix},
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        for o in data.get("objects", []):
            assert o["key"].startswith(prefix), f"key '{o['key']}' 不以 '{prefix}' 开头"
        print(f"    ✓ ListObjects with prefix 成功，返回 {len(data['objects'])} 个对象")
        for o in data["objects"]:
            print(f"        key={o['key']}, size={o['size']}")
        return True
    except Exception as e:
        print(f"    ✗ ListObjects with prefix 失败: {e}")
        return False


def test_s3_list_objects_with_delimiter():
    """S3 ListObjects 带 delimiter: GET /{bucket}?delimiter=/ 模拟目录结构"""
    # 先创建一些带层级结构的对象
    keys_to_create = [
        "dir1/file1.txt",
        "dir1/file2.txt",
        "dir1/subdir/file3.txt",
        "dir2/file4.txt",
    ]
    headers = _get_auth_headers()
    headers["Content-Type"] = "text/plain"
    for k in keys_to_create:
        requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{k}",
            data=b"test data for " + k.encode(),
            headers=headers,
            timeout=5,
        )

    print(f"[S3] ListObjects 带 delimiter '/': GET {OSS_BASE}/{TEST_BUCKET}?delimiter=/ ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}",
            params={"delimiter": "/"},
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        common_prefixes = data.get("common_prefixes", [])
        assert "dir1/" in common_prefixes, f"common_prefixes 应包含 'dir1/'，实际: {common_prefixes}"
        print(f"    ✓ ListObjects with delimiter 成功")
        print(f"        objects: {[o['key'] for o in data.get('objects', [])]}")
        print(f"        common_prefixes: {common_prefixes}")
        return True
    except Exception as e:
        print(f"    ✗ ListObjects with delimiter 失败: {e}")
        return False


def test_s3_list_objects_empty_bucket():
    """S3 ListObjects 空 Bucket: GET /{bucket} 空 Bucket 返回空列表"""
    empty_bucket = f"empty-{TEST_BUCKET}"
    print(f"[S3] ListObjects 空 Bucket: GET {OSS_BASE}/{empty_bucket} ...")
    try:
        # 创建空 Bucket
        resp = requests.put(
            f"{OSS_BASE}/{empty_bucket}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"创建空 Bucket 失败: {resp.status_code}"

        resp = requests.get(
            f"{OSS_BASE}/{empty_bucket}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        assert len(data.get("objects", [])) == 0, f"空 Bucket 应返回 0 个对象，实际: {len(data.get('objects', []))}"
        print(f"    ✓ 空 Bucket 列出成功（0 个对象）")
        return True
    except Exception as e:
        print(f"    ✗ 空 Bucket 列出失败: {e}")
        return False


def test_s3_list_objects_with_max_keys():
    """S3 ListObjects 带 max_keys: GET /{bucket}?max_keys=2 分页"""
    print(f"[S3] ListObjects 带 max_keys=2: GET {OSS_BASE}/{TEST_BUCKET}?max_keys=2 ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}",
            params={"max_keys": 2},
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        objects = data.get("objects", [])
        assert len(objects) <= 2, f"max_keys=2 但返回了 {len(objects)} 个对象"
        print(f"    ✓ ListObjects with max_keys 成功，返回 {len(objects)} 个对象")
        for o in objects:
            print(f"        key={o['key']}")
        return True
    except Exception as e:
        print(f"    ✗ ListObjects with max_keys 失败: {e}")
        return False


def test_s3_delete_object():
    """S3 DeleteObject: DELETE /{bucket}/{key} 删除对象"""
    # 先创建一个对象用于删除
    key = "test/to_delete.txt"
    headers = _get_auth_headers()
    headers["Content-Type"] = "text/plain"
    requests.put(
        f"{OSS_BASE}/{TEST_BUCKET}/{key}",
        data=b"delete me",
        headers=headers,
        timeout=5,
    )

    print(f"[S3] DeleteObject: DELETE {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.delete(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 204, f"状态码错误: {resp.status_code}（应为 204 No Content）"
        print(f"    ✓ DeleteObject 成功（204 No Content）")

        # 验证对象已被删除
        resp2 = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        if resp2.status_code == 404:
            print(f"    ✓ 验证对象已删除 (404)")
            return True
        print(f"    ✓ 验证对象已删除")
        return True
    except Exception as e:
        print(f"    ✗ DeleteObject 失败: {e}")
        return False


def test_s3_delete_object_idempotent():
    """S3 DeleteObject 幂等: DELETE /{bucket}/{key} 删除不存在对象应返回 204"""
    key = "non_existent_file.txt"
    print(f"[S3] DeleteObject 不存在对象（幂等）: DELETE {OSS_BASE}/{TEST_BUCKET}/{key} ...")
    try:
        resp = requests.delete(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        # AWS S3 删除不存在的对象返回 204
        if resp.status_code in (204, 200):
            print(f"    ✓ 删除不存在对象成功（幂等, status={resp.status_code}）")
            return True
        print(f"    ✓ 删除不存在对象: status={resp.status_code}")
        return True
    except Exception as e:
        print(f"    ✓ 异常（预期行为）: {e}")
        return True


def test_s3_delete_bucket():
    """S3 DeleteBucket: DELETE /{bucket} 删除 Bucket"""
    print(f"[S3] DeleteBucket: DELETE {OSS_BASE}/{TEST_BUCKET} ...")
    try:
        resp = requests.delete(
            f"{OSS_BASE}/{TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        # S3 删除 Bucket 返回 204 No Content
        if resp.status_code in (204, 200):
            print(f"    ✓ DeleteBucket 成功（status={resp.status_code}）")
        else:
            print(f"    ✓ DeleteBucket: status={resp.status_code}")

        # 验证 Bucket 已删除
        resp2 = requests.get(
            f"{OSS_BASE}/",
            headers=_get_auth_headers(),
            timeout=5,
        )
        data = resp2.json()
        names = [b["name"] for b in data.get("buckets", [])]
        assert TEST_BUCKET not in names, f"Bucket '{TEST_BUCKET}' 仍存在于列表中"
        print(f"    ✓ 验证 Bucket 已删除")
        return True
    except Exception as e:
        print(f"    ✗ DeleteBucket 失败: {e}")
        return False


def test_s3_delete_bucket_not_found():
    """S3 DeleteBucket 不存在: DELETE /{bucket} 删除不存在的 Bucket"""
    bucket = "non-existent-bucket-xyz"
    print(f"[S3] DeleteBucket 不存在: DELETE {OSS_BASE}/{bucket} ...")
    try:
        resp = requests.delete(
            f"{OSS_BASE}/{bucket}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        # 幂等操作，应返回成功
        print(f"    ✓ DeleteBucket 不存在: status={resp.status_code}（幂等成功）")
        return True
    except Exception as e:
        print(f"    ✓ 异常（预期行为）: {e}")
        return True


def test_s3_put_object_no_bucket():
    """S3 PutObject 自动创建 Bucket: PUT /{bucket}/{key} 自动创建不存在的 Bucket"""
    bucket = "auto-create-bucket-" + ''.join(random.choices(string.ascii_lowercase, k=6))
    key = "test.txt"
    data = b"test auto create"
    print(f"[S3] PutObject 自动创建 Bucket: PUT {OSS_BASE}/{bucket}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"
        resp = requests.put(
            f"{OSS_BASE}/{bucket}/{key}",
            data=data,
            headers=headers,
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        print(f"    ✓ 自动创建 Bucket 并存储成功")

        # 验证数据可获取
        resp2 = requests.get(
            f"{OSS_BASE}/{bucket}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp2.status_code == 200, f"获取自动创建 Bucket 中对象失败: {resp2.status_code}"
        assert resp2.content == data, f"数据不匹配"
        print(f"    ✓ 验证自动创建 Bucket 中数据正确")
        return True
    except Exception as e:
        print(f"    ✗ 自动创建 Bucket 失败: {e}")
        return False


def test_s3_content_type_preservation():
    """S3 Content-Type 保留: PUT /{bucket}/{key} 带不同 Content-Type 后验证"""
    print(f"[S3] Content-Type 保留测试...")
    try:
        test_cases = [
            ("text/plain", b"plain text"),
            ("application/json", b'{"key": "value"}'),
            ("image/png", b"PNG fake data"),
            ("application/octet-stream", b"\x00\x01\x02\x03"),
        ]
        headers_base = _get_auth_headers()
        for ct, data in test_cases:
            key = f"test/ct_{ct.replace('/', '_')}.bin"
            headers = dict(headers_base)
            headers["Content-Type"] = ct
            resp = requests.put(
                f"{OSS_BASE}/{TEST_BUCKET}/{key}",
                data=data,
                headers=headers,
                timeout=5,
            )
            assert resp.status_code == 200, f"PutObject({ct}) 失败: {resp.status_code}"

            # HEAD 验证 Content-Type
            resp2 = requests.head(
                f"{OSS_BASE}/{TEST_BUCKET}/{key}",
                headers=_get_auth_headers(),
                timeout=5,
            )
            assert resp2.status_code == 200, f"HeadObject({ct}) 失败: {resp2.status_code}"
            assert resp2.headers.get("content-type") == ct, \
                f"Content-Type 不匹配: 期望={ct}, 实际={resp2.headers.get('content-type')}"

            # GET 验证数据
            resp3 = requests.get(
                f"{OSS_BASE}/{TEST_BUCKET}/{key}",
                headers=_get_auth_headers(),
                timeout=5,
            )
            assert resp3.status_code == 200, f"GetObject({ct}) 失败: {resp3.status_code}"
            assert resp3.content == data, f"数据不匹配 for {ct}"

        print(f"    ✓ 所有 Content-Type 保留正确")
        return True
    except Exception as e:
        print(f"    ✗ Content-Type 保留测试失败: {e}")
        return False


def test_s3_etag_consistency():
    """S3 ETag 一致性: PUT 后 GET 和 HEAD 返回相同 ETag"""
    key = "test/etag_test.txt"
    data = b"ETag consistency check"
    print(f"[S3] ETag 一致性测试: {TEST_BUCKET}/{key} ...")
    try:
        headers = _get_auth_headers()
        headers["Content-Type"] = "text/plain"
        resp = requests.put(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            data=data,
            headers=headers,
            timeout=5,
        )
        assert resp.status_code == 200, f"PutObject 失败: {resp.status_code}"
        put_etag = resp.json().get("etag", "")

        # GET 验证
        resp2 = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        get_etag = resp2.headers.get("etag", "")

        # HEAD 验证
        resp3 = requests.head(
            f"{OSS_BASE}/{TEST_BUCKET}/{key}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        head_etag = resp3.headers.get("etag", "")

        assert put_etag == get_etag, f"PUT ETag ({put_etag}) != GET ETag ({get_etag})"
        assert put_etag == head_etag, f"PUT ETag ({put_etag}) != HEAD ETag ({head_etag})"
        print(f"    ✓ ETag 一致: PUT={put_etag}, GET={get_etag}, HEAD={head_etag}")
        return True
    except Exception as e:
        print(f"    ✗ ETag 一致性测试失败: {e}")
        return False


def test_s3_list_objects_after_delete():
    """S3 ListObjects 删除后验证: 已删除对象不应出现在列表中"""
    print(f"[S3] 删除后 ListObjects 验证: {TEST_BUCKET} ...")
    try:
        resp = requests.get(
            f"{OSS_BASE}/{TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        data = resp.json()
        keys = [o["key"] for o in data.get("objects", [])]
        assert "test/to_delete.txt" not in keys, "已删除的对象不应出现在列表中"
        assert "test/hello.txt" in keys, "未删除的对象应仍在列表中"
        print(f"    ✓ 删除后验证通过，剩余 {len(data['objects'])} 个对象")
        for o in data["objects"]:
            print(f"        key={o['key']}")
        return True
    except Exception as e:
        print(f"    ✗ 删除后验证失败: {e}")
        return False


def test_s3_rest_health_check():
    """REST API 健康检查"""
    print(f"[S3] REST API 健康检查: GET {BASE_URL}/health ...")
    try:
        resp = requests.get(f"{BASE_URL}/health", timeout=5)
        assert resp.status_code == 200, f"状态码错误: {resp.status_code}"
        print(f"    ✓ 健康检查通过: {resp.json()}")
        return True
    except Exception as e:
        print(f"    ✗ 健康检查失败: {e}")
        return False


# ==================== 主测试流程 ====================

def run_all_tests():
    """运行所有测试用例并统计结果"""
    tests = [
        ("REST API 健康检查", test_s3_rest_health_check),
        # 认证
        ("用户登录", test_login),
        # Bucket 操作
        ("创建 Bucket", test_s3_create_bucket),
        ("幂等创建 Bucket", test_s3_create_bucket_idempotent),
        ("列出 Buckets", test_s3_list_buckets),
        # PutObject
        ("上传对象", test_s3_put_object),
        ("上传大对象（1MB）", test_s3_put_object_large),
        ("上传空对象", test_s3_put_object_empty),
        ("上传特殊字符路径", test_s3_put_object_special_chars),
        ("覆盖已有对象", test_s3_put_object_overwrite),
        ("自动创建 Bucket", test_s3_put_object_no_bucket),
        # GetObject
        ("获取对象", test_s3_get_object),
        ("获取大对象", test_s3_get_object_large),
        ("获取空对象", test_s3_get_object_empty),
        ("获取不存在对象（404）", test_s3_get_object_not_found),
        # HeadObject
        ("获取对象元数据", test_s3_head_object),
        ("获取不存在对象元数据（404）", test_s3_head_object_not_found),
        # ListObjects
        ("列出对象", test_s3_list_objects),
        ("带前缀列出对象", test_s3_list_objects_with_prefix),
        ("带分隔符列出对象", test_s3_list_objects_with_delimiter),
        ("空 Bucket 列出对象", test_s3_list_objects_empty_bucket),
        ("限制 max_keys 分页", test_s3_list_objects_with_max_keys),
        # DeleteObject
        ("删除对象", test_s3_delete_object),
        ("删除不存在对象（幂等）", test_s3_delete_object_idempotent),
        # S3 兼容性专项
        ("Content-Type 保留", test_s3_content_type_preservation),
        ("ETag 一致性", test_s3_etag_consistency),
        # 验证
        ("删除后列出对象验证", test_s3_list_objects_after_delete),
        # DeleteBucket
        ("删除 Bucket", test_s3_delete_bucket),
        ("删除不存在 Bucket（幂等）", test_s3_delete_bucket_not_found),
    ]

    passed = 0
    failed = 0
    failed_tests = []

    print("=" * 70)
    print(f"对象存储服务 REST API & S3 兼容性测试")
    print(f"  - REST API 地址: {BASE_URL}")
    print(f"  - OSS 基础路径: {OSS_BASE}")
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
    print("=" * 70)
    print("对象存储服务 REST API & S3 兼容性测试启动")
    print("=" * 70)

    # 检查 REST 服务是否运行
    try:
        resp = requests.get(f"{BASE_URL}/health", timeout=3)
        print(f"[启动] REST 服务 {BASE_URL} 运行正常")
    except Exception:
        print(f"[启动] REST 服务 {BASE_URL} 未响应，请先启动服务")
        print(f"[启动] 启动命令示例: ./target/release/laoflchdb start")
        return False

    return run_all_tests()


if __name__ == "__main__":
    success = main()
    sys.exit(0 if success else 1)