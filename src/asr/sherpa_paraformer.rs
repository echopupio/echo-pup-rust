#![allow(dead_code)]

use crate::asr::types::{AsrBackendKind, AsrEngine, AsrRuntimeInfo, AsrSession, AsrSessionConfig};
use crate::audio::buffer::AudioRingBuffer;
use crate::config::SherpaParaformerConfig;
use anyhow::{Context, Result};
use sherpa_onnx::{
    OnlineModelConfig, OnlineParaformerModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

pub struct SherpaParaformerEngine {
    cfg: SherpaParaformerConfig,
}

struct SherpaParaformerSession {
    recognizer: OnlineRecognizer,
    sample_rate: i32,
    full_audio: Vec<f32>,
    partial_window: AudioRingBuffer,
    min_partial_samples: usize,
    has_pending_audio: bool,
}

impl SherpaParaformerEngine {
    pub fn new(cfg: SherpaParaformerConfig) -> Result<Self> {
        validate_config(&cfg)?;
        // Create recognizer to verify it works
        build_online_recognizer(&cfg)
            .with_context(|| format!("创建 sherpa Paraformer 识别器失败"))?;

        Ok(Self { cfg })
    }

    fn model_display_name(&self) -> String {
        Path::new(&self.cfg.encoder_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("paraformer")
            .to_string()
    }
}

impl AsrEngine for SherpaParaformerEngine {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::SherpaParaformer
    }

    fn runtime_info(&self) -> AsrRuntimeInfo {
        AsrRuntimeInfo {
            backend: self.backend_kind(),
            model: self.model_display_name(),
            threads: Some(self.cfg.num_threads),
            detail: Some(format!(
                "provider={}",
                self.cfg.provider.as_deref().unwrap_or("cpu"),
            )),
        }
    }

    fn start_session(&self, config: AsrSessionConfig) -> Result<Box<dyn AsrSession>> {
        Ok(Box::new(SherpaParaformerSession {
            recognizer: build_online_recognizer(&self.cfg)?,
            sample_rate: 16000, // Paraformer expects 16kHz
            full_audio: Vec::new(),
            partial_window: AudioRingBuffer::with_capacity(config.max_partial_window_samples),
            min_partial_samples: config.min_partial_samples,
            has_pending_audio: false,
        }))
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        let recognizer = build_online_recognizer(&self.cfg)?;
        decode_audio(&recognizer, 16000, audio)
    }
}

impl AsrSession for SherpaParaformerSession {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::SherpaParaformer
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
        if trimmed.is_empty() {
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

fn validate_config(cfg: &SherpaParaformerConfig) -> Result<()> {
    if !Path::new(&cfg.encoder_path).exists() {
        anyhow::bail!("未找到 Paraformer encoder 模型文件: {}", cfg.encoder_path);
    }
    if !Path::new(&cfg.decoder_path).exists() {
        anyhow::bail!("未找到 Paraformer decoder 模型文件: {}", cfg.decoder_path);
    }
    if !Path::new(&cfg.tokens_path).exists() {
        anyhow::bail!("未找到 Paraformer tokens 文件: {}", cfg.tokens_path);
    }
    Ok(())
}

fn build_online_recognizer(cfg: &SherpaParaformerConfig) -> Result<OnlineRecognizer> {
    let paraformer_model = OnlineParaformerModelConfig {
        encoder: Some(cfg.encoder_path.clone()),
        decoder: Some(cfg.decoder_path.clone()),
    };

    let model_config = OnlineModelConfig {
        paraformer: paraformer_model,
        tokens: Some(cfg.tokens_path.clone()),
        num_threads: cfg.num_threads.max(1),
        provider: cfg.provider.clone(),
        ..Default::default()
    };

    let recognizer_cfg = OnlineRecognizerConfig {
        model_config,
        enable_endpoint: false,
        rule1_min_trailing_silence: 2.4,
        rule2_min_trailing_silence: 1.2,
        rule3_min_utterance_length: 300.0,
        ..Default::default()
    };

    OnlineRecognizer::create(&recognizer_cfg)
        .context("sherpa-onnx OnlineRecognizer::create 返回空指针")
}

fn decode_audio(recognizer: &OnlineRecognizer, sample_rate: i32, audio: &[f32]) -> Result<String> {
    if audio.is_empty() {
        return Ok(String::new());
    }

    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, audio);
    stream.input_finished();
    while recognizer.is_ready(&stream) {
        recognizer.decode(&stream);
    }
    let result = recognizer
        .get_result(&stream)
        .context("sherpa-onnx 未返回识别结果")?;
    Ok(result.text)
}
