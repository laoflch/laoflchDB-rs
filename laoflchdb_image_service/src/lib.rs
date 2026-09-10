pub mod proto {
    tonic::include_proto!("laoflchdb.image_service");
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    body::Bytes,
    extract::{Json, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
};
use image::imageops::FilterType;
use laoflchdb_object_store_service::proto::object_store_service_server::ObjectStoreService;
use laoflchdb_object_store_service::proto::{
    CreateBucketRequest, DeleteObjectRequest, GetObjectRequest,
    ListObjectsRequest, PutObjectRequest,
};
use log::info;
use log::warn;
use snowflake_me::Snowflake;
use laoflchdb_embedding_service::proto::embedding_index_service_server::EmbeddingIndexService;
use laoflchdb_vector_service::proto::vector_service_server::VectorService;
use proto::image_service_server::ImageService;
use proto::*;
use tonic::{Request, Response, Status};

/// 默认 bucket 名称
const DEFAULT_BUCKET: &str = "images";

/// 元数据 key 前缀（用于在对象存储中存储图片元数据）
const IMAGE_META_PREFIX: &str = "__img_meta__";

/// 缩略图规格定义：(size 名称, 最大边长)
/// thumbnail: 128x128（缩略图）
/// small:     256x256（小图）
/// medium:    512x512（中图）
const THUMBNAIL_SIZES: &[(&str, u32)] = &[
    ("thumbnail", 128),
    ("small", 256),
    ("medium", 512),
];

/// 图片服务配置
#[derive(Debug, Clone)]
pub struct ImageServiceConfig {
    /// 是否启用
    pub enabled: bool,
    /// 默认 bucket 名称
    pub default_bucket: String,
    /// 图片重复检测距离阈值（Cosine 距离 < 此值视为重复，默认 0.0001）
    pub image_duplicate_distance: f32,
}

impl Default for ImageServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_bucket: DEFAULT_BUCKET.to_string(),
            image_duplicate_distance: 0.0001,
        }
    }
}

/// 图片服务实现
/// 基于 ObjectStoreService 提供图片上传（自动生成缩略图）和浏览功能
pub struct ImageServiceImpl {
    object_store: Arc<laoflchdb_object_store_service::ObjectStoreServiceImpl>,
    config: ImageServiceConfig,
    /// Snowflake ID 生成器，用于自动生成图片的唯一 key
    snowflake: Mutex<Snowflake>,
    /// 向量服务（可选，用于自动向量索引）
    #[cfg(feature = "auto_index")]
    vector_service: Option<Arc<laoflchdb_vector_service::VectorServiceImpl>>,
    /// 嵌入索引服务（可选，用于自动向量索引）
    #[cfg(feature = "auto_index")]
    embedding_service: Option<Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>>,
    /// 全文索引写入接口（可选，用于保存图片分类结果）
    #[cfg(feature = "auto_index")]
    index_sink: Option<Arc<dyn ImageIndexSink>>,
}

/// 全文索引写入接口抽象
/// 由主工程注入，用于将图片分类结果保存到全文索引（如 tantivy）
#[tonic::async_trait]
pub trait ImageIndexSink: Send + Sync + 'static {
    /// 删除索引（索引不存在时返回 Ok）
    async fn drop_index(&self, index_name: &str) -> Result<(), String>;
    /// 创建索引，fields 为 (序号, 字段名, 字段类型码(0=字符串,1=整数), 注释)
    async fn create_index(
        &self,
        index_name: &str,
        fields: &[(u32, &str, u8, Option<&str>)],
    ) -> Result<(), String>;
    /// 写入一条文档
    async fn add_document(
        &self,
        index_name: &str,
        doc_id: &str,
        fields: std::collections::HashMap<String, String>,
    ) -> Result<(), String>;
}

impl ImageServiceImpl {
    /// 创建图片服务
    /// object_store: 已初始化的对象存储服务实例
    #[allow(unused_variables)]
    pub fn new(
        object_store: Arc<laoflchdb_object_store_service::ObjectStoreServiceImpl>,
        config: ImageServiceConfig,
        vector_service: Option<Arc<laoflchdb_vector_service::VectorServiceImpl>>,
        embedding_service: Option<Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>>,
        index_sink: Option<Arc<dyn ImageIndexSink>>,
    ) -> Self {
        // 优先用默认配置（基于 IP 推导 machine_id）；失败时回退到 machine_id=0, data_center_id=0
        let snowflake = Snowflake::new().unwrap_or_else(|_| {
            log::warn!("Snowflake 默认初始化失败，回退到 machine_id=0, data_center_id=0");
            Snowflake::builder()
                .machine_id(&|| Ok(0u16))
                .data_center_id(&|| Ok(0u16))
                .finalize()
                .expect("Snowflake with machine_id=0, data_center_id=0 must succeed")
        });
        info!(
            "ImageService 初始化完成: default_bucket='{}'",
            config.default_bucket
        );
        Self {
            object_store,
            config,
            snowflake: Mutex::new(snowflake),
            #[cfg(feature = "auto_index")]
            vector_service,
            #[cfg(feature = "auto_index")]
            embedding_service,
            #[cfg(feature = "auto_index")]
            index_sink,
        }
    }

    /// 生成基于 Snowflake 算法的唯一图片 key
    /// Snowflake ID 为 64 位整数，保证分布式唯一且单调递增
    /// 失败时回退到当前毫秒级时间戳
    fn generate_image_key(&self) -> String {
        let id = match self.snowflake.lock() {
            Ok(guard) => guard.next_id().unwrap_or_else(|_| {
                log::warn!("Snowflake next_id 失败，回退到毫秒时间戳");
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }),
            Err(_) => {
                log::warn!("Snowflake mutex 锁定失败，回退到毫秒时间戳");
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }
        };
        format!("{}", id)
    }

    /// 解析 bucket，若为空则使用默认 bucket
    fn resolve_bucket(&self, bucket: &str) -> String {
        if bucket.is_empty() {
            self.config.default_bucket.clone()
        } else {
            bucket.to_string()
        }
    }

    /// 返回默认 bucket 名称
    pub fn default_bucket(&self) -> String {
        self.config.default_bucket.clone()
    }

    /// 确保 bucket 存在
    async fn ensure_bucket(&self, bucket: &str) -> Result<(), Status> {
        let req = Request::new(CreateBucketRequest {
            bucket: bucket.to_string(),
        });
        self.object_store.create_bucket(req).await?;
        Ok(())
    }

    /// 生成缩略图的 key
    /// 规则: {original_key}__{size_name}.jpg
    /// 统一使用 JPEG 编码以节省空间
    fn thumbnail_key(original_key: &str, size_name: &str) -> String {
        format!("{}__{}.jpg", original_key, size_name)
    }

    /// 生成元数据的 key
    fn metadata_key(image_key: &str) -> String {
        format!("{}{}", IMAGE_META_PREFIX, image_key)
    }

