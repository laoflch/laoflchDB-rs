//! 分布式对象存储自带的 S3 协议层（SigV4 / BucketPolicy / Range / List / Multipart）。
//!
//! 说明：本模块不依赖现有 `laoflchdb_object_store_service`，现有单机对象存储零改动。

use crate::backend::{
    S3Backend, S3BucketInfo, S3HeadResult, S3ListQuery, S3ListResult,
};
use crate::error::S3Error;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Request, Response, StatusCode, Uri};
use axum::Router;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use time::{Month, OffsetDateTime, Weekday};

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const MAX_XML_BODY: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
}

#[derive(Clone)]
struct S3State {
    backend: Arc<dyn S3Backend>,
    config: Arc<S3Config>,
}

pub fn create_s3_router(backend: Arc<dyn S3Backend>, config: S3Config) -> Router {
    let state = S3State {
        backend,
        config: Arc::new(config),
    };
    Router::new().fallback(s3_dispatch).with_state(state)
}

async fn s3_dispatch(State(state): State<S3State>, req: Request<Body>) -> Response<Body> {
    match handle_request(state, req).await {
        Ok(resp) => resp,
        Err(err) => error_response(err),
    }
}

async fn handle_request(state: S3State, req: Request<Body>) -> Result<Response<Body>, S3Error> {
    let (parts, body) = req.into_parts();
    let method = parts.method;
    let uri = parts.uri;
    let headers = parts.headers;
    let query = QueryParams::parse(uri.query());
    let (bucket, key) = parse_path(uri.path());

    // 尽量不预读 body：SigV4 优先使用 x-amz-content-sha256
    let content_sha = header_str(&headers, "x-amz-content-sha256");
    let presigned = query.contains("X-Amz-Algorithm");
    let need_buffer = content_sha.is_none()
        && !presigned
        && !matches!(method.as_str(), "GET" | "HEAD" | "DELETE");

    let (body, payload_hash) = if let Some(hash) = content_sha {
        (body, hash)
    } else if presigned {
        (body, "UNSIGNED-PAYLOAD".to_string())
    } else if need_buffer {
        let bytes = axum::body::to_bytes(body, MAX_XML_BODY)
            .await
            .map_err(|e| S3Error::bad_request(format!("读取请求体失败: {e}")))?;
        let hash = sha256_hex(&bytes);
        (Body::from(bytes), hash)
    } else {
        (body, EMPTY_SHA256.to_string())
    };

    let has_auth = headers.contains_key("authorization") || presigned;
    if has_auth {
        verify_signature(&state.config, &method, &uri, &headers, &query, &payload_hash)?;
    } else {
        check_anonymous(&state, &method, bucket.as_deref(), key.as_deref()).await?;
    }

    route(state, method, uri, headers, query, bucket, key, body).await
}

