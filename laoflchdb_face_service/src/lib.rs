//! 人脸服务 (FaceService)
//!
//! 技术体系：SCRFD ONNX 人脸检测 + 5 关键点 → 仿射对齐裁剪 112×112 → ArcFace 提取 512 维特征 + L2 归一化
//!
//! 基于 candle-onnx 加载 ONNX 模型推理，支持 CUDA GPU 加速（通过 `cuda` feature 启用）。
//! 可选通过 laoflchdb_image_service 保存对齐后的人脸图片。

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
use candle_core::{Device, Tensor};
use image::{ImageBuffer, Rgba, RgbaImage};
use log::{info, warn};
use snowflake_me::Snowflake;
use tonic::{Request, Response as TonicResponse, Status};

pub mod proto {
    tonic::include_proto!("laoflchdb.face_service");
}

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
/// SCRFD 多尺度输入尺寸（长边）
const DETECT_INPUT_SIZE: u32 = 640;

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
/// 基于 SCRFD 检测人脸 + 5 关键点，仿射对齐到 112x112，ArcFace 提取 512 维 L2 归一化特征。
/// 支持 CUDA GPU 加速（通过 `cuda` feature）。
pub struct FaceServiceImpl {
    /// SCRFD 检测模型
    scrfd: Mutex<Option<ScrfdModel>>,
    /// ArcFace 特征提取模型
    arcface: Mutex<Option<ArcfaceModel>>,
    /// 推理设备
    device: Device,
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
        let device = Self::detect_device();

        // 加载 SCRFD 模型
        let scrfd = Self::load_scrfd(&config, &device);
        // 加载 ArcFace 模型
        let arcface = Self::load_arcface(&config, &device);

        let snowflake = Snowflake::new().unwrap_or_else(|_| {
            warn!("Snowflake 默认初始化失败，回退到 machine_id=0, data_center_id=0");
            Snowflake::builder()
                .machine_id(&|| Ok(0u16))
                .data_center_id(&|| Ok(0u16))
                .finalize()
                .expect("Snowflake with machine_id=0, data_center_id=0 must succeed")
        });

        info!(
            "FaceService 初始化完成: device={:?}, scrfd_loaded={}, arcface_loaded={}",
            device,
            scrfd.is_some(),
            arcface.is_some()
        );

