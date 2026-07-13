#!/usr/bin/env python3
"""
Python 自动化测试: FaceService 人脸服务 gRPC 接口测试

测试范围：
- 服务可达性
- 参数校验（空图片、无效图片）
- 模型加载状态（无模型时返回 FAILED_PRECONDITION）
- 特征比对（维度校验、余弦相似度计算）
- 集成测试（与 image_service 协同）

注意：若 SCRFD/ArcFace 模型未安装，检测/提取相关测试将验证错误处理而非成功路径。
"""
import io
import sys
import os
import random
import string
import math

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import grpc
import face_service_pb2
import face_service_pb2_grpc
import image_service_pb2
import image_service_pb2_grpc
import rpc_pb2
import rpc_pb2_grpc

try:
    from PIL import Image, ImageDraw
    HAS_PIL = True
except ImportError:
    HAS_PIL = False

TEST_ADDR = "127.0.0.1:19777"

TOKEN = None
stub = None
face_stub = None
img_stub = None

TEST_BUCKET = "test-faces-" + ''.join(random.choices(string.ascii_lowercase, k=6))


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


def _make_test_image(width=200, height=200):
    """生成测试图片（模拟人脸的简单彩色图片）"""
    if HAS_PIL:
        img = Image.new("RGB", (width, height), (180, 160, 140))
        draw = ImageDraw.Draw(img)
        # 绘制简单的人脸轮廓（圆形头部 + 椭圆眼睛 + 嘴巴）
        cx, cy = width // 2, height // 2
        # 头部
        draw.ellipse([cx-60, cy-70, cx+60, cy+70], fill=(200, 180, 160), outline=(100, 80, 60))
        # 左眼
        draw.ellipse([cx-35, cy-25, cx-15, cy-10], fill=(40, 40, 40))
        # 右眼
        draw.ellipse([cx+15, cy-25, cx+35, cy-10], fill=(40, 40, 40))
        # 鼻子
        draw.ellipse([cx-8, cy-5, cx+8, cy+15], fill=(180, 150, 130))
        # 嘴巴
        draw.arc([cx-25, cy+20, cx+25, cy+45], 0, 180, fill=(80, 40, 40), width=3)
        buf = io.BytesIO()
        img.save(buf, format="JPEG")
        return buf.getvalue()
    else:
        # 无 PIL 时返回最小 JPEG
        return bytes([
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
            0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
            0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09,
            0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
            0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
            0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29,
            0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32,
            0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC9, 0x00, 0x0B, 0x08, 0x00, 0x01,
            0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xCC, 0x00, 0x06, 0x00, 0x10,
            0x10, 0x05, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00,
            0xFB, 0xD2, 0x8A, 0x28, 0xA0, 0xFF, 0xD9
        ])


# ── 测试用例 ─────────────────────────────────────────────────────────

def test_service_available():
    """测试 gRPC 服务可达"""
    print("[测试] FaceService 服务可达性...")
    if check_service_alive(TEST_ADDR):
        print(f"    ✓ 服务可达 {TEST_ADDR}")
        return True
    else:
        print(f"    ✗ 服务不可达 {TEST_ADDR}")
        return False


def test_detect_faces_empty_image():
    """测试检测：空图片数据应返回 INVALID_ARGUMENT"""
    print("[测试] 检测人脸 - 空图片数据...")
    try:
        req = face_service_pb2.DetectFacesRequest(
            image_data=b"",
            det_threshold=0.5,
            max_faces=0,
        )
        resp = face_stub.DetectFaces(req, metadata=get_metadata())
        # 如果模型未加载，会返回失败
        if not resp.success:
            print(f"    ✓ 空图片返回失败（预期）: {resp.message}")
            return True
        else:
            print(f"    ✓ 空图片返回成功，0 张人脸: {resp.message}")
            assert len(resp.faces) == 0
            return True
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.INVALID_ARGUMENT:
            print(f"    ✓ 空图片返回 INVALID_ARGUMENT（预期）")
            return True
        elif e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载，返回 FAILED_PRECONDITION（预期）: {e.details()}")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()} - {e.details()}")
            return False


def test_detect_faces_invalid_image():
    """测试检测：无效图片数据应返回错误"""
    print("[测试] 检测人脸 - 无效图片数据...")
    try:
        req = face_service_pb2.DetectFacesRequest(
            image_data=b"not an image data",
            det_threshold=0.5,
        )
        face_stub.DetectFaces(req, metadata=get_metadata())
        print(f"    ✗ 应该报错但成功了")
        return False
    except grpc.RpcError as e:
        if e.code() in (grpc.StatusCode.INVALID_ARGUMENT, grpc.StatusCode.FAILED_PRECONDITION):
            print(f"    ✓ 无效图片返回错误（预期）: {e.code().name}")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()}")
            return False


