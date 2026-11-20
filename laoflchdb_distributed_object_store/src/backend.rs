//! S3 后端抽象与分布式实现

use crate::chunk_io::{self, ChunkRef, ChunkWriteConfig};
use crate::config::DistributedObjectStoreConfig;
use crate::error::S3Error;
use crate::raft_meta::RaftMetaClient;
use crate::registry::RawStoreRegistry;
use async_trait::async_trait;
use axum::body::Body;
use laoflchdb_raw_store_service::client::RawStoreClientPool;
use laoflchdb_raw_store_service::registry::now_secs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

// ─────────────────────────── 协议层数据结构 ───────────────────────────

#[derive(Debug, Clone)]
pub struct S3BucketInfo {
    pub name: String,
    pub creation_date: u64,
}

#[derive(Debug, Clone, Default)]
pub struct S3ListQuery {
    pub prefix: String,
    pub delimiter: Option<String>,
    pub max_keys: usize,
    pub marker: Option<String>,
    pub reverse: bool,
    /// 是否为 ListObjectsV2（list-type=2），决定响应里输出 ContinuationToken 还是 Marker
    pub list_type_v2: bool,
    /// 客户端回传的 continuation-token（已解码后的原始 key），仅用于响应回显
    pub continuation_token: Option<String>,
    /// V2 的 start-after 参数，仅用于响应回显
    pub start_after: Option<String>,
}

#[derive(Debug, Clone)]
pub struct S3ObjectInfo {
    pub key: String,
    pub size: u64,
    pub last_modified: u64,
    pub etag: String,
}

#[derive(Debug, Clone, Default)]
pub struct S3ListResult {
    pub objects: Vec<S3ObjectInfo>,
    pub common_prefixes: Vec<String>,
    pub is_truncated: bool,
    pub next_marker: Option<String>,
}

#[derive(Debug, Clone)]
pub struct S3PutResult {
    pub etag: String,
}

pub struct S3GetResult {
    pub body: Body,
    pub content_length: u64,
    pub total_size: u64,
    pub content_type: String,
    pub etag: String,
    pub last_modified: u64,
    pub user_metadata: HashMap<String, String>,
    pub range: Option<(u64, u64)>,
}

#[derive(Debug, Clone)]
pub struct S3HeadResult {
    pub content_length: u64,
    pub content_type: String,
    pub etag: String,
    pub last_modified: u64,
    pub user_metadata: HashMap<String, String>,
}

// ─────────────────────────── 后端 trait ───────────────────────────

#[async_trait]
pub trait S3Backend: Send + Sync + 'static {
    async fn list_buckets(&self) -> Result<Vec<S3BucketInfo>, S3Error>;
    async fn create_bucket(&self, bucket: &str) -> Result<(), S3Error>;
    async fn delete_bucket(&self, bucket: &str) -> Result<(), S3Error>;
    async fn bucket_exists(&self, bucket: &str) -> Result<bool, S3Error>;

    async fn get_bucket_policy(&self, bucket: &str) -> Result<Option<String>, S3Error>;
    async fn put_bucket_policy(&self, bucket: &str, policy: &str) -> Result<(), S3Error>;
    async fn delete_bucket_policy(&self, bucket: &str) -> Result<(), S3Error>;

    async fn list_objects(&self, bucket: &str, query: S3ListQuery)
        -> Result<S3ListResult, S3Error>;

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        metadata: HashMap<String, String>,
        body: Body,
    ) -> Result<S3PutResult, S3Error>;

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
        range: Option<(u64, u64)>,
    ) -> Result<S3GetResult, S3Error>;

    async fn head_object(&self, bucket: &str, key: &str) -> Result<S3HeadResult, S3Error>;
    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3Error>;
    async fn delete_objects(&self, bucket: &str, keys: &[String]) -> Result<Vec<String>, S3Error>;
    async fn copy_object(
        &self,
        src: (&str, &str),
        dst: (&str, &str),
    ) -> Result<String, S3Error>;

    async fn create_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        metadata: HashMap<String, String>,
    ) -> Result<String, S3Error> {
        let _ = (bucket, key, content_type, metadata);
        Err(S3Error::unsupported("multipart upload"))
    }

    async fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: i32,
        body: Body,
    ) -> Result<String, S3Error> {
        let _ = (bucket, key, upload_id, part_number, body);
        Err(S3Error::unsupported("multipart upload"))
    }

    async fn complete_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: Vec<(i32, String)>,
    ) -> Result<String, S3Error> {
        let _ = (bucket, key, upload_id, parts);
        Err(S3Error::unsupported("multipart upload"))
    }

    async fn abort_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), S3Error> {
        let _ = (bucket, key, upload_id);
        Err(S3Error::unsupported("multipart upload"))
    }
}