async fn route(
    state: S3State,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    query: QueryParams,
    bucket: Option<String>,
    key: Option<String>,
    body: Body,
) -> Result<Response<Body>, S3Error> {
    match (method.as_str(), bucket.as_deref(), key.as_deref()) {
        ("GET", None, None) => {
            let buckets = state.backend.list_buckets().await?;
            Ok(xml_response(StatusCode::OK, list_buckets_xml(&buckets)))
        }
        ("GET", Some(b), None) => {
            if query.contains("policy") {
                match state.backend.get_bucket_policy(b).await? {
                    Some(policy) => Ok(Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Body::from(policy))
                        .map_err(|e| S3Error::internal(e.to_string()))?),
                    None => Err(S3Error::no_such_policy()),
                }
            } else if query.contains("location") {
                Ok(xml_response(
                    StatusCode::OK,
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">{}</LocationConstraint>",
                        xml_escape(&state.config.region)
                    ),
                ))
            } else if query.contains("uploads") {
                Ok(xml_response(StatusCode::OK, list_multipart_uploads_empty_xml(b)))
            } else {
                let list_query = parse_list_query(&query);
                let result = state.backend.list_objects(b, list_query.clone()).await?;
                Ok(xml_response(
                    StatusCode::OK,
                    list_objects_xml(b, &list_query, &result),
                ))
            }
        }
        ("HEAD", Some(b), None) => {
            if state.backend.bucket_exists(b).await? {
                Ok(empty_response(StatusCode::OK))
            } else {
                Err(S3Error::no_such_bucket(b))
            }
        }
        ("PUT", Some(b), None) => {
            if query.contains("policy") {
                let bytes = read_body(body).await?;
                let policy = String::from_utf8(bytes.to_vec())
                    .map_err(|_| S3Error::bad_request("policy 必须是 UTF-8"))?;
                state.backend.put_bucket_policy(b, &policy).await?;
                Ok(empty_response(StatusCode::NO_CONTENT))
            } else {
                state.backend.create_bucket(b).await?;
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("Location", format!("/{b}"))
                    .body(Body::empty())
                    .map_err(|e| S3Error::internal(e.to_string()))?)
            }
        }
        ("DELETE", Some(b), None) => {
            if query.contains("policy") {
                state.backend.delete_bucket_policy(b).await?;
            } else {
                state.backend.delete_bucket(b).await?;
            }
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        ("POST", Some(b), None) => {
            if query.contains("delete") {
                let bytes = read_body(body).await?;
                let text = String::from_utf8_lossy(&bytes);
                let keys = parse_delete_objects(&text);
                let deleted = state.backend.delete_objects(b, &keys).await?;
                Ok(xml_response(StatusCode::OK, delete_result_xml(&deleted)))
            } else {
                Err(S3Error::bad_request("不支持的 POST 操作"))
            }
        }
        ("GET", Some(b), Some(k)) => {
            if query.contains("uploadId") {
                Err(S3Error::unsupported("ListParts 暂未实现"))
            } else {
                get_object_response(&state, b, k, &headers).await
            }
        }
        ("HEAD", Some(b), Some(k)) => head_object_response(&state, b, k).await,
        ("PUT", Some(b), Some(k)) => {
            if let Some(copy_source) = header_str(&headers, "x-amz-copy-source") {
                let (src_bucket, src_key) = parse_copy_source(&copy_source)?;
                let etag = state
                    .backend
                    .copy_object((&src_bucket, &src_key), (b, k))
                    .await?;
                Ok(xml_response(StatusCode::OK, copy_result_xml(&etag)))
            } else if query.contains("partNumber") && query.contains("uploadId") {
                let part_number: i32 = query
                    .get("partNumber")
                    .unwrap()
                    .parse()
                    .map_err(|_| S3Error::bad_request("partNumber 非法"))?;
                let upload_id = query.get("uploadId").unwrap().clone();
                let etag = state
                    .backend
                    .upload_part(b, k, &upload_id, part_number, body)
                    .await?;
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("ETag", etag)
                    .body(Body::empty())
                    .map_err(|e| S3Error::internal(e.to_string()))?)
            } else {
                let content_type = header_str(&headers, "content-type")
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let metadata = extract_user_metadata(&headers);
                let result = state
                    .backend
                    .put_object(b, k, &content_type, metadata, body)
                    .await?;
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("ETag", result.etag)
                    .body(Body::empty())
                    .map_err(|e| S3Error::internal(e.to_string()))?)
            }
        }
        ("DELETE", Some(b), Some(k)) => {
            if let Some(upload_id) = query.get("uploadId") {
                state
                    .backend
                    .abort_multipart_upload(b, k, upload_id)
                    .await?;
            } else {
                state.backend.delete_object(b, k).await?;
            }
            Ok(empty_response(StatusCode::NO_CONTENT))
        }
        ("POST", Some(b), Some(k)) => {
            if query.contains("uploads") {
                let content_type = header_str(&headers, "content-type")
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let metadata = extract_user_metadata(&headers);
                let upload_id = state
                    .backend
                    .create_multipart_upload(b, k, &content_type, metadata)
                    .await?;
                Ok(xml_response(
                    StatusCode::OK,
                    initiate_multipart_xml(b, k, &upload_id),
                ))
            } else if let Some(upload_id) = query.get("uploadId") {
                let bytes = read_body(body).await?;
                let text = String::from_utf8_lossy(&bytes);
                let parts = parse_complete_multipart(&text);
                if parts.is_empty() {
                    return Err(S3Error::bad_request(
                        "CompleteMultipartUpload 缺少 Part 列表",
                    ));
                }
                let etag = state
                    .backend
                    .complete_multipart_upload(b, k, upload_id, parts)
                    .await?;
                Ok(xml_response(
                    StatusCode::OK,
                    complete_multipart_xml(b, k, &etag),
                ))
            } else {
                Err(S3Error::bad_request("不支持的 POST 操作"))
            }
        }
        _ => Err(S3Error::unsupported(format!(
            "不支持的请求: {} {}",
            method,
            uri.path()
        ))),
    }
}

