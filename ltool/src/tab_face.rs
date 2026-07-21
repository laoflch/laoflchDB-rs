//! 人脸 Tab 业务逻辑
//!
//! 提取人脸特征（Detect + Align + Embed）。

use anyhow::{anyhow, Result};
use log::warn;

use laoflchdb_face_service_proto::proto::{
    ExtractFaceFeaturesRequest,
};
use laoflchdb_image_service_proto::proto::ListImagesRequest;
use laoflchdb_embedding_service_proto::proto::DeleteEmbeddingRequest;

use crate::app::App;

/// 提取人脸特征
///
/// 从 `face_tab.file_path` 读取本地图片，调用 FaceService.ExtractFaceFeatures，
/// 把每个人脸的 (序号, score, bbox, saved_image_key, indexed_vector_id) 存入 `faces`。
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
    let save_aligned = app.face_tab.save_aligned_images;
    let index_embedding = app.face_tab.index_embedding;

    let req = ExtractFaceFeaturesRequest {
        image_data,
        det_threshold,
        max_faces,
        save_aligned_images: save_aligned,
        image_bucket,
        return_aligned_images: true, // 返回对齐后的人脸图片用于导出
        index_embedding,
    };

    app.set_status("正在提取人脸特征...");
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
            app.set_error(format!("提取失败: {}", e));
            return Ok(());
        }
    };
    if !resp.success {
        app.set_error(format!("提取失败: {}", resp.message));
        return Ok(());
    }

    let mut faces = Vec::new();
    let mut first_embedding = Vec::new();
    let mut aligned_images = Vec::new();
    let mut all_empty = true;
    for (i, f) in resp.faces.iter().enumerate() {
        let score = f.detection.as_ref().map(|d| d.score).unwrap_or(0.0);
        let bbox = f.detection.as_ref().map(|d| d.bbox.clone()).unwrap_or_default();
        let saved_key = f.saved_image_key.clone();
        let vector_id = f.indexed_vector_id;
        if i == 0 {
            first_embedding = f.embedding.clone();
        }
        faces.push((i + 1, score, bbox, saved_key, vector_id));
        aligned_images.push(f.aligned_image.clone());
        if !f.aligned_image.is_empty() {
            all_empty = false;
        }
    }

    let n = faces.len();
    app.face_tab.faces = faces;
    app.face_tab.selected_face = 0;
    app.face_tab.embedding_preview = first_embedding;
    app.face_tab.aligned_images = aligned_images;
    app.face_tab.list_scroll = 0;
    if all_empty && n > 0 {
        app.set_status(format!("检测到 {} 张人脸，但对齐图片数据为空", n));
    } else {
        app.set_status(format!("检测到 {} 张人脸", n));
    }
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
        let req = laoflchdb_image_service_proto::proto::DeleteImageRequest {
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

    let bucket = app.face_tab.bucket.value.clone();

    app.set_status(format!("正在导出人脸: {}", key));

    // 下载图片
    let req = laoflchdb_image_service_proto::proto::GetImageRequest {
        bucket,
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
