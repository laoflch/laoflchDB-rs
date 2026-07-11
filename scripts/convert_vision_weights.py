"""
转换视觉模型权重文件，将 safetensors 中的权重提取并映射到 Rust 代码期望的命名格式。

用法:
    python3 scripts/convert_vision_weights.py <model_dir>
    
示例:
    python3 scripts/convert_vision_weights.py laoflch_db_model/candle/jina-clip-v2
    python3 scripts/convert_vision_weights.py laoflch_db_model/candle/siglip2
"""

import os
import sys
import json
import re
from collections import OrderedDict


def load_safetensors(path):
    """加载 safetensors 文件"""
    from safetensors import safe_open
    tensors = {}
    with safe_open(path, framework='pt') as f:
        for k in f.keys():
            tensors[k] = f.get_tensor(k)
    return tensors


def save_safetensors(tensors, path):
    """保存 safetensors 文件"""
    from safetensors.torch import save_file
    save_file(tensors, path)
    print(f"  已保存 {len(tensors)} 个权重到 {path}")


def extract_vision_weights(tensors, prefix="vision_model."):
    """提取 vision_model 前缀的权重，并去掉前缀"""
    vision = {}
    for k, v in tensors.items():
        if k.startswith(prefix):
            new_key = k[len(prefix):]
            vision[new_key] = v
    print(f"  提取了 {len(vision)} 个视觉权重 (原共 {len(tensors)} 个)")
    return vision


def convert_siglip2(tensors, config):
    """转换 SigLIP2 权重命名"""
    import torch
    mapped = {}
    skipped = 0

    for key, tensor in tensors.items():
        # 跳过 head 相关权重（文本投影头）
        if key.startswith("head."):
            skipped += 1
            continue

        # 跳过 pre_layernorm（SigLIP2 没有这个）
        if key.startswith("pre_layernorm"):
            skipped += 1
            continue

        new_key = key

        # embeddings.patch_embedding → patch_embed.conv
        new_key = new_key.replace("embeddings.patch_embedding", "patch_embed.conv")

        # 处理位置编码：embeddings.position_embedding.weight → pos_embed
        # SigLIP2 没有 CLS token，pos_embed shape=[196, 768]
        # 需要添加 CLS 位置并 reshape 为 [1, 197, 768]
        if new_key == "embeddings.position_embedding.weight":
            new_key = "pos_embed"
            # 在 CLS 位置（索引0）插入一个零向量，并添加 batch 维度
            cls_embed = torch.zeros(1, 1, tensor.shape[1], dtype=tensor.dtype, device=tensor.device)
            mapped["cls_token"] = cls_embed  # [1, 1, hidden_size]
            # 插入 CLS 位置嵌入到 pos_embed 开头 → [197, 768]
            cls_embed_1d = cls_embed.squeeze(0)  # [1, 768]
            pos_with_cls = torch.cat([cls_embed_1d, tensor], dim=0)  # [197, 768]
            # 添加 batch 维度 → [1, 197, 768]
            mapped[new_key] = pos_with_cls.unsqueeze(0)
            print(f"  pos_embed: {list(tensor.shape)} → {list(mapped[new_key].shape)}")
            print(f"  cls_token created: {list(cls_embed.shape)}")
            continue

        # encoder.layers.{i}. → encoder.layer.{i}.
        new_key = re.sub(r'^encoder\.layers\.(\d+)\.', r'encoder.layer.\1.', new_key)

        # self_attn.q_proj → attention.query
        new_key = new_key.replace("self_attn.q_proj", "attention.query")
        new_key = new_key.replace("self_attn.k_proj", "attention.key")
        new_key = new_key.replace("self_attn.v_proj", "attention.value")
        new_key = new_key.replace("self_attn.out_proj", "attention.output")

        # layer_norm1 → attention_ln
        new_key = new_key.replace("layer_norm1", "attention_ln")
        # layer_norm2 → mlp_ln
        new_key = new_key.replace("layer_norm2", "mlp_ln")

        # post_layernorm → encoder.post_ln
        new_key = new_key.replace("post_layernorm", "post_ln")
        if new_key == "post_ln.weight" or new_key == "post_ln.bias":
            new_key = "encoder." + new_key

        mapped[new_key] = tensor

    print(f"  跳过 {skipped} 个 head 权重")
    return mapped


