pub mod proto {
    tonic::include_proto!("laoflchdb.vector");
}

use candle_core::{Device, Tensor, DType};
use candle_nn::{VarBuilder, Dropout, Module, ModuleT};
use log::{info, warn};
use std::collections::HashMap;
use std::path::Path;
use tokio::sync::RwLock as AsyncRwLock;
use proto::*;
use tonic::{Request, Response, Status};

/// 向量化服务实现
/// 使用 Candle 引擎在本地 GPU (RTX 2070S) 上运行模型推理
pub struct VectorServiceImpl {
    models: AsyncRwLock<HashMap<String, ModelInstance>>,
    default_device: Device,
    model_dir: String,
}

struct ModelInstance {
    embedding_dim: usize,
    model_path: String,
    loaded: bool,
    device: String,
    /// 真实的 BERT 模型（如果已加载）
    bert_model: Option<RealBertModel>,
}

impl VectorServiceImpl {
    /// 创建服务实例，使用默认模型目录，扫描 candle 子目录加载所有模型
    pub fn new() -> Self {
        Self::new_with_model_dir("./laoflch_db_model")
    }

    /// 创建服务实例，指定模型目录，扫描 `{model_dir}/candle/` 子目录加载所有有效模型
    pub fn new_with_model_dir(model_dir: &str) -> Self {
        Self::new_with_config(model_dir, None)
    }

    /// 创建服务实例，指定模型目录和自动加载配置
    /// - `model_dir`: 模型存储根目录
    /// - `auto_load_models`: `None`=加载 candle 下所有有效模型, `Some(vec)`=只加载指定名称的模型, `Some(vec![])`=不加载任何模型
    pub fn new_with_config(model_dir: &str, auto_load_models: Option<Vec<String>>) -> Self {
        let device = Self::detect_device();
        let model_dir = model_dir.to_string();
        let candle_dir = Path::new(&model_dir).join("candle");
        let models = Self::init_models_from_dir(&candle_dir, &device, auto_load_models);
        info!(
            "VectorService 初始化完成: device={:?}, candle_dir='{}', 已加载 {} 个模型",
            device,
            candle_dir.display(),
            models.len()
        );
        VectorServiceImpl {
            models: AsyncRwLock::new(models),
            default_device: device,
            model_dir,
        }
    }

    /// 从模型目录扫描并加载所有有效模型
    /// 仅扫描 `{candle_dir}` 子目录中的模型
    fn init_models_from_dir(
        candle_dir: &Path,
        device: &Device,
        auto_load_models: Option<Vec<String>>,
    ) -> HashMap<String, ModelInstance> {
        if !candle_dir.exists() || !candle_dir.is_dir() {
            info!(
                "Candle 模型目录 '{}' 不存在或不是目录，跳过自动加载",
                candle_dir.display()
            );
            return HashMap::new();
        }

        info!("扫描 Candle 模型目录: '{}'", candle_dir.display());
        let mut models = HashMap::new();

        let entries = match std::fs::read_dir(candle_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("读取模型目录失败: {}", e);
                return HashMap::new();
            }
        };

        // 确定要加载的模型名称集合
        let load_targets: Option<Vec<String>> = auto_load_models.map(|m| {
            m.into_iter()
                .map(|s| s.to_lowercase())
                .collect()
        });

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let model_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            // 如果指定了加载列表，只加载列表中的模型
            // Some(vec![]) = 不加载任何模型
            // Some(vec!["a", "b"]) = 只加载 a, b
            // None = 加载所有
            if let Some(ref targets) = load_targets {
                if targets.is_empty() {
                    // 空列表 = 跳过所有模型
                    continue;
                }
                if !targets.contains(&model_name.to_lowercase()) {
                    info!("跳过模型 '{}'（不在 load_models 列表中）", model_name);
                    continue;
                }
            }

            // 检查是否包含完整模型文件
            let config_path = path.join("config.json");
            let tokenizer_path = path.join("tokenizer.json");
            let weights_path = path.join("model.safetensors");

            let has_config = config_path.exists();
            let has_tokenizer = tokenizer_path.exists();
            let has_weights = weights_path.exists();

            if !has_config || !has_tokenizer || !has_weights {
                info!(
                    "跳过不完整模型目录 '{}': config={}, tokenizer={}, weights={}",
                    model_name, has_config, has_tokenizer, has_weights
                );
                continue;
            }

            // 从 config.json 读取 hidden_size 作为向量维度
            let dim = std::fs::read_to_string(&config_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                .and_then(|j| {
                    j.get("hidden_size")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize)
                })
                .unwrap_or(384);

            info!("自动发现模型: '{}' (dim={})", model_name, dim);

            let model_path_str = path.to_string_lossy().to_string();
            let bert_model = try_load_bert_model(&model_path_str, device);
            let loaded = bert_model.is_some();
            let device_str = format!("{:?}", device);

            if loaded {
                info!("模型 '{}' 自动加载成功 (dim={})", model_name, dim);
            } else {
                warn!("模型 '{}' 检测到文件但加载失败，仍注册为可用", model_name);
            }

            models.insert(
                model_name,
                ModelInstance {
                    embedding_dim: dim,
                    model_path: model_path_str,
                    loaded,
                    device: device_str,
                    bert_model,
                },
            );
        }

        info!("从目录 '{}' 加载了 {} 个模型", candle_dir.display(), models.len());
        models
    }

    fn detect_device() -> Device {
        #[cfg(feature = "cuda")]
        {
            info!("CUDA feature 已启用，检测 GPU...");
            match Device::cuda_if_available(0) {
                Ok(device) => {
                    info!("CUDA GPU 可用，使用 GPU 设备: RTX 2070S");
                    return device;
                }
                Err(e) => {
                    warn!("CUDA 设备初始化失败: {}，回退到 CPU", e);
                }
            }
        }

        #[cfg(not(feature = "cuda"))]
        {
            info!("CUDA feature 未启用，使用 CPU 设备");
            info!("提示: 如需启用 GPU 加速，请使用: cargo build --release --features cuda");
        }

        Device::Cpu
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }

        let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot_product / (norm_a * norm_b)
    }

    fn l2_normalize(embedding: &[f32]) -> Vec<f32> {
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm == 0.0 {
            return embedding.to_vec();
        }
        embedding.iter().map(|x| x / norm).collect()
    }
}

