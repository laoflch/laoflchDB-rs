"""
修复视觉模型 config.json，添加 Rust 代码期望的字段。

用法:
    python3 scripts/fix_vision_config.py <model_dir>
"""

import os
import sys
import json

def fix_jina_clip_v2(config):
    """修复 Jina-CLIP-v2 的 config.json"""
    vc = config.get("vision_config", {})
    # 从原始字段映射
    hidden_size = vc.get("width", 1024)
    num_layers = vc.get("layers", 24)
    head_width = vc.get("head_width", 64)
    num_heads = hidden_size // head_width  # 1024 / 64 = 16
    mlp_ratio = vc.get("mlp_ratio", 2.6667)
    intermediate_size = int(hidden_size * mlp_ratio)  # 1024 * 2.6667 ≈ 2730

    vc["hidden_size"] = hidden_size
    vc["num_hidden_layers"] = num_layers
    vc["num_attention_heads"] = num_heads
    vc["intermediate_size"] = intermediate_size
    vc["image_size"] = vc.get("image_size", 512)
    vc["patch_size"] = vc.get("patch_size", 14)
    # 保留原始字段
    config["vision_config"] = vc

    print(f"  Jina-CLIP-v2: hidden_size={hidden_size}, layers={num_layers}, "
          f"heads={num_heads}, intermediate={intermediate_size}, "
          f"image_size={vc['image_size']}, patch_size={vc['patch_size']}")
    return config

def fix_siglip2(config, model_dir):
    """修复 SigLIP2 的 config.json"""
    from safetensors import safe_open

    # 从模型权重推断参数
    model_path = None
    for f in os.listdir(model_dir):
        if f.endswith(".safetensors") and "bak" not in f:
            model_path = os.path.join(model_dir, f)
            break

    if not model_path:
        print("  ⚠️ 未找到 safetensors 文件，使用默认参数")
        return config

    vc = config.get("vision_config", {})
    with safe_open(model_path, framework="pt") as f:
        keys = list(f.keys())
        # 收集所有需要的形状信息
        shapes = {k: list(f.get_tensor(k).shape) for k in keys}

    # 从权重推断参数
    hidden_size = None
    num_layers = 0
    intermediate_size = None
    patch_size = None
    image_size = None

    # hidden_size = query.weight shape[0]
    for k, shape in shapes.items():
        clean_k = k
        if clean_k.startswith("vision_model."):
            clean_k = clean_k[len("vision_model."):]
        if "self_attn.q_proj.weight" in clean_k or "attention.query.weight" in clean_k:
            hidden_size = shape[0]
            break

    # 计算层数
    import re
    for k in shapes:
        m = re.search(r'(?:encoder\.layers\.|encoder\.layer\.|blocks\.)(\d+)', k)
        if m:
            num_layers = max(num_layers, int(m.group(1)) + 1)

    # 推断 intermediate_size
    for k, shape in shapes.items():
        clean_k = k
        if clean_k.startswith("vision_model."):
            clean_k = clean_k[len("vision_model."):]
        if "mlp.fc1.weight" in clean_k or "mlp.w2.weight" in clean_k:
            intermediate_size = shape[0]
            break
        if "mlp.w3.weight" in clean_k:
            intermediate_size = shape[1]
            break

    # 推断 patch_size 和 image_size from pos_embed
    for k, shape in shapes.items():
        clean_k = k
        if clean_k.startswith("vision_model."):
            clean_k = clean_k[len("vision_model."):]
        if "position_embedding" in clean_k or "pos_embed" in clean_k:
            if len(shape) == 3:
                seq_len = shape[1]
            elif len(shape) == 2:
                seq_len = shape[0]
            else:
                seq_len = shape[0]
            patch_size = 16
            if seq_len > 1:
                has_cls = any("cls_token" in k2 for k2 in shapes)
                if has_cls:
                    num_patches = seq_len - 1
                else:
                    num_patches = seq_len
                side = int(num_patches ** 0.5)
                if side * side == num_patches:
                    image_size = side * patch_size

    if hidden_size is None:
        hidden_size = 768
    if num_layers == 0:
        num_layers = 12
    if intermediate_size is None:
        intermediate_size = 3072
    if patch_size is None:
        patch_size = 16
    if image_size is None:
        image_size = 224

    num_heads = hidden_size // 64

    vc["hidden_size"] = hidden_size
    vc["num_hidden_layers"] = num_layers
    vc["num_attention_heads"] = num_heads
    vc["intermediate_size"] = intermediate_size
    vc["image_size"] = image_size
    vc["patch_size"] = patch_size
    config["vision_config"] = vc

    print(f"  SigLIP2: hidden_size={hidden_size}, layers={num_layers}, "
          f"heads={num_heads}, intermediate={intermediate_size}, "
          f"image_size={image_size}, patch_size={patch_size}")
    return config


def main():
    if len(sys.argv) < 2:
        print("用法: python3 fix_vision_config.py <model_dir>")
        sys.exit(1)

    model_dir = sys.argv[1]
    config_path = os.path.join(model_dir, "config.json")

    if not os.path.exists(config_path):
        print(f"错误: 找不到 {config_path}")
        sys.exit(1)

    with open(config_path) as f:
        config = json.load(f)

    model_type = config.get("model_type", "")
    vision_config = config.get("vision_config", {})
    vision_model_type = vision_config.get("model_type", "")

    print(f"修复 config.json: {model_dir}")
    print(f"  model_type: {model_type}")
    print(f"  vision_model_type: {vision_model_type}")

    if "jina" in model_type or "jina" in vision_model_type:
        config = fix_jina_clip_v2(config)
    elif "siglip" in model_type or "siglip" in vision_model_type:
        config = fix_siglip2(config, model_dir)
    else:
        print(f"  ⚠️ 未知的模型类型，跳过")
        return

    # 备份原始 config
    bak_path = config_path + ".bak"
    if not os.path.exists(bak_path):
        os.rename(config_path, bak_path)
        print(f"  原始 config 已备份到: {bak_path}")

    # 保存修复后的 config
    with open(config_path, "w") as f:
        json.dump(config, f, indent=2)
    print(f"  ✓ 已保存修复后的 config.json")


if __name__ == "__main__":
    main()