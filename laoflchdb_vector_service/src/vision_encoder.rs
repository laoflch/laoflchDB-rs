//! 视觉编码器模块
//!
//! 支持两类视觉模型：
//! 1. 标准 ViT 风格（SigLIP2 等）：绝对位置编码、GELU MLP、CLS pooling
//! 2. EVA02 风格（jina-clip-v2 vision tower）：SwiGLU MLP、2D RoPE、
//!    subln（inner_attn_ln/ffn_ln）、mean pooling + fc_norm、CLIP 预处理
//!
//! 提供图片预处理、模型加载、向量生成能力。

use candle_core::{Device, DType, ModuleT, Tensor};
use candle_nn::{conv2d, linear, linear_no_bias, Conv2d, Conv2dConfig, Dropout, Module, VarBuilder};
use log::{info, warn};
use std::collections::HashMap;
use std::path::Path;

// ============================================================================
// 配置
// ============================================================================

/// 视觉模型配置，从 config.json 的 vision_config 反序列化
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VisionConfig {
    pub hidden_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub intermediate_size: usize,
    pub image_size: usize,
    pub patch_size: usize,
    #[serde(default = "default_layer_norm_eps")]
    pub layer_norm_eps: f64,
    #[serde(default = "default_dropout_prob")]
    pub hidden_dropout_prob: f64,
    #[serde(default = "default_dropout_prob")]
    pub attention_probs_dropout_prob: f64,
    #[serde(default)]
    pub model_type: Option<String>,
    /// 权重名称中需要去除的前缀，如 "vision_model."
    #[serde(default)]
    pub weight_prefix: Option<String>,
    /// EVA02 风格视觉塔（jina-clip-v2）专用字段
    /// RoPE 预训练序列长度（网格边长，如 16）
    #[serde(default = "default_pt_hw_seq_len")]
    pub pt_hw_seq_len: usize,
    #[serde(default)]
    pub rope_embeddings: bool,
    #[serde(default)]
    pub naive_swiglu: bool,
    #[serde(default)]
    pub subln: bool,
}

fn default_layer_norm_eps() -> f64 {
    1e-6
}
fn default_dropout_prob() -> f64 {
    0.0
}
fn default_pt_hw_seq_len() -> usize {
    16
}

impl VisionConfig {
    /// 从 config.json 中提取 vision 配置
    /// 支持从顶层或嵌套的 vision_config 字段读取
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        // 尝试从 vision_config 字段读取（Jina-CLIP-v2 风格的嵌套配置）
        if let Some(vision_cfg) = value.get("vision_config") {
            if let Ok(config) = serde_json::from_value::<VisionConfig>(vision_cfg.clone()) {
                return Some(config);
            }
        }

        // 尝试直接从顶层读取（标准 ViT 风格）
        if let Ok(config) = serde_json::from_value::<VisionConfig>(value.clone()) {
            return Some(config);
        }

        None
    }

    /// 获取有效的前缀
    pub fn weight_prefix(&self) -> &str {
        self.weight_prefix.as_deref().unwrap_or("")
    }

    /// 是否为 EVA02 风格视觉塔（jina-clip-v2）
    pub fn is_eva_style(&self) -> bool {
        let vtype = self.model_type.as_deref().unwrap_or("");
        self.naive_swiglu || self.rope_embeddings || self.subln || vtype == "jina_clip_vision"
    }

    /// 网格边长（image_size / patch_size）
    pub fn grid_size(&self) -> usize {
        self.image_size / self.patch_size
    }
}

// ============================================================================
// 图片处理器
// ============================================================================

/// ImageNet 归一化参数（标准 ViT 路径）
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];
/// CLIP 归一化参数（jina-clip-v2 视觉塔）
const CLIP_MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const CLIP_STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

/// 图片预处理：解码、缩放、归一化、转 Tensor
pub struct ImageProcessor {
    image_size: usize,
    mean: [f32; 3],
    std: [f32; 3],
    /// true 时使用最短边缩放 + 中心裁剪（对齐 CLIP 预处理），否则直接拉伸为方形
    clip_resize: bool,
}

impl ImageProcessor {
    /// 标准 ViT 路径：直接缩放为方形，ImageNet 归一化
    pub fn new(image_size: usize) -> Self {
        Self {
            image_size,
            mean: IMAGENET_MEAN,
            std: IMAGENET_STD,
            clip_resize: false,
        }
    }

    /// EVA02/CLIP 路径：最短边缩放 + 中心裁剪，CLIP 归一化
    pub fn new_clip(image_size: usize) -> Self {
        Self {
            image_size,
            mean: CLIP_MEAN,
            std: CLIP_STD,
            clip_resize: true,
        }
    }

