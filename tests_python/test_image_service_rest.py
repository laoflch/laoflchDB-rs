#!/usr/bin/env python3
"""
Python 自动回归测试: ImageService 图片服务 REST 接口测试
基于对象存储服务实现图片上传（自动生成三种缩略图）和浏览
REST 端点挂载在 /api/v1/images 前缀下
"""
import io
import os
import sys
import json
import random
import string
import struct

import requests

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# 尝试导入 PIL 用于生成测试图片
try:
    from PIL import Image
    HAS_PIL = True
except ImportError:
    HAS_PIL = False

REST_HOST = os.environ.get("LAOFLCHDB_REST_HOST", "127.0.0.1")
REST_PORT = os.environ.get("LAOFLCHDB_REST_PORT", "8080")
REST_BASE = f"http://{REST_HOST}:{REST_PORT}"
IMG_BASE = f"{REST_BASE}/api/v1/images"
LOGIN_BASE = f"{REST_BASE}/api/v1/login"

# 测试 Bucket 名称
TEST_BUCKET = "test-images-rest-" + ''.join(random.choices(string.ascii_lowercase, k=6))

TOKEN = None


def _make_test_png(width=100, height=100, color=(255, 0, 0)):
    """生成测试 PNG 图片字节流"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), color)
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        return buf.getvalue()
    else:
        return _minimal_png(width, height, color)


def _minimal_png(width, height, color):
    """生成最小化 PNG（无 PIL 时的后备方案）"""
    import zlib

    def _png_chunk(chunk_type, data):
        chunk = chunk_type + data
        crc = struct.pack(">I", zlib.crc32(chunk) & 0xFFFFFFFF)
        return struct.pack(">I", len(data)) + chunk + crc

    sig = b'\x89PNG\r\n\x1a\n'
    ihdr_data = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    ihdr = _png_chunk(b'IHDR', ihdr_data)
    raw = b''
    for _ in range(height):
        raw += b'\x00' + bytes(color) * width
    compressed = zlib.compress(raw)
    idat = _png_chunk(b'IDAT', compressed)
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
        return _make_test_png(width, height, color)


def _get_auth_headers():
    """获取认证头"""
    if TOKEN:
        return {"Authorization": f"Bearer {TOKEN}"}
    return {}


def test_login():
    global TOKEN
    print("[测试] 用户登录...")
    try:
        resp = requests.post(
            LOGIN_BASE,
            json={"username": "admin", "password": "laoflchdb"},
            timeout=5,
        )
        assert resp.status_code == 200, f"登录失败: HTTP {resp.status_code}"
        data = resp.json()
        assert data.get("success"), f"登录失败: {data}"
        TOKEN = data["token"]
        print(f"    ✓ 登录成功")
        return True
    except Exception as e:
        print(f"    ✗ 登录失败: {e}")
        return False


def test_health():
    """测试健康检查"""
    print("[测试] 健康检查 /health...")
    try:
        resp = requests.get(f"{REST_BASE}/health", timeout=5)
        assert resp.status_code == 200, f"健康检查失败: HTTP {resp.status_code}"
        print(f"    ✓ 健康检查通过")
        return True
    except Exception as e:
        print(f"    ✗ 健康检查失败: {e}")
        return False


# ==================== 测试用例 ====================

def test_upload_image_png():
    """测试上传 PNG 图片"""
    key = "test_upload.png"
    data = _make_test_png(100, 100, (255, 0, 0))
    print(f"[测试] 上传 PNG 图片: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}, body={resp.text}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        assert result["key"] == key, f"key 不匹配: {result['key']}"
        assert result["etag"], "etag 不应为空"
        assert result["metadata"]["width"] == 100, f"宽度应为 100: {result['metadata']['width']}"
        assert result["metadata"]["height"] == 100, f"高度应为 100: {result['metadata']['height']}"
        assert "thumbnail" in result["metadata"]["thumbnails"], "缺少 thumbnail"
        assert "small" in result["metadata"]["thumbnails"], "缺少 small"
        assert "medium" in result["metadata"]["thumbnails"], "缺少 medium"
        print(f"    ✓ 上传 PNG 成功: {result['metadata']['width']}x{result['metadata']['height']}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_jpeg():
    """测试上传 JPEG 图片"""
    key = "test_upload.jpg"
    data = _make_test_jpeg(200, 150, (0, 255, 0))
    print(f"[测试] 上传 JPEG 图片: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/jpeg"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        assert result["metadata"]["width"] == 200, f"宽度应为 200: {result['metadata']['width']}"
        assert result["metadata"]["height"] == 150, f"高度应为 150: {result['metadata']['height']}"
        print(f"    ✓ 上传 JPEG 成功: {result['metadata']['width']}x{result['metadata']['height']}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_auto_key():
    """测试上传图片时自动生成 key"""
    print(f"[测试] 上传图片自动生成 key...")
    try:
        data = _make_test_png(50, 50, (0, 0, 255))
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        assert result["key"], "自动生成的 key 不应为空"
        print(f"    ✓ 自动生成 key 成功: {result['key']}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_image_with_metadata():
    """测试上传带自定义元数据的图片（通过 query 参数）"""
    key = "test_metadata.png"
    data = _make_test_png(80, 60, (128, 128, 128))
    print(f"[测试] 上传带元数据图片: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        print(f"    ✓ 上传成功: {result['key']}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_get_image():
    """测试获取原图"""
    key = "test_upload.png"
    print(f"[测试] 获取原图: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert resp.status_code == 200, f"获取失败: HTTP {resp.status_code}"
        assert resp.headers.get("content-type") == "image/png", f"content-type 不匹配: {resp.headers.get('content-type')}"
        assert resp.headers.get("etag"), "ETag 不应为空"
        # 验证是有效的 PNG
        assert resp.content[:8] == b'\x89PNG\r\n\x1a\n', "返回数据不是有效的 PNG"
        print(f"    ✓ 获取原图成功: {len(resp.content)} bytes, etag={resp.headers.get('etag')}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_image_not_found():
    """测试获取不存在的图片"""
    key = "non_existent.png"
    print(f"[测试] 获取不存在图片: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 404, f"应返回 404，实际: HTTP {resp.status_code}"
        print(f"    ✓ 正确返回 404")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_get_thumbnail():
    """测试获取 thumbnail 缩略图"""
    key = "test_upload.png"
    print(f"[测试] 获取 thumbnail: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/thumbnail?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert resp.status_code == 200, f"获取失败: HTTP {resp.status_code}"
        assert resp.headers.get("content-type") == "image/jpeg", f"应为 JPEG: {resp.headers.get('content-type')}"
        assert resp.headers.get("x-thumbnail-width") == "128", f"宽度应为 128: {resp.headers.get('x-thumbnail-width')}"
        assert resp.headers.get("x-thumbnail-height") == "128", f"高度应为 128: {resp.headers.get('x-thumbnail-height')}"
        # 验证是有效的 JPEG
        assert resp.content[:2] == b'\xff\xd8', "返回数据不是有效的 JPEG"
        print(f"    ✓ 获取 thumbnail 成功: {resp.headers.get('x-thumbnail-width')}x{resp.headers.get('x-thumbnail-height')}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_small():
    """测试获取 small 缩略图"""
    key = "test_upload.png"
    print(f"[测试] 获取 small: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/small?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert resp.status_code == 200, f"获取失败: HTTP {resp.status_code}"
        assert resp.headers.get("content-type") == "image/jpeg", f"应为 JPEG"
        width = int(resp.headers.get("x-thumbnail-width", 0))
        height = int(resp.headers.get("x-thumbnail-height", 0))
        assert width <= 256 and height <= 256, f"small 应不超过 256x256: {width}x{height}"
        print(f"    ✓ 获取 small 成功: {width}x{height}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_medium():
    """测试获取 medium 缩略图"""
    key = "test_upload.png"
    print(f"[测试] 获取 medium: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/medium?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert resp.status_code == 200, f"获取失败: HTTP {resp.status_code}"
        assert resp.headers.get("content-type") == "image/jpeg", f"应为 JPEG"
        print(f"    ✓ 获取 medium 成功")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_thumbnail_invalid_size():
    """测试获取无效 size 的缩略图"""
    key = "test_upload.png"
    print(f"[测试] 获取无效 size='large'...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/large?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 400, f"应返回 400，实际: HTTP {resp.status_code}"
        print(f"    ✓ 正确返回 400 Bad Request")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_get_image_metadata():
    """测试获取图片元数据"""
    key = "test_upload.png"
    print(f"[测试] 获取图片元数据: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/meta?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"获取失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["key"] == key, f"key 不匹配: {result['key']}"
        assert result["width"] == 100, f"宽度应为 100: {result['width']}"
        assert result["height"] == 100, f"高度应为 100: {result['height']}"
        assert "thumbnails" in result, "缺少 thumbnails"
        assert "thumbnail" in result["thumbnails"], "缺少 thumbnail"
        print(f"    ✓ 获取元数据成功: {result['width']}x{result['height']}")
        return True
    except Exception as e:
        print(f"    ✗ 获取失败: {e}")
        return False


def test_get_image_metadata_not_found():
    """测试获取不存在图片的元数据"""
    key = "non_existent_meta.png"
    print(f"[测试] 获取不存在图片元数据: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}/meta?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 404, f"应返回 404，实际: HTTP {resp.status_code}"
        print(f"    ✓ 正确返回 404")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_list_images():
    """测试列出图片"""
    print(f"[测试] 列出图片: {TEST_BUCKET}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&max_keys=100",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"列出失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["bucket"] == TEST_BUCKET, f"bucket 不匹配: {result['bucket']}"
        assert len(result["images"]) >= 3, f"应至少有 3 张图片: {len(result['images'])}"
        for img in result["images"]:
            assert img["key"], "图片 key 不应为空"
            assert img["width"] > 0, f"图片 {img['key']} 宽度应大于 0"
        print(f"    ✓ 列出 {len(result['images'])} 张图片")
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
        requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )

    print(f"[测试] 带前缀列出图片 '{prefix}'...")
    try:
        resp = requests.get(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&prefix={prefix}&max_keys=100",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"列出失败: HTTP {resp.status_code}"
        result = resp.json()
        assert len(result["images"]) >= 3, f"应至少有 3 张图片: {len(result['images'])}"
        for img in result["images"]:
            assert img["key"].startswith(prefix), f"key 应以 '{prefix}' 开头: {img['key']}"
        print(f"    ✓ 带前缀列出 {len(result['images'])} 张图片")
        return True
    except Exception as e:
        print(f"    ✗ 列出失败: {e}")
        return False


def test_upload_large_image():
    """测试上传大图片"""
    key = "test_large.png"
    data = _make_test_png(1000, 1000, (255, 128, 0))
    print(f"[测试] 上传大图片 1000x1000: {TEST_BUCKET}/{key} ({len(data)} bytes)...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=30,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        assert result["metadata"]["width"] == 1000, f"宽度应为 1000: {result['metadata']['width']}"
        print(f"    ✓ 上传大图片成功: {result['metadata']['width']}x{result['metadata']['height']}")
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


def test_upload_rectangular_image():
    """测试上传长方形图片"""
    key = "test_rect.png"
    data = _make_test_png(400, 100, (200, 100, 50))
    print(f"[测试] 上传长方形图片 400x100: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["metadata"]["width"] == 400, f"宽度应为 400: {result['metadata']['width']}"
        assert result["metadata"]["height"] == 100, f"高度应为 100: {result['metadata']['height']}"

        # 验证 thumbnail 是 128x128（cover 模式）
        thumb_resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/thumbnail?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert thumb_resp.status_code == 200, f"获取缩略图失败: HTTP {thumb_resp.status_code}"
        assert thumb_resp.headers.get("x-thumbnail-width") == "128", f"thumbnail 宽度应为 128"
        assert thumb_resp.headers.get("x-thumbnail-height") == "128", f"thumbnail 高度应为 128"

        # 验证 small 是等比缩放（400x100 → 256x64）
        small_resp = requests.get(
            f"{IMG_BASE}/{key}/thumbnails/small?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=10,
        )
        assert small_resp.headers.get("x-thumbnail-width") == "256", f"small 宽度应为 256"
        assert small_resp.headers.get("x-thumbnail-height") == "64", f"small 高度应为 64"

        print(f"    ✓ 长方形图片处理正确: thumbnail=128x128, small=256x64")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_delete_image():
    """测试删除图片"""
    key = "test_delete.png"
    # 先上传
    data = _make_test_png(100, 100, (255, 0, 255))
    upload_resp = requests.post(
        f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
        headers={**_get_auth_headers(), "Content-Type": "image/png"},
        data=data,
        timeout=10,
    )
    assert upload_resp.status_code == 200, "上传失败"

    print(f"[测试] 删除图片: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.delete(
            f"{IMG_BASE}/{key}?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"删除失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"删除失败: {result}"
        # 应删除原图 + 3 个缩略图 + 1 个元数据 = 5 个对象
        assert len(result["deleted_keys"]) >= 5, f"应删除至少 5 个对象: {len(result['deleted_keys'])}"
        assert key in result["deleted_keys"], f"应包含原图 key"
        print(f"    ✓ 删除成功，共删除 {len(result['deleted_keys'])} 个对象")
        return True
    except Exception as e:
        print(f"    ✗ 删除失败: {e}")
        return False


def test_delete_image_verify():
    """验证删除后图片不可访问"""
    key = "test_delete.png"
    print(f"[测试] 验证删除后获取失败: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.get(
            f"{IMG_BASE}/{key}?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 404, f"删除后应返回 404，实际: HTTP {resp.status_code}"
        print(f"    ✓ 删除后正确返回 404")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_delete_image_not_found():
    """测试删除不存在的图片（幂等）"""
    key = "non_existent_delete.png"
    print(f"[测试] 删除不存在图片（幂等）: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.delete(
            f"{IMG_BASE}/{key}?bucket={TEST_BUCKET}",
            headers=_get_auth_headers(),
            timeout=5,
        )
        assert resp.status_code == 200, f"应返回 200（幂等），实际: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"应成功: {result}"
        print(f"    ✓ 删除不存在图片成功（幂等）")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_upload_invalid_image_data():
    """测试上传无效的图片数据"""
    key = "test_invalid.png"
    data = b"this is not a valid image data"
    print(f"[测试] 上传无效图片数据: {TEST_BUCKET}/{key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?bucket={TEST_BUCKET}&key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=5,
        )
        assert resp.status_code == 400, f"应返回 400，实际: HTTP {resp.status_code}"
        print(f"    ✓ 正确返回 400 Bad Request")
        return True
    except Exception as e:
        print(f"    ✗ 测试失败: {e}")
        return False


def test_upload_default_bucket():
    """测试使用默认 bucket 上传"""
    key = "test_default.png"
    data = _make_test_png(100, 100, (0, 128, 255))
    print(f"[测试] 使用默认 bucket 上传: key={key}...")
    try:
        resp = requests.post(
            f"{IMG_BASE}?key={key}",
            headers={**_get_auth_headers(), "Content-Type": "image/png"},
            data=data,
            timeout=10,
        )
        assert resp.status_code == 200, f"上传失败: HTTP {resp.status_code}"
        result = resp.json()
        assert result["success"], f"上传失败: {result}"
        print(f"    ✓ 默认 bucket 上传成功: {result['key']}")

        # 清理
        requests.delete(
            f"{IMG_BASE}/{key}?bucket=images",
            headers=_get_auth_headers(),
            timeout=5,
        )
        return True
    except Exception as e:
        print(f"    ✗ 上传失败: {e}")
        return False


# ==================== 主测试流程 ====================

def run_all_tests():
    """运行所有测试用例并统计结果"""
    tests = [
        ("健康检查", test_health),
        ("用户登录", test_login),
        # 上传
        ("上传 PNG 图片", test_upload_image_png),
        ("上传 JPEG 图片", test_upload_image_jpeg),
        ("上传图片自动生成 key", test_upload_image_auto_key),
        ("上传带元数据图片", test_upload_image_with_metadata),
        ("上传大图片 1000x1000", test_upload_large_image),
        ("上传长方形图片", test_upload_rectangular_image),
        ("上传无效图片数据", test_upload_invalid_image_data),
        ("使用默认 bucket 上传", test_upload_default_bucket),
        # 获取原图
        ("获取原图", test_get_image),
        ("获取不存在图片", test_get_image_not_found),
        # 获取缩略图
        ("获取 thumbnail 缩略图", test_get_thumbnail),
        ("获取 small 缩略图", test_get_thumbnail_small),
        ("获取 medium 缩略图", test_get_thumbnail_medium),
        ("获取无效 size 缩略图", test_get_thumbnail_invalid_size),
        # 获取元数据
        ("获取图片元数据", test_get_image_metadata),
        ("获取不存在图片元数据", test_get_image_metadata_not_found),
        # 列出图片
        ("列出图片", test_list_images),
        ("带前缀列出图片", test_list_images_with_prefix),
        # 删除
        ("删除图片", test_delete_image),
        ("验证删除后不可访问", test_delete_image_verify),
        ("删除不存在图片（幂等）", test_delete_image_not_found),
    ]

    passed = 0
    failed = 0
    failed_tests = []

    print("=" * 70)
    print(f"图片服务 (ImageService) REST 接口测试")
    print(f"  - REST 地址: {IMG_BASE}")
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
    print("=" * 70)
    print("图片服务 (ImageService) REST 测试启动")
    print("=" * 70)

    # 检查服务是否运行
    try:
        resp = requests.get(f"{REST_BASE}/health", timeout=3)
        if resp.status_code != 200:
            print(f"[启动] REST 服务未运行在 {REST_BASE}")
            return False
        print(f"[启动] REST 服务已在运行: {REST_BASE}")
    except Exception:
        print(f"[启动] REST 服务未运行在 {REST_BASE}，请先手动启动服务")
        return False

    return run_all_tests()


if __name__ == "__main__":
    success = main()
    sys.exit(0 if success else 1)
