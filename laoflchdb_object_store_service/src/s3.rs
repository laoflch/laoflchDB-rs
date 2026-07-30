//! S3 协议支持模块
//!
//! 实现标准 S3 协议：
//! - AWS Signature V4 签名验证
//! - XML 响应格式
//! - 独立的 S3 兼容路由

use std::collections::HashMap;
use std::sync::Arc;

use urlencoding;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, Request, StatusCode, Uri},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, head, put},
};
use tower_http::normalize_path::NormalizePathLayer;
use hmac::{Hmac, Mac};
use log;
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
///
/// 支持两种模式：
/// 1. Authorization 头认证（标准 HTTP 请求）
/// 2. 预签名 URL 认证（查询参数中的 X-Amz-Signature，rusty-s3 使用此方式）
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

    // 检查是否有 Authorization 头
    if let Some(auth_value) = headers.get("authorization").or_else(|| headers.get("Authorization")) {
        if let Ok(auth_str) = auth_value.to_str() {
            log::debug!("使用 Authorization 头认证");
            return verify_header_auth(config, method, uri, headers, body, auth_str);
        }
    }

    // 否则尝试预签名 URL 认证（查询参数中的 X-Amz-Signature）
    let query = uri.query().unwrap_or("");
    log::debug!("使用预签名 URL 认证，查询字符串: {}", query);
    verify_query_auth(config, method, uri, headers, body, query)
}

/// 验证 Authorization 头认证
fn verify_header_auth(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
    auth_header: &str,
) -> Result<(), String> {
    log::debug!("Authorization 头: {}", auth_header);

    let auth = parse_authorization_header(auth_header)?;
    log::debug!("解析后的 auth: access_key={}, date={}, region={}, service={}, signed_headers={:?}", auth.access_key, auth.date, auth.region, auth.service, auth.signed_headers);

    // 验证 access key
    if auth.access_key != config.access_key {
        return Err(format!("Access Key 无效 (期望={}, 实际={})", config.access_key, auth.access_key));
    }

    let timestamp = get_timestamp(headers)?;
    log::debug!("请求时间戳: {}", timestamp);

    // 构建 credential scope
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

    // 计算签名密钥（使用客户端的 region 和 service）
    let signing_key = compute_signing_key(&config.secret_key, &auth.date, &auth.region, &auth.service);
    log::debug!("Signing Key: {}", hex::encode(&signing_key));

    // 计算期望签名
    let expected_signature = compute_signature(&signing_key, &string_to_sign);
    log::debug!("期望签名: {}", expected_signature);
    log::debug!("实际签名: {}", auth.signature);

    if expected_signature != auth.signature {
        return Err(format!("签名验证失败 (期望={}, 实际={})", expected_signature, auth.signature));
    }

    log::debug!("=== S3 签名验证通过（Authorization 头） ===");
    Ok(())
}

