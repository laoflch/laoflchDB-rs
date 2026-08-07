//! 文本精排（Rerank）服务（基于 candle + bge-reranker-v2-m3）
//!
//! bge-reranker-v2-m3 是基于 XLM-RoBERTa 的交叉编码器（Cross-Encoder），
//! 通过 OpenAI 兼容的 `/v1/rerank` HTTP 接口对外提供服务。
//!
//! 交叉编码器将 query 与每个 document 拼接为 `[CLS] query [SEP] document [SEP]`，
//! 一次性经过完整 Transformer 编码，输出一个相关性分数 logits。

use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config, XLMRobertaForSequenceClassification};
use log::{info, warn};
use serde::{Deserialize, Serialize};

/// 精排服务配置
#[derive(Debug, Clone)]
pub struct RerankerServiceConfig {
    /// 模型目录，需包含 config.json / tokenizer.json / model.safetensors
    pub model_path: String,
    /// 是否使用 GPU（需启用 cuda feature）
    pub use_cuda: bool,
    /// 输入最大 token 数（query + document 总长度上限）
    pub max_seq_len: usize,
    /// 默认返回的 top_n
    pub default_top_n: usize,
    /// 权重精度: "f32" / "f16"（fp16 可大幅降低显存占用）
    pub dtype: String,
}

impl Default for RerankerServiceConfig {
    fn default() -> Self {
        Self {
            model_path: "laoflch_db_model/bge-reranker-v2-m3".to_string(),
            use_cuda: false,
            max_seq_len: 1024,
            default_top_n: 10,
            dtype: "f32".to_string(),
        }
    }
}

/// 精排服务
pub struct RerankerService {
    model: Mutex<XLMRobertaForSequenceClassification>,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
    service_config: RerankerServiceConfig,
    /// XLM-R 特殊 token id
    sep_token_id: u32,
    pad_token_id: u32,
}

/// 从模型 config.json 反序列化所需字段，缺失字段使用 bge-reranker-v2-m3 默认值
#[derive(Debug, Clone, Deserialize)]
struct ModelConfig {
    #[serde(default = "mc_hidden")]
    hidden_size: usize,
    #[serde(default = "mc_eps")]
    layer_norm_eps: f64,
    #[serde(default)]
    attention_probs_dropout_prob: f32,
    #[serde(default)]
    hidden_dropout_prob: f32,
    #[serde(default = "mc_heads")]
    num_attention_heads: usize,
    #[serde(default = "mc_pet")]
    position_embedding_type: String,
    #[serde(default = "mc_inter")]
    intermediate_size: usize,
    #[serde(default = "mc_act")]
    hidden_act: candle_nn::Activation,
    #[serde(default = "mc_layers")]
    num_hidden_layers: usize,
    #[serde(default = "mc_vocab")]
    vocab_size: usize,
    #[serde(default = "mc_maxpos")]
    max_position_embeddings: usize,
    #[serde(default = "mc_typevocab")]
    type_vocab_size: usize,
    #[serde(default = "mc_pad")]
    pad_token_id: u32,
}

fn mc_hidden() -> usize { 1024 }
fn mc_eps() -> f64 { 1e-5 }
fn mc_heads() -> usize { 16 }
fn mc_pet() -> String { "absolute".to_string() }
fn mc_inter() -> usize { 4096 }
fn mc_act() -> candle_nn::Activation { candle_nn::Activation::Gelu }
fn mc_layers() -> usize { 24 }
fn mc_vocab() -> usize { 250002 }
fn mc_maxpos() -> usize { 8192 }
fn mc_typevocab() -> usize { 2 }
fn mc_pad() -> u32 { 1 }

impl ModelConfig {
    fn to_candle(&self) -> Config {
        Config {
            hidden_size: self.hidden_size,
            layer_norm_eps: self.layer_norm_eps,
            attention_probs_dropout_prob: self.attention_probs_dropout_prob,
            hidden_dropout_prob: self.hidden_dropout_prob,
            num_attention_heads: self.num_attention_heads,
            position_embedding_type: self.position_embedding_type.clone(),
            intermediate_size: self.intermediate_size,
            hidden_act: self.hidden_act.clone(),
            num_hidden_layers: self.num_hidden_layers,
            vocab_size: self.vocab_size,
            max_position_embeddings: self.max_position_embeddings,
            type_vocab_size: self.type_vocab_size,
            pad_token_id: self.pad_token_id,
        }
    }
}

/// 单条精排结果
#[derive(Debug, Clone)]
pub struct RerankResult {
    pub index: usize,
    pub score: f32,
}

/// 精排输出
#[derive(Debug, Clone)]
pub struct RerankOutput {
    pub results: Vec<RerankResult>,
    pub total_tokens: usize,
}