#[tonic::async_trait]
impl proto::vector_service_server::VectorService for VectorServiceImpl {
    async fn create_embedding(
        &self,
        request: Request<EmbeddingRequest>,
    ) -> Result<Response<EmbeddingResponse>, Status> {
        let req = request.into_inner();
        let model_name = req.model_name.as_str();

        let models = self.models.read().await;
        let model = models.get(model_name).ok_or_else(|| {
            Status::not_found(format!("模型 '{}' 未加载，请先调用 LoadModel", model_name))
        })?;

        if !model.loaded {
            return Err(Status::failed_precondition(format!(
                "模型 '{}' 未正确加载完成",
                model_name
            )));
        }

        info!(
            "生成向量化: model={}, texts_count={}, dim={}",
            model_name,
            req.texts.len(),
            model.embedding_dim
        );

        let mut results = Vec::new();

        // 如果有真实 BERT 模型，使用它进行推理
        if let Some(ref bert_model) = model.bert_model {
            for text in &req.texts {
                match bert_model.embed(text) {
                    Ok(embedding) => {
                        results.push(EmbeddingResult {
                            text: text.clone(),
                            embedding,
                            dim: model.embedding_dim as i32,
                        });
                    }
                    Err(e) => {
                        return Err(Status::internal(format!("向量化失败: {}", e)));
                    }
                }
            }
        } else {
            // fallback: 哈希嵌入
            for text in &req.texts {
                let embedding = generate_fallback_embedding(text, model.embedding_dim)
                    .map_err(|e| Status::internal(format!("向量化失败: {}", e)))?;

                results.push(EmbeddingResult {
                    text: text.clone(),
                    embedding,
                    dim: model.embedding_dim as i32,
                });
            }
        }

        Ok(Response::new(EmbeddingResponse {
            success: true,
            message: format!("成功生成 {} 条向量", results.len()),
            results,
        }))
    }

    async fn compute_similarity(
        &self,
        request: Request<SimilarityRequest>,
    ) -> Result<Response<SimilarityResponse>, Status> {
        let req = request.into_inner();

        if req.query_embedding.is_empty() {
            return Err(Status::invalid_argument("查询向量不能为空"));
        }

        if req.candidates.is_empty() {
            return Ok(Response::new(SimilarityResponse {
                success: true,
                message: "没有候选向量可供比较".to_string(),
                results: vec![],
            }));
        }

        let query_norm = VectorServiceImpl::l2_normalize(&req.query_embedding);

        let mut scored: Vec<(usize, EmbeddingResult, f32)> = req
            .candidates
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let score = VectorServiceImpl::cosine_similarity(&query_norm, &c.embedding);
                (i, c, score)
            })
            .collect();

        // 按相似度降序排序
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let top_k = req.top_k.max(1) as usize;
        let results: Vec<SimilarityResult> = scored
            .into_iter()
            .take(top_k)
            .enumerate()
            .map(|(rank, (_, candidate, score))| SimilarityResult {
                text: candidate.text,
                embedding: candidate.embedding,
                score,
                rank: rank as i32 + 1,
            })
            .collect();

        Ok(Response::new(SimilarityResponse {
            success: true,
            message: format!("成功计算 {} 条相似度结果", results.len()),
            results,
        }))
    }

    async fn get_model_info(
        &self,
        request: Request<ModelInfoRequest>,
    ) -> Result<Response<ModelInfoResponse>, Status> {
        let req = request.into_inner();
        let models = self.models.read().await;

        match models.get(&req.model_name) {
            Some(model) => Ok(Response::new(ModelInfoResponse {
                success: true,
                message: "成功获取模型信息".to_string(),
                model_name: req.model_name,
                embedding_dim: model.embedding_dim as i32,
                model_path: model.model_path.clone(),
                device: model.device.clone(),
                loaded: model.loaded,
            })),
            None => Err(Status::not_found(format!(
                "模型 '{}' 未找到",
                req.model_name
            ))),
        }
    }

    async fn list_models(
        &self,
        _request: Request<ListModelsRequest>,
    ) -> Result<Response<ListModelsResponse>, Status> {
        let models = self.models.read().await;

        let model_infos: Vec<ModelInfoResponse> = models
            .iter()
            .map(|(name, m)| ModelInfoResponse {
                success: true,
                message: String::new(),
                model_name: name.clone(),
                embedding_dim: m.embedding_dim as i32,
                model_path: m.model_path.clone(),
                device: m.device.clone(),
                loaded: m.loaded,
            })
            .collect();

        Ok(Response::new(ListModelsResponse {
            success: true,
            message: format!("共 {} 个模型", model_infos.len()),
            models: model_infos,
        }))
    }

    async fn load_model(
        &self,
        request: Request<LoadModelRequest>,
    ) -> Result<Response<LoadModelResponse>, Status> {
        let req = request.into_inner();

        info!(
            "加载模型: name={}, path={}, dim={}",
            req.model_name, req.model_path, req.embedding_dim
        );

        if req.model_name.is_empty() {
            return Err(Status::invalid_argument("模型名称不能为空"));
        }

        let model_path = Path::new(&req.model_path);
        if !model_path.exists() || !model_path.is_dir() {
            return Err(Status::not_found(format!(
                "模型路径 '{}' 不存在或不是目录",
                req.model_path
            )));
        }

        let device_str = format!("{:?}", self.default_device);

        // 尝试加载真实 BERT 模型
        let bert_model = try_load_bert_model(&req.model_path, &self.default_device);

        let bert_model = match bert_model {
            Some(model) => {
                let hidden_size = model.config.hidden_size;
                info!(
                    "模型 '{}' 成功加载为真实 BERT 模型，dim={}",
                    req.model_name, hidden_size
                );
                model
            }
            None => {
                return Err(Status::not_found(format!(
                    "模型路径 '{}' 缺少必要文件 (需要 config.json, tokenizer.json, model.safetensors)",
                    req.model_path
                )));
            }
        };

        let mut models = self.models.write().await;
        models.insert(
            req.model_name.clone(),
            ModelInstance {
                embedding_dim: bert_model.config.hidden_size,
                model_path: req.model_path,
                loaded: true,
                device: device_str,
                bert_model: Some(bert_model),
            },
        );

        Ok(Response::new(LoadModelResponse {
            success: true,
            message: format!("模型 '{}' 注册成功", req.model_name),
            model_name: req.model_name,
        }))
    }

    async fn unload_model(
        &self,
        request: Request<UnloadModelRequest>,
    ) -> Result<Response<UnloadModelResponse>, Status> {
        let req = request.into_inner();
        let mut models = self.models.write().await;

        if models.remove(&req.model_name).is_some() {
            info!("模型 '{}' 已卸载", req.model_name);
            Ok(Response::new(UnloadModelResponse {
                success: true,
                message: format!("模型 '{}' 已成功卸载", req.model_name),
                model_name: req.model_name,
            }))
        } else {
            Err(Status::not_found(format!(
                "模型 '{}' 未找到，无法卸载",
                req.model_name
            )))
        }
    }

    async fn list_loadable_models(
        &self,
        _request: Request<ListLoadableModelsRequest>,
    ) -> Result<Response<ListLoadableModelsResponse>, Status> {
        let candle_dir = Path::new(&self.model_dir).join("candle");
        let loaded_models = self.models.read().await;

        let mut loadable = Vec::new();

        if candle_dir.exists() && candle_dir.is_dir() {
            match std::fs::read_dir(&candle_dir) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }
                        let model_name = match path.file_name().and_then(|n| n.to_str()) {
                            Some(name) => name.to_string(),
                            None => continue,
                        };

                        let config_path = path.join("config.json");
                        let tokenizer_path = path.join("tokenizer.json");
                        let weights_path = path.join("model.safetensors");

                        let has_config = config_path.exists();
                        let has_tokenizer = tokenizer_path.exists();
                        let has_weights = weights_path.exists();

                        // 从 config.json 读取维度
                        let dim = if has_config {
                            std::fs::read_to_string(&config_path)
                                .ok()
                                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                                .and_then(|j| {
                                    j.get("hidden_size")
                                        .and_then(|v| v.as_u64())
                                        .map(|v| v as i32)
                                })
                                .unwrap_or(0)
                        } else {
                            0
                        };

                        let is_loaded = loaded_models.contains_key(&model_name);

                        loadable.push(LoadableModelInfo {
                            model_name,
                            model_path: path.to_string_lossy().to_string(),
                            embedding_dim: dim,
                            has_config,
                            has_tokenizer,
                            has_weights,
                            is_loaded,
                        });
                    }
                }
                Err(e) => {
                    warn!("读取模型目录 '{}' 失败: {}", self.model_dir, e);
                }
            }
        }

        Ok(Response::new(ListLoadableModelsResponse {
            success: true,
            message: format!("共 {} 个可加载模型", loadable.len()),
            model_dir: candle_dir.to_string_lossy().to_string(),
            models: loadable,
        }))
    }
}

