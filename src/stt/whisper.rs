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
    temperature: f32,
    initial_prompt: Option<String>,
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
            temperature: 0.0,  // 确定性输出，提高准确率
            initial_prompt: Some("以下是语音识别任务，请识别以下内容：开始、结束、测试、你好、再见、下一句。请注意使用中文标点符号。".to_string()),
        })
    }

    /// 创建实例并设置语言和翻译选项
    pub fn with_options(model_path: &str, language: Option<String>, translate: bool) -> Result<Self> {
        let mut instance = Self::new(model_path)?;
        instance.language = language;
        instance.translate = translate;
        Ok(instance)
    }

    /// 设置 temperature（0.0 = 确定性输出，更准确）
    pub fn set_temperature(&mut self, temperature: f32) {
        self.temperature = temperature;
    }

    /// 设置初始提示（帮助提高识别准确率，可传入热词列表）
    pub fn set_initial_prompt(&mut self, prompt: Option<String>) {
        self.initial_prompt = prompt;
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

        // 设置温度 (0.0 = 确定性输出，提高准确率)
        params.set_temperature(self.temperature);

        // 设置初始提示（帮助识别）
        if let Some(ref prompt) = self.initial_prompt {
            params.set_initial_prompt(prompt);
        }

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

        // 修复标点符号
        let result = Self::fix_punctuation(&result);

        Ok(result)
    }

    /// 修复标点符号（中文场景下 Whisper 往往不带标点）
    /// 在句尾添加适当的标点（句号、问号等）
    fn fix_punctuation(text: &str) -> String {
        if text.is_empty() {
            return text.to_string();
        }

        let mut result = String::with_capacity(text.len() + 16);
        let mut chars = text.chars().peekable();

        // 中文标点符号列表
        let chinese_punctuation = ['。', '？', '！', '，', '；', '：', '"', '"', '\''];

        // 句尾需要添加标点的情况
        let end_marks = ['。', '？', '！'];

        while let Some(c) = chars.next() {
            result.push(c);

            // 检查当前字符是否为句尾字符（字母、数字或中文）
            if c.is_alphanumeric() || '\u{4E00}' <= c && c <= '\u{9FFF}' {
                // 查看下一个字符
                match chars.peek() {
                    Some(&next) => {
                        // 如果下一个字符是换行或结束，且当前字符不是标点，则添加句号
                        if (next == '\n' || next.is_whitespace()) && !chinese_punctuation.contains(&c) {
                            // 检查当前字符是否已经是标点
                            let needs_punct = !end_marks.iter().any(|&m| {
                                result.ends_with(m)
                            });
                            if needs_punct && !c.is_whitespace() {
                                result.push('。');
                            }
                        }
                    }
                    None => {
                        // 文本结束，检查是否需要添加句号
                        if !chinese_punctuation.contains(&c) {
                            result.push('。');
                        }
                    }
                }
            }
        }

        // 清理多余的空格和换行
        let result = result
            .replace("  ", " ")
            .replace("\n ", "\n")
            .replace(" \n", "\n");

        // 连续多个句号只保留一个
        let result = result.replace("。。", "。");

        result
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
