pub mod proto {
    tonic::include_proto!("laoflchdb.image_service");
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use image::imageops::FilterType;
use laoflchdb_object_store_service::proto::object_store_service_server::ObjectStoreService;
use laoflchdb_object_store_service::proto::{
    CreateBucketRequest, DeleteObjectRequest, GetObjectRequest,
    ListObjectsRequest, PutObjectRequest,
};
use log::info;
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
}

impl Default for ImageServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_bucket: DEFAULT_BUCKET.to_string(),
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
        }
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

    async fn upload_image_stream(
        &self,
        request: tonic::Request<tonic::Streaming<UploadImageChunk>>,
    ) -> std::result::Result<tonic::Response<UploadImageResponse>, tonic::Status> {
        self.as_ref().upload_image_stream(request).await
    }
}

// ── 内部辅助方法（非 trait 方法） ──
impl ImageServiceImpl {
    /// 自动向量索引：调用向量服务生成向量并插入 image 索引
    /// 服务端内部调用，直接传数据不需要流式
    #[cfg(feature = "auto_index")]
    async fn auto_index_image(
        &self,
        image_data: &[u8],
        key: &str,
        model_name: &str,
    ) -> Result<(String, i32), Box<dyn std::error::Error + Send + Sync>> {
        let vector_svc = self.vector_service.as_ref().ok_or("向量服务未启用")?;
        let embedding_svc = self.embedding_service.as_ref().ok_or("嵌入索引服务未启用")?;

        // 1. 获取 image 索引的维度
        let index_dim = {
            use laoflchdb_embedding_service::proto::GetIndexInfoRequest;
            let info_req = tonic::Request::new(GetIndexInfoRequest {
                index_name: "image".to_string(),
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

        // 2. 调用向量服务生成嵌入向量（服务端内部调用，直接传数据）
        use laoflchdb_vector_service::proto::EmbeddingRequest;
        let emb_req = tonic::Request::new(EmbeddingRequest {
            model_name: model_name.to_string(),
            texts: vec![],
            dim: index_dim,
            images: vec![image_data.to_vec()],
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

        // 3. 插入嵌入索引
        use laoflchdb_embedding_service::proto::InsertEmbeddingRequest;
        let id = key.parse::<u64>().unwrap_or_else(|_| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        });
        let ins_req = tonic::Request::new(InsertEmbeddingRequest {
            id,
            index_name: "image".to_string(),
            embedding,
        });
        let ins_resp = embedding_svc.insert_embedding(ins_req).await
            .map_err(|e| format!("索引请求失败: {}", e))?;
        let ins = ins_resp.into_inner();
        if !ins.success {
            return Err(format!("索引失败: {}", ins.message).into());
        }

        Ok((key.to_string(), index_dim))
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

        // 生成图片 key（若未指定则使用 Snowflake ID 自动生成）
        let image_key = if req.key.is_empty() {
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
        let metadata = ImageMetadata {
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
        };

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
        });

        let meta_key = Self::metadata_key(&image_key);
        let meta_put_req = Request::new(PutObjectRequest {
            bucket: bucket.clone(),
            key: meta_key,
            data: meta_json.to_string().into_bytes(),
            content_type: "application/json".to_string(),
            metadata: HashMap::new(),
        });
        self.object_store.put_object(meta_put_req).await?;

        info!(
            "图片上传成功: bucket='{}', key='{}', size={}x{}, format={}",
            bucket, image_key, width, height, metadata.format
        );

        // ── 自动向量索引 ──
        let mut auto_indexed = false;
        let mut embedding_id = String::new();
        let mut embedding_dim = 0i32;

        #[cfg(feature = "auto_index")]
        if req.auto_index {
            let model_name = if req.auto_index_model.is_empty() {
                "jina-clip-v2"
            } else {
                &req.auto_index_model
            };

            match self.auto_index_image(&req.data, &image_key, model_name).await {
                Ok((eid, edim)) => {
                    auto_indexed = true;
                    embedding_id = eid;
                    embedding_dim = edim;
                    info!("图片自动向量索引成功: key='{}', model='{}', id='{}'", image_key, model_name, embedding_id);
                }
                Err(e) => {
                    log::warn!("图片自动向量索引失败: key='{}', model='{}', error={}", image_key, model_name, e);
                }
            }
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

        // 列出所有元数据 key
        let meta_prefix = format!("{}{}", IMAGE_META_PREFIX, req.prefix);
        let list_req = Request::new(ListObjectsRequest {
            bucket: bucket.clone(),
            prefix: meta_prefix,
            delimiter: String::new(),
            max_keys: max_keys as i32,
            marker: req.marker.clone(),
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
        };
        self.upload_image(Request::new(upload_req)).await
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
    });
    match service.list_images(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let images: Vec<serde_json::Value> = resp
                .images
                .iter()
                .map(|m| {
                    let thumbnails: serde_json::Value = m
                        .thumbnails
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
                        "format": m.format,
                        "name": m.name,
                    })
                })
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
    });

    match service.upload_image(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                let metadata_json = if let Some(ref m) = resp.metadata {
                    let thumbnails: serde_json::Value = m
                        .thumbnails
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
                        "format": m.format,
                        "name": m.name,
                    })
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
                    let result = serde_json::json!({
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
                    });
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
