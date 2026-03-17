//! 配置管理模块

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// 全局配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 热键配置
    pub hotkey: HotkeyConfig,
    /// 音频配置
    pub audio: AudioConfig,
    /// Whisper 配置
    pub whisper: WhisperConfig,
    /// LLM 配置
    pub llm: LLMConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig::default(),
            audio: AudioConfig::default(),
            whisper: WhisperConfig::default(),
            llm: LLMConfig::default(),
        }
    }
}

/// 热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// 按键名称，如 "F12"
    pub key: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            key: "CTRL+Space".to_string(),
        }
    }
}

/// 音频配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// 采样率
    pub sample_rate: u32,
    /// 声道数
    pub channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
        }
    }
}

/// Whisper 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    /// 模型路径
    pub model_path: String,
    /// 是否翻译
    pub translate: bool,
    /// 语言，null 表示自动检测
    pub language: Option<String>,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model_path: "models/ggml-small.bin".to_string(),
            translate: false,
            language: Some("zh".to_string()),
        }
    }
}

/// LLM 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            .join("typechoai")
            .join("config.toml")
    }

    /// 加载配置
    pub fn load(path: &str) -> Result<Self> {
        let path = path.replace("~", &dirs::home_dir().unwrap_or_default().display().to_string());
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
        let path = path.replace("~", &dirs::home_dir().unwrap_or_default().display().to_string());
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
        assert_eq!(config.hotkey.key, "CTRL+Space");
        assert_eq!(config.audio.sample_rate, 16000);
    }
}