def convert_jina_clip_v2(tensors, config):
    """转换 Jina-CLIP-v2 权重命名"""
    import torch
    mapped = {}
    skipped = 0

    for key, tensor in tensors.items():
        # 跳过 inner_attn_ln（额外的层归一化，ViT 不需要）
        if "attn.inner_attn_ln" in key:
            skipped += 1
            continue

        new_key = key

        # blocks.{i}. → encoder.layer.{i}.
        new_key = re.sub(r'^blocks\.(\d+)\.', r'encoder.layer.\1.', new_key)

        # 处理 q_bias 和 v_bias（特殊 biases，不在 q_proj 中）
        # blocks.{i}.attn.q_bias → encoder.layer.{i}.attn.query.bias
        # 然后再统一处理 .attn. → .attention.
        if new_key.endswith("q_bias"):
            prefix = new_key[:-len("q_bias")]
            new_key = prefix + "query.bias"
        elif new_key.endswith("v_bias"):
            prefix = new_key[:-len("v_bias")]
            new_key = prefix + "value.bias"

        # 标准 attention 投影
        new_key = new_key.replace(".attn.q_proj", ".attention.query")
        new_key = new_key.replace(".attn.k_proj", ".attention.key")
        new_key = new_key.replace(".attn.v_proj", ".attention.value")
        new_key = new_key.replace(".attn.proj", ".attention.output")

        # 处理残留的 .attn. → .attention.（处理 q_bias/v_bias 转换后仍有 .attn. 的情况）
        new_key = new_key.replace(".attn.", ".attention.")

        # norm1 → attention_ln, norm2 → mlp_ln
        new_key = new_key.replace(".norm1.", ".attention_ln.")
        new_key = new_key.replace(".norm2.", ".mlp_ln.")

        # patch_embed.proj → patch_embed.conv
        new_key = new_key.replace("patch_embed.proj", "patch_embed.conv")

        # norm → post_ln（最后的层归一化）
        if new_key == "norm.weight" or new_key == "norm.bias":
            new_key = "encoder.post_ln." + new_key.split(".")[1]

        # 处理 MLP（Jina-CLIP-v2 使用 SwiGLU MLP: w1=gate, w2=up, w3=down）
        # 标准 ViT MLP 使用 fc1 (hidden→intermediate), fc2 (intermediate→hidden)
        # w2.weight: [intermediate, hidden] → fc1.weight: [intermediate, hidden] (up projection)
        # w3.weight: [hidden, intermediate] → fc2.weight: [hidden, intermediate] (down projection)
        # 跳过 w1 (gate projection) 和 ffn_ln (内部 layer norm)
        if new_key.endswith(".w1.weight") or new_key.endswith(".w1.bias"):
            skipped += 1
            continue
        if "ffn_ln" in new_key:
            skipped += 1
            continue
        new_key = new_key.replace(".w2.", ".fc1.")
        new_key = new_key.replace(".w3.", ".fc2.")

        # cls_token, pos_embed 保持不变
        mapped[new_key] = tensor

    print(f"  跳过 {skipped} 个 inner_attn_ln/w1/ffn_ln 权重")

    # 为缺失的 key.bias 添加零偏置（Jina-CLIP-v2 的 k_proj 没有 bias）
    num_layers = 0
    for k in mapped:
        if k.startswith("encoder.layer.") and k.endswith(".attention.key.weight"):
            num_layers += 1
    added_key_bias = 0
    for i in range(num_layers):
        key_bias_key = f"encoder.layer.{i}.attention.key.bias"
        if key_bias_key not in mapped:
            # 从 key.weight 的 hidden_size 推断 bias 形状
            key_weight = mapped.get(f"encoder.layer.{i}.attention.key.weight")
            if key_weight is not None:
                hidden_size = key_weight.shape[0]  # [hidden, hidden]
                zero_bias = torch.zeros(hidden_size, dtype=key_weight.dtype, device=key_weight.device)
                mapped[key_bias_key] = zero_bias
                added_key_bias += 1
    if added_key_bias > 0:
        print(f"  添加了 {added_key_bias} 个缺失的 key.bias（零偏置）")

    # 检查 pos_embed 和 cls_token 形状
    if "pos_embed" in mapped:
        print(f"  pos_embed shape: {list(mapped['pos_embed'].shape)}")
    if "cls_token" in mapped:
        print(f"  cls_token shape: {list(mapped['cls_token'].shape)}")

    return mapped


def main():
    if len(sys.argv) < 2:
        print("用法: python3 convert_vision_weights.py <model_dir>")
        sys.exit(1)

    model_dir = sys.argv[1]
    safetensors_path = os.path.join(model_dir, "model.safetensors")
    config_path = os.path.join(model_dir, "config.json")

    if not os.path.exists(safetensors_path):
        print(f"错误: 找不到 {safetensors_path}")
        sys.exit(1)

    if not os.path.exists(config_path):
        print(f"错误: 找不到 {config_path}")
        sys.exit(1)

    with open(config_path) as f:
        config = json.load(f)

    model_type = config.get("model_type", "")
    vision_config = config.get("vision_config", {})
    vision_model_type = vision_config.get("model_type", "")

    print(f"模型目录: {model_dir}")
    print(f"  model_type: {model_type}")
    print(f"  vision_model_type: {vision_model_type}")

    print("加载 safetensors...")
    all_tensors = load_safetensors(safetensors_path)

    # 提取 vision_model 权重
    vision_tensors = extract_vision_weights(all_tensors, prefix="vision_model.")

    # 根据模型类型转换
    if vision_model_type in ("siglip2", "siglip_vision_model") or model_type == "siglip":
        print("应用 SigLIP2 权重映射...")
        converted = convert_siglip2(vision_tensors, config)
    elif vision_model_type in ("jina-clip-v2", "jina_clip_vision") or model_type in ("jina-clip-v2", "jina_clip"):
        print("应用 Jina-CLIP-v2 权重映射...")
        converted = convert_jina_clip_v2(vision_tensors, config)
    else:
        print(f"未知的模型类型: vision_model_type={vision_model_type}, model_type={model_type}, 使用原始名称")
        converted = vision_tensors

    print(f"转换后: {len(converted)} 个权重")

    # 打印权重名称预览
    print("\n权重名称预览:")
    for k in sorted(converted.keys())[:20]:
        print(f"  {k}: {list(converted[k].shape)}")
    if len(converted) > 20:
        print(f"  ... (共 {len(converted)} 个)")

    # 检查是否有残留的 .attn. 名称
    attn_issues = [k for k in converted if ".attn." in k]
    if attn_issues:
        print(f"\n⚠️ 警告: 以下 {len(attn_issues)} 个权重仍有 .attn. 前缀:")
        for k in attn_issues[:5]:
            print(f"  {k}")

    # 备份原始文件
    backup_path = safetensors_path + ".bak"
    if not os.path.exists(backup_path):
        os.rename(safetensors_path, backup_path)
        print(f"\n原始文件已备份到: {backup_path}")

    # 保存转换后的文件
    save_safetensors(converted, safetensors_path)


if __name__ == "__main__":
    main()