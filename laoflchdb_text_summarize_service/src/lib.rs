//! 文本摘要服务（基于 candle + T5/Flan-T5）
//!
//! 针对中文和英文文本生成摘要。通过 gRPC 对外提供服务。

pub mod proto {
    tonic::include_proto!("laoflchdb.text_summarize");
}

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use candle_core::{DType, Device, Tensor};
use candle_nn::ops::softmax;
use candle_nn::VarBuilder;
use candle_transformers::models::t5::{Config as T5Config, T5ForConditionalGeneration};
use log::{info, warn};
use std::sync::Mutex;
use tonic::{Request, Response, Status};

use crate::proto::text_summarize_service_server::TextSummarizeService as TextSummarizeServiceTrait;
use crate::proto::{
    HealthCheckRequest, HealthCheckResponse, SummarizeRequest, SummarizeResponse,
};

pub use crate::proto::text_summarize_service_server::TextSummarizeServiceServer as SummarizeServiceServer;

/// 文本摘要服务配置
#[derive(Debug, Clone)]
pub struct TextSummarizeServiceConfig {
    /// 模型目录，需包含 config.json / tokenizer.json / model.safetensors
    pub model_path: String,
    /// 是否使用 GPU（需启用 cuda feature）
    pub use_cuda: bool,
    /// 输入最大 token 数（默认 1024）
    pub max_input_tokens: usize,
    /// 摘要默认最大输出 token 数
    pub default_max_length: usize,
    /// 摘要默认最小输出 token 数
    pub default_min_length: usize,
    /// 中文任务前缀
    pub prefix_zh: String,
    /// 英文任务前缀
    pub prefix_en: String,
    /// 权重精度: "f32" / "f16"（fp16 可大幅降低显存占用）
    pub dtype: String,
}

impl Default for TextSummarizeServiceConfig {
    fn default() -> Self {
        Self {
            model_path: "laoflch_db_model/flan-t5-small".to_string(),
            use_cuda: false,
            max_input_tokens: 1024,
            default_max_length: 150,
            default_min_length: 40,
            prefix_zh: "请用中文总结以下内容：\n".to_string(),
            prefix_en: "summarize: ".to_string(),
            dtype: "f32".to_string(),
        }
    }
}

/// 文本摘要服务
pub struct TextSummarizeService {
    model: Mutex<T5ForConditionalGeneration>,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
    service_config: TextSummarizeServiceConfig,
    eos_token_id: u32,
    decoder_start_token_id: u32,
}

impl TextSummarizeService {
    /// 加载模型并构建服务实例
    pub fn new(service_config: TextSummarizeServiceConfig) -> anyhow::Result<Self> {
        let device = detect_device(service_config.use_cuda);
        let path = Path::new(&service_config.model_path);

        // 加载 config.json
        let config_path = path.join("config.json");
        let config_json = std::fs::read_to_string(&config_path)?;
        let config: T5Config = serde_json::from_str(&config_json)?;
        info!(
            "T5 配置加载成功: vocab={}, d_model={}, layers={}, heads={}",
            config.vocab_size, config.d_model, config.num_layers, config.num_heads
        );

        // 加载 tokenizer.json
        let tokenizer_path = path.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow::anyhow!("加载 tokenizer 失败: {}", e))?;

        // 加载 model.safetensors
        let safetensors_path = path.join("model.safetensors");
        let tensors = candle_core::safetensors::load(safetensors_path, &device)?;
        let dtype = parse_dtype(&service_config.dtype)?;
        info!("T5 模型权重精度: {:?}", dtype);
        let vb = VarBuilder::from_tensors(tensors, dtype, &device);

        // 构建 T5 模型
        let model = T5ForConditionalGeneration::load(vb, &config)?;
        info!("T5 模型加载成功: {}", service_config.model_path);

        // 特殊 token id
        let eos_token_id = config.eos_token_id as u32;
        let decoder_start_token_id =
            config.decoder_start_token_id.unwrap_or(config.pad_token_id) as u32;

