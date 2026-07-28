//! S3 协议支持模块
//!
//! 实现标准 S3 协议：
//! - AWS Signature V4 签名验证
//! - XML 响应格式
//! - 独立的 S3 兼容路由

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, Request, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, head, put},
};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::proto::object_store_service_server::ObjectStoreService;
use crate::proto::*;
use crate::ObjectStoreServiceImpl;

type HmacSha256 = Hmac<Sha256>;

// ── S3 配置 ──

/// S3 协议配置
#[derive(Debug, Clone)]
pub struct S3Config {
    /// S3 Access Key
    pub access_key: String,
    /// S3 Secret Key
    pub secret_key: String,
    /// 默认 region
    pub region: String,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            access_key: "admin".to_string(),
            secret_key: "laoflchdb".to_string(),
            region: "us-east-1".to_string(),
        }
    }
}

// ── AWS Signature V4 验证 ──

/// 解析后的 AWS Signature V4 认证信息
struct AwsV4Auth {
    access_key: String,
    date: String,
    region: String,
    service: String,
    signed_headers: Vec<String>,
    signature: String,
}

/// 解析 Authorization 头
fn parse_authorization_header(header: &str) -> Result<AwsV4Auth, String> {
    if !header.starts_with("AWS4-HMAC-SHA256 ") {
        return Err("不支持的签名算法".to_string());
    }

    let rest = header.strip_prefix("AWS4-HMAC-SHA256 ").unwrap_or("");
    let mut access_key = String::new();
    let mut date = String::new();
    let mut region = String::new();
    let mut service = String::new();
    let mut signed_headers = Vec::new();
    let mut signature = String::new();

    // 格式: Credential=<ak>/<date>/<region>/<service>/aws4_request, SignedHeaders=<h1>;<h2>, Signature=<sig>
    for part in rest.split(", ") {
        if let Some(value) = part.strip_prefix("Credential=") {
            let parts: Vec<&str> = value.split('/').collect();
            if parts.len() >= 5 {
                access_key = parts[0].to_string();
                date = parts[1].to_string();
                region = parts[2].to_string();
                service = parts[3].to_string();
            }
        } else if let Some(value) = part.strip_prefix("SignedHeaders=") {
            signed_headers = value.split(';').map(|s| s.to_string()).collect();
        } else if let Some(value) = part.strip_prefix("Signature=") {
            signature = value.to_string();
        }
    }

    if access_key.is_empty() || signature.is_empty() {
        return Err("Authorization 头格式无效".to_string());
    }

    Ok(AwsV4Auth {
        access_key,
        date,
        region,
        service,
        signed_headers,
        signature,
    })
}

/// 获取请求时间戳
fn get_timestamp(headers: &HeaderMap) -> Result<String, String> {
    if let Some(x_amz_date) = headers.get("x-amz-date") {
        return Ok(x_amz_date.to_str().unwrap_or("").to_string());
    }
    if let Some(date) = headers.get("date") {
        return Ok(date.to_str().unwrap_or("").to_string());
    }
    Err("缺少 x-amz-date 或 date 头".to_string())
}

/// 获取 payload hash（x-amz-content-sha256 头，或计算 body 的 SHA256）
fn get_payload_hash(headers: &HeaderMap, body: &[u8]) -> String {
    if let Some(hash) = headers.get("x-amz-content-sha256") {
        return hash.to_str().unwrap_or("").to_string();
    }
    // 未提供时计算 body 的 SHA256
    let mut hasher = Sha256::new();
    hasher.update(body);
    hex::encode(hasher.finalize())
}