    /// 将图片字节数据预处理为模型输入 Tensor
    /// 返回 shape: [1, 3, image_size, image_size]
    pub fn preprocess_bytes(
        &self,
        image_bytes: &[u8],
        device: &Device,
    ) -> std::result::Result<Tensor, String> {
        // 1. 解码图片
        let img = image::load_from_memory(image_bytes).map_err(|e| format!("图片解码失败: {}", e))?;
        let img = img.to_rgb8();

        // 2. 缩放
        let resized = if self.clip_resize {
            // 最短边缩放至 image_size（双三次），再中心裁剪
            let (w, h) = img.dimensions();
            let scale = self.image_size as f32 / (h.min(w)) as f32;
            let new_h = ((h as f32 * scale).round().max(self.image_size as f32)) as u32;
            let new_w = ((w as f32 * scale).round().max(self.image_size as f32)) as u32;
            let resized = image::imageops::resize(
                &img,
                new_w,
                new_h,
                image::imageops::FilterType::CatmullRom,
            );
            let left = (new_w - self.image_size as u32) / 2;
            let top = (new_h - self.image_size as u32) / 2;
            image::imageops::crop_imm(
                &resized,
                left,
                top,
                self.image_size as u32,
                self.image_size as u32,
            )
            .to_image()
        } else {
            image::imageops::resize(
                &img,
                self.image_size as u32,
                self.image_size as u32,
                image::imageops::FilterType::Lanczos3,
            )
        };

        // 3. 转换为 [H, W, C] float32 数据
        let (w, h) = resized.dimensions();
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for pixel in resized.pixels() {
            data.push(pixel[0] as f32 / 255.0);
            data.push(pixel[1] as f32 / 255.0);
            data.push(pixel[2] as f32 / 255.0);
        }

        let tensor = Tensor::from_slice(&data, (h as usize, w as usize, 3), device)
            .map_err(|e| format!("创建 Tensor 失败: {}", e))?;

        // 4. 转置为 [C, H, W]
        let tensor = tensor
            .permute((2, 0, 1))
            .map_err(|e| format!("转置 Tensor 失败: {}", e))?;

        // 5. 归一化: (x / 255.0 - mean) / std
        let mean = Tensor::new(&self.mean[..], device)
            .map_err(|e| format!("创建 mean Tensor 失败: {}", e))?
            .reshape((3, 1, 1))
            .map_err(|e| format!("reshape mean Tensor 失败: {}", e))?;
        let std = Tensor::new(&self.std[..], device)
            .map_err(|e| format!("创建 std Tensor 失败: {}", e))?
            .reshape((3, 1, 1))
            .map_err(|e| format!("reshape std Tensor 失败: {}", e))?;
        let normalized = tensor
            .broadcast_sub(&mean)
            .map_err(|e| format!("归一化减法失败: {}", e))?
            .broadcast_div(&std)
            .map_err(|e| format!("归一化除法失败: {}", e))?;

        // 6. 添加 batch 维度: [1, C, H, W]
        normalized
            .unsqueeze(0)
            .map_err(|e| format!("添加 batch 维度失败: {}", e))
    }
}

// ============================================================================
// 手动 LayerNorm（CUDA 兼容）
// ============================================================================

/// 与 lib.rs 中相同的自定义 LayerNorm，避免 CUDA kernel 依赖
pub struct CudaLayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
    size: usize,
}

impl CudaLayerNorm {
    pub fn load(size: usize, eps: f64, vb: VarBuilder) -> std::result::Result<Self, candle_core::Error> {
        let weight = vb.get_with_hints(size, "weight", candle_nn::Init::Const(1.0))?;
        let bias = vb.get_with_hints(size, "bias", candle_nn::Init::Const(0.0))?;
        Ok(Self { weight, bias, eps, size })
    }