        Self {
            scrfd: Mutex::new(scrfd),
            arcface: Mutex::new(arcface),
            device,
            config,
            image_service,
            embedding_service,
            snowflake: Mutex::new(snowflake),
        }
    }

    /// 检测推理设备：CUDA 优先，回退 CPU
    fn detect_device() -> Device {
        #[cfg(feature = "cuda")]
        {
            info!("CUDA feature 已启用，检测 GPU...");
            match Device::cuda_if_available(0) {
                Ok(device) => {
                    info!("CUDA GPU 可用，使用 GPU 设备进行人脸推理");
                    return device;
                }
                Err(e) => {
                    warn!("CUDA 设备初始化失败: {}，回退到 CPU", e);
                }
            }
        }

        #[cfg(not(feature = "cuda"))]
        {
            info!("CUDA feature 未启用，使用 CPU 设备进行人脸推理");
            info!("提示: 如需启用 GPU 加速，请使用: cargo build --release --features cuda");
        }

        Device::Cpu
    }

    /// 加载 SCRFD ONNX 模型
    fn load_scrfd(config: &FaceServiceConfig, device: &Device) -> Option<ScrfdModel> {
        let model_path = Path::new(&config.model_dir).join(&config.scrfd_model_file);
        if !model_path.exists() {
            warn!(
                "SCRFD 模型文件不存在: {}，人脸检测功能不可用",
                model_path.display()
            );
            return None;
        }

        info!("加载 SCRFD 模型: {}", model_path.display());
        match candle_onnx::read_file(&model_path) {
            Ok(mut model) => {
                // 预处理 Resize 算子：清空冲突的 scales 输入（保留 sizes）
                // ONNX Resize 输入顺序: [X, roi, scales, sizes]
                // SCRFD 模型中 scales 和 roi 用了同一个 initializer，导致 candle-onnx 报错
                // 解决：将 scales 输入（第 3 个，index=2）设为空字符串
                if let Some(ref mut graph) = model.graph.as_mut() {
                    let mut fixed = 0;
                    for node in &mut graph.node {
                        if node.op_type == "Resize" && node.input.len() >= 4 {
                            // input[2] 是 scales，清空它让 candle-onnx 忽略
                            if !node.input[2].is_empty() {
                                node.input[2] = String::new();
                                fixed += 1;
                            }
                        }
                    }
                    if fixed > 0 {
                        info!("SCRFD 模型预处理: 修复了 {} 个 Resize 节点的 scales/sizes 冲突", fixed);
                    }
                }
                info!("SCRFD 模型加载成功");
                Some(ScrfdModel {
                    model,
                    device: device.clone(),
                })
            }
            Err(e) => {
                warn!("SCRFD 模型加载失败: {}", e);
                None
            }
        }
    }

    /// 加载 ArcFace ONNX 模型
    fn load_arcface(config: &FaceServiceConfig, device: &Device) -> Option<ArcfaceModel> {
        let model_path = Path::new(&config.model_dir).join(&config.arcface_model_file);
        if !model_path.exists() {
            warn!(
                "ArcFace 模型文件不存在: {}，人脸特征提取功能不可用",
                model_path.display()
            );
            return None;
        }

        info!("加载 ArcFace 模型: {}", model_path.display());
        match candle_onnx::read_file(&model_path) {
            Ok(model) => {
                info!("ArcFace 模型加载成功");
                Some(ArcfaceModel {
                    model,
                    device: device.clone(),
                })
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
    model: candle_onnx::onnx::ModelProto,
    device: Device,
}

/// ArcFace 特征提取模型封装
struct ArcfaceModel {
    model: candle_onnx::onnx::ModelProto,
    device: Device,
}

// ── SCRFD 检测实现 ───────────────────────────────────────────────

impl ScrfdModel {
    /// 运行 SCRFD 检测
    ///
    /// 输入图片，返回检测到的人脸列表（bbox + score + 5 landmarks）
    fn detect(
        &self,
        img: &image::DynamicImage,
        threshold: f32,
        max_faces: i32,
    ) -> Result<Vec<DetectedFaceInfo>, Status> {
        let (orig_w, orig_h) = (img.width(), img.height());

        // 预处理：resize 到 640x640，保持长宽比的 letterbox
        let (input_tensor, scale, pad_w, pad_h) =
            Self::preprocess(img, DETECT_INPUT_SIZE, &self.device)?;

        // 构造输入 map
        let graph = self.model.graph.as_ref().ok_or_else(|| {
            Status::internal("SCRFD 模型无计算图")
        })?;
        let input_name = graph
            .input
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| Status::internal("SCRFD 模型无输入"))?;

        let mut inputs = std::collections::HashMap::new();
        inputs.insert(input_name, input_tensor);

        // 推理
        let outputs = candle_onnx::simple_eval(&self.model, inputs).map_err(|e| {
            Status::internal(format!("SCRFD 推理失败: {}", e))
        })?;

        // 解析输出：SCRFD 输出多个尺度的 bbox/score/kps
        // 典型输出名: score_0, score_1, score_2, bbox_0, bbox_1, bbox_2, kps_0, kps_1, kps_2
        // 或按顺序排列
        let mut all_faces = Vec::new();

        // 按输出顺序解析（3 个尺度）
        let mut sorted_outputs: Vec<(&String, &Tensor)> = outputs.iter().collect();
        sorted_outputs.sort_by(|a, b| a.0.cmp(b.0));

        // SCRFD 有 9 个输出，按 stride 分组：每组 3 个 (score, bbox, kps)
        // 模型实际输出顺序（按名称排序后）：[s8_score, s8_bbox, s8_kps, s16_score, s16_bbox, s16_kps, s32_score, s32_bbox, s32_kps]
        // 因此连续 3 个为一组，而不是 i, i+3, i+6
        let strides = [8u32, 16, 32];
        for (i, &stride) in strides.iter().enumerate() {
            let score_idx = i * 3;
            let bbox_idx = i * 3 + 1;
            let kps_idx = i * 3 + 2;

            if sorted_outputs.len() < 9 {
                break;
            }

            let score_tensor = sorted_outputs[score_idx].1;
            let bbox_tensor = sorted_outputs[bbox_idx].1;
            let kps_tensor = sorted_outputs[kps_idx].1;

            let faces = Self::parse_stride_output(
                score_tensor,
                bbox_tensor,
                kps_tensor,
                stride,
                DETECT_INPUT_SIZE,
                orig_w,
                orig_h,
                scale,
                pad_w,
                pad_h,
                threshold,
                &self.device,
            )?;
            all_faces.extend(faces);
        }

        // 按置信度降序排序
        all_faces.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 限制最大数量
        if max_faces > 0 && all_faces.len() > max_faces as usize {
            all_faces.truncate(max_faces as usize);
        }

        Ok(all_faces)
    }

    /// 预处理：letterbox resize 到目标尺寸
    ///
    /// 返回 (input_tensor, scale, pad_w, pad_h)
    fn preprocess(
        img: &image::DynamicImage,
        target_size: u32,
        device: &Device,
    ) -> Result<(Tensor, f32, u32, u32), Status> {
        let (orig_w, orig_h) = (img.width(), img.height());
        let scale = target_size as f32 / orig_w.max(orig_h) as f32;
        let new_w = (orig_w as f32 * scale).round() as u32;
        let new_h = (orig_h as f32 * scale).round() as u32;
        let pad_w = (target_size - new_w) / 2;
        let pad_h = (target_size - new_h) / 2;

        // resize
        let resized = img.resize_exact(
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        );

        // 转换为 RGB 并填充到 target_size x target_size
        let rgb = resized.to_rgb8();
        let mut pixels: Vec<f32> = Vec::with_capacity((target_size * target_size * 3) as usize);

        // 填充值为 0（黑色）
        // 先按行填充
        for y in 0..target_size {
            for x in 0..target_size {
                if x < new_w && y < new_h {
                    let p = rgb.get_pixel(x, y);
                    // SCRFD 使用 BGR 顺序，归一化到 [0,1]，均值 [0.485, 0.456, 0.406]，方差 [0.229, 0.224, 0.225]
                    let r = p.0[0] as f32 / 255.0;
                    let g = p.0[1] as f32 / 255.0;
                    let b = p.0[2] as f32 / 255.0;
                    // BGR
                    let b = (b - 0.406) / 0.225;
                    let g = (g - 0.456) / 0.224;
                    let r = (r - 0.485) / 0.229;
                    pixels.push(b);
                    pixels.push(g);
                    pixels.push(r);
                } else {
                    pixels.push(0.0);
                    pixels.push(0.0);
                    pixels.push(0.0);
                }
            }
        }

        // 构造 tensor [1, 3, H, W] NCHW
        let input_tensor = Tensor::from_vec(pixels, (1, 3, target_size as usize, target_size as usize), device)
            .map_err(|e| Status::internal(format!("构造输入 tensor 失败: {}", e)))?;

        Ok((input_tensor, scale, pad_w, pad_h))
    }

    /// 解析单个 stride 的输出
    #[allow(clippy::too_many_arguments)]
    fn parse_stride_output(
        score_tensor: &Tensor,
        bbox_tensor: &Tensor,
        kps_tensor: &Tensor,
        stride: u32,
        input_size: u32,
        orig_w: u32,
        orig_h: u32,
        scale: f32,
        pad_w: u32,
        pad_h: u32,
        threshold: f32,
        device: &Device,
    ) -> Result<Vec<DetectedFaceInfo>, Status> {
        // 将 tensor 移到 CPU 提取数据
        let score_tensor = score_tensor.to_device(&Device::Cpu).map_err(|e| {
            Status::internal(format!("score tensor 转换失败: {}", e))
        })?;
        let bbox_tensor = bbox_tensor.to_device(&Device::Cpu).map_err(|e| {
            Status::internal(format!("bbox tensor 转换失败: {}", e))
        })?;
        let kps_tensor = kps_tensor.to_device(&Device::Cpu).map_err(|e| {
            Status::internal(format!("kps tensor 转换失败: {}", e))
        })?;

        // SCRFD 输出维度可能为 [num_anchors, 1] (2D) 或 [batch, num_anchors, 1] (3D)
        // 这里统一 flatten 到 1D 处理
        let score_dims = score_tensor.dims();
        let num_anchors = if score_dims.len() >= 2 {
            score_dims[score_dims.len() - 2]
        } else {
            score_dims.first().copied().unwrap_or(0)
        };

        // 将 score_tensor flatten 到 1D: [num_anchors * 1] → [num_anchors]
        let scores = score_tensor
            .flatten_all()
            .map_err(|e| Status::internal(format!("score flatten 失败: {}", e)))?
            .to_vec1::<f32>()
            .map_err(|e| Status::internal(format!("score 提取失败: {}", e)))?;
        // bbox/kps 可能是 2D [num_anchors, 4/10] 或 3D [batch, num_anchors, 4/10]
        // 去掉 batch 维度（如果有）
        let bboxes = if bbox_tensor.dims().len() == 3 {
            bbox_tensor
                .squeeze(0)
                .map_err(|e| Status::internal(format!("bbox squeeze 失败: {}", e)))?
                .to_vec2::<f32>()
                .map_err(|e| Status::internal(format!("bbox 提取失败: {}", e)))?
        } else {
            bbox_tensor
                .to_vec2::<f32>()
                .map_err(|e| Status::internal(format!("bbox 提取失败: {}", e)))?
        };
        let kps = if kps_tensor.dims().len() == 3 {
            kps_tensor
                .squeeze(0)
                .map_err(|e| Status::internal(format!("kps squeeze 失败: {}", e)))?
                .to_vec2::<f32>()
                .map_err(|e| Status::internal(format!("kps 提取失败: {}", e)))?
        } else {
            kps_tensor
                .to_vec2::<f32>()
                .map_err(|e| Status::internal(format!("kps 提取失败: {}", e)))?
        };

        let _ = device; // 已转 CPU

        // 计算每个 anchor 的中心点
        // SCRFD 每个特征图 cell 有 num_anchors_per_pos 个 anchor（默认 2）
        // num_cells = (input_size / stride)^2，每个 cell 对应 num_anchors_per_pos 个 anchor
        let num_cells_per_row = input_size / stride;
        let num_anchors_per_pos = 2u32; // SCRFD 默认每个位置 2 个 anchor
        let mut faces = Vec::new();

        for i in 0..num_anchors {
            let score = scores[i];
            if score < threshold {
                continue;
            }

            // bbox: [4] - distance to left/top/right/bottom
            let row = &bboxes[i];
            if row.len() < 4 {
                continue;
            }
            // 每个 cell 有 num_anchors_per_pos 个 anchor，所以 cell_idx = i / num_anchors_per_pos
            let cell_idx = i as u32 / num_anchors_per_pos;
            let cx = (cell_idx % num_cells_per_row) as f32 * stride as f32 + stride as f32 / 2.0;
            let cy = (cell_idx / num_cells_per_row) as f32 * stride as f32 + stride as f32 / 2.0;

            // 解码 bbox（距离 → 绝对坐标）
            let x1 = cx - row[0] * stride as f32;
            let y1 = cy - row[1] * stride as f32;
            let x2 = cx + row[2] * stride as f32;
            let y2 = cy + row[3] * stride as f32;

            // 反 letterbox：减去 pad，除以 scale
            let x1 = ((x1 - pad_w as f32) / scale).clamp(0.0, orig_w as f32);
            let y1 = ((y1 - pad_h as f32) / scale).clamp(0.0, orig_h as f32);
            let x2 = ((x2 - pad_w as f32) / scale).clamp(0.0, orig_w as f32);
            let y2 = ((y2 - pad_h as f32) / scale).clamp(0.0, orig_h as f32);

            // 解码 landmarks（10 个值：5 个点 x,y）
            let mut landmarks = [0.0f32; 10];
            if kps[i].len() >= 10 {
                for j in 0..5 {
                    let lx = cx + kps[i][j * 2] * stride as f32;
                    let ly = cy + kps[i][j * 2 + 1] * stride as f32;
                    landmarks[j * 2] = ((lx - pad_w as f32) / scale).clamp(0.0, orig_w as f32);
                    landmarks[j * 2 + 1] = ((ly - pad_h as f32) / scale).clamp(0.0, orig_h as f32);
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

// ── ArcFace 特征提取实现 ─────────────────────────────────────────

impl ArcfaceModel {
    /// 从对齐的 112x112 人脸图片提取 512 维特征并 L2 归一化
    fn extract(&self, aligned: &image::DynamicImage) -> Result<Vec<f32>, Status> {
        let input_tensor = Self::preprocess(aligned, &self.device)?;

        let graph = self
            .model
            .graph
            .as_ref()
            .ok_or_else(|| Status::internal("ArcFace 模型无计算图"))?;
        let input_name = graph
            .input
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| Status::internal("ArcFace 模型无输入"))?;

        let mut inputs = std::collections::HashMap::new();
        inputs.insert(input_name, input_tensor);

        let outputs = candle_onnx::simple_eval(&self.model, inputs).map_err(|e| {
            Status::internal(format!("ArcFace 推理失败: {}", e))
        })?;

        // 取第一个输出
        let (_name, embedding_tensor) = outputs
            .iter()
            .next()
            .ok_or_else(|| Status::internal("ArcFace 无输出"))?;

        let embedding_tensor = embedding_tensor
            .to_device(&Device::Cpu)
            .map_err(|e| Status::internal(format!("embedding tensor 转换失败: {}", e)))?;

        // ArcFace 输出维度可能是 [1, 512] (2D) 或 [512] (1D)，统一 flatten 到 1D
        let embedding = embedding_tensor
            .flatten_all()
            .map_err(|e| Status::internal(format!("embedding flatten 失败: {}", e)))?
            .to_vec1::<f32>()
            .map_err(|e| Status::internal(format!("embedding 提取失败: {}", e)))?;

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
        device: &Device,
    ) -> Result<Tensor, Status> {
        let rgb = img.to_rgb8();
        let (w, h) = rgb.dimensions();
        if w != FACE_SIZE || h != FACE_SIZE {
            return Err(Status::invalid_argument(format!(
                "对齐图片尺寸应为 {}x{}，实际 {}x{}",
                FACE_SIZE, FACE_SIZE, w, h
            )));
        }

        let mut pixels = Vec::with_capacity((FACE_SIZE * FACE_SIZE * 3) as usize);
        for y in 0..FACE_SIZE {
            for x in 0..FACE_SIZE {
                let p = rgb.get_pixel(x, y);
                // 归一化到 [-1, 1]
                pixels.push((p.0[0] as f32 - 127.5) / 127.5);
                pixels.push((p.0[1] as f32 - 127.5) / 127.5);
                pixels.push((p.0[2] as f32 - 127.5) / 127.5);
            }
        }

        Tensor::from_vec(
            pixels,
            (1usize, 3, FACE_SIZE as usize, FACE_SIZE as usize),
            device,
        )
        .map_err(|e| Status::internal(format!("构造 ArcFace 输入 tensor 失败: {}", e)))
    }
}

// ── 仿射对齐 ─────────────────────────────────────────────────────

/// 检测到的人脸信息（内部结构）
#[derive(Debug, Clone)]
struct DetectedFaceInfo {
    bbox: [f32; 4],
    score: f32,
    landmarks: [f32; 10],
}

/// ArcFace 标准对齐目标关键点（112x112 图像中 5 个关键点的标准位置）
/// 来源：InsightFace 标准 alignment
const ARCFACE_TARGET_LANDMARKS: [[f32; 2]; 5] = [
    [38.2946, 51.6963], // 左眼
    [73.5318, 51.5014], // 右眼
    [56.0252, 71.7366], // 鼻尖
    [41.5493, 92.3655], // 左嘴角
    [70.7299, 92.2041], // 右嘴角
];

/// 基于 5 个关键点计算仿射变换矩阵，并将人脸对齐裁剪到 112x112
///
/// 使用相似变换（旋转+缩放+平移）将检测到的 5 个关键点对齐到标准位置。
/// 参考 InsightFace 的 `get_affine_matrix` 实现。
fn align_face(
    img: &image::DynamicImage,
    landmarks: &[f32; 10],
) -> Result<image::DynamicImage, Status> {
    // 源关键点（检测到的）
    let src_points: [[f32; 2]; 5] = [
        [landmarks[0], landmarks[1]],
        [landmarks[2], landmarks[3]],
        [landmarks[4], landmarks[5]],
        [landmarks[6], landmarks[7]],
        [landmarks[8], landmarks[9]],
    ];

    // 计算相似变换矩阵 M（2x3）
    let m = compute_similarity_transform(&src_points, &ARCFACE_TARGET_LANDMARKS);

    // 应用仿射变换
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut output = ImageBuffer::new(FACE_SIZE, FACE_SIZE);

    for oy in 0..FACE_SIZE {
        for ox in 0..FACE_SIZE {
            // 反向映射：output → input
            let ix = m[0][0] * ox as f32 + m[0][1] * oy as f32 + m[0][2];
            let iy = m[1][0] * ox as f32 + m[1][1] * oy as f32 + m[1][2];

            let pixel = sample_bilinear(&rgba, w, h, ix, iy);
            output.put_pixel(ox, oy, pixel);
        }
    }

    Ok(image::DynamicImage::ImageRgba8(output))
}

/// 双线性采样
fn sample_bilinear(
    img: &RgbaImage,
    w: u32,
    h: u32,
    x: f32,
    y: f32,
) -> Rgba<u8> {
    if x < 0.0 || y < 0.0 || x >= w as f32 - 1.0 || y >= h as f32 - 1.0 {
        return Rgba([0, 0, 0, 255]);
    }

    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let dx = x - x0 as f32;
    let dy = y - y0 as f32;

    let p00 = img.get_pixel(x0, y0);
    let p10 = img.get_pixel(x1, y0);
    let p01 = img.get_pixel(x0, y1);
    let p11 = img.get_pixel(x1, y1);

    let mut result = [0u8; 4];
    for c in 0..4 {
        let v = p00.0[c] as f32 * (1.0 - dx) * (1.0 - dy)
            + p10.0[c] as f32 * dx * (1.0 - dy)
            + p01.0[c] as f32 * (1.0 - dx) * dy
            + p11.0[c] as f32 * dx * dy;
        result[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    Rgba(result)
}

/// 计算相似变换矩阵（2x3）将 src 点对齐到 dst 点
///
/// 相似变换：旋转 + 均匀缩放 + 平移，共 4 个参数（角度、缩放、tx、ty）
/// 使用最小二乘法求解
fn compute_similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [[f32; 3]; 2] {
    // 参考 Umeyama 算法的简化版本
    // 计算 src 和 dst 的质心
    let src_mean = mean_2d(src);
    let dst_mean = mean_2d(dst);

    // 中心化
    let src_centered: Vec<[f32; 2]> = src
        .iter()
        .map(|p| [p[0] - src_mean[0], p[1] - src_mean[1]])
        .collect();
    let dst_centered: Vec<[f32; 2]> = dst
        .iter()
        .map(|p| [p[0] - dst_mean[0], p[1] - dst_mean[1]])
        .collect();

    // 计算旋转和缩放
    // 使用最小二乘法：[d_x, d_y] = R * [s_x, s_y] * scale
    // 简化为 2x2 矩阵求解
    let mut a = [[0.0f64; 2]; 2];
    let mut b = [0.0f64; 2];
    for i in 0..5 {
        let sx = src_centered[i][0] as f64;
        let sy = src_centered[i][1] as f64;
        let dx = dst_centered[i][0] as f64;
        let dy = dst_centered[i][1] as f64;
        a[0][0] += sx * sx + sy * sy;
        a[0][1] += 0.0;
        a[1][0] += 0.0;
        a[1][1] += sx * sx + sy * sy;
        b[0] += sx * dx + sy * dy;
        b[1] += sx * dy - sy * dx;
    }

    // 求解 [cos_theta * scale, sin_theta * scale]
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let scale_cos = if det.abs() > 1e-10 {
        (a[1][1] * b[0] - a[0][1] * b[1]) / det
    } else {
        b[0] / a[0][0]
    };
    let scale_sin = if det.abs() > 1e-10 {
        (-a[1][0] * b[0] + a[0][0] * b[1]) / det
    } else {
        0.0
    };

    // 仿射矩阵 M = [scale_cos, -scale_sin, tx; scale_sin, scale_cos, ty]
    // 其中 tx = dst_mean[0] - scale_cos * src_mean[0] + scale_sin * src_mean[1]
    let scale_cos = scale_cos as f32;
    let scale_sin = scale_sin as f32;
    let tx = dst_mean[0] - scale_cos * src_mean[0] + scale_sin * src_mean[1];
    let ty = dst_mean[1] - scale_sin * src_mean[0] - scale_cos * src_mean[1];

    [
        [scale_cos, -scale_sin, tx],
        [scale_sin, scale_cos, ty],
    ]
}

fn mean_2d(points: &[[f32; 2]; 5]) -> [f32; 2] {
    let mut sx = 0.0;
    let mut sy = 0.0;
    for p in points.iter() {
        sx += p[0];
        sy += p[1];
    }
    [sx / 5.0, sy / 5.0]
}

// ── gRPC 服务实现 ────────────────────────────────────────────────

/// 为 Arc<FaceServiceImpl> 实现 FaceService trait（便于注册到 tonic Server）
#[tonic::async_trait]
impl FaceService for std::sync::Arc<FaceServiceImpl> {
    async fn detect_faces(
        &self,
        request: Request<DetectFacesRequest>,
    ) -> Result<TonicResponse<DetectFacesResponse>, Status> {
        self.as_ref().detect_faces(request).await
    }

    async fn extract_face_features(
        &self,
        request: Request<ExtractFaceFeaturesRequest>,
    ) -> Result<TonicResponse<ExtractFaceFeaturesResponse>, Status> {
        self.as_ref().extract_face_features(request).await
    }

    async fn extract_feature_from_aligned(
        &self,
        request: Request<ExtractFeatureFromAlignedRequest>,
    ) -> Result<TonicResponse<ExtractFeatureFromAlignedResponse>, Status> {
        self.as_ref().extract_feature_from_aligned(request).await
    }

    async fn compare_features(
        &self,
        request: Request<CompareFeaturesRequest>,
    ) -> Result<TonicResponse<CompareFeaturesResponse>, Status> {
        self.as_ref().compare_features(request).await
    }
}

#[tonic::async_trait]
impl FaceService for FaceServiceImpl {
    async fn detect_faces(
        &self,
        request: Request<DetectFacesRequest>,
    ) -> Result<TonicResponse<DetectFacesResponse>, Status> {
        let req = request.into_inner();
        let threshold = if req.det_threshold > 0.0 {
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
        let scrfd_guard = self.scrfd.lock().map_err(|e| {
            Status::internal(format!("SCRFD 模型锁获取失败: {}", e))
        })?;
        let scrfd = scrfd_guard.as_ref().ok_or_else(|| {
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
        let threshold = if req.det_threshold > 0.0 {
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

        // 在同步块中完成检测 + 对齐 + 特征提取（避免 MutexGuard 跨 await）
        let detected_and_embeddings: Vec<(DetectedFaceInfo, image::DynamicImage, Vec<f32>)>;
        {
            let scrfd_guard = self.scrfd.lock().map_err(|e| {
                Status::internal(format!("SCRFD 模型锁获取失败: {}", e))
            })?;
            let scrfd = scrfd_guard.as_ref().ok_or_else(|| {
                Status::failed_precondition("SCRFD 模型未加载，人脸特征提取不可用")
            })?;

            let arcface_guard = self.arcface.lock().map_err(|e| {
                Status::internal(format!("ArcFace 模型锁获取失败: {}", e))
            })?;
            let arcface = arcface_guard.as_ref().ok_or_else(|| {
                Status::failed_precondition("ArcFace 模型未加载，人脸特征提取不可用")
            })?;

            // 1. 检测人脸
            let detected = scrfd.detect(&img, threshold, max_faces)?;

            let mut results = Vec::with_capacity(detected.len());
            for face_info in &detected {
                // 2. 仿射对齐到 112x112
                let aligned = align_face(&img, &face_info.landmarks)?;
                // 3. 提取 512 维特征并 L2 归一化
                let embedding = arcface.extract(&aligned)?;
                results.push((face_info.clone(), aligned, embedding));
            }
            detected_and_embeddings = results;
        }
        // 锁已释放，可以安全 await

        let mut face_features = Vec::with_capacity(detected_and_embeddings.len());
        for (face_info, aligned, embedding) in &detected_and_embeddings {
            // 4. 可选：保存对齐后的人脸图片到 image_service
            // 当 index_embedding=true 时，向量 ID 复用图片的 Snowflake ID，所以即使不保存图片也要生成 ID
            let (saved_key, saved_bucket, face_id) = if req.save_aligned_images {
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

            // 5. 可选：把人脸特征向量写入 embedding_service 的 HNSW 索引
            let indexed_vector_id = if req.index_embedding && face_id > 0 {
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
        let arcface_guard = self.arcface.lock().map_err(|e| {
            Status::internal(format!("ArcFace 模型锁获取失败: {}", e))
        })?;
        let arcface = arcface_guard.as_ref().ok_or_else(|| {
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