    /// 获取当前 Unix 时间戳（秒）
    fn now_string() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("{}", now.as_secs())
    }

    /// 生成缩略图并返回 (size_name, thumbnail_key, thumbnail_bytes, width, height)
    fn generate_thumbnail(
        img: &image::DynamicImage,
        max_size: u32,
        original_key: &str,
        size_name: &str,
    ) -> Result<(String, String, Vec<u8>, u32, u32), Status> {
        use image::ImageFormat;

        // 按最大边长等比缩放（thumbnail 规格使用 cover 模式裁剪为正方形，其他用 contain 模式）
        let (thumb_img, width, height) = if size_name == "thumbnail" {
            // thumbnail: 裁剪为正方形（cover 模式）
            let resized = img.resize_to_fill(max_size, max_size, FilterType::Lanczos3);
            let w = resized.width();
            let h = resized.height();
            (resized, w, h)
        } else {
            // small/medium: 等比缩放，不超过 max_size（contain 模式）
            let resized = img.resize(max_size, max_size, FilterType::Lanczos3);
            let w = resized.width();
            let h = resized.height();
            (resized, w, h)
        };

        // 编码为 JPEG
        let mut buf = std::io::Cursor::new(Vec::new());
        thumb_img
            .write_to(&mut buf, ImageFormat::Jpeg)
            .map_err(|e| Status::internal(format!("Failed to encode thumbnail: {}", e)))?;

        let thumbnail_key = Self::thumbnail_key(original_key, size_name);
        Ok((
            size_name.to_string(),
            thumbnail_key,
            buf.into_inner(),
            width,
            height,
        ))
    }

    /// 从对象存储获取图片元数据
    async fn get_metadata_from_store(
        &self,
        bucket: &str,
        image_key: &str,
    ) -> Result<Option<ImageMetadata>, Status> {
        let meta_key = Self::metadata_key(image_key);
        let get_req = Request::new(GetObjectRequest {
            bucket: bucket.to_string(),
            key: meta_key,
        });
        let resp = self.object_store.get_object(get_req).await?;
        let resp = resp.into_inner();
        if !resp.success || resp.data.is_empty() {
            return Ok(None);
        }
        let meta: serde_json::Value =
            serde_json::from_slice(&resp.data).map_err(|e| Status::internal(format!("Failed to parse image metadata: {}", e)))?;
        Ok(Some(Self::parse_metadata(&meta)))
    }

    /// 从 JSON 解析图片元数据
    fn parse_metadata(meta: &serde_json::Value) -> ImageMetadata {
        let mut thumbnails = HashMap::new();
        if let Some(thumbs) = meta.get("thumbnails").and_then(|v| v.as_object()) {
            for (k, v) in thumbs {
                if let Some(s) = v.as_str() {
                    thumbnails.insert(k.clone(), s.to_string());
                }
            }
        }
        let mut user_metadata = HashMap::new();
        if let Some(um) = meta.get("user_metadata").and_then(|v| v.as_object()) {
            for (k, v) in um {
                if let Some(s) = v.as_str() {
                    user_metadata.insert(k.clone(), s.to_string());
                }
            }
        }
        ImageMetadata {
            key: meta.get("key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            content_type: meta.get("content_type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            content_length: meta.get("content_length").and_then(|v| v.as_i64()).unwrap_or(0),
            width: meta.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            height: meta.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            etag: meta.get("etag").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            last_modified: meta.get("last_modified").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            thumbnails,
            user_metadata,
            format: meta.get("format").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            name: meta.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            is_indexed: meta.get("is_indexed").and_then(|v| v.as_bool()).unwrap_or(false),
            index_model: meta.get("index_model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        }
    }

    /// 保存图片元数据到对象存储
    async fn save_metadata_to_store(
        &self,
        bucket: &str,
        image_key: &str,
        metadata: &ImageMetadata,
    ) -> Result<(), Status> {
        let meta_json = serde_json::json!({
            "key": metadata.key,
            "content_type": metadata.content_type,
            "content_length": metadata.content_length,
            "width": metadata.width,
            "height": metadata.height,
            "etag": metadata.etag,
            "last_modified": metadata.last_modified,
            "thumbnails": metadata.thumbnails,
            "user_metadata": metadata.user_metadata,
            "format": metadata.format,
            "name": metadata.name,
            "is_indexed": metadata.is_indexed,
            "index_model": metadata.index_model,
        });
        let meta_key = Self::metadata_key(image_key);
        let put_req = Request::new(PutObjectRequest {
            bucket: bucket.to_string(),
            key: meta_key,
            data: meta_json.to_string().into_bytes(),
            content_type: "application/json".to_string(),
            metadata: HashMap::new(),
        });
        self.object_store.put_object(put_req).await?;
        Ok(())
    }
}

/// 允许通过 `Arc<ImageServiceImpl>` 直接作为 gRPC 服务注册
#[tonic::async_trait]
impl proto::image_service_server::ImageService for std::sync::Arc<ImageServiceImpl> {
    async fn upload_image(
        &self,
        request: tonic::Request<UploadImageRequest>,
    ) -> std::result::Result<tonic::Response<UploadImageResponse>, tonic::Status> {
        self.as_ref().upload_image(request).await
    }

    async fn get_image(
        &self,
        request: tonic::Request<GetImageRequest>,
    ) -> std::result::Result<tonic::Response<GetImageResponse>, tonic::Status> {
        self.as_ref().get_image(request).await
    }

    async fn get_thumbnail(
        &self,
        request: tonic::Request<GetThumbnailRequest>,
    ) -> std::result::Result<tonic::Response<GetThumbnailResponse>, tonic::Status> {
        self.as_ref().get_thumbnail(request).await
    }

    async fn get_image_metadata(
        &self,
        request: tonic::Request<GetImageMetadataRequest>,
    ) -> std::result::Result<tonic::Response<GetImageMetadataResponse>, tonic::Status> {
        self.as_ref().get_image_metadata(request).await
    }

    async fn list_images(
        &self,
        request: tonic::Request<ListImagesRequest>,
    ) -> std::result::Result<tonic::Response<ListImagesResponse>, tonic::Status> {
        self.as_ref().list_images(request).await
    }

    async fn delete_image(
        &self,
        request: tonic::Request<DeleteImageRequest>,
    ) -> std::result::Result<tonic::Response<DeleteImageResponse>, tonic::Status> {
        self.as_ref().delete_image(request).await
    }

    async fn search_images_by_text(
        &self,
        request: tonic::Request<SearchImagesByTextRequest>,
    ) -> std::result::Result<tonic::Response<SearchImagesByTextResponse>, tonic::Status> {
        self.as_ref().search_images_by_text(request).await
    }

    async fn search_images_by_image(
        &self,
        request: tonic::Request<SearchImagesByImageRequest>,
    ) -> std::result::Result<tonic::Response<SearchImagesByImageResponse>, tonic::Status> {
        self.as_ref().search_images_by_image(request).await
    }

    async fn update_image_metadata(
        &self,
        request: tonic::Request<UpdateImageMetadataRequest>,
    ) -> std::result::Result<tonic::Response<UpdateImageMetadataResponse>, tonic::Status> {
        self.as_ref().update_image_metadata(request).await
    }

    async fn index_image(
        &self,
        request: tonic::Request<IndexImageRequest>,
    ) -> std::result::Result<tonic::Response<IndexImageResponse>, tonic::Status> {
        self.as_ref().index_image(request).await
    }

    async fn classify_images(
        &self,
        request: tonic::Request<ClassifyImagesRequest>,
    ) -> std::result::Result<tonic::Response<ClassifyImagesResponse>, tonic::Status> {
        self.as_ref().classify_images(request).await
    }

    async fn upload_image_stream(
        &self,
        request: tonic::Request<tonic::Streaming<UploadImageChunk>>,
    ) -> std::result::Result<tonic::Response<UploadImageResponse>, tonic::Status> {
        self.as_ref().upload_image_stream(request).await
    }
}

// ── 内部辅助方法（非 trait 方法） ──
impl ImageServiceImpl {
    /// 生成图片 embedding（单次 GPU 推理）
    #[cfg(feature = "auto_index")]
    async fn generate_image_embedding(
        &self,
        image_data: &[u8],
        model_name: &str,
        index_name: &str,
    ) -> Result<(Vec<f32>, i32), Box<dyn std::error::Error + Send + Sync>> {
        let vector_svc = self.vector_service.as_ref().ok_or("向量服务未启用")?;
        let embedding_svc = self.embedding_service.as_ref().ok_or("嵌入索引服务未启用")?;

        // 1. 获取索引的维度
        let index_dim = {
            use laoflchdb_embedding_service::proto::GetIndexInfoRequest;
            let info_req = tonic::Request::new(GetIndexInfoRequest {
                index_name: index_name.to_string(),
            });
            let info_resp = embedding_svc.get_index_info(info_req).await
                .map_err(|e| format!("获取索引信息失败: {}", e))?;
            let info = info_resp.into_inner();
            if info.success {
                info.stats.map(|s| s.dim as i32).unwrap_or(512)
            } else {
                512
            }
        };

        // 2. 调用向量服务生成嵌入向量（仅一次 GPU 推理）
        let model = if model_name.is_empty() { "jina-clip-v2" } else { model_name };
        use laoflchdb_vector_service::proto::EmbeddingRequest;
        let emb_req = tonic::Request::new(EmbeddingRequest {
            model_name: model.to_string(),
            texts: vec![],
            dim: index_dim,
            images: vec![image_data.to_vec()],
            image_keys: vec![],
            image_bucket: String::new(),
        });
        let emb_resp = vector_svc.create_embedding(emb_req).await
            .map_err(|e| format!("向量化失败: {}", e))?;
        let emb = emb_resp.into_inner();
        if !emb.success {
            return Err(format!("向量化失败: {}", emb.message).into());
        }
        let embedding = emb.results.first()
            .ok_or("向量化结果为空")?
            .embedding.clone();

        Ok((embedding, index_dim))
    }

    /// 使用已有 embedding 搜索重复图片
    #[cfg(feature = "auto_index")]
    async fn check_existing_image_with_embedding(
        &self,
        embedding: &[f32],
        index_dim: i32,
    ) -> Result<Vec<(String, i32)>, Box<dyn std::error::Error + Send + Sync>> {
        let embedding_svc = match self.embedding_service.as_ref() {
            Some(svc) => svc,
            None => return Ok(vec![]),
        };

        use laoflchdb_embedding_service::proto::SearchEmbeddingRequest;
        let search_req = tonic::Request::new(SearchEmbeddingRequest {
            query_embedding: embedding.to_vec(),
            top_k: 100,
            index_name: "image".to_string(),
            field_filters: Default::default(),
            filter_multiplier: 0.0,
            max_filter_iterations: 0,
            max_distance: 0.0,
        });
        let search_resp = embedding_svc.search_embedding(search_req).await
            .map_err(|e| format!("搜索向量索引失败: {}", e))?;
        let search = search_resp.into_inner();
        let mut duplicates: Vec<(String, i32)> = Vec::new();
        if search.success {
            for result in &search.results {
                if result.distance < self.config.image_duplicate_distance {
                    let existing_key = result.id.to_string();
                    duplicates.push((existing_key, index_dim));
                }
            }
            if !duplicates.is_empty() {
                info!("去重检查: 发现 {} 张重复图片", duplicates.len());
            }
        }

        Ok(duplicates)
    }

    /// 使用已有 embedding 插入索引（不再重复向量化）
    #[cfg(feature = "auto_index")]
    async fn index_image_with_embedding(
        &self,
        embedding: Vec<f32>,
        key: &str,
        index_name: &str,
    ) -> Result<(String, i32), Box<dyn std::error::Error + Send + Sync>> {
        let embedding_svc = self.embedding_service.as_ref().ok_or("嵌入索引服务未启用")?;

        use laoflchdb_embedding_service::proto::InsertEmbeddingRequest;
        let id = key.parse::<u64>().map_err(|_| {
            format!("图片 key 不是有效数字 ID: {}", key)
        })?;
        let dim = embedding.len() as i32;
        let ins_req = tonic::Request::new(InsertEmbeddingRequest {
            id,
            index_name: index_name.to_string(),
            embedding,
            fields: Default::default(),
        });
        let ins_resp = embedding_svc.insert_embedding(ins_req).await
            .map_err(|e| format!("索引请求失败: {}", e))?;
        let ins = ins_resp.into_inner();
        if !ins.success {
            return Err(format!("索引失败: {}", ins.message).into());
        }

        Ok((key.to_string(), dim))
    }

    /// 生成文本向量（用于文搜图）
    #[cfg(feature = "auto_index")]
    async fn generate_text_embedding(
        &self,
        text: &str,
        model_name: &str,
        index_name: &str,
    ) -> Result<(Vec<f32>, i32), Box<dyn std::error::Error + Send + Sync>> {
        let vector_svc = self.vector_service.as_ref().ok_or("向量服务未启用")?;
        let embedding_svc = self.embedding_service.as_ref().ok_or("嵌入索引服务未启用")?;

        // 1. 获取索引的维度
        let index_dim = {
            use laoflchdb_embedding_service::proto::GetIndexInfoRequest;
            let info_req = tonic::Request::new(GetIndexInfoRequest {
                index_name: index_name.to_string(),
            });
            let info_resp = embedding_svc.get_index_info(info_req).await
                .map_err(|e| format!("获取索引信息失败: {}", e))?;
            let info = info_resp.into_inner();
            if info.success {
                info.stats.map(|s| s.dim as i32).unwrap_or(512)
            } else {
                512
            }
        };

        // 2. 调用向量服务生成文本向量
        let model = if model_name.is_empty() { "jina-clip-v2" } else { model_name };
        use laoflchdb_vector_service::proto::EmbeddingRequest;
        let emb_req = tonic::Request::new(EmbeddingRequest {
            model_name: model.to_string(),
            texts: vec![text.to_string()],
            dim: index_dim,
            images: vec![],
            image_keys: vec![],
            image_bucket: String::new(),
        });
        let emb_resp = vector_svc.create_embedding(emb_req).await
            .map_err(|e| format!("文本向量化失败: {}", e))?;
        let emb = emb_resp.into_inner();
        if !emb.success {
            return Err(format!("文本向量化失败: {}", emb.message).into());
        }
        let embedding = emb.results.first()
            .ok_or("向量化结果为空")?
            .embedding.clone();

        Ok((embedding, index_dim))
    }

    /// 用向量搜索相似图片（公共逻辑，文搜图和图搜图都用）
    #[cfg(feature = "auto_index")]
    async fn search_similar_images(
        &self,
        embedding: Vec<f32>,
        top_k: i32,
        index_name: &str,
        bucket: &str,
    ) -> Result<Vec<(ImageMetadata, f32)>, Box<dyn std::error::Error + Send + Sync>> {
        let embedding_svc = self.embedding_service.as_ref().ok_or("嵌入索引服务未启用")?;

        use laoflchdb_embedding_service::proto::SearchEmbeddingRequest;
        let search_req = tonic::Request::new(SearchEmbeddingRequest {
            query_embedding: embedding,
            top_k: if top_k <= 0 { 10 } else { top_k },
            index_name: index_name.to_string(),
            field_filters: Default::default(),
            filter_multiplier: 0.0,
            max_filter_iterations: 0,
            max_distance: 0.0,
        });
        let search_resp = embedding_svc.search_embedding(search_req).await
            .map_err(|e| format!("搜索向量索引失败: {}", e))?;
        let search = search_resp.into_inner();
        if !search.success {
            return Err(format!("向量搜索失败: {}", search.message).into());
        }

        let mut results: Vec<(ImageMetadata, f32)> = Vec::new();
        for sr in &search.results {
            let key = sr.id.to_string();
            if let Ok(Some(meta)) = self.get_metadata_from_store(bucket, &key).await {
                results.push((meta, sr.distance));
            }
        }

        Ok(results)
    }
}

#[tonic::async_trait]
impl ImageService for ImageServiceImpl {
    async fn upload_image(
        &self,
        request: Request<UploadImageRequest>,
    ) -> Result<Response<UploadImageResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);
        self.ensure_bucket(&bucket).await?;

        // ── 生成 embedding（单次 GPU 推理）+ 去重检查 ──
        #[cfg(feature = "auto_index")]
        let (duplicate_info, cached_embedding, cached_dim) = if req.auto_index {
            // 先生成 embedding（仅一次 GPU 推理）
            match self.generate_image_embedding(&req.data, &req.auto_index_model, "image").await {
                Ok((embedding, dim)) => {
                    // 用同一 embedding 做去重检查
                    let dups = self.check_existing_image_with_embedding(&embedding, dim).await
                        .map_err(|e| {
                            log::warn!("图片去重检查失败: {}", e);
                            Status::internal(format!("图片去重检查失败: {}", e))
                        })?;
                    (dups, Some(embedding), dim)
                }
                Err(e) => {
                    log::warn!("图片向量化失败，跳过去重检查: {}", e);
                    (vec![], None, 0)
                }
            }
        } else {
            (vec![], None, 0)
        };
        #[cfg(not(feature = "auto_index"))]
        let (duplicate_info, cached_embedding, cached_dim): (Vec<(String, i32)>, Option<Vec<f32>>, i32) = (vec![], None, 0);

        if !duplicate_info.is_empty() {
            let first_key = &duplicate_info[0].0;
            let first_dim = duplicate_info[0].1;
            match req.duplicate_action.as_str() {
                "overwrite" => {
                    info!("图片已存在，用户选择覆盖: 共 {} 张重复图片，全部删除", duplicate_info.len());
                    // 删除所有重复图片和索引
                    for (existing_key, _) in &duplicate_info {
                        #[cfg(feature = "auto_index")]
                        if let Some(embedding_svc) = &self.embedding_service {
                            if let Ok(id) = existing_key.parse::<u64>() {
                                use laoflchdb_embedding_service::proto::DeleteEmbeddingRequest;
                                let del_emb_req = tonic::Request::new(DeleteEmbeddingRequest {
                                    id,
                                    index_name: "image".to_string(),
                                });
                                let _ = embedding_svc.delete_embedding(del_emb_req).await;
                            }
                        }
                        // 删除原图
                        let del_req_orig = Request::new(DeleteObjectRequest {
                            bucket: bucket.clone(),
                            key: existing_key.clone(),
                        });
                        let _ = self.object_store.delete_object(del_req_orig).await;
                        // 删除缩略图
                        if let Ok(Some(meta)) = self.get_metadata_from_store(&bucket, existing_key).await {
                            for (_, thumb_key) in &meta.thumbnails {
                                let del_req = Request::new(DeleteObjectRequest {
                                    bucket: bucket.clone(),
                                    key: thumb_key.clone(),
                                });
                                let _ = self.object_store.delete_object(del_req).await;
                            }
                        }
                        // 删除元数据
                        let meta_key = Self::metadata_key(existing_key);
                        let del_req = Request::new(DeleteObjectRequest {
                            bucket: bucket.clone(),
                            key: meta_key.clone(),
                        });
                        let _ = self.object_store.delete_object(del_req).await;
                    }
                    // 覆盖模式强制生成新 key
                }
                "new" => {
                    info!("图片已存在，用户选择新增: key='{}'", first_key);
                    // 新增：继续执行上传流程，生成新 key
                }
                "skip" => {
                    info!("图片已存在，用户选择跳过: key='{}'", first_key);
                    return Ok(Response::new(UploadImageResponse {
                        success: true,
                        message: "Image already exists, skipped storage".to_string(),
                        key: first_key.clone(),
                        etag: String::new(),
                        metadata: None,
                        auto_indexed: true,
                        embedding_id: first_key.clone(),
                        embedding_dim: first_dim,
                        duplicate_detected: true,
                        existing_key: first_key.clone(),
                    }));
                }
                _ => {
                    // 默认行为：返回前端确认
                    info!("图片已存在，返回前端确认: key='{}'", first_key);
                    return Ok(Response::new(UploadImageResponse {
                        success: true,
                        message: "Duplicate image detected, please confirm".to_string(),
                        key: String::new(),
                        etag: String::new(),
                        metadata: None,
                        auto_indexed: false,
                        embedding_id: String::new(),
                        embedding_dim: 0,
                        duplicate_detected: true,
                        existing_key: first_key.clone(),
                    }));
                }
            }
        }

        // 生成图片 key（覆盖模式和新增模式都生成新 key）
        let has_duplicate = !duplicate_info.is_empty();
        let is_overwrite = has_duplicate && req.duplicate_action == "overwrite";
        let is_new = has_duplicate && req.duplicate_action == "new";
        let image_key = if is_overwrite || is_new {
            self.generate_image_key()
        } else if req.key.is_empty() {
            self.generate_image_key()
        } else {
            req.key.clone()
        };

        // 解码图片
        let img = image::load_from_memory(&req.data)
            .map_err(|e| Status::invalid_argument(format!("Failed to decode image: {}", e)))?;

        let width = img.width() as i32;
        let height = img.height() as i32;
        let format_str = match image::guess_format(&req.data) {
            Ok(fmt) => format!("{:?}", fmt),
            Err(_) => "UNKNOWN".to_string(),
        };

        // 上传原图
        let put_req = Request::new(PutObjectRequest {
            bucket: bucket.clone(),
            key: image_key.clone(),
            data: req.data.clone(),
            content_type: req.content_type.clone(),
            metadata: req.metadata.clone(),
        });
        let put_resp = self.object_store.put_object(put_req).await?.into_inner();
        if !put_resp.success {
            return Ok(Response::new(UploadImageResponse {
                success: false,
                message: "Failed to upload original image".to_string(),
                key: image_key,
                etag: String::new(),
                metadata: None,
                auto_indexed: false,
                embedding_id: String::new(),
                embedding_dim: 0,
                duplicate_detected: false,
                existing_key: String::new(),
            }));
        }
        let etag = put_resp.etag;

        // 生成并上传三种缩略图
        let mut thumbnails: HashMap<String, String> = HashMap::new();
        for (size_name, max_size) in THUMBNAIL_SIZES {
            let (name, thumb_key, thumb_data, thumb_w, thumb_h) =
                Self::generate_thumbnail(&img, *max_size, &image_key, size_name)?;

            let thumb_put_req = Request::new(PutObjectRequest {
                bucket: bucket.clone(),
                key: thumb_key.clone(),
                data: thumb_data,
                content_type: "image/jpeg".to_string(),
                metadata: HashMap::new(),
            });
            let thumb_resp = self.object_store.put_object(thumb_put_req).await?;
            if !thumb_resp.into_inner().success {
                log::warn!("Failed to upload thumbnail '{}': {}", name, size_name);
            }
            thumbnails.insert(name, thumb_key);
            let _ = (thumb_w, thumb_h); // 缩略图尺寸不存入主元数据
        }

        // 获取当前时间戳
        let now = Self::now_string();

        // 构建并存储图片元数据
        let mut metadata = ImageMetadata {
            key: image_key.clone(),
            content_type: req.content_type.clone(),
            content_length: req.data.len() as i64,
            width,
            height,
            etag: etag.clone(),
            last_modified: now.clone(),
            thumbnails: thumbnails.clone(),
            user_metadata: req.metadata.clone(),
            format: format_str,
            name: req.name.clone(),
            is_indexed: false,
            index_model: String::new(),
        };

        self.save_metadata_to_store(&bucket, &image_key, &metadata).await?;

        info!(
            "图片上传成功: bucket='{}', key='{}', size={}x{}, format={}",
            bucket, image_key, width, height, metadata.format
        );

        // ── 自动向量索引（复用已生成的 embedding，不再重复向量化） ──
        let mut auto_indexed = false;
        let mut embedding_id = String::new();
        let mut embedding_dim = 0i32;

        #[cfg(feature = "auto_index")]
        if req.auto_index {
            if let Some(embedding) = cached_embedding {
                match self.index_image_with_embedding(embedding, &image_key, "image").await {
                    Ok((eid, edim)) => {
                        auto_indexed = true;
                        embedding_id = eid.clone();
                        embedding_dim = edim;
                        // 更新元数据中的 is_indexed 标志
                        metadata.is_indexed = true;
                        metadata.index_model = if req.auto_index_model.is_empty() {
                            "jina-clip-v2".to_string()
                        } else {
                            req.auto_index_model.clone()
                        };
                        if let Err(e) = self.save_metadata_to_store(&bucket, &image_key, &metadata).await {
                            log::warn!("更新图片元数据 is_indexed 失败: {}", e);
                        }
                        info!("图片自动向量索引成功: key='{}', id='{}'", image_key, embedding_id);
                    }
                    Err(e) => {
                        log::warn!("图片自动向量索引失败: key='{}', error={}", image_key, e);
                    }
                }
            } else {
                log::warn!("图片自动向量索引跳过: 无缓存的 embedding（向量化此前已失败）");
            }
            let _ = cached_dim;
        }

        Ok(Response::new(UploadImageResponse {
            success: true,
            message: "OK".to_string(),
            key: image_key,
            etag,
            metadata: Some(metadata),
            auto_indexed,
            embedding_id,
            embedding_dim,
            duplicate_detected: false,
            existing_key: String::new(),
        }))
    }

    async fn get_image(
        &self,
        request: Request<GetImageRequest>,
    ) -> Result<Response<GetImageResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);

        let get_req = Request::new(GetObjectRequest {
            bucket: bucket.clone(),
            key: req.key.clone(),
        });
        let resp = self.object_store.get_object(get_req).await?.into_inner();
        if !resp.success {
            return Ok(Response::new(GetImageResponse {
                success: false,
                message: "Image not found".to_string(),
                data: Vec::new(),
                content_type: String::new(),
                content_length: 0,
                etag: String::new(),
            }));
        }

        Ok(Response::new(GetImageResponse {
            success: true,
            message: "OK".to_string(),
            data: resp.data,
            content_type: resp.content_type,
            content_length: resp.content_length,
            etag: resp.etag,
        }))
    }

    async fn get_thumbnail(
        &self,
        request: Request<GetThumbnailRequest>,
    ) -> Result<Response<GetThumbnailResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);

        // 验证 size 参数
        let valid_size = THUMBNAIL_SIZES
            .iter()
            .any(|(name, _)| *name == req.size);
        if !valid_size {
            return Err(Status::invalid_argument(format!(
                "Invalid thumbnail size '{}'. Valid sizes: thumbnail, small, medium",
                req.size
            )));
        }

        // 先获取元数据，找到缩略图 key
        let metadata = self
            .get_metadata_from_store(&bucket, &req.key)
            .await?
            .ok_or_else(|| Status::not_found(format!("Image metadata '{}' not found", req.key)))?;

        let thumb_key = metadata
            .thumbnails
            .get(&req.size)
            .ok_or_else(|| Status::not_found(format!("Thumbnail '{}' for image '{}' not found", req.size, req.key)))?;

        // 获取缩略图数据
        let get_req = Request::new(GetObjectRequest {
            bucket: bucket.clone(),
            key: thumb_key.clone(),
        });
        let resp = self.object_store.get_object(get_req).await?.into_inner();
        if !resp.success {
            return Ok(Response::new(GetThumbnailResponse {
                success: false,
                message: "Thumbnail not found".to_string(),
                data: Vec::new(),
                content_type: String::new(),
                content_length: 0,
                width: 0,
                height: 0,
            }));
        }

        // 从缩略图数据中读取尺寸
        let (width, height) = match image::load_from_memory(&resp.data) {
            Ok(img) => (img.width() as i32, img.height() as i32),
            Err(_) => (0, 0),
        };

        Ok(Response::new(GetThumbnailResponse {
            success: true,
            message: "OK".to_string(),
            data: resp.data,
            content_type: resp.content_type,
            content_length: resp.content_length,
            width,
            height,
        }))
    }

    async fn get_image_metadata(
        &self,
        request: Request<GetImageMetadataRequest>,
    ) -> Result<Response<GetImageMetadataResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);

        let metadata = self
            .get_metadata_from_store(&bucket, &req.key)
            .await?
            .ok_or_else(|| Status::not_found(format!("Image '{}' not found", req.key)))?;

        Ok(Response::new(GetImageMetadataResponse {
            success: true,
            message: "OK".to_string(),
            metadata: Some(metadata),
        }))
    }

    async fn list_images(
        &self,
        request: Request<ListImagesRequest>,
    ) -> Result<Response<ListImagesResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);
        let max_keys = if req.max_keys <= 0 { 100 } else { req.max_keys as usize };
        let reverse = req.sort_order == "desc";

        // 列出所有元数据 key
        let meta_prefix = format!("{}{}", IMAGE_META_PREFIX, req.prefix);
        let list_req = Request::new(ListObjectsRequest {
            bucket: bucket.clone(),
            prefix: meta_prefix,
            delimiter: String::new(),
            max_keys: max_keys as i32,
            marker: req.marker.clone(),
            reverse,
        });
        let resp = self.object_store.list_objects(list_req).await?.into_inner();
        if !resp.success {
            return Ok(Response::new(ListImagesResponse {
                success: false,
                message: resp.message,
                bucket: bucket.clone(),
                images: Vec::new(),
                is_truncated: false,
                next_marker: String::new(),
            }));
        }

        // 获取每个图片的元数据
        let mut images = Vec::new();
        for obj in &resp.objects {
            // 从 key 中提取图片 key（去掉 __img_meta__ 前缀）
            let image_key = obj.key.strip_prefix(IMAGE_META_PREFIX).unwrap_or(&obj.key);
            if let Some(Some(meta)) = self.get_metadata_from_store(&bucket, image_key).await.ok() {
                images.push(meta);
            }
        }

        Ok(Response::new(ListImagesResponse {
            success: true,
            message: "OK".to_string(),
            bucket,
            images,
            is_truncated: resp.is_truncated,
            next_marker: resp.next_marker,
        }))
    }

    async fn search_images_by_text(
        &self,
        request: Request<SearchImagesByTextRequest>,
    ) -> Result<Response<SearchImagesByTextResponse>, Status> {
        #[cfg(feature = "auto_index")]
        {
            let req = request.into_inner();
            let bucket = self.resolve_bucket(&req.bucket);
            let index_name = if req.index_name.is_empty() {
                "image".to_string()
            } else {
                req.index_name.clone()
            };

            let (embedding, _dim) = self.generate_text_embedding(
                &req.text,
                &req.model_name,
                &index_name,
            ).await.map_err(|e| Status::internal(format!("文本向量化失败: {}", e)))?;

            let results = self.search_similar_images(
                embedding,
                req.top_k,
                &index_name,
                &bucket,
            ).await.map_err(|e| Status::internal(format!("图片搜索失败: {}", e)))?;

            let image_results: Vec<ImageSearchResult> = results
                .into_iter()
                .map(|(meta, score)| ImageSearchResult {
                    metadata: Some(meta),
                    score,
                })
                .collect();

            Ok(Response::new(SearchImagesByTextResponse {
                success: true,
                message: "OK".to_string(),
                results: image_results,
            }))
        }
        #[cfg(not(feature = "auto_index"))]
        {
            let _ = request;
            Ok(Response::new(SearchImagesByTextResponse {
                success: false,
                message: "auto_index feature 未启用，文搜图不可用".to_string(),
                results: vec![],
            }))
        }
    }

    async fn search_images_by_image(
        &self,
        request: Request<SearchImagesByImageRequest>,
    ) -> Result<Response<SearchImagesByImageResponse>, Status> {
        #[cfg(feature = "auto_index")]
        {
            let req = request.into_inner();
            let bucket = self.resolve_bucket(&req.bucket);
            let index_name = if req.index_name.is_empty() {
                "image".to_string()
            } else {
                req.index_name.clone()
            };

            let (embedding, _dim) = self.generate_image_embedding(
                &req.image_data,
                &req.model_name,
                &index_name,
            ).await.map_err(|e| Status::internal(format!("图片向量化失败: {}", e)))?;

            let results = self.search_similar_images(
                embedding,
                req.top_k,
                &index_name,
                &bucket,
            ).await.map_err(|e| Status::internal(format!("图片搜索失败: {}", e)))?;

            let image_results: Vec<ImageSearchResult> = results
                .into_iter()
                .map(|(meta, score)| ImageSearchResult {
                    metadata: Some(meta),
                    score,
                })
                .collect();

            Ok(Response::new(SearchImagesByImageResponse {
                success: true,
                message: "OK".to_string(),
                results: image_results,
            }))
        }
        #[cfg(not(feature = "auto_index"))]
        {
            let _ = request;
            Ok(Response::new(SearchImagesByImageResponse {
                success: false,
                message: "auto_index feature 未启用，图搜图不可用".to_string(),
                results: vec![],
            }))
        }
    }

    async fn update_image_metadata(
        &self,
        request: Request<UpdateImageMetadataRequest>,
    ) -> Result<Response<UpdateImageMetadataResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);

        // 获取现有元数据
        let mut meta = match self.get_metadata_from_store(&bucket, &req.key).await? {
            Some(m) => m,
            None => {
                return Ok(Response::new(UpdateImageMetadataResponse {
                    success: false,
                    message: "图片不存在".to_string(),
                    metadata: None,
                }));
            }
        };

        // 更新名称
        if !req.name.is_empty() {
            meta.name = req.name;
        }

        // 更新/新增用户自定义 metadata
        for (k, v) in req.user_metadata {
            meta.user_metadata.insert(k, v);
        }

        // 删除用户自定义 metadata key
        for k in &req.delete_user_metadata_keys {
            meta.user_metadata.remove(k);
        }

        // 更新 last_modified
        meta.last_modified = Self::now_string();

        // 保存
        self.save_metadata_to_store(&bucket, &req.key, &meta).await?;

        Ok(Response::new(UpdateImageMetadataResponse {
            success: true,
            message: "OK".to_string(),
            metadata: Some(meta),
        }))
    }

    async fn index_image(
        &self,
        request: Request<IndexImageRequest>,
    ) -> Result<Response<IndexImageResponse>, Status> {
        #[cfg(feature = "auto_index")]
        {
            let req = request.into_inner();
            let bucket = self.resolve_bucket(&req.bucket);
            let index_name = if req.index_name.is_empty() {
                "image".to_string()
            } else {
                req.index_name.clone()
            };
            let model_name = if req.model_name.is_empty() {
                "jina-clip-v2".to_string()
            } else {
                req.model_name.clone()
            };

            // 1. 获取现有元数据（确认图片存在）
            let mut meta = match self.get_metadata_from_store(&bucket, &req.key).await? {
                Some(m) => m,
                None => {
                    return Ok(Response::new(IndexImageResponse {
                        success: false,
                        message: "图片不存在".to_string(),
                        embedding_id: String::new(),
                        embedding_dim: 0,
                        metadata: None,
                    }));
                }
            };

            // 2. 读取图片数据
            let get_req = Request::new(GetObjectRequest {
                bucket: bucket.clone(),
                key: req.key.clone(),
            });
            let obj_resp = self.object_store.get_object(get_req).await
                .map_err(|e| Status::internal(format!("读取图片失败: {}", e)))?;
            let obj = obj_resp.into_inner();
            if !obj.success {
                return Ok(Response::new(IndexImageResponse {
                    success: false,
                    message: format!("读取图片失败: {}", obj.message),
                    embedding_id: String::new(),
                    embedding_dim: 0,
                    metadata: None,
                }));
            }

            // 3. 生成向量
            let (embedding, dim) = self.generate_image_embedding(
                &obj.data,
                &model_name,
                &index_name,
            ).await.map_err(|e| Status::internal(format!("图片向量化失败: {}", e)))?;

            // 4. 插入向量索引（先做索引，成功后再更新 meta）
            // 幂等：HNSW 持久化索引中可能已存在该 node 的 embedding（历史索引过但元数据
            // is_indexed 未回写，导致"已索引却显示未索引"）。此时跳过插入，仅回写元数据。
            let insert_result = self.index_image_with_embedding(
                embedding,
                &req.key,
                &index_name,
            ).await;
            if let Err(e) = &insert_result {
                if e.to_string().contains("already exists") {
                    // 复用第 1 步已获取的 meta，标记为已索引即可
                    meta.is_indexed = true;
                    meta.index_model = model_name.clone();
                    meta.last_modified = Self::now_string();
                    if let Err(e2) = self.save_metadata_to_store(&bucket, &req.key, &meta).await {
                        warn!("索引已存在但更新元数据 is_indexed 失败: {}", e2);
                    }
                    info!("图片已存在向量索引，回写元数据 is_indexed: key='{}', index='{}', model='{}'", req.key, index_name, model_name);
                    return Ok(Response::new(IndexImageResponse {
                        success: true,
                        message: "图片已存在向量索引，已标记为已索引".to_string(),
                        embedding_id: req.key.clone(),
                        embedding_dim: 0,
                        metadata: Some(meta),
                    }));
                }
            }
            let (eid, edim) = insert_result.map_err(|e| Status::internal(format!("向量索引失败: {}", e)))?;

            // 5. 索引成功，更新元数据 is_indexed 标志
            meta.is_indexed = true;
            meta.index_model = model_name.clone();
            meta.last_modified = Self::now_string();
            if let Err(e) = self.save_metadata_to_store(&bucket, &req.key, &meta).await {
                warn!("索引成功但更新元数据 is_indexed 失败: {}", e);
            }

            info!("图片独立向量索引成功: key='{}', index='{}', model='{}'", req.key, index_name, model_name);

            Ok(Response::new(IndexImageResponse {
                success: true,
                message: "OK".to_string(),
                embedding_id: eid,
                embedding_dim: edim,
                metadata: Some(meta),
            }))
        }
        #[cfg(not(feature = "auto_index"))]
        {
            let _ = request;
            Ok(Response::new(IndexImageResponse {
                success: false,
                message: "auto_index feature 未启用，图片索引不可用".to_string(),
                embedding_id: String::new(),
                embedding_dim: 0,
                metadata: None,
            }))
        }
    }

    async fn delete_image(
        &self,
        request: Request<DeleteImageRequest>,
    ) -> Result<Response<DeleteImageResponse>, Status> {
        let req = request.into_inner();
        let bucket = self.resolve_bucket(&req.bucket);

        // 先获取元数据，以便知道要删除哪些缩略图
        // 元数据不存在时（图片已被删除），返回 None 实现幂等删除
        let metadata = match self.get_metadata_from_store(&bucket, &req.key).await {
            Ok(m) => m,
            Err(e) if e.code() == tonic::Code::NotFound => None,
            Err(e) => return Err(e),
        };

        let mut deleted_keys = Vec::new();

        // 先删除向量索引（如果存在且启用了 auto_index 功能）
        #[cfg(feature = "auto_index")]
        if let Some(embedding_svc) = &self.embedding_service {
            if let Ok(id) = req.key.parse::<u64>() {
                use laoflchdb_embedding_service::proto::DeleteEmbeddingRequest;
                let del_emb_req = tonic::Request::new(DeleteEmbeddingRequest {
                    id,
                    index_name: "image".to_string(),
                });
                let _ = embedding_svc.delete_embedding(del_emb_req).await;
            }
        }

        // 删除原图
        let del_req = Request::new(DeleteObjectRequest {
            bucket: bucket.clone(),
            key: req.key.clone(),
        });
        if self.object_store.delete_object(del_req).await.is_ok() {
            deleted_keys.push(req.key.clone());
        }

        // 删除缩略图
        if let Some(meta) = metadata {
            for (size_name, thumb_key) in &meta.thumbnails {
                let del_req = Request::new(DeleteObjectRequest {
                    bucket: bucket.clone(),
                    key: thumb_key.clone(),
                });
                if self.object_store.delete_object(del_req).await.is_ok() {
                    deleted_keys.push(thumb_key.clone());
                }
                let _ = size_name;
            }
        }

        // 删除元数据
        let meta_key = Self::metadata_key(&req.key);
        let del_req = Request::new(DeleteObjectRequest {
            bucket: bucket.clone(),
            key: meta_key.clone(),
        });
        if self.object_store.delete_object(del_req).await.is_ok() {
            deleted_keys.push(meta_key);
        }

        info!(
            "图片删除完成: bucket='{}', key='{}', deleted {} objects",
            bucket,
            req.key,
            deleted_keys.len()
        );

        Ok(Response::new(DeleteImageResponse {
            success: true,
            message: "OK".to_string(),
            deleted_keys,
        }))
    }

    async fn upload_image_stream(
        &self,
        request: Request<tonic::Streaming<UploadImageChunk>>,
    ) -> Result<Response<UploadImageResponse>, Status> {
        use futures::StreamExt;
        let mut stream = request.into_inner();

        let mut bucket = String::new();
        let mut key = String::new();
        let mut content_type = String::new();
        let mut metadata = std::collections::HashMap::new();
        let mut name = String::new();
        let mut all_data: Vec<u8> = Vec::new();
        let mut chunk_count = 0;
        let mut auto_index = false;
        let mut auto_index_model = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if chunk.chunk_index == 0 {
                bucket = chunk.bucket;
                key = chunk.key;
                content_type = chunk.content_type;
                metadata = chunk.metadata;
                name = chunk.name;
                auto_index = chunk.auto_index;
                auto_index_model = chunk.auto_index_model;
            }
            all_data.extend_from_slice(&chunk.data);
            chunk_count += 1;
        }

        if chunk_count == 0 {
            return Err(Status::invalid_argument("空的上传流"));
        }

        info!(
            "流式上传完成: {} chunks, total_size={} bytes",
            chunk_count,
            all_data.len()
        );

        // 将累积的数据作为普通上传处理
        let upload_req = UploadImageRequest {
            bucket,
            key,
            data: all_data,
            content_type,
            metadata,
            name,
            auto_index,
            auto_index_model,
            duplicate_action: String::new(),
        };
        self.upload_image(Request::new(upload_req)).await
    }

    /// 图片自动分类（HDBSCAN 聚类，结果可写入全文索引）
    async fn classify_images(
        &self,
        request: Request<ClassifyImagesRequest>,
    ) -> Result<Response<ClassifyImagesResponse>, Status> {
        #[cfg(feature = "auto_index")]
        {
        let req = request.into_inner();
        let bucket = if req.bucket.is_empty() {
            DEFAULT_BUCKET.to_string()
        } else {
            req.bucket.clone()
        };
        let vec_index = if req.index_name.is_empty() {
            "image".to_string()
        } else {
            req.index_name.clone()
        };
        let classify_index = if req.classify_index_name.is_empty() {
            "image_classification".to_string()
        } else {
            req.classify_index_name.clone()
        };
        let min_cluster_size = if req.min_cluster_size > 0 {
            req.min_cluster_size as usize
        } else {
            3
        };
        let cut_percentile = if req.cut_percentile > 0.0 && req.cut_percentile < 1.0 {
            req.cut_percentile
        } else {
            0.6
        };

        let embedding_svc = self
            .embedding_service
            .as_ref()
            .ok_or_else(|| Status::failed_precondition("嵌入索引服务未启用"))?;

        // 1. 循环分页拉取 bucket 全部图片 key
        let mut all_keys: Vec<String> = Vec::new();
        let mut marker = String::new();
        loop {
            let list_req = ListImagesRequest {
                bucket: bucket.clone(),
                prefix: String::new(),
                max_keys: 100,
                marker: marker.clone(),
                sort_order: "asc".to_string(),
            };
            let resp = self
                .list_images(Request::new(list_req))
                .await
                .map_err(|e| Status::internal(format!("列出图片失败: {}", e)))?
                .into_inner();
            if !resp.success {
                return Err(Status::internal(resp.message));
            }
            for img in &resp.images {
                all_keys.push(img.key.clone());
            }
            if !resp.is_truncated || resp.next_marker.is_empty() {
                break;
            }
            marker = resp.next_marker.clone();
            if all_keys.len() > 50000 {
                break;
            }
        }

        if all_keys.is_empty() {
            return Ok(Response::new(ClassifyImagesResponse {
                success: true,
                message: "当前 bucket 没有图片".to_string(),
                groups: vec![],
                total: 0,
                noise_count: 0,
                classify_index_name: classify_index.clone(),
            }));
        }

        // 2. 拉取向量索引全部条目（id 即图片 key）
        use laoflchdb_embedding_service::proto::ListEmbeddingsRequest;
        let emb_req = Request::new(ListEmbeddingsRequest {
            index_name: vec_index.clone(),
            limit: 0,
            offset: 0,
        });
        let emb_resp = embedding_svc
            .list_embeddings(emb_req)
            .await
            .map_err(|e| Status::internal(format!("读取向量索引失败: {}", e)))?;
        let emb = emb_resp.into_inner();
        if !emb.success {
            return Err(Status::internal(emb.message));
        }

        // 3. 匹配有向量的图片
        let mut vec_map: HashMap<String, Vec<f32>> = HashMap::new();
        for entry in &emb.entries {
            if !entry.embedding.is_empty() {
                vec_map.insert(entry.id.to_string(), entry.embedding.clone());
            }
        }
        let mut samples: Vec<(String, Vec<f32>)> = Vec::new();
        for key in &all_keys {
            if let Some(v) = vec_map.get(key) {
                samples.push((key.clone(), v.clone()));
            }
        }

        if samples.len() < 2 {
            return Ok(Response::new(ClassifyImagesResponse {
                success: true,
                message: format!("向量化图片不足({} 张)，无法聚类", samples.len()),
                groups: vec![],
                total: samples.len() as i32,
                noise_count: 0,
                classify_index_name: classify_index.clone(),
            }));
        }

        // 4. HDBSCAN 聚类
        let vectors: Vec<Vec<f32>> = samples.iter().map(|s| s.1.clone()).collect();
        let min_samples = 5.min(vectors.len() - 1).max(1);
        let result = hdbscan_cluster(
            &vectors,
            min_cluster_size,
            min_samples,
            cut_percentile,
        );

        // 5. 组装分类组（按图片数降序，不含噪声）
        let mut groups_map: HashMap<i32, Vec<String>> = HashMap::new();
        for (i, s) in samples.iter().enumerate() {
            let label = result.labels[i];
            if label == -1 {
                continue;
            }
            groups_map.entry(label).or_default().push(s.0.clone());
        }
        let mut group_list: Vec<(i32, Vec<String>)> = groups_map.into_iter().collect();
        group_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

        let mut groups: Vec<ImageClassGroup> = Vec::new();
        let mut label_to_meta: HashMap<i32, (String, String)> = HashMap::new();
        for (idx, (label, keys)) in group_list.iter().enumerate() {
            let category = format!("cat_{}", idx);
            let name = format!("分类 {}", idx + 1);
            label_to_meta.insert(*label, (category.clone(), name.clone()));
            groups.push(ImageClassGroup {
                name,
                category,
                keys: keys.clone(),
            });
        }

        // 6. 写入全文索引（可选）
        if req.write_to_index {
            let sink = self
                .index_sink
                .as_ref()
                .ok_or_else(|| Status::failed_precondition("全文索引未启用，无法保存分类结果"))?;
            sink.drop_index(&classify_index)
                .await
                .map_err(|e| Status::internal(format!("删除分类索引失败: {}", e)))?;
            let fields: Vec<(u32, &str, u8, Option<&str>)> = vec![
                (0, "key", 0, Some("图片 key")),
                (1, "category", 0, Some("分类 ID")),
                (2, "label", 0, Some("分类名称")),
                (3, "bucket", 0, Some("所属 bucket")),
            ];
            sink.create_index(&classify_index, &fields)
                .await
                .map_err(|e| Status::internal(format!("创建分类索引失败: {}", e)))?;
            for (i, s) in samples.iter().enumerate() {
                let meta = match label_to_meta.get(&result.labels[i]) {
                    Some((cat, name)) => (cat.clone(), name.clone()),
                    None => ("noise".to_string(), "未分类".to_string()),
                };
                let mut doc_fields = HashMap::new();
                doc_fields.insert("key".to_string(), s.0.clone());
                doc_fields.insert("category".to_string(), meta.0);
                doc_fields.insert("label".to_string(), meta.1);
                doc_fields.insert("bucket".to_string(), bucket.clone());
                sink.add_document(&classify_index, &s.0, doc_fields)
                    .await
                    .map_err(|e| Status::internal(format!("写入分类文档失败: {}", e)))?;
            }
        }

        Ok(Response::new(ClassifyImagesResponse {
            success: true,
            message: format!(
                "分类完成: {} 张图片，{} 个分类，{} 张噪声",
                samples.len(),
                result.cluster_count,
                result.noise_count
            ),
            groups,
            total: samples.len() as i32,
            noise_count: result.noise_count as i32,
            classify_index_name: classify_index,
        }))
        }
        #[cfg(not(feature = "auto_index"))]
        {
            let _ = request;
            Ok(Response::new(ClassifyImagesResponse {
                success: false,
                message: "auto_index feature 未启用，图片分类不可用".to_string(),
                groups: vec![],
                total: 0,
                noise_count: 0,
                classify_index_name: String::new(),
            }))
        }
    }
}

