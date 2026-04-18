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

    /// 构造草稿提交动作：利用标点锚定 + 增量 diff 最小化退格。
    ///
    /// 规则：已提交文字中最后一个中文标点（，。！？）之前（含标点）的部分视为"锚定文本"，
    /// 不会被退格删除。只有标点之后的"活跃尾部"参与 diff。
    ///
    /// 例如：committed = "你好，世界吧" → 锚定到 "你好，"，活跃尾部 "世界吧"
    ///       new_text  = "你好，世界啊" → 活跃尾部 "世界啊"
    ///       → delete 1("吧"), type "啊"
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() {
            return None;
        }

        // 找到 committed_text 中最后一个中文标点的锚定位置（char 粒度）
        let anchor_chars = last_chinese_punct_boundary_chars(&self.committed_text);
        // 如果新文本也以同一锚定前缀开头，只 diff 锚定之后的活跃部分
        let natural_common = common_char_prefix_len(&self.committed_text, &normalized);
        let safe_prefix = natural_common.max(anchor_chars);

        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars.saturating_sub(safe_prefix);
        let new_suffix: String = normalized.chars().skip(safe_prefix).collect();

        self.committed_text = normalized;

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

    /// 从草稿平滑过渡到最终文本：利用标点锚定只替换尾部差异。
    ///
    /// 如果没有活跃草稿，返回 None（调用方应 fallback 到 CommitFinal）。
    pub fn prepare_final_from_draft(&mut self, final_text: &str) -> Option<CommitAction> {
        if self.committed_text.is_empty() {
            return None;
        }

        let normalized = final_text.replace('\n', " ").trim().to_string();
        if normalized.is_empty() {
            return self.prepare_draft_clear();
        }

        let anchor_chars = last_chinese_punct_boundary_chars(&self.committed_text);
        let natural_common = common_char_prefix_len(&self.committed_text, &normalized);
        let safe_prefix = natural_common.max(anchor_chars);

        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars.saturating_sub(safe_prefix);
        let new_suffix: String = normalized.chars().skip(safe_prefix).collect();

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

/// 返回文本中最后一个中文标点（，。！？；）之后的 char 位置。
/// 如果没有找到标点，返回 0（无锚定）。
///
/// 例如 "你好，世界吧" → 3 (逗号在 index 2，boundary = 3)
fn last_chinese_punct_boundary_chars(text: &str) -> usize {
    const PUNCT: &[char] = &['，', '。', '！', '？', '；', '、'];
    let chars: Vec<char> = text.chars().collect();
    for i in (0..chars.len()).rev() {
        if PUNCT.contains(&chars[i]) {
            return i + 1;
        }
    }
    0
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
                assert_eq!(new_text, "啊");
                assert_eq!(delete_chars, 1);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_punct_anchors_prevents_full_delete() {
        let mut manager = PartialResultManager::default();
        // committed: "你好，世界吧" — 逗号锚定在 char index 3
        manager.prepare_draft_commit("你好，世界吧");
        // 新 partial: "经典，世界啊" — natural common prefix = 0 (你 vs 经)
        // 但锚定到 char 3 (逗号之后)，所以只删 "世界吧"(3 chars)
        let action = manager.prepare_draft_commit("经典，世界啊").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                // 锚定 "你好，" → safe_prefix=3，旧 6 chars → delete 3
                // 新文本跳 3 chars: "世界啊"
                assert_eq!(delete_chars, 3);
                assert_eq!(new_text, "世界啊");
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_no_punct_falls_back_to_natural_diff() {
        let mut manager = PartialResultManager::default();
        // 无标点：完全靠 natural common prefix
        manager.prepare_draft_commit("今天天气");
        let action = manager.prepare_draft_commit("今天天气很好").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "很好");
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
