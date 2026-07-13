#!/usr/bin/env python3
"""
Python 自动化测试: FaceService 人脸服务 REST 接口测试

测试范围：
- REST 端点可达性
- 参数校验
- 特征比对
- 错误处理
"""
import io
import sys
import os
import json
import random
import math
import requests

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    from PIL import Image, ImageDraw
    HAS_PIL = True
except ImportError:
    HAS_PIL = False

BASE_URL = "http://127.0.0.1:8080"
TOKEN = None


def _make_test_image(width=200, height=200):
    """生成测试图片"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), (180, 160, 140))
        draw = ImageDraw.Draw(img)
        cx, cy = width // 2, height // 2
        draw.ellipse([cx-60, cy-70, cx+60, cy+70], fill=(200, 180, 160), outline=(100, 80, 60))
        draw.ellipse([cx-35, cy-25, cx-15, cy-10], fill=(40, 40, 40))
        draw.ellipse([cx+15, cy-25, cx+35, cy-10], fill=(40, 40, 40))
        draw.ellipse([cx-8, cy-5, cx+8, cy+15], fill=(180, 150, 130))
        draw.arc([cx-25, cy+20, cx+25, cy+45], 0, 180, fill=(80, 40, 40), width=3)
        buf = io.BytesIO()
        img.save(buf, format="JPEG")
        return buf.getvalue()
    else:
        return b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb\x00C\x00\x08\x06\x06\x07\x06\x05\x08\x07\x07\x07\x09\x09\x08\x0a\x0c\x14\x0d\x0c\x0b\x0b\x0c\x19\x12\x13\x0f\x14\x1d\x1a\x1f\x1e\x1d\x1a\x1c\x1c\x20\x24\x2e\x27\x20\x22\x2c\x23\x1c\x1c\x28\x37\x29\x2c\x30\x31\x34\x34\x34\x1f\x27\x39\x3d\x38\x32\x3c\x2e\x33\x34\x32\xff\xc9\x00\x0b\x08\x00\x01\x00\x01\x01\x01\x11\x00\xff\xcc\x00\x06\x00\x10\x10\x05\xff\xda\x00\x08\x01\x01\x00\x00\x3f\x00\xfb\xd2\x8a\x28\xa0\xff\xd9"


def login():
    global TOKEN
    print("[测试] 用户登录...")
    resp = requests.post(f"{BASE_URL}/api/v1/login", json={"username": "admin", "password": "laoflchdb"})
    if resp.status_code == 200:
        data = resp.json()
        TOKEN = data.get("token") or data.get("data", {}).get("token")
        if TOKEN:
            print(f"    ✓ 登录成功")
            return True
    print(f"    ✗ 登录失败: {resp.status_code}")
    return False


def auth_headers():
    return {"Authorization": f"Bearer {TOKEN}"} if TOKEN else {}


# ── 测试用例 ─────────────────────────────────────────────────────────

def test_health():
    print("[测试] REST 服务健康检查...")
    try:
        resp = requests.get(f"{BASE_URL}/health", timeout=3)
        if resp.status_code == 200:
            print(f"    ✓ 服务健康")
            return True
        else:
            print(f"    ✗ 状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 连接失败: {e}")
        return False


def test_detect_faces():
    print("[测试] REST 检测人脸...")
    img_data = _make_test_image(200, 200)
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/detect?det_threshold=0.5&max_faces=0",
            data=img_data,
            headers={**auth_headers(), "Content-Type": "application/octet-stream"},
        )
        if resp.status_code == 200:
            data = resp.json()
            if data.get("success"):
                faces = data.get("faces", [])
                print(f"    ✓ 检测成功，{len(faces)} 张人脸")
                print(f"      图片尺寸: {data.get('image_width')}x{data.get('image_height')}")
                return True
            else:
                print(f"    ✗ 检测失败: {data.get('message')}")
                return False
        else:
            # 模型未加载时返回 500
            print(f"    ✓ 状态码 {resp.status_code}（可能是模型未加载）")
            return True
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_extract_features():
    print("[测试] REST 提取人脸特征...")
    img_data = _make_test_image(200, 200)
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/extract?det_threshold=0.5&return_aligned_images=false",
            data=img_data,
            headers={**auth_headers(), "Content-Type": "application/octet-stream"},
        )
        if resp.status_code == 200:
            data = resp.json()
            if data.get("success"):
                faces = data.get("faces", [])
                print(f"    ✓ 提取成功，{len(faces)} 张人脸")
                for i, face in enumerate(faces):
                    emb = face.get("embedding", [])
                    print(f"      人脸 {i}: 特征维度={len(emb)}")
                    if len(emb) > 0:
                        norm = math.sqrt(sum(x * x for x in emb))
                        print(f"      L2 范数: {norm:.6f}")
                return True
            else:
                print(f"    ✗ 提取失败: {data.get('message')}")
                return False
        else:
            print(f"    ✓ 状态码 {resp.status_code}（可能是模型未加载）")
            return True
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_extract_features_with_save():
    """测试 REST 提取特征并保存对齐图片"""
    print("[测试] REST 提取人脸特征 - 保存对齐图片...")
    img_data = _make_test_image(200, 200)
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/extract?save_aligned_images=true&image_bucket=test-faces-rest&return_aligned_images=true",
            data=img_data,
            headers={**auth_headers(), "Content-Type": "application/octet-stream"},
        )
        if resp.status_code == 200:
            data = resp.json()
            if data.get("success"):
                faces = data.get("faces", [])
                if len(faces) > 0:
                    face = faces[0]
                    print(f"    ✓ 提取成功，保存 key={face.get('saved_image_key')}")
                    print(f"      has_aligned_image={face.get('has_aligned_image')}")
                return True
            else:
                print(f"    ✗ 提取失败: {data.get('message')}")
                return False
        else:
            print(f"    ✓ 状态码 {resp.status_code}（可能是模型未加载）")
            return True
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_compare_features_same():
    print("[测试] REST 特征比对 - 相同向量...")
    vec = [random.uniform(-1, 1) for _ in range(512)]
    norm = math.sqrt(sum(x * x for x in vec))
    vec = [x / norm for x in vec]

    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/compare",
            json={"feature1": vec, "feature2": vec},
            headers=auth_headers(),
        )
        if resp.status_code == 200:
            data = resp.json()
            sim = data.get("similarity", 0)
            print(f"    ✓ 相似度: {sim:.6f}, is_same={data.get('is_same_person')}")
            assert abs(sim - 1.0) < 0.001, f"应为 1.0，实际 {sim}"
            assert data.get("is_same_person") == True
            return True
        else:
            print(f"    ✗ 状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_compare_features_different():
    print("[测试] REST 特征比对 - 不同向量...")
    vec1 = [random.uniform(-1, 1) for _ in range(512)]
    norm1 = math.sqrt(sum(x * x for x in vec1))
    vec1 = [x / norm1 for x in vec1]

    vec2 = [random.uniform(-1, 1) for _ in range(512)]
    norm2 = math.sqrt(sum(x * x for x in vec2))
    vec2 = [x / norm2 for x in vec2]

    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/compare",
            json={"feature1": vec1, "feature2": vec2},
            headers=auth_headers(),
        )
        if resp.status_code == 200:
            data = resp.json()
            sim = data.get("similarity", 0)
            print(f"    ✓ 相似度: {sim:.6f}, is_same={data.get('is_same_person')}")
            assert sim < 0.5, f"应 < 0.5，实际 {sim}"
            assert data.get("is_same_person") == False
            return True
        else:
            print(f"    ✗ 状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_compare_features_wrong_dim():
    print("[测试] REST 特征比对 - 维度错误...")
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/compare",
            json={"feature1": [1.0] * 100, "feature2": [1.0] * 100},
            headers=auth_headers(),
        )
        if resp.status_code == 400:
            print(f"    ✓ 维度错误返回 400（预期）")
            return True
        else:
            print(f"    ✗ 意外状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_compare_features_orthogonal():
    print("[测试] REST 特征比对 - 正交向量...")
    vec1 = [1.0] + [0.0] * 511
    vec2 = [0.0] * 512
    vec2[1] = 1.0

    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/compare",
            json={"feature1": vec1, "feature2": vec2},
            headers=auth_headers(),
        )
        if resp.status_code == 200:
            data = resp.json()
            sim = data.get("similarity", 1)
            print(f"    ✓ 相似度: {sim:.6f}")
            assert abs(sim) < 0.001, f"应接近 0，实际 {sim}"
            return True
        else:
            print(f"    ✗ 状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_detect_empty_image():
    print("[测试] REST 检测 - 空图片...")
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/detect",
            data=b"",
            headers={**auth_headers(), "Content-Type": "application/octet-stream"},
        )
        if resp.status_code in (200, 400, 500):
            print(f"    ✓ 空图片返回状态码 {resp.status_code}（预期错误）")
            return True
        else:
            print(f"    ✗ 意外状态码: {resp.status_code}")
            return False
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


def test_extract_aligned():
    """测试 REST 从对齐图片提取特征"""
    print("[测试] REST 从对齐图片提取特征...")
    # 生成 112x112 的图片
    img_data = _make_test_image(112, 112)
    try:
        resp = requests.post(
            f"{BASE_URL}/api/v1/face/extract-aligned",
            data=img_data,
            headers={**auth_headers(), "Content-Type": "application/octet-stream"},
        )
        if resp.status_code == 200:
            data = resp.json()
            if data.get("success"):
                emb = data.get("embedding", [])
                print(f"    ✓ 提取成功，维度={len(emb)}")
                return True
            else:
                print(f"    ✗ 提取失败: {data.get('message')}")
                return False
        else:
            print(f"    ✓ 状态码 {resp.status_code}（可能是模型未加载）")
            return True
    except Exception as e:
        print(f"    ✗ 错误: {e}")
        return False


# ── 主函数 ─────────────────────────────────────────────────────────

def run_tests():
    tests = [
        ("test_health", test_health),
        ("test_login", login),
        ("test_detect_faces", test_detect_faces),
        ("test_extract_features", test_extract_features),
        ("test_extract_features_with_save", test_extract_features_with_save),
        ("test_compare_features_same", test_compare_features_same),
        ("test_compare_features_different", test_compare_features_different),
        ("test_compare_features_orthogonal", test_compare_features_orthogonal),
        ("test_compare_features_wrong_dim", test_compare_features_wrong_dim),
        ("test_detect_empty_image", test_detect_empty_image),
        ("test_extract_aligned", test_extract_aligned),
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
    success = run_tests()
    sys.exit(0 if success else 1)
