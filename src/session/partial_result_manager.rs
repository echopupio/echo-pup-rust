use crate::commit::CommitAction;

/// partial 更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialUpdate {
    pub text: String,
    pub status_text: String,
}

/// 用于避免 partial 重复刷屏的最小状态管理器，同时跟踪已提交草稿文本。
#[derive(Debug, Default)]
pub struct PartialResultManager {
    latest_text: String,
    /// 已提交到光标处的草稿文本（用于计算增量 diff）
    committed_text: String,
}

impl PartialResultManager {
    pub fn clear(&mut self) {
        self.latest_text.clear();
        self.committed_text.clear();
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

    /// 构造草稿提交动作：只删除与新文本不同的尾部，再输入新的尾部。
    ///
    /// 例如：已提交 "你好世界" → 新 partial "你好世界啊"
    ///   → 公共前缀 "你好世界"(4字符)，delete_chars=0，new_text="啊"
    ///
    /// 已提交 "你好世界吗" → 新 partial "你好世界啊"
    ///   → 公共前缀 "你好世界"(4字符)，delete_chars=1("吗")，new_text="啊"
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() {
            return None;
        }

        let common_prefix_chars = common_char_prefix_len(&self.committed_text, &normalized);
        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars - common_prefix_chars;

        // 取新文本中公共前缀之后的部分
        let new_suffix: String = normalized.chars().skip(common_prefix_chars).collect();

        self.committed_text = normalized;

        // 如果没有要删也没有要输入的，跳过
        if delete_chars == 0 && new_suffix.is_empty() {
            return None;
        }

        Some(CommitAction::UpdateDraft {
            new_text: new_suffix,
            delete_chars,
        })
    }

    /// 构造清除草稿动作（录音结束时调用）。
    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        let char_count = self.committed_text.chars().count();
        if char_count == 0 {
            return None;
        }
        self.committed_text.clear();
        Some(CommitAction::ClearDraft {
            delete_chars: char_count,
        })
    }

    /// 从草稿平滑过渡到最终文本：只替换尾部差异。
    ///
    /// 如果没有活跃草稿，返回 None（调用方应 fallback 到 CommitFinal）。
    pub fn prepare_final_from_draft(&mut self, final_text: &str) -> Option<CommitAction> {
        if self.committed_text.is_empty() {
            return None;
        }

        let normalized = final_text.replace('\n', " ").trim().to_string();
        if normalized.is_empty() {
            // final 为空时，清除草稿
            return self.prepare_draft_clear();
        }

        let common_prefix_chars = common_char_prefix_len(&self.committed_text, &normalized);
        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars - common_prefix_chars;
        let new_suffix: String = normalized.chars().skip(common_prefix_chars).collect();

        self.committed_text.clear();

        Some(CommitAction::CommitFinalFromDraft {
            new_text: new_suffix,
            delete_chars,
        })
    }
}

fn normalize_partial_text(text: &str) -> String {
    text.replace('\n', " ").trim().to_string()
}

/// 计算两个字符串的公共前缀字符数（按 char 粒度）。
fn common_char_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
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
    fn draft_commit_appends_when_prefix_matches() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        let action = manager.prepare_draft_commit("你好世界").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                // 公共前缀 "你好"(2 chars)，旧文本 2 chars → delete 0，输入 "世界"
                assert_eq!(new_text, "世界");
                assert_eq!(delete_chars, 0);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_replaces_divergent_tail() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界吗");
        let action = manager.prepare_draft_commit("你好世界啊").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                // 公共前缀 "你好世界"(4 chars)，旧 5 chars → delete 1("吗")，输入 "啊"
                assert_eq!(new_text, "啊");
                assert_eq!(delete_chars, 1);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_identical_text_returns_none() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        // 同样的文本 → 无需操作
        assert!(manager.prepare_draft_commit("你好").is_none());
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

    #[test]
    fn final_from_draft_smooth_transition() {
        let mut manager = PartialResultManager::default();
        // 模拟草稿: "你好世界吗"
        manager.prepare_draft_commit("你好世界吗");
        // final 文本: "你好世界啊！"
        let action = manager.prepare_final_from_draft("你好世界啊！").unwrap();
        match action {
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                // 公共前缀 "你好世界"(4 chars)，旧 5 chars → delete 1("吗")，输入 "啊！"
                assert_eq!(delete_chars, 1);
                assert_eq!(new_text, "啊！");
            }
            _ => panic!("expected CommitFinalFromDraft"),
        }
        // committed_text 应已清空
        assert!(manager.prepare_draft_clear().is_none());
    }

    #[test]
    fn final_from_draft_returns_none_when_no_draft() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_final_from_draft("any text").is_none());
    }
}
