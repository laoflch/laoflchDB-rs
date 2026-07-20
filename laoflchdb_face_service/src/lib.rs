//! 人脸服务 (FaceService)
//!
//! 技术体系：SCRFD ONNX 人脸检测 + 5 关键点 → 裁剪对齐 112×112 → ArcFace 提取 512 维特征 + L2 归一化
//!
//! 基于 ort (ONNX Runtime) 加载 ONNX 模型推理，支持 CUDA GPU 加速。

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::post,
    Router,
};
use image::ImageBuffer;
use log::{info, warn};
use ndarray::{Array4, ArrayViewD};
use snowflake_me::Snowflake;
use tonic::{Request, Response as TonicResponse, Status};

use ort::{
    execution_providers::{CUDAExecutionProvider, CPUExecutionProvider, ExecutionProviderDispatch},
    session::Session,
    value::Value,
};

// Proto 定义来自独立的 laoflchdb_face_service_proto crate（避免客户端拉入 ort）
pub use laoflchdb_face_service_proto::proto;

use proto::face_service_server::{FaceService, FaceServiceServer};
use proto::*;

// ── 常量 ─────────────────────────────────────────────────────────

/// ArcFace 输入尺寸（112x112）
const FACE_SIZE: u32 = 112;
/// ArcFace 特征维度
const EMBEDDING_DIM: usize = 512;
/// SCRFD 默认检测阈值
const DEFAULT_DET_THRESHOLD: f32 = 0.5;
/// 默认最大检测人脸数（0=不限）
const DEFAULT_MAX_FACES: i32 = 0;
/// 人脸相似度判定阈值（余弦相似度 >= 0.5 视为同一人）
const SAME_PERSON_THRESHOLD: f32 = 0.5;
/// SCRFD 输入最大尺寸（长边）
const DETECT_MAX_INPUT_SIZE: u32 = 1280;
/// SCRFD 输入最小尺寸（短边，避免小图片检测效果差）
const DETECT_MIN_INPUT_SIZE: u32 = 320;

// ── 配置 ─────────────────────────────────────────────────────────

/// 人脸服务配置
#[derive(Debug, Clone)]
pub struct FaceServiceConfig {
    /// 是否启用
    pub enabled: bool,
    /// 模型根目录（SCRFD 和 ArcFace 的 ONNX 模型应位于此目录下）
    pub model_dir: String,
    /// SCRFD 模型文件名（相对 model_dir）
    pub scrfd_model_file: String,
    /// ArcFace 模型文件名（相对 model_dir）
    pub arcface_model_file: String,
    /// 默认检测阈值
    pub det_threshold: f32,
    /// 默认最大检测人脸数（0=不限）
    pub max_faces: i32,
}

impl Default for FaceServiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_dir: "./laoflch_db_model".to_string(),
            scrfd_model_file: "scrfd_2.5g.onnx".to_string(),
            arcface_model_file: "arcface_r100.onnx".to_string(),
            det_threshold: DEFAULT_DET_THRESHOLD,
            max_faces: DEFAULT_MAX_FACES,
        }
    }
}

// ── 服务实现 ─────────────────────────────────────────────────────

/// 人脸服务实现
///
/// 基于 SCRFD 检测人脸 + 5 关键点，裁剪对齐到 112x112，ArcFace 提取 512 维 L2 归一化特征。
/// 支持 CUDA GPU 加速（通过 ort 的 CUDAExecutionProvider）。
pub struct FaceServiceImpl {
    /// SCRFD 检测模型
    scrfd: Mutex<Option<ScrfdModel>>,
    /// ArcFace 特征提取模型
    arcface: Mutex<Option<ArcfaceModel>>,
    /// 是否使用 GPU
    #[allow(dead_code)]
    use_gpu: bool,
    /// 配置
    config: FaceServiceConfig,
    /// 图片服务（可选，用于保存对齐后的人脸图片）
    image_service: Option<Arc<laoflchdb_image_service::ImageServiceImpl>>,
    /// 向量索引服务（可选，用于把人脸特征向量写入 HNSW 索引）
    embedding_service: Option<Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>>,
    /// Snowflake ID 生成器（用于保存人脸图片时生成 key）
    snowflake: Mutex<Snowflake>,
}

impl FaceServiceImpl {
    /// 创建人脸服务
    ///
    /// - `config`: 服务配置（model_dir 下应包含 SCRFD 和 ArcFace 的 ONNX 模型文件）
    /// - `image_service`: 可选的图片服务实例，用于保存对齐后的人脸图片
    /// - `embedding_service`: 可选的向量索引服务实例，用于把人脸特征向量写入 HNSW 索引
    pub fn new(
        config: FaceServiceConfig,
        image_service: Option<Arc<laoflchdb_image_service::ImageServiceImpl>>,
        embedding_service: Option<Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>>,
    ) -> Self {
        // 检测 GPU 可用性，尝试优先使用 GPU 0（因为人脸服务独立运行，不与向量服务冲突）
        let use_gpu = Self::detect_gpu();

        // 加载 SCRFD 模型
        let scrfd = Self::load_scrfd(&config, use_gpu);
        // 加载 ArcFace 模型
        let arcface = Self::load_arcface(&config, use_gpu);

        let snowflake = Snowflake::new().unwrap_or_else(|_| {
            warn!("Snowflake 默认初始化失败，回退到 machine_id=0, data_center_id=0");
            Snowflake::builder()
                .machine_id(&|| Ok(0u16))
                .data_center_id(&|| Ok(0u16))
                .finalize()
                .expect("Snowflake with machine_id=0, data_center_id=0 must succeed")
        });

        info!(
            "FaceService 初始化完成: gpu={}, scrfd_loaded={}, arcface_loaded={}",
            use_gpu,
            scrfd.is_some(),
            arcface.is_some()
        );

        Self {
            scrfd: Mutex::new(scrfd),
            arcface: Mutex::new(arcface),
            use_gpu,
            config,
            image_service,
            embedding_service,
            snowflake: Mutex::new(snowflake),
        }
    }

    /// 检测 GPU 可用性
    ///
    /// `CUDAExecutionProvider::default().build()` 在 ort-rs 2.0.0-rc.10 中直接返回
    /// `ExecutionProviderDispatch`（非 Result），因此此处仅通过 feature 开关决定。
    /// 实际的 CUDA 可用性验证在 `create_ort_session` 的 `with_execution_providers` 中处理。
    fn detect_gpu() -> bool {
        #[cfg(feature = "cuda")]
        {
            info!("人脸服务启用 CUDA GPU 推理（尝试）");
            return true;
        }
        #[cfg(not(feature = "cuda"))]
        {
            info!("CUDA feature 未启用，人脸服务使用 CPU 推理");
            false
        }
    }

