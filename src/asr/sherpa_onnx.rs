#![allow(dead_code)]

use crate::asr::types::{AsrBackendKind, AsrEngine, AsrRuntimeInfo, AsrSession, AsrSessionConfig};
use crate::audio::buffer::AudioRingBuffer;
use anyhow::{Context, Result};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

#[derive(Debug, Clone)]
pub struct SherpaSenseVoiceConfig {
    pub model_path: String,
    pub tokens_path: String,
    pub language: Option<String>,
    pub use_itn: bool,
    pub provider: Option<String>,
    pub num_threads: i32,
    pub sample_rate: i32,
}

impl SherpaSenseVoiceConfig {
    pub fn validate(&self) -> Result<()> {
        if !Path::new(&self.model_path).exists() {
            anyhow::bail!("未找到 SenseVoice 模型文件: {}", self.model_path);
        }
        if !Path::new(&self.tokens_path).exists() {
            anyhow::bail!("未找到 SenseVoice tokens 文件: {}", self.tokens_path);
        }
        Ok(())
    }
}

pub struct SherpaSenseVoiceEngine {
    cfg: SherpaSenseVoiceConfig,
}

struct SherpaSenseVoiceSession {
    recognizer: OfflineRecognizer,
    sample_rate: i32,
    full_audio: Vec<f32>,
    partial_window: AudioRingBuffer,
    min_partial_samples: usize,
    has_pending_audio: bool,
}

impl SherpaSenseVoiceEngine {
    pub fn new(cfg: SherpaSenseVoiceConfig) -> Result<Self> {
        cfg.validate()?;
        build_offline_recognizer(&cfg)
            .with_context(|| format!("创建 sherpa SenseVoice 识别器失败: {}", cfg.model_path))?;

        Ok(Self { cfg })
    }

    fn model_display_name(&self) -> String {
        Path::new(&self.cfg.model_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(self.cfg.model_path.as_str())
            .to_string()
    }
}

impl AsrEngine for SherpaSenseVoiceEngine {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::SherpaSenseVoice
    }

    fn runtime_info(&self) -> AsrRuntimeInfo {
        AsrRuntimeInfo {
            backend: self.backend_kind(),
            model: self.model_display_name(),
            threads: Some(self.cfg.num_threads),
            detail: Some(format!(
                "provider={} language={} itn={}",
                self.cfg.provider.as_deref().unwrap_or("cpu"),
                self.cfg.language.as_deref().unwrap_or("auto"),
                self.cfg.use_itn
            )),
        }
    }

    fn start_session(&self, config: AsrSessionConfig) -> Result<Box<dyn AsrSession>> {
        Ok(Box::new(SherpaSenseVoiceSession {
            recognizer: build_offline_recognizer(&self.cfg)?,
            sample_rate: self.cfg.sample_rate,
            full_audio: Vec::new(),
            partial_window: AudioRingBuffer::with_capacity(config.max_partial_window_samples),
            min_partial_samples: config.min_partial_samples,
            has_pending_audio: false,
        }))
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        let recognizer = build_offline_recognizer(&self.cfg)?;
        decode_audio(&recognizer, self.cfg.sample_rate, audio)
    }
}

impl AsrSession for SherpaSenseVoiceSession {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::SherpaSenseVoice
    }

    fn accept_audio(&mut self, audio: &[f32]) -> Result<()> {
        if audio.is_empty() {
            return Ok(());
        }

        self.full_audio.extend_from_slice(audio);
        self.partial_window.push_samples(audio);
        self.has_pending_audio = true;
        Ok(())
    }

    fn poll_partial(&mut self, abort_flag: Arc<AtomicBool>) -> Result<Option<String>> {
        if abort_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(None);
        }

        if !self.has_pending_audio || self.partial_window.len() < self.min_partial_samples {
            return Ok(None);
        }

        let preview_audio = self.partial_window.snapshot();
        self.has_pending_audio = false;
        let text = decode_audio(&self.recognizer, self.sample_rate, &preview_audio)?;
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
            return Ok(None);
        }

        Ok(Some(trimmed.to_string()))
    }

    fn finalize(&mut self, abort_flag: Arc<AtomicBool>) -> Result<String> {
        if abort_flag.load(std::sync::atomic::Ordering::Relaxed) || self.full_audio.is_empty() {
            return Ok(String::new());
        }

        decode_audio(&self.recognizer, self.sample_rate, &self.full_audio)
    }

    fn buffered_samples(&self) -> usize {
        self.full_audio.len()
    }
}

fn build_offline_recognizer(cfg: &SherpaSenseVoiceConfig) -> Result<OfflineRecognizer> {
    let mut recognizer_cfg = OfflineRecognizerConfig::default();
    recognizer_cfg.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(cfg.model_path.clone()),
        language: cfg.language.clone(),
        use_itn: cfg.use_itn,
    };
    recognizer_cfg.model_config.tokens = Some(cfg.tokens_path.clone());
    recognizer_cfg.model_config.num_threads = cfg.num_threads.max(1);
    recognizer_cfg.model_config.provider = cfg.provider.clone();

    OfflineRecognizer::create(&recognizer_cfg)
        .context("sherpa-onnx OfflineRecognizer::create 返回空指针")
}

fn decode_audio(recognizer: &OfflineRecognizer, sample_rate: i32, audio: &[f32]) -> Result<String> {
    if audio.is_empty() {
        return Ok(String::new());
    }

    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, audio);
    recognizer.decode(&stream);
    let result = stream.get_result().context("sherpa-onnx 未返回识别结果")?;
    Ok(result.text)
}