impl RerankerService {
    /// 加载模型并构建服务实例
    pub fn new(service_config: RerankerServiceConfig) -> anyhow::Result<Self> {
        let device = detect_device(service_config.use_cuda);
        let path = Path::new(&service_config.model_path);

        // 加载 config.json
        let config_json = std::fs::read_to_string(path.join("config.json"))?;
        let model_config: ModelConfig = serde_json::from_str(&config_json)?;
        let cfg = model_config.to_candle();
        info!(
            "Reranker 配置加载成功: vocab={}, hidden={}, layers={}, heads={}",
            cfg.vocab_size, cfg.hidden_size, cfg.num_hidden_layers, cfg.num_attention_heads
        );

        // 加载 tokenizer.json
        let tokenizer = tokenizers::Tokenizer::from_file(path.join("tokenizer.json"))
            .map_err(|e| anyhow::anyhow!("加载 tokenizer 失败: {}", e))?;

        // 加载 model.safetensors
        let tensors = candle_core::safetensors::load(path.join("model.safetensors"), &device)?;
        let dtype = parse_dtype(&service_config.dtype)?;
        info!("Reranker 权重精度: {:?}", dtype);
        let vb = VarBuilder::from_tensors(tensors, dtype, &device);

        // 构建交叉编码器（num_labels = 1）
        let model = XLMRobertaForSequenceClassification::new(1, &cfg, vb)?;
        info!("Reranker 模型加载成功: {}", service_config.model_path);

        let sep_token_id = tokenizer
            .token_to_id("</s>")
            .ok_or_else(|| anyhow::anyhow!("tokenizer 缺少 </s> token"))?;
        let pad_token_id = cfg.pad_token_id;

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            device,
            service_config,
            sep_token_id,
            pad_token_id,
        })
    }

    /// 执行精排：query 与每个 document 拼接，输出相关性分数（降序）。
    pub fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_n: Option<usize>,
    ) -> anyhow::Result<RerankOutput> {
        if documents.is_empty() {
            return Ok(RerankOutput {
                results: Vec::new(),
                total_tokens: 0,
            });
        }
        let max_seq = self.service_config.max_seq_len;
        let top_n = top_n.unwrap_or(self.service_config.default_top_n);

        // 编码 query：[CLS] query [SEP]
        let q_enc = self
            .tokenizer
            .encode(query, true)
            .map_err(|e| anyhow::anyhow!("query 编码失败: {}", e))?;
        let q_ids = q_enc.get_ids().to_vec();
        if q_ids.is_empty() {
            return Err(anyhow::anyhow!("query 编码后为空"));
        }
        let max_doc_len = max_seq.saturating_sub(q_ids.len() + 1);

        // 为每个 document 构建 [CLS] query [SEP] document [SEP] 并计算实际长度
        let mut pairs: Vec<Vec<u32>> = Vec::with_capacity(documents.len());
        let mut token_counts: Vec<usize> = Vec::with_capacity(documents.len());
        let mut total_tokens = 0usize;
        for doc in documents {
            let d_enc = self
                .tokenizer
                .encode(doc.as_str(), false)
                .map_err(|e| anyhow::anyhow!("document 编码失败: {}", e))?;
            let mut doc_ids: Vec<u32> = d_enc.get_ids().to_vec();
            if doc_ids.len() > max_doc_len {
                doc_ids.truncate(max_doc_len);
            }
            let mut pair = Vec::with_capacity(q_ids.len() + doc_ids.len() + 1);
            pair.extend_from_slice(&q_ids);
            pair.extend_from_slice(&doc_ids);
            pair.push(self.sep_token_id);
            let len = pair.len();
            token_counts.push(len);
            total_tokens += len;
            pairs.push(pair);
        }

        let batch_size = pairs.len();
        let max_len = pairs.iter().map(|p| p.len()).max().unwrap_or(0);

        // 填充到 batch 内最大长度
        let mut input_ids: Vec<u32> = Vec::with_capacity(batch_size * max_len);
        let mut attention_mask: Vec<u32> = Vec::with_capacity(batch_size * max_len);
        let mut token_type_ids: Vec<u32> = Vec::with_capacity(batch_size * max_len);
        for pair in &pairs {
            for i in 0..max_len {
                if i < pair.len() {
                    input_ids.push(pair[i]);
                    attention_mask.push(1);
                } else {
                    input_ids.push(self.pad_token_id);
                    attention_mask.push(0);
                }
                token_type_ids.push(0);
            }
        }

        let input_t = Tensor::new(input_ids.as_slice(), &self.device)?.reshape((batch_size, max_len))?;
        let mask_t = Tensor::new(attention_mask.as_slice(), &self.device)?.reshape((batch_size, max_len))?;
        let tt_t = Tensor::new(token_type_ids.as_slice(), &self.device)?.reshape((batch_size, max_len))?;

        let model = self.model.lock().unwrap();
        let logits = model.forward(&input_t, &mask_t, &tt_t)?;
        let logits_vec = logits.to_vec2::<f32>()?;

        // 收集 (index, score) 并按分数降序
        let mut scored: Vec<(usize, f32)> = logits_vec
            .into_iter()
            .enumerate()
            .map(|(i, row)| (i, row.first().copied().unwrap_or(0.0)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<RerankResult> = scored
            .into_iter()
            .take(top_n)
            .map(|(i, s)| RerankResult { index: i, score: s })
            .collect();

        Ok(RerankOutput {
            results,
            total_tokens,
        })
    }

    /// 健康检查：模型是否就绪
    pub fn is_ready(&self) -> bool {
        true
    }

    pub fn model_name(&self) -> String {
        Path::new(&self.service_config.model_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.service_config.model_path.clone())
    }
}

/// 解析权重精度字符串为 DType
fn parse_dtype(s: &str) -> anyhow::Result<DType> {
    match s.to_ascii_lowercase().as_str() {
        "f32" | "fp32" => Ok(DType::F32),
        "f16" | "fp16" => Ok(DType::F16),
        other => Err(anyhow::anyhow!("不支持的 dtype: {}（支持 f32/fp32、f16/fp16）", other)),
    }
}

/// 设备检测：CUDA 可用则用 GPU，否则回退 CPU
fn detect_device(_use_cuda: bool) -> Device {
    #[cfg(feature = "cuda")]
    {
        if _use_cuda {
            info!("CUDA feature 已启用，检测 GPU...");
            match Device::cuda_if_available(0) {
                Ok(device) => {
                    info!("CUDA GPU 可用，使用 GPU 设备");
                    return device;
                }
                Err(e) => {
                    warn!("CUDA 设备初始化失败: {}，回退到 CPU", e);
                }
            }
        } else {
            info!("配置禁用 CUDA，使用 CPU");
        }
    }

    #[cfg(not(feature = "cuda"))]
    {
        info!("CUDA feature 未启用，使用 CPU 设备");
        info!("提示: 如需启用 GPU 加速，请使用: cargo build --release --features cuda");
    }

    Device::Cpu
}

// ============================ OpenAI 兼容 HTTP 接口 ============================

/// OpenAI 兼容的 /v1/rerank 请求体
#[derive(Debug, Deserialize)]
pub struct RerankHttpRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub query: String,
    pub documents: Vec<String>,
    #[serde(default)]
    pub top_n: Option<usize>,
    #[serde(default)]
    pub return_documents: bool,
}

