//! 存储 Tab：S3 兼容对象存储浏览器
//!
//! 通过 REST API 访问 laoflchdb 的对象存储服务，也支持通用的 S3 兼容 API。

use anyhow::{anyhow, Result};
use crate::app::{App, StorageObject};
use chrono::Local;

/// 登录 REST API 获取 token
pub async fn login(app: &mut App) -> Result<()> {
    let endpoint = app.storage_tab.endpoint.value.trim().to_string();

    // 从 endpoint 推导基础 URL（去掉 /api/v1/object-store 部分）
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

/// 列出 bucket
pub async fn list_buckets(app: &mut App) -> Result<()> {
    if !app.storage_tab.logged_in {
        login(app).await?;
    }

    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let client = reqwest::Client::new();
    let resp = client
        .get(&endpoint)
        .header("Authorization", format!("Bearer {}", app.storage_tab.token))
        .send()
        .await
        .map_err(|e| anyhow!("列出 buckets 失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        // token 可能过期，尝试重新登录
        if status == 401 || status == 403 {
            app.storage_tab.logged_in = false;
            return login(app).await;
        }
        return Err(anyhow!("列出 buckets 失败 ({}): {}", status, body));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow!("解析响应失败: {}", e))?;

    let buckets = body["buckets"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v["name"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    app.set_status(format!("找到 {} 个 bucket", buckets.len()));
    Ok(())
}

/// 列出对象（在当前 bucket 和 prefix 下）
pub async fn list_objects(app: &mut App) -> Result<()> {
    if !app.storage_tab.logged_in {
        login(app).await?;
    }

    let endpoint = app.storage_tab.endpoint.value.trim().to_string();
    let bucket = app.storage_tab.bucket.value.trim().to_string();
    let prefix = app.storage_tab.prefix.value.trim().to_string();
    let marker = app.storage_tab.pagination.marker.clone();

    // 构建 URL
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
        .query(&[
            ("max_keys", "50"),
            ("marker", &marker),
        ])
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

    // 推断 content_type
    let ext = std::path::Path::new(local_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_lowercase();
    let content_type = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        "yaml" | "yml" => "application/x-yaml",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" => "application/gzip",
        _ => "application/octet-stream",
    };

    put_object(app, object_key, data, content_type).await
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