//! 图片 Tab 业务逻辑
//!
//! 提供上传图片（自动向量索引）、列出图片、查看元数据、删除图片、向量搜索等操作。

use anyhow::{anyhow, Result};
use std::path::Path;
use std::time::SystemTime;

use laoflchdb_image_service_proto::proto::{
    DeleteImageRequest, GetImageMetadataRequest, GetImageRequest, ListImagesRequest,
    UploadImageRequest,
};

use crate::app::App;

/// 上传图片并自动向量索引
///
/// 从 `image_tab.file_path` 读取本地文件，上传到图片服务后，
/// 自动调用 VectorService 生成向量并插入到 EmbeddingIndexService。
/// 向量索引失败不影响上传结果。
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
        bucket: bucket.clone(),
        key: key.clone(),
        data,
        content_type,
        metadata: Default::default(),
        name: file_path.clone(),
    };

    app.set_status("正在上传图片...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        match clients.image.upload_image(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_error(format!("上传请求失败: {}", e));
                return Ok(());
            }
        }
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

    // ── 自动向量索引 ──────────────────────────────────
    app.set_status(format!("上传成功: {}, 正在生成向量索引...", resp.key));

    // 重新读取文件（data 已被 UploadImageRequest 消费）
    let image_data = match std::fs::read(&file_path) {
        Ok(d) => d,
        Err(_) => {
            app.set_status(format!("上传成功: {}（向量索引跳过: 读取文件失败）", resp.key));
            return Ok(());
        }
    };

    use laoflchdb_vector_service_proto::proto::EmbeddingRequest;
    let emb_req = EmbeddingRequest {
        model_name: "jina-clip-v2".to_string(),
        texts: vec![],
        dim: 512,
        images: vec![image_data],
    };

    let emb_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(emb_req);
        match clients.vector.create_embedding(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_status(format!("上传成功: {}（向量索引跳过: {}", resp.key, e));
                return Ok(());
            }
        }
    };
    if !emb_resp.success {
        app.set_status(format!("上传成功: {}（向量化失败: {}）", resp.key, emb_resp.message));
        return Ok(());
    }

    let embedding = match emb_resp.results.first() {
        Some(r) => r.embedding.clone(),
        None => {
            app.set_status(format!("上传成功: {}（向量索引跳过: 向量化结果为空）", resp.key));
            return Ok(());
        }
    };

    // 使用 image key（Snowflake ID）作为向量索引 ID，可直接关联回图片
    let id = resp.key.parse::<u64>().unwrap_or_else(|_| {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });

    use laoflchdb_embedding_service_proto::proto::InsertEmbeddingRequest;
    let ins_req = InsertEmbeddingRequest {
        id,
        index_name: "image".to_string(),
        embedding,
    };

    let ins_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(ins_req);
        match clients.embedding.insert_embedding(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_status(format!("上传成功: {}（索引请求失败: {}）", resp.key, e));
                return Ok(());
            }
        }
    };
    if ins_resp.success {
        app.set_status(format!("上传成功: {}, 向量索引成功 (id={})", resp.key, id));
    } else {
        app.set_status(format!("上传成功: {}（索引失败: {}）", resp.key, ins_resp.message));
    }

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
    app.image_tab.selected_index = if count > 0 { Some(0) } else { None };
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
    let deleted_key = key.clone();

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

    // 删除后刷新列表
    let _ = list_images(app).await;

    app.set_status(format!("删除图片 {}", deleted_key));
    Ok(())
}