async fn read_body(body: Body) -> Result<Bytes, S3Error> {
    axum::body::to_bytes(body, MAX_XML_BODY)
        .await
        .map_err(|e| S3Error::bad_request(format!("读取请求体失败: {e}")))
}

// ─────────────────────────── GET / HEAD 对象 ───────────────────────────

async fn get_object_response(
    state: &S3State,
    bucket: &str,
    key: &str,
    headers: &HeaderMap,
) -> Result<Response<Body>, S3Error> {
    let head = state.backend.head_object(bucket, key).await?;
    let range = parse_range_header(headers, head.content_length)?;
    let result = state.backend.get_object(bucket, key, range).await?;

    let mut builder = Response::builder()
        .status(if result.range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header("Content-Type", result.content_type)
        .header("ETag", result.etag)
        .header("Last-Modified", format_http_date(result.last_modified))
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", result.content_length.to_string());

    for (k, v) in &result.user_metadata {
        builder = builder.header(format!("x-amz-meta-{k}"), v);
    }
    if let Some((start, end)) = result.range {
        builder = builder.header(
            "Content-Range",
            format!("bytes {start}-{end}/{}", result.total_size),
        );
    }

    builder
        .body(result.body)
        .map_err(|e| S3Error::internal(e.to_string()))
}

async fn head_object_response(
    state: &S3State,
    bucket: &str,
    key: &str,
) -> Result<Response<Body>, S3Error> {
    let head: S3HeadResult = state.backend.head_object(bucket, key).await?;
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", head.content_type)
        .header("ETag", head.etag)
        .header("Last-Modified", format_http_date(head.last_modified))
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", head.content_length.to_string());
    for (k, v) in &head.user_metadata {
        builder = builder.header(format!("x-amz-meta-{k}"), v);
    }
    builder
        .body(Body::empty())
        .map_err(|e| S3Error::internal(e.to_string()))
}

fn parse_range_header(
    headers: &HeaderMap,
    total: u64,
) -> Result<Option<(u64, u64)>, S3Error> {
    let Some(value) = header_str(headers, "range") else {
        return Ok(None);
    };
    let value = value.trim();
    let Some(spec) = value.strip_prefix("bytes=") else {
        return Ok(None);
    };
    if spec.contains(',') {
        // 多区间暂不支持，回退全量
        return Ok(None);
    }
    let Some((start_text, end_text)) = spec.split_once('-') else {
        return Ok(None);
    };
    if total == 0 {
        return Err(S3Error::invalid_range(total));
    }

    let (start, end) = if start_text.is_empty() {
        let suffix: u64 = end_text
            .parse()
            .map_err(|_| S3Error::invalid_range(total))?;
        if suffix == 0 {
            return Err(S3Error::invalid_range(total));
        }
        (total.saturating_sub(suffix), total - 1)
    } else {
        let start: u64 = start_text
            .parse()
            .map_err(|_| S3Error::invalid_range(total))?;
        let end = if end_text.is_empty() {
            total - 1
        } else {
            end_text
                .parse::<u64>()
                .map_err(|_| S3Error::invalid_range(total))?
        };
        if start >= total || start > end {
            return Err(S3Error::invalid_range(total));
        }
        (start, end.min(total - 1))
    };

    Ok(Some((start, end)))
}

// ─────────────────────────── 匿名访问与 BucketPolicy ───────────────────────────

async fn check_anonymous(
    state: &S3State,
    method: &Method,
    bucket: Option<&str>,
    key: Option<&str>,
) -> Result<(), S3Error> {
    let Some(bucket) = bucket else {
        return Err(S3Error::access_denied());
    };
    if !matches!(method.as_str(), "GET" | "HEAD") {
        return Err(S3Error::access_denied());
    }

    let policy = state
        .backend
        .get_bucket_policy(bucket)
        .await?
        .ok_or_else(S3Error::access_denied)?;

    let (action, resource) = match key {
        Some(key) => (
            "s3:GetObject",
            format!("arn:aws:s3:::{bucket}/{key}"),
        ),
        None => ("s3:ListBucket", format!("arn:aws:s3:::{bucket}")),
    };

    if policy_allows(&policy, action, &resource) {
        Ok(())
    } else {
        Err(S3Error::access_denied())
    }
}

fn policy_allows(policy: &str, action: &str, resource: &str) -> bool {
    let value: serde_json::Value = match serde_json::from_str(policy) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let statements = match value.get("Statement") {
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(item) => vec![item.clone()],
        None => return false,
    };

    for statement in statements {
        let effect = statement
            .get("Effect")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !effect.eq_ignore_ascii_case("Allow") {
            continue;
        }
        if !principal_is_public(statement.get("Principal")) {
            continue;
        }
        if !statement_value_matches(statement.get("Action"), action) {
            continue;
        }
        if !statement_value_matches(statement.get("Resource"), resource) {
            continue;
        }
        return true;
    }
    false
}

fn principal_is_public(principal: Option<&serde_json::Value>) -> bool {
    match principal {
        Some(serde_json::Value::String(s)) => s == "*",
        Some(serde_json::Value::Object(map)) => map
            .get("AWS")
            .map(|v| value_contains_star(v))
            .unwrap_or(false),
        _ => false,
    }
}

fn value_contains_star(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s == "*",
        serde_json::Value::Array(items) => items
            .iter()
            .any(|v| v.as_str().map(|s| s == "*").unwrap_or(false)),
        _ => false,
    }
}