/// 验证预签名 URL 认证（查询参数中的 X-Amz-Signature）
fn verify_query_auth(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    _body: &[u8],
    query: &str,
) -> Result<(), String> {
    // 解析查询参数
    let params = parse_query_string(query);

    let signature = params.get("X-Amz-Signature")
        .ok_or_else(|| "缺少 X-Amz-Signature".to_string())?;
    let credential = params.get("X-Amz-Credential")
        .ok_or_else(|| "缺少 X-Amz-Credential".to_string())?;
    let signed_headers_str = params.get("X-Amz-SignedHeaders")
        .ok_or_else(|| "缺少 X-Amz-SignedHeaders".to_string())?;
    let date = params.get("X-Amz-Date")
        .ok_or_else(|| "缺少 X-Amz-Date".to_string())?;
    let algorithm = params.get("X-Amz-Algorithm")
        .ok_or_else(|| "缺少 X-Amz-Algorithm".to_string())?;

    log::debug!("预签名 URL 参数: algorithm={}, credential={}, date={}, signed_headers={}",
        algorithm, credential, date, signed_headers_str);

    // 解析 Credential = access_key/date/region/service/aws4_request
    let cred_parts: Vec<&str> = credential.split('/').collect();
    if cred_parts.len() < 5 {
        return Err("X-Amz-Credential 格式无效".to_string());
    }
    let access_key = cred_parts[0];
    let cred_date = cred_parts[1];
    let region = cred_parts[2];
    let service = cred_parts[3];

    log::debug!("解析凭据: access_key={}, date={}, region={}, service={}", access_key, cred_date, region, service);

    // 验证 access key
    if access_key != config.access_key {
        return Err(format!("Access Key 无效 (期望={}, 实际={})", config.access_key, access_key));
    }

    // 验证算法
    if algorithm != "AWS4-HMAC-SHA256" {
        return Err(format!("不支持的签名算法: {}", algorithm));
    }

    // 构建规范查询字符串（移除 X-Amz-Signature）
    let canonical_querystring = build_canonical_query_string(query, &params);

    // 构建规范头
    let signed_headers_list: Vec<String> = signed_headers_str.split(';')
        .map(|s| s.trim().to_string())
        .collect();
    let mut canonical_headers = String::new();
    for h in &signed_headers_list {
        let h_lower = h.to_lowercase();
        if let Some(value) = headers.get(&h_lower) {
            if let Ok(v) = value.to_str() {
                canonical_headers.push_str(&format!("{}:{}\n", h_lower, v.trim()));
            }
        }
    }
    let signed_headers_str_joined = signed_headers_list.iter()
        .map(|h| h.to_lowercase())
        .collect::<Vec<_>>()
        .join(";");

    // 预签名 URL 使用 "UNSIGNED-PAYLOAD" 作为 payload hash
    let payload_hash = "UNSIGNED-PAYLOAD";

    // 构建规范请求
    let canonical_uri = uri.path();
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.as_str(),
        canonical_uri,
        canonical_querystring,
        canonical_headers,
        signed_headers_str_joined,
        payload_hash,
    );
    log::debug!("Canonical Request:\n{}\n", canonical_request);

    // 构建 credential scope
    let credential_scope = format!(
        "{}/{}/{}/aws4_request",
        cred_date, region, service
    );

    // 构建待签名字符串
    let string_to_sign = build_string_to_sign(date, &credential_scope, &canonical_request);
    log::debug!("StringToSign:\n{}\n", string_to_sign);

    // 计算签名密钥
    let signing_key = compute_signing_key(&config.secret_key, cred_date, region, service);
    log::debug!("Signing Key: {}", hex::encode(&signing_key));

    // 计算期望签名
    let expected_signature = compute_signature(&signing_key, &string_to_sign);
    log::debug!("期望签名: {}", expected_signature);
    log::debug!("实际签名: {}", signature);

    if expected_signature != *signature {
        return Err(format!("签名验证失败 (期望={}, 实际={})", expected_signature, signature));
    }

    log::debug!("=== S3 签名验证通过（预签名 URL） ===");
    Ok(())
}

/// 解析查询字符串为 KV 对
fn parse_query_string(query: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in query.split('&') {
        if let Some(idx) = pair.find('=') {
            let key = url_decode(&pair[..idx]);
            let value = url_decode(&pair[idx + 1..]);
            map.insert(key, value);
        } else {
            map.insert(url_decode(pair), String::new());
        }
    }
    map
}

