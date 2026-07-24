//! 人脸 Tab 业务逻辑
//!
//! 提取人脸特征（Detect + Align + Embed）。
//! F1 仅检测人脸不保存/索引，用户可在检测结果中选择人脸后通过菜单保存和索引。

use anyhow::{anyhow, Result};
use log::warn;

use laoflchdb_face_service_proto::proto::ExtractFaceFeaturesRequest;
use laoflchdb_image_service_proto::proto::{DeleteImageRequest, ListImagesRequest};
use laoflchdb_embedding_service_proto::proto::DeleteEmbeddingRequest;

use crate::app::App;

/// 提取人脸特征（仅检测，不保存/索引）
///
/// 从 `face_tab.file_path` 读取本地图片，调用 FaceService.ExtractFaceFeatures，
/// 始终设置 save_aligned_images=false, index_embedding=false，
/// 仅返回检测结果和对齐图片、embedding 数据。
pub async fn extract_features(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }
    let file_path = app.face_tab.file_path.value.clone();
    if file_path.is_empty() {
        app.set_error("请输入本地图片路径");
        return Ok(());
    }

    let image_data = std::fs::read(&file_path).map_err(|e| anyhow!("读取文件失败: {}", e))?;

    let det_threshold: f32 = app.face_tab.det_threshold.value.parse().unwrap_or(0.5);
    let max_faces: i32 = app.face_tab.max_faces.value.parse().unwrap_or(0);
    let image_bucket = app.face_tab.bucket.value.clone();

    // 始终仅检测，不保存/索引
    let req = ExtractFaceFeaturesRequest {
        image_data,
        det_threshold,
        max_faces,
        save_aligned_images: false,
        image_bucket,
        return_aligned_images: true,
        index_embedding: false,
        save_original_image: false,
    };

    app.set_status("正在检测人脸...");
    let resp = match {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .face
            .extract_face_features(auth_req)
            .await
    } {
        Ok(r) => r.into_inner(),
        Err(e) => {
            app.set_error(format!("检测失败: {}", e));
            return Ok(());
        }
    };
    if !resp.success {
        app.set_error(format!("检测失败: {}", resp.message));
        return Ok(());
    }

    let mut faces = Vec::new();
    let mut embeddings = Vec::new();
    let mut first_embedding = Vec::new();
    let mut aligned_images = Vec::new();
    let mut all_empty = true;
    for (i, f) in resp.faces.iter().enumerate() {
        let score = f.detection.as_ref().map(|d| d.score).unwrap_or(0.0);
        let bbox = f.detection.as_ref().map(|d| d.bbox.clone()).unwrap_or_default();
        // 初始状态：saved_key 为空，vector_id=0
        faces.push((i + 1, score, bbox, String::new(), 0u64));
        embeddings.push(f.embedding.clone());
        if i == 0 {
            first_embedding = f.embedding.clone();
        }
        aligned_images.push(f.aligned_image.clone());
        if !f.aligned_image.is_empty() {
            all_empty = false;
        }
    }

    let n = faces.len();
    app.face_tab.faces = faces;
    app.face_tab.embeddings = embeddings;
    app.face_tab.selected_face = 0;
    app.face_tab.embedding_preview = first_embedding;
    app.face_tab.aligned_images = aligned_images;
    app.face_tab.list_scroll = 0;
    app.face_tab.selected_face_num = if n > 0 { Some(0) } else { None };
    app.face_tab.detection_action_open = false;
    if all_empty && n > 0 {
        app.set_status(format!("检测到 {} 张人脸，但对齐图片数据为空", n));
    } else {
        app.set_status(format!("检测到 {} 张人脸，↑↓ 选择人脸，Enter 打开操作菜单", n));
    }
    Ok(())
}

