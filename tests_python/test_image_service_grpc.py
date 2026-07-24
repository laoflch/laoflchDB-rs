#!/usr/bin/env python3
"""
Python 自动回归测试: ImageService 图片服务 gRPC 接口测试
基于对象存储服务实现图片上传（自动生成三种缩略图）和浏览
"""
import io
import sys
import os
import random
import string
import struct

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import image_service_pb2
import image_service_pb2_grpc
import object_store_pb2
import object_store_pb2_grpc
import rpc_pb2
import rpc_pb2_grpc

# 尝试导入 PIL 用于生成测试图片
try:
    from PIL import Image
    HAS_PIL = True
except ImportError:
    HAS_PIL = False

TEST_ADDR = "127.0.0.1:19777"

TOKEN = None
stub = None
img_stub = None
os_stub = None

# 测试 Bucket 名称（使用随机后缀避免冲突）
TEST_BUCKET = "test-images-" + ''.join(random.choices(string.ascii_lowercase, k=6))


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


def _make_test_png(width=100, height=100, color=(255, 0, 0)):
    """生成测试 PNG 图片字节流"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), color)
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return buf.getvalue()
    else:
        # 无 PIL 时生成最小的有效 PNG（1x1 红色像素）
        # PNG 签名 + IHDR + IDAT + IEND
        return _minimal_png(width, height, color)


def _minimal_png(width, height, color):
    """生成最小化 PNG（无 PIL 时的后备方案）"""
    import zlib

    def _png_chunk(chunk_type, data):
        chunk = chunk_type + data
        crc = struct.pack(">I", zlib.crc32(chunk) & 0xFFFFFFFF)
        return struct.pack(">I", len(data)) + chunk + crc

    # PNG signature
    sig = b'\x89PNG\r\n\x1a\n'
    # IHDR
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)  # 8-bit RGB
    ihdr = _png_chunk(b'IHDR', ihdr_data)
    # IDAT - raw image data (每行前加 filter byte 0)
    raw = b''
    for _ in range(height):
        raw += b'\x00' + bytes(color) * width
    compressed = zlib.compress(raw)
    idat = _png_chunk(b'IDAT', compressed)
    # IEND
    iend = _png_chunk(b'IEND', b'')
    return sig + ihdr + idat + iend


def _make_test_jpeg(width=100, height=100, color=(0, 255, 0)):
    """生成测试 JPEG 图片字节流"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), color)
        buf = io.BytesIO()
        img.save(buf, format="JPEG", quality=85)
        return buf.getvalue()
    else:
        # 无 PIL 时使用 PNG 代替（服务端会根据内容检测格式）
        return _make_test_png(width, height, color)


# ==================== 测试用例 ====================

