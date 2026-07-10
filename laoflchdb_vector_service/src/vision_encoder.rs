//! 视觉编码器模块
//!
//! 支持 ViT（Vision Transformer）架构的视觉模型，包括：
//! - Jina-CLIP-v2 (SigLIP-style ViT, patch_size=14, dim=1024)
//! - SigLIP2 (ViT, patch_size=16, dim=768)
//!
//! 提供图片预处理、模型加载、向量生成能力。

use candle_core::{Device, Tensor, DType, ModuleT};
use candle_nn::{VarBuilder, Dropout, Module, Conv2d, Conv2dConfig, linear};
use log::{info, warn};
use std::collections::HashMap;
use std::path::Path;

// ============================================================================
// 配置
// ============================================================================

/// 视觉模型配置，从 config.json 反序列化
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
}

fn default_layer_norm_eps() -> f64 { 1e-6 }
fn default_dropout_prob() -> f64 { 0.0 }

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
}

// ============================================================================
// 图片处理器
// ============================================================================

/// 图片预处理：解码、缩放、归一化、转 Tensor
pub struct ImageProcessor {
    image_size: usize,
    mean: [f32; 3],
    std: [f32; 3],
}

impl ImageProcessor {
    pub fn new(image_size: usize) -> Self {
        Self {
            image_size,
            mean: [0.485, 0.456, 0.406],
            std: [0.229, 0.224, 0.225],
        }
    }

    /// 将图片字节数据预处理为模型输入 Tensor
    /// 返回 shape: [1, 3, image_size, image_size]
    pub fn preprocess_bytes(&self, image_bytes: &[u8], device: &Device) -> std::result::Result<Tensor, String> {
        // 1. 解码图片
        let img = image::load_from_memory(image_bytes)
            .map_err(|e| format!("图片解码失败: {}", e))?;
        let img = img.to_rgb8();

        // 2. 缩放至 image_size x image_size
        let resized = image::imageops::resize(
            &img,
            self.image_size as u32,
            self.image_size as u32,
            image::imageops::FilterType::Lanczos3,
        );

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
        let tensor = tensor.permute((2, 0, 1))
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
        let normalized = tensor.broadcast_sub(&mean)
            .map_err(|e| format!("归一化减法失败: {}", e))?
            .broadcast_div(&std)
            .map_err(|e| format!("归一化除法失败: {}", e))?;

        // 6. 添加 batch 维度: [1, C, H, W]
        normalized.unsqueeze(0)
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
// Patch Embedding (Conv2D)
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
            vb.pp("patch_embed"),
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

// ============================================================================
// 自注意力层
// ============================================================================

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

// ============================================================================
// MLP (FFN)
// ============================================================================

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

// ============================================================================
// Transformer 层（Pre-LN）
// ============================================================================

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

// ============================================================================
// Vision Transformer 编码器
// ============================================================================

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

// ============================================================================
// 完整 Vision 模型
// ============================================================================

/// 完整的 Vision Transformer 模型
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

        // 4. 添加位置编码
        let pos_embed = self.pos_embed.unsqueeze(0)?.expand((b, self.pos_embed.dim(1)?, self.pos_embed.dim(2)?))?;
        let hidden_states = (hidden_states + pos_embed)?;

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
// 模型加载辅助函数
// ============================================================================

/// 尝试从指定路径加载视觉模型
/// 检查 config.json + model.safetensors 是否存在，且 model_type 为视觉模型
pub fn try_load_vision_model(model_path: &str, device: &Device) -> Option<VisionTransformer> {
    let path = Path::new(model_path);

    if !path.exists() || !path.is_dir() {
        return None;
    }

    let config_path = path.join("config.json");
    let weights_path = path.join("model.safetensors");

    if !config_path.exists() || !weights_path.exists() {
        return None;
    }

    // 检查 model_type 是否为视觉模型
    let config_json = std::fs::read_to_string(&config_path).ok()?;
    let config_value: serde_json::Value = serde_json::from_str(&config_json).ok()?;

    let model_type = config_value.get("model_type").and_then(|v| v.as_str()).unwrap_or("");

    let is_vision_model = matches!(
        model_type,
        "jina-clip-v2" | "siglip" | "siglip2" | "vit" | "clip" | "vision"
    );

    // 如果没有明确的 model_type，尝试通过 vision_config 字段判断
    let has_vision_config = config_value.get("vision_config").is_some();

    // 检查是否有视觉模型的关键字段
    let has_image_size = config_value.get("image_size").is_some()
        || config_value.get("vision_config").and_then(|v| v.get("image_size")).is_some();
    let has_patch_size = config_value.get("patch_size").is_some()
        || config_value.get("vision_config").and_then(|v| v.get("patch_size")).is_some();

    if !is_vision_model && !has_vision_config && !(has_image_size && has_patch_size) {
        return None;
    }

    info!("检测到视觉模型 (model_type={}), 开始加载...", model_type);

    match VisionTransformer::load_from_dir(model_path, device) {
        Ok(model) => {
            info!("Vision 模型加载成功: dim={}", model.config().hidden_size);
            Some(model)
        }
        Err(e) => {
            warn!("Vision 模型加载失败: {}", e);
            None
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
    // 处理 Jina-CLIP-v2 类型的命名差异
    if model_type == "jina-clip-v2" {
        // self_attn 命名映射
        let mut mapped = HashMap::new();
        for (key, tensor) in tensors.drain() {
            let new_key = key
                .replace("self_attn.q_proj", "attention.query")
                .replace("self_attn.k_proj", "attention.key")
                .replace("self_attn.v_proj", "attention.value")
                .replace("self_attn.out_proj", "attention.output")
                .replace("layer_norm1", "attention_ln")
                .replace("layer_norm2", "mlp_ln")
                .replace("pre_layernorm", "pre_ln")
                .replace("post_layernorm", "post_ln")
                .replace("patch_embed", "patch_embed")
                .replace("class_embedding", "cls_token")
                .replace("position_embedding", "pos_embed");
            mapped.insert(new_key, tensor);
        }
        mapped
    } else {
        // 对于 SigLIP2 等模型，直接使用原始名称
        tensors
    }
}