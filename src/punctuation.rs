//! 离线标点恢复模块
//!
//! 使用 sherpa-onnx 的 OfflinePunctuation（ct_transformer）为无标点文本添加标点。

use anyhow::{anyhow, Result};
use sherpa_onnx::{OfflinePunctuation, OfflinePunctuationConfig, OfflinePunctuationModelConfig};
use std::path::Path;
use std::time::Instant;
use tracing::{info, warn};

use crate::config::PunctuationConfig;

pub struct PunctuationRestorer {
    inner: OfflinePunctuation,
}

impl PunctuationRestorer {
    pub fn new(config: &PunctuationConfig) -> Result<Option<Self>> {
        if !config.enabled {
            info!("离线标点恢复已禁用");
            return Ok(None);
        }

        let model_path = &config.model_path;
        if !Path::new(model_path).exists() {
            warn!(
                "标点模型不存在: {}，跳过标点恢复。请运行 scripts/download_punctuation_model.sh 下载模型",
                model_path
            );
            return Ok(None);
        }

        let punct_config = OfflinePunctuationConfig {
            model: OfflinePunctuationModelConfig {
                ct_transformer: Some(model_path.to_string()),
                num_threads: 2,
                debug: false,
                provider: Some("cpu".to_string()),
            },
        };

        let start = Instant::now();
        let punct = OfflinePunctuation::create(&punct_config)
            .ok_or_else(|| anyhow!("创建标点恢复引擎失败，请检查模型文件: {}", model_path))?;

        info!(
            "离线标点恢复引擎初始化完成，耗时 {}ms",
            start.elapsed().as_millis()
        );

        Ok(Some(Self { inner: punct }))
    }

    /// 为无标点文本添加标点
    pub fn add_punctuation(&self, text: &str) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }

        let start = Instant::now();
        match self.inner.add_punctuation(text) {
            Some(result) => {
                info!(
                    "标点恢复完成: {}ms, {}字 -> {}字",
                    start.elapsed().as_millis(),
                    text.chars().count(),
                    result.chars().count()
                );
                result
            }
            None => {
                warn!("标点恢复返回空结果，使用原文");
                text.to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_returns_none() {
        let config = PunctuationConfig {
            enabled: false,
            model_path: String::new(),
        };
        let restorer = PunctuationRestorer::new(&config).unwrap();
        assert!(restorer.is_none());
    }

    #[test]
    fn test_missing_model_returns_none() {
        let config = PunctuationConfig {
            enabled: true,
            model_path: "/nonexistent/model.onnx".to_string(),
        };
        let restorer = PunctuationRestorer::new(&config).unwrap();
        assert!(restorer.is_none());
    }
}
