#!/usr/bin/env python3
"""从 hf-mirror.com 下载 bge-reranker-v2-m3 精排模型。

小文件（config.json / tokenizer.json / tokenizer_config.json / special_tokens_map.json）
下载到模型目录（/workspace），大文件 model.safetensors 下载到 /home 并建立软链接，
以规避 /workspace 磁盘空间不足。
"""
import os

os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"

from huggingface_hub import hf_hub_download

REPO = "BAAI/bge-reranker-v2-m3"
MODEL_DIR = "/workspace/rust_space/laoflchDB-rust/laoflch_db_model/bge-reranker-v2-m3"
BIN_DIR = "/home/laoflch/tmp_reranker"

SMALL_FILES = [
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "special_tokens_map.json",
]
BIG_FILE = "model.safetensors"

os.makedirs(MODEL_DIR, exist_ok=True)
os.makedirs(BIN_DIR, exist_ok=True)

for fn in SMALL_FILES:
    print("downloading", fn, flush=True)
    p = hf_hub_download(repo_id=REPO, filename=fn, local_dir=MODEL_DIR,
                        local_dir_use_symlinks=False)
    print("ok", fn, os.path.getsize(p), flush=True)

print("downloading", BIG_FILE, flush=True)
p = hf_hub_download(repo_id=REPO, filename=BIG_FILE, local_dir=BIN_DIR,
                    local_dir_use_symlinks=False)
print("ok", BIG_FILE, os.path.getsize(p), flush=True)
print("DONE", flush=True)
