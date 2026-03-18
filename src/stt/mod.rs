//! Whisper 语音识别模块

pub mod postprocess;
pub mod whisper;

pub use postprocess::TextPostProcessor;
pub use whisper::{DecodingStrategy, WhisperSTT};