/// 简单的 URL 解码
fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// 构建规范查询字符串（移除 X-Amz-Signature，按 key 排序）
/// 直接从原始查询字符串操作，不经过 decode→re-encode 循环，避免多字节 UTF-8 序列损坏
fn build_canonical_query_string(query: &str, _params: &std::collections::HashMap<String, String>) -> String {
    let mut pairs: Vec<(&str, &str)> = query.split('&')
        .filter_map(|pair| {
            if let Some(idx) = pair.find('=') {
                let key = &pair[..idx];
                if key == "X-Amz-Signature" {
                    return None;
                }
                Some((key, &pair[idx + 1..]))
            } else {
                Some((pair, ""))
            }
        })
        .collect();
    // 按 key 的字节序排序
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs.iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&")
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
    #[serde(rename = "NextContinuationToken")]
    #[serde(skip_serializing_if = "Option::is_none")]
    next_continuation_token: Option<String>,
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

/// 简单的路径标准化中间件，在路由匹配前运行
async fn normalize_path_middleware(
    req: Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let (mut parts, body) = req.into_parts();

    // 保存原始 URI（用于签名）
    let original_uri = parts.uri.clone();
    parts.extensions.insert(OriginalUri(original_uri.clone()));

    // 去除路径尾部斜杠（但保留根路径 "/"）
    let path_str = parts.uri.path().to_string(); // 先复制到 String，避免借用
    let query = parts.uri.query().map(|q| format!("?{}", q)).unwrap_or_default();
    if path_str != "/" && path_str.ends_with('/') {
        let new_path = path_str.trim_end_matches('/');
        if let Ok(new_uri) = format!("{}{}", new_path, query).parse::<Uri>() {
            parts.uri = new_uri;
            log::debug!("S3 标准化路径: {} -> {}", path_str, new_path);
        }
    }

    let req = Request::from_parts(parts, body);
    next.run(req).await
}

/// 保存原始 URI 的包装类型
#[derive(Debug, Clone)]
struct OriginalUri(Uri);

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

    // 从扩展中获取原始 URI（用于签名）
    let original_uri = parts.extensions.get::<OriginalUri>()
        .map(|ou| &ou.0)
        .unwrap_or(&parts.uri);

    log::debug!("S3 使用 URI 验证签名: {:?}", original_uri);

    // 验证签名
    if let Err(e) = verify_aws_v4_signature(
        &state.config,
        &parts.method,
        original_uri,
        &parts.headers,
        &body_bytes,
    ) {
        return s3_error_response("SignatureDoesNotMatch", &e, StatusCode::FORBIDDEN);
    }

    // 重建请求
    let req = Request::from_parts(parts, axum::body::Body::from(body_bytes));
    log::debug!("S3 签名验证通过，转发到路由处理");
    let resp = next.run(req).await;
    log::debug!("S3 路由响应: status={}", resp.status());
    resp
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
    log::debug!("head_bucket_handler 被调用: bucket={}", bucket);
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
                    Some(resp.next_marker.clone())
                } else {
                    None
                },
                next_continuation_token: if query.list_type.as_deref() == Some("2") && resp.is_truncated && !resp.next_marker.is_empty() {
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
            headers.insert("Last-Modified", timestamp_to_rfc7231(&resp.last_modified).parse().unwrap());
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
            headers.insert("Last-Modified", timestamp_to_rfc7231(&resp.last_modified).parse().unwrap());
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

/// 将 Unix 时间戳（秒）转换为 RFC 7231 格式（用于 Last-Modified HTTP 头）
fn timestamp_to_rfc7231(timestamp: &str) -> String {
    if let Ok(secs) = timestamp.parse::<i64>() {
        // RFC 7231 格式: Thu, 29 Jul 2026 06:09:09 GMT
        let naive = chrono::NaiveDateTime::from_timestamp_opt(secs, 0)
            .unwrap_or_default();
        let utc: chrono::DateTime<chrono::Utc> =
            chrono::DateTime::from_naive_utc_and_offset(naive, chrono::Utc);
        return utc.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    }
    // 无效时间戳，返回 epoch
    "Thu, 01 Jan 1970 00:00:00 GMT".to_string()
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

    // 统一使用 fallback 处理所有请求，手动解析路径和方法进行分发
    // 这样可以完全控制路径解析（包括尾部斜杠）和签名验证顺序
    Router::new()
        .fallback(s3_dispatch)
        .with_state(state)
}

/// 统一 S3 请求分发
async fn s3_dispatch(
    State(state): State<Arc<S3State>>,
    req: Request<axum::body::Body>,
) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();

    // 读取 body
    let body_bytes = axum::body::to_bytes(body, 50 * 1024 * 1024) // 50MB 限制
        .await
        .unwrap_or_default();

    log::debug!("S3 请求: method={}, uri={}", method, uri);

    // 验证签名（使用原始 URI）
    if let Err(e) = verify_aws_v4_signature(
        &state.config,
        &method,
        &uri,
        &parts.headers,
        &body_bytes,
    ) {
        log::warn!("S3 签名验证失败: {}", e);
        return s3_error_response("SignatureDoesNotMatch", &e, StatusCode::FORBIDDEN);
    }

    log::debug!("S3 签名验证通过");

    // 解析路径（去除尾部斜杠，但不包含根路径）
    let path = uri.path();
    let normalized_path = if path != "/" && path.ends_with('/') {
        path.trim_end_matches('/')
    } else {
        path
    };

    // 解析查询参数
    let query_str = uri.query().unwrap_or("");

    // 分发请求
    if normalized_path == "/" || normalized_path.is_empty() {
        // GET / - ListBuckets
        if method == Method::GET {
            return list_buckets_handler(State(state)).await.into_response();
        }
        return method_not_allowed(&method);
    }

    // 解析路径段: /{bucket} 或 /{bucket}/{key...}
    let path_segments: Vec<&str> = normalized_path.trim_start_matches('/')
        .splitn(2, '/')
        .collect();

    // URL 解码 bucket（bucket 名称通常简单，但以防万一）
    let bucket = {
        let decoded = urlencoding::decode_binary(path_segments[0].as_bytes());
        String::from_utf8_lossy(&decoded).to_string()
    };
    if bucket.is_empty() {
        return s3_error_response("InvalidRequest", "Bucket 名称不能为空", StatusCode::BAD_REQUEST);
    }

    // /{bucket} - 没有 key
    if path_segments.len() == 1 {
        match method {
            Method::GET => {
                // ListObjects
                return list_objects_handler_inner(&state, &bucket, query_str).await;
            }
            Method::HEAD => {
                // HeadBucket
                return head_bucket_handler_inner(&state, &bucket).await;
            }
            Method::PUT => {
                // CreateBucket
                return create_bucket_handler_inner(&state, &bucket).await;
            }
            Method::DELETE => {
                // DeleteBucket
                return delete_bucket_handler_inner(&state, &bucket).await;
            }
            _ => return method_not_allowed(&method),
        }
    }

    // /{bucket}/{key} - 有 key
    // URL 解码 key，在 RocksDB 中存储解码后的原始 key
    let key = {
        let decoded = urlencoding::decode_binary(path_segments[1].as_bytes());
        String::from_utf8_lossy(&decoded).to_string()
    };
    log::debug!("GET/PUT/DELETE object: bucket={}, key={}", bucket, key);
    match method {
        Method::GET => {
            // GetObject
            return get_object_handler_inner(&state, &bucket, &key).await;
        }
        Method::HEAD => {
            // HeadObject
            return head_object_handler_inner(&state, &bucket, &key).await;
        }
        Method::PUT => {
            // PutObject
            return put_object_handler_inner(&state, &bucket, &key, &parts.headers, &body_bytes).await;
        }
        Method::DELETE => {
            // DeleteObject
            return delete_object_handler_inner(&state, &bucket, &key).await;
        }
        _ => return method_not_allowed(&method),
    }
}

