"""
下载 SigLIP2 和 Jina-CLIP-v2 视觉模型，提取视觉权重并转换格式。

用法:
    python3 scripts/download_vision_models.py
"""
import os
import sys
import shutil
import argparse
from pathlib import Path

# 在导入 huggingface_hub 之前设置镜像端点
USE_MIRROR = True
if "--no-mirror" in sys.argv:
    USE_MIRROR = False

HF_ENDPOINT = "https://hf-mirror.com" if USE_MIRROR else "https://huggingface.co"
os.environ["HF_ENDPOINT"] = HF_ENDPOINT

# 现在导入 huggingface_hub
from huggingface_hub import hf_hub_download, HfApi

# 模型根目录
MODEL_ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "laoflch_db_model", "candle")

# 模型配置
MODELS = {
    "siglip2": {
        "hf_model_id": "google/siglip2-base-patch16-224",
        "output_dir": os.path.join(MODEL_ROOT, "siglip2"),
    },
    "jina-clip-v2": {
        "hf_model_id": "jinaai/jina-clip-v2",
        "output_dir": os.path.join(MODEL_ROOT, "jina-clip-v2"),
    },
}


def download_model(model_id, output_dir):
    """从 HuggingFace 下载模型"""
    os.makedirs(output_dir, exist_ok=True)

    print(f"  从 {HF_ENDPOINT} 下载模型 {model_id} ...")
    print(f"  保存到: {output_dir}")

    # 下载 config.json
    config_path = hf_hub_download(
        repo_id=model_id,
        filename="config.json",
        local_dir=output_dir,
        local_dir_use_symlinks=False,
    )
    print(f"  config.json -> {config_path}")

    # 获取所有 safetensors 文件列表
    api = HfApi(endpoint=HF_ENDPOINT)
    model_info = api.model_info(model_id)

    safetensors_files = sorted([
        f.rfilename for f in model_info.siblings
        if f.rfilename.endswith(".safetensors")
    ])

    if not safetensors_files:
        print(f"  ⚠️ 未找到 safetensors 文件")
        return []

    print(f"  找到 {len(safetensors_files)} 个 safetensors 文件")
    downloaded_files = []
    for filename in safetensors_files:
        file_path = hf_hub_download(
            repo_id=model_id,
            filename=filename,
            local_dir=output_dir,
            local_dir_use_symlinks=False,
        )
        downloaded_files.append(file_path)
        size_mb = os.path.getsize(file_path) / 1024 / 1024
        print(f"    {filename}: {size_mb:.1f} MB")

    return downloaded_files


def merge_sharded_safetensors(input_dir, output_path):
    """合并分片的 safetensors 文件"""
    from safetensors.torch import load_file, save_file

    shard_files = sorted([
        os.path.join(input_dir, f)
        for f in os.listdir(input_dir)
        if f.startswith("model-") and f.endswith(".safetensors")
    ])

    if not shard_files:
        model_file = os.path.join(input_dir, "model.safetensors")
        if os.path.exists(model_file):
            return model_file
        return None

    print(f"  合并 {len(shard_files)} 个分片文件...")
    merged = {}
    for shard_path in shard_files:
        tensors = load_file(shard_path)
        merged.update(tensors)

    save_file(merged, output_path)
    print(f"  合并完成: {len(merged)} 个权重 -> {output_path}")
    return output_path


def main():
    parser = argparse.ArgumentParser(description="下载视觉模型")
    parser.add_argument("--no-mirror", action="store_true", help="不使用镜像")
    parser.add_argument("models", nargs="*", default=["all"],
                        help="要下载的模型: siglip2, jina-clip-v2, all")
    args = parser.parse_args()

    models_to_download = args.models
    if not models_to_download or "all" in models_to_download:
        models_to_download = list(MODELS.keys())

    mirror_info = "使用镜像 hf-mirror.com" if USE_MIRROR else "直连 huggingface.co"
    print(f"📦 {mirror_info}\n")

    for model_name in models_to_download:
        if model_name not in MODELS:
            print(f"未知模型: {model_name}")
            continue

        config = MODELS[model_name]
        output_dir = config["output_dir"]
        hf_model_id = config["hf_model_id"]

        print(f"{'='*60}")
        print(f"下载模型: {model_name} ({hf_model_id})")
        print(f"{'='*60}")

        # 备份旧文件
        if os.path.exists(output_dir):
            bak_dir = output_dir.rstrip("/") + ".bak"
            if os.path.exists(bak_dir):
                shutil.rmtree(bak_dir)
            shutil.move(output_dir, bak_dir)
            print(f"  备份旧目录 -> {bak_dir}")

        # 下载模型
        try:
            downloaded = download_model(hf_model_id, output_dir)
            if not downloaded:
                print(f"  ❌ 未下载到 safetensors 文件")
                continue
        except Exception as e:
            print(f"  ❌ 下载失败: {e}")
            continue

        # 合并分片
        model_safetensors = os.path.join(output_dir, "model.safetensors")
        if not os.path.exists(model_safetensors):
            merged = merge_sharded_safetensors(output_dir, model_safetensors)
            if merged:
                print(f"  ✓ 合并完成")
        else:
            print(f"  ✓ model.safetensors 已存在")

        # 打印文件列表
        print(f"\n  下载完成，文件列表:")
        for f in sorted(os.listdir(output_dir)):
            filepath = os.path.join(output_dir, f)
            if os.path.isfile(filepath):
                size_mb = os.path.getsize(filepath) / 1024 / 1024
                print(f"    {f:40s} {size_mb:.1f} MB")

    print(f"\n{'='*60}")
    print(f"下载完成！请运行转换脚本:")
    print(f"  python3 scripts/convert_vision_weights.py {MODELS['siglip2']['output_dir']}")
    print(f"  python3 scripts/convert_vision_weights.py {MODELS['jina-clip-v2']['output_dir']}")
    print(f"{'='*60}")


if __name__ == "__main__":
    main()