/// 保存并索引选中的检测结果人脸
///
/// 利用 F1 已缓存的 aligned_image 和 embedding，直接上传对齐图片到 image_service
/// 并插入向量到 embedding_service，避免重复调用 ExtractFaceFeatures 重新处理整张图片。
/// 使用相同的 Snowflake ID 作为图片 key（face_ 前缀）和向量 ID，确保一致性。
///
/// 如果开启了 save_original，先上传原图并索引到 image 索引，原图 key 作为人脸元数据 name。
pub async fn save_and_index_face(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let face_idx = match app.face_tab.selected_face_num {
        Some(idx) => idx,
        None => {
            app.set_error("请先选中一张人脸");
            return Ok(());
        }
    };

    if face_idx >= app.face_tab.faces.len() {
        app.set_error("选中的人脸索引无效");
        return Ok(());
    }

    let aligned_image = app.face_tab.aligned_images[face_idx].clone();
    if aligned_image.is_empty() {
        app.set_error("该人脸没有对齐图片数据，无法保存");
        return Ok(());
    }

    let embedding = app.face_tab.embeddings.get(face_idx).cloned().unwrap_or_default();
    if embedding.is_empty() {
        app.set_error("该人脸没有 embedding 数据，无法索引");
        return Ok(());
    }

    let bucket = app.face_tab.bucket.value.clone();
    let face_num = face_idx + 1;

    // ── 1. 先搜索 face 索引，检查是否已存在相同向量 ──
    use laoflchdb_embedding_service_proto::proto::SearchEmbeddingRequest;
    let search_req = SearchEmbeddingRequest {
        query_embedding: embedding.clone(),
        top_k: 1,
        index_name: "face".to_string(),
    };

    app.set_status(format!("正在检查人脸 #{} 是否已存在...", face_num));
    let search_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(search_req);
        match clients.embedding.search_embedding(auth_req).await {
            Ok(r) => r.into_inner(),
            Err(e) => {
                app.set_error(format!("搜索人脸索引失败: {}", e));
                return Ok(());
            }
        }
    };

    if search_resp.success {
        if let Some(result) = search_resp.results.first() {
            if result.distance < 0.01 {
                let existing_vector_id = result.id;
                let existing_key = format!("face_{}", existing_vector_id);
                app.face_tab.faces[face_idx].3 = existing_key.clone();
                app.face_tab.faces[face_idx].4 = existing_vector_id;
                app.set_status(format!(
                    "人脸 #{} 已存在: key={}, vector_id={}, distance={:.4}，跳过保存和索引",
                    face_num, existing_key, existing_vector_id, result.distance
                ));
                return Ok(());
            }
        }
    }

    // ── 2. 如果开启了保存原图，先处理原图上传和索引 ──
    let mut original_image_key = String::new();
    if app.face_tab.save_original {
        let file_path = app.face_tab.file_path.value.clone();
        if file_path.is_empty() {
            app.set_error("保存原图时原始图片路径不能为空");
            return Ok(());
        }

        // 根据文件扩展名推断 content_type
        let path = std::path::Path::new(&file_path);
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

        // 原图 key 为空，服务端自动生成 Snowflake ID
        let orig_key = String::new();

        // 复用图片上传功能：上传 + 向量化 + 索引
        app.set_status("正在处理原图...");
        match crate::tab_image::upload_and_index_file(
            app,
            &file_path,
            "images",
            &orig_key,
            &content_type,
            "",
        )
        .await
        {
            Ok(key) => {
                original_image_key = key.clone();
                app.set_status(format!("原图已上传并索引: key={}", key));
            }
            Err(e) => {
                app.set_warning(format!("原图上传/索引失败（不影响人脸保存）: {}", e));
            }
        }
    }

    // ── 3. 上传对齐图片到 image_service（key 为空，服务端自动生成 Snowflake ID）──
    //     先上传获取到服务端生成的 key，再用其数字部分作为向量 ID
    app.set_status(format!("正在保存人脸 #{}...", face_num));

    // ── 3.1 上传对齐图片到 image_service（key 为空，服务端自动生成）──
    use std::collections::HashMap;
    use laoflchdb_image_service_proto::proto::UploadImageRequest;
    let mut metadata = HashMap::new();
    if !original_image_key.is_empty() {
        metadata.insert("name".to_string(), original_image_key.clone());
    }
    let upload_req = UploadImageRequest {
        bucket: bucket.clone(),
        key: String::new(),  // 空 key，服务端自动生成 Snowflake ID
        data: aligned_image,
        content_type: "image/jpeg".to_string(),
        metadata,
        name: original_image_key.clone(),
        auto_index: false,
        auto_index_model: String::new(),
    };

    let upload_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(upload_req);
        clients
            .image
            .upload_image(auth_req)
            .await
            .map_err(|e| anyhow!("上传人脸图片失败: {}", e))?
            .into_inner()
    };

    if !upload_resp.success {
        app.set_error(format!("上传人脸图片失败: {}", upload_resp.message));
        return Ok(());
    }

    let face_key = upload_resp.key.clone();
    let snowflake_id = face_key.parse::<u64>().unwrap_or_else(|_| {
        warn!("服务端返回的 key 不是有效数字: {}", face_key);
        0
    });

    // ── 4. 插入 embedding 到 embedding_service ──
    use laoflchdb_embedding_service_proto::proto::InsertEmbeddingRequest;
    let insert_req = InsertEmbeddingRequest {
        index_name: "face".to_string(),
        id: snowflake_id,
        embedding,
    };

    let insert_resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(insert_req);
        clients
            .embedding
            .insert_embedding(auth_req)
            .await
            .map_err(|e| anyhow!("插入向量失败: {}", e))?
            .into_inner()
    };

    if !insert_resp.success {
        app.set_error(format!("插入向量失败: {}", insert_resp.message));
        // 尝试清理已上传的图片
        use laoflchdb_image_service_proto::proto::DeleteImageRequest;
        let del_req = DeleteImageRequest {
            bucket: bucket.clone(),
            key: face_key.clone(),
        };
        if let Ok(del_resp) = {
            let clients = app.clients.as_mut().unwrap();
            let auth_req = clients.auth_request(del_req);
            clients.image.delete_image(auth_req).await
        } {
            if !del_resp.into_inner().success {
                warn!("清理已上传图片失败: {}", face_key);
            }
        }
        return Ok(());
    }

    // ── 6. 更新检测结果列表中的 saved_key 和 vector_id ──
    app.face_tab.faces[face_idx].3 = face_key.clone();
    app.face_tab.faces[face_idx].4 = snowflake_id;

    let mut status = format!(
        "人脸 #{} 已保存并索引: key={}, vector_id={}",
        face_num, face_key, snowflake_id
    );
    if !original_image_key.is_empty() {
        status.push_str(&format!("，原图 key={}", original_image_key));
    }
    app.set_status(&status);
    Ok(())
}