#[derive(Debug, Serialize)]
pub struct RerankHttpResult {
    pub index: usize,
    pub relevance_score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<RerankHttpDocument>,
}

#[derive(Debug, Serialize)]
pub struct RerankHttpDocument {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct RerankHttpUsage {
    pub total_tokens: usize,
}

#[derive(Debug, Serialize)]
pub struct RerankHttpResponse {
    pub id: String,
    pub object: String,
    pub model: String,
    pub results: Vec<RerankHttpResult>,
    pub usage: RerankHttpUsage,
}

/// 生成简单请求 id
fn make_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("rerank-{:016x}", ts)
}

/// 创建 REST Router，暴露 OpenAI 兼容的 `/rerank` 与 `/health`
pub fn create_rest_router(service: Arc<RerankerService>) -> Router {
    Router::new()
        .route("/rerank", post(rerank_handler))
        .route("/health", get(health_handler))
        .with_state(service)
}

async fn health_handler(State(service): State<Arc<RerankerService>>) -> impl IntoResponse {
    let body = serde_json::json!({
        "ready": service.is_ready(),
        "model": service.model_name(),
        "status": "ready",
    });
    Json(body)
}

async fn rerank_handler(
    State(service): State<Arc<RerankerService>>,
    Json(req): Json<RerankHttpRequest>,
) -> impl IntoResponse {
    let query = req.query.trim();
    if query.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {"message": "query 不能为空", "type": "invalid_request_error"}
            })),
        );
    }
    if req.documents.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {"message": "documents 不能为空", "type": "invalid_request_error"}
            })),
        );
    }

    let model = req
        .model
        .clone()
        .unwrap_or_else(|| service.model_name());

    match service.rerank(query, &req.documents, req.top_n) {
        Ok(out) => {
            let results: Vec<RerankHttpResult> = out
                .results
                .into_iter()
                .map(|r| RerankHttpResult {
                    index: r.index,
                    relevance_score: r.score,
                    document: if req.return_documents {
                        Some(RerankHttpDocument {
                            text: req.documents[r.index].clone(),
                        })
                    } else {
                        None
                    },
                })
                .collect();
            let resp = RerankHttpResponse {
                id: make_id(),
                object: "list".to_string(),
                model,
                results,
                usage: RerankHttpUsage {
                    total_tokens: out.total_tokens,
                },
            };
            match serde_json::to_value(&resp) {
                Ok(v) => (StatusCode::OK, Json(v)),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": {"message": "响应序列化失败", "type": "internal_error"}
                    })),
                ),
            }
        }
        Err(e) => {
            let msg = format!("精排失败: {}", e);
            warn!("{}", msg);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": {"message": msg, "type": "internal_error"}
                })),
            )
        }
    }
}