/// 返回 Method Not Allowed
fn method_not_allowed(method: &Method) -> Response {
    s3_error_response(
        "MethodNotAllowed",
        &format!("不支持的 HTTP 方法: {}", method),
        StatusCode::METHOD_NOT_ALLOWED,
    )
}

// ── 内部处理函数（直接调用，不经过 axum 路由提取器） ──

async fn list_objects_handler_inner(
    state: &Arc<S3State>,
    bucket: &str,
    query_str: &str,
) -> Response {
    // 解析查询参数并 URL 解码
    let query_params = query_str.split('&')
        .filter_map(|pair| {
            pair.split_once('=').map(|(k, v)| (k, v))
        })
        .collect::<Vec<_>>();
    let get_param = |name: &str| -> String {
        query_params.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| {
                // URL 解码参数值
                let decoded = urlencoding::decode_binary(v.as_bytes());
                String::from_utf8_lossy(&decoded).to_string()
            })
            .unwrap_or_default()
    };
    let prefix = get_param("prefix");
    let delimiter = get_param("delimiter");
    let max_keys: i32 = get_param("max-keys")
        .parse()
        .unwrap_or(1000)
        .max(1).min(1000);
    let marker = {
        let ct = get_param("continuation-token");
        if ct.is_empty() { get_param("marker") } else { ct }
    };

    let is_v2 = get_param("list-type") == "2";

    let req = tonic::Request::new(ListObjectsRequest {
        bucket: bucket.to_string(),
        prefix: prefix.clone(),
        delimiter: delimiter.clone(),
        max_keys,
        marker: marker.clone(),
        reverse: false,
    });

    match state.service.list_objects(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let contents: Vec<ObjectContent> = resp
                .objects
                .iter()
                .map(|o| ObjectContent {
                    // URL 编码 key 用于 S3 响应
                    key: urlencoding::encode(&o.key).to_string(),
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
                    // URL 编码前缀用于 S3 响应
                    prefix: urlencoding::encode(p).to_string(),
                })
                .collect();

            let result = ListBucketResult {
                name: resp.bucket.clone(),
                prefix: if prefix.is_empty() { String::new() } else { urlencoding::encode(&prefix).to_string() },
                max_keys,
                is_truncated: resp.is_truncated,
                contents,
                common_prefixes,
                marker: if marker.is_empty() { None } else { Some(urlencoding::encode(&marker).to_string()) },
                next_marker: if resp.is_truncated && !resp.next_marker.is_empty() {
                    Some(urlencoding::encode(&resp.next_marker).to_string())
                } else {
                    None
                },
                next_continuation_token: if is_v2 && resp.is_truncated && !resp.next_marker.is_empty() {
                    Some(urlencoding::encode(&resp.next_marker).to_string())
                } else {
                    None
                },
                delimiter: if delimiter.is_empty() { None } else { Some(urlencoding::encode(&delimiter).to_string()) },
            };

            match to_xml(&result) {
                Ok(xml) => {
                    log::debug!("list_objects 返回 XML（前 1000 字节）: {}", &xml[..xml.len().min(1000)]);
                    (
                        StatusCode::OK,
                        [("Content-Type", "application/xml")],
                        xml,
                    ).into_response()
                }
                Err(_) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [("Content-Type", "application/xml")],
                    String::new(),
                ).into_response(),
            }
        }
        Err(e) => {
            log::error!("list_objects 调用失败: bucket={}, err={}", bucket, e.message());
            (
            StatusCode::INTERNAL_SERVER_ERROR,
            [("Content-Type", "application/xml")],
            format!("<Error><Code>InternalError</Code><Message>{}</Message></Error>", e.message()),
        ).into_response()
        }
    }
}