// ==================== REST API Router ====================

/// 创建 REST API Router
/// 返回的 Router 已绑定状态，可直接合并到主服务器 Router 中
/// 注意：路由路径使用根相对路径（如 "/", "/:key"），因为此 Router 会被 nest 到 "/api/v1/images" 下
pub fn create_rest_router(service: Arc<ImageServiceImpl>) -> Router {
    Router::new()
        // 上传图片: POST / (multipart/form-data 或 raw body)
        .route("/", post(upload_image_handler))
        // 列出图片: GET /
        .route("/", get(list_images_handler))
        // 获取图片元数据: GET /:key/meta
        .route("/:key/meta", get(get_image_meta_handler))
        // 获取或删除图片: GET/DELETE /:key
        .route(
            "/:key",
            get(get_image_handler).delete(delete_image_handler),
        )
        // 获取缩略图: GET /:key/thumbnails/:size
        .route("/:key/thumbnails/:size", get(get_thumbnail_handler))
        // 文搜图: POST /search/text
        .route("/search/text", post(search_by_text_handler))
        // 图搜图: POST /search/image
        .route("/search/image", post(search_by_image_handler))
        // 修改图片元数据: PUT /:key/meta
        .route("/:key/meta", put(update_metadata_handler))
        // 对已保存图片建立向量索引: POST /:key/index
        .route("/:key/index", post(index_image_handler))
        .with_state(service)
}