// ============================================================================
// BERT 模型实现 (使用 candle-nn 0.10.2 从零构建)
// ============================================================================

/// BERT 配置，从 config.json 反序列化
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BertConfig {
    pub vocab_size: usize,
    pub hidden_size: usize,
    pub num_hidden_layers: usize,
    pub num_attention_heads: usize,
    pub intermediate_size: usize,
    #[serde(default)]
    pub hidden_act: Option<String>,
    #[serde(default)]
    pub hidden_dropout_prob: Option<f64>,
    #[serde(default)]
    pub attention_probs_dropout_prob: Option<f64>,
    pub max_position_embeddings: usize,
    #[serde(default)]
    pub type_vocab_size: Option<usize>,
    #[serde(default)]
    pub layer_norm_eps: Option<f64>,
    #[serde(default)]
    pub pad_token_id: Option<usize>,
}

impl BertConfig {
    fn hidden_dropout_prob(&self) -> f64 {
        self.hidden_dropout_prob.unwrap_or(0.1)
    }

    fn attention_probs_dropout_prob(&self) -> f64 {
        self.attention_probs_dropout_prob.unwrap_or(0.1)
    }

    fn type_vocab_size(&self) -> usize {
        self.type_vocab_size.unwrap_or(2)
    }

    fn layer_norm_eps(&self) -> f64 {
        self.layer_norm_eps.unwrap_or(1e-12)
    }

    fn pad_token_id(&self) -> usize {
        self.pad_token_id.unwrap_or(0)
    }
}

// ---- 手动 LayerNorm（CUDA 兼容） ----

/// 自定义 LayerNorm，使用基础运算（mean、std、sub、div、mul、add）
/// 避免依赖 candle_nn::layer_norm 的 CUDA kernel
struct CudaLayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
    size: usize,
}

impl CudaLayerNorm {
    fn load(size: usize, eps: f64, vb: VarBuilder) -> Result<Self, candle_core::Error> {
        let weight = vb.get_with_hints(size, "weight", candle_nn::Init::Const(1.0))?;
        let bias = vb.get_with_hints(size, "bias", candle_nn::Init::Const(0.0))?;
        Ok(Self { weight, bias, eps, size })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor, candle_core::Error> {
        // LayerNorm on last dimension: y = (x - mean) / sqrt(var + eps) * weight + bias
        let dim = x.dims().len() - 1;
        let mean = x.mean_keepdim(dim)?;
        let centered = x.broadcast_sub(&mean)?;
        let variance = centered.sqr()?.mean_keepdim(dim)?;
        let eps_t = Tensor::new(self.eps as f32, x.device())?;
        let normalized = centered.broadcast_div(&(variance.broadcast_add(&eps_t)?).sqrt()?)?;
        normalized.broadcast_mul(&self.weight)?.broadcast_add(&self.bias)
    }
}

// ---- BERT 子模块 ----

struct BertEmbeddings {
    word_embeddings: candle_nn::Embedding,
    position_embeddings: candle_nn::Embedding,
    token_type_embeddings: candle_nn::Embedding,
    layer_norm: CudaLayerNorm,
    dropout: Dropout,
}

impl BertEmbeddings {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let word_embeddings = candle_nn::embedding(config.vocab_size, config.hidden_size, vb.pp("word_embeddings"))?;
        let position_embeddings = candle_nn::embedding(
            config.max_position_embeddings,
            config.hidden_size,
            vb.pp("position_embeddings"),
        )?;
        let token_type_embeddings = candle_nn::embedding(
            config.type_vocab_size(),
            config.hidden_size,
            vb.pp("token_type_embeddings"),
        )?;
        let layer_norm = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps(),
            vb.pp("LayerNorm"),
        )?;
        let dropout = Dropout::new(config.hidden_dropout_prob() as f32);
        Ok(Self {
            word_embeddings,
            position_embeddings,
            token_type_embeddings,
            layer_norm,
            dropout,
        })
    }
}

