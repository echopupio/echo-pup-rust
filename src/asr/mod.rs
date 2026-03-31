//! 语音识别运行时抽象

pub mod types;
pub mod whisper;

pub use types::AsrEngine;
pub use whisper::WhisperAsrEngine;