// ─────────────────────────── 元数据模型 ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BucketMeta {
    name: String,
    creation_date: u64,
    #[serde(default)]
    policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ObjectMeta {
    key: String,
    size: u64,
    etag: String,
    content_type: String,
    last_modified: u64,
    #[serde(default)]
    user_metadata: HashMap<String, String>,
    chunks: Vec<ChunkRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadMeta {
    upload_id: String,
    bucket: String,
    key: String,
    content_type: String,
    #[serde(default)]
    user_metadata: HashMap<String, String>,
    created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PartMeta {
    part_number: i32,
    size: u64,
    etag: String,
    chunks: Vec<ChunkRef>,
}

// ─────────────────────────── 元数据键 ───────────────────────────

fn bucket_key(bucket: &str) -> Vec<u8> {
    format!("b\0{bucket}").into_bytes()
}
fn object_key(bucket: &str, key: &str) -> Vec<u8> {
    format!("o\0{bucket}\0{key}").into_bytes()
}
fn object_prefix(bucket: &str) -> Vec<u8> {
    format!("o\0{bucket}\0").into_bytes()
}
fn upload_key(bucket: &str, upload_id: &str) -> Vec<u8> {
    format!("u\0{bucket}\0{upload_id}").into_bytes()
}
fn part_key(bucket: &str, upload_id: &str, part_number: i32) -> Vec<u8> {
    format!("p\0{bucket}\0{upload_id}\0{part_number:05}").into_bytes()
}
fn part_prefix(bucket: &str, upload_id: &str) -> Vec<u8> {
    format!("p\0{bucket}\0{upload_id}\0").into_bytes()
}

fn to_json<T: Serialize>(value: &T) -> Result<Vec<u8>, S3Error> {
    serde_json::to_vec(value).map_err(|e| S3Error::internal(format!("元数据序列化失败: {e}")))
}

fn from_json<T: for<'de> Deserialize<'de>>(value: &[u8]) -> Result<T, S3Error> {
    serde_json::from_slice(value).map_err(|e| S3Error::internal(format!("元数据解析失败: {e}")))
}

// ─────────────────────────── 分布式实现 ───────────────────────────

pub struct DistributedObjectStore {
    meta: RaftMetaClient,
    registry: Arc<RawStoreRegistry>,
    pool: Arc<RawStoreClientPool>,
    config: Arc<DistributedObjectStoreConfig>,
}

impl DistributedObjectStore {
    pub fn new(
        meta: RaftMetaClient,
        registry: Arc<RawStoreRegistry>,
        pool: Arc<RawStoreClientPool>,
        config: Arc<DistributedObjectStoreConfig>,
    ) -> Self {
        Self {
            meta,
            registry,
            pool,
            config,
        }
    }

    fn chunk_config(&self) -> ChunkWriteConfig {
        ChunkWriteConfig {
            chunk_size: self.config.chunk_size_bytes.max(1),
            replication_factor: self.config.replication_factor.max(1),
            write_quorum: self.config.effective_write_quorum(),
        }
    }

    async fn bucket_meta(&self, bucket: &str) -> Result<Option<BucketMeta>, S3Error> {
        match self.meta.get(bucket_key(bucket)).await? {
            Some(value) => Ok(Some(from_json(&value)?)),
            None => Ok(None),
        }
    }

    async fn require_bucket(&self, bucket: &str) -> Result<BucketMeta, S3Error> {
        self.bucket_meta(bucket)
            .await?
            .ok_or_else(|| S3Error::no_such_bucket(bucket))
    }

    async fn object_meta(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMeta>, S3Error> {
        match self.meta.get(object_key(bucket, key)).await? {
            Some(value) => Ok(Some(from_json(&value)?)),
            None => Ok(None),
        }
    }

    async fn store_object_meta(
        &self,
        bucket: &str,
        key: &str,
        object: &ObjectMeta,
    ) -> Result<(), S3Error> {
        let old = self.meta.get(object_key(bucket, key)).await?;
        self.meta
            .put(object_key(bucket, key), to_json(object)?)
            .await?;
        if let Some(old) = old {
            if let Ok(old_meta) = from_json::<ObjectMeta>(&old) {
                chunk_io::delete_chunks(&self.pool, &old_meta.chunks).await;
            }
        }
        Ok(())
    }

    async fn delete_meta_prefix(&self, prefix: &[u8]) -> Result<(), S3Error> {
        let entries = self.meta.scan().await?;
        for (key, _) in entries {
            if key.starts_with(prefix) {
                let _ = self.meta.delete(key).await;
            }
        }
        Ok(())
    }

    async fn get_upload(&self, bucket: &str, upload_id: &str) -> Result<UploadMeta, S3Error> {
        match self.meta.get(upload_key(bucket, upload_id)).await? {
            Some(value) => from_json(&value),
            None => Err(S3Error::new(
                axum::http::StatusCode::NOT_FOUND,
                "NoSuchUpload",
                "The specified multipart upload does not exist",
            )),
        }
    }
}

#[async_trait]
impl S3Backend for DistributedObjectStore {
    async fn list_buckets(&self) -> Result<Vec<S3BucketInfo>, S3Error> {
        let entries = self.meta.scan().await?;
        let mut buckets = Vec::new();
        for (key, value) in entries {
            if !key.starts_with(b"b\0") {
                continue;
            }
            if let Ok(meta) = from_json::<BucketMeta>(&value) {
                buckets.push(S3BucketInfo {
                    name: meta.name,
                    creation_date: meta.creation_date,
                });
            }
        }
        buckets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(buckets)
    }

    async fn create_bucket(&self, bucket: &str) -> Result<(), S3Error> {
        if self.bucket_meta(bucket).await?.is_some() {
            return Ok(());
        }
        let meta = BucketMeta {
            name: bucket.to_string(),
            creation_date: now_secs(),
            policy: None,
        };
        self.meta.put(bucket_key(bucket), to_json(&meta)?).await
    }

    async fn delete_bucket(&self, bucket: &str) -> Result<(), S3Error> {
        self.require_bucket(bucket).await?;
        let prefix = object_prefix(bucket);
        let entries = self.meta.scan().await?;
        if entries.iter().any(|(key, _)| key.starts_with(&prefix)) {
            return Err(S3Error::bucket_not_empty(bucket));
        }
        self.meta.delete(bucket_key(bucket)).await
    }

    async fn bucket_exists(&self, bucket: &str) -> Result<bool, S3Error> {
        Ok(self.bucket_meta(bucket).await?.is_some())
    }

    async fn get_bucket_policy(&self, bucket: &str) -> Result<Option<String>, S3Error> {
        Ok(self.require_bucket(bucket).await?.policy)
    }

    async fn put_bucket_policy(&self, bucket: &str, policy: &str) -> Result<(), S3Error> {
        let mut meta = self.require_bucket(bucket).await?;
        meta.policy = Some(policy.to_string());
        self.meta.put(bucket_key(bucket), to_json(&meta)?).await
    }

    async fn delete_bucket_policy(&self, bucket: &str) -> Result<(), S3Error> {
        let mut meta = self.require_bucket(bucket).await?;
        meta.policy = None;
        self.meta.put(bucket_key(bucket), to_json(&meta)?).await
    }

    async fn list_objects(
        &self,
        bucket: &str,
        query: S3ListQuery,
    ) -> Result<S3ListResult, S3Error> {
        self.require_bucket(bucket).await?;

        let prefix = object_prefix(bucket);
        let entries = self.meta.scan().await?;
        let mut objects: Vec<ObjectMeta> = Vec::new();
        for (key, value) in entries {
            if !key.starts_with(&prefix) {
                continue;
            }
            if let Ok(meta) = from_json::<ObjectMeta>(&value) {
                objects.push(meta);
            }
        }
        objects.sort_by(|a, b| a.key.cmp(&b.key));
        if query.reverse {
            objects.reverse();
        }

        let max_keys = if query.max_keys == 0 { 1000 } else { query.max_keys };
        let mut result_objects = Vec::new();
        let mut common_prefixes: Vec<String> = Vec::new();
        let mut count = 0usize;
        let mut is_truncated = false;
        let mut next_marker: Option<String> = None;

        for meta in objects {
            if !meta.key.starts_with(&query.prefix) {
                continue;
            }

            if let Some(marker) = &query.marker {
                if !query.reverse {
                    if meta.key.as_str() <= marker.as_str() {
                        continue;
                    }
                } else if meta.key.as_str() >= marker.as_str() {
                    continue;
                }
            }

            if let Some(delimiter) = &query.delimiter {
                if !delimiter.is_empty() {
                    let rest = &meta.key[query.prefix.len()..];
                    if let Some(pos) = rest.find(delimiter.as_str()) {
                        let cp = format!("{}{}", query.prefix, &rest[..pos + delimiter.len()]);
                        // 已翻过的 common prefix 不能再返回一次，否则客户端会拿到重复前缀并原地打转
                        if let Some(marker) = &query.marker {
                            if !query.reverse && cp.as_str() <= marker.as_str() {
                                continue;
                            }
                            if query.reverse && cp.as_str() >= marker.as_str() {
                                continue;
                            }
                        }
                        if !common_prefixes.iter().any(|p| p == &cp) {
                            if count >= max_keys {
                                is_truncated = true;
                                break;
                            }
                            common_prefixes.push(cp.clone());
                            count += 1;
                            next_marker = Some(cp);
                        }
                        continue;
                    }
                }
            }

            if count >= max_keys {
                is_truncated = true;
                break;
            }

            result_objects.push(S3ObjectInfo {
                key: meta.key.clone(),
                size: meta.size,
                last_modified: meta.last_modified,
                etag: meta.etag.clone(),
            });
            count += 1;
            next_marker = Some(meta.key);
        }

        Ok(S3ListResult {
            objects: result_objects,
            common_prefixes,
            is_truncated,
            next_marker,
        })
    }

    async fn put_object(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        metadata: HashMap<String, String>,
        body: Body,
    ) -> Result<S3PutResult, S3Error> {
        self.require_bucket(bucket).await?;
        let cfg = self.chunk_config();
        let (chunks, size, etag_hex) =
            chunk_io::write_object_stream(&self.registry, &self.pool, &cfg, body).await?;
        let etag = format!("\"{etag_hex}\"");
        let object = ObjectMeta {
            key: key.to_string(),
            size,
            etag: etag.clone(),
            content_type: content_type.to_string(),
            last_modified: now_secs(),
            user_metadata: metadata,
            chunks,
        };

        if let Err(e) = self.store_object_meta(bucket, key, &object).await {
            chunk_io::delete_chunks(&self.pool, &object.chunks).await;
            return Err(e);
        }

        Ok(S3PutResult { etag })
    }

    async fn get_object(
        &self,
        bucket: &str,
        key: &str,
        range: Option<(u64, u64)>,
    ) -> Result<S3GetResult, S3Error> {
        let meta = self
            .object_meta(bucket, key)
            .await?
            .ok_or_else(|| S3Error::no_such_key(bucket, key))?;

        let total = meta.size;
        let applied_range = match range {
            Some((start, end)) => {
                if start >= total {
                    return Err(S3Error::invalid_range(total));
                }
                let end = end.min(total.saturating_sub(1));
                if start > end {
                    return Err(S3Error::invalid_range(total));
                }
                Some((start, end))
            }
            None => None,
        };

        let content_length = match applied_range {
            Some((start, end)) => end - start + 1,
            None => total,
        };

        let body = chunk_io::read_object_stream(
            meta.chunks.clone(),
            applied_range,
            self.pool.clone(),
        );

        Ok(S3GetResult {
            body,
            content_length,
            total_size: total,
            content_type: meta.content_type,
            etag: meta.etag,
            last_modified: meta.last_modified,
            user_metadata: meta.user_metadata,
            range: applied_range,
        })
    }

    async fn head_object(&self, bucket: &str, key: &str) -> Result<S3HeadResult, S3Error> {
        let meta = self
            .object_meta(bucket, key)
            .await?
            .ok_or_else(|| S3Error::no_such_key(bucket, key))?;
        Ok(S3HeadResult {
            content_length: meta.size,
            content_type: meta.content_type,
            etag: meta.etag,
            last_modified: meta.last_modified,
            user_metadata: meta.user_metadata,
        })
    }

    async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), S3Error> {
        let old = self.meta.get(object_key(bucket, key)).await?;
        self.meta.delete(object_key(bucket, key)).await?;
        if let Some(value) = old {
            if let Ok(meta) = from_json::<ObjectMeta>(&value) {
                chunk_io::delete_chunks(&self.pool, &meta.chunks).await;
            }
        }
        Ok(())
    }

    async fn delete_objects(
        &self,
        bucket: &str,
        keys: &[String],
    ) -> Result<Vec<String>, S3Error> {
        for key in keys {
            let _ = self.delete_object(bucket, key).await;
        }
        Ok(keys.to_vec())
    }

    async fn copy_object(
        &self,
        src: (&str, &str),
        dst: (&str, &str),
    ) -> Result<String, S3Error> {
        let (src_bucket, src_key) = src;
        let (dst_bucket, dst_key) = dst;
        self.require_bucket(dst_bucket).await?;

        let src_meta = self
            .object_meta(src_bucket, src_key)
            .await?
            .ok_or_else(|| S3Error::no_such_key(src_bucket, src_key))?;

        let cfg = self.chunk_config();
        let mut new_chunks = Vec::with_capacity(src_meta.chunks.len());
        let mut offset = 0u64;
        for chunk in &src_meta.chunks {
            let data = chunk_io::fetch_chunk(&self.pool, chunk).await?;
            let new_chunk =
                chunk_io::write_chunk_data(&self.registry, &self.pool, &cfg, &data, offset).await?;
            offset += data.len() as u64;
            new_chunks.push(new_chunk);
        }

        let object = ObjectMeta {
            key: dst_key.to_string(),
            size: src_meta.size,
            etag: src_meta.etag.clone(),
            content_type: src_meta.content_type.clone(),
            last_modified: now_secs(),
            user_metadata: src_meta.user_metadata.clone(),
            chunks: new_chunks,
        };

        if let Err(e) = self.store_object_meta(dst_bucket, dst_key, &object).await {
            chunk_io::delete_chunks(&self.pool, &object.chunks).await;
            return Err(e);
        }

        Ok(src_meta.etag)
    }

    async fn create_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        content_type: &str,
        metadata: HashMap<String, String>,
    ) -> Result<String, S3Error> {
        self.require_bucket(bucket).await?;
        let upload_id = uuid::Uuid::new_v4().simple().to_string();
        let upload = UploadMeta {
            upload_id: upload_id.clone(),
            bucket: bucket.to_string(),
            key: key.to_string(),
            content_type: content_type.to_string(),
            user_metadata: metadata,
            created_at: now_secs(),
        };
        self.meta
            .put(upload_key(bucket, &upload_id), to_json(&upload)?)
            .await?;
        Ok(upload_id)
    }

    async fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: i32,
        body: Body,
    ) -> Result<String, S3Error> {
        let upload = self.get_upload(bucket, upload_id).await?;
        if upload.key != key {
            return Err(S3Error::bad_request("uploadId 与 key 不匹配"));
        }
        if !(1..=10000).contains(&part_number) {
            return Err(S3Error::bad_request("partNumber 必须在 1..=10000"));
        }

        let cfg = self.chunk_config();
        let (chunks, size, etag_hex) =
            chunk_io::write_object_stream(&self.registry, &self.pool, &cfg, body).await?;
        let part = PartMeta {
            part_number,
            size,
            etag: format!("\"{etag_hex}\""),
            chunks,
        };

        let key_bytes = part_key(bucket, upload_id, part_number);
        let old = self.meta.get(key_bytes.clone()).await?;
        self.meta.put(key_bytes, to_json(&part)?).await?;
        if let Some(old) = old {
            if let Ok(old_part) = from_json::<PartMeta>(&old) {
                chunk_io::delete_chunks(&self.pool, &old_part.chunks).await;
            }
        }

        Ok(part.etag)
    }

    async fn complete_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: Vec<(i32, String)>,
    ) -> Result<String, S3Error> {
        let upload = self.get_upload(bucket, upload_id).await?;
        if upload.key != key {
            return Err(S3Error::bad_request("uploadId 与 key 不匹配"));
        }
        if parts.is_empty() {
            return Err(S3Error::bad_request("至少需要一个 part"));
        }

        let part_count = parts.len();
        let mut chunks: Vec<ChunkRef> = Vec::new();
        let mut offset = 0u64;
        let mut size = 0u64;
        let mut etag_concat = String::new();

        for (part_number, expected_etag) in parts {
            let value = self
                .meta
                .get(part_key(bucket, upload_id, part_number))
                .await?
                .ok_or_else(|| S3Error::bad_request(format!("part {part_number} 不存在")))?;
            let part: PartMeta = from_json(&value)?;
            if !expected_etag.is_empty()
                && expected_etag.trim_matches('"') != part.etag.trim_matches('"')
            {
                return Err(S3Error::bad_request(format!(
                    "part {part_number} etag 不匹配"
                )));
            }
            for mut chunk in part.chunks {
                chunk.offset += offset;
                chunks.push(chunk);
            }
            offset += part.size;
            size += part.size;
            etag_concat.push_str(&part.etag);
        }

        let etag = format!(
            "\"{}-{}\"",
            hex::encode(Sha256::digest(etag_concat.as_bytes())),
            part_count
        );
        let object = ObjectMeta {
            key: key.to_string(),
            size,
            etag: etag.clone(),
            content_type: upload.content_type,
            last_modified: now_secs(),
            user_metadata: upload.user_metadata,
            chunks,
        };

        if let Err(e) = self.store_object_meta(bucket, key, &object).await {
            chunk_io::delete_chunks(&self.pool, &object.chunks).await;
            return Err(e);
        }

        // part 的 chunk 已被对象引用，只清理 part 元数据与 upload 记录
        let _ = self.delete_meta_prefix(&part_prefix(bucket, upload_id)).await;
        let _ = self.meta.delete(upload_key(bucket, upload_id)).await;

        Ok(etag)
    }

    async fn abort_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), S3Error> {
        let upload = self.get_upload(bucket, upload_id).await?;
        if upload.key != key {
            return Err(S3Error::bad_request("uploadId 与 key 不匹配"));
        }

        let prefix = part_prefix(bucket, upload_id);
        let entries = self.meta.scan().await?;
        for (meta_key, value) in entries {
            if !meta_key.starts_with(&prefix) {
                continue;
            }
            if let Ok(part) = from_json::<PartMeta>(&value) {
                chunk_io::delete_chunks(&self.pool, &part.chunks).await;
            }
            let _ = self.meta.delete(meta_key).await;
        }
        let _ = self.meta.delete(upload_key(bucket, upload_id)).await;
        Ok(())
    }
}