    /// 创建 ort Session 的通用函数
    fn create_ort_session(
        model_path: &Path,
        use_gpu: bool,
    ) -> Result<Session, Box<dyn std::error::Error + Send + Sync>> {
        let mut builder = Session::builder()?;

        // 配置执行提供者
        if use_gpu {
            // GPU 优先，CPU 作为 fallback
            let mut providers: Vec<ExecutionProviderDispatch> = Vec::new();
            providers.push(CUDAExecutionProvider::default().build());
            providers.push(CPUExecutionProvider::default().build());
            builder = builder.with_execution_providers(providers)?;
        }

        let session = builder.commit_from_file(model_path)?;

        Ok(session)
    }

    /// 加载 SCRFD ONNX 模型
    fn load_scrfd(config: &FaceServiceConfig, use_gpu: bool) -> Option<ScrfdModel> {
        let model_path = Path::new(&config.model_dir).join(&config.scrfd_model_file);
        if !model_path.exists() {
            warn!(
                "SCRFD 模型文件不存在: {}，人脸检测功能不可用",
                model_path.display()
            );
            return None;
        }

        info!("加载 SCRFD 模型: {} (GPU={})", model_path.display(), use_gpu);
        match Self::create_ort_session(&model_path, use_gpu) {
            Ok(session) => {
                info!("SCRFD 模型加载成功");
                Some(ScrfdModel { session })
            }
            Err(e) => {
                warn!("SCRFD 模型加载失败: {}", e);
                None
            }
        }
    }

    /// 加载 ArcFace ONNX 模型
    fn load_arcface(config: &FaceServiceConfig, use_gpu: bool) -> Option<ArcfaceModel> {
        let model_path = Path::new(&config.model_dir).join(&config.arcface_model_file);
        if !model_path.exists() {
            warn!(
                "ArcFace 模型文件不存在: {}，人脸特征提取功能不可用",
                model_path.display()
            );
            return None;
        }

        info!("加载 ArcFace 模型: {} (GPU={})", model_path.display(), use_gpu);
        match Self::create_ort_session(&model_path, use_gpu) {
            Ok(session) => {
                info!("ArcFace 模型加载成功");
                Some(ArcfaceModel { session })
            }
            Err(e) => {
                warn!("ArcFace 模型加载失败: {}", e);
                None
            }
        }
    }