fn statement_value_matches(value: Option<&serde_json::Value>, expected: &str) -> bool {
    match value {
        Some(serde_json::Value::String(pattern)) => pattern_matches(pattern, expected),
        Some(serde_json::Value::Array(items)) => items.iter().any(|item| {
            item.as_str()
                .map(|pattern| pattern_matches(pattern, expected))
                .unwrap_or(false)
        }),
        _ => false,
    }
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    pattern == value
}

// ─────────────────────────── SigV4 ───────────────────────────

type HmacSha256 = Hmac<Sha256>;

fn verify_signature(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    query: &QueryParams,
    payload_hash: &str,
) -> Result<(), S3Error> {
    if let Some(authorization) = header_str(headers, "authorization") {
        return verify_header_auth(config, method, uri, headers, &authorization, payload_hash);
    }
    if query.contains("X-Amz-Algorithm") {
        return verify_presigned(config, method, uri, headers, query);
    }
    Err(S3Error::access_denied())
}

fn verify_header_auth(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    authorization: &str,
    payload_hash: &str,
) -> Result<(), S3Error> {
    let rest = authorization
        .strip_prefix("AWS4-HMAC-SHA256 ")
        .ok_or_else(S3Error::signature_mismatch)?;

    let mut fields: HashMap<&str, &str> = HashMap::new();
    for part in rest.split(',') {
        if let Some((key, value)) = part.trim().split_once('=') {
            fields.insert(key.trim(), value.trim());
        }
    }

    let credential = fields
        .get("Credential")
        .copied()
        .ok_or_else(S3Error::signature_mismatch)?;
    let signed_headers = fields
        .get("SignedHeaders")
        .copied()
        .ok_or_else(S3Error::signature_mismatch)?;
    let signature = fields
        .get("Signature")
        .copied()
        .ok_or_else(S3Error::signature_mismatch)?;
    let amz_date = header_str(headers, "x-amz-date").ok_or_else(S3Error::signature_mismatch)?;

    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() != 5 {
        return Err(S3Error::signature_mismatch());
    }
    let access_key = parts[0];
    let date = parts[1];
    let region = parts[2];
    let service = parts[3];
    if access_key != config.access_key {
        return Err(S3Error::invalid_access_key());
    }

    let canonical_request = build_canonical_request(
        method.as_str(),
        uri,
        query_string_from_uri(uri),
        headers,
        signed_headers,
        payload_hash,
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signing_key = signing_key(&config.secret_key, date, region, service);
    let expected = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    if expected == signature {
        Ok(())
    } else {
        Err(S3Error::signature_mismatch())
    }
}

fn verify_presigned(
    config: &S3Config,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    query: &QueryParams,
) -> Result<(), S3Error> {
    let algorithm = query
        .get("X-Amz-Algorithm")
        .ok_or_else(S3Error::signature_mismatch)?;
    if algorithm != "AWS4-HMAC-SHA256" {
        return Err(S3Error::signature_mismatch());
    }

    let credential = query
        .get("X-Amz-Credential")
        .ok_or_else(S3Error::signature_mismatch)?;
    let amz_date = query
        .get("X-Amz-Date")
        .ok_or_else(S3Error::signature_mismatch)?;
    let signed_headers = query
        .get("X-Amz-SignedHeaders")
        .ok_or_else(S3Error::signature_mismatch)?;
    let signature = query
        .get("X-Amz-Signature")
        .ok_or_else(S3Error::signature_mismatch)?;
    let expires: i64 = query
        .get("X-Amz-Expires")
        .ok_or_else(S3Error::signature_mismatch)?
        .parse()
        .map_err(|_| S3Error::signature_mismatch())?;

    let request_time = parse_amz_date(amz_date).ok_or_else(S3Error::signature_mismatch)?;
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if now > request_time.unix_timestamp() + expires {
        return Err(S3Error::new(
            StatusCode::FORBIDDEN,
            "AccessDenied",
            "Request has expired",
        ));
    }

    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() != 5 {
        return Err(S3Error::signature_mismatch());
    }
    let access_key = parts[0];
    let date = parts[1];
    let region = parts[2];
    let service = parts[3];
    if access_key != config.access_key {
        return Err(S3Error::invalid_access_key());
    }

    let canonical_request = build_canonical_request(
        method.as_str(),
        uri,
        query.canonical(true),
        headers,
        signed_headers,
        "UNSIGNED-PAYLOAD",
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signing_key = signing_key(&config.secret_key, date, region, service);
    let expected = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    if expected == *signature {
        Ok(())
    } else {
        Err(S3Error::signature_mismatch())
    }
}

fn build_canonical_request(
    method: &str,
    uri: &Uri,
    canonical_query: String,
    headers: &HeaderMap,
    signed_headers: &str,
    payload_hash: &str,
) -> String {
    let canonical_uri = if uri.path().is_empty() {
        "/".to_string()
    } else {
        uri.path().to_string()
    };

    let mut canonical_headers = String::new();
    for name in signed_headers.split(';') {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        let value = if name == "host" {
            header_str(headers, "host")
                .or_else(|| uri.authority().map(|a| a.to_string()))
                .unwrap_or_default()
        } else {
            headers
                .get_all(name.as_str())
                .iter()
                .filter_map(|v| v.to_str().ok())
                .collect::<Vec<_>>()
                .join(",")
        };
        canonical_headers.push_str(&format!("{name}:{}\n", value.trim()));
    }

    format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    )
}

fn query_string_from_uri(uri: &Uri) -> String {
    QueryParams::parse(uri.query()).canonical(true)
}

fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC 支持任意长度 key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn parse_amz_date(value: &str) -> Option<OffsetDateTime> {
    let format = time::format_description::parse(
        "[year][month][day]T[hour][minute][second]Z",
    )
    .ok()?;
    OffsetDateTime::parse(value, &format).ok()
}

// ─────────────────────────── 查询参数 / 路径 ───────────────────────────

#[derive(Debug, Clone, Default)]
struct QueryParams(Vec<(String, String)>);

impl QueryParams {
    fn parse(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        let pairs = raw
            .split('&')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let (key, value) = part.split_once('=').unwrap_or((part, ""));
                (percent_decode(key), percent_decode(value))
            })
            .collect();
        Self(pairs)
    }

    fn get(&self, name: &str) -> Option<&String> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    fn canonical(&self, exclude_signature: bool) -> String {
        let mut pairs: Vec<(String, String)> = self
            .0
            .iter()
            .filter(|(key, _)| !(exclude_signature && key == "X-Amz-Signature"))
            .map(|(key, value)| (uri_encode(key, true), uri_encode(value, true)))
            .collect();
        pairs.sort();
        pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&")
    }
}

