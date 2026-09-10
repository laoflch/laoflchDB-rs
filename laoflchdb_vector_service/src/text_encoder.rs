//! JinaBERT 文本编码器模块
//!
//! 为 jina-clip-v2 的文本塔（基于 jina-embeddings-v3 的 JinaBERT）提供
//! 纯 candle 推理实现：
//! - 无 position_embeddings，使用 RoPE 旋转位置编码
//! - 融合 QKV 权重（Wqkv）
//! - Post-LN（prenorm=False，对齐官方 xlm-roberta-flash-implementation）
//! - GELU MLP
//! - mean pooling + L2 归一化，输出 1024 维向量
//!
//! 权重文件：text_model.safetensors（由 Python 脚本从完整权重提取并融合 LoRA）

use candle_core::{Device, DType, Tensor};
use candle_nn::VarBuilder;
use log::{info, warn};
use std::path::Path;

/// RoPE 基频
const ROPE_THETA: f64 = 10000.0;

// ============================================================================
// 手动 LayerNorm（与 vision_encoder.rs 一致，避免 CUDA kernel 依赖）
// ============================================================================

struct CudaLayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
}

impl CudaLayerNorm {
    fn load(size: usize, eps: f64, vb: VarBuilder) -> Result<Self, candle_core::Error> {
        let weight = vb.get_with_hints(size, "weight", candle_nn::Init::Const(1.0))?;
        let bias = vb.get_with_hints(size, "bias", candle_nn::Init::Const(0.0))?;
        Ok(Self { weight, bias, eps })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor, candle_core::Error> {
        let dim = x.dims().len() - 1;
        let mean = x.mean_keepdim(dim)?;
        let centered = x.broadcast_sub(&mean)?;
        let variance = centered.sqr()?.mean_keepdim(dim)?;
        let eps_t = Tensor::new(self.eps as f32, x.device())?;
        let normalized = centered.broadcast_div(&(variance.broadcast_add(&eps_t)?).sqrt()?)?;
        normalized.broadcast_mul(&self.weight)?.broadcast_add(&self.bias)
    }
}

// ============================================================================
// RoPE 旋转位置编码
// ============================================================================

/// 生成旋转位置编码的 inv_freq，形状 [head_dim/2]
fn rope_inv_freq(head_dim: usize, device: &Device) -> Result<Tensor, candle_core::Error> {
    let half = head_dim / 2;
    let mut freqs = Vec::with_capacity(half);
    for i in 0..half {
        let theta = ROPE_THETA.powf(-2.0 * i as f64 / head_dim as f64);
        freqs.push(theta as f32);
    }
    Tensor::from_vec(freqs, (half,), device)
}

/// 对 Q/K 应用 RoPE
/// x: [B, H, S, D]，返回相同形状
fn apply_rope(x: &Tensor, inv_freq: &Tensor) -> Result<Tensor, candle_core::Error> {
    let (_b, _h, s, d) = x.dims4()?;
    let half = d / 2;

    // freqs: [S, half]
    let positions = Tensor::arange(0u32, s as u32, x.device())?; // [S]
    let freqs = positions
        .unsqueeze(1)?
        .to_dtype(DType::F32)?
        .matmul(&inv_freq.unsqueeze(0)?)?; // [S, half]

    let cos = freqs.cos()?; // [S, half]
    let sin = freqs.sin()?;

    // 拼接为 [S, D]
    let cos = Tensor::cat(&[&cos, &cos], 1)?;
    let sin = Tensor::cat(&[&sin, &sin], 1)?;
    let cos = cos.unsqueeze(0)?.unsqueeze(0)?; // [1, 1, S, D]
    let sin = sin.unsqueeze(0)?.unsqueeze(0)?;

    // 旋转半维: rotated = [-x2, x1]
    let x1 = x.narrow(3, 0, half)?;
    let x2 = x.narrow(3, half, half)?;
    let rotated = Tensor::cat(&[&x2.neg()?, &x1], 3)?;

    x.broadcast_mul(&cos)? + rotated.broadcast_mul(&sin)?
}

// ============================================================================
// 文本编码器层
// ============================================================================

struct JinaBertLayer {
    norm1: CudaLayerNorm,
    wqkv: Tensor,      // [3*hidden, hidden]
    wqkv_bias: Tensor, // [3*hidden]
    out_proj: Tensor,  // [hidden, hidden]
    out_proj_bias: Tensor,
    norm2: CudaLayerNorm,
    fc1: Tensor,      // [intermediate, hidden]
    fc1_bias: Tensor, // [intermediate]
    fc2: Tensor,      // [hidden, intermediate]
    fc2_bias: Tensor, // [hidden]
}

impl JinaBertLayer {
    fn load(vb: VarBuilder, hidden: usize, intermediate: usize) -> Result<Self, candle_core::Error> {
        Ok(Self {
            norm1: CudaLayerNorm::load(hidden, 1e-5, vb.pp("norm1"))?,
            wqkv: vb.get((3 * hidden, hidden), "mixer.Wqkv.weight")?,
            wqkv_bias: vb.get(3 * hidden, "mixer.Wqkv.bias")?,
            out_proj: vb.get((hidden, hidden), "mixer.out_proj.weight")?,
            out_proj_bias: vb.get(hidden, "mixer.out_proj.bias")?,
            norm2: CudaLayerNorm::load(hidden, 1e-5, vb.pp("norm2"))?,
            fc1: vb.get((intermediate, hidden), "mlp.fc1.weight")?,
            fc1_bias: vb.get(intermediate, "mlp.fc1.bias")?,
            fc2: vb.get((hidden, intermediate), "mlp.fc2.weight")?,
            fc2_bias: vb.get(hidden, "mlp.fc2.bias")?,
        })
    }

