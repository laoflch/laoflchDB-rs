#!/usr/bin/env python3
"""从 hf-mirror.com 下载 mt5-base 模型（config.json/tokenizer.json/model.safetensors）"""
import os

# 必须在 import huggingface_hub 之前设置，否则 ENDPOINT 常量已在 import 时固化
os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"

from huggingface_hub import hf_hub_download

REPO = "google/mt5-base"
OUT = "/workspace/rust_space/laoflchDB-rust/laoflch_db_model/mt5-base"
# mt5-base 官方仓库无 model.safetensors 与 tokenizer.json，需先下载源文件再转换
FILES = [
    "config.json",
    "spiece.model",
    "tokenizer_config.json",
    "generation_config.json",
    "special_tokens_map.json",
    "pytorch_model.bin",
]

os.makedirs(OUT, exist_ok=True)
for fn in FILES:
    print("downloading", fn, flush=True)
    p = hf_hub_download(repo_id=REPO, filename=fn, local_dir=OUT,
                        local_dir_use_symlinks=False)
    print("ok", fn, os.path.getsize(p), flush=True)
print("DONE", flush=True)