    /// 是否已加载检测模型
    pub fn is_scrfd_loaded(&self) -> bool {
        self.scrfd.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// 是否已加载特征提取模型
    pub fn is_arcface_loaded(&self) -> bool {
        self.arcface.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    /// 生成基于 Snowflake 的唯一 key
    fn generate_face_key(&self) -> u64 {
        match self.snowflake.lock() {
            Ok(guard) => guard.next_id().unwrap_or_else(|_| {
                warn!("Snowflake next_id 失败，回退到毫秒时间戳");
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }),
            Err(_) => {
                warn!("Snowflake mutex 锁定失败，回退到毫秒时间戳");
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64
            }
        }
    }

    /// 解码图片二进制为 DynamicImage
    fn decode_image(image_data: &[u8]) -> Result<image::DynamicImage, Status> {
        image::load_from_memory(image_data).map_err(|e| {
            Status::invalid_argument(format!("图片解码失败: {}", e))
        })
    }

    /// 生成人脸图片保存的 key（基于 Snowflake ID，同时返回 ID 用于向量索引）
    fn make_face_image_key(&self) -> (String, u64) {
        let id = self.generate_face_key();
        (format!("face_{}", id), id)
    }
}

// ── 模型封装 ─────────────────────────────────────────────────────

/// SCRFD 检测模型封装
struct ScrfdModel {
    session: Session,
}

/// ArcFace 特征提取模型封装
struct ArcfaceModel {
    session: Session,
}

// ── SCRFD 检测实现 ───────────────────────────────────────────────

impl ScrfdModel {
    /// 获取模型的输入名称
    fn get_input_name(&self) -> Result<String, Status> {
        self.session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| Status::internal("SCRFD 模型无输入"))
    }

    /// 获取模型输出名称列表
    fn get_output_names(&self) -> Vec<String> {
        self.session
            .outputs
            .iter()
            .map(|o| o.name.clone())
            .collect()
    }

    /// 运行 SCRFD 检测
    ///
    /// 输入图片，返回检测到的人脸列表（bbox + score + 5 landmarks）
    fn detect(
        &mut self,
        img: &image::DynamicImage,
        threshold: f32,
        max_faces: i32,
    ) -> Result<Vec<DetectedFaceInfo>, Status> {
        let (orig_w, orig_h) = (img.width(), img.height());

        // 动态计算输入尺寸：根据原图长边，在 [DETECT_MIN_INPUT_SIZE, DETECT_MAX_INPUT_SIZE] 范围内
        let long_side = orig_w.max(orig_h);
        let input_size = long_side
            .max(DETECT_MIN_INPUT_SIZE)
            .min(DETECT_MAX_INPUT_SIZE);

        // 预处理：letterbox resize 到目标尺寸
        let (input_tensor, scale, pad_w, pad_h) =
            Self::preprocess(img, input_size)?;
        info!(
            "检测预处理: orig={}x{}, input_size={}, scale={:.6}, pad_w={}, pad_h={}",
            orig_w, orig_h, input_size, scale, pad_w, pad_h
        );

        let input_name = self.get_input_name()?;
        let output_names = self.get_output_names();

        // 运行推理（使用 HashMap 构造输入，ort::inputs! 宏在 rc.10 中不可用）
        let mut input_map = HashMap::new();
        input_map.insert(input_name, input_tensor);
        let outputs = self
            .session
            .run(input_map)
            .map_err(|e| Status::internal(format!("SCRFD 推理失败: {}", e)))?;

        // 调试日志：收集所有输出
        info!("SCRFD 推理输出数: {}", outputs.len());
        let mut all_outputs: Vec<(String, ndarray::ArrayD<f32>)> = Vec::new();
        for (i, name) in output_names.iter().enumerate() {
            if let Some(v) = outputs.get(name) {
                if let Ok(arr) = v.try_extract_array::<f32>() {
                    let shape: Vec<usize> = arr.shape().to_vec();
                    info!("  输出[{}] name={}, shape={:?}", i, name, shape);
                    all_outputs.push((name.clone(), arr.to_owned().into_dyn()));
                }
            }
        }

        // 解析输出：按每个 tensor 的第一维长度匹配 stride
        let mut all_faces = Vec::new();
        let strides = [8u32, 16u32, 32u32];

        for stride in strides.iter() {
            // 根据当前 input_size 动态计算这个 stride 的 tensor 长度
            let num_cells_per_side = input_size / stride;
            let expected_len = (num_cells_per_side * num_cells_per_side * 2) as usize;

            // 找这个 stride 的所有 tensor（len 匹配）
            let group: Vec<_> = all_outputs
                .iter()
                .filter(|(_, arr)| arr.shape()[0] == expected_len)
                .collect();

            // 在这个组里找 score, bbox, kps（按 last_dim）
            let mut score_arr: Option<&ndarray::ArrayD<f32>> = None;
            let mut bbox_arr: Option<&ndarray::ArrayD<f32>> = None;
            let mut kps_arr: Option<&ndarray::ArrayD<f32>> = None;

            for (_, arr) in group.iter() {
                let last_dim = *arr.shape().last().unwrap_or(&0);
                match last_dim {
                    1 => score_arr = Some(arr),
                    4 => bbox_arr = Some(arr),
                    10 => kps_arr = Some(arr),
                    _ => {}
                }
            }

            // 如果找到了三个输出，解析
            if let (Some(sa), Some(ba), Some(ka)) = (score_arr, bbox_arr, kps_arr) {
                let faces = Self::parse_stride_output(
                    &sa.view(),
                    &ba.view(),
                    &ka.view(),
                    *stride,
                    input_size,
                    orig_w,
                    orig_h,
                    scale,
                    pad_w,
                    pad_h,
                    threshold,
                )?;
                all_faces.extend(faces);
            }
        }

        // 按置信度降序排序
        all_faces.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // NMS: 非极大值抑制，去除重叠检测
        let nms_threshold = 0.4;
        let mut keep = Vec::new();
        let mut suppressed = vec![false; all_faces.len()];

        for i in 0..all_faces.len() {
            if suppressed[i] {
                continue;
            }
            keep.push(i);

            let bbox_i = &all_faces[i].bbox;
            for j in (i + 1)..all_faces.len() {
                if suppressed[j] {
                    continue;
                }
                let bbox_j = &all_faces[j].bbox;
                let iou = compute_iou(bbox_i, bbox_j);
                if iou > nms_threshold {
                    suppressed[j] = true;
                }
            }
        }

        let mut nms_faces = Vec::new();
        for &idx in &keep {
            nms_faces.push(all_faces[idx].clone());
        }

        info!("检测到 {} 张人脸 (NMS 后):", nms_faces.len());
        for (i, face) in nms_faces.iter().enumerate() {
            info!("  人脸 {}: score={:.4}, bbox={:?}, landmarks={:?}", 
                i, face.score, face.bbox, face.landmarks);
        }

        // 过滤假阳性检测
        // 策略：1) 边缘微小检测（padding 导致的误检） 2) 整体尺寸过小的检测
        let edge_margin_x = (orig_w as f32 * 0.02).max(10.0);
        let edge_margin_y = (orig_h as f32 * 0.02).max(10.0);
        // 最小人脸尺寸：原图短边的 1%，但至少 30px
        // 用短边（w.min(h)）判断，避免窄长人脸被误过滤
        let min_face_size = (orig_w.min(orig_h) as f32 * 0.01).max(30.0);
        nms_faces.retain(|f| {
            let (x1, y1, x2, y2) = (f.bbox[0], f.bbox[1], f.bbox[2], f.bbox[3]);
            let w = x2 - x1;
            let h = y2 - y1;
            // 过滤整体尺寸过小的检测（用短边判断）
            if w.min(h) < min_face_size {
                info!("  过滤过小假阳性: bbox=({:.1},{:.1})-({:.1},{:.1}), score={:.4}, size={:.0}x{:.0}, min_face={:.0}",
                    x1, y1, x2, y2, f.score, w, h, min_face_size);
                return false;
            }
            // 排除完全在边缘上的微小检测（宽或高 < 20px 且贴着边缘）
            let near_left = x1 <= edge_margin_x;
            let near_right = x2 >= orig_w as f32 - edge_margin_x;
            let near_top = y1 <= edge_margin_y;
            let near_bottom = y2 >= orig_h as f32 - edge_margin_y;
            if (w < 20.0 || h < 20.0) && (near_left || near_right || near_top || near_bottom) {
                info!("  过滤边缘假阳性: bbox=({:.1},{:.1})-({:.1},{:.1}), score={:.4}", x1, y1, x2, y2, f.score);
                return false;
            }
            true
        });

        // 限制最大数量
        if max_faces > 0 && nms_faces.len() > max_faces as usize {
            nms_faces.truncate(max_faces as usize);
        }

        Ok(nms_faces)
    }

    /// 预处理：参考 InsightFace 官方实现
    /// 关键点：1) 不居中 padding，放在左上角  2) 使用 (pixel - 127.5) / 128 归一化
    ///
    /// 返回 (input_tensor, scale, pad_w, pad_h)
    /// 这里 pad_w 和 pad_h 实际上都是 0（因为不 padding），但保留参数兼容接口
    fn preprocess(
        img: &image::DynamicImage,
        target_size: u32,
    ) -> Result<(Value, f32, u32, u32), Status> {
        let (orig_w, orig_h) = (img.width(), img.height());

        // 官方逻辑：按 height 计算 scale，不居中
        let im_ratio = orig_h as f32 / orig_w as f32;
        let model_ratio = 1.0; // target_size x target_size 是正方形
        let (new_w, new_h) = if im_ratio > model_ratio {
            let new_h = target_size;
            let new_w = (new_h as f32 / im_ratio) as u32;
            (new_w, new_h)
        } else {
            let new_w = target_size;
            let new_h = (new_w as f32 * im_ratio) as u32;
            (new_w, new_h)
        };

        // scale = new_height / orig_height（官方只按 height 算）
        let scale = new_h as f32 / orig_h as f32;

        // resize（官方用 cv2.resize，这里用 image::resize，效果接近）
        let resized = img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle);
        let rgb = resized.to_rgb8();

        // 创建 ndarray: [1, 3, H, W]  NCHW
        // 官方：det_img = np.zeros((input_size[1], input_size[0], 3))
        //       det_img[:new_height, :new_width, :] = resized_img
        // 即：图片放在左上角，不 padding！
        let mut array = Array4::<f32>::zeros((1, 3, target_size as usize, target_size as usize));

        for y in 0..new_h {
            for x in 0..new_w {
                let p = rgb.get_pixel(x, y);
                let r = p.0[0] as f32;
                let g = p.0[1] as f32;
                let b = p.0[2] as f32;
                // 官方：blob = cv2.dnn.blobFromImage(img, 1/std, input_size, (mean,mean,mean), swapRB=True)
                // swapRB=True 表示 BGR → RGB
                // 所以最终 channel 顺序是 R, G, B
                array[[0, 0, y as usize, x as usize]] = (r - 127.5) / 128.0;
                array[[0, 1, y as usize, x as usize]] = (g - 127.5) / 128.0;
                array[[0, 2, y as usize, x as usize]] = (b - 127.5) / 128.0;
            }
        }

        // 构造 ort Value
        let input_tensor = Value::from_array(array)
            .map_err(|e| Status::internal(format!("构造输入 tensor 失败: {}", e)))?
            .into_dyn();

        // pad_w = pad_h = 0，因为图片放在左上角，没有 padding
        Ok((input_tensor, scale, 0, 0))
    }