/// 构建规范请求（CanonicalRequest）
fn build_canonical_request(
    method: &str,
    uri: &Uri,
    signed_headers: &[String],
    headers: &HeaderMap,
    payload_hash: &str,
) -> String {
    let canonical_uri = uri.path();
    let canonical_query = uri.query().unwrap_or("");

    // 规范查询字符串需要 URL 编码排序，S3 一般使用原始查询字符串
    let canonical_querystring = if canonical_query.is_empty() {
        String::new()
    } else {
        // 按 key 排序
        let mut params: Vec<(&str, &str)> = Vec::new();
        for pair in canonical_query.split('&') {
            if let Some(idx) = pair.find('=') {
                let key = &pair[..idx];
                let value = &pair[idx + 1..];
                params.push((key, value));
            } else {
                params.push((pair, ""));
            }
        }
        params.sort_by(|a, b| a.0.cmp(b.0));
        params
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    };

    // 规范头：按 signed_headers 顺序提取，小写，trim 值
    let mut canonical_headers = String::new();
    for h in signed_headers {
        if let Some(value) = headers.get(h) {
            if let Ok(v) = value.to_str() {
                canonical_headers.push_str(&format!(
                    "{}:{}\n",
                    h.to_lowercase(),
                    v.trim()
                ));
            }
        }
    }

    let signed_headers_str = signed_headers
        .iter()
        .map(|h| h.to_lowercase())
        .collect::<Vec<_>>()
        .join(";");

    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        canonical_uri,
        canonical_querystring,
        canonical_headers,
        signed_headers_str,
        payload_hash,
    )
}

/// 构建待签名字符串（StringToSign）
fn build_string_to_sign(timestamp: &str, credential_scope: &str, canonical_request: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_request.as_bytes());
    let hash = hex::encode(hasher.finalize());

    format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        timestamp, credential_scope, hash
    )
}

/// 计算签名密钥
fn compute_signing_key(secret_key: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_secret = format!("AWS4{}", secret_key);

    let mut mac = HmacSha256::new_from_slice(k_secret.as_bytes()).expect("HMAC 初始化失败");
    mac.update(date.as_bytes());
    let k_date = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_date).expect("HMAC 初始化失败");
    mac.update(region.as_bytes());
    let k_region = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_region).expect("HMAC 初始化失败");
    mac.update(service.as_bytes());
    let k_service = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&k_service).expect("HMAC 初始化失败");
    mac.update(b"aws4_request");
    mac.finalize().into_bytes().to_vec()
}

/// 计算签名
fn compute_signature(signing_key: &[u8], string_to_sign: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(signing_key).expect("HMAC 初始化失败");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    hex::encode(result)
}

/// 验证 AWS Signature V4 签名
fn verify_aws_v4_signature(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), String> {
    log::debug!("=== S3 签名验证开始 ===");
    log::debug!("方法: {:?}", method);
    log::debug!("URI: {:?}", uri);
    log::debug!("请求头:");
    for (k, v) in headers.iter() {
        log::debug!("  {}: {:?}", k, v);
    }
    log::debug!("Body 长度: {}", body.len());

    let auth_header = headers
        .get("authorization")
        .or_else(|| headers.get("Authorization"))
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "缺少 Authorization 头".to_string())?;
    log::debug!("Authorization 头: {}", auth_header);

    let auth = parse_authorization_header(auth_header)?;
    log::debug!("解析后的 auth: access_key={}, date={}, region={}, service={}, signed_headers={:?}", auth.access_key, auth.date, auth.region, auth.service, auth.signed_headers);

    // 验证 access key
    if auth.access_key != config.access_key {
        return Err(format!("Access Key 无效 (期望={}, 实际={})", config.access_key, auth.access_key));
    }

    let timestamp = get_timestamp(headers)?;
    log::debug!("请求时间戳: {}", timestamp);

    // 构建 credential scope (使用客户端的 region 和 service，而不是硬编码！)
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        auth.date, auth.region, auth.service
    );
    log::debug!("Credential Scope: {}", credential_scope);

    // 获取 payload hash
    let payload_hash = get_payload_hash(headers, body);
    log::debug!("Payload Hash: {}", payload_hash);

    // 构建规范请求
    let canonical_request = build_canonical_request(
        method.as_str(),
        uri,
        &auth.signed_headers,
        headers,
        &payload_hash,
    );
    log::debug!("Canonical Request:\n{}\n", canonical_request);

    // 构建待签名字符串
    let string_to_sign = build_string_to_sign(&timestamp, &credential_scope, &canonical_request);
    log::debug!("StringToSign:\n{}\n", string_to_sign);

    // 计算签名密钥 (使用客户端的 region 和 service)
    let signing_key = compute_signing_key(&config.secret_key, &auth.date, &auth.region, &auth.service);
    log::debug!("Signing Key: {}", hex::encode(&signing_key));

    // 计算期望签名
    let expected_signature = compute_signature(&signing_key, &string_to_sign);
    log::debug!("期望签名: {}", expected_signature);
    log::debug!("实际签名: {}", auth.signature);

    // 比较签名（常量时间比较）
    if expected_signature != auth.signature {
        return Err(format!("签名验证失败 (期望={}, 实际={})", expected_signature, auth.signature));
    }

    log::debug!("=== S3 签名验证通过 ===");
    Ok(())
}