    pub fn forward(&self, x: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
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
// 标准 ViT 风格实现（SigLIP2 等）
// ============================================================================

/// 将图片切分为 patches 并投影到 hidden_size 维度
struct VisionPatchEmbed {
    conv: Conv2d,
    num_patches: usize,
    hidden_size: usize,
}

impl VisionPatchEmbed {
    fn load(vb: VarBuilder, config: &VisionConfig) -> std::result::Result<Self, candle_core::Error> {
        let conv_cfg = Conv2dConfig {
            padding: 0,
            stride: config.patch_size,
            dilation: 1,
            groups: 1,
            ..Default::default()
        };
        let conv = candle_nn::conv2d_no_bias(
            3,                           // in_channels (RGB)
            config.hidden_size,          // out_channels
            config.patch_size,           // kernel_size
            conv_cfg,
            vb,                          // 直接使用传入的 vb，调用方已提供 patch_embed 前缀
        )?;
        let num_patches = (config.image_size / config.patch_size).pow(2);
        Ok(Self {
            conv,
            num_patches,
            hidden_size: config.hidden_size,
        })
    }

    fn forward(&self, pixel_values: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        // pixel_values: [B, 3, H, W]
        let x = self.conv.forward(pixel_values)?; // [B, hidden_size, H/patch, W/patch]
        let (b, _c, _h, _w) = x.dims4()?;
        x.reshape((b, self.hidden_size, self.num_patches))?
            .transpose(1, 2) // [B, num_patches, hidden_size]
    }
}

struct VisionSelfAttention {
    query: candle_nn::Linear,
    key: candle_nn::Linear,
    value: candle_nn::Linear,
    output: candle_nn::Linear,
    dropout: Dropout,
    num_attention_heads: usize,
    attention_head_size: usize,
    hidden_size: usize,
}

impl VisionSelfAttention {
    fn load(vb: VarBuilder, config: &VisionConfig) -> std::result::Result<Self, candle_core::Error> {
        let hidden_size = config.hidden_size;
        let num_heads = config.num_attention_heads;
        let head_size = hidden_size / num_heads;

        let query = linear(hidden_size, hidden_size, vb.pp("query"))?;
        let key = linear(hidden_size, hidden_size, vb.pp("key"))?;
        let value = linear(hidden_size, hidden_size, vb.pp("value"))?;
        let output = linear(hidden_size, hidden_size, vb.pp("output"))?;
        let dropout = Dropout::new(config.attention_probs_dropout_prob as f32);

        Ok(Self {
            query,
            key,
            value,
            output,
            dropout,
            num_attention_heads: num_heads,
            attention_head_size: head_size,
            hidden_size,
        })
    }

    fn transpose_for_scores(&self, xs: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let (b_sz, seq_len, _hidden) = xs.dims3()?;
        let xs = xs.reshape((b_sz, seq_len, self.num_attention_heads, self.attention_head_size))?;
        xs.permute((0, 2, 1, 3))?.contiguous()
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let query = self.query.forward(hidden_states)?;
        let key = self.key.forward(hidden_states)?;
        let value = self.value.forward(hidden_states)?;

        let query = self.transpose_for_scores(&query)?;
        let key = self.transpose_for_scores(&key)?;
        let value = self.transpose_for_scores(&value)?;

        let scale = 1.0f32 / (self.attention_head_size as f32).sqrt();
        let attention_scores = query.matmul(&key.t()?.contiguous()?)?
            .broadcast_mul(&Tensor::new(scale, query.device())?)?;
        let attention_scores = attention_scores.broadcast_add(attention_mask)?;

        // Softmax 在 CPU 上执行（CUDA 兼容性）
        let orig_device = attention_scores.device().clone();
        let attention_scores_cpu = attention_scores.to_device(&Device::Cpu)?;
        let attention_probs = candle_nn::ops::softmax_last_dim(&attention_scores_cpu)?;
        let attention_probs = attention_probs.to_device(&orig_device)?;
        let attention_probs = self.dropout.forward_t(&attention_probs, false)?;

        let context = attention_probs.matmul(&value)?;
        let context = context.permute((0, 2, 1, 3))?.contiguous()?;
        let (b_sz, seq_len, _heads, _head_size) = context.dims4()?;
        let context = context.reshape((b_sz, seq_len, self.hidden_size))?;

        // Output projection
        self.output.forward(&context)
    }
}

struct VisionMlp {
    fc1: candle_nn::Linear,
    fc2: candle_nn::Linear,
    dropout: Dropout,
}

impl VisionMlp {
    fn load(vb: VarBuilder, config: &VisionConfig) -> std::result::Result<Self, candle_core::Error> {
        let fc1 = linear(config.hidden_size, config.intermediate_size, vb.pp("fc1"))?;
        let fc2 = linear(config.intermediate_size, config.hidden_size, vb.pp("fc2"))?;
        let dropout = Dropout::new(config.hidden_dropout_prob as f32);
        Ok(Self { fc1, fc2, dropout })
    }

    fn forward(&self, hidden_states: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let x = self.fc1.forward(hidden_states)?;
        let x = x.gelu_erf()?;
        let x = self.fc2.forward(&x)?;
        self.dropout.forward_t(&x, false)
    }
}

struct VisionLayer {
    attention_ln: CudaLayerNorm,
    attention: VisionSelfAttention,
    mlp_ln: CudaLayerNorm,
    mlp: VisionMlp,
}

impl VisionLayer {
    fn load(vb: VarBuilder, config: &VisionConfig, _layer_idx: usize) -> std::result::Result<Self, candle_core::Error> {
        let attention_ln = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("attention_ln"),
        )?;
        let attention = VisionSelfAttention::load(vb.pp("attention"), config)?;
        let mlp_ln = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("mlp_ln"),
        )?;
        let mlp = VisionMlp::load(vb.pp("mlp"), config)?;
        Ok(Self {
            attention_ln,
            attention,
            mlp_ln,
            mlp,
        })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        // Pre-LN: LayerNorm -> Attention -> Residual
        let ln1 = self.attention_ln.forward(hidden_states)?;
        let attn_out = self.attention.forward(&ln1, attention_mask)?;
        let x = (hidden_states + attn_out)?;

        // Pre-LN: LayerNorm -> MLP -> Residual
        let ln2 = self.mlp_ln.forward(&x)?;
        let mlp_out = self.mlp.forward(&ln2)?;
        x + mlp_out
    }
}

struct VisionEncoder {
    layers: Vec<VisionLayer>,
    post_ln: Option<CudaLayerNorm>,
}

impl VisionEncoder {
    fn load(vb: VarBuilder, config: &VisionConfig) -> std::result::Result<Self, candle_core::Error> {
        let mut layers = Vec::with_capacity(config.num_hidden_layers);
        for i in 0..config.num_hidden_layers {
            layers.push(VisionLayer::load(vb.pp("layer").pp(&i.to_string()), config, i)?);
        }
        // 有些模型有 post_layernorm
        let post_ln = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("post_ln"),
        ).ok();
        Ok(Self { layers, post_ln })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let mut h = hidden_states.clone();
        for layer in &self.layers {
            h = layer.forward(&h, attention_mask)?;
        }
        if let Some(ref post_ln) = self.post_ln {
            h = post_ln.forward(&h)?;
        }
        Ok(h)
    }
}

/// 完整的标准 ViT 视觉模型
pub struct VisionTransformer {
    patch_embed: VisionPatchEmbed,
    pos_embed: Tensor,
    cls_token: Tensor,
    pre_ln: Option<CudaLayerNorm>,
    encoder: VisionEncoder,
    config: VisionConfig,
    image_processor: ImageProcessor,
    device: Device,
}

impl VisionTransformer {
    /// 从本地目录加载视觉模型
    /// 目录需包含: config.json, model.safetensors
    pub fn load_from_dir(model_path: &str, device: &Device) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(model_path);

        // 加载 config.json
        let config_path = path.join("config.json");
        let config_json = std::fs::read_to_string(&config_path)?;
        let config_value: serde_json::Value = serde_json::from_str(&config_json)?;

        let config = VisionConfig::from_json(&config_value)
            .ok_or_else(|| format!("无法从 config.json 中读取 vision 配置"))?;