// ==================== REST Handlers ====================

#[derive(serde::Deserialize)]
struct ListImagesQuery {
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    max_keys: i32,
    #[serde(default)]
    marker: String,
    #[serde(default)]
    sort_order: String,
}

async fn list_images_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Query(query): Query<ListImagesQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(ListImagesRequest {
        bucket: query.bucket,
        prefix: query.prefix,
        max_keys: query.max_keys,
        marker: query.marker,
        sort_order: query.sort_order,
    });
    match service.list_images(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let images: Vec<serde_json::Value> = resp
                .images
                .iter()
                .map(|m| metadata_to_json(m))
                .collect();
            let result = serde_json::json!({
                "bucket": resp.bucket,
                "images": images,
                "is_truncated": resp.is_truncated,
                "next_marker": resp.next_marker,
            });
            (
                StatusCode::OK,
                serde_json::to_string(&result).unwrap_or_default(),
            )
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

#[derive(serde::Deserialize)]
struct IndexImageQuery {
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    index_name: String,
}

async fn index_image_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path(key): Path<String>,
    Query(query): Query<IndexImageQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(IndexImageRequest {
        bucket: query.bucket,
        key,
        model_name: query.model_name,
        index_name: query.index_name,
    });
    match service.index_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let meta = resp.metadata.as_ref()
                    .map(metadata_to_json)
                    .unwrap_or(serde_json::Value::Null);
                let result = serde_json::json!({
                    "success": true,
                    "embedding_id": resp.embedding_id,
                    "embedding_dim": resp.embedding_dim,
                    "metadata": meta,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (StatusCode::NOT_FOUND, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

async fn upload_image_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Query(query): Query<UploadImageQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let req = tonic::Request::new(UploadImageRequest {
        bucket: query.bucket,
        key: query.key,
        data: body.to_vec(),
        content_type,
        metadata: HashMap::new(),
        name: query.name,
        auto_index: false,
        auto_index_model: String::new(),
        duplicate_action: String::new(),
    });

    match service.upload_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let metadata_json = if let Some(ref m) = resp.metadata {
                    metadata_to_json(m)
                } else {
                    serde_json::Value::Null
                };
                let result = serde_json::json!({
                    "success": true,
                    "key": resp.key,
                    "etag": resp.etag,
                    "metadata": metadata_json,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    resp.message,
                )
            }
        }
        Err(e) => (StatusCode::BAD_REQUEST, e.message().to_string()),
    }
}

#[derive(serde::Deserialize)]
struct UploadImageQuery {
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    name: String,
}

async fn get_image_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path(key): Path<String>,
    Query(query): Query<BucketQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(GetImageRequest {
        bucket: query.bucket,
        key,
    });
    match service.get_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "content-type",
                    resp.content_type
                        .parse()
                        .unwrap_or("application/octet-stream".parse().unwrap()),
                );
                headers.insert(
                    "content-length",
                    resp.content_length.to_string().parse().unwrap(),
                );
                headers.insert("etag", resp.etag.parse().unwrap());
                (StatusCode::OK, headers, resp.data)
            } else {
                (
                    StatusCode::NOT_FOUND,
                    HeaderMap::new(),
                    resp.message.as_bytes().to_vec(),
                )
            }
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            HeaderMap::new(),
            e.message().as_bytes().to_vec(),
        ),
    }
}