// ── XML 响应结构 ──

/// S3 错误响应
#[derive(serde::Serialize)]
struct S3Error {
    #[serde(rename = "Code")]
    code: String,
    #[serde(rename = "Message")]
    message: String,
    #[serde(rename = "Resource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
    #[serde(rename = "RequestId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

/// ListAllMyBucketsResult
#[derive(serde::Serialize)]
struct ListAllMyBucketsResult {
    #[serde(rename = "Owner")]
    owner: Owner,
    #[serde(rename = "Buckets")]
    buckets: Buckets,
}

#[derive(serde::Serialize)]
struct Owner {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "DisplayName")]
    display_name: String,
}

#[derive(serde::Serialize)]
struct Buckets {
    #[serde(rename = "Bucket")]
    bucket: Vec<BucketInfo>,
}

#[derive(serde::Serialize)]
struct BucketInfo {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "CreationDate")]
    creation_date: String,
}

/// ListBucketResult
#[derive(serde::Serialize)]
struct ListBucketResult {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Prefix")]
    prefix: String,
    #[serde(rename = "MaxKeys")]
    max_keys: i32,
    #[serde(rename = "IsTruncated")]
    is_truncated: bool,
    #[serde(rename = "Contents")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    contents: Vec<ObjectContent>,
    #[serde(rename = "CommonPrefixes")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    common_prefixes: Vec<CommonPrefix>,
    #[serde(rename = "Marker")]
    #[serde(skip_serializing_if = "Option::is_none")]
    marker: Option<String>,
    #[serde(rename = "NextMarker")]
    #[serde(skip_serializing_if = "Option::is_none")]
    next_marker: Option<String>,
    #[serde(rename = "Delimiter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    delimiter: Option<String>,
}

#[derive(serde::Serialize)]
struct ObjectContent {
    #[serde(rename = "Key")]
    key: String,
    #[serde(rename = "LastModified")]
    last_modified: String,
    #[serde(rename = "ETag")]
    etag: String,
    #[serde(rename = "Size")]
    size: i64,
    #[serde(rename = "StorageClass")]
    #[serde(skip_serializing_if = "Option::is_none")]
    storage_class: Option<String>,
    #[serde(rename = "Owner")]
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<Owner>,
}

#[derive(serde::Serialize)]
struct CommonPrefix {
    #[serde(rename = "Prefix")]
    prefix: String,
}

/// CopyObjectResult
#[derive(serde::Serialize)]
struct CopyObjectResult {
    #[serde(rename = "LastModified")]
    last_modified: String,
    #[serde(rename = "ETag")]
    etag: String,
}

/// 生成 XML 响应
fn to_xml<T: serde::Serialize>(value: &T) -> Result<String, String> {
    quick_xml::se::to_string(value).map_err(|e| format!("XML 序列化失败: {}", e))
}