        // 加载 model.safetensors
        let safetensors_path = path.join("model.safetensors");
        let tensors = candle_core::safetensors::load(safetensors_path, device)?;

        // 处理权重名称前缀
        let prefix = config.weight_prefix();
        let mapped_tensors = if prefix.is_empty() {
            tensors
        } else {
            let mut mapped = HashMap::new();
            for (key, tensor) in tensors.iter() {
                if let Some(stripped) = key.strip_prefix(prefix) {
                    mapped.insert(stripped.to_string(), tensor.clone());
                } else {
                    mapped.insert(key.clone(), tensor.clone());
                }
            }
            mapped
        };

        // 应用权重名称映射（处理不同模型间的命名差异）
        let model_type_str = config.model_type.as_deref().unwrap_or("");
        let mapped_tensors = map_vision_weights(mapped_tensors, model_type_str);

        let vb = VarBuilder::from_tensors(mapped_tensors, DType::F32, device);

        // 构建 Vision Transformer
        let patch_embed = VisionPatchEmbed::load(vb.pp("patch_embed"), &config)?;
        let num_patches = (config.image_size / config.patch_size).pow(2);

        // 位置编码: [1, num_patches + 1, hidden_size]
        let pos_embed = vb.get((1, num_patches + 1, config.hidden_size), "pos_embed")?;

        // CLS token: [1, 1, hidden_size]
        let cls_token = vb.get((1, 1, config.hidden_size), "cls_token")?;

        // Pre-LayerNorm (可选)
        let pre_ln = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("pre_ln"),
        ).ok();

        let encoder = VisionEncoder::load(vb.pp("encoder"), &config)?;

        let image_processor = ImageProcessor::new(config.image_size);

        info!(
            "Vision 模型加载成功: hidden_size={}, layers={}, heads={}, patch_size={}, image_size={}",
            config.hidden_size,
            config.num_hidden_layers,
            config.num_attention_heads,
            config.patch_size,
            config.image_size,
        );

        Ok(Self {
            patch_embed,
            pos_embed,
            cls_token,
            pre_ln,
            encoder,
            config,
            image_processor,
            device: device.clone(),
        })
    }

    /// 对单张图片生成向量
    pub fn embed_image(&self, image_bytes: &[u8]) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let device = &self.device;

        // 1. 预处理图片
        let pixel_values = self.image_processor.preprocess_bytes(image_bytes, device)
            .map_err(|e| format!("图片预处理失败: {}", e))?;

        // 2. Patch embedding
        let patch_embeds = self.patch_embed.forward(&pixel_values)?; // [1, num_patches, hidden]
        let (b, _n, _h) = patch_embeds.dims3()?;

        // 3. 添加 CLS token
        let cls_token = self.cls_token.expand((b, 1, self.config.hidden_size))?;
        let hidden_states = Tensor::cat(&[&cls_token, &patch_embeds], 1)?; // [1, 1+num_patches, hidden]

        // 4. 添加位置编码（pos_embed 已是 [1, 1+num_patches, hidden] 形状）
        let hidden_states = (hidden_states + self.pos_embed.clone())?;

        // 5. Pre-LayerNorm (可选)
        let hidden_states = if let Some(ref pre_ln) = self.pre_ln {
            pre_ln.forward(&hidden_states)?
        } else {
            hidden_states
        };

        // 6. 创建 attention mask（全1，无 padding）
        let seq_len = hidden_states.dim(1)?;
        let attention_mask = Tensor::zeros((1, 1, 1, seq_len), DType::F32, device)?; // 全0 = 不 mask

        // 7. Transformer encoder
        let encoder_out = self.encoder.forward(&hidden_states, &attention_mask)?; // [1, 1+num_patches, hidden]

        // 8. CLS pooling: 取第一个 token (CLS)
        let cls_output = encoder_out.narrow(1, 0, 1)?; // [1, 1, hidden]
        let pooled = cls_output.squeeze(1)?; // [1, hidden]

        // 9. L2 normalize
        let eps_t = Tensor::new(1e-8f32, device)?;
        let norm = pooled.sqr()?.sum(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm.broadcast_add(&eps_t)?)?;

        // 10. 转为 Vec<f32>
        let result: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;
        Ok(result)
    }

    pub fn config(&self) -> &VisionConfig {
        &self.config
    }
}

// ============================================================================
// EVA02 风格视觉塔（jina-clip-v2 vision_model）
// ============================================================================

