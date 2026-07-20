#!/usr/bin/env python3
import numpy as np
from PIL import Image
import onnxruntime as ort

# 加载模型
model_path = '/workspace/rust_space/laoflchDB-rust/laoflch_db_model/scrfd_10g.onnx'
session = ort.InferenceSession(model_path)
input_name = session.get_inputs()[0].name
print("输入名称:", input_name)
print("输入形状:", session.get_inputs()[0].shape)
print("\n输出:")
for o in session.get_outputs():
    print(f"  {o.name}: {o.shape}")

# 读取测试图片
img_path = '/home/laoflch/Pictures/bda1aede-5d48-3bad-a742-1d6201e13044.jpeg'
img = Image.open(img_path).convert('RGB')
orig_w, orig_h = img.size
print(f"\n原图尺寸: {orig_w}x{orig_h}")

# 预处理
target_size = 1280
scale = target_size / max(orig_w, orig_h)
new_w = int(orig_w * scale + 0.5)
new_h = int(orig_h * scale + 0.5)
pad_w = (target_size - new_w) // 2
pad_h = (target_size - new_h) // 2

#  resize
resized = img.resize((new_w, new_h), Image.Resampling.LANCZOS)  # LANCZOS

# letterbox
canvas = Image.new('RGB', (target_size, target_size), (0,0,0))
canvas.paste(resized, (pad_w, pad_h))

# 预处理
# RGB -> numpy
np_img = np.array(canvas)
# (H, W, 3)
# 归一化
mean = np.array([0.485, 0.456, 0.406], dtype=np.float32)
std = np.array([0.229, 0.224, 0.225], dtype=np.float32)
input_data = np_img.astype(np.float32) / 255.0
input_data = (input_data - mean) / std

# 转换为 NCHW, (H, W, C) -> (C, H, W)
input_data = input_data.transpose(2, 0, 1)
input_data = np.expand_dims(input_data, axis=0)
print(f"\n预处理后输入形状: {input_data.shape}")
print(f"  scale: {scale}")
print(f"  pad_w: {pad_w}")
print(f"  pad_h: {pad_h}")

# 推理
outputs = session.run(None, {input_name: input_data})
print(f"\n推理输出数量: {len(outputs)}")
for i, o in enumerate(outputs):
    print(f"  输出 [{i}]: shape {o.shape}")

# 按输出名称排序
output_names = [o.name for o in session.get_outputs()]
sorted_pairs = sorted(zip(output_names, outputs), key=lambda x: x[0])
print("\n按名称排序后的输出:")
for name, arr in sorted_pairs:
    print(f"  {name}: shape {arr.shape}")

# 解析
print("\n=== 我们尝试解析所有检测到的人脸（使用 stride 32,16,8，sorted_pairs 顺序 ===")
all_faces = []

# sorted_pairs 是:
# [0]: score_32
# [1]: bbox_32
# [2]: kps_32
# [3]: score_16
# [4]: bbox_16
# [5]: kps_16
# [6]: score_8
# [7]: bbox_8
# [8]: kps_8
strides = [32, 16, 8]
for stride_idx, stride in enumerate(strides):
    score_idx = stride_idx * 3
    bbox_idx = stride_idx * 3 + 1
    kps_idx = stride_idx * 3 + 2
    print(f"\n--- Stride {stride} ---")
    score_arr = sorted_pairs[score_idx][1].flatten()
    bbox_arr = sorted_pairs[bbox_idx][1].reshape(-1,4)
    kps_arr = sorted_pairs[kps_idx][1].reshape(-1,10)
    print(f"score min/max: {score_arr.min():.4f}/{score_arr.max():.4f}")

    # 找出 score > 0.1 的
    threshold = 0.1
    good_indices = np.where(score_arr > threshold)[0]
    print(f"Good indices (score > {threshold}): {len(good_indices)}")

    # num cells per row
    num_cells = target_size // stride
    for i in good_indices:
        cell_idx = i // 2  # 每个cell有2个anchor
        cx = (cell_idx % num_cells) * stride + stride // 2
        cy = (cell_idx // num_cells) * stride + stride // 2
        dx1, dy1, dx2, dy2 = bbox_arr[i]
        x1 = cx - dx1 * stride
        y1 = cy - dy1 * stride
        x2 = cx + dx2 * stride
        y2 = cy + dy2 * stride
        # 反 letterbox
        orig_x1 = (x1 - pad_w) / scale
        orig_y1 = (y1 - pad_h) / scale
        orig_x2 = (x2 - pad_w) / scale
        orig_y2 = (y2 - pad_h) / scale
        # 解析 kps
        kps = []
        for j in range(5):
            kx = cx + kps_arr[i][j*2] * stride
            ky = cy + kps_arr[i][j*2+1] * stride
            k_orig_x = (kx - pad_w) / scale
            k_orig_y = (ky - pad_h) / scale
            kps.append([k_orig_x, k_orig_y])
        all_faces.append({
            "score": score_arr[i],
            "bbox": [orig_x1, orig_y1, orig_x2, orig_y2],
            "landmarks": kps
        })
        print(f"  [{i}] score={score_arr[i]:.4f} bbox=[{orig_x1:.1f},{orig_y1:.1f},{orig_x2:.1f},{orig_y2:.1f}]")


# 现在画出检测框（用PIL画线）
from PIL import ImageDraw
img_draw = Image.open(img_path).copy()
draw = ImageDraw.Draw(img_draw)
for face in all_faces:
    x1, y1, x2, y2 = face["bbox"]
    draw.rectangle([(x1, y1), (x2, y2)], outline=(0,255,0), width=10)
    # 画关键点
    for p in face["landmarks"]:
        draw.ellipse([(p[0]-10, p[1]-10), (p[0]+10, p[1]+10)], fill=(0,0,255))
out_img_path = "/tmp/test_detected.jpg"
img_draw.save(out_img_path)
print(f"\n检测图保存到: {out_img_path}")

# 按置信度降序，取最大的一个，然后裁剪
if all_faces:
    all_faces.sort(key=lambda x: -x["score"])
    best_face = all_faces[0]
    print(f"\n最佳人脸: score={best_face['score']:.4f}, bbox={best_face['bbox']}")
    # 裁剪
    x1, y1, x2, y2 = best_face["bbox"]
    # 扩展一下（2.0倍）
    cx = (x1 + x2) / 2
    cy = (y1 + y2) / 2
    width = x2 - x1
    height = y2 - y1
    size = max(width, height) * 2.0
    half_size = size / 2
    crop_x1 = cx - half_size
    crop_y1 = cy - half_size
    crop_x2 = cx + half_size
    crop_y2 = cy + half_size
    # 也可以用关键点的眼中心！
    eye1 = best_face["landmarks"][0]
    eye2 = best_face["landmarks"][1]
    eye_cx = (eye1[0] + eye2[0])/2
    eye_cy = (eye1[1] + eye2[1])/2
    print(f"用关键点眼中心：eye center [{eye_cx:.1f}, {eye_cy:.1f}]")
    crop_x1 = eye_cx - half_size
    crop_y1 = eye_cy - half_size - size * 0.15 # 向上移一点
    crop_x2 = eye_cx + half_size
    crop_y2 = eye_cy + half_size - size * 0.15

    # 裁剪
    cropped = img_draw.crop((crop_x1, crop_y1, crop_x2, crop_y2))
    # resize到112x112
    cropped = cropped.resize((112, 112), Image.Resampling.LANCZOS)
    out_crop_path = "/tmp/test_best_face.jpg"
    cropped.save(out_crop_path)
    print(f"最佳人脸裁剪到: {out_crop_path}")

print("\n=== END ===")
