//! 人脸 Tab 业务逻辑
//!
//! 提取人脸特征（Detect + Align + Embed）以及比较两个人脸特征向量。

use anyhow::{anyhow, Result};

use laoflchdb_face_service_proto::proto::{
    CompareFeaturesRequest, ExtractFaceFeaturesRequest,
};

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
        return_aligned_images: false,
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
    for (i, f) in resp.faces.iter().enumerate() {
        let score = f.detection.as_ref().map(|d| d.score).unwrap_or(0.0);
        let bbox = f.detection.as_ref().map(|d| d.bbox.clone()).unwrap_or_default();
        let saved_key = f.saved_image_key.clone();
        let vector_id = f.indexed_vector_id;
        if i == 0 {
            first_embedding = f.embedding.clone();
        }
        faces.push((i + 1, score, bbox, saved_key, vector_id));
    }

    let n = faces.len();
    app.face_tab.faces = faces;
    app.face_tab.selected_face = 0;
    app.face_tab.embedding_preview = first_embedding;
    app.face_tab.list_scroll = 0;
    app.set_status(format!("检测到 {} 张人脸", n));
    Ok(())
}

/// 比较两个人脸特征向量
///
/// 简化实现：在 det_threshold 输入框中输入两组用 `;` 分隔的浮点数向量。
pub async fn compare_features(app: &mut App) -> Result<()> {
    if !app.require_login() {
        return Ok(());
    }

    // 解析输入：要求 det_threshold 字段中输入两组浮点数，用 `;` 分隔
    let raw = app.face_tab.det_threshold.value.clone();
    let parts: Vec<&str> = raw.split(';').collect();
    if parts.len() != 2 {
        app.set_error("请在阈值框输入两组向量，用 ; 分隔（每组用逗号分隔的浮点数）");
        return Ok(());
    }

    let f1: Vec<f32> = parts[0]
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let f2: Vec<f32> = parts[1]
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if f1.is_empty() || f2.is_empty() {
        app.set_error("向量解析为空，请检查输入");
        return Ok(());
    }

    let req = CompareFeaturesRequest {
        feature1: f1,
        feature2: f2,
    };
    app.set_status("正在比较特征...");
    let resp = match {
        let clients = app.clients.as_mut().unwrap();
        let auth_req = clients.auth_request(req);
        clients
            .face
            .compare_features(auth_req)
            .await
    } {
        Ok(r) => r.into_inner(),
        Err(e) => {
            app.set_error(format!("比较失败: {}", e));
            return Ok(());
        }
    };
    if !resp.success {
        app.set_error(format!("比较失败: {}", resp.message));
        return Ok(());
    }
    app.set_status(format!(
        "相似度: {:.4}，是否同一人: {}",
        resp.similarity, resp.is_same_person
    ));
    Ok(())
}