impl ModuleT for BertEmbeddings {
    fn forward_t(&self, input: &Tensor, train: bool) -> Result<Tensor, candle_core::Error> {
        let (input_ids, token_type_ids) = if input.dims().len() == 2 {
            (input.clone(), Tensor::zeros(input.shape(), DType::I64, input.device())?)
        } else {
            let input_ids = input.narrow(2, 0, 1)?;
            let token_type_ids = input.narrow(2, 1, 1)?;
            (input_ids.squeeze(2)?, token_type_ids.squeeze(2)?)
        };

        let seq_len = input_ids.dim(1)?;
        let pos_ids = Tensor::arange(0u32, seq_len as u32, input_ids.device())?
            .unsqueeze(0)?
            .expand(input_ids.shape())?;

        let word_emb = self.word_embeddings.forward(&input_ids)?;
        let pos_emb = self.position_embeddings.forward(&pos_ids)?;
        let type_emb = self.token_type_embeddings.forward(&token_type_ids)?;

        let emb = ((word_emb + pos_emb)? + type_emb)?;
        let emb = self.layer_norm.forward(&emb)?;
        let emb = self.dropout.forward_t(&emb, train)?;
        Ok(emb)
    }
}

struct BertSelfAttention {
    query: candle_nn::Linear,
    key: candle_nn::Linear,
    value: candle_nn::Linear,
    dropout: Dropout,
    num_attention_heads: usize,
    attention_head_size: usize,
    hidden_size: usize,
}

impl BertSelfAttention {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let hidden_size = config.hidden_size;
        let num_heads = config.num_attention_heads;
        let head_size = hidden_size / num_heads;

        let query = candle_nn::linear(hidden_size, hidden_size, vb.pp("query"))?;
        let key = candle_nn::linear(hidden_size, hidden_size, vb.pp("key"))?;
        let value = candle_nn::linear(hidden_size, hidden_size, vb.pp("value"))?;
        let dropout = Dropout::new(config.attention_probs_dropout_prob() as f32);

        Ok(Self {
            query,
            key,
            value,
            dropout,
            num_attention_heads: num_heads,
            attention_head_size: head_size,
            hidden_size,
        })
    }

    fn transpose_for_scores(&self, xs: &Tensor) -> Result<Tensor, candle_core::Error> {
        let (b_sz, seq_len, _hidden) = xs.dims3()?;
        let xs = xs.reshape((b_sz, seq_len, self.num_attention_heads, self.attention_head_size))?;
        xs.permute((0, 2, 1, 3))?.contiguous()
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor, candle_core::Error> {
        let query = self.query.forward(hidden_states)?;
        let key = self.key.forward(hidden_states)?;
        let value = self.value.forward(hidden_states)?;

        let query = self.transpose_for_scores(&query)?;
        let key = self.transpose_for_scores(&key)?;
        let value = self.transpose_for_scores(&value)?;

        let scale = 1.0f32 / (self.attention_head_size as f32).sqrt();
        let attention_scores = query.matmul(&key.t()?.contiguous()?)?.broadcast_mul(&Tensor::new(scale, query.device())?)?;
        let attention_scores = attention_scores.broadcast_add(attention_mask)?;
        // 软最大化在 CPU 上执行（CUDA 可能不支持 softmax_last_dim）
        let orig_device = attention_scores.device().clone();
        let attention_scores_cpu = attention_scores.to_device(&Device::Cpu)?;
        let attention_probs = candle_nn::ops::softmax_last_dim(&attention_scores_cpu)?;
        let attention_probs = attention_probs.to_device(&orig_device)?;
        let attention_probs = self.dropout.forward_t(&attention_probs, false)?;

        let context = attention_probs.matmul(&value)?;
        let context = context.permute((0, 2, 1, 3))?.contiguous()?;
        let (b_sz, seq_len, _heads, _head_size) = context.dims4()?;
        context.reshape((b_sz, seq_len, self.hidden_size))
    }
}

struct BertAttention {
    self_attention: BertSelfAttention,
    self_output: BertSelfOutput,
}

impl BertAttention {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let self_attention = BertSelfAttention::load(vb.pp("self"), config)?;
        let self_output = BertSelfOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            self_attention,
            self_output,
        })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor, candle_core::Error> {
        let self_out = self.self_attention.forward(hidden_states, attention_mask)?;
        self.self_output.forward_with_residual(&self_out, hidden_states)
    }
}

struct BertIntermediate {
    dense: candle_nn::Linear,
}

impl BertIntermediate {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let dense = candle_nn::linear(config.hidden_size, config.intermediate_size, vb.pp("dense"))?;
        Ok(Self { dense })
    }

    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor, candle_core::Error> {
        let hidden = self.dense.forward(hidden_states)?;
        hidden.gelu_erf()
    }
}

struct BertLayer {
    attention: BertAttention,
    intermediate: BertIntermediate,
    output: BertOutput,
}

