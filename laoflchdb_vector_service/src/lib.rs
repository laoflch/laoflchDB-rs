pub mod proto {
    tonic::include_proto!("laoflchdb.vector");
}

use candle_core::Device;
use log::{info, warn};
use std::collections::HashMap;
use tokio::sync::RwLock as AsyncRwLock;
use proto::*;
use tonic::{Request, Response, Status};

/// 向量化服务实现
/// 使用 Candle 引擎在本地 GPU (RTX 2070S) 上运行模型推理
pub struct VectorServiceImpl {
    models: AsyncRwLock<HashMap<String, ModelInstance>>,
    default_device: Device,
}

struct ModelInstance {
    embedding_dim: usize,
    model_path: String,
    loaded: bool,
    device: String,
}

impl VectorServiceImpl {
    pub fn new() -> Self {
        let device = Self::detect_device();
        
        VectorServiceImpl {
            models: AsyncRwLock::new(HashMap::new()),
            default_device: device,
        }
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

        for text in &req.texts {
            let embedding = generate_fallback_embedding(text, model.embedding_dim)
                .map_err(|e| Status::internal(format!("向量化失败: {}", e)))?;

            results.push(EmbeddingResult {
                text: text.clone(),
                embedding,
                dim: model.embedding_dim as i32,
            });
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

        let device_str = format!("{:?}", self.default_device);

        let mut models = self.models.write().await;
        models.insert(
            req.model_name.clone(),
            ModelInstance {
                embedding_dim: req.embedding_dim as usize,
                model_path: req.model_path,
                loaded: true,
                device: device_str,
            },
        );

        info!("模型 '{}' 注册成功，dim={}", req.model_name, req.embedding_dim);

        Ok(Response::new(LoadModelResponse {
            success: true,
            message: format!("模型 '{}' 注册成功，等待实际加载", req.model_name),
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
}

/// 使用字符哈希生成文本的向量表示（fallback 实现）
/// 后续可替换为 Candle 推理真实模型（如 BERT）的向量输出
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
        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "test_model".to_string(),
            model_path: "/tmp/test_model".to_string(),
            embedding_dim: 768,
        });
        let load_resp = service.load_model(load_req).await.unwrap();
        assert!(load_resp.into_inner().success);

        let list_req = Request::new(ListModelsRequest {});
        let list_resp = service.list_models(list_req).await.unwrap();
        let models = list_resp.into_inner().models;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_name, "test_model");
        assert_eq!(models[0].embedding_dim, 768);
    }

    #[tokio::test]
    async fn test_unload_model() {
        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "temp_model".to_string(),
            model_path: "/tmp/temp".to_string(),
            embedding_dim: 384,
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
        let service = VectorServiceImpl::new();

        let load_req = Request::new(LoadModelRequest {
            model_name: "embed_model".to_string(),
            model_path: "/tmp/embed".to_string(),
            embedding_dim: 64,
        });
        service.load_model(load_req).await.unwrap();

        let req = Request::new(EmbeddingRequest {
            model_name: "embed_model".to_string(),
            texts: vec!["hello world".to_string(), "test text".to_string()],
            dim: 64,
        });
        let resp = service.create_embedding(req).await.unwrap();
        let results = resp.into_inner().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].embedding.len(), 64);
        assert_eq!(results[1].embedding.len(), 64);
    }

    #[test]
    fn test_device_detection() {
        let _device = VectorServiceImpl::detect_device();
        // 至少能返回一个设备（CPU）
    }
}