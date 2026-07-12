pub mod proto {
    tonic::include_proto!("laoflchdb.object_store");
}

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, head, put},
};
use laoflchdb_engines::{EngineOptions, StorageEngine};
use laoflchdb_kv_rocksdb_engine::{BlobDBConfig, KVRocksDBEngine};
use log::info;
use proto::object_store_service_server::ObjectStoreService;
use proto::*;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};
use uuid::Uuid;

/// 对象存储服务配置
#[derive(Debug, Clone)]
pub struct ObjectStoreConfig {
    pub enabled: bool,
    pub db_path: String,
    pub schema_name: String,
    /// BlobDB 配置，用于大对象存储
    pub blob_db: BlobDBConfig,
}

impl Default for ObjectStoreConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            db_path: "./laoflch_object_store_data".to_string(),
            schema_name: "object_store".to_string(),
            blob_db: BlobDBConfig {
                enabled: true,
                min_blob_size: 0,
                blob_file_size: 256 * 1024 * 1024,
                blob_compression_type: "zstd".to_string(),
                enable_blob_garbage_collection: true,
                blob_garbage_collection_age_cutoff: 0.25,
            },
        }
    }
}

// Constants for object storage keys
const OBJECT_DATA_PREFIX: &str = "__obj__";
const OBJECT_META_PREFIX: &str = "__meta__";
const BUCKET_META_KEY: &str = "__bucket_meta__";

/// 对象存储服务实现
pub struct ObjectStoreServiceImpl {
    engine: Mutex<KVRocksDBEngine>,
    config: Arc<ObjectStoreConfig>,
}

impl ObjectStoreServiceImpl {
    pub async fn new(
        config: &ObjectStoreConfig,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // 确保数据目录存在
        tokio::fs::create_dir_all(&config.db_path).await?;

        let options = EngineOptions {
            db_path: config.db_path.clone(),
            schema_name: config.schema_name.clone(),
        };
        let engine = KVRocksDBEngine::new_with_blob_db(&options, &config.blob_db)?;
        info!(
            "ObjectStoreService 初始化完成: db_path='{}'",
            config.db_path
        );
        Ok(Self {
            engine: Mutex::new(engine),
            config: Arc::new(config.clone()),
        })
    }

    /// 生成 ETag（基于 UUID 的简化实现）
    fn generate_etag() -> String {
        format!("\"{}\"", Uuid::new_v4().to_string().replace('-', ""))
    }

    /// 获取当前 Unix 时间戳（秒）
    fn now_string() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        format!("{}", now.as_secs())
    }

    /// 确保 bucket 对应的表存在
    async fn ensure_bucket_table(&self, bucket: &str) -> Result<(), Status> {
        let mut engine = self.engine.lock().await;
        if !engine.table_exists(bucket) {
            drop(engine);
            let mut engine = self.engine.lock().await;
            let columns: Vec<(u32, &str, laoflchdb_engines::ColumnType, Option<&str>)> = vec![];
            engine
                .create_table(bucket, Some("S3 bucket"), &columns)
                .await
                .map_err(|e| {
                    Status::internal(format!("Failed to create bucket '{}': {}", bucket, e))
                })?;
            // 记录 bucket 创建时间
            let bucket_meta = serde_json::json!({
                "name": bucket,
                "creation_date": Self::now_string(),
            });
            engine
                .put(bucket, BUCKET_META_KEY.as_bytes(), bucket_meta.to_string().as_bytes())
                .await
                .map_err(|e| Status::internal(format!("Failed to save bucket meta: {}", e)))?;
        }
        Ok(())
    }
}

