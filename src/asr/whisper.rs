use crate::asr::types::{AsrBackendKind, AsrEngine, AsrRuntimeInfo, AsrSession, AsrSessionConfig};
use crate::audio::buffer::AudioRingBuffer;
use crate::stt::{DecodingStrategy, WhisperSTT};
use anyhow::Result;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

/// 当前 Whisper 路径的适配层。
pub struct WhisperAsrEngine {
    inner: Arc<Mutex<WhisperSTT>>,
}

struct WhisperAsrSession {
    inner: Arc<Mutex<WhisperSTT>>,
    full_audio: Vec<f32>,
    partial_window: AudioRingBuffer,
    min_partial_samples: usize,
    has_pending_audio: bool,
}

impl WhisperAsrEngine {
    pub fn new(inner: WhisperSTT) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    fn strategy_label(&self) -> &'static str {
        match self.inner.lock().decoding_strategy() {
            DecodingStrategy::Greedy { .. } => "greedy",
            DecodingStrategy::BeamSearch { .. } => "beam_search",
        }
    }

    fn model_display_name(&self) -> String {
        let inner = self.inner.lock();
        Path::new(inner.model_path())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(inner.model_path())
            .to_string()
    }
}

impl AsrEngine for WhisperAsrEngine {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::Whisper
    }

    fn runtime_info(&self) -> AsrRuntimeInfo {
        AsrRuntimeInfo {
            backend: self.backend_kind(),
            model: self.model_display_name(),
            threads: Some(self.inner.lock().n_threads()),
            detail: Some(self.strategy_label().to_string()),
        }
    }

    fn start_session(&self, config: AsrSessionConfig) -> Result<Box<dyn AsrSession>> {
        Ok(Box::new(WhisperAsrSession {
            inner: self.inner.clone(),
            full_audio: Vec::new(),
            partial_window: AudioRingBuffer::with_capacity(config.max_partial_window_samples),
            min_partial_samples: config.min_partial_samples,
            has_pending_audio: false,
        }))
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        self.inner.lock().transcribe(audio)
    }
}

impl AsrSession for WhisperAsrSession {
    fn backend_kind(&self) -> AsrBackendKind {
        AsrBackendKind::Whisper
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
        if !self.has_pending_audio || self.partial_window.len() < self.min_partial_samples {
            return Ok(None);
        }

        let preview_audio = self.partial_window.snapshot();
        self.has_pending_audio = false;

        let text = self
            .inner
            .lock()
            .transcribe_incremental_abortable(&preview_audio, abort_flag)?;
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed == "[BLANK_AUDIO]" {
            return Ok(None);
        }

        Ok(Some(trimmed.to_string()))
    }

    fn finalize(&mut self, abort_flag: Arc<AtomicBool>) -> Result<String> {
        if self.full_audio.is_empty() {
            return Ok(String::new());
        }

        self.inner
            .lock()
            .transcribe_incremental_abortable(&self.full_audio, abort_flag)
    }

    fn buffered_samples(&self) -> usize {
        self.full_audio.len()
    }
}