    /// 解析单个 stride 的输出
    #[allow(clippy::too_many_arguments)]
    fn parse_stride_output(
        score_view: &ArrayViewD<f32>,
        bbox_view: &ArrayViewD<f32>,
        kps_view: &ArrayViewD<f32>,
        stride: u32,
        input_size: u32,
        orig_w: u32,
        orig_h: u32,
        scale: f32,
        pad_w: u32,
        pad_h: u32,
        threshold: f32,
    ) -> Result<Vec<DetectedFaceInfo>, Status> {
        // ── 第一步：正确提取数据，确保三者长度一致 ──
        // 展平所有张量，方便统一处理
        let scores: Vec<f32> = score_view.iter().copied().collect();
        let bboxes_flat: Vec<f32> = bbox_view.iter().copied().collect();
        let kps_flat: Vec<f32> = kps_view.iter().copied().collect();

        let num_anchors = scores.len(); // score 的长度就是 anchor 数量
        let num_bbox_anchors = bboxes_flat.len() / 4; // 每个 anchor 4个坐标
        let num_kps_anchors = kps_flat.len() /10; // 每个 anchor 10个坐标 (5个点)
        // 确保三者的 anchor 数量一致，取最小的避免越界
        let actual_num_anchors = num_anchors.min(num_bbox_anchors).min(num_kps_anchors);

        // 调试日志
        let max_score = scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let min_score = scores.iter().fold(f32::INFINITY, |a, &b| a.min(b));
        let num_above_thresh = scores.iter().filter(|&&s| s >= threshold).count();
        info!("  stride={}: num_anchors={}, score_range=[{:.4}, {:.4}], threshold={:.4}, num_above={}",
              stride, actual_num_anchors, min_score, max_score, threshold, num_above_thresh);

        // 计算每个 anchor 的中心点
        let num_cells_per_row = input_size / stride;
        let num_anchors_per_pos = 2u32; // SCRFD 默认每个位置 2 个 anchor
        let mut faces = Vec::new();

        for i in 0..actual_num_anchors {
            let score = scores[i];
            if score < threshold {
                continue;
            }

            // bbox: [4] - distance to left/top/right/bottom (从展平数组读取)
            let bbox_base = i *4;
            if bbox_base +4 > bboxes_flat.len() { continue; } // 安全检查避免越界

            let dx1 = bboxes_flat[bbox_base];
            let dy1 = bboxes_flat[bbox_base+1];
            let dx2 = bboxes_flat[bbox_base+2];
            let dy2 = bboxes_flat[bbox_base+3];

            // 每个 cell 有 num_anchors_per_pos 个 anchor，所以 cell_idx = i / num_anchors_per_pos
            let cell_idx = i as u32 / num_anchors_per_pos;
            // 官方实现：anchor_centers = cell_idx * stride（不加 stride/2）
            let cx = (cell_idx % num_cells_per_row) as f32 * stride as f32;
            let cy = (cell_idx / num_cells_per_row) as f32 * stride as f32;

            // 解码 bbox（距离 → 绝对坐标）
            let x1 = cx - dx1 * stride as f32;
            let y1 = cy - dy1 * stride as f32;
            let x2 = cx + dx2 * stride as f32;
            let y2 = cy + dy2 * stride as f32;

            // 反 letterbox：减去 pad，除以 scale
            let orig_x1 = (x1 - pad_w as f32) / scale;
            let orig_y1 = (y1 - pad_h as f32) / scale;
            let orig_x2 = (x2 - pad_w as f32) / scale;
            let orig_y2 = (y2 - pad_h as f32) / scale;
            info!(
                "解码 bbox: scaled=[{:.2},{:.2},{:.2},{:.2}], orig=[{:.2},{:.2},{:.2},{:.2}]",
                x1, y1, x2, y2, orig_x1, orig_y1, orig_x2, orig_y2
            );
            let x1 = orig_x1.clamp(0.0, orig_w as f32);
            let y1 = orig_y1.clamp(0.0, orig_h as f32);
            let x2 = orig_x2.clamp(0.0, orig_w as f32);
            let y2 = orig_y2.clamp(0.0, orig_h as f32);

            // 解码 landmarks（10 个值：5 个点 x,y）（从展平数组读取）
            let mut landmarks = [0.0f32;10];
            let kps_base = i*10;
            if kps_base +10 <= kps_flat.len() {
                for j in 0..5 {
                    let lx = cx + kps_flat[kps_base + j*2] * stride as f32;
                    let ly = cy + kps_flat[kps_base + j*2 +1] * stride as f32;
                    landmarks[j*2] = ((lx - pad_w as f32)/scale).clamp(0.0, orig_w as f32);
                    landmarks[j*2 +1] = ((ly - pad_h as f32)/scale).clamp(0.0, orig_h as f32);
                }
            }

            faces.push(DetectedFaceInfo {
                bbox: [x1, y1, x2, y2],
                score,
                landmarks,
            });
        }

        Ok(faces)
    }
}

