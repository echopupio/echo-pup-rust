//! 音频录制模块

pub mod buffer;
pub mod recorder;

pub use recorder::{AudioChunkCursor, AudioRecorder};
