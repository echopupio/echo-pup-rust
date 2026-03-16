//! Whisper 语音识别 - 简化实现

use anyhow::Result;

/// Whisper 语音识别
pub struct WhisperSTT {
    model_path: String,
}

impl WhisperSTT {
    /// 创建新的 Whisper 实例
    pub fn new(model_path: &str) -> Result<Self> {
        let path = std::path::Path::new(model_path);
        if !path.exists() {
            tracing::warn!("Whisper 模型文件不存在: {}", model_path);
            tracing::info!("请下载 Whisper 模型到 models/ 目录");
            return Err(anyhow::anyhow!("未找到 Whisper 模型"));
        }

        tracing::info!("Whisper 模型路径已设置: {}", model_path);

        Ok(Self {
            model_path: model_path.to_string(),
        })
    }

    /// 转写音频数据 (简化版本，需要模型文件)
    pub fn transcribe(&self, _audio: &[f32]) -> Result<String> {
        tracing::warn!("Whisper 转写功能需要模型文件");
        Ok("请下载 Whisper 模型".to_string())
    }

    /// 检查模型是否已加载
    pub fn is_ready(&self) -> bool {
        std::path::Path::new(&self.model_path).exists()
    }
}