fn parse_path(path: &str) -> (Option<String>, Option<String>) {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() {
        return (None, None);
    }
    match trimmed.split_once('/') {
        Some((bucket, key)) => (
            Some(percent_decode(bucket)),
            if key.is_empty() {
                None
            } else {
                Some(percent_decode(key))
            },
        ),
        None => (Some(percent_decode(trimmed)), None),
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b'/' if !encode_slash => out.push('/'),
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

// ─────────────────────────── 请求头辅助 ───────────────────────────

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
}

fn extract_user_metadata(headers: &HeaderMap) -> HashMap<String, String> {
    let mut metadata = HashMap::new();
    for (name, value) in headers.iter() {
        let name = name.as_str().to_ascii_lowercase();
        if let Some(key) = name.strip_prefix("x-amz-meta-") {
            if let Ok(value) = value.to_str() {
                metadata.insert(key.to_string(), value.to_string());
            }
        }
    }
    metadata
}

fn parse_copy_source(value: &str) -> Result<(String, String), S3Error> {
    let value = value.trim();
    let value = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))
        .unwrap_or(value);
    let value = if value.contains("://") {
        value.split_once("://").map(|(_, rest)| rest).unwrap_or(value)
    } else {
        value
    };
    let path = value
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(value)
        .trim_start_matches('/');
    let (bucket, key) = path
        .split_once('/')
        .ok_or_else(|| S3Error::bad_request("x-amz-copy-source 格式非法"))?;
    Ok((percent_decode(bucket), percent_decode(key)))
}

