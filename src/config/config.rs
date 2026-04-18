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
    /// ASR 后端选择与运行时配置
    pub asr: AsrConfig,
    /// LLM 配置
    pub llm: LLMConfig,
    /// 文本纠错配置
    pub text_correction: TextCorrectionConfig,
    /// 反馈配置（通知/声音）
    pub feedback: FeedbackConfig,
    /// 文本提交配置
    pub commit: CommitConfig,
    /// 离线标点恢复配置
    pub punctuation: PunctuationConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeyConfig::default(),
            audio: AudioConfig::default(),
            asr: AsrConfig::default(),
            llm: LLMConfig::default(),
            text_correction: TextCorrectionConfig::default(),
            feedback: FeedbackConfig::default(),
            commit: CommitConfig::default(),
            punctuation: PunctuationConfig::default(),
        }
    }
}

/// 热键配置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyTriggerMode {
    /// 长按达到阈值后开始录音，松开结束
    HoldToRecord,
    /// 长按达到阈值后开始录音，再次按下结束
    PressToToggle,
}

impl Default for HotkeyTriggerMode {
    fn default() -> Self {
        Self::PressToToggle
    }
}

impl HotkeyTriggerMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::HoldToRecord => "长按模式",
            Self::PressToToggle => "按压切换模式",
        }
    }
}

/// 热键配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// 触发模式
    pub trigger_mode: HotkeyTriggerMode,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            trigger_mode: HotkeyTriggerMode::default(),
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
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: 16000,
            channels: 1,
        }
    }
}

/// ASR 后端类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    SherpaParaformer,
}

impl Default for AsrBackend {
    fn default() -> Self {
        Self::SherpaParaformer
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    /// 当前启用的本地 ASR 后端
    pub backend: AsrBackend,
    /// sherpa-onnx + Paraformer 配置
    pub sherpa_paraformer: SherpaParaformerConfig,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            backend: AsrBackend::SherpaParaformer,
            sherpa_paraformer: SherpaParaformerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SherpaParaformerConfig {
    /// encoder.onnx 模型文件路径
    pub encoder_path: String,
    /// decoder.onnx 模型文件路径
    pub decoder_path: String,
    /// tokens 文件路径
    pub tokens_path: String,
    /// 推理 provider，默认 cpu
    pub provider: Option<String>,
    /// 推理线程数
    pub num_threads: i32,
}

impl Default for SherpaParaformerConfig {
    fn default() -> Self {
        Self {
            encoder_path: default_paraformer_model_path("encoder.onnx"),
            decoder_path: default_paraformer_model_path("decoder.onnx"),
            tokens_path: default_paraformer_model_path("tokens.txt"),
            provider: Some("cpu".to_string()),
            num_threads: default_asr_num_threads(),
        }
    }
}

fn default_paraformer_model_path(file_name: &str) -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".echopup")
        .join("models")
        .join("asr")
        .join("sherpa-onnx-streaming-paraformer-bilingual-zh-en")
        .join(file_name)
        .to_string_lossy()
        .into_owned()
}

fn default_asr_num_threads() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .clamp(1, 8)
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

/// 反馈配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackConfig {
    /// 是否启用状态栏反馈（当前实现：macOS 菜单栏）
    pub status_bar_enabled: bool,
    /// 是否启用录音开始/结束提示音
    pub sound_enabled: bool,
    /// 启动时是否提示 macOS 通知设置
    pub notify_tip_on_start: bool,
}

impl Default for FeedbackConfig {
    fn default() -> Self {
        Self {
            status_bar_enabled: true,
            sound_enabled: true,
            notify_tip_on_start: true,
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
    /// API Key
    #[serde(alias = "api_key_env")]
    pub api_key: String,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "openai".to_string(),
            model: "gpt-4o-mini".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
        }
    }
}

/// 文本提交配置（流式草稿现在始终启用，保留空结构体以兼容已有配置文件）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitConfig {}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {}
    }
}

/// 离线标点恢复配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PunctuationConfig {
    /// 是否启用离线标点恢复
    pub enabled: bool,
    /// ct_transformer 模型路径
    pub model_path: String,
}

impl Default for PunctuationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_path: default_punctuation_model_path(),
        }
    }
}

fn default_punctuation_model_path() -> String {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".echopup")
        .join("models")
        .join("punctuation")
        .join("model.onnx")
        .to_string_lossy()
        .into_owned()
}

impl Config {
    /// 获取默认配置路径
    #[allow(dead_code)]
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("echopup")
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

        // Ollama 不需要 API 密钥
        if self.llm.provider == "ollama" {
            return true;
        }

        // 其他 provider 必须填写 api_key
        !self.llm.api_key.is_empty()
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
    use std::path::Path;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hotkey.trigger_mode, HotkeyTriggerMode::PressToToggle);
        assert_eq!(config.audio.sample_rate, 16000);
        assert_eq!(config.asr.backend, AsrBackend::SherpaParaformer);
        assert!(config.feedback.status_bar_enabled);
        assert!(config.feedback.sound_enabled);
        assert!(config.feedback.notify_tip_on_start);
        assert!(
            Path::new(&config.asr.sherpa_paraformer.encoder_path).ends_with(
                ".echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/encoder.onnx"
            )
        );
        assert!(
            Path::new(&config.asr.sherpa_paraformer.decoder_path).ends_with(
                ".echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/decoder.onnx"
            )
        );
        assert!(
            Path::new(&config.asr.sherpa_paraformer.tokens_path).ends_with(
                ".echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/tokens.txt"
            )
        );
    }

    #[test]
    fn test_default_asr_num_threads() {
        let num_threads = default_asr_num_threads();
        assert!(num_threads >= 1);
        assert!(num_threads <= 8);
    }
}
