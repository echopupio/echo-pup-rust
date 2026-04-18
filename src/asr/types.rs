use anyhow::Result;
use std::sync::{atomic::AtomicBool, Arc};

/// 识别后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrBackendKind {
    SherpaParaformer,
}

impl AsrBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::SherpaParaformer => "sherpa_paraformer",
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

/// 流式识别会话配置。
///
/// 当前阶段先为 Whisper 预览线程提供统一入口，
/// 后续 sherpa-onnx / SenseVoice 将直接复用这套 session 边界。
#[derive(Debug, Clone, Copy)]
pub struct AsrSessionConfig {
    pub min_partial_samples: usize,
}

/// 单次录音生命周期内的流式识别会话。
pub trait AsrSession {
    fn backend_kind(&self) -> AsrBackendKind;

    fn accept_audio(&mut self, audio: &[f32]) -> Result<()>;

    fn poll_partial(&mut self, abort_flag: Arc<AtomicBool>) -> Result<Option<String>>;

    fn finalize(&mut self, abort_flag: Arc<AtomicBool>) -> Result<String>;

    fn buffered_samples(&self) -> usize;
}

/// 统一的识别运行时接口。
///
/// 第一阶段先把当前 Whisper 路径收口到该 trait 上，
/// 后续再继续演进为常驻引擎 + session 形态。
pub trait AsrEngine: Send {
    fn backend_kind(&self) -> AsrBackendKind;

    fn runtime_info(&self) -> AsrRuntimeInfo;

    fn start_session(&self, _config: AsrSessionConfig) -> Result<Box<dyn AsrSession>> {
        Err(anyhow::anyhow!(
            "backend {} does not support streaming sessions yet",
            self.backend_kind().label()
        ))
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String>;
}