impl BertLayer {
    fn load(vb: VarBuilder, config: &BertConfig, layer_idx: usize) -> Result<Self, candle_core::Error> {
        let vb = vb.pp("layer").pp(&layer_idx.to_string());
        let attention = BertAttention::load(vb.pp("attention"), config)?;
        let intermediate = BertIntermediate::load(vb.pp("intermediate"), config)?;
        let output = BertOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            attention,
            intermediate,
            output,
        })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor, candle_core::Error> {
        let att_out = self.attention.forward(hidden_states, attention_mask)?;
        let inter_out = self.intermediate.forward(&att_out)?;
        self.output.forward_with_residual(&inter_out, &att_out)
    }
}

struct BertEncoder {
    layers: Vec<BertLayer>,
}

impl BertEncoder {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let mut layers = Vec::with_capacity(config.num_hidden_layers);
        for i in 0..config.num_hidden_layers {
            layers.push(BertLayer::load(vb.clone(), config, i)?);
        }
        Ok(Self { layers })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor, candle_core::Error> {
        let mut h = hidden_states.clone();
        for layer in &self.layers {
            h = layer.forward(&h, attention_mask)?;
        }
        Ok(h)
    }
}

/// 完整的 BERT 模型
pub struct BertModel {
    embeddings: BertEmbeddings,
    encoder: BertEncoder,
}

impl BertModel {
    pub fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let embeddings = BertEmbeddings::load(vb.pp("embeddings"), config)?;
        let encoder = BertEncoder::load(vb.pp("encoder"), config)?;
        Ok(Self {
            embeddings,
            encoder,
        })
    }

    pub fn forward(
        &self,
        input_ids: &Tensor,
        attention_mask: &Tensor,
        _token_type_ids: &Tensor,
    ) -> Result<Tensor, candle_core::Error> {
        // 扩展 attention_mask: [batch, seq] -> [batch, 1, 1, seq]
        let mask = attention_mask.unsqueeze(1)?.unsqueeze(2)?;
        // 转换: 1 -> 0 (keep), 0 -> -10000 (mask out)
        // standard formula: (1 - mask_f32) * -10000.0
        let ones = Tensor::new(1.0f32, mask.device())?;
        let mask = mask.to_dtype(DType::F32)?;
        let scale = Tensor::new(-10000.0f32, mask.device())?;
        let mask = ones.broadcast_sub(&mask)?.broadcast_mul(&scale)?;
        let mask = mask.broadcast_as((input_ids.dim(0)?, 1usize, 1usize, input_ids.dim(1)?))?;

        // embeddings 接受拼接输入 [input_ids, token_type_ids] 或只接收 input_ids
        // 这里分开处理
        let emb = self.embeddings.forward_t(input_ids, false)?;
        let enc = self.encoder.forward(&emb, &mask)?;
        Ok(enc)
    }
}

// ---- BertSelfOutput - 不使用 ModuleT trait，改用普通方法 ----

struct BertSelfOutput {
    dense: candle_nn::Linear,
    layer_norm: CudaLayerNorm,
    dropout: Dropout,
}

impl BertSelfOutput {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let dense = candle_nn::linear(config.hidden_size, config.hidden_size, vb.pp("dense"))?;
        let layer_norm = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps(),
            vb.pp("LayerNorm"),
        )?;
        let dropout = Dropout::new(config.hidden_dropout_prob() as f32);
        Ok(Self {
            dense,
            layer_norm,
            dropout,
        })
    }

    fn forward_with_residual(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor, candle_core::Error> {
        let hidden = self.dense.forward(hidden_states)?;
        let hidden = self.dropout.forward_t(&hidden, false)?;
        self.layer_norm.forward(&(hidden + input_tensor)?)
    }
}

// ---- BertOutput - 不使用 ModuleT trait，改用普通方法 ----

struct BertOutput {
    dense: candle_nn::Linear,
    layer_norm: CudaLayerNorm,
    dropout: Dropout,
}

impl BertOutput {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self, candle_core::Error> {
        let dense = candle_nn::linear(config.intermediate_size, config.hidden_size, vb.pp("dense"))?;
        let layer_norm = CudaLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps(),
            vb.pp("LayerNorm"),
        )?;
        let dropout = Dropout::new(config.hidden_dropout_prob() as f32);
        Ok(Self {
            dense,
            layer_norm,
            dropout,
        })
    }

    fn forward_with_residual(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor, candle_core::Error> {
        let hidden = self.dense.forward(hidden_states)?;
        let hidden = self.dropout.forward_t(&hidden, false)?;
        self.layer_norm.forward(&(hidden + input_tensor)?)
    }
}

/// 封装真实 BERT 模型和 tokenizer
pub struct RealBertModel {
    model: BertModel,
    tokenizer: tokenizers::Tokenizer,
    config: BertConfig,
    device: Device,
}

impl RealBertModel {
    /// 从本地目录加载 BERT 模型
    /// 目录需包含: config.json, tokenizer.json, model.safetensors
    pub fn load_from_dir(model_path: &str, device: &Device) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = Path::new(model_path);

        // 加载 config.json
        let config_path = path.join("config.json");
        let config_json = std::fs::read_to_string(&config_path)?;
        let config: BertConfig = serde_json::from_str(&config_json)?;

