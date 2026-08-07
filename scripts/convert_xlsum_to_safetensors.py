#!/usr/bin/env python3
"""将 mT5_multilingual_XLSum 的 pytorch_model.bin 转换为 model.safetensors（输出到 /home）"""
import os
import torch
from safetensors.torch import save_file

SRC = "/home/laoflch/tmp_xlsum/pytorch_model.bin"
OUT_DIR = "/home/laoflch/tmp_xlsum"
os.makedirs(OUT_DIR, exist_ok=True)
DST = os.path.join(OUT_DIR, "model.safetensors")

print("loading state_dict from", SRC, flush=True)
sd = torch.load(SRC, map_location="cpu")
# mT5 的 shared.weight 与 encoder/decoder.embed_tokens.weight 共享内存，
# safetensors 拒绝保存共享张量。candle 的 T5 优先加载 shared.weight，
# 删除两个别名即可。
sd.pop("encoder.embed_tokens.weight", None)
sd.pop("decoder.embed_tokens.weight", None)
sd = {k: v.contiguous() for k, v in sd.items()}
print("num tensors:", len(sd), flush=True)
print("saving safetensors to", DST, flush=True)
save_file(sd, DST)
print("DONE", flush=True)