    fn forward(
        &self,
        hidden_states: &Tensor,
        attention_mask: &Tensor,
        num_heads: usize,
        head_dim: usize,
        inv_freq: &Tensor,
        device: &Device,
    ) -> Result<Tensor, candle_core::Error> {
        let (b, s, hidden) = hidden_states.dims3()?;

        // ---- Post-LN 注意力（官方 prenorm=False：先 mixer，再 norm1 于残差和之后）----
        // 融合 QKV（直接对 hidden_states 计算，不先过 norm1）
        let qkv = hidden_states
            .broadcast_matmul(&self.wqkv.t()?.contiguous()?)?
            .broadcast_add(&self.wqkv_bias)?; // [B, S, 3*hidden]

        let q = qkv.narrow(2, 0, hidden)?;
        let k = qkv.narrow(2, hidden, hidden)?;
        let v = qkv.narrow(2, 2 * hidden, hidden)?;

        let q = q.reshape((b, s, num_heads, head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;
        let k = k.reshape((b, s, num_heads, head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;
        let v = v.reshape((b, s, num_heads, head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;

        // RoPE（仅作用于 Q/K，V 不旋转，对齐官方 ApplyRotaryEmbQKV_）
        let q = apply_rope(&q, inv_freq)?;
        let k = apply_rope(&k, inv_freq)?;

        let scale = 1.0f32 / (head_dim as f32).sqrt();
        let scores = q.matmul(&k.t()?.contiguous()?)?; // [B, H, S, S]
        let scores = scores.broadcast_mul(&Tensor::new(scale, device)?)?;
        let scores = scores.broadcast_add(attention_mask)?;

        // softmax 在 CPU 上执行（CUDA 兼容）
        let orig_device = scores.device().clone();
        let scores_cpu = scores.to_device(&Device::Cpu)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores_cpu)?;
        let probs = probs.to_device(&orig_device)?;

        let context = probs.matmul(&v)?; // [B, H, S, D]
        let context = context.permute((0, 2, 1, 3))?.contiguous()?;
        let context = context.reshape((b, s, hidden))?;

        let attn_out = context
            .broadcast_matmul(&self.out_proj.t()?.contiguous()?)?
            .broadcast_add(&self.out_proj_bias)?;
        let h = self.norm1.forward(&(hidden_states + attn_out)?)?;

        // ---- Post-LN MLP（norm2 于残差和之后）----
        let x = h
            .broadcast_matmul(&self.fc1.t()?.contiguous()?)?
            .broadcast_add(&self.fc1_bias)?;
        let x = x.gelu_erf()?;
        let x = x
            .broadcast_matmul(&self.fc2.t()?.contiguous()?)?
            .broadcast_add(&self.fc2_bias)?;

        self.norm2.forward(&(h + x)?)
    }
}

// ============================================================================
// 完整 JinaBERT 文本编码器
// ============================================================================

pub struct JinaBertTextEncoder {
    word_embeddings: Tensor,       // [vocab, hidden]
    token_type_embeddings: Tensor, // [1, hidden]
    emb_ln: CudaLayerNorm,
    layers: Vec<JinaBertLayer>,
    tokenizer: tokenizers::Tokenizer,
    hidden_size: usize,
    vocab_size: usize,
    num_heads: usize,
    head_dim: usize,
    max_seq_len: usize,
    inv_freq: Tensor,
    device: Device,
}

impl JinaBertTextEncoder {
    /// 从本地目录加载文本编码器
    /// 目录需包含: config.json, tokenizer.json, text_model.safetensors
    pub fn load_from_dir(
        model_path: &str,
        device: &Device,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(model_path);

        // 加载 tokenizer.json
        let tokenizer_path = path.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| format!("加载 tokenizer 失败: {}", e))?;

        // 加载 text_model.safetensors
        let safetensors_path = path.join("text_model.safetensors");
        let tensors = candle_core::safetensors::load(safetensors_path, device)?;

        // 从权重推导配置
        let vocab_size = tensors
            .get("embeddings.word_embeddings.weight")
            .and_then(|t| t.dim(0).ok())
            .unwrap_or(250002);
        let hidden_size = tensors
            .get("embeddings.word_embeddings.weight")
            .and_then(|t| t.dim(1).ok())
            .unwrap_or(1024);
        let intermediate_size = tensors
            .get("encoder.layers.0.mlp.fc1.weight")
            .and_then(|t| t.dim(0).ok())
            .unwrap_or(4096);

        // 统计层数
        let num_layers = tensors
            .keys()
            .filter(|k| k.starts_with("encoder.layers.") && k.ends_with(".mixer.Wqkv.weight"))
            .count();
        if num_layers == 0 {
            return Err(format!("text_model.safetensors 中未找到 encoder.layers，权重文件可能不完整").into());
        }

        // 尝试从 config.json 读取 num_attention_heads
        let mut num_heads = hidden_size / 64;
        if let Ok(config_json) = std::fs::read_to_string(path.join("config.json")) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&config_json) {
                if let Some(n) = v.get("text_config").and_then(|t| t.get("num_attention_heads")).and_then(|x| x.as_u64()) {
                    num_heads = n as usize;
                }
            }
        }
        let head_dim = hidden_size / num_heads;

        let vb = VarBuilder::from_tensors(tensors, DType::F32, device);

        let word_embeddings = vb.get((vocab_size, hidden_size), "embeddings.word_embeddings.weight")?;
        let token_type_embeddings = vb.get((1, hidden_size), "embeddings.token_type_embeddings.weight")?;
        let emb_ln = CudaLayerNorm::load(hidden_size, 1e-5, vb.pp("emb_ln"))?;

        let mut layers = Vec::with_capacity(num_layers);
        for i in 0..num_layers {
            layers.push(JinaBertLayer::load(
                vb.pp("encoder.layers").pp(&i.to_string()),
                hidden_size,
                intermediate_size,
            )?);
        }

        let inv_freq = rope_inv_freq(head_dim, device)?;

        info!(
            "JinaBERT 文本编码器加载成功: hidden={}, layers={}, heads={}, head_dim={}, intermediate={}, vocab={}",
            hidden_size, num_layers, num_heads, head_dim, intermediate_size, vocab_size
        );

        Ok(Self {
            word_embeddings,
            token_type_embeddings,
            emb_ln,
            layers,
            tokenizer,
            hidden_size,
            vocab_size,
            num_heads,
            head_dim,
            max_seq_len: 512,
            inv_freq,
            device: device.clone(),
        })
    }

    pub fn hidden_size(&self) -> usize {
        self.hidden_size
    }

    /// 对单个文本生成 1024 维 L2 归一化向量
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenize 失败: {}", e))?;

        let raw_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask().to_vec();

        if raw_ids.is_empty() {
            return Ok(vec![0.0f32; self.hidden_size]);
        }

        // 截断到最大长度
        let max_seq = self.max_seq_len;
        let seq_len = raw_ids.len().min(max_seq);
        if raw_ids.len() > max_seq {
            warn!("输入长度 {} 超过 max_seq_len={}, 已截断", raw_ids.len(), max_seq);
        }
        let raw_ids = &raw_ids[..seq_len];

        // 裁剪 token ID 到 [0, vocab_size-1]
        let max_id = self.vocab_size as u32 - 1;
        let input_ids: Vec<u32> = raw_ids
            .iter()
            .map(|&id| if id > max_id { max_id } else { id })
            .collect();

        let seq_len = input_ids.len();
        let device = &self.device;

        // embedding 需要 1 维索引
        let input_ids_t = Tensor::from_slice(&input_ids, (seq_len,), device)?;
        let attention_mask_t = Tensor::from_slice(&attention_mask, (1, seq_len), device)?;

        // 词嵌入 + token_type 嵌入（type 全 0，取第 0 行）
        let word_emb = self.word_embeddings.embedding(&input_ids_t)?.unsqueeze(0)?; // [1, S, hidden]
        let type_ids = Tensor::zeros((seq_len,), DType::U32, device)?;
        let type_emb = self.token_type_embeddings.embedding(&type_ids)?.unsqueeze(0)?;
        let hidden = (word_emb + type_emb)?;
        let hidden = self.emb_ln.forward(&hidden)?;

        // 构造 attention mask: 1 -> 0 (保留), 0 -> -10000 (屏蔽)
        // mask 形状 [1, 1, 1, S]
        let ones = Tensor::new(1.0f32, device)?;
        let mask_f32 = attention_mask_t.to_dtype(DType::F32)?;
        let mask = ones
            .broadcast_sub(&mask_f32)?
            .broadcast_mul(&Tensor::new(-10000.0f32, device)?)?
            .unsqueeze(0)?
            .unsqueeze(0)?;

        let mut h = hidden;
        for layer in &self.layers {
            h = layer.forward(&h, &mask, self.num_heads, self.head_dim, &self.inv_freq, device)?;
        }

        // mean pooling（按 attention_mask 加权）
        let mask_t = attention_mask_t.unsqueeze(2)?.to_dtype(DType::F32)?; // [1, S, 1]
        let sum_hidden = h.broadcast_mul(&mask_t)?.sum(1)?; // [1, hidden]
        let num_tokens = mask_t.sum(1)?; // [1, 1]
        let eps_t = Tensor::new(1e-8f32, device)?;
        let pooled = sum_hidden.broadcast_div(&num_tokens.broadcast_add(&eps_t)?)?;

        // L2 归一化
        let norm = pooled.sqr()?.sum(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm.broadcast_add(&eps_t)?)?;

        Ok(normalized.squeeze(0)?.to_vec1()?)
    }
}

/// 尝试从指定路径加载 JinaBERT 文本编码器
/// 需要 config.json + tokenizer.json + text_model.safetensors
pub fn try_load_text_encoder(
    model_path: &str,
    device: &Device,
) -> Option<JinaBertTextEncoder> {
    let path = Path::new(model_path);
    if !path.exists() || !path.is_dir() {
        return None;
    }
    let has_config = path.join("config.json").exists();
    let has_tokenizer = path.join("tokenizer.json").exists();
    let has_text_weights = path.join("text_model.safetensors").exists();

    if !has_config || !has_tokenizer || !has_text_weights {
        info!(
            "模型 '{}' 缺少文本编码器文件 (config.json={}, tokenizer.json={}, text_model.safetensors={})，跳过文本编码器加载",
            model_path, has_config, has_tokenizer, has_text_weights
        );
        return None;
    }

    info!("检测到 text_model.safetensors，开始加载 JinaBERT 文本编码器...");
    match JinaBertTextEncoder::load_from_dir(model_path, device) {
        Ok(model) => {
            info!("JinaBERT 文本编码器加载成功: dim={}", model.hidden_size());
            Some(model)
        }
        Err(e) => {
            warn!("JinaBERT 文本编码器加载失败: {}", e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let na: f64 = a.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        let nb: f64 = b.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        dot / (na * nb)
    }

    #[test]
    fn test_postln_matches_numpy_reference() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("laoflch_db_model/candle/jina-clip-v2");
        if !model_path.join("text_model.safetensors").exists() {
            return; // 模型文件不存在，跳过
        }

        let device = Device::Cpu;
        let encoder = JinaBertTextEncoder::load_from_dir(
            model_path.to_str().unwrap(),
            &device,
        )
        .unwrap();

        // 读取 numpy 参考向量
        let ref_json = std::fs::read_to_string("/tmp/ref_vectors.json").unwrap();
        let refs: serde_json::Value = serde_json::from_str(&ref_json).unwrap();

        for (text, refv) in [("猫", "猫"), ("海边日落", "海边日落"), ("a cat sleeping on a sofa", "a cat sleeping on a sofa")] {
            let rust_vec = encoder.embed(text).unwrap();
            let ref_vec: Vec<f64> = refs[refv].as_array().unwrap().iter()
                .map(|v| v.as_f64().unwrap()).collect();
            let ref_vec_f32: Vec<f32> = ref_vec.iter().map(|v| *v as f32).collect();
            let c = cosine(&rust_vec, &ref_vec_f32);
            println!("'{}': cosine vs numpy reference = {:.6}", text, c);
            // 端到端浮点实现差异容忍 1e-3
            assert!(c > 0.999, "向量与 numpy 参考不一致: cos={}", c);
        }
    }
}