/// 2D 旋转位置编码频率（对齐 EVA-CLIP VisionRotaryEmbeddingFast）
/// 返回: (cos, sin)，形状均为 [grid*grid, head_dim]
fn vision_rope_freqs(
    grid: usize,
    pt_seq_len: usize,
    head_dim: usize,
    device: &Device,
) -> std::result::Result<(Tensor, Tensor), candle_core::Error> {
    let half = head_dim / 2; // 每个半维的维度数
    let num_freqs = head_dim / 4; // 频率数量（对齐 python dim=half_head_dim=32 时 arange(0,32,2) 为 16 个）
    let num_tokens = grid * grid;

    // 基础频率: freqs[j] = theta^(-(2j)/32)（python: 1/(theta**(arange(0,32,2)/32))）
    // 其中 32 = head_dim/2，故指数 = (2j)/(head_dim/2)
    let theta = 10000.0f64;
    let mut cos_data = vec![0.0f32; num_tokens * head_dim];
    let mut sin_data = vec![0.0f32; num_tokens * head_dim];

    for idx in 0..num_tokens {
        let row = (idx / grid) as f32;
        let col = (idx % grid) as f32;
        // t = arange(ft_seq_len)/ft_seq_len*pt_seq_len，ft_seq_len=grid
        let t_row = row * pt_seq_len as f32 / grid as f32;
        let t_col = col * pt_seq_len as f32 / grid as f32;

        for j in 0..num_freqs {
            let base = (theta.powf(-2.0 * j as f64 / (head_dim as f64 / 2.0))) as f32;
            let phase_r = t_row * base;
            let phase_c = t_col * base;
            // 前半维（row 部分）与后半维（col 部分），每对 dim 重复同频
            cos_data[idx * head_dim + 2 * j] = phase_r.cos();
            cos_data[idx * head_dim + 2 * j + 1] = phase_r.cos();
            sin_data[idx * head_dim + 2 * j] = phase_r.sin();
            sin_data[idx * head_dim + 2 * j + 1] = phase_r.sin();
            cos_data[idx * head_dim + half + 2 * j] = phase_c.cos();
            cos_data[idx * head_dim + half + 2 * j + 1] = phase_c.cos();
            sin_data[idx * head_dim + half + 2 * j] = phase_c.sin();
            sin_data[idx * head_dim + half + 2 * j + 1] = phase_c.sin();
        }
    }

    let cos = Tensor::from_vec(cos_data, (num_tokens, head_dim), device)?;
    let sin = Tensor::from_vec(sin_data, (num_tokens, head_dim), device)?;
    Ok((cos, sin))
}

/// 应用 2D RoPE（交错成对旋转）
/// x: [B, H, S, D]，cos/sin: [S, D]
fn apply_vision_rope(
    x: &Tensor,
    cos: &Tensor,
    sin: &Tensor,
) -> std::result::Result<Tensor, candle_core::Error> {
    let (b, hh, s, d) = x.dims4()?;
    let half = d / 2;

    // rotate_half: (x0,x1) -> (-x1,x0)，交错成对
    let xr = x.reshape((b, hh, s, half, 2))?;
    let x1 = xr.narrow(4, 0, 1)?;
    let x2 = xr.narrow(4, 1, 1)?;
    let rotated = Tensor::cat(&[&x2.neg()?, &x1], 4)?.reshape((b, hh, s, d))?;

    x.broadcast_mul(cos)? + rotated.broadcast_mul(sin)?
}

/// EVA02 自注意力（subln 风格：q/k/v 分离 + q_bias/v_bias + inner_attn_ln + RoPE）
struct EvaVisionAttention {
    q_proj: candle_nn::Linear, // 无 bias（q_bias 独立）
    k_proj: candle_nn::Linear, // 无 bias
    v_proj: candle_nn::Linear, // 无 bias（v_bias 独立）
    q_bias: Tensor,
    v_bias: Tensor,
    proj: candle_nn::Linear, // 有 bias
    inner_attn_ln: CudaLayerNorm,
    num_heads: usize,
    head_dim: usize,
    hidden_size: usize,
    rope_cos: Tensor,
    rope_sin: Tensor,
}