/// 生成 S3 错误响应
fn s3_error_response(code: &str, message: &str, status_code: StatusCode) -> Response {
    let error = S3Error {
        code: code.to_string(),
        message: message.to_string(),
        resource: None,
        request_id: None,
    };
    let xml = to_xml(&error).unwrap_or_default();
    Response::builder()
        .status(status_code)
        .header("Content-Type", "application/xml")
        .body(axum::body::Body::from(xml))
        .unwrap()
}

// ── S3 认证中间件 ──

/// S3 认证中间件
async fn s3_auth(
    State(state): State<Arc<S3State>>,
    req: Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let (parts, body) = req.into_parts();

    // 读取 body
    let body_bytes = axum::body::to_bytes(body, 5 * 1024 * 1024) // 5MB 限制
        .await
        .unwrap_or_default();

    // 验证签名
    if let Err(e) = verify_aws_v4_signature(
        &state.config,
        &parts.method,
        &parts.uri,
        &parts.headers,
        &body_bytes,
    ) {
        return s3_error_response("SignatureDoesNotMatch", &e, StatusCode::FORBIDDEN);
    }

    // 重建请求
    let req = Request::from_parts(parts, axum::body::Body::from(body_bytes));
    next.run(req).await
}

// ── S3 请求处理函数 ──

/// S3 共享状态
struct S3State {
    service: Arc<ObjectStoreServiceImpl>,
    config: Arc<S3Config>,
}

/// 列出所有 Bucket (GET /)
async fn list_buckets_handler(
    State(state): State<Arc<S3State>>,
) -> impl IntoResponse {
    let req = tonic::Request::new(ListBucketsRequest {});
    match state.service.list_buckets(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let buckets: Vec<BucketInfo> = resp
                .buckets
                .iter()
                .map(|b| BucketInfo {
                    name: b.name.clone(),
                    creation_date: timestamp_to_iso(&b.creation_date),
                })
                .collect();

            let result = ListAllMyBucketsResult {
                owner: Owner {
                    id: state.config.access_key.clone(),
                    display_name: state.config.access_key.clone(),
                },
                buckets: Buckets { bucket: buckets },
            };

            match to_xml(&result) {
                Ok(xml) => (
                    StatusCode::OK,
                    [("Content-Type", "application/xml")],
                    xml,
                ),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [("Content-Type", "application/xml")],
                    String::new(),
                ),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("Content-Type", "application/xml")],
            format!("<Error><Code>InternalError</Code><Message>{}</Message></Error>", e.message()),
        ),
    }
}