/// 导出所有检测到的人脸图片到指定目录
pub async fn export_faces(app: &mut App, output_dir: &str) -> Result<()> {
    if app.face_tab.aligned_images.is_empty() {
        app.set_error(format!(
            "没有检测到的人脸可导出，请先提取人脸特征。faces={}, aligned={}",
            app.face_tab.faces.len(),
            app.face_tab.aligned_images.len()
        ));
        return Ok(());
    }

    let output_path = std::path::Path::new(output_dir);
    if !output_path.exists() {
        std::fs::create_dir_all(output_path)
            .map_err(|e| anyhow!("创建输出目录失败: {}", e))?;
    }

    let mut success_count = 0;
    let mut all_empty = true;
    // 先克隆数据，避免借用问题
    let images_to_export = app.face_tab.aligned_images.clone();
    for (i, image_data) in images_to_export.iter().enumerate() {
        let face_num = i + 1;
        let filename = format!("face_{:03}.jpg", face_num);
        let filepath = output_path.join(filename);
        
        if image_data.is_empty() {
            continue;
        }
        all_empty = false;
        
        std::fs::write(&filepath, image_data)
            .map_err(|e| anyhow!("保存人脸 {} 失败: {}", face_num, e))?;
        
        success_count += 1;
    }

    if success_count > 0 {
        app.set_status(format!(
            "成功导出 {} 张人脸图片到 {}",
            success_count,
            output_path.to_str().unwrap_or("")
        ));
    } else if all_empty && !images_to_export.is_empty() {
        app.set_error("检测到人脸，但对齐图片数据为空，无法导出");
    } else {
        app.set_error("没有成功导出任何人脸图片");
    }
    Ok(())
}