impl EvaVisionAttention {
    fn load(
        vb: VarBuilder,
        config: &VisionConfig,
        grid: usize,
        device: &Device,
    ) -> std::result::Result<Self, candle_core::Error> {
        let hidden = config.hidden_size;
        let heads = config.num_attention_heads;
        let head_dim = hidden / heads;

        let q_proj = linear_no_bias(hidden, hidden, vb.pp("q_proj"))?;
        let k_proj = linear_no_bias(hidden, hidden, vb.pp("k_proj"))?;
        let v_proj = linear_no_bias(hidden, hidden, vb.pp("v_proj"))?;
        let q_bias = vb.get(hidden, "q_bias")?;
        let v_bias = vb.get(hidden, "v_bias")?;
        let proj = linear(hidden, hidden, vb.pp("proj"))?;
        let inner_attn_ln = CudaLayerNorm::load(hidden, config.layer_norm_eps, vb.pp("inner_attn_ln"))?;

        let (rope_cos, rope_sin) = vision_rope_freqs(grid, config.pt_hw_seq_len, head_dim, device)?;

        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            q_bias,
            v_bias,
            proj,
            inner_attn_ln,
            num_heads: heads,
            head_dim,
            hidden_size: hidden,
            rope_cos,
            rope_sin,
        })
    }

    fn forward(&self, x: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let (b, s, _) = x.dims3()?;

        let q = self.q_proj.forward(x)?.broadcast_add(&self.q_bias)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?.broadcast_add(&self.v_bias)?;

        let q = q.reshape((b, s, self.num_heads, self.head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;
        let k = k.reshape((b, s, self.num_heads, self.head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;
        let v = v.reshape((b, s, self.num_heads, self.head_dim))?.permute((0, 2, 1, 3))?.contiguous()?;

        // RoPE 仅作用于非 CLS token（对齐官方：q_t = q[:, :, 1:, :]）
        let q_t = apply_vision_rope(&q.narrow(2, 1, s - 1)?, &self.rope_cos, &self.rope_sin)?;
        let k_t = apply_vision_rope(&k.narrow(2, 1, s - 1)?, &self.rope_cos, &self.rope_sin)?;
        let q = Tensor::cat(&[&q.narrow(2, 0, 1)?, &q_t], 2)?.contiguous()?;
        let k = Tensor::cat(&[&k.narrow(2, 0, 1)?, &k_t], 2)?.contiguous()?;

        let scale = 1.0f32 / (self.head_dim as f32).sqrt();
        let scores = q.matmul(&k.t()?.contiguous()?)?;
        let scores = scores.broadcast_mul(&Tensor::new(scale, x.device())?)?;

        // Softmax 在 CPU 上执行（CUDA 兼容）
        let orig_device = scores.device().clone();
        let scores_cpu = scores.to_device(&Device::Cpu)?;
        let probs = candle_nn::ops::softmax_last_dim(&scores_cpu)?;
        let probs = probs.to_device(&orig_device)?;

        let ctx = probs.matmul(&v)?; // [B, H, S, D]
        let ctx = ctx.permute((0, 2, 1, 3))?.contiguous()?;
        let ctx = ctx.reshape((b, s, self.hidden_size))?;

        // inner_attn_ln 在投影之前
        let ctx = self.inner_attn_ln.forward(&ctx)?;
        self.proj.forward(&ctx)
    }
}

/// EVA02 MLP（naive SwiGLU + ffn_ln）
struct EvaVisionMlp {
    w1: candle_nn::Linear,
    w2: candle_nn::Linear,
    w3: candle_nn::Linear,
    ffn_ln: CudaLayerNorm,
}

impl EvaVisionMlp {
    fn load(vb: VarBuilder, config: &VisionConfig) -> std::result::Result<Self, candle_core::Error> {
        let hidden = config.hidden_size;
        let intermediate = config.intermediate_size;

        let w1 = linear(hidden, intermediate, vb.pp("w1"))?;
        let w2 = linear(hidden, intermediate, vb.pp("w2"))?;
        let w3 = linear(intermediate, hidden, vb.pp("w3"))?;
        let ffn_ln = CudaLayerNorm::load(intermediate, config.layer_norm_eps, vb.pp("ffn_ln"))?;

        Ok(Self { w1, w2, w3, ffn_ln })
    }

    fn forward(&self, x: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        let x1 = self.w1.forward(x)?;
        let x2 = self.w2.forward(x)?;
        let hidden = (x1.silu()? * x2)?;
        let hidden = self.ffn_ln.forward(&hidden)?;
        self.w3.forward(&hidden)
    }
}

/// EVA02 Transformer Block（Pre-LN，post_norm=false）
struct EvaVisionBlock {
    norm1: CudaLayerNorm,
    attn: EvaVisionAttention,
    norm2: CudaLayerNorm,
    mlp: EvaVisionMlp,
}

impl EvaVisionBlock {
    fn load(
        vb: VarBuilder,
        config: &VisionConfig,
        grid: usize,
        device: &Device,
    ) -> std::result::Result<Self, candle_core::Error> {
        let norm1 = CudaLayerNorm::load(config.hidden_size, config.layer_norm_eps, vb.pp("norm1"))?;
        let attn = EvaVisionAttention::load(vb.pp("attn"), config, grid, device)?;
        let norm2 = CudaLayerNorm::load(config.hidden_size, config.layer_norm_eps, vb.pp("norm2"))?;
        let mlp = EvaVisionMlp::load(vb.pp("mlp"), config)?;

        Ok(Self { norm1, attn, norm2, mlp })
    }

    fn forward(&self, x: &Tensor) -> std::result::Result<Tensor, candle_core::Error> {
        // Pre-LN: x = x + attn(norm1(x))
        let n1 = self.norm1.forward(x)?;
        let attn_out = self.attn.forward(&n1)?;
        let x = (x + attn_out)?;

        // Pre-LN: x = x + mlp(norm2(x))
        let n2 = self.norm2.forward(&x)?;
        let mlp_out = self.mlp.forward(&n2)?;
        x + mlp_out
    }
}

/// EVA02 完整视觉模型（jina-clip-v2 vision tower）
pub struct EvaVisionTransformer {
    patch_embed: Conv2d,
    pos_embed: Tensor,
    cls_token: Tensor,
    blocks: Vec<EvaVisionBlock>,
    fc_norm: CudaLayerNorm,
    config: VisionConfig,
    image_processor: ImageProcessor,
    device: Device,
}

impl EvaVisionTransformer {
    /// 从本地目录加载视觉塔
    /// 目录需包含: config.json, vision_model.safetensors（或 model.safetensors）
    pub fn load_from_dir(
        model_path: &str,
        device: &Device,
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(model_path);

        // 加载 config.json
        let config_path = path.join("config.json");
        let config_json = std::fs::read_to_string(&config_path)?;
        let config_value: serde_json::Value = serde_json::from_str(&config_json)?;

        let config = VisionConfig::from_json(&config_value)
            .ok_or_else(|| format!("无法从 config.json 中读取 vision 配置"))?;

        // 优先加载 vision_model.safetensors（提取的视觉塔），回退 model.safetensors
        let vision_weights = path.join("vision_model.safetensors");
        let safetensors_path = if vision_weights.exists() {
            vision_weights
        } else {
            path.join("model.safetensors")
        };
        let tensors = candle_core::safetensors::load(safetensors_path, device)?;

        // 去除 "vision_model." 前缀（若存在）
        let mut mapped_tensors = HashMap::new();
        for (key, tensor) in tensors.iter() {
            let k = if let Some(stripped) = key.strip_prefix("vision_model.") {
                stripped.to_string()
            } else {
                key.clone()
            };
            mapped_tensors.insert(k, tensor.clone());
        }

        let vb = VarBuilder::from_tensors(mapped_tensors, DType::F32, device);

        let grid = config.grid_size();
        let num_patches = grid * grid;
        let hidden = config.hidden_size;

        // patch_embed.proj: Conv2d(3, hidden, kernel=patch, stride=patch, 有 bias)
        let conv_cfg = Conv2dConfig {
            padding: 0,
            stride: config.patch_size,
            dilation: 1,
            groups: 1,
            ..Default::default()
        };
        let patch_embed = conv2d(3, hidden, config.patch_size, conv_cfg, vb.pp("patch_embed.proj"))?;

        let pos_embed = vb.get((1, num_patches + 1, hidden), "pos_embed")?;
        let cls_token = vb.get((1, 1, hidden), "cls_token")?;

        let mut blocks = Vec::with_capacity(config.num_hidden_layers);
        for i in 0..config.num_hidden_layers {
            blocks.push(EvaVisionBlock::load(vb.pp("blocks").pp(&i.to_string()), &config, grid, device)?);
        }

        // fc_norm（norm.weight/norm.bias）
        let fc_norm = CudaLayerNorm::load(hidden, config.layer_norm_eps, vb.pp("norm"))?;

        let image_processor = ImageProcessor::new_clip(config.image_size);

        info!(
            "EVA02 视觉塔加载成功: hidden_size={}, layers={}, heads={}, patch_size={}, image_size={}, grid={}",
            config.hidden_size,
            config.num_hidden_layers,
            config.num_attention_heads,
            config.patch_size,
            config.image_size,
            grid,
        );

        Ok(Self {
            patch_embed,
            pos_embed,
            cls_token,
            blocks,
            fc_norm,
            config,
            image_processor,
            device: device.clone(),
        })
    }

    /// 对单张图片生成向量（mean pooling + fc_norm + L2 归一化）
    pub fn embed_image(
        &self,
        image_bytes: &[u8],
    ) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        let device = &self.device;

        // 1. 预处理（CLIP: 最短边缩放 + 中心裁剪 + CLIP mean/std 归一化）
        let pixel_values = self
            .image_processor
            .preprocess_bytes(image_bytes, device)
            .map_err(|e| format!("图片预处理失败: {}", e))?;

        // 2. Patch embedding: [1, 3, H, W] -> [1, hidden, grid, grid]
        let x = self.patch_embed.forward(&pixel_values)?;
        let (b, _c, h, w) = x.dims4()?;
        let num_patches = h * w;
        let x = x.reshape((b, self.config.hidden_size, num_patches))?.transpose(1, 2)?;

        // 3. 拼接 CLS token + 位置编码
        let cls = self.cls_token.expand((b, 1, self.config.hidden_size))?;
        let mut hidden_states = Tensor::cat(&[&cls, &x], 1)?; // [1, 1+num_patches, hidden]
        hidden_states = (hidden_states + self.pos_embed.clone())?;

        // 4. 24 层 Transformer Block（Pre-LN）
        for block in &self.blocks {
            hidden_states = block.forward(&hidden_states)?;
        }

        // 5. mean pooling（含 CLS token）+ fc_norm
        let pooled = hidden_states.mean(1)?; // [1, hidden]
        let pooled = self.fc_norm.forward(&pooled)?;

        // 6. L2 归一化（与文本编码器输出对齐）
        let eps_t = Tensor::new(1e-8f32, device)?;
        let norm = pooled.sqr()?.sum(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm.broadcast_add(&eps_t)?)?;

        Ok(normalized.squeeze(0)?.to_vec1()?)
    }

    pub fn config(&self) -> &VisionConfig {
        &self.config
    }
}

// ============================================================================
// 统一视觉模型类型
// ============================================================================

/// 视觉模型统一封装
pub enum VisionModel {
    /// 标准 ViT 风格（SigLIP2 等）
    Standard(VisionTransformer),
    /// EVA02 风格（jina-clip-v2 vision tower）
    Eva(EvaVisionTransformer),
}

impl VisionModel {
    pub fn embed_image(
        &self,
        image_bytes: &[u8],
    ) -> std::result::Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            VisionModel::Standard(m) => m.embed_image(image_bytes),
            VisionModel::Eva(m) => m.embed_image(image_bytes),
        }
    }

    pub fn config(&self) -> &VisionConfig {
        match self {
            VisionModel::Standard(m) => m.config(),
            VisionModel::Eva(m) => m.config(),
        }
    }
}

