#!/usr/bin/env python3
"""从 mt5-base 的 spiece.model 直接构建 candle/tokenizers 可加载的 tokenizer.json。

transformers 无法将 SentencePiece 慢分词器转换为 fast tokenizer.json，
因此这里使用 tokenizers 库直接构造 T5 风格的分词器（Unigram + Metaspace）。
"""
import os
import json

import sentencepiece as spm
from tokenizers import Tokenizer, models, pre_tokenizers, decoders, processors, normalizers

MODEL_DIR = "/workspace/rust_space/laoflchDB-rust/laoflch_db_model/mt5-xlsum"
SPIECE = os.path.join(MODEL_DIR, "spiece.model")
OUT = os.path.join(MODEL_DIR, "tokenizer.json")

# 使用 sentencepiece 读取 unigram 词表及分数
sp = spm.SentencePieceProcessor(model_file=SPIECE)
vocab_size = sp.get_piece_size()
# Unigram 模型需要按 id 顺序的 (piece, score) 列表，id 即列表下标
vocab = [(sp.id_to_piece(i), sp.get_score(i)) for i in range(vocab_size)]
print("vocab_size:", vocab_size)

tok = Tokenizer(models.Unigram(vocab, unk_id=2))

# T5 分词器配置
# mT5 的 sentencepiece 使用 nfkc 归一化（如全角 ， → ASCII ,）
tok.normalizer = normalizers.NFKC()
tok.pre_tokenizer = pre_tokenizers.Metaspace(
    replacement="\u2581", prepend_scheme="always"
)
tok.decoder = decoders.Metaspace(
    replacement="\u2581", prepend_scheme="always"
)
# 将 <pad>/</s>/<unk> 注册为特殊 token，便于解码时跳过
tok.add_special_tokens(["<pad>", "</s>", "<unk>"])
# T5 标准模板：输入前加 <pad>，后加 </s>
tok.post_processor = processors.TemplateProcessing(
    single="<pad> $A </s>",
    pair="<pad> $A </s> $B </s>",
    special_tokens=[("<pad>", 0), ("</s>", 1)],
)

tok.save(OUT)
print("saved:", OUT)

# 验证特殊 token id
enc = tok.encode("你好世界", add_special_tokens=True)
print("ids:", enc.ids)
print("tokens:", enc.tokens)
print("vocab_size:", tok.get_vocab_size())

# 验证 round-trip 解码
ids = tok.encode("今天天气很好，适合出去玩。", add_special_tokens=True).ids
print("decoded:", tok.decode(ids, skip_special_tokens=True))