/// 计算两个 bbox 的 IOU
fn compute_iou(bbox1: &[f32; 4], bbox2: &[f32; 4]) -> f32 {
    let x1 = bbox1[0].max(bbox2[0]);
    let y1 = bbox1[1].max(bbox2[1]);
    let x2 = bbox1[2].min(bbox2[2]);
    let y2 = bbox1[3].min(bbox2[3]);

    let w = (x2 - x1).max(0.0);
    let h = (y2 - y1).max(0.0);
    let inter = w * h;

    let area1 = (bbox1[2] - bbox1[0]) * (bbox1[3] - bbox1[1]);
    let area2 = (bbox2[2] - bbox2[0]) * (bbox2[3] - bbox2[1]);
    let union = area1 + area2 - inter;

    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

// ── ArcFace 特征提取实现 ─────────────────────────────────────────

impl ArcfaceModel {
    /// 获取模型的输入名称
    fn get_input_name(&self) -> Result<String, Status> {
        self.session
            .inputs
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| Status::internal("ArcFace 模型无输入"))
    }

    /// 从对齐的 112x112 人脸图片提取 512 维特征并 L2 归一化
    fn extract(&mut self, aligned: &image::DynamicImage) -> Result<Vec<f32>, Status> {
        let input_tensor = Self::preprocess(aligned)?;

        let input_name = self.get_input_name()?;

        // 运行推理（使用 HashMap 构造输入）
        let mut input_map = HashMap::new();
        input_map.insert(input_name, input_tensor);
        let outputs = self
            .session
            .run(input_map)
            .map_err(|e| Status::internal(format!("ArcFace 推理失败: {}", e)))?;

        // 取第一个输出
        let (_name, output_value) = outputs
            .iter()
            .next()
            .ok_or_else(|| Status::internal("ArcFace 无输出"))?;

        let output_arr = output_value
            .try_extract_tensor::<f32>()
            .map_err(|e| Status::internal(format!("ArcFace 输出提取失败: {}", e)))?;

        // ArcFace 输出维度可能是 [1, 512] (2D) 或 [512] (1D)，统一 flatten 到 1D
        let (_shape, data) = output_arr;
        let embedding: Vec<f32> = data.to_vec();

        // L2 归一化
        let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        let normalized: Vec<f32> = if norm > 0.0 {
            embedding.iter().map(|v| v / norm).collect()
        } else {
            embedding
        };

        Ok(normalized)
    }

    /// 预处理：112x112 RGB → [1,3,112,112] NCHW
    /// ArcFace 标准预处理：像素值归一化到 [-1, 1]（即 (x - 127.5) / 127.5）
    fn preprocess(
        img: &image::DynamicImage,
    ) -> Result<Value, Status> {
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        if w != FACE_SIZE || h != FACE_SIZE {
            return Err(Status::invalid_argument(format!(
                "对齐图片尺寸应为 {}x{}，实际 {}x{}",
                FACE_SIZE, FACE_SIZE, w, h
            )));
        }

        let mut array = Array4::<f32>::zeros((1, 3, FACE_SIZE as usize, FACE_SIZE as usize));

        for y in 0..FACE_SIZE {
            for x in 0..FACE_SIZE {
                let p = rgb.get_pixel(x, y);
                // 归一化到 [-1, 1]
                array[[0, 0, y as usize, x as usize]] = (p.0[0] as f32 - 127.5) / 127.5;
                array[[0, 1, y as usize, x as usize]] = (p.0[1] as f32 - 127.5) / 127.5;
                array[[0, 2, y as usize, x as usize]] = (p.0[2] as f32 - 127.5) / 127.5;
            }
        }

        Value::from_array(array)
            .map_err(|e| Status::internal(format!("构造 ArcFace 输入 tensor 失败: {}", e)))
            .map(|v| v.into_dyn())
    }
}

// ── 裁剪对齐 ─────────────────────────────────────────────────────

/// 检测到的人脸信息（内部结构）
#[derive(Debug, Clone)]
struct DetectedFaceInfo {
    bbox: [f32; 4],
    score: f32,
    landmarks: [f32; 10],
}

/// 基于检测到的人脸 bbox 和关键点裁剪人脸并调整大小到 112x112
///
/// 使用关键点（5 个：左眼、右眼、鼻子、左嘴角、右嘴角）来精确定位人脸中心，
/// 比单纯依赖 bbox 中心更准确，尤其对大幅面图片中的小脸更鲁棒。
fn align_face(
    img: &image::DynamicImage,
    bbox: &[f32; 4], // [x1, y1, x2, y2]
    landmarks: &[f32; 10], // 5 个关键点 [x,y] 对
) -> Result<image::DynamicImage, Status> {
    // 确保 bbox 坐标正序（x1 <= x2, y1 <= y2）
    let mut x1 = bbox[0];
    let mut y1 = bbox[1];
    let mut x2 = bbox[2];
    let mut y2 = bbox[3];
    if x1 > x2 { std::mem::swap(&mut x1, &mut x2); }
    if y1 > y2 { std::mem::swap(&mut y1, &mut y2); }

    // 只用 bbox 中心，不使用 landmarks
    let face_cx = (x1 + x2) / 2.0;
    let face_cy = (y1 + y2) / 2.0;

    info!("使用 bbox 中心定位人脸: ({:.1},{:.1})", face_cx, face_cy);

    // 传递归一化后的 bbox
    let normalized_bbox = [x1, y1, x2, y2];
    crop_face_centered(img, &normalized_bbox, face_cx, face_cy)
}

/// 以指定人脸中心点裁剪并扩展为正方形，然后 resize 到 112x112
fn crop_face_centered(
    img: &image::DynamicImage,
    bbox: &[f32; 4],
    face_cx: f32,
    face_cy: f32,
) -> Result<image::DynamicImage, Status> {
    let rgba = img.to_rgba8();
    let (img_w, img_h) = rgba.dimensions();

    let (x1, y1, x2, y2) = (bbox[0], bbox[1], bbox[2], bbox[3]);
    let width = x2 - x1;
    let height = y2 - y1;

    // 稍微扩展一点，包含额头和下巴
    let size = width.max(height) * 1.2; // 1.2倍
    let half_size = size / 2.0;

    // 不向上移，直接用 face_cy
    let adjusted_cy = face_cy;

    // 正确计算裁剪区域，避免 crop_w/crop_h 为 0
    let crop_x1 = (face_cx - half_size).max(0.0);
    let crop_y1 = (adjusted_cy - half_size).max(0.0);
    let crop_x2 = (face_cx + half_size).min(img_w as f32);
    let crop_y2 = (adjusted_cy + half_size).min(img_h as f32);
    
    let crop_x = crop_x1 as u32;
    let crop_y = crop_y1 as u32;
    let crop_w = (crop_x2 - crop_x1) as u32;
    let crop_h = (crop_y2 - crop_y1) as u32;

    // 边界检查：如果 crop_w 或 crop_h 太小 (<16)，直接用 bbox 作为后备
    let (crop_x, crop_y, crop_w, crop_h) = if crop_w < 16 || crop_h <16 {
        info!("裁剪区域过小，回退到 bbox");
        let fallback_crop_x = x1.max(0.0) as u32;
        let fallback_crop_y = y1.max(0.0) as u32;
        let fallback_crop_w = (x2 -x1).max(1.0) as u32;
        let fallback_crop_h = (y2 -y1).max(1.0) as u32;
        (fallback_crop_x, fallback_crop_y, fallback_crop_w, fallback_crop_h)
    } else {
        (crop_x, crop_y, crop_w, crop_h)
    };

    info!("人脸 bbox: ({:.1},{:.1})-({:.1},{:.1})", x1, y1, x2, y2);
    info!("裁剪人脸: x={}, y={}, w={}, h={} (原图: {}x{})", crop_x, crop_y, crop_w, crop_h, img_w, img_h);

    // 手动裁剪图片，创建新的 ImageBuffer
    let mut cropped_buf = ImageBuffer::new(crop_w, crop_h);
    for y in 0..crop_h {
        for x in 0..crop_w {
            let src_x = crop_x + x;
            let src_y = crop_y + y;
            if src_x < img_w && src_y < img_h {
                let pixel = *rgba.get_pixel(src_x, src_y);
                cropped_buf.put_pixel(x, y, pixel);
            }
        }
    }

    // 调整大小到 112x112
    let resized = image::imageops::resize(
        &cropped_buf,
        FACE_SIZE,
        FACE_SIZE,
        image::imageops::FilterType::Triangle,
    );

    Ok(image::DynamicImage::ImageRgba8(resized))
}



