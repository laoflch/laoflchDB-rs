//! 存储 Tab：S3 兼容对象存储浏览器
//!
//! 支持两种模式：
//! - REST 模式：laoflchdb 的 REST API（Bearer Token 认证，JSON 响应）
//! - S3 模式：标准 S3 协议（AWS Signature V4 签名，XML 响应）

use std::time::Duration;

use anyhow::{anyhow, Result};
use chrono::Local;
use rusty_s3::actions::{ListObjectsV2, S3Action};
use rusty_s3::{Bucket, Credentials, UrlStyle};
use url::Url;

use crate::app::{App, StorageObject};

// ── 辅助函数 ──

/// 获取 S3 客户端（Bucket + Credentials）
fn get_s3_client(app: &App) -> Result<(Bucket, Credentials)> {
    let endpoint_str = app.storage_tab.endpoint.value.trim().trim_end_matches('/');
    let endpoint = Url::parse(endpoint_str)
        .map_err(|e| anyhow!("端点 URL 无效: {}", e))?;

    let bucket_name = app.storage_tab.bucket.value.trim().to_string();
    let region = app.storage_tab.region.value.trim().to_string();
    if region.is_empty() {
        return Err(anyhow!("Region 不能为空"));
    }

    // 判断 URL 风格：如果 endpoint 包含 "s3.amazonaws.com" 或类似 AWS 域名，用 VirtualHost
    // 否则用 Path 风格（兼容 MinIO、laoflchdb 等）
    let host = endpoint.host_str().unwrap_or("");
    let path_style = if host.contains("amazonaws.com") || host.contains("wasabisys.com") {
        UrlStyle::VirtualHost
    } else {
        UrlStyle::Path
    };

    let bucket = Bucket::new(endpoint, path_style, bucket_name, region)
        .map_err(|e| anyhow!("创建 S3 Bucket 失败: {}", e))?;

    let access_key = app.storage_tab.access_key.value.trim();
    let secret_key = app.storage_tab.secret_key.value.trim();
    let credentials = Credentials::new(String::from(access_key), String::from(secret_key));

    Ok((bucket, credentials))
}

/// 发送 S3 请求并返回响应文本
async fn s3_request<'a, A: S3Action<'a>>(action: &A) -> Result<String> {
    let url = action.sign(Duration::from_secs(3600));
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("S3 请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("S3 请求失败 ({}): {}", status, body));
    }

    Ok(resp.text().await.map_err(|e| anyhow!("读取响应失败: {}", e))?)
}