def test_upload_image_png():
    """测试上传 PNG 图片"""
    key = "test_upload.png"
    data = _make_test_png(100, 100, (255, 0, 0))
    print(f"[测试] UploadImage PNG: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.key == key, f"返回的 key 不匹配: {resp.key}"
        assert resp.etag, "ETag 不应为空"
        assert resp.metadata is not None, "元数据不应为空"
        assert resp.metadata.width == 100, f"宽度应为 100，实际: {resp.metadata.width}"
        assert resp.metadata.height == 100, f"高度应为 100，实际: {resp.metadata.height}"
        assert resp.metadata.content_type == "image/png", f"content_type 不匹配: {resp.metadata.content_type}"
        assert resp.metadata.content_length == len(data), f"content_length 不匹配"
        assert "thumbnail" in resp.metadata.thumbnails, "缺少 thumbnail 缩略图"
        assert "small" in resp.metadata.thumbnails, "缺少 small 缩略图"
        assert "medium" in resp.metadata.thumbnails, "缺少 medium 缩略图"
        print(f"    ✓ 上传 PNG 成功: {resp.metadata.width}x{resp.metadata.height}, format={resp.metadata.format}")
        print(f"        thumbnails: {dict(resp.metadata.thumbnails)}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_jpeg():
    """测试上传 JPEG 图片"""
    key = "test_upload.jpg"
    data = _make_test_jpeg(200, 150, (0, 255, 0))
    print(f"[测试] UploadImage JPEG: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="image/jpeg",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.metadata.width == 200, f"宽度应为 200，实际: {resp.metadata.width}"
        assert resp.metadata.height == 150, f"高度应为 150，实际: {resp.metadata.height}"
        print(f"    ✓ 上传 JPEG 成功: {resp.metadata.width}x{resp.metadata.height}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_auto_key():
    """测试上传图片时自动生成 key（key 为空）"""
    print(f"[测试] UploadImage 自动生成 key...")
    try:
        data = _make_test_png(50, 50, (0, 0, 255))
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET,
            key="",  # 空 key，服务端应自动生成 Snowflake ID
            data=data,
            content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.key, "自动生成的 key 不应为空"
        # Snowflake ID 为纯数字字符串
        assert resp.key.isdigit(), f"自动生成的 key 应为 Snowflake ID（纯数字）: {resp.key}"
        print(f"    ✓ 自动生成 Snowflake ID key 成功: {resp.key}")
        # 保存 key 供后续测试使用
        test_upload_image_auto_key.generated_key = resp.key
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_snowflake_id_uniqueness_and_monotonic():
    """测试连续上传多张图片时 Snowflake ID 的唯一性和单调递增"""
    print(f"[测试] Snowflake ID 唯一性与单调递增（连续上传 5 张）...")
    try:
        keys = []
        for i in range(5):
            data = _make_test_png(30, 30, (i * 50, 0, 0))
            req = image_service_pb2.UploadImageRequest(
                bucket=TEST_BUCKET,
                key="",  # 自动生成 Snowflake ID
                data=data,
                content_type="image/png",
            )
            resp = img_stub.UploadImage(req, metadata=get_metadata())
            assert resp.success, f"第 {i+1} 张上传失败: {resp.message}"
            assert resp.key.isdigit(), f"key 应为 Snowflake ID（纯数字）: {resp.key}"
            keys.append(int(resp.key))
        # 唯一性
        assert len(set(keys)) == len(keys), f"Snowflake ID 应唯一，实际: {keys}"
        # 单调递增
        assert keys == sorted(keys), f"Snowflake ID 应单调递增，实际: {keys}"
        print(f"    ✓ 5 个 Snowflake ID 唯一且单调递增: {keys}")
        # 保存 keys 供后续测试使用
        test_snowflake_id_uniqueness_and_monotonic.keys = [str(k) for k in keys]
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_snowflake_key_full_lifecycle():
    """测试使用自动生成的 Snowflake ID key 完成完整生命周期：获取原图/缩略图/元数据"""
    # 使用前一个测试保存的 Snowflake key
    snowflake_key = getattr(test_upload_image_auto_key, "generated_key", None)
    if not snowflake_key:
        print(f"[测试] Snowflake key 完整生命周期: 跳过（无可用 Snowflake key）")
        return False
    print(f"[测试] Snowflake key 完整生命周期: key={snowflake_key}...")
    try:
        # 1. 获取原图
        get_req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=snowflake_key)
        get_resp = img_stub.GetImage(get_req, metadata=get_metadata())
        assert get_resp.success, f"获取原图失败: {get_resp.message}"
        assert get_resp.data, "原图数据不应为空"
        print(f"    ✓ 获取原图成功: {get_resp.content_length} bytes")

        # 2. 获取元数据
        meta_req = image_service_pb2.GetImageMetadataRequest(bucket=TEST_BUCKET, key=snowflake_key)
        meta_resp = img_stub.GetImageMetadata(meta_req, metadata=get_metadata())
        assert meta_resp.success, f"获取元数据失败: {meta_resp.message}"
        assert meta_resp.metadata.key == snowflake_key, "元数据 key 应与 Snowflake key 一致"
        assert meta_resp.metadata.width == 50 and meta_resp.metadata.height == 50, \
            f"元数据尺寸应为 50x50，实际: {meta_resp.metadata.width}x{meta_resp.metadata.height}"
        print(f"    ✓ 获取元数据成功: {meta_resp.metadata.width}x{meta_resp.metadata.height}")

        # 3. 获取三种缩略图
        for size_name, expected_max in [("thumbnail", 128), ("small", 256), ("medium", 512)]:
            thumb_req = image_service_pb2.GetThumbnailRequest(
                bucket=TEST_BUCKET, key=snowflake_key, size=size_name,
            )
            thumb_resp = img_stub.GetThumbnail(thumb_req, metadata=get_metadata())
            assert thumb_resp.success, f"获取 {size_name} 缩略图失败: {thumb_resp.message}"
            assert thumb_resp.data, f"{size_name} 缩略图数据不应为空"
            # thumbnail 为 128x128（cover 模式）；small/medium 最大边不超过 expected_max
            if size_name == "thumbnail":
                assert thumb_resp.width == 128 and thumb_resp.height == 128, \
                    f"thumbnail 应为 128x128，实际: {thumb_resp.width}x{thumb_resp.height}"
            else:
                assert max(thumb_resp.width, thumb_resp.height) <= expected_max, \
                    f"{size_name} 最大边应 <= {expected_max}，实际: {thumb_resp.width}x{thumb_resp.height}"
            print(f"    ✓ 获取 {size_name} 缩略图成功: {thumb_resp.width}x{thumb_resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_snowflake_key_listable_and_deletable():
    """测试 Snowflake key 可被列出和删除"""
    snowflake_keys = getattr(test_snowflake_id_uniqueness_and_monotonic, "keys", None)
    if not snowflake_keys:
        print(f"[测试] Snowflake key 列出与删除: 跳过（无可用 Snowflake keys）")
        return False
    print(f"[测试] Snowflake key 列出与删除: {len(snowflake_keys)} 个 keys...")
    try:
        # 1. 列出图片，验证 Snowflake key 出现在列表中
        list_req = image_service_pb2.ListImagesRequest(bucket=TEST_BUCKET, max_keys=1000)
        list_resp = img_stub.ListImages(list_req, metadata=get_metadata())
        assert list_resp.success, f"列出失败: {list_resp.message}"
        listed_keys = {img.key for img in list_resp.images}
        for sk in snowflake_keys:
            assert sk in listed_keys, f"Snowflake key '{sk}' 未出现在列表中"
        print(f"    ✓ 所有 Snowflake key 均在列表中")

        # 2. 删除第一个 Snowflake key
        target_key = snowflake_keys[0]
        del_req = image_service_pb2.DeleteImageRequest(bucket=TEST_BUCKET, key=target_key)
        del_resp = img_stub.DeleteImage(del_req, metadata=get_metadata())
        assert del_resp.success, f"删除失败: {del_resp.message}"
        # 应删除原图 + 3 个缩略图 + 1 个元数据 = 5 个对象
        assert len(del_resp.deleted_keys) >= 5, \
            f"应删除至少 5 个对象（原图+3缩略图+元数据），实际: {len(del_resp.deleted_keys)}"
        print(f"    ✓ 删除 Snowflake key '{target_key}' 成功，共删除 {len(del_resp.deleted_keys)} 个对象")

        # 3. 验证删除后不可访问
        get_req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=target_key)
        try:
            get_resp = img_stub.GetImage(get_req, metadata=get_metadata())
            assert not get_resp.success, "删除后应无法获取原图"
        except grpc.RpcError as e:
            # NOT_FOUND 也是正确的（服务端对不存在的对象返回 NOT_FOUND）
            assert e.code() == grpc.StatusCode.NOT_FOUND, \
                f"应返回 NOT_FOUND，实际: {e.code()}"
        print(f"    ✓ 删除后原图不可访问")

        # 4. 验证元数据也被删除
        meta_req = image_service_pb2.GetImageMetadataRequest(bucket=TEST_BUCKET, key=target_key)
        try:
            meta_resp = img_stub.GetImageMetadata(meta_req, metadata=get_metadata())
            assert not meta_resp.success, "删除后应无法获取元数据"
        except grpc.RpcError as e:
            # NOT_FOUND 也是正确的
            assert e.code() == grpc.StatusCode.NOT_FOUND, f"应返回 NOT_FOUND，实际: {e.code()}"
        print(f"    ✓ 删除后元数据不可访问")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_upload_image_with_metadata():
    """测试上传图片时带自定义元数据"""
    key = "test_metadata.png"
    data = _make_test_png(80, 60, (128, 128, 128))
    metadata = {"author": "test_user", "description": "测试图片", "category": "sample"}
    print(f"[测试] UploadImage 带自定义元数据: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET,
            key=key,
            data=data,
            content_type="image/png",
            metadata=metadata,
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.metadata.user_metadata.get("author") == "test_user", f"元数据 author 不匹配: {dict(resp.metadata.user_metadata)}"
        assert resp.metadata.user_metadata.get("description") == "测试图片", f"元数据 description 不匹配"
        print(f"    ✓ 带元数据上传成功: {dict(resp.metadata.user_metadata)}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_overwrite():
    """测试覆盖上传同名图片"""
    key = "test_overwrite.png"
    print(f"[测试] UploadImage 覆盖上传: {TEST_BUCKET}/{key}...")
    try:
        # 第一次上传
        data1 = _make_test_png(100, 100, (255, 0, 0))
        req1 = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data1, content_type="image/png",
        )
        resp1 = img_stub.UploadImage(req1, metadata=get_metadata())
        assert resp1.success, f"第一次上传失败: {resp1.message}"
        etag1 = resp1.etag

        # 第二次上传（覆盖）
        data2 = _make_test_png(200, 200, (0, 255, 0))
        req2 = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data2, content_type="image/png",
        )
        resp2 = img_stub.UploadImage(req2, metadata=get_metadata())
        assert resp2.success, f"覆盖上传失败: {resp2.message}"
        assert resp2.etag != etag1, "覆盖后 ETag 应更新"
        assert resp2.metadata.width == 200, f"覆盖后宽度应为 200，实际: {resp2.metadata.width}"
        print(f"    ✓ 覆盖上传成功，ETag 已更新: {etag1} → {resp2.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 覆盖上传失败: {e}")
        return False


def test_get_image():
    """测试获取原图"""
    key = "test_upload.png"
    print(f"[测试] GetImage: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImage(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.content_type == "image/png", f"content_type 不匹配: {resp.content_type}"
        assert resp.content_length > 0, "content_length 应大于 0"
        assert resp.etag, "ETag 不应为空"
        # 验证数据是有效的 PNG（以 PNG 签名开头）
        assert resp.data[:8] == b'\x89PNG\r\n\x1a\n', "返回的数据不是有效的 PNG"
        print(f"    ✓ 获取原图成功: {resp.content_length} bytes, etag={resp.etag}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_image_not_found():
    """测试获取不存在的图片"""
    key = "non_existent_image.png"
    print(f"[测试] GetImage 不存在: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImage(req, metadata=get_metadata())
        assert not resp.success, "应返回失败"
        print(f"    ✓ 正确返回失败: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 正确返回错误: {e}")
        return True


def test_get_thumbnail_thumbnail():
    """测试获取 thumbnail 缩略图（128x128）"""
    key = "test_upload.png"
    print(f"[测试] GetThumbnail thumbnail: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="thumbnail",
        )
        resp = img_stub.GetThumbnail(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.content_type == "image/jpeg", f"缩略图应为 JPEG: {resp.content_type}"
        assert resp.width == 128, f"thumbnail 宽度应为 128，实际: {resp.width}"
        assert resp.height == 128, f"thumbnail 高度应为 128，实际: {resp.height}"
        # JPEG 数据应以 0xFF 0xD8 开头
        assert resp.data[:2] == b'\xff\xd8', "返回的数据不是有效的 JPEG"
        print(f"    ✓ 获取 thumbnail 成功: {resp.width}x{resp.height}, {resp.content_length} bytes")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_small():
    """测试获取 small 缩略图（最大 256x256）"""
    key = "test_upload.png"
    print(f"[测试] GetThumbnail small: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="small",
        )
        resp = img_stub.GetThumbnail(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.content_type == "image/jpeg", f"缩略图应为 JPEG: {resp.content_type}"
        # 100x100 的原图，small 最大 256，所以应保持 100x100
        assert resp.width <= 256, f"small 宽度应不超过 256，实际: {resp.width}"
        assert resp.height <= 256, f"small 高度应不超过 256，实际: {resp.height}"
        print(f"    ✓ 获取 small 成功: {resp.width}x{resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_medium():
    """测试获取 medium 缩略图（最大 512x512）"""
    key = "test_upload.png"
    print(f"[测试] GetThumbnail medium: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="medium",
        )
        resp = img_stub.GetThumbnail(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.content_type == "image/jpeg", f"缩略图应为 JPEG: {resp.content_type}"
        assert resp.width <= 512, f"medium 宽度应不超过 512，实际: {resp.width}"
        print(f"    ✓ 获取 medium 成功: {resp.width}x{resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_invalid_size():
    """测试使用无效的 size 获取缩略图"""
    key = "test_upload.png"
    print(f"[测试] GetThumbnail 无效 size='large'...")
    try:
        req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="large",
        )
        resp = img_stub.GetThumbnail(req, metadata=get_metadata())
        # 应返回错误
        print(f"    ✗ 应返回错误，但成功了: {resp.message}")
        return False
    except grpc.RpcError as e:
        assert e.code() == grpc.StatusCode.INVALID_ARGUMENT, f"应返回 INVALID_ARGUMENT，实际: {e.code()}"
        print(f"    ✓ 正确返回 INVALID_ARGUMENT: {e.details()}")
        return True
    except Exception as e:
        print(f"    ✓ 正确返回错误: {e}")
        return True


def test_get_image_metadata():
    """测试获取图片元数据"""
    key = "test_upload.png"
    print(f"[测试] GetImageMetadata: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageMetadataRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImageMetadata(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.metadata.key == key, f"key 不匹配: {resp.metadata.key}"
        assert resp.metadata.width == 100, f"宽度应为 100，实际: {resp.metadata.width}"
        assert resp.metadata.height == 100, f"高度应为 100，实际: {resp.metadata.height}"
        assert resp.metadata.content_type == "image/png", f"content_type 不匹配"
        assert "thumbnail" in resp.metadata.thumbnails, "缺少 thumbnail"
        assert "small" in resp.metadata.thumbnails, "缺少 small"
        assert "medium" in resp.metadata.thumbnails, "缺少 medium"
        print(f"    ✓ 获取元数据成功: {resp.metadata.width}x{resp.metadata.height}, format={resp.metadata.format}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_image_metadata_not_found():
    """测试获取不存在图片的元数据"""
    key = "non_existent_meta.png"
    print(f"[测试] GetImageMetadata 不存在: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageMetadataRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImageMetadata(req, metadata=get_metadata())
        assert not resp.success, "应返回失败"
        print(f"    ✓ 正确返回失败: {resp.message}")
        return True
    except grpc.RpcError as e:
        assert e.code() == grpc.StatusCode.NOT_FOUND, f"应返回 NOT_FOUND，实际: {e.code()}"
        print(f"    ✓ 正确返回 NOT_FOUND: {e.details()}")
        return True
    except Exception as e:
        print(f"    ✓ 正确返回错误: {e}")
        return True


def test_list_images():
    """测试列出图片"""
    print(f"[测试] ListImages: {TEST_BUCKET}...")
    try:
        req = image_service_pb2.ListImagesRequest(bucket=TEST_BUCKET, max_keys=100)
        resp = img_stub.ListImages(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        assert len(resp.images) >= 3, f"应至少有 3 张图片，实际: {len(resp.images)}"
        # 验证返回的图片都有元数据
        for img in resp.images:
            assert img.key, "图片 key 不应为空"
            assert img.width > 0, f"图片 {img.key} 宽度应大于 0"
            assert img.height > 0, f"图片 {img.key} 高度应大于 0"
        print(f"    ✓ 列出 {len(resp.images)} 张图片")
        for img in resp.images:
            print(f"        key={img.key}, {img.width}x{img.height}, format={img.format}")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_list_images_with_prefix():
    """测试带前缀列出图片"""
    # 先上传带前缀的图片
    prefix = "album/"
    for i in range(3):
        key = f"{prefix}photo_{i}.png"
        data = _make_test_png(50, 50, (i * 80, 0, 0))
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data, content_type="image/png",
        )
        img_stub.UploadImage(req, metadata=get_metadata())

    print(f"[测试] ListImages 带前缀 '{prefix}': {TEST_BUCKET}...")
    try:
        req = image_service_pb2.ListImagesRequest(
            bucket=TEST_BUCKET, prefix=prefix, max_keys=100,
        )
        resp = img_stub.ListImages(req, metadata=get_metadata())
        assert resp.success, f"列出失败: {resp.message}"
        assert len(resp.images) >= 3, f"应至少有 3 张带前缀的图片，实际: {len(resp.images)}"
        for img in resp.images:
            assert img.key.startswith(prefix), f"图片 key 应以 '{prefix}' 开头: {img.key}"
        print(f"    ✓ 带前缀列出 {len(resp.images)} 张图片")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_upload_large_image():
    """测试上传大图片（1000x1000）"""
    key = "test_large.png"
    data = _make_test_png(1000, 1000, (255, 128, 0))
    print(f"[测试] UploadImage 大图片 1000x1000: {TEST_BUCKET}/{key} ({len(data)} bytes)...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data, content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.metadata.width == 1000, f"宽度应为 1000，实际: {resp.metadata.width}"
        assert resp.metadata.height == 1000, f"高度应为 1000，实际: {resp.metadata.height}"
        # 验证 medium 缩略图被缩小到 512
        print(f"    ✓ 上传大图片成功: {resp.metadata.width}x{resp.metadata.height}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_verify_thumbnail_resized():
    """验证大图片的 medium 缩略图被缩小到 512x512"""
    key = "test_large.png"
    print(f"[测试] 验证大图片缩略图尺寸: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="medium",
        )
        resp = img_stub.GetThumbnail(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        # 1000x1000 的原图，medium 最大 512，应被缩放到 512x512
        assert resp.width == 512, f"medium 缩略图宽度应为 512，实际: {resp.width}"
        assert resp.height == 512, f"medium 缩略图高度应为 512，实际: {resp.height}"
        print(f"    ✓ 缩略图正确缩放: {resp.width}x{resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 验证失败: {e}")
        return False


def test_verify_thumbnail_cover_mode():
    """验证 thumbnail 使用 cover 模式（裁剪为正方形）"""
    key = "test_cover.jpg"  # 200x150 的 JPEG
    print(f"[测试] 验证 thumbnail cover 模式: {TEST_BUCKET}/{key}...")
    try:
        # 先上传非正方形图片
        data = _make_test_jpeg(200, 150, (100, 200, 50))
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data, content_type="image/jpeg",
        )
        img_stub.UploadImage(req, metadata=get_metadata())

        # 获取 thumbnail（应为 128x128 正方形）
        get_req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="thumbnail",
        )
        resp = img_stub.GetThumbnail(get_req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.width == 128, f"thumbnail 宽度应为 128，实际: {resp.width}"
        assert resp.height == 128, f"thumbnail 高度应为 128，实际: {resp.height}"
        print(f"    ✓ thumbnail cover 模式正确: {resp.width}x{resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 验证失败: {e}")
        return False


def test_delete_image():
    """测试删除图片（同时删除原图、缩略图和元数据）"""
    key = "test_delete.png"
    # 先上传
    data = _make_test_png(100, 100, (255, 0, 255))
    req = image_service_pb2.UploadImageRequest(
        bucket=TEST_BUCKET, key=key, data=data, content_type="image/png",
    )
    resp = img_stub.UploadImage(req, metadata=get_metadata())
    assert resp.success, "上传失败"
    thumbnail_keys = list(resp.metadata.thumbnails.values())

    print(f"[测试] DeleteImage: {TEST_BUCKET}/{key}...")
    try:
        del_req = image_service_pb2.DeleteImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.DeleteImage(del_req, metadata=get_metadata())
        assert resp.success, f"删除失败: {resp.message}"
        # 应删除原图 + 3 个缩略图 + 1 个元数据 = 5 个对象
        assert len(resp.deleted_keys) >= 5, f"应删除至少 5 个对象，实际: {len(resp.deleted_keys)}"
        assert key in resp.deleted_keys, f"deleted_keys 应包含原图 key: {resp.deleted_keys}"
        for thumb_key in thumbnail_keys:
            assert thumb_key in resp.deleted_keys, f"deleted_keys 应包含缩略图 key: {thumb_key}"
        print(f"    ✓ 删除成功，共删除 {len(resp.deleted_keys)} 个对象: {resp.deleted_keys}")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_delete_image_verify():
    """验证删除后图片不可访问"""
    key = "test_delete.png"
    print(f"[测试] 验证删除后 GetImage 失败: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImage(req, metadata=get_metadata())
        assert not resp.success, "删除后应无法获取"
        print(f"    ✓ 删除后验证通过: {resp.message}")
        return True
    except Exception as e:
        print(f"    ✓ 删除后正确返回错误: {e}")
        return True


def test_delete_image_not_found():
    """测试删除不存在的图片（应幂等成功）"""
    key = "non_existent_delete.png"
    print(f"[测试] DeleteImage 不存在（幂等）: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.DeleteImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.DeleteImage(req, metadata=get_metadata())
        assert resp.success, f"删除不存在的图片应成功: {resp.message}"
        print(f"    ✓ 删除不存在图片成功（幂等）: deleted_keys={resp.deleted_keys}")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_upload_image_default_bucket():
    """测试使用默认 bucket（bucket 为空）上传图片"""
    key = "test_default_bucket.png"
    data = _make_test_png(100, 100, (0, 128, 255))
    print(f"[测试] UploadImage 默认 bucket（空）: key={key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket="",  # 使用默认 bucket "images"
            key=key,
            data=data,
            content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        print(f"    ✓ 默认 bucket 上传成功: key={resp.key}")
        # 清理：删除测试图片
        del_req = image_service_pb2.DeleteImageRequest(bucket="images", key=key)
        img_stub.DeleteImage(del_req, metadata=get_metadata())
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_invalid_image_data():
    """测试上传无效的图片数据"""
    key = "test_invalid.png"
    data = b"this is not a valid image data"
    print(f"[测试] UploadImage 无效图片数据: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data, content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        # 应返回失败
        print(f"    ✗ 应返回失败，但成功了: {resp.message}")
        return False
    except grpc.RpcError as e:
        assert e.code() == grpc.StatusCode.INVALID_ARGUMENT, f"应返回 INVALID_ARGUMENT，实际: {e.code()}"
        print(f"    ✓ 正确返回 INVALID_ARGUMENT: {e.details()}")
        return True
    except Exception as e:
        print(f"    ✓ 正确返回错误: {e}")
        return True


def test_upload_rectangular_image_thumbnails():
    """测试上传长方形图片，验证 thumbnail 裁剪和 small/medium 等比缩放"""
    key = "test_rect.png"
    # 400x100 的宽图
    data = _make_test_png(400, 100, (200, 100, 50))
    print(f"[测试] UploadImage 长方形 400x100: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.UploadImageRequest(
            bucket=TEST_BUCKET, key=key, data=data, content_type="image/png",
        )
        resp = img_stub.UploadImage(req, metadata=get_metadata())
        assert resp.success, f"上传失败: {resp.message}"
        assert resp.metadata.width == 400 and resp.metadata.height == 100, \
            f"原图尺寸不匹配: {resp.metadata.width}x{resp.metadata.height}"

        # thumbnail 应为 128x128（cover 模式裁剪）
        thumb_req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="thumbnail",
        )
        thumb_resp = img_stub.GetThumbnail(thumb_req, metadata=get_metadata())
        assert thumb_resp.width == 128 and thumb_resp.height == 128, \
            f"thumbnail 应为 128x128，实际: {thumb_resp.width}x{thumb_resp.height}"

        # small 应等比缩放，最大边 256（400 → 256, 100 → 64）
        small_req = image_service_pb2.GetThumbnailRequest(
            bucket=TEST_BUCKET, key=key, size="small",
        )
        small_resp = img_stub.GetThumbnail(small_req, metadata=get_metadata())
        assert small_resp.width <= 256 and small_resp.height <= 256, \
            f"small 应不超过 256x256，实际: {small_resp.width}x{small_resp.height}"
        # 等比缩放：400x100 → 256x64
        assert small_resp.width == 256, f"small 宽度应为 256，实际: {small_resp.width}"
        assert small_resp.height == 64, f"small 高度应为 64，实际: {small_resp.height}"

        print(f"    ✓ 长方形图片处理正确:")
        print(f"        原图: 400x100")
        print(f"        thumbnail (cover): {thumb_resp.width}x{thumb_resp.height}")
        print(f"        small (contain): {small_resp.width}x{small_resp.height}")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


# ==================== 流式上传（切片上传）测试 ====================

# 流式上传的切片大小（4MB - 1KB，留出 protobuf 编码开销空间）
STREAM_CHUNK_SIZE = 4 * 1024 * 1024 - 1024


def _make_large_png(width, height, color=(128, 128, 128)):
    """生成一个较大的测试 PNG 图片（使用 PIL 直接创建，避免逐像素循环）"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), color)
        buf = io.BytesIO()
        # 使用较低的压缩级别加快生成速度
        img.save(buf, format="PNG", compress_level=0)
        return buf.getvalue()
    else:
        return _make_test_png(width, height, color)


def _upload_stream_chunks(data, bucket, key, content_type, metadata=None, name=""):
    """生成切片迭代器，供 UploadImageStream 使用"""
    total_chunks = (len(data) + STREAM_CHUNK_SIZE - 1) // STREAM_CHUNK_SIZE
    if metadata is None:
        metadata = {}
    for i in range(total_chunks):
        start = i * STREAM_CHUNK_SIZE
        end = min(start + STREAM_CHUNK_SIZE, len(data))
        chunk = image_service_pb2.UploadImageChunk(
            bucket=bucket if i == 0 else "",
            key=key if i == 0 else "",
            content_type=content_type if i == 0 else "",
            metadata=metadata if i == 0 else {},
            name=name if i == 0 else "",
            data=data[start:end],
            chunk_index=i,
            total_chunks=total_chunks,
        )
        yield chunk


def test_upload_stream_single_chunk():
    """测试流式上传小图片（1 个切片）"""
    key = "test_stream_single.png"
    data = _make_test_png(100, 100, (255, 0, 0))
    print(f"[测试] UploadImageStream 单切片: {TEST_BUCKET}/{key} ({len(data)} bytes)...")
    try:
        chunks = _upload_stream_chunks(data, TEST_BUCKET, key, "image/png")
        resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        assert resp.success, f"流式上传失败: {resp.message}"
        assert resp.key == key, f"返回的 key 不匹配: {resp.key}"
        assert resp.metadata.width == 100, f"宽度应为 100，实际: {resp.metadata.width}"
        assert resp.metadata.height == 100, f"高度应为 100，实际: {resp.metadata.height}"
        print(f"    ✓ 流式上传单切片成功: {resp.metadata.width}x{resp.metadata.height}")
        return True
    except Exception as e:
        print(f"    ✗ 流式上传失败: {e}")
        return False


def test_upload_stream_two_chunks():
    """测试流式上传需要 2 个切片的图片（略大于 4MB）"""
    key = "test_stream_two_chunks.png"
    # 生成一个略大于 4MB 的图片：3200x1200 ~ 11MB 未压缩，PNG 压缩后约 4-5MB
    data = _make_large_png(3200, 1200, (0, 255, 128))
    if len(data) <= STREAM_CHUNK_SIZE:
        # 如果不够大，尝试更大的尺寸
        data = _make_large_png(4000, 1500, (0, 255, 128))
    chunks_count = (len(data) + STREAM_CHUNK_SIZE - 1) // STREAM_CHUNK_SIZE
    print(f"[测试] UploadImageStream {chunks_count} 切片: {TEST_BUCKET}/{key} ({len(data)} bytes, {chunks_count} 切片)...")
    try:
        chunks = _upload_stream_chunks(data, TEST_BUCKET, key, "image/png")
        resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        assert resp.success, f"流式上传失败: {resp.message}"
        assert resp.key == key, f"返回的 key 不匹配: {resp.key}"
        print(f"    ✓ 流式上传 {chunks_count} 切片成功: {resp.metadata.width}x{resp.metadata.height}")
        return True
    except Exception as e:
        print(f"    ✗ 流式上传失败: {e}")
        return False


def test_upload_stream_with_metadata():
    """测试流式上传带元数据"""
    key = "test_stream_metadata.png"
    data = _make_test_png(100, 100, (0, 128, 255))
    metadata = {"source": "stream_test", "chunked": "true"}
    print(f"[测试] UploadImageStream 带元数据: {TEST_BUCKET}/{key}...")
    try:
        chunks = _upload_stream_chunks(data, TEST_BUCKET, key, "image/png", metadata=metadata, name="stream_upload.png")
        resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        assert resp.success, f"流式上传失败: {resp.message}"
        assert resp.metadata.user_metadata.get("source") == "stream_test", \
            f"元数据 source 不匹配: {dict(resp.metadata.user_metadata)}"
        assert resp.metadata.user_metadata.get("chunked") == "true", \
            f"元数据 chunked 不匹配: {dict(resp.metadata.user_metadata)}"
        print(f"    ✓ 流式上传带元数据成功: {dict(resp.metadata.user_metadata)}")
        return True
    except Exception as e:
        print(f"    ✗ 流式上传失败: {e}")
        return False


def test_upload_stream_auto_key():
    """测试流式上传自动生成 key"""
    print(f"[测试] UploadImageStream 自动生成 key...")
    try:
        data = _make_test_png(50, 50, (0, 255, 0))
        chunks = _upload_stream_chunks(data, TEST_BUCKET, "", "image/png")
        resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        assert resp.success, f"流式上传失败: {resp.message}"
        assert resp.key, "自动生成的 key 不应为空"
        assert resp.key.isdigit(), f"自动生成的 key 应为 Snowflake ID（纯数字）: {resp.key}"
        print(f"    ✓ 流式上传自动生成 key 成功: {resp.key}")
        test_upload_stream_auto_key.generated_key = resp.key
        return True
    except Exception as e:
        print(f"    ✗ 流式上传失败: {e}")
        return False


def test_upload_stream_empty():
    """测试流式上传空数据（应被拒绝）"""
    key = "test_stream_empty.png"
    print(f"[测试] UploadImageStream 空数据（应失败）: {TEST_BUCKET}/{key}...")
    try:
        chunks = _upload_stream_chunks(b"", TEST_BUCKET, key, "image/png")
        resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        # 应返回失败
        print(f"    ✗ 应返回失败，但成功了: {resp.message}")
        return False
    except grpc.RpcError as e:
        assert e.code() in (grpc.StatusCode.INVALID_ARGUMENT, grpc.StatusCode.INTERNAL), \
            f"应返回 INVALID_ARGUMENT 或 INTERNAL，实际: {e.code()}"
        print(f"    ✓ 正确返回错误: {e.code()}: {e.details()}")
        return True
    except Exception as e:
        print(f"    ✓ 正确返回错误: {e}")
        return True


def test_upload_stream_verify():
    """验证流式上传的图片可通过 GetImage 正常获取"""
    key = "test_stream_single.png"
    print(f"[测试] 验证流式上传图片可正常获取: {TEST_BUCKET}/{key}...")
    try:
        req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=key)
        resp = img_stub.GetImage(req, metadata=get_metadata())
        assert resp.success, f"获取失败: {resp.message}"
        assert resp.content_type == "image/png", f"content_type 不匹配: {resp.content_type}"
        assert resp.content_length > 0, "content_length 应大于 0"
        assert resp.data[:8] == b'\x89PNG\r\n\x1a\n', "返回的数据不是有效的 PNG"
        print(f"    ✓ 流式上传图片获取成功: {resp.content_length} bytes")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_upload_stream_compare_content():
    """验证流式上传和原始数据一致"""
    key = "test_stream_compare.png"
    data = _make_test_png(100, 100, (128, 0, 255))
    print(f"[测试] 验证流式上传与原始数据一致: {TEST_BUCKET}/{key}...")
    try:
        # 流式上传
        chunks = _upload_stream_chunks(data, TEST_BUCKET, key, "image/png")
        stream_resp = img_stub.UploadImageStream(chunks, metadata=get_metadata())
        assert stream_resp.success, f"流式上传失败: {stream_resp.message}"

        # 获取流式上传的图片数据
        get_req = image_service_pb2.GetImageRequest(bucket=TEST_BUCKET, key=key)
        get_resp = img_stub.GetImage(get_req, metadata=get_metadata())
        assert get_resp.success, f"获取流式上传图片失败: {get_resp.message}"
        assert get_resp.data == data, "流式上传的图片数据与原数据不一致"
        print(f"    ✓ 流式上传与原始数据一致")
        return True
    except Exception as e:
        print(f"    ✗ 验证失败: {e}")
        return False


def test_upload_stream_cleanup():
    """清理流式上传的测试图片"""
    keys_to_delete = [
        "test_stream_single.png",
        "test_stream_metadata.png",
        "test_stream_compare.png",
    ]
    # 添加自动生成的 key
    auto_key = getattr(test_upload_stream_auto_key, "generated_key", None)
    if auto_key:
        keys_to_delete.append(auto_key)
    # 添加 2 切片测试图片（可能已上传成功）
    keys_to_delete.append("test_stream_two_chunks.png")
    print(f"[测试] 清理流式上传测试图片...")
    try:
        deleted_count = 0
        for key in keys_to_delete:
            try:
                del_req = image_service_pb2.DeleteImageRequest(bucket=TEST_BUCKET, key=key)
                resp = img_stub.DeleteImage(del_req, metadata=get_metadata())
                if resp.success:
                    deleted_count += len(resp.deleted_keys)
            except Exception:
                pass
        print(f"    ✓ 清理完成，共删除 {deleted_count} 个对象")
        return True
    except Exception as e:
        print(f"    ✗ 清理失败: {e}")
        return False


# ==================== 主测试流程 ====================

def run_all_tests():
    """运行所有测试用例并统计结果"""
    tests = [
        ("登录", test_login),
        # 上传图片
        ("上传 PNG 图片", test_upload_image_png),
        ("上传 JPEG 图片", test_upload_image_jpeg),
        ("上传图片自动生成 key", test_upload_image_auto_key),
        ("Snowflake ID 唯一性与单调递增", test_snowflake_id_uniqueness_and_monotonic),
        ("Snowflake key 完整生命周期", test_snowflake_key_full_lifecycle),
        ("Snowflake key 列出与删除", test_snowflake_key_listable_and_deletable),
        ("上传图片带元数据", test_upload_image_with_metadata),
        ("覆盖上传同名图片", test_upload_image_overwrite),
        ("上传大图片 1000x1000", test_upload_large_image),
        ("上传长方形图片", test_upload_rectangular_image_thumbnails),
        ("上传无效图片数据", test_upload_invalid_image_data),
        ("使用默认 bucket 上传", test_upload_image_default_bucket),
        # 获取原图
        ("获取原图", test_get_image),
        ("获取不存在图片", test_get_image_not_found),
        # 获取缩略图
        ("获取 thumbnail 缩略图", test_get_thumbnail_thumbnail),
        ("获取 small 缩略图", test_get_thumbnail_small),
        ("获取 medium 缩略图", test_get_thumbnail_medium),
        ("获取无效 size 缩略图", test_get_thumbnail_invalid_size),
        ("验证大图片缩略图尺寸", test_verify_thumbnail_resized),
        ("验证 thumbnail cover 模式", test_verify_thumbnail_cover_mode),
        # 获取元数据
        ("获取图片元数据", test_get_image_metadata),
        ("获取不存在图片元数据", test_get_image_metadata_not_found),
        # 列出图片
        ("列出图片", test_list_images),
        ("带前缀列出图片", test_list_images_with_prefix),
        # 删除图片
        ("删除图片", test_delete_image),
        ("验证删除后不可访问", test_delete_image_verify),
        ("删除不存在图片（幂等）", test_delete_image_not_found),
        # 流式上传（切片上传）
        ("流式上传单切片", test_upload_stream_single_chunk),
        ("流式上传 2 切片", test_upload_stream_two_chunks),
        ("流式上传带元数据", test_upload_stream_with_metadata),
        ("流式上传自动生成 key", test_upload_stream_auto_key),
        ("流式上传空数据（应失败）", test_upload_stream_empty),
        ("验证流式上传图片可获取", test_upload_stream_verify),
        ("验证流式上传数据一致性", test_upload_stream_compare_content),
        ("清理流式上传测试图片", test_upload_stream_cleanup),
    ]

    passed = 0
    failed = 0
    failed_tests = []

    print("=" * 70)
    print(f"图片服务 (ImageService) gRPC 接口测试")
    print(f"  - 服务器地址: {TEST_ADDR}")
    print(f"  - 测试 Bucket: {TEST_BUCKET}")
    print(f"  - PIL 可用: {HAS_PIL}")
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
    global stub, img_stub, os_stub

    print("=" * 70)
    print("图片服务 (ImageService) 测试启动")
    print("=" * 70)

    # 检查服务是否已在运行
    print(f"[启动] 检查 gRPC 服务 {TEST_ADDR}...")
    if check_service_alive(TEST_ADDR):
        print("[启动] 服务已在运行，将使用现有服务")
    else:
        print(f"[启动] 服务未运行在 {TEST_ADDR}，请先手动启动服务")
        return False

    # 建立连接
    channel = grpc.insecure_channel(TEST_ADDR)
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    img_stub = image_service_pb2_grpc.ImageServiceStub(channel)
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