// ─────────────────────────── XML 构建 ───────────────────────────

fn xml_response(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/xml")
        .body(Body::from(body))
        .unwrap_or_else(|_| empty_response(StatusCode::INTERNAL_SERVER_ERROR))
}

fn empty_response(status: StatusCode) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn error_response(err: S3Error) -> Response<Body> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{}</Code><Message>{}</Message><RequestId>{}</RequestId></Error>",
        xml_escape(&err.code),
        xml_escape(&err.message),
        uuid::Uuid::new_v4()
    );
    let mut builder = Response::builder()
        .status(err.status)
        .header("Content-Type", "application/xml");
    if let Some(total) = err.actual_size {
        builder = builder.header("Content-Range", format!("bytes */{total}"));
    }
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn list_buckets_xml(buckets: &[S3BucketInfo]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Owner><ID>laoflchdb</ID><DisplayName>laoflchdb</DisplayName></Owner><Buckets>",
    );
    for bucket in buckets {
        out.push_str(&format!(
            "<Bucket><Name>{}</Name><CreationDate>{}</CreationDate></Bucket>",
            xml_escape(&bucket.name),
            format_s3_time(bucket.creation_date)
        ));
    }
    out.push_str("</Buckets></ListAllMyBucketsResult>");
    out
}

fn list_objects_xml(bucket: &str, query: &S3ListQuery, result: &S3ListResult) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Name>{}</Name><Prefix>{}</Prefix><MaxKeys>{}</MaxKeys><IsTruncated>{}</IsTruncated>",
        xml_escape(bucket),
        xml_escape(&query.prefix),
        if query.max_keys == 0 { 1000 } else { query.max_keys },
        result.is_truncated
    );

    if query.list_type_v2 {
        // ListObjectsV2：必须给出 KeyCount，且分页令牌字段是 ContinuationToken / NextContinuationToken
        out.push_str(&format!(
            "<KeyCount>{}</KeyCount>",
            result.objects.len() + result.common_prefixes.len()
        ));
        if let Some(token) = &query.continuation_token {
            out.push_str(&format!(
                "<ContinuationToken>{}</ContinuationToken>",
                xml_escape(token)
            ));
        }
        if let Some(start_after) = &query.start_after {
            if !start_after.is_empty() {
                out.push_str(&format!(
                    "<StartAfter>{}</StartAfter>",
                    xml_escape(start_after)
                ));
            }
        }
        if result.is_truncated {
            if let Some(next) = &result.next_marker {
                out.push_str(&format!(
                    "<NextContinuationToken>{}</NextContinuationToken>",
                    xml_escape(&encode_list_token(next))
                ));
            }
        }
    } else {
        // ListObjectsV1：Marker + NextMarker
        out.push_str(&format!(
            "<Marker>{}</Marker>",
            xml_escape(query.marker.as_deref().unwrap_or(""))
        ));
        if let Some(next) = &result.next_marker {
            out.push_str(&format!("<NextMarker>{}</NextMarker>", xml_escape(next)));
        }
    }

    if let Some(delimiter) = &query.delimiter {
        out.push_str(&format!(
            "<Delimiter>{}</Delimiter>",
            xml_escape(delimiter)
        ));
    }
    for object in &result.objects {
        out.push_str(&format!(
            "<Contents><Key>{}</Key><LastModified>{}</LastModified><ETag>{}</ETag><Size>{}</Size><StorageClass>STANDARD</StorageClass></Contents>",
            xml_escape(&object.key),
            format_s3_time(object.last_modified),
            xml_escape(&object.etag),
            object.size
        ));
    }
    for prefix in &result.common_prefixes {
        out.push_str(&format!(
            "<CommonPrefixes><Prefix>{}</Prefix></CommonPrefixes>",
            xml_escape(prefix)
        ));
    }
    out.push_str("</ListBucketResult>");
    out
}