// ── gRPC 服务实现 ────────────────────────────────────────────────

#[tonic::async_trait]
impl FaceService for FaceServiceImpl {
    async fn detect_faces(
        &self,
        request: Request<DetectFacesRequest>,
    ) -> Result<TonicResponse<DetectFacesResponse>, Status> {
        let req = request.into_inner();
        let threshold = if req.det_threshold >= 0.3 {
            req.det_threshold
        } else {
            self.config.det_threshold
        };
        let max_faces = if req.max_faces > 0 {
            req.max_faces
        } else {
            self.config.max_faces
        };

        // 解码图片
        let img = Self::decode_image(&req.image_data)?;

        // 检查 SCRFD 模型
        let mut scrfd_guard = self.scrfd.lock().map_err(|e| {
            Status::internal(format!("SCRFD 模型锁获取失败: {}", e))
        })?;
        let scrfd = scrfd_guard.as_mut().ok_or_else(|| {
            Status::failed_precondition("SCRFD 模型未加载，人脸检测不可用")
        })?;

        // 检测
        let faces = scrfd.detect(&img, threshold, max_faces)?;

        let detected_faces: Vec<DetectedFace> = faces
            .iter()
            .map(|f| DetectedFace {
                bbox: f.bbox.to_vec(),
                score: f.score,
                landmarks: f.landmarks.to_vec(),
            })
            .collect();

        let count = detected_faces.len();
        Ok(TonicResponse::new(DetectFacesResponse {
            success: true,
            message: format!("检测到 {} 张人脸", count),
            faces: detected_faces,
            image_width: img.width() as i32,
            image_height: img.height() as i32,
        }))
    }

    async fn extract_face_features(
        &self,
        request: Request<ExtractFaceFeaturesRequest>,
    ) -> Result<TonicResponse<ExtractFaceFeaturesResponse>, Status> {
        let req = request.into_inner();
        // 强制最小阈值 0.3，避免客户端传入过低的阈值导致大量假阳性
        let threshold = if req.det_threshold >= 0.3 {
            req.det_threshold
        } else {
            self.config.det_threshold
        };
        // 限制最大人脸数：客户端未指定时用配置值，配置为 0 时最多处理 20 张（避免假阳性导致超时）
        let max_faces = if req.max_faces > 0 {
            req.max_faces
        } else if self.config.max_faces > 0 {
            self.config.max_faces
        } else {
            20
        };

        // 解码图片
        let img = Self::decode_image(&req.image_data)?;

        // 在同步块中完成检测 + 对齐 + 特征提取（避免 MutexGuard 跨 await）
        let detected_and_embeddings: Vec<(DetectedFaceInfo, image::DynamicImage, Vec<f32>)>;
        {
            let mut scrfd_guard = self.scrfd.lock().map_err(|e| {
                Status::internal(format!("SCRFD 模型锁获取失败: {}", e))
            })?;
            let scrfd = scrfd_guard.as_mut().ok_or_else(|| {
                Status::failed_precondition("SCRFD 模型未加载，人脸特征提取不可用")
            })?;

            let mut arcface_guard = self.arcface.lock().map_err(|e| {
                Status::internal(format!("ArcFace 模型锁获取失败: {}", e))
            })?;
            let arcface = arcface_guard.as_mut().ok_or_else(|| {
                Status::failed_precondition("ArcFace 模型未加载，人脸特征提取不可用")
            })?;

            // 1. 检测人脸
            let detected = scrfd.detect(&img, threshold, max_faces)?;
            info!("检测到 {} 张人脸，原图尺寸: {}x{}", detected.len(), img.width(), img.height());
            for (i, face) in detected.iter().enumerate() {
                info!("  人脸 {}: bbox={:?}, score={}", i, face.bbox, face.score);
            }

            let mut results = Vec::with_capacity(detected.len());
            for face_info in &detected {
                // 2. 使用 bbox 裁剪并调整到 112x112
                let aligned = align_face(&img, &face_info.bbox, &face_info.landmarks)?;
                // 3. 提取 512 维特征并 L2 归一化
                let embedding = arcface.extract(&aligned)?;
                results.push((face_info.clone(), aligned, embedding));
            }
            detected_and_embeddings = results;
        }
        // 锁已释放，可以安全 await

        let mut face_features = Vec::with_capacity(detected_and_embeddings.len());
        for (face_info, aligned, embedding) in &detected_and_embeddings {
            // 4. 去重检测：搜索 face 索引中是否有相同向量
            let existing_id = if req.index_embedding || req.save_aligned_images {
                if let Some(ref emb_svc) = self.embedding_service {
                    search_face_embedding(emb_svc, embedding).await
                } else {
                    None
                }
            } else {
                None
            };

            // 5. 已存在时跳过保存和索引，直接使用已有的 key 和 vector_id
            let (saved_key, saved_bucket, face_id) = if let Some(existing_vid) = existing_id {
                (format!("face_{}", existing_vid), String::new(), existing_vid)
            } else if req.save_aligned_images {
                if let Some(ref img_svc) = self.image_service {
                    let (key, id) = self.make_face_image_key();
                    let bucket = if req.image_bucket.is_empty() {
                        img_svc.default_bucket()
                    } else {
                        req.image_bucket.clone()
                    };
                    match save_aligned_to_image_service(img_svc, aligned, &key, &bucket).await {
                        Ok(_) => (key, bucket, id),
                        Err(e) => {
                            warn!("保存对齐人脸图片失败: {}", e);
                            (String::new(), String::new(), id)
                        }
                    }
                } else {
                    warn!("请求保存对齐人脸图片但 image_service 未启用");
                    (String::new(), String::new(), 0)
                }
            } else if req.index_embedding {
                // 不保存图片，但需要索引向量：单独生成 ID
                (String::new(), String::new(), self.generate_face_key())
            } else {
                (String::new(), String::new(), 0)
            };

            // 6. 把人脸特征向量写入 embedding_service 的 HNSW 索引（已存在则跳过）
            let indexed_vector_id = if existing_id.is_some() {
                existing_id.unwrap()
            } else if req.index_embedding && face_id > 0 {
                if let Some(ref emb_svc) = self.embedding_service {
                    match index_face_embedding(emb_svc, face_id, embedding).await {
                        Ok(_) => face_id,
                        Err(e) => {
                            warn!("向量索引失败 (id={}): {}", face_id, e);
                            0
                        }
                    }
                } else {
                    warn!("请求索引向量但 embedding_service 未启用");
                    0
                }
            } else {
                0
            };

            // 6. 可选：返回对齐图片数据
            let aligned_bytes = if req.return_aligned_images {
                let mut buf = std::io::Cursor::new(Vec::new());
                match aligned.write_to(&mut buf, image::ImageOutputFormat::Jpeg(95)) {
                    Ok(_) => buf.into_inner(),
                    Err(e) => {
                        warn!("编码对齐人脸图片失败: {}", e);
                        Vec::new()
                    }
                }
            } else {
                Vec::new()
            };

            face_features.push(FaceFeature {
                detection: Some(DetectedFace {
                    bbox: face_info.bbox.to_vec(),
                    score: face_info.score,
                    landmarks: face_info.landmarks.to_vec(),
                }),
                embedding: embedding.clone(),
                aligned_image: aligned_bytes,
                saved_image_key: saved_key,
                saved_image_bucket: saved_bucket,
                indexed_vector_id,
            });
        }

        let count = face_features.len();
        Ok(TonicResponse::new(ExtractFaceFeaturesResponse {
            success: true,
            message: format!("提取了 {} 张人脸的特征", count),
            faces: face_features,
            image_width: img.width() as i32,
            image_height: img.height() as i32,
        }))
    }

