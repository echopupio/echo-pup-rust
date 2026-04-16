use crate::commit::CommitAction;

/// final 提交动作管理器。
#[derive(Debug, Default)]
pub struct FinalResultManager {
    latest_final_text: String,
}

impl FinalResultManager {
    pub fn clear(&mut self) {
        self.latest_final_text.clear();
    }

    pub fn prepare_commit(&mut self, text: &str) -> Option<CommitAction> {
        let normalized = text.trim().to_string();
        if normalized.is_empty() || normalized == self.latest_final_text {
            return None;
        }

        self.latest_final_text = normalized.clone();
        Some(CommitAction::CommitFinal { text: normalized })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_commit_ignores_blank_text() {
        let mut manager = FinalResultManager::default();
        assert!(manager.prepare_commit("   ").is_none());
    }

    #[test]
    fn prepare_commit_deduplicates_latest_final() {
        let mut manager = FinalResultManager::default();
        assert!(manager.prepare_commit("你好").is_some());
        assert!(manager.prepare_commit("你好").is_none());
    }
}
