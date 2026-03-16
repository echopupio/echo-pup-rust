//! Whisper 语音识别

use anyhow::{Context, Result};
use std::path::Path;
use whisper_rs::{FullParams, WhisperContext};

/// Whisper 语音识别
pub struct WhisperSTT {
    context: WhisperContext,
    model_path: String,
}

impl WhisperSTT {
    /// 创建新的 Whisper 实例
    pub fn new(model_path: &str) -> Result<Self> {
        let path = Path::new(model_path);
        if !path.exists() {
            tracing::warn!("Whisper 模型文件不存在: {}", model_path);
            tracing::info!("请下载 Whisper 模型到 models/ 目录");
            // 尝试从系统路径加载
            let default_paths = [
                "/usr/share/whisper/models/ggml-small.bin",
                "/usr/local/share/whisper/models/ggml-small.bin",
            ];
            for p in &default_paths {
                if Path::new(p).exists() {
                    let context = WhisperContext::new(p)
                        .context("无法加载 Whisper 模型")?;
                    tracing::info!("Whisper 模型已从系统路径加载: {}", p);
                    return Ok(Self {
                        context,
                        model_path: p.to_string(),
                    });
                }
            }
            return Err(anyhow::anyhow!("未找到 Whisper 模型"));
        }

        let context = WhisperContext::new(model_path)
            .context("无法加载 Whisper 模型")?;

        tracing::info!("Whisper 模型已加载: {}", model_path);

        Ok(Self {
            context,
            model_path: model_path.to_string(),
        })
    }

    /// 转写音频数据
    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        if audio.is_empty() {
            return Ok(String::new());
        }

        let mut params = FullParams::new();
        params.set_language(self.get_language());
        params.set_translate(false);
        params.set_n_threads(4);
        // 禁用打印
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self.context.create_state()
            .context("无法创建 Whisper 状态")?;

        state
            .full(params, audio)
            .context("转写失败")?;

        let num_segments = state
            .full_n_segments()
            .context("获取分段数失败")?;

        let mut result = String::new();
        for i in 0..num_segments {
            if let Ok(text) = state.full_get_segment_text(i) {
                result.push_str(&text);
                result.push(' ');
            }
        }

        let result = result.trim().to_string();
        tracing::info!("转写完成，文本长度: {} 字符", result.len());
        Ok(result)
    }

    /// 获取语言设置
    fn get_language(&self) -> Option<&str> {
        if self.model_path.contains("zh") {
            Some("zh")
        } else {
            Some("zh")
        }
    }

    /// 检查模型是否已加载
    pub fn is_ready(&self) -> bool {
        true
    }
}