/// 列出所有已保存的人脸（F3）
///
/// 调用 image_service.ListImages(bucket="faces", prefix="face_") 获取所有已保存的人脸图片。
pub async fn list_saved_faces(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let bucket = app.face_tab.bucket.value.clone();
    if bucket.is_empty() {
        app.set_error("请先设置 bucket 名称");
        return Ok(());
    }

    let req = ListImagesRequest {
        bucket,
        prefix: "face_".to_string(),
        max_keys: 1000,
        marker: String::new(),
    };

    app.set_status("正在获取已保存人脸列表...");
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .list_images(auth_req)
            .await
            .map_err(|e| anyhow!("列出人脸失败: {}", e))?
            .into_inner()
    };

    if !resp.success {
        app.set_error(format!("列出人脸失败: {}", resp.message));
        return Ok(());
    }

    let count = resp.images.len();
    app.face_tab.saved_faces = resp.images;
    app.face_tab.saved_scroll = 0;
    app.face_tab.saved_selected = if count > 0 { Some(0) } else { None };
    app.face_tab.show_saved = true;
    app.set_status(format!("已保存人脸: {} 张", count));
    Ok(())
}

/// 删除已保存的人脸（图片 + 向量）
///
/// 从 key 中提取 vector_id，分别调用 DeleteImage 和 DeleteEmbedding。
pub async fn delete_saved_face(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let key = match app.face_tab.saved_delete_key.take() {
        Some(k) => k,
        None => {
            app.set_error("未指定要删除的人脸 key");
            return Ok(());
        }
    };

    // 从 key 中提取 vector_id: "face_{id}" → id
    let vector_id: u64 = key
        .strip_prefix("face_")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let bucket = app.face_tab.bucket.value.clone();

    // 1. 删除图片
    app.set_status(format!("正在删除人脸: {}", key));
    {
        let clients = app.clients.as_mut().unwrap();
        let req = DeleteImageRequest {
            bucket: bucket.clone(),
            key: key.clone(),
        };
        let auth_req = clients.auth_request(req);
        let resp = clients
            .image
            .delete_image(auth_req)
            .await
            .map_err(|e| anyhow!("删除图片失败: {}", e))?
            .into_inner();
        if !resp.success {
            app.set_error(format!("删除图片失败: {}", resp.message));
            return Ok(());
        }
    }

    // 2. 删除向量
    if vector_id > 0 {
        let clients = app.clients.as_mut().unwrap();
        let req = DeleteEmbeddingRequest {
            id: vector_id,
            index_name: "face".to_string(),
        };
        let auth_req = clients.auth_request(req);
        if let Err(e) = clients.embedding.delete_embedding(auth_req).await {
            warn!("删除向量失败 (id={}): {}", vector_id, e);
        }
    }

    // 删除后刷新列表
    let _ = list_saved_faces(app).await;
    app.set_status(format!("已删除人脸: {}", key));
    Ok(())
}

/// 导出已保存人脸到导出路径
///
/// 从 image_service 下载人脸图片，保存到导出路径设置的目录。
pub async fn export_saved_face(app: &mut App, key: &str) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    let export_dir = app.face_tab.export_path.value.clone();
    if export_dir.is_empty() {
        app.set_error("请先设置导出路径");
        return Ok(());
    }

    app.set_status(format!("正在导出人脸: {}", key));

    // 下载图片
    let req = laoflchdb_image_service_proto::proto::GetImageRequest {
        bucket: app.face_tab.bucket.value.clone(),
        key: key.to_string(),
    };
    let resp = {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .image
            .get_image(auth_req)
            .await
            .map_err(|e| anyhow!("下载人脸失败: {}", e))?
            .into_inner()
    };
    if !resp.success {
        app.set_error(format!("下载人脸失败: {}", resp.message));
        return Ok(());
    }

    // 确保导出目录存在
    let output_path = std::path::Path::new(&export_dir);
    if !output_path.exists() {
        std::fs::create_dir_all(output_path)
            .map_err(|e| anyhow!("创建导出目录失败: {}", e))?;
    }

    // 保存文件
    let extension = if resp.content_type.contains("png") {
        "png"
    } else {
        "jpg"
    };
    let filename = format!("{}.{}", key, extension);
    let filepath = output_path.join(&filename);
    let file_size = resp.data.len();
    std::fs::write(&filepath, &resp.data)
        .map_err(|e| anyhow!("保存人脸图片失败: {}", e))?;

    app.set_status(format!("已导出人脸: {} ({} bytes) → {}", key, file_size, filepath.to_string_lossy()));
    Ok(())
}