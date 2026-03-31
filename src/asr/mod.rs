//! 语音识别运行时抽象

pub mod sherpa_onnx;
pub mod types;
pub mod whisper;

pub use types::{AsrEngine, AsrSession, AsrSessionConfig};
pub use whisper::WhisperAsrEngine;