/// 下载图片
///
/// 从 `download_confirm` 获取 key，调用 GetImage 获取数据，保存到 `download_path` 指定路径。
pub async fn download_image(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let bucket = app.image_tab.bucket.value.clone();
    let key = match &app.image_tab.download_confirm {
        Some(k) => k.clone(),
        None => {
            app.set_error("未指定下载 key");
            return Ok(());
        }
    };
    let save_path = app.image_tab.download_path.value.clone();
    if save_path.is_empty() {
        app.set_error("请输入保存路径");
        return Ok(());
    }

    let req = GetImageRequest {
        bucket,
        key: key.clone(),
    };
    app.set_status("正在下载图片...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .get_image(auth_req)
            .await
            .map_err(|e| anyhow!("下载失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("下载失败: {}", resp.message));
        return Ok(());
    }

    // 写入本地文件
    std::fs::write(&save_path, &resp.data)
        .map_err(|e| anyhow!("写入文件失败: {}", e))?;

    app.set_status(format!("下载完成: {} → {}", key, save_path));
    Ok(())
}

/// 搜索相似图片
///
/// 1. 读取本地图片文件
/// 2. 调用 VectorService.CreateEmbedding 生成向量
/// 3. 调用 EmbeddingIndexService.SearchEmbedding 在指定索引中搜索相似向量
/// 4. 结果存入 app.image_tab.search_results，弹窗显示
pub async fn search_similar_image(app: &mut App, model_name: &str, index_name: &str, dim: i32, top_k: i32, max_distance: f32) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let file_path = app.image_tab.file_path.value.clone();
    if file_path.is_empty() {
        app.set_error("请先选择图片文件");
        return Ok(());
    }

    let data = std::fs::read(&file_path).map_err(|e| anyhow!("读取文件失败: {}", e))?;

    // 1. 调用 VectorService.CreateEmbedding 生成向量
    use laoflchdb_vector_service_proto::proto::EmbeddingRequest;
    let req = EmbeddingRequest {
        model_name: model_name.to_string(),
        texts: vec![],
        dim,
        images: vec![data],
    };

    app.set_status("正在生成查询向量...");
    let emb_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        match clients.vector.create_embedding(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_error(format!("向量化请求失败: {}", e));
                return Ok(());
            }
        }
    };
    if !emb_resp.success {
        app.set_error(format!("向量化失败: {}", emb_resp.message));
        return Ok(());
    }

    let query_embedding = match emb_resp.results.first() {
        Some(r) => r.embedding.clone(),
        None => {
            app.set_error("向量化结果为空");
            return Ok(());
        }
    };

    // 2. 调用 EmbeddingIndexService.SearchEmbedding 搜索相似向量
    use laoflchdb_embedding_service_proto::proto::SearchEmbeddingRequest;
    let search_req = SearchEmbeddingRequest {
        query_embedding,
        top_k,
        index_name: index_name.to_string(),
    };

    app.set_status("正在搜索相似图片...");
    let search_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(search_req);
        match clients.embedding.search_embedding(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_error(format!("搜索请求失败: {}", e));
                return Ok(());
            }
        }
    };
    if !search_resp.success {
        app.set_error(format!("搜索失败: {}", search_resp.message));
        return Ok(());
    }

    // 3. 按最大距离过滤并保存结果
    app.image_tab.search_results_scroll = 0;
    use crate::app::SearchResultItem;
    let total = search_resp.results.len();
    let filtered: Vec<SearchResultItem> = search_resp
        .results
        .into_iter()
        .filter(|r| r.distance <= max_distance)
        .map(|r| SearchResultItem {
            id: r.id,
            score: r.distance,
        })
        .collect();
    app.image_tab.search_results = filtered;
    app.image_tab.search_index_name = index_name.to_string();

    let count = app.image_tab.search_results.len();
    if count == 0 {
        app.image_tab.search_selected = None;
        if total > 0 {
            app.set_status(format!("搜索完成，原始 {} 个结果均被距离阈值 {:.2} 过滤掉", total, max_distance));
        } else {
            app.set_status("未找到相似图片");
        }
    } else {
        app.image_tab.search_selected = Some(0);
        app.set_status(format!("搜索完成，{} 个结果（过滤前 {} 个，距离阈值 ≤ {:.2}）", count, total, max_distance));
        app.image_tab.show_search_results = true;
    }

    Ok(())
}