    async fn extract_feature_from_aligned(
        &self,
        request: Request<ExtractFeatureFromAlignedRequest>,
    ) -> Result<TonicResponse<ExtractFeatureFromAlignedResponse>, Status> {
        let req = request.into_inner();

        // 解码对齐图片
        let img = Self::decode_image(&req.aligned_image_data)?;

        // 检查 ArcFace 模型
        let mut arcface_guard = self.arcface.lock().map_err(|e| {
            Status::internal(format!("ArcFace 模型锁获取失败: {}", e))
        })?;
        let arcface = arcface_guard.as_mut().ok_or_else(|| {
            Status::failed_precondition("ArcFace 模型未加载，特征提取不可用")
        })?;

        // 提取特征
        let embedding = arcface.extract(&img)?;

        let dim = embedding.len();
        Ok(TonicResponse::new(ExtractFeatureFromAlignedResponse {
            success: true,
            message: format!("提取 {} 维特征成功", dim),
            embedding,
        }))
    }

    async fn compare_features(
        &self,
        request: Request<CompareFeaturesRequest>,
    ) -> Result<TonicResponse<CompareFeaturesResponse>, Status> {
        let req = request.into_inner();

        if req.feature1.len() != EMBEDDING_DIM || req.feature2.len() != EMBEDDING_DIM {
            return Err(Status::invalid_argument(format!(
                "特征向量维度应为 {}，实际 feature1={}, feature2={}",
                EMBEDDING_DIM,
                req.feature1.len(),
                req.feature2.len()
            )));
        }

        // 计算余弦相似度（特征已 L2 归一化，点积即余弦相似度）
        let dot: f32 = req
            .feature1
            .iter()
            .zip(req.feature2.iter())
            .map(|(a, b)| a * b)
            .sum();

        let is_same = dot >= SAME_PERSON_THRESHOLD;

        Ok(TonicResponse::new(CompareFeaturesResponse {
            success: true,
            message: format!("相似度: {:.4}", dot),
            similarity: dot,
            is_same_person: is_same,
        }))
    }
}

/// 保存对齐后的人脸图片到 image_service
async fn save_aligned_to_image_service(
    img_svc: &Arc<laoflchdb_image_service::ImageServiceImpl>,
    aligned: &image::DynamicImage,
    key: &str,
    bucket: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use laoflchdb_image_service::proto::image_service_server::ImageService;
    use laoflchdb_image_service::proto::UploadImageRequest;

    // 编码为 JPEG
    let mut buf = std::io::Cursor::new(Vec::new());
    aligned.write_to(&mut buf, image::ImageOutputFormat::Jpeg(95))?;
    let data = buf.into_inner();

    let request = tonic::Request::new(UploadImageRequest {
        bucket: bucket.to_string(),
        key: key.to_string(),
        data,
        content_type: "image/jpeg".to_string(),
        metadata: HashMap::new(),
        name: String::new(),
    });

    img_svc
        .upload_image(request)
        .await
        .map(|_| ())
        .map_err(|e| format!("image_service 上传失败: {}", e).into())
}

/// 把人脸特征向量写入 embedding_service 的 HNSW 索引
///
/// - `emb_svc`: 向量索引服务实例
/// - `id`: 向量唯一 ID（复用图片 Snowflake ID）
/// - `embedding`: 512 维 L2 归一化特征向量
/// - `index_name`: 固定为 "face"
async fn index_face_embedding(
    emb_svc: &Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>,
    id: u64,
    embedding: &[f32],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use laoflchdb_embedding_service::proto::embedding_index_service_server::EmbeddingIndexService;
    use laoflchdb_embedding_service::proto::InsertEmbeddingRequest;

    let request = tonic::Request::new(InsertEmbeddingRequest {
        id,
        index_name: "face".to_string(),
        embedding: embedding.to_vec(),
    });

    let resp = emb_svc.insert_embedding(request).await?;
    let resp = resp.into_inner();
    if !resp.success {
        return Err(format!("embedding_service 索引失败: {}", resp.message).into());
    }
    Ok(())
}