/// 分页令牌：把 key 十六进制编码成不含特殊字符的不透明字符串，
/// 避免 token 里出现 `&`、`%`、非 ASCII 等需要反复 URL 编解码的内容。
fn encode_list_token(key: &str) -> String {
    hex::encode(key.as_bytes())
}

/// 解码分页令牌；非法编码（例如老客户端直接传了明文 key）时按明文处理。
fn decode_list_token(token: &str) -> String {
    match hex::decode(token) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(key) => key,
            Err(_) => token.to_string(),
        },
        Err(_) => token.to_string(),
    }
}

fn copy_result_xml(etag: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <CopyObjectResult><LastModified>{}</LastModified><ETag>{}</ETag></CopyObjectResult>",
        format_s3_time(now_secs()),
        xml_escape(etag)
    )
}

fn delete_result_xml(keys: &[String]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
    );
    for key in keys {
        out.push_str(&format!("<Deleted><Key>{}</Key></Deleted>", xml_escape(key)));
    }
    out.push_str("</DeleteResult>");
    out
}

fn initiate_multipart_xml(bucket: &str, key: &str, upload_id: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <InitiateMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Bucket>{}</Bucket><Key>{}</Key><UploadId>{}</UploadId></InitiateMultipartUploadResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(upload_id)
    )
}

