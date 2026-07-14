//! 图片 Tab 业务逻辑
//!
//! 提供上传图片、列出图片、查看元数据、删除图片四个操作的异步封装。

use anyhow::{anyhow, Result};
use std::path::Path;

use laoflchdb_image_service_proto::proto::{
    DeleteImageRequest, GetImageMetadataRequest, ListImagesRequest, UploadImageRequest,
};

use crate::app::App;

/// 上传图片
///
/// 从 `image_tab.file_path` 读取本地文件，根据扩展名推断 content_type，
/// 调用 ImageService.UploadImage 上传，结果写入 `upload_result`。
pub async fn upload_image(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let file_path = app.image_tab.file_path.value.clone();
    let bucket = app.image_tab.bucket.value.clone();
    let key = app.image_tab.key.value.clone();

    if file_path.is_empty() {
        app.set_error("请输入本地文件路径");
        return Ok(());
    }

    let path = Path::new(&file_path);
    let data = std::fs::read(path).map_err(|e| anyhow!("读取文件失败: {}", e))?;

    // 根据扩展名推断 content_type
    let content_type = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "application/octet-stream",
        })
        .unwrap_or("application/octet-stream")
        .to_string();

    let req = UploadImageRequest {
        bucket,
        key,
        data,
        content_type,
        metadata: Default::default(),
    };

    app.set_status("正在上传图片...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .upload_image(auth_req)
            .await
            .map_err(|e| anyhow!("上传失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("上传失败: {}", resp.message));
        return Ok(());
    }

    let info = if let Some(ref m) = resp.metadata {
        format!(
            "key={}, etag={}, size={}x{}, format={}",
            resp.key, resp.etag, m.width, m.height, m.format
        )
    } else {
        format!("key={}, etag={}", resp.key, resp.etag)
    };
    app.image_tab.upload_result = Some(info);
    app.image_tab.key.set_value("");
    app.set_status(format!("上传成功: {}", resp.key));
    Ok(())
}

/// 列出 bucket 中的图片
pub async fn list_images(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let bucket = app.image_tab.bucket.value.clone();

    let req = ListImagesRequest {
        bucket,
        prefix: String::new(),
        max_keys: 100,
        marker: String::new(),
    };

    app.set_status("正在列出图片...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .list_images(auth_req)
            .await
            .map_err(|e| anyhow!("列出失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("列出失败: {}", resp.message));
        return Ok(());
    }

    let count = resp.images.len();
    app.image_tab.images = resp.images;
    app.image_tab.list_scroll = 0;
    app.set_status(format!("列出 {} 张图片", count));
    Ok(())
}

/// 查看选中图片的元数据详情
///
/// 需要 `key` 输入框有值，否则使用列表中第一张。
pub async fn get_metadata(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let bucket = app.image_tab.bucket.value.clone();
    let key = if !app.image_tab.key.value.is_empty() {
        app.image_tab.key.value.clone()
    } else if let Some(img) = app.image_tab.images.first() {
        img.key.clone()
    } else {
        app.set_error("请输入图片 key 或先列出图片");
        return Ok(());
    };

    let req = GetImageMetadataRequest { bucket, key };
    app.set_status("正在获取元数据...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .get_image_metadata(auth_req)
            .await
            .map_err(|e| anyhow!("获取元数据失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("获取失败: {}", resp.message));
        return Ok(());
    }
    if let Some(m) = resp.metadata {
        let info = format!(
            "key={}, type={}, len={}, {}x{}, etag={}, fmt={}",
            m.key, m.content_type, m.content_length, m.width, m.height, m.etag, m.format
        );
        app.image_tab.meta_detail = Some(info);
        app.set_status("元数据已显示");
    }
    Ok(())
}

/// 删除图片
pub async fn delete_image(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let bucket = app.image_tab.bucket.value.clone();
    let key = if !app.image_tab.key.value.is_empty() {
        app.image_tab.key.value.clone()
    } else {
        app.set_error("请输入要删除的图片 key");
        return Ok(());
    };

    let req = DeleteImageRequest { bucket, key };
    app.set_status("正在删除图片...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .delete_image(auth_req)
            .await
            .map_err(|e| anyhow!("删除失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("删除失败: {}", resp.message));
        return Ok(());
    }
    app.set_status(format!("已删除 {} 个对象", resp.deleted_keys.len()));
    Ok(())
}
