use crate::asr::types::{AsrBackendKind, AsrEngine, AsrRuntimeInfo};
use crate::stt::{DecodingStrategy, WhisperSTT};
use anyhow::Result;
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

/// 当前 Whisper 路径的适配层。
pub struct WhisperAsrEngine {
    inner: WhisperSTT,
}

impl WhisperAsrEngine {
    pub fn new(inner: WhisperSTT) -> Self {
        Self { inner }
    }

    fn strategy_label(&self) -> &'static str {
        match self.inner.decoding_strategy() {
            DecodingStrategy::Greedy { .. } => "greedy",
            DecodingStrategy::BeamSearch { .. } => "beam_search",
        }
    }

    fn model_display_name(&self) -> String {
        Path::new(self.inner.model_path())
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(self.inner.model_path())
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
            threads: Some(self.inner.n_threads()),
            detail: Some(self.strategy_label().to_string()),
        }
    }

    fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        self.inner.transcribe(audio)
    }

    fn transcribe_abortable(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
    ) -> Result<String> {
        self.inner
            .transcribe_incremental_abortable(audio, abort_flag)
    }

    fn transcribe_with_segment_callback(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
        on_segment: Box<dyn FnMut(String) + Send>,
    ) -> Result<String> {
        self.inner
            .transcribe_with_callback_abortable(audio, abort_flag, on_segment)
    }
}
