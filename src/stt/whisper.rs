//! Whisper 语音识别

use anyhow::Result;

pub struct WhisperSTT {
    // TODO: 添加 whisper-rs 相关字段
}

impl WhisperSTT {
    pub fn new(model_path: &str) -> Result<Self> {
        todo!()
    }

    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        todo!()
    }
}