/// 允许通过 `Arc<ObjectStoreServiceImpl>` 直接作为 gRPC 服务注册
#[tonic::async_trait]
impl proto::object_store_service_server::ObjectStoreService
    for std::sync::Arc<ObjectStoreServiceImpl>
{
    async fn put_object(
        &self,
        request: tonic::Request<PutObjectRequest>,
    ) -> std::result::Result<tonic::Response<PutObjectResponse>, tonic::Status> {
        self.as_ref().put_object(request).await
    }

    async fn get_object(
        &self,
        request: tonic::Request<GetObjectRequest>,
    ) -> std::result::Result<tonic::Response<GetObjectResponse>, tonic::Status> {
        self.as_ref().get_object(request).await
    }

    async fn delete_object(
        &self,
        request: tonic::Request<DeleteObjectRequest>,
    ) -> std::result::Result<tonic::Response<DeleteObjectResponse>, tonic::Status> {
        self.as_ref().delete_object(request).await
    }

    async fn list_objects(
        &self,
        request: tonic::Request<ListObjectsRequest>,
    ) -> std::result::Result<tonic::Response<ListObjectsResponse>, tonic::Status> {
        self.as_ref().list_objects(request).await
    }

    async fn head_object(
        &self,
        request: tonic::Request<HeadObjectRequest>,
    ) -> std::result::Result<tonic::Response<HeadObjectResponse>, tonic::Status> {
        self.as_ref().head_object(request).await
    }

    async fn copy_object(
        &self,
        request: tonic::Request<CopyObjectRequest>,
    ) -> std::result::Result<tonic::Response<CopyObjectResponse>, tonic::Status> {
        self.as_ref().copy_object(request).await
    }

    async fn delete_objects(
        &self,
        request: tonic::Request<DeleteObjectsRequest>,
    ) -> std::result::Result<tonic::Response<DeleteObjectsResponse>, tonic::Status> {
        self.as_ref().delete_objects(request).await
    }

    async fn create_bucket(
        &self,
        request: tonic::Request<CreateBucketRequest>,
    ) -> std::result::Result<tonic::Response<CreateBucketResponse>, tonic::Status> {
        self.as_ref().create_bucket(request).await
    }

    async fn delete_bucket(
        &self,
        request: tonic::Request<DeleteBucketRequest>,
    ) -> std::result::Result<tonic::Response<DeleteBucketResponse>, tonic::Status> {
        self.as_ref().delete_bucket(request).await
    }

    async fn list_buckets(
        &self,
        request: tonic::Request<ListBucketsRequest>,
    ) -> std::result::Result<tonic::Response<ListBucketsResponse>, tonic::Status> {
        self.as_ref().list_buckets(request).await
    }
}

#[tonic::async_trait]
impl ObjectStoreService for ObjectStoreServiceImpl {
    async fn put_object(
        &self,
        request: Request<PutObjectRequest>,
    ) -> Result<Response<PutObjectResponse>, Status> {
        let req = request.into_inner();
        self.ensure_bucket_table(&req.bucket).await?;

        let data_key = format!("{}{}", OBJECT_DATA_PREFIX, req.key);
        let meta_key = format!("{}{}", OBJECT_META_PREFIX, req.key);
        let etag = Self::generate_etag();
        let now = Self::now_string();

        // 存储对象数据
        let mut engine = self.engine.lock().await;
        engine
            .put(&req.bucket, data_key.as_bytes(), &req.data)
            .await
            .map_err(|e| Status::internal(format!("Failed to store object: {}", e)))?;

        // 存储对象元数据
        let metadata = serde_json::json!({
            "key": req.key,
            "content_type": req.content_type,
            "content_length": req.data.len(),
            "etag": etag,
            "last_modified": now,
            "user_metadata": req.metadata,
        });
        engine
            .put(
                &req.bucket,
                meta_key.as_bytes(),
                metadata.to_string().as_bytes(),
            )
            .await
            .map_err(|e| Status::internal(format!("Failed to store metadata: {}", e)))?;

        Ok(Response::new(PutObjectResponse {
            success: true,
            message: "OK".to_string(),
            etag: etag.clone(),
        }))
    }