fn complete_multipart_xml(bucket: &str, key: &str, etag: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <CompleteMultipartUploadResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Location>/{}/{}</Location><Bucket>{}</Bucket><Key>{}</Key><ETag>{}</ETag>\
         </CompleteMultipartUploadResult>",
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(bucket),
        xml_escape(key),
        xml_escape(etag)
    )
}

fn list_multipart_uploads_empty_xml(bucket: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <ListMultipartUploadsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\
         <Bucket>{}</Bucket><KeyMarker></KeyMarker><UploadIdMarker></UploadIdMarker>\
         <IsTruncated>false</IsTruncated></ListMultipartUploadsResult>",
        xml_escape(bucket)
    )
}

// ─────────────────────────── XML 解析 ───────────────────────────

fn parse_delete_objects(xml: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        rest = &rest[start + 5..];
        if let Some(end) = rest.find("</Key>") {
            let key = xml_unescape(&rest[..end]);
            keys.push(key);
            rest = &rest[end + 6..];
        } else {
            break;
        }
    }
    keys
}

fn parse_complete_multipart(xml: &str) -> Vec<(i32, String)> {
    let mut parts = Vec::new();
    for block in xml.split("<Part>").skip(1) {
        let part_number = extract_tag(block, "PartNumber")
            .and_then(|v| v.trim().parse::<i32>().ok());
        let etag = extract_tag(block, "ETag")
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default();
        if let Some(part_number) = part_number {
            parts.push((part_number, etag));
        }
    }
    parts
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml_unescape(&xml[start..end]))
}

fn parse_list_query(query: &QueryParams) -> S3ListQuery {
    let list_type_v2 = query
        .get("list-type")
        .map(|v| v == "2")
        .unwrap_or(false);

    // V2 用 continuation-token（Base16 编码的 key），V1 用 marker；两者都按 key 处理
    let continuation_token = query.get("continuation-token").map(|v| decode_list_token(v));
    let start_after = query.get("start-after").cloned().filter(|v| !v.is_empty());

    let marker = if list_type_v2 {
        continuation_token
            .clone()
            .or_else(|| start_after.clone())
    } else {
        query
            .get("marker")
            .cloned()
            .filter(|m| !m.is_empty())
            .or_else(|| continuation_token.clone())
    };

    S3ListQuery {
        prefix: query.get("prefix").cloned().unwrap_or_default(),
        delimiter: query
            .get("delimiter")
            .cloned()
            .filter(|d| !d.is_empty()),
        max_keys: query
            .get("max-keys")
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(1000)
            .min(1000),
        marker,
        reverse: query
            .get("reverse")
            .map(|v| v == "true")
            .unwrap_or(false),
        list_type_v2,
        continuation_token,
        start_after,
    }
}

// ─────────────────────────── 时间格式化 ───────────────────────────

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_s3_time(seconds: u64) -> String {
    match OffsetDateTime::from_unix_timestamp(seconds as i64) {
        Ok(t) => format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
            t.year(),
            u8::from(t.month()),
            t.day(),
            t.hour(),
            t.minute(),
            t.second()
        ),
        Err(_) => String::new(),
    }
}

fn format_http_date(seconds: u64) -> String {
    match OffsetDateTime::from_unix_timestamp(seconds as i64) {
        Ok(t) => format!(
            "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
            weekday_name(t.weekday()),
            t.day(),
            month_name(t.month()),
            t.year(),
            t.hour(),
            t.minute(),
            t.second()
        ),
        Err(_) => String::new(),
    }
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

fn month_name(month: Month) -> &'static str {
    match month {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    }
}
