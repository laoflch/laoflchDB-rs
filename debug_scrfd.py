#!/usr/bin/env python3
import onnxruntime as ort
import numpy as np
from PIL import Image
import sys

# 加载图片
img_path = sys.argv[1] if len(sys.argv) > 1 else "debug_face.jpg"
img = Image.open(img_path)
orig_w, orig_h = img.size

target_size = 1280

# 计算 scale 和 padding
scale = target_size / max(orig_w, orig_h)
new_w = int(orig_w * scale)
new_h = int(orig_h * scale)
pad_w = (target_size - new_w) // 2
pad_h = (target_size - new_h) // 2

print(f"原图: {orig_w}x{orig_h}")
print(f"resize to {new_w}x{new_h}, scale={scale:.6f}")
print(f"padding: left/top {pad_w}/{pad_h}")

# 预处理
img_resized = img.resize((new_w, new_h), Image.Resampling.BILINEAR)
img_pad = Image.new("RGB", (target_size, target_size), (0, 0, 0))
img_pad.paste(img_resized, (pad_w, pad_h))

# 转换为 tensor [1,3,H,W], BGR, normalized
img_np = np.array(img_pad).astype(np.float32) / 255.0
# RGB → BGR
img_np = img_np[:, :, ::-1].copy()
# 减均值除方差
mean = np.array([0.406, 0.456, 0.485], dtype=np.float32)
std = np.array([0.225, 0.224, 0.229], dtype=np.float32)
img_np = (img_np - mean) / std
# HWC → CHW
img_np = img_np.transpose(2, 0, 1)
# 添加 batch
img_np = np.expand_dims(img_np, axis=0)

# 加载 ONNX 模型
session = ort.InferenceSession("../laoflch_db_model/scrfd_10g.onnx")
input_name = session.get_inputs()[0].name
output_names = [o.name for o in session.get_outputs()]
output_names.sort()

print("\n输出名称（按字母排序）:")
for i, name in enumerate(output_names):
    print(f"  [{i}]: {name}")

# 推理
outputs = session.run(output_names, {input_name: img_np})

print("\n输出形状:")
for name, out in zip(output_names, outputs):
    print(f"  {name}: {out.shape}")

# SCRFD 的输出通常是按 [s32, s16, s8] 顺序排列，每个 stride 有 3 个输出（score, bbox, kps）
strides = [32, 16, 8]
threshold = 0.5

bboxes = []
landmarks = []
scores = []

for stride_idx, stride in enumerate(strides):
    score_idx = stride_idx * 3
    bbox_idx = stride_idx * 3 + 1
    kps_idx = stride_idx * 3 + 2

    if score_idx >= len(outputs):
        break

    score_out = outputs[score_idx]
    bbox_out = outputs[bbox_idx]
    kps_out = outputs[kps_idx]

    print(f"\nstride={stride}")
    print(f"  score shape: {score_out.shape}")
    print(f"  bbox shape: {bbox_out.shape}")
    print(f"  kps shape: {kps_out.shape}")

    # 整理数据
    scores_stride = score_out.flatten()
    bboxes_stride = bbox_out.reshape(-1, 4)
    kps_stride = kps_out.reshape(-1, 10)

    # 计算 anchor 中心
    num_cells = target_size // stride
    num_anchors = len(scores_stride)

    for i in range(num_anchors):
        if scores_stride[i] < threshold:
            continue

        cell_idx = i // 2  # 每个 cell 有 2 个 anchor
        cx = (cell_idx % num_cells) * stride + stride / 2
        cy = (cell_idx // num_cells) * stride + stride / 2

        # 解码 bbox
        bbox = bboxes_stride[i]
        x1 = cx - bbox[0] * stride
        y1 = cy - bbox[1] * stride
        x2 = cx + bbox[2] * stride
        y2 = cy + bbox[3] * stride

        # 反 letterbox
        x1 = (x1 - pad_w) / scale
        y1 = (y1 - pad_h) / scale
        x2 = (x2 - pad_w) / scale
        y2 = (y2 - pad_h) / scale

        # 解码关键点
        kp = kps_stride[i]
        lms = []
        for j in range(5):
            lx = cx + kp[j*2] * stride
            ly = cy + kp[j*2+1] * stride
            lx = (lx - pad_w) / scale
            ly = (ly - pad_h) / scale
            lms.extend([lx, ly])

        bboxes.append([x1, y1, x2, y2])
        landmarks.append(lms)
        scores.append(scores_stride[i])
        print(f"  Face: bbox={[x1,y1,x2,y2]}, score={scores_stride[i]:.4f