// ============================================================================
// 模型加载辅助函数
// ============================================================================

/// 尝试从指定路径加载视觉模型
/// 检查 config.json + model.safetensors 是否存在，且 model_type 为视觉模型
pub fn try_load_vision_model(model_path: &str, device: &Device) -> Option<VisionModel> {
    let path = Path::new(model_path);

    if !path.exists() || !path.is_dir() {
        return None;
    }

    let config_path = path.join("config.json");
    if !config_path.exists() {
        return None;
    }

    // 检查 model_type 是否为视觉模型
    let config_json = std::fs::read_to_string(&config_path).ok()?;
    let config_value: serde_json::Value = serde_json::from_str(&config_json).ok()?;

    let model_type = config_value
        .get("model_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let is_vision_model = matches!(
        model_type,
        "jina-clip-v2" | "jina_clip" | "siglip" | "siglip2" | "vit" | "clip" | "vision"
    );

    // 如果没有明确的 model_type，尝试通过 vision_config 字段判断
    let has_vision_config = config_value.get("vision_config").is_some();

    // 检查是否有视觉模型的关键字段
    let has_image_size = config_value.get("image_size").is_some()
        || config_value
            .get("vision_config")
            .and_then(|v| v.get("image_size"))
            .is_some();
    let has_patch_size = config_value.get("patch_size").is_some()
        || config_value
            .get("vision_config")
            .and_then(|v| v.get("patch_size"))
            .is_some();

    if !is_vision_model && !has_vision_config && !(has_image_size && has_patch_size) {
        return None;
    }

    info!("检测到视觉模型 (model_type={}), 开始加载...", model_type);

    // 根据架构类型分发：EVA02 风格（jina-clip-v2）优先
    let vision_cfg = VisionConfig::from_json(&config_value);
    let is_eva = vision_cfg.as_ref().map(|c| c.is_eva_style()).unwrap_or(false);

    if is_eva {
        match EvaVisionTransformer::load_from_dir(model_path, device) {
            Ok(model) => {
                info!("EVA02 视觉模型加载成功: dim={}", model.config().hidden_size);
                Some(VisionModel::Eva(model))
            }
            Err(e) => {
                warn!("EVA02 视觉模型加载失败: {}", e);
                None
            }
        }
    } else {
        let weights_path = path.join("model.safetensors");
        if !weights_path.exists() {
            return None;
        }
        match VisionTransformer::load_from_dir(model_path, device) {
            Ok(model) => {
                info!("Vision 模型加载成功: dim={}", model.config().hidden_size);
                Some(VisionModel::Standard(model))
            }
            Err(e) => {
                warn!("Vision 模型加载失败: {}", e);
                None
            }
        }
    }
}