/// 发送 S3 PUT 请求（上传）
async fn s3_put_request<'a, A: S3Action<'a>>(action: &A, data: Vec<u8>, content_type: &str) -> Result<String> {
    let url = action.sign(Duration::from_secs(3600));
    let client = reqwest::Client::new();
    let resp = client
        .put(url)
        .header("Content-Type", content_type)
        .body(data)
        .send()
        .await
        .map_err(|e| anyhow!("S3 上传失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("S3 上传失败 ({}): {}", status, body));
    }

    Ok("OK".to_string())
}

/// 发送 S3 DELETE 请求
async fn s3_delete_request<'a, A: S3Action<'a>>(action: &A) -> Result<String> {
    let url = action.sign(Duration::from_secs(3600));
    let client = reqwest::Client::new();
    let resp = client
        .delete(url)
        .send()
        .await
        .map_err(|e| anyhow!("S3 删除失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("S3 删除失败 ({}): {}", status, body));
    }

    Ok("OK".to_string())
}

/// 解析 S3 ListObjectsV2 XML 响应
fn parse_s3_list_response(xml: &str) -> Result<(Vec<StorageObject>, bool, String)> {
    let parsed = ListObjectsV2::parse_response(xml)
        .map_err(|e| anyhow!("解析 XML 失败: {}", e))?;

    let objects: Vec<StorageObject> = parsed.contents.into_iter().map(|c| {
        let content_type = infer_content_type(&c.key);
        StorageObject {
            key: c.key,
            size: c.size as i64,
            last_modified: c.last_modified,
            content_type,
        }
    }).collect();

    let is_truncated = parsed.next_continuation_token.is_some();
    let next_token = parsed.next_continuation_token.unwrap_or_default();

    Ok((objects, is_truncated, next_token))
}

/// 从文件名推断 content type
fn infer_content_type(key: &str) -> String {
    let ext = std::path::Path::new(key)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "png" => "image/png".to_string(),
        "gif" => "image/gif".to_string(),
        "webp" => "image/webp".to_string(),
        "bmp" => "image/bmp".to_string(),
        "pdf" => "application/pdf".to_string(),
        "txt" | "md" => "text/plain".to_string(),
        "json" => "application/json".to_string(),
        "csv" => "text/csv".to_string(),
        "yaml" | "yml" => "application/x-yaml".to_string(),
        "mp4" => "video/mp4".to_string(),
        "mp3" => "audio/mpeg".to_string(),
        "zip" => "application/zip".to_string(),
        "tar" => "application/x-tar".to_string(),
        "gz" => "application/gzip".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

// ── 主要 API 函数 ──

/// 登录（仅 REST 模式需要）
pub async fn login(app: &mut App) -> Result<()> {
    if app.storage_tab.use_s3 {
        // S3 模式不需要登录
        return Ok(());
    }

    let endpoint = app.storage_tab.endpoint.value.trim().to_string();

    // 从 endpoint 推导基础 URL
    let base_url = endpoint
        .trim_end_matches('/')
        .trim_end_matches("/api/v1/object-store")
        .trim_end_matches('/')
        .to_string();

    let login_url = format!("{}/api/v1/auth/login", base_url);

    let client = reqwest::Client::new();
    let resp = client
        .post(&login_url)
        .json(&serde_json::json!({
            "username": app.username,
            "password": app.password,
        }))
        .send()
        .await
        .map_err(|e| anyhow!("登录请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("登录失败 ({}): {}", status, body));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("解析登录响应失败: {}", e))?;

    let token = body["token"]
        .as_str()
        .ok_or_else(|| anyhow!("登录响应中缺少 token"))?
        .to_string();

    app.storage_tab.token = token;
    app.storage_tab.logged_in = true;
    app.set_status(format!("登录成功（{}）", Local::now().format("%H:%M:%S")));
    Ok(())
}

/// 列出对象
pub async fn list_objects(app: &mut App) -> Result<()> {
    if app.storage_tab.use_s3 {
        s3_list_objects(app).await
    } else {
        rest_list_objects(app).await
    }
}

/// S3 模式：列出对象
async fn s3_list_objects(app: &mut App) -> Result<()> {
    let (bucket, credentials) = get_s3_client(app)?;
    let prefix = app.storage_tab.prefix.value.trim().to_string();

    let mut action = bucket.list_objects_v2(Some(&credentials));
    if !prefix.is_empty() {
        action.query_mut().insert("prefix", prefix.clone());
    }

    let xml = s3_request(&action).await?;
    let (mut objects, is_truncated, next_token) = parse_s3_list_response(&xml)?;

    // 排序：按 key 排序
    objects.sort_by(|a, b| a.key.cmp(&b.key));

    let count = objects.len();
    app.storage_tab.objects = objects;
    app.storage_tab.selected_index = if count > 0 { Some(0) } else { None };
    app.storage_tab.list_scroll = 0;
    app.storage_tab.pagination.on_response(&next_token, is_truncated, count);
    app.set_status(format!("bucket '{}' 下有 {} 个对象", app.storage_tab.bucket.value.trim(), count));
    Ok(())
}

/// REST 模式：列出对象
async fn rest_list_objects(app: &mut App) -> Result<()> {
    if !app.storage_tab.logged_in {
        login(app).await?;
    }

    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let bucket = app.storage_tab.bucket.value.trim().to_string();
    let prefix = app.storage_tab.prefix.value.trim().to_string();
    let marker = app.storage_tab.pagination.marker.clone();

    let list_url = if prefix.is_empty() {
        format!("{}/{}", endpoint.trim_end_matches('/'), bucket)
    } else {
        format!(
            "{}/{}/{}?delimiter=/",
            endpoint.trim_end_matches('/'),
            bucket,
            prefix
        )
    };

    let client = reqwest::Client::new();
    let resp = client
        .get(&list_url)
        .query(&[("max_keys", "50"), ("marker", &marker)])
        .header("Authorization", format!("Bearer {}", app.storage_tab.token))
        .send()
        .await
        .map_err(|e| anyhow!("列出对象失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status == 401 || status == 403 {
            app.storage_tab.logged_in = false;
            return login(app).await;
        }
        return Err(anyhow!("列出对象失败 ({}): {}", status, body));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("解析响应失败: {}", e))?;

    let objects: Vec<StorageObject> = body["objects"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| StorageObject {
                    key: v["key"].as_str().unwrap_or("").to_string(),
                    size: v["size"].as_i64().unwrap_or(0),
                    last_modified: v["last_modified"].as_str().unwrap_or("").to_string(),
                    content_type: v["content_type"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    let is_truncated = body["is_truncated"].as_bool().unwrap_or(false);
    let next_marker = body["next_marker"].as_str().unwrap_or("").to_string();

    let count = objects.len();
    app.storage_tab.objects = objects;
    app.storage_tab.selected_index = if count > 0 { Some(0) } else { None };
    app.storage_tab.list_scroll = 0;
    app.storage_tab.pagination.on_response(&next_marker, is_truncated, count);
    app.set_status(format!("bucket '{}' 下有 {} 个对象", bucket, count));
    Ok(())
}

/// 获取对象内容
pub async fn get_object(app: &mut App, key: &str) -> Result<Vec<u8>> {
    if app.storage_tab.use_s3 {
        s3_get_object(app, key).await
    } else {
        rest_get_object(app, key).await
    }
}

/// S3 模式：获取对象
async fn s3_get_object(app: &mut App, key: &str) -> Result<Vec<u8>> {
    let (bucket, credentials) = get_s3_client(app)?;
    let action = bucket.get_object(Some(&credentials), key);
    let url = action.sign(Duration::from_secs(3600));

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("获取对象失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!("获取对象失败 ({}): {}", status, resp.status().canonical_reason().unwrap_or("")));
    }

    let data = resp.bytes().await.map_err(|e| anyhow!("读取数据失败: {}", e))?;
    Ok(data.to_vec())
}

/// REST 模式：获取对象
async fn rest_get_object(app: &mut App, key: &str) -> Result<Vec<u8>> {
    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let bucket = app.storage_tab.bucket.value.trim().to_string();
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", app.storage_tab.token))
        .send()
        .await
        .map_err(|e| anyhow!("获取对象失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(anyhow!("获取对象失败 ({}): {}", status, resp.status().canonical_reason().unwrap_or("")));
    }

    let data = resp.bytes().await.map_err(|e| anyhow!("读取数据失败: {}", e))?;
    Ok(data.to_vec())
}

/// 上传对象
pub async fn put_object(app: &mut App, key: &str, data: Vec<u8>, content_type: &str) -> Result<()> {
    if app.storage_tab.use_s3 {
        s3_put_object(app, key, data, content_type).await
    } else {
        rest_put_object(app, key, data, content_type).await
    }
}

/// S3 模式：上传对象
async fn s3_put_object(app: &mut App, key: &str, data: Vec<u8>, content_type: &str) -> Result<()> {
    let (bucket, credentials) = get_s3_client(app)?;
    let action = bucket.put_object(Some(&credentials), key);
    s3_put_request(&action, data, content_type).await?;
    app.set_status(format!("对象 '{}' 上传成功", key));
    Ok(())
}

/// REST 模式：上传对象
async fn rest_put_object(app: &mut App, key: &str, data: Vec<u8>, content_type: &str) -> Result<()> {
    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let bucket = app.storage_tab.bucket.value.trim().to_string();
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);

    let client = reqwest::Client::new();
    let resp = client
        .put(&url)
        .header("Content-Type", content_type)
        .header("Authorization", format!("Bearer {}", app.storage_tab.token))
        .body(data)
        .send()
        .await
        .map_err(|e| anyhow!("上传对象失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("上传失败 ({}): {}", status, body));
    }

    app.set_status(format!("对象 '{}' 上传成功", key));
    Ok(())
}

/// 删除对象
pub async fn delete_object(app: &mut App, key: &str) -> Result<()> {
    if app.storage_tab.use_s3 {
        s3_delete_object(app, key).await
    } else {
        rest_delete_object(app, key).await
    }
}

/// S3 模式：删除对象
async fn s3_delete_object(app: &mut App, key: &str) -> Result<()> {
    let (bucket, credentials) = get_s3_client(app)?;
    let action = bucket.delete_object(Some(&credentials), key);
    s3_delete_request(&action).await?;
    app.set_status(format!("对象 '{}' 已删除", key));
    Ok(())
}

/// REST 模式：删除对象
async fn rest_delete_object(app: &mut App, key: &str) -> Result<()> {
    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let bucket = app.storage_tab.bucket.value.trim().to_string();
    let url = format!("{}/{}/{}", endpoint.trim_end_matches('/'), bucket, key);

    let client = reqwest::Client::new();
    let resp = client
        .delete(&url)
        .header("Authorization", format!("Bearer {}", app.storage_tab.token))
        .send()
        .await
        .map_err(|e| anyhow!("删除对象失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("删除失败 ({}): {}", status, body));
    }

    app.set_status(format!("对象 '{}' 已删除", key));
    Ok(())
}

/// 从本地文件上传到对象存储
pub async fn upload_file(app: &mut App, local_path: &str, object_key: &str) -> Result<()> {
    let data = std::fs::read(local_path)
        .map_err(|e| anyhow!("读取文件失败: {}", e))?;

    let content_type = infer_content_type(object_key);
    put_object(app, object_key, data, &content_type).await
}

/// 下载对象到本地文件
pub async fn download_object(app: &mut App, key: &str, local_path: &str) -> Result<()> {
    let data = get_object(app, key).await?;

    // 确保父目录存在
    if let Some(parent) = std::path::Path::new(local_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(local_path, &data)
        .map_err(|e| anyhow!("写入文件失败: {}", e))?;

    let size_str = if data.len() > 1024 * 1024 {
        format!("{:.1} MB", data.len() as f64 / (1024.0 * 1024.0))
    } else if data.len() > 1024 {
        format!("{:.1} KB", data.len() as f64 / 1024.0)
    } else {
        format!("{} B", data.len())
    };

    app.set_status(format!("已下载 '{}' 到 '{}'（{}）", key, local_path, size_str));
    Ok(())
}