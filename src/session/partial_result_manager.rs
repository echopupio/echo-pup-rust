use crate::commit::CommitAction;

/// partial 更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialUpdate {
    pub text: String,
    pub status_text: String,
}

/// 用于避免 partial 重复刷屏的最小状态管理器，同时跟踪已提交草稿字符数。
#[derive(Debug, Default)]
pub struct PartialResultManager {
    latest_text: String,
    committed_char_count: usize,
}

impl PartialResultManager {
    pub fn clear(&mut self) {
        self.latest_text.clear();
        self.committed_char_count = 0;
    }

    pub fn update(&mut self, text: &str) -> Option<PartialUpdate> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() || normalized == self.latest_text {
            return None;
        }

        self.latest_text = normalized.clone();
        Some(PartialUpdate {
            status_text: format_partial_status(&normalized),
            text: normalized,
        })
    }

    /// 构造草稿提交动作：删掉已提交的旧草稿，输入新文本。
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() {
            return None;
        }

        let delete_chars = self.committed_char_count;
        self.committed_char_count = normalized.chars().count();
        Some(CommitAction::UpdateDraft {
            new_text: normalized,
            delete_chars,
        })
    }

    /// 构造清除草稿动作（录音结束时调用）。
    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        if self.committed_char_count == 0 {
            return None;
        }
        let delete_chars = self.committed_char_count;
        self.committed_char_count = 0;
        Some(CommitAction::ClearDraft { delete_chars })
    }
}

fn normalize_partial_text(text: &str) -> String {
    text.replace('\n', " ").trim().to_string()
}

fn format_partial_status(text: &str) -> String {
    const MAX_CHARS: usize = 48;
    let mut chars = text.chars();
    let clipped: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("识别中: {}…", clipped)
    } else {
        format!("识别中: {}", clipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::CommitAction;

    #[test]
    fn update_ignores_blank_text() {
        let mut manager = PartialResultManager::default();
        assert!(manager.update("   ").is_none());
    }

    #[test]
    fn update_deduplicates_same_text() {
        let mut manager = PartialResultManager::default();
        assert!(manager.update("你好").is_some());
        assert!(manager.update("你好").is_none());
    }

    #[test]
    fn update_formats_status() {
        let mut manager = PartialResultManager::default();
        let update = manager.update("第一行\n第二行").unwrap();
        assert_eq!(update.text, "第一行 第二行");
        assert!(update.status_text.starts_with("识别中: "));
    }

    #[test]
    fn draft_commit_first_call_has_zero_delete() {
        let mut manager = PartialResultManager::default();
        let action = manager.prepare_draft_commit("你好").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                assert_eq!(new_text, "你好");
                assert_eq!(delete_chars, 0);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_second_call_deletes_previous() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        let action = manager.prepare_draft_commit("你好世界").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                assert_eq!(new_text, "你好世界");
                assert_eq!(delete_chars, 2); // "你好" = 2 chars
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_clear_returns_none_when_no_draft() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_draft_clear().is_none());
    }

    #[test]
    fn draft_clear_returns_correct_count() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("abc");
        let action = manager.prepare_draft_clear().unwrap();
        match action {
            CommitAction::ClearDraft { delete_chars } => {
                assert_eq!(delete_chars, 3);
            }
            _ => panic!("expected ClearDraft"),
        }
        // 再次清除应返回 None
        assert!(manager.prepare_draft_clear().is_none());
    }
}