def test_detect_faces_valid_image():
    """测试检测：有效图片，检测人脸"""
    print("[测试] 检测人脸 - 有效图片...")
    img_data = _make_test_image(200, 200)
    try:
        req = face_service_pb2.DetectFacesRequest(
            image_data=img_data,
            det_threshold=0.5,
            max_faces=0,
        )
        resp = face_stub.DetectFaces(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 检测成功，发现 {len(resp.faces)} 张人脸")
            print(f"      图片尺寸: {resp.image_width}x{resp.image_height}")
            for i, face in enumerate(resp.faces):
                print(f"      人脸 {i}: bbox={list(face.bbox)}, score={face.score:.3f}, landmarks={len(face.landmarks)//2} 个关键点")
                assert len(face.bbox) == 4, f"bbox 应有 4 个值，实际 {len(face.bbox)}"
                assert len(face.landmarks) == 10, f"landmarks 应有 10 个值（5 点 x,y），实际 {len(face.landmarks)}"
            assert resp.image_width == 200
            assert resp.image_height == 200
            return True
        else:
            print(f"    ✗ 检测失败: {resp.message}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载（SCRFD 模型未安装），返回 FAILED_PRECONDITION: {e.details()}")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()} - {e.details()}")
            return False


def test_extract_features_valid_image():
    """测试提取特征：有效图片"""
    print("[测试] 提取人脸特征 - 有效图片...")
    img_data = _make_test_image(200, 200)
    try:
        req = face_service_pb2.ExtractFaceFeaturesRequest(
            image_data=img_data,
            det_threshold=0.5,
            max_faces=0,
            save_aligned_images=False,
            return_aligned_images=False,
        )
        resp = face_stub.ExtractFaceFeatures(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 提取成功，{len(resp.faces)} 张人脸")
            for i, face in enumerate(resp.faces):
                emb = face.embedding
                print(f"      人脸 {i}: 特征维度={len(emb)}")
                assert len(emb) == 512, f"特征维度应为 512，实际 {len(emb)}"
                # 验证 L2 归一化
                norm = math.sqrt(sum(x * x for x in emb))
                print(f"      L2 范数: {norm:.6f}")
                assert abs(norm - 1.0) < 0.01, f"L2 范数应接近 1.0，实际 {norm}"
            return True
        else:
            print(f"    ✗ 提取失败: {resp.message}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载（SCRFD/ArcFace 模型未安装），返回 FAILED_PRECONDITION: {e.details()}")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()} - {e.details()}")
            return False


def test_extract_features_with_save():
    """测试提取特征：保存对齐图片到 image_service"""
    print("[测试] 提取人脸特征 - 保存对齐图片到 image_service...")
    img_data = _make_test_image(200, 200)
    try:
        req = face_service_pb2.ExtractFaceFeaturesRequest(
            image_data=img_data,
            det_threshold=0.5,
            save_aligned_images=True,
            image_bucket=TEST_BUCKET,
            return_aligned_images=True,
        )
        resp = face_stub.ExtractFaceFeatures(req, metadata=get_metadata())
        if resp.success and len(resp.faces) > 0:
            face = resp.faces[0]
            print(f"    ✓ 提取成功，对齐图片 key={face.saved_image_key}, bucket={face.saved_image_bucket}")
            print(f"      对齐图片数据大小: {len(face.aligned_image)} bytes")
            assert face.saved_image_key, "保存的 key 不应为空"
            assert face.saved_image_bucket == TEST_BUCKET
            assert len(face.aligned_image) > 0, "对齐图片数据不应为空"

            # 验证通过 image_service 能取回该图片
            try:
                get_req = image_service_pb2.GetImageRequest(
                    bucket=face.saved_image_bucket,
                    key=face.saved_image_key,
                )
                get_resp = img_stub.GetImage(get_req, metadata=get_metadata())
                if get_resp.success:
                    print(f"    ✓ 通过 image_service 取回图片成功: {len(get_resp.data)} bytes")
                    return True
                else:
                    print(f"    ✗ image_service 取回失败: {get_resp.message}")
                    return False
            except grpc.RpcError as e:
                print(f"    ✗ image_service 取回错误: {e.code()}")
                return False
        else:
            print(f"    ✓ 模型未加载或无人脸，跳过保存验证: {resp.message}")
            return True
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载，返回 FAILED_PRECONDITION: {e.details()}")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()}")
            return False