        // 加载 tokenizer.json
        let tokenizer_path = path.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| format!("加载 tokenizer 失败: {}", e))?;

        // 加载 model.safetensors
        let safetensors_path = path.join("model.safetensors");
        let tensors = candle_core::safetensors::load(safetensors_path, device)?;
        let vb = VarBuilder::from_tensors(tensors, DType::F32, device);

        // 构建 BERT 模型
        let model = BertModel::load(vb, &config)?;

        info!(
            "BERT 模型加载成功: hidden_size={}, layers={}, heads={}",
            config.hidden_size, config.num_hidden_layers, config.num_attention_heads
        );

        Ok(Self {
            model,
            tokenizer,
            config,
            device: device.clone(),
        })
    }

    /// 对单个文本生成向量
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
        // tokenize
        let encoding = self.tokenizer.encode(text, true)
            .map_err(|e| format!("tokenize 失败: {}", e))?;

        let input_ids = encoding.get_ids().to_vec();
        let attention_mask = encoding.get_attention_mask().to_vec();
        let token_type_ids = encoding.get_type_ids().to_vec();

        if input_ids.is_empty() {
            return Ok(vec![0.0f32; self.config.hidden_size]);
        }

        let seq_len = input_ids.len();
        let device = &self.device;

        // 创建 tensor
        let input_ids_t = Tensor::from_slice(&input_ids, (1, seq_len), device)?;
        let attention_mask_t = Tensor::from_slice(&attention_mask, (1, seq_len), device)?;
        let token_type_ids_t = Tensor::from_slice(&token_type_ids, (1, seq_len), device)?;

        // BERT forward
        let output = self.model.forward(&input_ids_t, &attention_mask_t, &token_type_ids_t)?;

        // mean pooling (忽略 padding token)
        let mask = attention_mask_t.unsqueeze(2)?.to_dtype(DType::F32)?;
        let masked_output = output.broadcast_mul(&mask)?;
        let sum_hidden = masked_output.sum(1)?; // [1, hidden_size]
        let num_tokens = mask.sum(1)?;
        let pooled = sum_hidden.broadcast_div(&num_tokens)?;

        // L2 normalize
        let norm = pooled.sqr()?.sum(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm)?;

        // 转为 Vec<f32>
        let result: Vec<f32> = normalized.squeeze(0)?.to_vec1()?;
        Ok(result)
    }
}

/// 尝试从指定路径加载真实 BERT 模型
/// 如果路径下包含 config.json + tokenizer.json + model.safetensors，则加载
fn try_load_bert_model(model_path: &str, device: &Device) -> Option<RealBertModel> {
    let path = Path::new(model_path);

    if !path.exists() || !path.is_dir() {
        info!("模型路径 '{}' 不存在或不是目录，跳过真实模型加载", model_path);
        return None;
    }

    let config_exists = path.join("config.json").exists();
    let tokenizer_exists = path.join("tokenizer.json").exists();
    let model_exists = path.join("model.safetensors").exists();

    if !config_exists || !tokenizer_exists || !model_exists {
        info!(
            "模型路径 '{}' 缺少必要文件 (config.json={}, tokenizer.json={}, model.safetensors={})",
            model_path, config_exists, tokenizer_exists, model_exists
        );
        return None;
    }

    info!("检测到真实模型文件，开始加载 BERT 模型...");
    match RealBertModel::load_from_dir(model_path, device) {
        Ok(model) => {
            info!("BERT 模型加载成功");
            Some(model)
        }
        Err(e) => {
            warn!("BERT 模型加载失败: {}，将使用 fallback 实现", e);
            None
        }
    }
}

