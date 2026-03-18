//! 配置管理模块

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 热键配置
    pub hotkey: HotkeyConfig,
    /// 音频配置
    pub audio: AudioConfig,
    /// Whisper 配置
    pub whisper: WhisperConfig,
    /// LLM 配置
    pub llm: LLMConfig,
    /// 文本纠错配置
    pub text_correction: TextCorrectionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig::default(),
            audio: AudioConfig::default(),
            whisper: WhisperConfig::default(),
            llm: LLMConfig::default(),
            text_correction: TextCorrectionConfig::default(),
        }
    }
}

/// 热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 按键名称，如 "F12"
    pub key: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            // 使用 ctrl+space，这是最常见的语音输入热键
            key: "ctrl+space".to_string(),
        }
    }
}

/// 音频配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// 采样率
    pub sample_rate: u32,
    /// 声道数
    pub channels: u16,
    /// 是否启用 VAD（语音活动检测）
    pub vad_enabled: bool,
    /// VAD 静音阈值（秒），超过此时间自动结束录音
    pub vad_silence_threshold_ms: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
            vad_enabled: false, // 默认关闭 VAD
            vad_silence_threshold_ms: 1500,
        }
    }
}

/// Whisper 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperConfig {
    /// 模型路径
    pub model_path: String,
    /// 是否翻译
    pub translate: bool,
    /// 语言，null 表示自动检测
    pub language: Option<String>,
    /// 解码策略（beam_search 或 greedy）
    pub decoding_strategy: WhisperDecodingStrategy,
    /// Greedy 模式下的 best_of
    pub greedy_best_of: i32,
    /// BeamSearch 模式下的 beam_size
    pub beam_size: i32,
    /// 解码温度
    pub temperature: f32,
    /// 禁用跨段上下文，避免上一次话语影响本次识别
    pub no_context: bool,
    /// 抑制非语音标记 token
    pub suppress_nst: bool,
    /// 解码线程数
    pub n_threads: i32,
    /// 可选初始提示词（可放业务热词）
    pub initial_prompt: Option<String>,
    /// 热词列表（用于增强特定词汇的识别优先级）
    pub hotwords: Vec<String>,
}

/// Whisper 解码策略
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhisperDecodingStrategy {
    Greedy,
    BeamSearch,
}

impl Default for WhisperDecodingStrategy {
    fn default() -> Self {
        Self::BeamSearch
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model_path: "models/ggml-large-v3.bin".to_string(),
            translate: false,
            language: Some("zh".to_string()),
            decoding_strategy: WhisperDecodingStrategy::BeamSearch,
            greedy_best_of: 5,
            beam_size: 5,
            temperature: 0.0,
            no_context: true,
            suppress_nst: true,
            n_threads: 4,
            initial_prompt: None,
            hotwords: Vec::new(),
        }
    }
}

/// 文本纠错配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TextCorrectionConfig {
    /// 是否启用谐音纠错
    pub enabled: bool,
    /// 谐音词纠错映射（错误词 -> 正确词）
    pub homophone_map: HashMap<String, String>,
}

impl Default for TextCorrectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            homophone_map: HashMap::new(),
        }
    }
}

/// LLM 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LLMConfig {
    /// 是否启用 LLM 整理
    pub enabled: bool,
    /// 提供商
    pub provider: String,
    /// 模型名称
    pub model: String,
    /// API 地址
    pub api_base: String,
    /// API Key 环境变量名
    pub api_key_env: String,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
        }
    }
}

impl Config {
    /// 获取默认配置路径
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("catecho")
            .join("config.toml")
    }

    /// 检查是否是首次运行（配置文件不存在）
    pub fn is_first_run(path: &str) -> bool {
        let path = path.replace(
            "~",
            &dirs::home_dir().unwrap_or_default().display().to_string(),
        );
        !PathBuf::from(path).exists()
    }

    /// 检查 LLM 是否已配置（用于首次运行引导）
    pub fn is_llm_configured(&self) -> bool {
        // 检查是否启用了 LLM
        if !self.llm.enabled {
            return false;
        }

        // 对于 Ollama（api_key_env 为空），不需要检查环境变量
        if self.llm.api_key_env.is_empty() {
            return true;
        }

        // 检查环境变量是否存在
        std::env::var(&self.llm.api_key_env).is_ok()
    }

    /// 加载配置
    pub fn load(path: &str) -> Result<Self> {
        let path = path.replace(
            "~",
            &dirs::home_dir().unwrap_or_default().display().to_string(),
        );
        let path = PathBuf::from(path);

        if path.exists() {
            let content = fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            // 返回默认配置
            Ok(Config::default())
        }
    }

    /// 保存配置
    pub fn save(&self, path: &str) -> Result<()> {
        let path = path.replace(
            "~",
            &dirs::home_dir().unwrap_or_default().display().to_string(),
        );
        let path = PathBuf::from(path);

        // 确保目录存在
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hotkey.key, "ctrl+space");
        assert_eq!(config.audio.sample_rate, 16000);
    }
}