async fn head_bucket_handler_inner(state: &Arc<S3State>, bucket: &str) -> Response {
    log::debug!("head_bucket_handler 被调用: bucket={}", bucket);
    let req = tonic::Request::new(ListObjectsRequest {
        bucket: bucket.to_string(),
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
            log::debug!("HeadBucket 成功: bucket={}, 对象数={}", bucket, resp.objects.len());
            (StatusCode::OK, headers).into_response()
        }
        Err(e) => {
            log::warn!("HeadBucket 失败: bucket={}, error={}", bucket, e.message());
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/xml".parse().unwrap());
            (StatusCode::NOT_FOUND, headers).into_response()
        }
    }
}

async fn create_bucket_handler_inner(state: &Arc<S3State>, bucket: &str) -> Response {
    if bucket.is_empty() || bucket.len() > 63 {
        return s3_error_response("InvalidBucketName", "Bucket 名称无效", StatusCode::BAD_REQUEST);
    }
    let req = tonic::Request::new(CreateBucketRequest { bucket: bucket.to_string() });
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

async fn delete_bucket_handler_inner(state: &Arc<S3State>, bucket: &str) -> Response {
    let req = tonic::Request::new(DeleteBucketRequest { bucket: bucket.to_string() });
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

async fn put_object_handler_inner(
    state: &Arc<S3State>,
    bucket: &str,
    key: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    let req = tonic::Request::new(PutObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
        data: body.to_vec(),
        content_type: content_type,
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
                log::error!("put_object 返回失败: bucket={}, key={}, msg={}", bucket, key, resp.message);
                s3_error_response("InternalError", &resp.message, StatusCode::INTERNAL_SERVER_ERROR)
            }
        }
        Err(e) => {
            log::error!("put_object 调用失败: bucket={}, key={}, err={}", bucket, key, e.message());
            s3_error_response("InternalError", &e.message(), StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn get_object_handler_inner(state: &Arc<S3State>, bucket: &str, key: &str) -> Response {
    let req = tonic::Request::new(GetObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
    });

    match state.service.get_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", resp.content_type.parse().unwrap_or("application/octet-stream".parse().unwrap()));
            headers.insert("Content-Length", resp.content_length.to_string().parse().unwrap());
            headers.insert("ETag", resp.etag.parse().unwrap_or("\"\"".parse().unwrap()));
            headers.insert("Last-Modified", timestamp_to_rfc7231(&resp.last_modified).parse().unwrap());
            (StatusCode::OK, headers, resp.data).into_response()
        }
        Err(e) => {
            let error = S3Error {
                code: "NoSuchKey".to_string(),
                message: e.message().to_string(),
                resource: None,
                request_id: None,
            };
            let xml = to_xml(&error).unwrap_or_default();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", "application/xml".parse().unwrap());
            (StatusCode::NOT_FOUND, headers, xml.into_bytes()).into_response()
        }
    }
}

async fn head_object_handler_inner(state: &Arc<S3State>, bucket: &str, key: &str) -> Response {
    let req = tonic::Request::new(HeadObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
    });

    match state.service.head_object(req).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            let mut headers = HeaderMap::new();
            headers.insert("Content-Type", resp.content_type.parse().unwrap_or("application/octet-stream".parse().unwrap()));
            headers.insert("Content-Length", resp.content_length.to_string().parse().unwrap());
            headers.insert("ETag", resp.etag.parse().unwrap_or("\"\"".parse().unwrap()));
            headers.insert("Last-Modified", timestamp_to_rfc7231(&resp.last_modified).parse().unwrap());
            (StatusCode::OK, headers).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, HeaderMap::new()).into_response(),
    }
}

async fn delete_object_handler_inner(state: &Arc<S3State>, bucket: &str, key: &str) -> Response {
    let req = tonic::Request::new(DeleteObjectRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
    });

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