/// 使用字符哈希生成文本的向量表示（fallback 实现）
fn generate_fallback_embedding(text: &str, dim: usize) -> Result<Vec<f32>, Box<dyn std::error::Error + Send + Sync>> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    if text.is_empty() {
        return Ok(vec![0.0f32; dim]);
    }

    let mut embedding = vec![0.0f32; dim];

    for (i, ch) in text.chars().enumerate() {
        let mut hasher = DefaultHasher::new();
        ch.hash(&mut hasher);
        let hash = hasher.finish();

        let pos = i % dim;
        let val = (hash as f64 / u64::MAX as f64) as f32;
        embedding[pos] += val;
    }

    // 归一化
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut embedding {
            *val /= norm;
        }
    }

    Ok(embedding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::proto::vector_service_server::VectorService;
    use tokio;

    #[tokio::test]
    async fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((VectorServiceImpl::cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((VectorServiceImpl::cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_cosine_similarity_empty() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(VectorServiceImpl::cosine_similarity(&a, &b), 0.0);
    }

    #[tokio::test]
    async fn test_cosine_similarity_different_lengths() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert_eq!(VectorServiceImpl::cosine_similarity(&a, &b), 0.0);
    }

    #[tokio::test]
    async fn test_l2_normalize() {
        let v = vec![3.0, 4.0];
        let normalized = VectorServiceImpl::l2_normalize(&v);
        assert!((normalized[0] - 0.6).abs() < 1e-6);
        assert!((normalized[1] - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_l2_normalize_zero() {
        let v = vec![0.0, 0.0];
        let normalized = VectorServiceImpl::l2_normalize(&v);
        assert_eq!(normalized, vec![0.0, 0.0]);
    }

    #[tokio::test]
    async fn test_fallback_embedding() {
        let embedding = generate_fallback_embedding("hello world", 128).unwrap();
        assert_eq!(embedding.len(), 128);

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "向量应已归一化, norm={}", norm);
    }

    #[tokio::test]
    async fn test_fallback_embedding_consistency() {
        let e1 = generate_fallback_embedding("same text", 64).unwrap();
        let e2 = generate_fallback_embedding("same text", 64).unwrap();
        assert_eq!(e1, e2, "相同文本应产生相同向量");
    }

    #[tokio::test]
    async fn test_fallback_embedding_empty() {
        let embedding = generate_fallback_embedding("", 64).unwrap();
        assert_eq!(embedding.len(), 64);
        assert!(embedding.iter().all(|&x| x == 0.0), "空文本应产生零向量");
    }

    #[tokio::test]
    async fn test_fallback_embedding_different() {
        let e1 = generate_fallback_embedding("hello", 64).unwrap();
        let e2 = generate_fallback_embedding("world", 64).unwrap();
        assert_ne!(e1, e2, "不同文本应产生不同向量");
    }

    #[tokio::test]
    async fn test_list_models_empty() {
        let service = VectorServiceImpl::new();
        let req = Request::new(ListModelsRequest {});
        let resp = service.list_models(req).await.unwrap();
        assert!(resp.into_inner().success);
    }

    #[tokio::test]
    async fn test_get_model_not_found() {
        let service = VectorServiceImpl::new();
        let req = Request::new(ModelInfoRequest {
            model_name: "non_existent".to_string(),
        });
        let result = service.get_model_info(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_load_and_list_model() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("laoflch_db_model/candle/bge-small-zh-v1.5");
        if !model_path.join("model.safetensors").exists() {
            return; // 模型文件不存在，跳过测试
        }

        let device = VectorServiceImpl::detect_device();
        let bert_model = crate::try_load_bert_model(
            &model_path.to_string_lossy(),
            &device,
        );
        assert!(bert_model.is_some(), "BERT model should load from: {}",
            model_path.display());

        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "test_model".to_string(),
            model_path: model_path.to_string_lossy().to_string(),
            embedding_dim: 512,
        });
        let load_resp = service.load_model(load_req).await.unwrap();
        assert!(load_resp.into_inner().success);

        let list_req = Request::new(ListModelsRequest {});
        let list_resp = service.list_models(list_req).await.unwrap();
        let models = list_resp.into_inner().models;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_name, "test_model");
        assert_eq!(models[0].embedding_dim, 512);
    }

    #[tokio::test]
    async fn test_unload_model() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("laoflch_db_model/candle/bge-small-zh-v1.5");
        if !model_path.join("model.safetensors").exists() {
            return; // 模型文件不存在，跳过测试
        }

        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "temp_model".to_string(),
            model_path: model_path.to_string_lossy().to_string(),
            embedding_dim: 512,
        });
        service.load_model(load_req).await.unwrap();

        let unload_req = Request::new(UnloadModelRequest {
            model_name: "temp_model".to_string(),
        });
        let unload_resp = service.unload_model(unload_req).await.unwrap();
        assert!(unload_resp.into_inner().success);

        let list_req = Request::new(ListModelsRequest {});
        let list_resp = service.list_models(list_req).await.unwrap();
        assert_eq!(list_resp.into_inner().models.len(), 0);
    }

    #[tokio::test]
    async fn test_unload_non_existent_model() {
        let service = VectorServiceImpl::new();
        let req = Request::new(UnloadModelRequest {
            model_name: "ghost".to_string(),
        });
        let result = service.unload_model(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_compute_similarity_empty_query() {
        let service = VectorServiceImpl::new();
        let req = Request::new(SimilarityRequest {
            model_name: "".to_string(),
            query_embedding: vec![],
            candidates: vec![],
            top_k: 5,
        });
        let result = service.compute_similarity(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_compute_similarity_no_candidates() {
        let service = VectorServiceImpl::new();
        let req = Request::new(SimilarityRequest {
            model_name: "test".to_string(),
            query_embedding: vec![1.0, 0.0, 0.0],
            candidates: vec![],
            top_k: 5,
        });
        let resp = service.compute_similarity(req).await.unwrap();
        assert!(resp.into_inner().results.is_empty());
    }

    #[tokio::test]
    async fn test_compute_similarity_ranked() {
        let service = VectorServiceImpl::new();

        let candidates = vec![
            EmbeddingResult {
                text: "cat".to_string(),
                embedding: vec![1.0, 0.0, 0.0],
                dim: 3,
            },
            EmbeddingResult {
                text: "dog".to_string(),
                embedding: vec![0.9, 0.1, 0.0],
                dim: 3,
            },
            EmbeddingResult {
                text: "car".to_string(),
                embedding: vec![0.0, 1.0, 0.0],
                dim: 3,
            },
        ];

        let req = Request::new(SimilarityRequest {
            model_name: "test".to_string(),
            query_embedding: vec![1.0, 0.0, 0.0],
            candidates,
            top_k: 2,
        });

        let resp = service.compute_similarity(req).await.unwrap();
        let results = resp.into_inner().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].text, "cat");
        assert_eq!(results[0].rank, 1);
        assert_eq!(results[1].text, "dog");
        assert_eq!(results[1].rank, 2);
    }

    #[tokio::test]
    async fn test_create_embedding_without_model() {
        let service = VectorServiceImpl::new();
        let req = Request::new(EmbeddingRequest {
            model_name: "no_model".to_string(),
            texts: vec!["hello".to_string()],
            dim: 128,
        });
        let result = service.create_embedding(req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_embedding_with_model() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("laoflch_db_model/candle/bge-small-zh-v1.5");
        if !model_path.join("model.safetensors").exists() {
            return; // 模型文件不存在，跳过测试
        }

        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "embed_model".to_string(),
            model_path: model_path.to_string_lossy().to_string(),
            embedding_dim: 512,
        });
        service.load_model(load_req).await.unwrap();

        let req = Request::new(EmbeddingRequest {
            model_name: "embed_model".to_string(),
            texts: vec!["hello world".to_string(), "test text".to_string()],
            dim: 512,
        });
        let resp = service.create_embedding(req).await.unwrap();
        let results = resp.into_inner().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].embedding.len(), 512);
        assert_eq!(results[1].embedding.len(), 512);
    }

    #[test]
    fn test_device_detection() {
        let _device = VectorServiceImpl::detect_device();
        // 至少能返回一个设备（CPU）
    }

    // ---- BERT 模型单元测试 ----

    #[test]
    fn test_bert_config_deserialize() {
        let json = r#"{
            "vocab_size": 30522,
            "hidden_size": 384,
            "num_hidden_layers": 6,
            "num_attention_heads": 6,
            "intermediate_size": 1536,
            "max_position_embeddings": 512
        }"#;
        let config: BertConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.vocab_size, 30522);
        assert_eq!(config.hidden_size, 384);
        assert_eq!(config.num_hidden_layers, 6);
        assert_eq!(config.num_attention_heads, 6);
        assert_eq!(config.intermediate_size, 1536);
        assert_eq!(config.max_position_embeddings, 512);
        assert_eq!(config.hidden_dropout_prob(), 0.1); // default
        assert_eq!(config.layer_norm_eps(), 1e-12); // default
        assert_eq!(config.pad_token_id(), 0); // default
    }

    #[test]
    fn test_bert_model_structure() {
        // 验证 BERT 模型结构定义编译通过
        // 实际加载需要模型文件，这里只验证结构
        let config = BertConfig {
            vocab_size: 30522,
            hidden_size: 128,
            num_hidden_layers: 2,
            num_attention_heads: 4,
            intermediate_size: 512,
            hidden_act: Some("gelu".to_string()),
            hidden_dropout_prob: Some(0.1),
            attention_probs_dropout_prob: Some(0.1),
            max_position_embeddings: 128,
            type_vocab_size: Some(2),
            layer_norm_eps: Some(1e-12),
            pad_token_id: Some(0),
        };

        // 验证 attention head size 计算
        let head_size = config.hidden_size / config.num_attention_heads;
        assert_eq!(head_size, 32);

        // 验证 num_hidden_layers
        assert_eq!(config.num_hidden_layers, 2);
    }

    #[test]
    fn test_real_bert_model_not_found() {
        // 不存在的路径应返回 None
        let device = Device::Cpu;
        let result = try_load_bert_model("/tmp/non_existent_model_dir", &device);
        assert!(result.is_none());
    }

    #[test]
    fn test_real_bert_model_missing_files() {
        // 目录存在但缺少模型文件，应返回 None
        let tmp_dir = std::env::temp_dir().join("bert_test_missing");
        std::fs::create_dir_all(&tmp_dir).ok();
        let device = Device::Cpu;
        let result = try_load_bert_model(tmp_dir.to_str().unwrap(), &device);
        assert!(result.is_none());
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_list_loadable_models_empty_dir() {
        // 空目录应返回空列表
        let tmp_dir = std::env::temp_dir().join("loadable_test_empty");
        std::fs::create_dir_all(tmp_dir.join("candle")).ok();
        let service = VectorServiceImpl::new_with_model_dir(tmp_dir.to_str().unwrap());
        let req = Request::new(ListLoadableModelsRequest {});
        let resp = service.list_loadable_models(req).await.unwrap();
        let inner = resp.into_inner();
        assert!(inner.success);
        assert_eq!(inner.models.len(), 0);
        assert!(inner.model_dir.ends_with("candle"));
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_list_loadable_models_with_incomplete() {
        // 不完整的模型目录（缺少文件）应被列出但标记为未加载
        let tmp_dir = std::env::temp_dir().join("loadable_test_incomplete");
        let model_dir = tmp_dir.join("candle").join("my_model");
        std::fs::create_dir_all(&model_dir).ok();
        std::fs::write(model_dir.join("config.json"), r#"{"hidden_size": 384}"#).ok();
        let service = VectorServiceImpl::new_with_model_dir(tmp_dir.to_str().unwrap());
        let req = Request::new(ListLoadableModelsRequest {});
        let resp = service.list_loadable_models(req).await.unwrap();
        let inner = resp.into_inner();
        assert!(inner.success);
        assert_eq!(inner.models.len(), 1, "不完整模型也应列出");
        let m = &inner.models[0];
        assert_eq!(m.model_name, "my_model");
        assert!(m.has_config);
        assert!(!m.has_tokenizer);
        assert!(!m.has_weights);
        assert!(!m.is_loaded);
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_list_loadable_models_non_existent() {
        // 不存在的目录应返回空列表且不报错
        let service = VectorServiceImpl::new_with_model_dir("/tmp/non_existent_path_12345");
        let req = Request::new(ListLoadableModelsRequest {});
        let resp = service.list_loadable_models(req).await.unwrap();
        let inner = resp.into_inner();
        assert!(inner.success);
        assert_eq!(inner.models.len(), 0);
    }

    #[tokio::test]
    async fn test_init_models_from_dir_empty() {
        // 空 candle 目录初始化不应加载任何模型
        let tmp_dir = std::env::temp_dir().join("empty_candle_test");
        std::fs::create_dir_all(tmp_dir.join("candle")).ok();
        let service = VectorServiceImpl::new_with_model_dir(tmp_dir.to_str().unwrap());
        let req = Request::new(ListModelsRequest {});
        let resp = service.list_models(req).await.unwrap();
        let inner = resp.into_inner();
        assert!(inner.success);
        assert_eq!(inner.models.len(), 0);
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_new_with_config_none() {
        // auto_load_models=None 应加载所有（但 candle 目录为空）
        let tmp_dir = std::env::temp_dir().join("config_test_none");
        std::fs::create_dir_all(tmp_dir.join("candle")).ok();
        let service = VectorServiceImpl::new_with_config(tmp_dir.to_str().unwrap(), None);
        let req = Request::new(ListModelsRequest {});
        let resp = service.list_models(req).await.unwrap();
        assert_eq!(resp.into_inner().models.len(), 0);
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_new_with_config_empty_list() {
        // auto_load_models=Some(vec![]) 应跳过所有
        let tmp_dir = std::env::temp_dir().join("config_test_empty_list");
        let model_dir = tmp_dir.join("candle").join("test_model");
        std::fs::create_dir_all(&model_dir).ok();
        std::fs::write(model_dir.join("config.json"), r#"{"hidden_size": 128}"#).ok();
        std::fs::write(model_dir.join("tokenizer.json"), "{}").ok();
        std::fs::write(model_dir.join("model.safetensors"), "").ok();
        let service = VectorServiceImpl::new_with_config(tmp_dir.to_str().unwrap(), Some(vec![]));
        let req = Request::new(ListModelsRequest {});
        let resp = service.list_models(req).await.unwrap();
        assert_eq!(resp.into_inner().models.len(), 0, "空列表应跳过所有");
        std::fs::remove_dir_all(&tmp_dir).ok();
    }

    #[tokio::test]
    async fn test_new_with_config_filter() {
        // auto_load_models=Some(vec!["model_a"]) 应只加载 model_a
        let tmp_dir = std::env::temp_dir().join("config_test_filter");
        for name in &["model_a", "model_b", "model_c"] {
            let d = tmp_dir.join("candle").join(name);
            std::fs::create_dir_all(&d).ok();
            std::fs::write(d.join("config.json"), r#"{"hidden_size": 128}"#).ok();
            std::fs::write(d.join("tokenizer.json"), "{}").ok();
            std::fs::write(d.join("model.safetensors"), "").ok();
        }
        let service = VectorServiceImpl::new_with_config(
            tmp_dir.to_str().unwrap(),
            Some(vec!["model_a".to_string()]),
        );
        let req = Request::new(ListModelsRequest {});
        let resp = service.list_models(req).await.unwrap();
        let models = resp.into_inner().models;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_name, "model_a");
        std::fs::remove_dir_all(&tmp_dir).ok();
    }
}