//! 图片 Tab 业务逻辑
//!
//! 提供上传图片（自动向量索引）、列出图片、查看元数据、删除图片、向量搜索等操作。

use anyhow::{anyhow, Result};
use log::warn;
use std::path::Path;
use tokio_stream::wrappers::ReceiverStream;

use laoflchdb_image_service_proto::proto::{
    DeleteImageRequest, GetImageMetadataRequest, GetImageRequest, ListImagesRequest,
    UploadImageChunk, UploadImageRequest,
};

use crate::app::App;

/// 切片大小（4MB - 1KB，留出 protobuf 编码开销空间）
const CHUNK_SIZE: usize = 4 * 1024 * 1024 - 1024;

/// 上传图片文件并自动向量索引
///
/// 读取指定文件，上传到图片服务（设置 auto_index=true），服务端自动完成向量化。
/// 返回上传后的图片 key。
/// 此函数不依赖 image_tab 状态，可在 face_tab 等地方复用。
///
/// 大文件（>4MB）自动使用流式切片上传，小文件走普通上传路径。
pub async fn upload_and_index_file(
    app: &mut App,
    file_path: &str,
    bucket: &str,
    key: &str,
    content_type: &str,
    name: &str,
) -> Result<String> {
    let data = std::fs::read(file_path).map_err(|e| anyhow!("读取文件失败: {}", e))?;

    // ── 上传（设置 auto_index=true，服务端自动完成向量化） ──
    let resp_key = if data.len() > CHUNK_SIZE {
        upload_file_chunked(app, &data, bucket, key, content_type, name, true, "jina-clip-v2").await?
    } else {
        let req = UploadImageRequest {
            bucket: bucket.to_string(),
            key: key.to_string(),
            data,
            content_type: content_type.to_string(),
            metadata: Default::default(),
            name: name.to_string(),
            auto_index: true,
            auto_index_model: "jina-clip-v2".to_string(),
        };
        let resp = {
            let clients = app.clients.as_mut().unwrap();
            let auth_req = clients.auth_request(req);
            match clients.image.upload_image(auth_req).await {
                Ok(r) => r.into_inner(),
                Err(e) => return Err(anyhow!("上传请求失败: {}", e)),
            }
        };
        if !resp.success {
            return Err(anyhow!("上传失败: {}", resp.message));
        }
        resp.key
    };

    Ok(resp_key)
}

