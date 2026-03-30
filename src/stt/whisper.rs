//! Whisper 语音识别实现 (whisper-rs 0.16+)
#![allow(dead_code)]

use anyhow::{Context, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

/// 解码策略
#[derive(Debug, Clone, Copy)]
pub enum DecodingStrategy {
    /// 贪心解码，best_of 越高越稳
    Greedy { best_of: i32 },
    /// Beam Search，精度通常更好但更慢
    BeamSearch { beam_size: i32 },
}

/// Whisper 语音识别
pub struct WhisperSTT {
    context: WhisperContext,
    state: Option<WhisperState>,
    model_path: String,
    language: Option<String>,
    translate: bool,
    temperature: f32,
    decoding_strategy: DecodingStrategy,
    no_context: bool,
    suppress_nst: bool,
    n_threads: i32,
    initial_prompt: Option<String>,
    hotwords: Vec<String>,
}

impl WhisperSTT {
    /// 创建新的 Whisper 实例
    pub fn new(model_path: &str) -> Result<Self> {
        let path = std::path::Path::new(model_path);
        if !path.exists() {
            return Err(anyhow::anyhow!("未找到 Whisper 模型: {}", model_path));
        }

        // 使用新版本 API 创建 context
        let context =
            WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
                .map_err(|e| anyhow::anyhow!("模型加载失败: {:?}", e))?;

        // 预先创建 state
        let state = context
            .create_state()
            .map_err(|e| anyhow::anyhow!("创建 state 失败: {:?}", e))?;

        Ok(Self {
            context,
            state: Some(state),
            model_path: model_path.to_string(),
            language: Some("zh".to_string()),
            translate: false,
            temperature: 0.0, // 确定性输出，提高准确率
            decoding_strategy: DecodingStrategy::BeamSearch { beam_size: 5 },
            no_context: true,
            suppress_nst: true,
            n_threads: 4,
            initial_prompt: None, // 不使用 initial_prompt，避免干扰识别
            hotwords: Vec::new(),
        })
    }

    /// 创建实例并设置语言和翻译选项
    pub fn with_options(
        model_path: &str,
        language: Option<String>,
        translate: bool,
    ) -> Result<Self> {
        let mut instance = Self::new(model_path)?;
        instance.language = language;
        instance.translate = translate;
        Ok(instance)
    }

    /// 设置 temperature（0.0 = 确定性输出，更准确）
    pub fn set_temperature(&mut self, temperature: f32) {
        self.temperature = temperature;
    }

    /// 设置解码策略
    pub fn set_decoding_strategy(&mut self, strategy: DecodingStrategy) {
        self.decoding_strategy = strategy;
    }

    /// 设置是否禁用跨段上下文
    pub fn set_no_context(&mut self, no_context: bool) {
        self.no_context = no_context;
    }

    /// 设置是否抑制非语音 token
    pub fn set_suppress_nst(&mut self, suppress_nst: bool) {
        self.suppress_nst = suppress_nst;
    }

    /// 设置线程数
    pub fn set_n_threads(&mut self, n_threads: i32) {
        self.n_threads = n_threads.max(1);
    }

    /// 设置初始提示（帮助提高识别准确率，可传入热词列表）
    pub fn set_initial_prompt(&mut self, prompt: Option<String>) {
        self.initial_prompt = prompt;
    }

    /// 设置热词词典
    pub fn set_hotwords(&mut self, hotwords: Vec<String>) {
        self.hotwords = hotwords;
    }

    /// 组合最终 initial prompt（用户提示词 + 热词词典）
    fn build_initial_prompt(&self) -> Option<String> {
        let base_prompt = self
            .initial_prompt
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let hotwords: Vec<String> = self
            .hotwords
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        let hotword_prompt = if hotwords.is_empty() {
            None
        } else {
            Some(format!(
                "以下是高优先级热词，请尽量按原词输出：{}",
                hotwords.join("、")
            ))
        };

        match (base_prompt, hotword_prompt) {
            (Some(base), Some(hot)) => Some(format!("{base}\n{hot}")),
            (Some(base), None) => Some(base),
            (None, Some(hot)) => Some(hot),
            (None, None) => None,
        }
    }

    fn configure_full_params(
        &self,
        params: &mut FullParams<'_, '_>,
        abort_flag: Option<Arc<AtomicBool>>,
    ) {
        params.set_n_threads(self.n_threads.max(1));
        params.set_print_progress(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_no_timestamps(true);
        params.set_no_context(self.no_context);
        params.set_suppress_nst(self.suppress_nst);
        params.set_suppress_blank(true);
        params.set_temperature(self.temperature);

        if let Some(abort_flag) = abort_flag {
            let abort_callback =
                Box::new(move || abort_flag.load(Ordering::Relaxed)) as Box<dyn FnMut() -> bool>;
            params.set_abort_callback_safe::<_, Box<dyn FnMut() -> bool>>(Some(abort_callback));
        }
    }

    fn transcribe_inner(
        &mut self,
        audio: &[f32],
        abort_flag: Option<Arc<AtomicBool>>,
    ) -> Result<String> {
        if audio.is_empty() {
            return Ok(String::new());
        }

        let initial_prompt = self.build_initial_prompt();

        let strategy = match self.decoding_strategy {
            DecodingStrategy::Greedy { best_of } => SamplingStrategy::Greedy {
                best_of: best_of.max(1),
            },
            DecodingStrategy::BeamSearch { beam_size } => SamplingStrategy::BeamSearch {
                beam_size: beam_size.max(1),
                patience: -1.0,
            },
        };
        let mut params = FullParams::new(strategy);
        self.configure_full_params(&mut params, abort_flag);
        if let Some(prompt) = initial_prompt.as_deref() {
            params.set_initial_prompt(prompt);
        }
        if let Some(ref lang) = self.language {
            if lang != "auto" {
                params.set_language(Some(lang.as_str()));
            } else {
                params.set_detect_language(true);
            }
        }
        params.set_translate(self.translate);

        let state = self.state.as_mut().context("Whisper state 未创建")?;
        state
            .full(params, audio)
            .map_err(|e| anyhow::anyhow!("Whisper 转写失败: {:?}", e))?;

        let num_segments = state.full_n_segments();
        let mut result = String::new();

        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(text) = segment.to_str() {
                    result.push_str(text);
                }
            }
        }

        Ok(Self::fix_punctuation(&result))
    }

    /// 转写音频数据
    pub fn transcribe(&mut self, audio: &[f32]) -> Result<String> {
        self.transcribe_inner(audio, None)
    }

    /// 用于实时预览场景的可复用转写入口
    pub fn transcribe_incremental(&mut self, audio: &[f32]) -> Result<String> {
        self.transcribe_inner(audio, None)
    }

    /// 用于实时预览场景的可中断转写入口
    pub fn transcribe_incremental_abortable(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
    ) -> Result<String> {
        self.transcribe_inner(audio, Some(abort_flag))
    }

    fn transcribe_with_callback_inner<C>(
        &mut self,
        audio: &[f32],
        abort_flag: Option<Arc<AtomicBool>>,
        on_segment: C,
    ) -> Result<String>
    where
        C: FnMut(String) + Send + 'static,
    {
        if audio.is_empty() {
            return Ok(String::new());
        }

        let initial_prompt = self.build_initial_prompt();

        let strategy = match self.decoding_strategy {
            DecodingStrategy::Greedy { best_of } => SamplingStrategy::Greedy {
                best_of: best_of.max(1),
            },
            DecodingStrategy::BeamSearch { beam_size } => SamplingStrategy::BeamSearch {
                beam_size: beam_size.max(1),
                patience: -1.0,
            },
        };
        let mut params = FullParams::new(strategy);
        self.configure_full_params(&mut params, abort_flag);
        if let Some(prompt) = initial_prompt.as_deref() {
            params.set_initial_prompt(prompt);
        }
        if let Some(ref lang) = self.language {
            if lang != "auto" {
                params.set_language(Some(lang.as_str()));
            } else {
                params.set_detect_language(true);
            }
        }
        params.set_translate(self.translate);
        params.set_single_segment(true);

        let callback = std::sync::Arc::new(std::sync::Mutex::new(on_segment));
        let callback_clone = callback.clone();
        let segment_callback: Box<dyn FnMut(whisper_rs::SegmentCallbackData)> =
            Box::new(move |segment: whisper_rs::SegmentCallbackData| {
                let text = segment.text.trim();
                if text.is_empty() || text == "[BLANK_AUDIO]" {
                    return;
                }
                if let Ok(mut cb) = callback_clone.lock() {
                    cb(text.to_string());
                }
            });
        params.set_segment_callback_safe::<
            Option<Box<dyn FnMut(whisper_rs::SegmentCallbackData)>>,
            Box<dyn FnMut(whisper_rs::SegmentCallbackData)>,
        >(Some(segment_callback));

        let state = self.state.as_mut().context("Whisper state 未创建")?;
        state
            .full(params, audio)
            .map_err(|e| anyhow::anyhow!("Whisper 回调转写失败: {:?}", e))?;

        let num_segments = state.full_n_segments();
        let mut result = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(text) = segment.to_str() {
                    result.push_str(text);
                }
            }
        }

        Ok(Self::fix_punctuation(&result))
    }

    /// 回调模式转写：开启 single_segment，并在每个新分段时触发回调。
    pub fn transcribe_with_callback<C>(&mut self, audio: &[f32], on_segment: C) -> Result<String>
    where
        C: FnMut(String) + Send + 'static,
    {
        self.transcribe_with_callback_inner(audio, None, on_segment)
    }

    /// 回调模式转写：支持通过 stop flag 中断长时间运行的预览转写。
    pub fn transcribe_with_callback_abortable<C>(
        &mut self,
        audio: &[f32],
        abort_flag: Arc<AtomicBool>,
        on_segment: C,
    ) -> Result<String>
    where
        C: FnMut(String) + Send + 'static,
    {
        self.transcribe_with_callback_inner(audio, Some(abort_flag), on_segment)
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
                        if (next == '\n' || next.is_whitespace())
                            && !chinese_punctuation.contains(&c)
                        {
                            // 检查当前字符是否已经是标点
                            let needs_punct = !end_marks.iter().any(|&m| result.ends_with(m));
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
