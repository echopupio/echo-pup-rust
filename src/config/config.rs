//! 配置管理

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub whisper: WhisperConfig,
    pub llm: LLMConfig,
    pub input: InputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    pub model_path: String,
    pub translate: bool,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub api_base: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub typing_delay: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig {
                key: "F12".to_string(),
            },
            audio: AudioConfig {
                sample_rate: 16000,
                channels: 1,
            },
            whisper: WhisperConfig {
                model_path: "models/ggml-small.bin".to_string(),
                translate: false,
                language: "zh".to_string(),
            },
            llm: LLMConfig {
                enabled: true,
                provider: "openai".to_string(),
                model: "gpt-4o-mini".to_string(),
                api_base: "https://api.openai.com/v1".to_string(),
                api_key_env: "OPENAI_API_KEY".to_string(),
            },
            input: InputConfig { typing_delay: 5 },
        }
    }
}

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let path = Self::expand_path(path);
        
        if !path.exists() {
            let config = Self::default();
            config.save(path.as_path_str())?;
            return Ok(config);
        }

        let content = fs::read_to_string(&path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self, path: &str) -> anyhow::Result<()> {
        let path = Self::expand_path(path);
        
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;
        Ok(())
    }

    fn expand_path(path: &str) -> PathBuf {
        if path.starts_with('~') {
            let home = dirs::home_dir().unwrap_or_default();
            home.join(path.trim_start_matches('~').trim_start_matches('/'))
        } else {
            PathBuf::from(path)
        }
    }
}
