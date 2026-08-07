#!/usr/bin/env python3
"""从 hf-mirror.com 下载 mT5_multilingual_XLSum 摘要模型。

小文件下载到模型目录（/workspace），大文件 pytorch_model.bin 下载到 /home
以规避 /workspace 磁盘空间不足。随后需转换 safetensors 并生成 tokenizer.json。
"""
import os

# 必须在 import huggingface_hub 之前设置
os.environ["HF_ENDPOINT"] = "https://hf-mirror.com"

from huggingface_hub import hf_hub_download

REPO = "csebuetnlp/mT5_multilingual_XLSum"
MODEL_DIR = "/workspace/rust_space/laoflchDB-rust/laoflch_db_model/mt5-xlsum"
BIN_DIR = "/home/laoflch/tmp_xlsum"

# 小文件：直接进模型目录
SMALL_FILES = [
    "config.json",
    "spiece.model",
    "tokenizer_config.json",
    "special_tokens_map.json",
]
# 大文件：单独下载到 /home
BIG_FILE = "pytorch_model.bin"

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
