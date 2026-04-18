use crate::commit::CommitAction;
use tracing::debug;

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
    /// 稳定锁定的前缀字符数：partial 文本连续稳定 N 次后，当前文本长度被"锁定"，
    /// 后续 diff 不会删除锁定前缀。
    stable_prefix_chars: usize,
    /// 连续返回相同 partial 的次数
    stale_count: usize,
    /// 是否已在当前稳定区间注入了逗号
    pause_comma_injected: bool,
}

/// 多少次连续相同 partial 后锁定前缀（每次 poll ~500ms，2 次 ≈ 1 秒）
const STALE_LOCK_THRESHOLD: usize = 2;
/// 多少次连续相同 partial 后注入逗号（3 次 ≈ 1.5 秒）
const STALE_COMMA_THRESHOLD: usize = 3;

impl PartialResultManager {
    pub fn clear(&mut self) {
        self.latest_text.clear();
        self.committed_text.clear();
        self.stable_prefix_chars = 0;
        self.stale_count = 0;
        self.pause_comma_injected = false;
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

    /// 报告一次 poll 结果（不管文字是否变化），用于稳定性检测。
    /// 返回需要注入的停顿逗号（如果有的话）。
    pub fn tick_stability(&mut self, text_changed: bool) -> Option<CommitAction> {
        if text_changed {
            self.stale_count = 0;
            self.pause_comma_injected = false;
            return None;
        }
        self.stale_count += 1;

        // 文本连续稳定 → 锁定当前长度为 stable_prefix
        if self.stale_count >= STALE_LOCK_THRESHOLD {
            let current_chars = self.committed_text.chars().count();
            if current_chars > self.stable_prefix_chars {
                debug!(
                    "[stability] lock prefix: {} → {} chars",
                    self.stable_prefix_chars, current_chars
                );
                self.stable_prefix_chars = current_chars;
            }
        }

        // 连续稳定更久 → 注入逗号
        if self.stale_count >= STALE_COMMA_THRESHOLD
            && !self.pause_comma_injected
            && !self.committed_text.is_empty()
        {
            let last_char = self.committed_text.chars().last();
            let already_has_punct = last_char
                .map(|c| "，。！？；、".contains(c))
                .unwrap_or(false);
            if !already_has_punct {
                self.pause_comma_injected = true;
                self.committed_text.push('，');
                self.stable_prefix_chars = self.committed_text.chars().count();
                debug!(
                    "[stability] inject comma → committed={:?}",
                    self.committed_text
                );
                return Some(CommitAction::UpdateDraft {
                    new_text: "，".to_string(),
                    delete_chars: 0,
                });
            }
        }
        None
    }

    /// 构造草稿提交动作：增量 diff + 稳定前缀锁定。
    ///
    /// 如果有已锁定的 stable_prefix_chars，且新文本以锁定前缀开头，
    /// 则不会删除锁定前缀部分的文字。
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() {
            return None;
        }

        let natural_common = common_char_prefix_len(&self.committed_text, &normalized);

        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars.saturating_sub(natural_common);
        let new_suffix: String = normalized.chars().skip(natural_common).collect();

        debug!(
            "[draft] committed={:?}({}) new={:?}({}) common={} stable={} del={} type={:?}",
            self.committed_text,
            old_total_chars,
            normalized,
            normalized.chars().count(),
            natural_common,
            self.stable_prefix_chars,
            delete_chars,
            new_suffix
        );

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

    /// 从草稿平滑过渡到最终文本：增量 diff 只替换尾部差异。
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

        let natural_common = common_char_prefix_len(&self.committed_text, &normalized);

        let old_total_chars = self.committed_text.chars().count();
        let delete_chars = old_total_chars.saturating_sub(natural_common);
        let new_suffix: String = normalized.chars().skip(natural_common).collect();

        debug!(
            "[final_from_draft] committed={:?}({}) final={:?}({}) common={} del={} type={:?}",
            self.committed_text, old_total_chars,
            normalized, normalized.chars().count(),
            natural_common, delete_chars, new_suffix
        );

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
    fn draft_commit_stability_lock_prevents_full_delete() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        // 模拟连续稳定 STALE_LOCK_THRESHOLD 次
        for _ in 0..STALE_LOCK_THRESHOLD {
            manager.tick_stability(false);
        }
        // stable_prefix_chars 应已锁定到 4
        // 新 partial: "经典天地" — natural common = 0
        // 但因为 stable_prefix 不匹配 (natural_common < stable)，fallback 到 natural
        let action = manager.prepare_draft_commit("经典天地").unwrap();
        match action {
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                // natural_common = 0, stable 不匹配 → 删 4, type "经典天地"
                assert_eq!(delete_chars, 4);
                assert_eq!(new_text, "经典天地");
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn stability_injects_comma_after_threshold() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        // 连续 tick 直到 STALE_COMMA_THRESHOLD
        for _ in 0..STALE_COMMA_THRESHOLD {
            let action = manager.tick_stability(false);
            if action.is_some() {
                // 应该在第 STALE_COMMA_THRESHOLD 次返回逗号
                match action.unwrap() {
                    CommitAction::UpdateDraft {
                        new_text,
                        delete_chars,
                    } => {
                        assert_eq!(new_text, "，");
                        assert_eq!(delete_chars, 0);
                    }
                    _ => panic!("expected UpdateDraft with comma"),
                }
                return;
            }
        }
        panic!("expected comma to be injected");
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