/// 创建 Bucket (PUT /{bucket})
async fn create_bucket_handler(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    // 验证 bucket 名称
    if bucket.is_empty() || bucket.len() > 63 {
        return s3_error_response("InvalidBucketName", "Bucket 名称无效", StatusCode::BAD_REQUEST);
    }

    let req = tonic::Request::new(CreateBucketRequest { bucket });
    match state.service.create_bucket(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                Response::builder()
                    .status(StatusCode::OK)
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else {
                s3_error_response("InternalError", "创建 Bucket 失败", StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
        Err(e) => s3_error_response("InternalError", &e.message(), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// 检查 Bucket 是否存在 (HEAD /{bucket})
async fn head_bucket_handler(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    // 通过列出对象检查 bucket 是否存在（max_keys=1，没有 key 表示只检查 bucket）
    let req = tonic::Request::new(ListObjectsRequest {
        bucket: bucket.clone(),
        prefix: String::new(),
        delimiter: String::new(),
        max_keys: 1,
        marker: String::new(),
        reverse: false,
    });
    match state.service.list_objects(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let mut headers = HeaderMap::new();
            headers.insert("x-amz-bucket-region", state.config.region.parse().unwrap());
            headers.insert("Content-Length", "0".parse().unwrap());
            (StatusCode::OK, headers)
        }
        Err(_) => {
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/xml".parse().unwrap());
            (StatusCode::NOT_FOUND, headers)
        }
    }
}

/// 删除 Bucket (DELETE /{bucket})
async fn delete_bucket_handler(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    let req = tonic::Request::new(DeleteBucketRequest { bucket });
    match state.service.delete_bucket(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else {
                s3_error_response("InternalError", "删除 Bucket 失败", StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
        Err(e) => s3_error_response("InternalError", &e.message(), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// S3 ListObjects 查询参数
#[derive(serde::Deserialize, Default)]
struct ListObjectsQuery {
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    delimiter: String,
    #[serde(default = "default_max_keys")]
    #[serde(rename = "max-keys")]
    max_keys: i32,
    #[serde(default)]
    marker: String,
    #[serde(default)]
    #[serde(rename = "list-type")]
    list_type: Option<String>,
    #[serde(default)]
    #[serde(rename = "continuation-token")]
    continuation_token: Option<String>,
}

fn default_max_keys() -> i32 {
    1000
}

/// 列出对象 (GET /{bucket})
async fn list_objects_handler(
    State(state): State<Arc<S3State>>,
    Path(bucket): Path<String>,
    Query(query): Query<ListObjectsQuery>,
) -> impl IntoResponse {
    let max_keys = query.max_keys.max(1).min(1000);
    let marker = query.continuation_token.clone().unwrap_or(query.marker.clone());

    let req = tonic::Request::new(ListObjectsRequest {
        bucket,
        prefix: query.prefix.clone(),
        delimiter: query.delimiter.clone(),
        max_keys,
        marker,
        reverse: false,
    });

    match state.service.list_objects(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let contents: Vec<ObjectContent> = resp
                .objects
                .iter()
                .map(|o| ObjectContent {
                    key: o.key.clone(),
                    last_modified: timestamp_to_iso(&o.last_modified),
                    etag: o.etag.clone(),
                    size: o.size,
                    storage_class: Some("STANDARD".to_string()),
                    owner: None,
                })
                .collect();

            let common_prefixes: Vec<CommonPrefix> = resp
                .common_prefixes
                .iter()
                .map(|p| CommonPrefix {
                    prefix: p.clone(),
                })
                .collect();

            let result = ListBucketResult {
                name: resp.bucket.clone(),
                prefix: query.prefix,
                max_keys,
                is_truncated: resp.is_truncated,
                contents,
                common_prefixes,
                marker: if query.marker.is_empty() {
                    None
                } else {
                    Some(query.marker)
                },
                next_marker: if resp.is_truncated && !resp.next_marker.is_empty() {
                    Some(resp.next_marker)
                } else {
                    None
                },
                delimiter: if query.delimiter.is_empty() {
                    None
                } else {
                    Some(query.delimiter)
                },
            };

            match to_xml(&result) {
                Ok(xml) => (
                    StatusCode::OK,
                    [("Content-Type", "application/xml")],
                    xml,
                ),
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [("Content-Type", "application/xml")],
                    String::new(),
                ),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("Content-Type", "application/xml")],
            format!("<Error><Code>InternalError</Code><Message>{}</Message></Error>", e.message()),
        ),
    }
}

/// 上传对象 (PUT /{bucket}/{key})
async fn put_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let req = tonic::Request::new(PutObjectRequest {
        bucket,
        key,
        data: body.to_vec(),
        content_type,
        metadata: HashMap::new(),
    });

    match state.service.put_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success {
                Response::builder()
                    .status(StatusCode::OK)
                    .header("ETag", resp.etag.clone())
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else {
                s3_error_response("InternalError", &resp.message, StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
        Err(e) => s3_error_response("InternalError", &e.message(), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// 获取对象 (GET /{bucket}/{key})
async fn get_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(GetObjectRequest { bucket, key });

    match state.service.get_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", resp.content_type.parse().unwrap_or("application/octet-stream".parse().unwrap()));
            headers.insert("Content-Length", resp.content_length.to_string().parse().unwrap());
            headers.insert("ETag", resp.etag.parse().unwrap_or("\"\"".parse().unwrap()));
            headers.insert("Last-Modified", resp.content_length.to_string().parse().unwrap_or("0".parse().unwrap()));
            (StatusCode::OK, headers, resp.data)
        }
        Err(e) => {
            // 返回 S3 风格的 404 错误
            let error = S3Error {
                code: "NoSuchKey".to_string(),
                message: e.message().to_string(),
                resource: None,
                request_id: None,
            };
            let xml = to_xml(&error).unwrap_or_default();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/xml".parse().unwrap());
            (StatusCode::NOT_FOUND, headers, xml.into_bytes())
        }
    }
}

/// 获取对象元数据 (HEAD /{bucket}/{key})
async fn head_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(HeadObjectRequest { bucket, key });

    match state.service.head_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", resp.content_type.parse().unwrap_or("application/octet-stream".parse().unwrap()));
            headers.insert("Content-Length", resp.content_length.to_string().parse().unwrap());
            headers.insert("ETag", resp.etag.parse().unwrap_or("\"\"".parse().unwrap()));
            headers.insert("Last-Modified", resp.last_modified.parse().unwrap_or("0".parse().unwrap()));
            (StatusCode::OK, headers)
        }
        Err(_) => (StatusCode::NOT_FOUND, HeaderMap::new()),
    }
}

/// 删除对象 (DELETE /{bucket}/{key})
async fn delete_object_handler(
    State(state): State<Arc<S3State>>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = key.strip_prefix('/').unwrap_or(&key).to_string();
    let req = tonic::Request::new(DeleteObjectRequest { bucket, key });

    match state.service.delete_object(req).await {
        Ok(resp) => {
            if resp.into_inner().success {
                Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(axum::body::Body::empty())
                    .unwrap()
            } else {
                s3_error_response("InternalError", "删除对象失败", StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
        Err(e) => s3_error_response("InternalError", &e.message(), StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// ── 辅助函数 ──

/// 将 Unix 时间戳（秒）转换为 ISO 8601 格式
fn timestamp_to_iso(timestamp: &str) -> String {
    if let Ok(secs) = timestamp.parse::<i64>() {
        if let Ok(dt) = OffsetDateTime::from_unix_timestamp(secs) {
            if let Ok(formatted) = dt.format(&Rfc3339) {
                return formatted;
            }
        }
    }
    // 如果是 ISO 格式或无效，直接返回
    timestamp.to_string()
}

// ── 创建 S3 Router ──

/// 创建 S3 协议 Router
///
/// 返回的 Router 已绑定状态，可在独立端口上运行。
/// 路由路径符合标准 S3 协议（无 `/api/v1/object-store/` 前缀）。
pub fn create_s3_router(
    service: Arc<ObjectStoreServiceImpl>,
    s3_config: S3Config,
) -> Router {
    let state = Arc::new(S3State {
        service,
        config: Arc::new(s3_config),
    });

    let router = Router::new()
        // GET / - ListBuckets
        .route("/", get(list_buckets_handler))
        // PUT /{bucket} - CreateBucket
        .route("/{bucket}", put(create_bucket_handler))
        // DELETE /{bucket} - DeleteBucket
        .route("/{bucket}", delete(delete_bucket_handler))
        // GET /{bucket} - ListObjects
        .route("/{bucket}", get(list_objects_handler))
        // HEAD /{bucket} - HeadBucket
        .route("/{bucket}", head(head_bucket_handler))
        // PUT /{bucket}/{key} - PutObject
        .route("/{bucket}/{key}", put(put_object_handler))
        // GET /{bucket}/{key} - GetObject
        .route("/{bucket}/{key}", get(get_object_handler))
        // HEAD /{bucket}/{key} - HeadObject
        .route("/{bucket}/{key}", head(head_object_handler))
        // DELETE /{bucket}/{key} - DeleteObject
        .route("/{bucket}/{key}", delete(delete_object_handler))
        .layer(middleware::from_fn_with_state(state.clone(), s3_auth))
        .with_state(state);

    router
}