/// 搜索 face 索引中是否有相同的人脸特征向量
///
/// 对 embedding 执行 ANN 搜索（top_k=1），如果距离 ≈ 0 则视为同一张人脸。
/// 使用 Cosine 距离，对于 L2 归一化的向量，相同向量的距离为 0。
async fn search_face_embedding(
    emb_svc: &Arc<laoflchdb_embedding_service::EmbeddingIndexServiceImpl>,
    embedding: &[f32],
) -> Option<u64> {
    use laoflchdb_embedding_service::proto::embedding_index_service_server::EmbeddingIndexService;
    use laoflchdb_embedding_service::proto::SearchEmbeddingRequest;

    let request = tonic::Request::new(SearchEmbeddingRequest {
        query_embedding: embedding.to_vec(),
        top_k: 1,
        index_name: "face".to_string(),
    });

    match emb_svc.search_embedding(request).await {
        Ok(resp) => {
            let resp = resp.into_inner();
            if resp.success && !resp.results.is_empty() {
                let result = &resp.results[0];
                // Cosine 距离 < 0.01 视为完全相同的人脸
                // 对于 L2 归一化向量，cosine_distance = 1 - dot_product
                // 完全相同 → distance = 0
                if result.distance < 0.01 {
                    info!("  检测到重复人脸: existing_id={}, distance={}", result.id, result.distance);
                    return Some(result.id);
                }
            }
            None
        }
        Err(e) => {
            warn!("搜索 face 索引失败: {}", e);
            None
        }
    }
}

// ── REST API ─────────────────────────────────────────────────────

/// 创建 REST API Router
///
/// 路由使用根相对路径，会被 nest 到 "/api/v1/face" 下：
/// - POST /detect              检测人脸
/// - POST /extract             提取人脸特征
/// - POST /extract-aligned     从对齐图片提取特征
/// - POST /compare             比较两个特征
pub fn create_rest_router(service: Arc<FaceServiceImpl>) -> Router {
    Router::new()
        .route("/detect", post(detect_faces_handler))
        .route("/extract", post(extract_features_handler))
        .route("/extract-aligned", post(extract_aligned_handler))
        .route("/compare", post(compare_features_handler))
        .with_state(service)
}

#[derive(serde::Deserialize)]
struct DetectQuery {
    #[serde(default)]
    det_threshold: Option<f32>,
    #[serde(default)]
    max_faces: Option<i32>,
}

async fn detect_faces_handler(
    State(service): State<Arc<FaceServiceImpl>>,
    Query(query): Query<DetectQuery>,
    body: axum::body::Bytes,
) -> Response {
    let req = DetectFacesRequest {
        image_data: body.to_vec(),
        det_threshold: query.det_threshold.unwrap_or(0.0),
        max_faces: query.max_faces.unwrap_or(0),
    };

    match service
        .detect_faces(Request::new(req))
        .await
    {
        Ok(resp) => {
            let resp = resp.into_inner();
            // 手动构造 JSON，避免 proto 类型未实现 Serialize
            let faces_json: Vec<serde_json::Value> = resp
                .faces
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "bbox": f.bbox,
                        "score": f.score,
                        "landmarks": f.landmarks,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "success": resp.success,
                "message": resp.message,
                "faces": faces_json,
                "image_width": resp.image_width,
                "image_height": resp.image_height,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("人脸检测失败: {}", e),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ExtractQuery {
    #[serde(default)]
    det_threshold: Option<f32>,
    #[serde(default)]
    max_faces: Option<i32>,
    #[serde(default)]
    save_aligned_images: Option<bool>,
    #[serde(default)]
    image_bucket: Option<String>,
    #[serde(default)]
    return_aligned_images: Option<bool>,
    #[serde(default)]
    index_embedding: Option<bool>,
}

async fn extract_features_handler(
    State(service): State<Arc<FaceServiceImpl>>,
    Query(query): Query<ExtractQuery>,
    body: axum::body::Bytes,
) -> Response {
    let req = ExtractFaceFeaturesRequest {
        image_data: body.to_vec(),
        det_threshold: query.det_threshold.unwrap_or(0.0),
        max_faces: query.max_faces.unwrap_or(0),
        save_aligned_images: query.save_aligned_images.unwrap_or(false),
        image_bucket: query.image_bucket.unwrap_or_default(),
        return_aligned_images: query.return_aligned_images.unwrap_or(false),
        index_embedding: query.index_embedding.unwrap_or(false),
    };

    match service
        .extract_face_features(Request::new(req))
        .await
    {
        Ok(resp) => {
            let resp = resp.into_inner();
            Json(serde_json::json!({
                "success": resp.success,
                "message": resp.message,
                "faces": resp.faces.iter().map(|f| {
                    let detection = f.detection.as_ref();
                    serde_json::json!({
                        "detection": detection.map(|d| serde_json::json!({
                            "bbox": d.bbox,
                            "score": d.score,
                            "landmarks": d.landmarks,
                        })),
                        "embedding": f.embedding,
                        "saved_image_key": f.saved_image_key,
                        "saved_image_bucket": f.saved_image_bucket,
                        "indexed_vector_id": f.indexed_vector_id,
                        "has_aligned_image": !f.aligned_image.is_empty(),
                    })
                }).collect::<Vec<_>>(),
                "image_width": resp.image_width,
                "image_height": resp.image_height,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("人脸特征提取失败: {}", e),
        )
            .into_response(),
    }
}

async fn extract_aligned_handler(
    State(service): State<Arc<FaceServiceImpl>>,
    body: axum::body::Bytes,
) -> Response {
    let req = ExtractFeatureFromAlignedRequest {
        aligned_image_data: body.to_vec(),
    };

    match service
        .extract_feature_from_aligned(Request::new(req))
        .await
    {
        Ok(resp) => {
            let resp = resp.into_inner();
            Json(serde_json::json!({
                "success": resp.success,
                "message": resp.message,
                "embedding": resp.embedding,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("特征提取失败: {}", e),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CompareFeaturesJson {
    feature1: Vec<f32>,
    feature2: Vec<f32>,
}

async fn compare_features_handler(
    State(_service): State<Arc<FaceServiceImpl>>,
    Json(req): Json<CompareFeaturesJson>,
) -> Response {
    if req.feature1.len() != EMBEDDING_DIM || req.feature2.len() != EMBEDDING_DIM {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "特征维度应为 {}，实际 f1={}, f2={}",
                EMBEDDING_DIM,
                req.feature1.len(),
                req.feature2.len()
            ),
        )
            .into_response();
    }

    let dot: f32 = req
        .feature1
        .iter()
        .zip(req.feature2.iter())
        .map(|(a, b)| a * b)
        .sum();
    let is_same = dot >= SAME_PERSON_THRESHOLD;

    Json(serde_json::json!({
        "success": true,
        "similarity": dot,
        "is_same_person": is_same,
    }))
    .into_response()
}

/// 提供 gRPC FaceServiceServer
pub fn into_grpc_service(svc: Arc<FaceServiceImpl>) -> FaceServiceServer<FaceServiceImpl> {
    FaceServiceServer::from_arc(svc)
}