/// 流式切片上传大文件
///
/// 将数据分成 4MB 的切片，通过 UploadImageStream 流式上传。
/// 第一个切片携带元数据，后续切片仅包含数据。
async fn upload_file_chunked(
    app: &mut App,
    data: &[u8],
    bucket: &str,
    key: &str,
    content_type: &str,
    name: &str,
    auto_index: bool,
    auto_index_model: &str,
) -> Result<String> {
    let total_chunks = (data.len() + CHUNK_SIZE - 1) / CHUNK_SIZE;
    let (tx, rx) = tokio::sync::mpsc::channel::<UploadImageChunk>(8);

    // 在后台任务中发送切片
    let data_owned = data.to_vec();
    let bucket_owned = bucket.to_string();
    let key_owned = key.to_string();
    let content_type_owned = content_type.to_string();
    let name_owned = name.to_string();
    let auto_index_model_owned = auto_index_model.to_string();
    tokio::spawn(async move {
        for i in 0..total_chunks {
            let start = i * CHUNK_SIZE;
            let end = std::cmp::min(start + CHUNK_SIZE, data_owned.len());
            let chunk_data = data_owned[start..end].to_vec();

            let chunk = UploadImageChunk {
                bucket: if i == 0 { bucket_owned.clone() } else { String::new() },
                key: if i == 0 { key_owned.clone() } else { String::new() },
                content_type: if i == 0 { content_type_owned.clone() } else { String::new() },
                metadata: Default::default(),
                name: if i == 0 { name_owned.clone() } else { String::new() },
                data: chunk_data,
                chunk_index: i as i32,
                total_chunks: total_chunks as i32,
                auto_index: i == 0 && auto_index,
                auto_index_model: if i == 0 { auto_index_model_owned.clone() } else { String::new() },
            };
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
    });

    let stream = ReceiverStream::new(rx);
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request_stream(stream);
        match clients.image.upload_image_stream(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => return Err(anyhow!("流式上传请求失败: {}", e)),
        }
    };
    if !resp.success {
        return Err(anyhow!("流式上传失败: {}", resp.message));
    }

    Ok(resp.key)
}

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

    // key 为空时，服务端自动生成 Snowflake ID
    app.set_status("正在上传图片...");
    match upload_and_index_file(app, &file_path, &bucket, &key, &content_type, &file_path).await {
        Ok(uploaded_key) => {
            app.image_tab.upload_result = Some(format!("key={}", uploaded_key));
            app.image_tab.key.set_value("");
            app.set_status(format!("上传成功: {}, 向量索引成功", uploaded_key));
        }
        Err(e) => {
            let msg = format!("{}", e);
            if msg.starts_with("上传") {
                app.set_error(&msg);
            } else {
                // 可能是上传成功但索引失败，以状态栏显示
                app.set_status(format!("上传成功（向量索引跳过: {}）", msg));
            }
        }
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

    let req = DeleteImageRequest { bucket: bucket.clone(), key };
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

    // 同时删除对应的向量索引（如果 key 包含 vector_id，如 face_{id} 格式）
    if let Some(vector_id_str) = deleted_key.strip_prefix("face_") {
        if let Ok(vector_id) = vector_id_str.parse::<u64>() {
            use laoflchdb_embedding_service_proto::proto::DeleteEmbeddingRequest;
            let del_vec_req = DeleteEmbeddingRequest {
                id: vector_id,
                index_name: "face".to_string(),
            };
            let clients = app.clients.as_mut().unwrap();
            let auth_req = clients.auth_request(del_vec_req);
            if let Err(e) = clients.embedding.delete_embedding(auth_req).await {
                warn!("删除向量失败 (id={}): {}", vector_id, e);
            }
        }
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
/// 2. 调用 VectorService.CreateEmbeddingStream 生成向量（支持大图片的流式上传）
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

    // 1. 调用 VectorService.CreateEmbeddingStream 生成向量（使用流式上传支持大图片）
    app.set_status("正在生成查询向量...");
    let emb_resp = {
        let chunk_size = CHUNK_SIZE;
        let total_chunks = (data.len() + chunk_size - 1) / chunk_size;
        let (tx, rx) = tokio::sync::mpsc::channel::<laoflchdb_vector_service_proto::proto::EmbeddingChunk>(8);

        // 后台发送切片
        let data_owned = data.clone();
        let model_name_owned = model_name.to_string();
        tokio::spawn(async move {
            for i in 0..total_chunks {
                let start = i * chunk_size;
                let end = std::cmp::min(start + chunk_size, data_owned.len());
                let chunk = laoflchdb_vector_service_proto::proto::EmbeddingChunk {
                    model_name: if i == 0 { model_name_owned.clone() } else { String::new() },
                    dim: if i == 0 { dim } else { 0 },
                    data: data_owned[start..end].to_vec(),
                    chunk_index: i as i32,
                    total_chunks: total_chunks as i32,
                };
                if tx.send(chunk).await.is_err() {
                    break;
                }
            }
        });

        let stream = ReceiverStream::new(rx);
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request_stream(stream);
        match clients.vector.create_embedding_stream(auth_req).await {
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

/// 获取 image 索引的 dim，失败时返回默认值 "512"
pub async fn get_image_index_dim(app: &mut App) -> String {
    use laoflchdb_embedding_service_proto::proto::GetIndexInfoRequest;
    if let Some(clients) = app.clients.as_mut() {
        let req = GetIndexInfoRequest { index_name: "image".to_string() };
        let auth_req = clients.auth_request(req);
        match clients.embedding.get_index_info(auth_req).await {
            Ok(r) => {
                let resp = r.into_inner();
                if resp.success {
                    if let Some(stats) = resp.stats {
                        return stats.dim.to_string();
                    }
                }
            }
            Err(_) => {}
        }
    }
    "512".to_string()
}
