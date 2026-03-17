//! Whisper 语音识别实现 (whisper-rs 0.16+)

use anyhow::{Context, Result};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState};

/// Whisper 语音识别
pub struct WhisperSTT {
    context: WhisperContext,
    state: Option<WhisperState>,
    model_path: String,
    language: Option<String>,
    translate: bool,
}

impl WhisperSTT {
    /// 创建新的 Whisper 实例
    pub fn new(model_path: &str) -> Result<Self> {
        let path = std::path::Path::new(model_path);
        if !path.exists() {
            return Err(anyhow::anyhow!("未找到 Whisper 模型: {}", model_path));
        }

        // 使用新版本 API 创建 context
        let context = WhisperContext::new_with_params(
            model_path,
            WhisperContextParameters::default(),
        ).map_err(|e| anyhow::anyhow!("模型加载失败: {:?}", e))?;

        // 预先创建 state
        let state = context.create_state()
            .map_err(|e| anyhow::anyhow!("创建 state 失败: {:?}", e))?;

        Ok(Self {
            context,
            state: Some(state),
            model_path: model_path.to_string(),
            language: Some("zh".to_string()),
            translate: false,
        })
    }

    /// 创建实例并设置语言和翻译选项
    pub fn with_options(model_path: &str, language: Option<String>, translate: bool) -> Result<Self> {
        let mut instance = Self::new(model_path)?;
        instance.language = language;
        instance.translate = translate;
        Ok(instance)
    }

    /// 转写音频数据
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        let state = self.state.as_mut()
            .context("Whisper state 未创建")?;

        if audio.is_empty() {
            return Ok(String::new());
        }

        // 创建转写参数
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(4);
        params.set_print_progress(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);

        // 设置语言
        if let Some(ref lang) = self.language {
            if lang != "auto" {
                // 设置目标语言
                params.set_language(Some(lang.as_str()));
            } else {
                // 自动检测语言
                params.set_detect_language(true);
            }
        }

        // 设置翻译
        params.set_translate(self.translate);

        // 执行转写 - 新版本 API
        state.full(params, audio)
            .map_err(|e| anyhow::anyhow!("Whisper 转写失败: {:?}", e))?;

        // 获取结果
        let num_segments = state.full_n_segments();
        let mut result = String::new();

        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(text) = segment.to_str() {
                    result.push_str(text);
                }
            }
        }

        Ok(result)
    }

    /// 检查模型是否已加载
    pub fn is_ready(&self) -> bool {
        self.state.is_some()
    }

    /// 获取模型路径
    pub fn model_path(&self) -> &str {
        &self.model_path
    }

    /// 设置语言
    pub fn set_language(&mut self, language: Option<String>) {
        self.language = language;
    }

    /// 设置是否翻译
    pub fn set_translate(&mut self, translate: bool) {
        self.translate = translate;
    }
}
