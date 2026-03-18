//! 识别后文本纠错（热词保持、谐音替换）

use crate::config::TextCorrectionConfig;

/// 转写后处理器
#[derive(Debug, Clone)]
pub struct TextPostProcessor {
    enabled: bool,
    replacement_rules: Vec<(String, String)>,
}

impl TextPostProcessor {
    /// 根据配置构建后处理器
    pub fn new(config: &TextCorrectionConfig) -> Self {
        // 长词优先替换，避免“北京大学”先被“北京”截断
        let mut replacement_rules: Vec<(String, String)> = config
            .homophone_map
            .iter()
            .filter(|(from, to)| !from.trim().is_empty() && !to.trim().is_empty())
            .map(|(from, to)| (from.trim().to_string(), to.trim().to_string()))
            .collect();
        replacement_rules.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));

        Self {
            enabled: config.enabled,
            replacement_rules,
        }
    }

    /// 应用谐音纠错规则
    pub fn process(&self, text: &str) -> String {
        if !self.enabled || text.is_empty() || self.replacement_rules.is_empty() {
            return text.to_string();
        }

        let mut result = text.to_string();
        for (from, to) in &self.replacement_rules {
            if from != to && result.contains(from) {
                result = result.replace(from, to);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_homophone_replace() {
        let mut map = HashMap::new();
        map.insert("公做".to_string(), "工作".to_string());
        map.insert("行好".to_string(), "型号".to_string());

        let cfg = TextCorrectionConfig {
            enabled: true,
            homophone_map: map,
        };
        let processor = TextPostProcessor::new(&cfg);
        let output = processor.process("这个公做的行好不错");
        assert_eq!(output, "这个工作的型号不错");
    }

    #[test]
    fn test_longer_rule_first() {
        let mut map = HashMap::new();
        map.insert("北京大学".to_string(), "北大".to_string());
        map.insert("北京".to_string(), "京城".to_string());

        let cfg = TextCorrectionConfig {
            enabled: true,
            homophone_map: map,
        };
        let processor = TextPostProcessor::new(&cfg);
        let output = processor.process("我在北京大学读书");
        assert_eq!(output, "我在北大读书");
    }
}