    async fn get_object(
        &self,
        request: Request<GetObjectRequest>,
    ) -> Result<Response<GetObjectResponse>, Status> {
        let req = request.into_inner();
        let data_key = format!("{}{}", OBJECT_DATA_PREFIX, req.key);
        let meta_key = format!("{}{}", OBJECT_META_PREFIX, req.key);

        let mut engine = self.engine.lock().await;
        let data = engine
            .get(&req.bucket, data_key.as_bytes())
            .await
            .map_err(|e| Status::internal(format!("Failed to get object: {}", e)))?
            .ok_or_else(|| {
                Status::not_found(format!("Object '{}/{}' not found", req.bucket, req.key))
            })?;

        let meta_bytes = engine
            .get(&req.bucket, meta_key.as_bytes())
            .await
            .map_err(|e| Status::internal(format!("Failed to get metadata: {}", e)))?
            .unwrap_or_default();

        let mut content_type = String::new();
        let mut etag = String::new();
        let mut metadata = HashMap::new();

        if !meta_bytes.is_empty() {
            if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&meta_bytes) {
                content_type = meta
                    .get("content_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                etag = meta
                    .get("etag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(um) = meta.get("user_metadata").and_then(|v| v.as_object()) {
                    for (k, v) in um {
                        if let Some(s) = v.as_str() {
                            metadata.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
        }

        let content_length = data.len() as i64;

        Ok(Response::new(GetObjectResponse {
            success: true,
            message: "OK".to_string(),
            data,
            content_type,
            content_length,
            etag,
            metadata,
        }))
    }

    async fn delete_object(
        &self,
        request: Request<DeleteObjectRequest>,
    ) -> Result<Response<DeleteObjectResponse>, Status> {
        let req = request.into_inner();
        let data_key = format!("{}{}", OBJECT_DATA_PREFIX, req.key);
        let meta_key = format!("{}{}", OBJECT_META_PREFIX, req.key);

        let mut engine = self.engine.lock().await;
        engine
            .delete(&req.bucket, data_key.as_bytes())
            .await
            .map_err(|e| Status::internal(format!("Failed to delete object: {}", e)))?;
        engine
            .delete(&req.bucket, meta_key.as_bytes())
            .await
            .map_err(|e| Status::internal(format!("Failed to delete metadata: {}", e)))?;

        Ok(Response::new(DeleteObjectResponse {
            success: true,
            message: "OK".to_string(),
        }))
    }

    async fn list_objects(
        &self,
        request: Request<ListObjectsRequest>,
    ) -> Result<Response<ListObjectsResponse>, Status> {
        let req = request.into_inner();
        let max_keys = if req.max_keys <= 0 {
            1000
        } else {
            req.max_keys as usize
        };
        let prefix = req.prefix.clone();
        let delimiter = if req.delimiter.is_empty() {
            None
        } else {
            Some(req.delimiter.clone())
        };

        let mut engine = self.engine.lock().await;
        let data_prefix = format!("{}{}", OBJECT_DATA_PREFIX, prefix);
        let keys = engine
            .list_keys(
                &req.bucket,
                Some(data_prefix.as_bytes()),
                Some(max_keys),
            )
            .map_err(|e| Status::internal(format!("Failed to list objects: {}", e)))?;

        let mut objects = Vec::new();
        let mut common_prefixes = Vec::new();
        let mut seen_prefixes = std::collections::HashSet::new();

        for key_bytes in keys {
            let key_str = String::from_utf8_lossy(&key_bytes);
            if let Some(obj_key) = key_str.strip_prefix(OBJECT_DATA_PREFIX) {
                // If delimiter is specified, extract common prefixes
                if let Some(delim) = &delimiter {
                    if let Some(rest) = obj_key.strip_prefix(&prefix) {
                        if let Some(idx) = rest.find(delim.as_str()) {
                            let cp = format!("{}{}", prefix, &rest[..=idx]);
                            if seen_prefixes.insert(cp.clone()) {
                                common_prefixes.push(cp);
                                continue;
                            }
                        }
                    }
                }

                // Get metadata for this object
                let meta_key = format!("{}{}", OBJECT_META_PREFIX, obj_key);
                let meta_bytes = engine
                    .get(&req.bucket, meta_key.as_bytes())
                    .await
                    .map_err(|e| Status::internal(format!("Failed to get metadata: {}", e)))?
                    .unwrap_or_default();

                let mut size: i64 = 0;
                let mut etag = String::new();
                let mut last_modified = String::new();
                let mut content_type = String::new();

                if !meta_bytes.is_empty() {
                    if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&meta_bytes) {
                        size = meta
                            .get("content_length")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        etag = meta
                            .get("etag")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        last_modified = meta
                            .get("last_modified")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        content_type = meta
                            .get("content_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                }

                objects.push(ObjectInfo {
                    key: obj_key.to_string(),
                    size,
                    etag,
                    last_modified,
                    content_type,
                });
            }
        }

        let is_truncated = objects.len() >= max_keys;
        let next_marker = if is_truncated {
            objects
                .last()
                .map(|o| o.key.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };

        Ok(Response::new(ListObjectsResponse {
            success: true,
            message: "OK".to_string(),
            bucket: req.bucket,
            objects,
            common_prefixes,
            is_truncated,
            next_marker,
        }))
    }

    async fn head_object(
        &self,
        request: Request<HeadObjectRequest>,
    ) -> Result<Response<HeadObjectResponse>, Status> {
        let req = request.into_inner();
        let meta_key = format!("{}{}", OBJECT_META_PREFIX, req.key);

        let mut engine = self.engine.lock().await;
        let meta_bytes = engine
            .get(&req.bucket, meta_key.as_bytes())
            .await
            .map_err(|e| Status::internal(format!("Failed to get metadata: {}", e)))?
            .ok_or_else(|| {
                Status::not_found(format!("Object '{}/{}' not found", req.bucket, req.key))
            })?;

        let mut content_type = String::new();
        let mut content_length: i64 = 0;
        let mut etag = String::new();
        let mut last_modified = String::new();
        let mut metadata = HashMap::new();

        if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&meta_bytes) {
            content_type = meta
                .get("content_type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            content_length = meta
                .get("content_length")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            etag = meta
                .get("etag")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            last_modified = meta
                .get("last_modified")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if let Some(um) = meta.get("user_metadata").and_then(|v| v.as_object()) {
                for (k, v) in um {
                    if let Some(s) = v.as_str() {
                        metadata.insert(k.clone(), s.to_string());
                    }
                }
            }
        }

        Ok(Response::new(HeadObjectResponse {
            success: true,
            message: "OK".to_string(),
            content_type,
            content_length,
            etag,
            last_modified,
            metadata,
        }))
    }

    async fn copy_object(
        &self,
        request: Request<CopyObjectRequest>,
    ) -> Result<Response<CopyObjectResponse>, Status> {
        let req = request.into_inner();

        // Get source object
        let get_req = GetObjectRequest {
            bucket: req.source_bucket.clone(),
            key: req.source_key.clone(),
        };
        let get_resp = self.get_object(Request::new(get_req)).await?.into_inner();

        if !get_resp.success {
            return Ok(Response::new(CopyObjectResponse {
                success: false,
                message: "Source object not found".to_string(),
                etag: String::new(),
            }));
        }

        // Put to destination
        let put_req = PutObjectRequest {
            bucket: req.destination_bucket.clone(),
            key: req.destination_key.clone(),
            data: get_resp.data,
            content_type: get_resp.content_type,
            metadata: get_resp.metadata,
        };
        let put_resp = self.put_object(Request::new(put_req)).await?.into_inner();

        Ok(Response::new(CopyObjectResponse {
            success: put_resp.success,
            message: put_resp.message,
            etag: put_resp.etag,
        }))
    }

    async fn delete_objects(
        &self,
        request: Request<DeleteObjectsRequest>,
    ) -> Result<Response<DeleteObjectsResponse>, Status> {
        let req = request.into_inner();
        let mut deleted_keys = Vec::new();

        for key in &req.keys {
            let del_req = DeleteObjectRequest {
                bucket: req.bucket.clone(),
                key: key.clone(),
            };
            if self.delete_object(Request::new(del_req)).await.is_ok() {
                deleted_keys.push(key.clone());
            }
        }

        Ok(Response::new(DeleteObjectsResponse {
            success: true,
            message: "OK".to_string(),
            deleted_keys,
        }))
    }

    async fn create_bucket(
        &self,
        request: Request<CreateBucketRequest>,
    ) -> Result<Response<CreateBucketResponse>, Status> {
        let req = request.into_inner();
        self.ensure_bucket_table(&req.bucket).await?;

        Ok(Response::new(CreateBucketResponse {
            success: true,
            message: "OK".to_string(),
        }))
    }

    async fn delete_bucket(
        &self,
        request: Request<DeleteBucketRequest>,
    ) -> Result<Response<DeleteBucketResponse>, Status> {
        let req = request.into_inner();

        let mut engine = self.engine.lock().await;
        if engine.table_exists(&req.bucket) {
            drop(engine);
            let mut engine = self.engine.lock().await;
            engine
                .drop_table(&req.bucket)
                .await
                .map_err(|e| Status::internal(format!("Failed to delete bucket: {}", e)))?;
        }

        Ok(Response::new(DeleteBucketResponse {
            success: true,
            message: "OK".to_string(),
        }))
    }

    async fn list_buckets(
        &self,
        _request: Request<ListBucketsRequest>,
    ) -> Result<Response<ListBucketsResponse>, Status> {
        let mut engine = self.engine.lock().await;

        let tables = engine
            .list_tables()
            .await
            .map_err(|e| Status::internal(format!("Failed to list buckets: {}", e)))?;

        let mut buckets = Vec::new();
        for table in tables {
            // Get bucket creation date from metadata
            let meta_bytes = engine
                .get(&table, BUCKET_META_KEY.as_bytes())
                .await
                .map_err(|e| Status::internal(format!("Failed to get bucket meta: {}", e)))?
                .unwrap_or_default();

            let creation_date = if !meta_bytes.is_empty() {
                if let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&meta_bytes) {
                    meta.get("creation_date")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            buckets.push(BucketInfo {
                name: table,
                creation_date,
            });
        }

        Ok(Response::new(ListBucketsResponse {
            success: true,
            message: "OK".to_string(),
            buckets,
        }))
    }
}

// ==================== REST API Router（S3 兼容的 HTTP 接口） ====================

/// 创建 REST API Router
/// 返回的 Router 已绑定状态，可直接合并到主服务器 Router 中
pub fn create_rest_router(service: Arc<ObjectStoreServiceImpl>) -> Router {
    Router::new()
        .route("/", get(list_buckets_handler))
        .route(
            "/{bucket}",
            put(create_bucket_handler)
                .get(list_objects_handler)
                .delete(delete_bucket_handler),
        )
        .route(
            "/{bucket}/*key",
            put(put_object_handler)
                .get(get_object_handler)
                .head(head_object_handler)
                .delete(delete_object_handler),
        )
        .with_state(service)
}

// ==================== REST Handlers ====================

async fn list_buckets_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
) -> impl IntoResponse {
    let req = tonic::Request::new(ListBucketsRequest {});
    match service.list_buckets(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let buckets: Vec<serde_json::Value> = resp
                .buckets
                .iter()
                .map(|b| {
                    serde_json::json!({
                        "name": b.name,
                        "creation_date": b.creation_date,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                serde_json::to_string(&serde_json::json!({"buckets": buckets}))
                    .unwrap_or_default(),
            )
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::to_string(&serde_json::json!({"error": e.message()}))
                .unwrap_or_default(),
        ),
    }
}

async fn create_bucket_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    let req = tonic::Request::new(CreateBucketRequest { bucket });
    match service.create_bucket(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                (StatusCode::OK, "".to_string())
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to create bucket".to_string(),
                )
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

async fn delete_bucket_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    let req = tonic::Request::new(DeleteBucketRequest { bucket });
    match service.delete_bucket(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                (StatusCode::NO_CONTENT, "".to_string())
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to delete bucket".to_string(),
                )
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

#[derive(serde::Deserialize)]
struct ListObjectsQuery {
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    delimiter: String,
    #[serde(default)]
    max_keys: i32,
    #[serde(default)]
    marker: String,
}

async fn list_objects_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path(bucket): Path<String>,
    Query(query): Query<ListObjectsQuery>,
) -> impl IntoResponse {
    let req = tonic::Request::new(ListObjectsRequest {
        bucket,
        prefix: query.prefix,
        delimiter: query.delimiter,
        max_keys: query.max_keys,
        marker: query.marker,
    });
    match service.list_objects(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let objects: Vec<serde_json::Value> = resp
                .objects
                .iter()
                .map(|o| {
                    serde_json::json!({
                        "key": o.key,
                        "size": o.size,
                        "etag": o.etag,
                        "last_modified": o.last_modified,
                        "content_type": o.content_type,
                    })
                })
                .collect();
            let result = serde_json::json!({
                "bucket": resp.bucket,
                "objects": objects,
                "common_prefixes": resp.common_prefixes,
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

async fn put_object_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path((bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let key = key.strip_prefix('/').unwrap_or(&key).to_string();

    let req = tonic::Request::new(PutObjectRequest {
        bucket,
        key,
        data: body.to_vec(),
        content_type,
        metadata: HashMap::new(),
    });

    match service.put_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                (
                    StatusCode::OK,
                    serde_json::to_string(&serde_json::json!({"etag": resp.etag}))
                        .unwrap_or_default(),
                )
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, resp.message)
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}

async fn get_object_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(GetObjectRequest { bucket, key });

    match service.get_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
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
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            HeaderMap::new(),
            e.message().as_bytes().to_vec(),
        ),
    }
}

async fn head_object_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(HeadObjectRequest { bucket, key });

    match service.head_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
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
            headers.insert("last-modified", resp.last_modified.parse().unwrap());
            (StatusCode::OK, headers)
        }
        Err(_) => (StatusCode::NOT_FOUND, HeaderMap::new()),
    }
}

async fn delete_object_handler(
    State(service): State<Arc<ObjectStoreServiceImpl>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(DeleteObjectRequest { bucket, key });

    match service.delete_object(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                (StatusCode::NO_CONTENT, "".to_string())
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to delete object".to_string(),
                )
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.message().to_string()),
    }
}