        Ok(Self {
            model: Mutex::new(model),
            tokenizer,
            device,
            service_config,
            eos_token_id,
            decoder_start_token_id,
        })
    }

    /// 检测文本语言："zh" / "en"
    fn detect_language(&self, text: &str) -> &'static str {
        let mut zh_chars = 0usize;
        let mut total = 0usize;
        for c in text.chars() {
            if c.is_alphabetic() {
                total += 1;
                if ('\u{4e00}'..='\u{9fff}').contains(&c) {
                    zh_chars += 1;
                }
            }
        }
        if total == 0 {
            return "en";
        }
        if (zh_chars as f64 / total as f64) > 0.3 {
            "zh"
        } else {
            "en"
        }
    }

    /// 执行文本摘要
    pub fn generate_summary(
        &self,
        text: &str,
        target_language: Option<&str>,
        max_length: i32,
        min_length: i32,
        temperature: f32,
    ) -> anyhow::Result<(String, String, u128, usize, usize)> {
        let start = Instant::now();

        // 语言检测
        let detected = self.detect_language(text);
        let target_lang = match target_language {
            Some(l) if l == "zh" || l == "en" => l,
            _ => detected,
        };

        // 构造任务前缀
        let prefix = if target_lang == "zh" {
            self.service_config.prefix_zh.as_str()
        } else {
            self.service_config.prefix_en.as_str()
        };
        let full_text = format!("{}{}", prefix, text);

        // 编码输入
        let encoding = self
            .tokenizer
            .encode(full_text, true)
            .map_err(|e| anyhow::anyhow!("tokenize 失败: {}", e))?;
        let mut input_ids: Vec<u32> = encoding.get_ids().to_vec();

        // 截断输入
        if input_ids.len() > self.service_config.max_input_tokens {
            warn!(
                "输入长度 {} 超过 max_input_tokens={}, 已截断",
                input_ids.len(),
                self.service_config.max_input_tokens
            );
            input_ids.truncate(self.service_config.max_input_tokens);
        }
        let input_len = input_ids.len();

        // 长度参数
        let max_len = if max_length > 0 {
            max_length as usize
        } else {
            self.service_config.default_max_length
        };
        let min_len = if min_length > 0 {
            min_length as usize
        } else {
            self.service_config.default_min_length
        };
        let temperature = if temperature > 0.0 { temperature } else { 0.0 };

        // 自回归生成
        let mut model = self.model.lock().unwrap();
        let input_tensor = Tensor::new(input_ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let encoder_output = model.encode(&input_tensor)?;

        let mut decoder_input_ids =
            Tensor::new(&[self.decoder_start_token_id], &self.device)?.unsqueeze(0)?;
        let mut output_ids: Vec<u32> = Vec::new();

        for _ in 0..max_len {
            let logits = model.decode(&decoder_input_ids, &encoder_output)?;
            model.clear_kv_cache();
            // decode 已内部取最后一个位置的 logits 并投影到词表，输出形状为 [batch, vocab]
            let last_logits = logits.squeeze(0)?;

            let next_token: u32 = if temperature > 0.0 && output_ids.len() >= min_len {
                // 温度采样
                let temp_t = Tensor::new(&[temperature], &self.device)?;
                let scaled = (&last_logits).div(&temp_t)?;
                let probs = softmax(&scaled, 0)?;
                let probs_cpu = probs.to_vec1::<f32>()?;
                sample_from_probs(&probs_cpu)
            } else {
                // 贪婪解码
                let next_idx = last_logits.argmax(0)?;
                next_idx.to_scalar::<u32>()?
            };

            output_ids.push(next_token);
            if next_token == self.eos_token_id {
                break;
            }

            let next_tensor = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;
            decoder_input_ids = Tensor::cat(&[decoder_input_ids, next_tensor], 1)?;
        }

        // 解码输出（跳过 pad/eos 特殊 token）
        let summary = self
            .tokenizer
            .decode(&output_ids, true)
            .map_err(|e| anyhow::anyhow!("decode 失败: {}", e))?;
        let summary = summary.trim().to_string();

        let elapsed_ms = start.elapsed().as_millis();
        let output_len = summary.chars().count();

        Ok((
            summary,
            target_lang.to_string(),
            elapsed_ms,
            input_len,
            output_len,
        ))
    }

    /// 健康检查：模型是否就绪
    pub fn is_ready(&self) -> bool {
        true
    }

    pub fn model_name(&self) -> String {
        Path::new(&self.service_config.model_path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| self.service_config.model_path.clone())
    }
}