#[derive(serde::Deserialize)]
struct BucketQuery {
    #[serde(default)]
    bucket: String,
}

async fn get_thumbnail_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path((key, size)): Path<(String, String)>,
    Query(query): Query<BucketQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(GetThumbnailRequest {
        bucket: query.bucket,
        key,
        size,
    });
    match service.get_thumbnail(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let mut headers = HeaderMap::new();
                headers.insert(
                    "content-type",
                    resp.content_type
                        .parse()
                        .unwrap_or("image/jpeg".parse().unwrap()),
                );
                headers.insert(
                    "content-length",
                    resp.content_length.to_string().parse().unwrap(),
                );
                headers.insert(
                    "x-thumbnail-width",
                    resp.width.to_string().parse().unwrap(),
                );
                headers.insert(
                    "x-thumbnail-height",
                    resp.height.to_string().parse().unwrap(),
                );
                (StatusCode::OK, headers, resp.data)
            } else {
                (
                    StatusCode::NOT_FOUND,
                    HeaderMap::new(),
                    resp.message.as_bytes().to_vec(),
                )
            }
        }
        Err(e) => {
            let status = if e.code() == tonic::Code::InvalidArgument {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::NOT_FOUND
            };
            (status, HeaderMap::new(), e.message().as_bytes().to_vec())
        }
    }
}