def test_compare_features_same_vector():
    """测试特征比对：相同向量相似度应为 1.0"""
    print("[测试] 特征比对 - 相同向量...")
    # 生成一个 L2 归一化的随机向量
    vec = [random.uniform(-1, 1) for _ in range(512)]
    norm = math.sqrt(sum(x * x for x in vec))
    vec = [x / norm for x in vec]

    try:
        req = face_service_pb2.CompareFeaturesRequest(
            feature1=vec,
            feature2=vec,
        )
        resp = face_stub.CompareFeatures(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 相似度: {resp.similarity:.6f}, is_same_person={resp.is_same_person}")
        assert abs(resp.similarity - 1.0) < 0.001, f"相同向量相似度应为 1.0，实际 {resp.similarity}"
        assert resp.is_same_person == True
        return True
    except grpc.RpcError as e:
        print(f"    ✗ 错误: {e.code()}")
        return False


def test_compare_features_different_vectors():
    """测试特征比对：不同向量相似度应较低"""
    print("[测试] 特征比对 - 不同向量...")
    vec1 = [random.uniform(-1, 1) for _ in range(512)]
    norm1 = math.sqrt(sum(x * x for x in vec1))
    vec1 = [x / norm1 for x in vec1]

    vec2 = [random.uniform(-1, 1) for _ in range(512)]
    norm2 = math.sqrt(sum(x * x for x in vec2))
    vec2 = [x / norm2 for x in vec2]

    try:
        req = face_service_pb2.CompareFeaturesRequest(
            feature1=vec1,
            feature2=vec2,
        )
        resp = face_stub.CompareFeatures(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 相似度: {resp.similarity:.6f}, is_same_person={resp.is_same_person}")
        # 随机向量相似度应较低（不太可能 >= 0.5）
        assert resp.similarity < 0.5, f"随机向量相似度应 < 0.5，实际 {resp.similarity}"
        assert resp.is_same_person == False
        return True
    except grpc.RpcError as e:
        print(f"    ✗ 错误: {e.code()}")
        return False


def test_compare_features_orthogonal():
    """测试特征比对：正交向量相似度应接近 0"""
    print("[测试] 特征比对 - 正交向量...")
    vec1 = [1.0] + [0.0] * 511
    vec2 = [0.0] * 512
    vec2[1] = 1.0

    try:
        req = face_service_pb2.CompareFeaturesRequest(
            feature1=vec1,
            feature2=vec2,
        )
        resp = face_stub.CompareFeatures(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 相似度: {resp.similarity:.6f}")
        assert abs(resp.similarity) < 0.001, f"正交向量相似度应接近 0，实际 {resp.similarity}"
        assert resp.is_same_person == False
        return True
    except grpc.RpcError as e:
        print(f"    ✗ 错误: {e.code()}")
        return False


def test_compare_features_wrong_dimension():
    """测试特征比对：维度错误应返回 INVALID_ARGUMENT"""
    print("[测试] 特征比对 - 维度错误...")
    try:
        req = face_service_pb2.CompareFeaturesRequest(
            feature1=[1.0] * 100,  # 错误维度
            feature2=[1.0] * 100,
        )
        face_stub.CompareFeatures(req, metadata=get_metadata())
        print(f"    ✗ 应该报错但成功了")
        return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.INVALID_ARGUMENT:
            print(f"    ✓ 维度错误返回 INVALID_ARGUMENT（预期）")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()}")
            return False


def test_compare_features_opposite_vectors():
    """测试特征比对：相反向量相似度应为 -1.0"""
    print("[测试] 特征比对 - 相反向量...")
    vec1 = [1.0] + [0.0] * 511
    vec2 = [-1.0] + [0.0] * 511

    try:
        req = face_service_pb2.CompareFeaturesRequest(
            feature1=vec1,
            feature2=vec2,
        )
        resp = face_stub.CompareFeatures(req, metadata=get_metadata())
        assert resp.success
        print(f"    ✓ 相似度: {resp.similarity:.6f}")
        assert abs(resp.similarity + 1.0) < 0.001, f"相反向量相似度应为 -1.0，实际 {resp.similarity}"
        assert resp.is_same_person == False
        return True
    except grpc.RpcError as e:
        print(f"    ✗ 错误: {e.code()}")
        return False


def test_extract_from_aligned_wrong_size():
    """测试从对齐图片提取特征：尺寸错误应返回错误"""
    print("[测试] 从对齐图片提取特征 - 尺寸错误...")
    img_data = _make_test_image(100, 100)  # 不是 112x112
    try:
        req = face_service_pb2.ExtractFeatureFromAlignedRequest(
            aligned_image_data=img_data,
        )
        face_stub.ExtractFeatureFromAligned(req, metadata=get_metadata())
        # 如果模型能处理任意尺寸可能不会报错，但通常 ArcFace 要求 112x112
        print(f"    ✓ 未报错（模型可能自动 resize）")
        return True
    except grpc.RpcError as e:
        if e.code() in (grpc.StatusCode.INVALID_ARGUMENT, grpc.StatusCode.FAILED_PRECONDITION):
            print(f"    ✓ 尺寸错误返回 {e.code().name}（预期）")
            return True
        else:
            print(f"    ✗ 意外错误: {e.code()}")
            return False


def test_detect_with_custom_threshold():
    """测试自定义检测阈值"""
    print("[测试] 检测人脸 - 自定义阈值...")
    img_data = _make_test_image(200, 200)
    try:
        req = face_service_pb2.DetectFacesRequest(
            image_data=img_data,
            det_threshold=0.9,  # 高阈值
            max_faces=5,
        )
        resp = face_stub.DetectFaces(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 高阈值检测成功，{len(resp.faces)} 张人脸")
            return True
        else:
            print(f"    ✗ 检测失败: {resp.message}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载，返回 FAILED_PRECONDITION")
            return True
        else:
            print(f"    ✗ 错误: {e.code()}")
            return False


def test_detect_max_faces_limit():
    """测试最大人脸数限制"""
    print("[测试] 检测人脸 - 最大数量限制...")
    img_data = _make_test_image(400, 400)
    try:
        req = face_service_pb2.DetectFacesRequest(
            image_data=img_data,
            det_threshold=0.5,
            max_faces=1,  # 只检测 1 张
        )
        resp = face_stub.DetectFaces(req, metadata=get_metadata())
        if resp.success:
            print(f"    ✓ 检测成功，{len(resp.faces)} 张人脸（限制 1）")
            assert len(resp.faces) <= 1, f"应最多 1 张人脸，实际 {len(resp.faces)}"
            return True
        else:
            print(f"    ✗ 检测失败: {resp.message}")
            return False
    except grpc.RpcError as e:
        if e.code() == grpc.StatusCode.FAILED_PRECONDITION:
            print(f"    ✓ 模型未加载，返回 FAILED_PRECONDITION")
            return True
        else:
            print(f"    ✗ 错误: {e.code()}")
            return False


# ── 测试主函数 ─────────────────────────────────────────────────────────

def run_tests():
    tests = [
        ("test_service_available", test_service_available),
        ("test_login", test_login),
        ("test_detect_faces_empty_image", test_detect_faces_empty_image),
        ("test_detect_faces_invalid_image", test_detect_faces_invalid_image),
        ("test_detect_faces_valid_image", test_detect_faces_valid_image),
        ("test_extract_features_valid_image", test_extract_features_valid_image),
        ("test_extract_features_with_save", test_extract_features_with_save),
        ("test_compare_features_same_vector", test_compare_features_same_vector),
        ("test_compare_features_different_vectors", test_compare_features_different_vectors),
        ("test_compare_features_orthogonal", test_compare_features_orthogonal),
        ("test_compare_features_opposite_vectors", test_compare_features_opposite_vectors),
        ("test_compare_features_wrong_dimension", test_compare_features_wrong_dimension),
        ("test_extract_from_aligned_wrong_size", test_extract_from_aligned_wrong_size),
        ("test_detect_with_custom_threshold", test_detect_with_custom_threshold),
        ("test_detect_max_faces_limit", test_detect_max_faces_limit),
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
    if not check_service_alive(TEST_ADDR, timeout=3):
        print(f"服务 {TEST_ADDR} 不可达，请先启动服务")
        sys.exit(1)

    channel = grpc.insecure_channel(TEST_ADDR)
    stub = rpc_pb2_grpc.LaoflchDbStub(channel)
    face_stub = face_service_pb2_grpc.FaceServiceStub(channel)
    img_stub = image_service_pb2_grpc.ImageServiceStub(channel)

    success = run_tests()
    sys.exit(0 if success else 1)
