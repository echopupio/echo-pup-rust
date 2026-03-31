use anyhow::Result;
use std::sync::{atomic::AtomicBool, Arc};

/// 识别后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrBackendKind {
    Whisper,
}

impl AsrBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Whisper => "whisper",
        }
    }
}

/// 运行时信息
#[derive(Debug, Clone)]
pub struct AsrRuntimeInfo {
    pub backend: AsrBackendKind,
    pub model: String,
    pub threads: Option<i32>,
    pub detail: Option<String>,
}

/// 统一的识别运行时接口。
///
/// 第一阶段先把当前 Whisper 路径收口到该 trait 上，
/// 后续再继续演进为常驻引擎 + session 形态。
pub trait AsrEngine: Send {
    fn backend_kind(&self) -> AsrBackendKind;

    fn runtime_info(&self) -> AsrRuntimeInfo;

    fn is_ready(&self) -> bool;

    fn transcribe(&mut self, audio: &[f32]) -> Result<String>;

    fn transcribe_abortable(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
    ) -> Result<String> {
        let _ = abort_flag;
        self.transcribe(audio)
    }

    fn transcribe_with_segment_callback(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
        on_segment: Box<dyn FnMut(String) + Send>,
    ) -> Result<String> {
        let _ = on_segment;
        self.transcribe_abortable(audio, abort_flag)
    }
}