async fn get_image_meta_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path(key): Path<String>,
    Query(query): Query<BucketQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(GetImageMetadataRequest {
        bucket: query.bucket,
        key,
    });
    match service.get_image_metadata(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                if let Some(ref m) = resp.metadata {
                    let result = metadata_to_json(m);
                    (
                        StatusCode::OK,
                        serde_json::to_string(&result).unwrap_or_default(),
                    )
                } else {
                    (StatusCode::NOT_FOUND, "{}".to_string())
                }
            } else {
                (StatusCode::NOT_FOUND, resp.message)
            }
        }
        Err(e) => (StatusCode::NOT_FOUND, e.message().to_string()),
    }
}

async fn delete_image_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path(key): Path<String>,
    Query(query): Query<BucketQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(DeleteImageRequest {
        bucket: query.bucket,
        key,
    });
    match service.delete_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let result = serde_json::json!({
                    "success": true,
                    "deleted_keys": resp.deleted_keys,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

// re-export for metadata access
pub use laoflchdb_object_store_service::ObjectStoreServiceImpl;

// ==================== REST Handlers for Search & Update ====================

#[derive(serde::Deserialize)]
struct SearchByTextBody {
    text: String,
    #[serde(default = "default_top_k")]
    top_k: i32,
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    index_name: String,
}

fn default_top_k() -> i32 { 10 }

fn metadata_to_json(m: &ImageMetadata) -> serde_json::Value {
    let thumbnails: serde_json::Value = m
        .thumbnails
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    let user_metadata: serde_json::Value = m
        .user_metadata
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    serde_json::json!({
        "key": m.key,
        "content_type": m.content_type,
        "content_length": m.content_length,
        "width": m.width,
        "height": m.height,
        "etag": m.etag,
        "last_modified": m.last_modified,
        "thumbnails": thumbnails,
        "user_metadata": user_metadata,
        "format": m.format,
        "name": m.name,
        "is_indexed": m.is_indexed,
        "index_model": m.index_model,
    })
}

async fn search_by_text_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Json(body): Json<SearchByTextBody>,
) -> impl IntoResponse {
    let req = tonic::Request::new(SearchImagesByTextRequest {
        text: body.text,
        top_k: body.top_k,
        bucket: body.bucket,
        model_name: body.model_name,
        index_name: body.index_name,
    });
    match service.search_images_by_text(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let results: Vec<serde_json::Value> = resp
                    .results
                    .iter()
                    .map(|r| {
                        let meta = r.metadata.as_ref()
                            .map(metadata_to_json)
                            .unwrap_or(serde_json::Value::Null);
                        serde_json::json!({
                            "metadata": meta,
                            "score": r.score,
                        })
                    })
                    .collect();
                let result = serde_json::json!({
                    "success": true,
                    "results": results,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

#[derive(serde::Deserialize)]
struct SearchByImageQuery {
    #[serde(default)]
    top_k: i32,
    #[serde(default)]
    bucket: String,
    #[serde(default)]
    model_name: String,
    #[serde(default)]
    index_name: String,
}

async fn search_by_image_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Query(query): Query<SearchByImageQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let req = tonic::Request::new(SearchImagesByImageRequest {
        image_data: body.to_vec(),
        top_k: if query.top_k > 0 { query.top_k } else { 10 },
        bucket: query.bucket,
        model_name: query.model_name,
        index_name: query.index_name,
    });
    match service.search_images_by_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let results: Vec<serde_json::Value> = resp
                    .results
                    .iter()
                    .map(|r| {
                        let meta = r.metadata.as_ref()
                            .map(metadata_to_json)
                            .unwrap_or(serde_json::Value::Null);
                        serde_json::json!({
                            "metadata": meta,
                            "score": r.score,
                        })
                    })
                    .collect();
                let result = serde_json::json!({
                    "success": true,
                    "results": results,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

#[derive(serde::Deserialize)]
struct UpdateMetadataBody {
    #[serde(default)]
    name: String,
    #[serde(default)]
    user_metadata: std::collections::HashMap<String, String>,
    #[serde(default)]
    delete_keys: Vec<String>,
}

async fn update_metadata_handler(
    State(service): State<Arc<ImageServiceImpl>>,
    Path(key): Path<String>,
    Query(query): Query<BucketQuery>,
    Json(body): Json<UpdateMetadataBody>,
) -> impl IntoResponse {
    let req = tonic::Request::new(UpdateImageMetadataRequest {
        bucket: query.bucket,
        key,
        name: body.name,
        user_metadata: body.user_metadata,
        delete_user_metadata_keys: body.delete_keys,
    });
    match service.update_image_metadata(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let meta = resp.metadata.as_ref()
                    .map(metadata_to_json)
                    .unwrap_or(serde_json::Value::Null);
                let result = serde_json::json!({
                    "success": true,
                    "metadata": meta,
                });
                (
                    StatusCode::OK,
                    serde_json::to_string(&result).unwrap_or_default(),
                )
            } else {
                (StatusCode::NOT_FOUND, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

// ==================== HDBSCAN 聚类（图片分类） ====================

/// HDBSCAN 聚类结果
struct HdbscanResult {
    /// 每个点的聚类标签，-1 表示噪声
    labels: Vec<i32>,
    /// 聚类数量（不含噪声）
    cluster_count: usize,
    /// 噪声点数
    noise_count: usize,
}

/// 基于 hdbscan-rs 的 HDBSCAN 聚类（余弦距离，结果与 scikit-learn 兼容）
///
/// min_cluster_size: 最少聚类点数（低于此数量的簇归为噪声）
/// min_samples: 密度估计近邻数，越大聚类越保守
fn hdbscan_cluster(
    vectors: &[Vec<f32>],
    min_cluster_size: usize,
    min_samples: usize,
    _cut_percentile: f64,
) -> HdbscanResult {
    let n = vectors.len();
    if n == 0 {
        return HdbscanResult { labels: vec![], cluster_count: 0, noise_count: 0 };
    }
    if n == 1 {
        return HdbscanResult { labels: vec![-1], cluster_count: 0, noise_count: 1 };
    }
    let dim = vectors[0].len();
    let flat: Vec<f64> = vectors.iter().flatten().map(|&v| v as f64).collect();
    let data = match ndarray::Array2::from_shape_vec((n, dim), flat) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("构建聚类输入失败: {}", e);
            return HdbscanResult { labels: vec![-1; n], cluster_count: 0, noise_count: n };
        }
    };
    let params = hdbscan_rs::HdbscanParams {
        min_cluster_size,
        min_samples: Some(min_samples),
        metric: hdbscan_rs::Metric::Cosine,
        ..Default::default()
    };
    let mut hdbscan = hdbscan_rs::Hdbscan::new(params);
    let labels = match hdbscan.fit_predict(&data.view()) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("HDBSCAN 聚类失败: {}", e);
            return HdbscanResult { labels: vec![-1; n], cluster_count: 0, noise_count: n };
        }
    };
    let mut seen: Vec<i32> = labels.iter().filter(|&&l| l != -1).copied().collect();
    seen.sort_unstable();
    seen.dedup();
    let cluster_count = seen.len();
    let noise_count = labels.iter().filter(|&&l| l == -1).count();
    HdbscanResult { labels, cluster_count, noise_count }
}