// ============================================================================
// 权重名称映射工具
// ============================================================================

/// 将模型特定的视觉权重名称映射到内部统一命名
/// 处理不同模型间命名差异
pub fn map_vision_weights(
    mut tensors: HashMap<String, Tensor>,
    model_type: &str,
) -> HashMap<String, Tensor> {
    let mut mapped = HashMap::new();

    for (key, tensor) in tensors.drain() {
        let mut new_key = key;

        // 处理 Jina-CLIP-v2 类型的命名差异
        if model_type == "jina-clip-v2" || model_type == "jina_clip_vision" {
            new_key = new_key
                .replace("self_attn.q_proj", "attention.query")
                .replace("self_attn.k_proj", "attention.key")
                .replace("self_attn.v_proj", "attention.value")
                .replace("self_attn.out_proj", "attention.output")
                .replace("layer_norm1", "attention_ln")
                .replace("layer_norm2", "mlp_ln")
                .replace("pre_layernorm", "pre_ln")
                .replace("post_layernorm", "post_ln")
                .replace("class_embedding", "cls_token")
                .replace("position_embedding", "pos_embed");
        }

        // 处理 SigLIP2 类型的命名差异
        if model_type == "siglip" || model_type == "siglip_vision_model" {
            new_key = new_key
                .replace("self_attn.q_proj", "attention.query")
                .replace("self_attn.k_proj", "attention.key")
                .replace("self_attn.v_proj", "attention.value")
                .replace("self_attn.out_proj", "attention.output")
                .replace("layer_norm1", "attention_ln")
                .replace("layer_norm2", "mlp_ln")
                .replace("pre_layernorm", "pre_ln")
                .replace("post_layernorm", "post_ln")
                .replace("class_embedding", "cls_token")
                .replace("position_embedding", "pos_embed");
        }

        // 通用映射：处理 patch_embed.conv 到 patch_embed 的映射
        // candle-nn 的 conv2d_no_bias 直接在 vb 下查找 "weight"，不需要 "conv" 子前缀
        if new_key.starts_with("patch_embed.conv.") {
            new_key = new_key.replacen("patch_embed.conv.", "patch_embed.", 1);
        }

        // 跳过不需要的 bias 权重（conv2d_no_bias 不使用 bias）
        if new_key == "patch_embed.bias" {
            continue;
        }

        mapped.insert(new_key, tensor);
    }

    mapped
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
        let na: f64 = a.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        let nb: f64 = b.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
        dot / (na * nb)
    }

    /// 生成与 Python 参考脚本完全一致的测试图（512x512 整数公式）
    fn make_test_image(name: &str) -> image::RgbImage {
        image::RgbImage::from_fn(512, 512, |x, y| {
            let (r, g, b): (u32, u32, u32) = match name {
                "g1" => (
                    30 + (200 * x) / 512,
                    220 - (150 * y) / 512,
                    80 + (120 * (x + y)) / 1024,
                ),
                "g2" => (
                    200 - (140 * x) / 512,
                    40 + (160 * y) / 512,
                    190 - (90 * (x + y)) / 1024,
                ),
                "g3" => (
                    (110 * x) / 512 + 60,
                    (90 * y) / 512 + 90,
                    (150 * (x + y)) / 1024 + 30,
                ),
                _ => (0, 0, 0),
            };
            image::Rgb([r as u8, g as u8, b as u8])
        })
    }

    #[test]
    fn test_eva_vision_matches_python_reference() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("laoflch_db_model/candle/jina-clip-v2");
        if !model_path.join("vision_model.safetensors").exists() {
            return; // 模型文件不存在，跳过
        }

        let device = Device::Cpu;
        let model = EvaVisionTransformer::load_from_dir(model_path.to_str().unwrap(), &device).unwrap();

        let ref_json = std::fs::read_to_string("/tmp/vision_ref.json").unwrap();
        let refs: serde_json::Value = serde_json::from_str(&ref_json).unwrap();

        for name in ["g1", "g2", "g3"] {
            let img = make_test_image(name);
            let mut bytes: Vec<u8> = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();

            let rust_vec = model.embed_image(&bytes).unwrap();
            let ref_vec: Vec<f64> = refs[name]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            let ref_vec_f32: Vec<f32> = ref_vec.iter().map(|v| *v as f32).collect();
            let c = cosine(&rust_vec, &ref_vec_f32);
            println!("vision '{}': cosine vs python reference = {:.6}", name, c);
            assert!(c > 0.99, "vision 向量与 python 参考不一致: cos={}", c);
        }
    }
}
