#!/usr/bin/env python3
"""将 mt5-base 的 pytorch_model.bin 转换为 model.safetensors（输出到 /home 以规避 /workspace 磁盘不足）"""
import os
import torch
from safetensors.torch import save_file

SRC = "/workspace/rust_space/laoflchDB-rust/laoflch_db_model/mt5-base/pytorch_model.bin"
OUT_DIR = "/home/laoflch/tmp_mt5"
os.makedirs(OUT_DIR, exist_ok=True)
DST = os.path.join(OUT_DIR, "model.safetensors")

print("loading state_dict from", SRC, flush=True)
sd = torch.load(SRC, map_location="cpu")
# mt5 的 shared.weight 与 encoder/decoder.embed_tokens.weight 共享内存，
# safetensors 拒绝保存共享张量。candle 的 T5 优先加载 shared.weight，
# encoder/decoder 的 embed_tokens 只是别名，直接删除即可。
sd.pop("encoder.embed_tokens.weight", None)
sd.pop("decoder.embed_tokens.weight", None)
# 确保所有张量连续，否则 safetensors 拒绝保存
sd = {k: v.contiguous() for k, v in sd.items()}
print("num tensors:", len(sd), flush=True)
print("saving safetensors to", DST, flush=True)
save_file(sd, DST)
print("DONE", flush=True)