/// 解析权重精度字符串为 DType
fn parse_dtype(s: &str) -> anyhow::Result<DType> {
    match s.to_ascii_lowercase().as_str() {
        "f32" | "fp32" => Ok(DType::F32),
        "f16" | "fp16" => Ok(DType::F16),
        other => Err(anyhow::anyhow!("不支持的 dtype: {}（支持 f32/fp32、f16/fp16）", other)),
    }
}

/// 根据概率分布采样一个 token
fn sample_from_probs(probs: &[f32]) -> u32 {
    // 使用当前时间作为简单随机种子
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut state = seed as u64 ^ 0x9E3779B97F4A7C15;
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    let r = (state >> 11) as f32 / (u64::MAX >> 11) as f32;
    let mut cum = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cum += p;
        if r <= cum {
            return i as u32;
        }
    }
    (probs.len() as u32).saturating_sub(1)
}

/// 设备检测：CUDA 可用则用 GPU，否则回退 CPU
fn detect_device(_use_cuda: bool) -> Device {
    #[cfg(feature = "cuda")]
    {
        if _use_cuda {
            info!("CUDA feature 已启用，检测 GPU...");
            match Device::cuda_if_available(0) {
                Ok(device) => {
                    info!("CUDA GPU 可用，使用 GPU 设备");
                    return device;
                }
                Err(e) => {
                    warn!("CUDA 设备初始化失败: {}，回退到 CPU", e);
                }
            }
        } else {
            info!("配置禁用 CUDA，使用 CPU");
        }
    }

    #[cfg(not(feature = "cuda"))]
    {
        info!("CUDA feature 未启用，使用 CPU 设备");
        info!("提示: 如需启用 GPU 加速，请使用: cargo build --release --features cuda");
    }

    Device::Cpu
}

#[tonic::async_trait]
impl TextSummarizeServiceTrait for Arc<TextSummarizeService> {
    async fn summarize(
        &self,
        request: Request<SummarizeRequest>,
    ) -> Result<Response<SummarizeResponse>, Status> {
        let req = request.into_inner();
        if req.text.trim().is_empty() {
            return Ok(Response::new(SummarizeResponse {
                success: false,
                message: "输入文本不能为空".to_string(),
                summary: String::new(),
                detected_language: String::new(),
                processing_time_ms: 0,
                input_length: 0,
                output_length: 0,
            }));
        }

        match self.generate_summary(
            &req.text,
            Some(&req.target_language),
            req.max_length,
            req.min_length,
            req.temperature,
        ) {
            Ok((summary, detected, elapsed_ms, input_len, output_len)) => {
                Ok(Response::new(SummarizeResponse {
                    success: true,
                    message: "ok".to_string(),
                    summary,
                    detected_language: detected,
                    processing_time_ms: elapsed_ms as i64,
                    input_length: input_len as i32,
                    output_length: output_len as i32,
                }))
            }
            Err(e) => {
                let msg = format!("摘要生成失败: {}", e);
                warn!("{}", msg);
                Ok(Response::new(SummarizeResponse {
                    success: false,
                    message: msg,
                    summary: String::new(),
                    detected_language: String::new(),
                    processing_time_ms: 0,
                    input_length: 0,
                    output_length: 0,
                }))
            }
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        Ok(Response::new(HealthCheckResponse {
            ready: self.is_ready(),
            model_name: self.model_name(),
            model_status: "ready".to_string(),
        }))
    }
}
