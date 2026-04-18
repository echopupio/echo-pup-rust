use crate::commit::CommitAction;
use tracing::debug;

/// partial 更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialUpdate {
    pub text: String,
    pub status_text: String,
}

/// 滑动窗口文本融合管理器。
///
/// ASR 的 partial 结果来自一个固定长度的 ring buffer（约 3 秒），
/// 每次 poll 只识别最近 3 秒音频。随着录音持续，旧音频被挤出 buffer，
/// 对应的文字必须被"确认"（不再删除）。
///
/// 数据模型：
/// - `confirmed_text`: 已确认的文字，绝不会被删除
/// - `draft_text`:     当前 ring buffer 对应的可变草稿
/// - 屏幕显示 = confirmed_text + draft_text
///
/// 每次 poll 得到 window_partial（ring buffer 识别结果）后：
/// 1. 找 draft_text 的后缀与 window_partial 的前缀的最长重叠
/// 2. draft_text 中重叠左边的部分 → 确认到 confirmed_text
/// 3. window_partial 中重叠之后的部分 → 追加
/// 4. 若无重叠，用 common_prefix diff 作为 fallback
#[derive(Debug, Default)]
pub struct PartialResultManager {
    latest_text: String,
    /// 已确认的文字（绝不删除）
    confirmed_text: String,
    /// 当前可变草稿
    draft_text: String,
    /// 连续返回相同 partial 的次数
    stale_count: usize,
    /// 是否已在当前稳定区间注入了逗号
    pause_comma_injected: bool,
}

/// 多少次连续相同 partial 后注入逗号（3 次 × 500ms ≈ 1.5 秒）
const STALE_COMMA_THRESHOLD: usize = 3;

impl PartialResultManager {
    pub fn clear(&mut self) {
        self.latest_text.clear();
        self.confirmed_text.clear();
        self.draft_text.clear();
        self.stale_count = 0;
        self.pause_comma_injected = false;
    }

    pub fn update(&mut self, text: &str) -> Option<PartialUpdate> {
        let normalized = normalize_partial_text(text);
        if normalized.is_empty() || normalized == self.latest_text {
            return None;
        }

        self.latest_text = normalized.clone();
        let full_display = format!("{}{}", self.confirmed_text, &normalized);
        Some(PartialUpdate {
            status_text: format_partial_status(&full_display),
            text: normalized,
        })
    }

    /// 报告一次 poll 周期，用于停顿检测和逗号注入。
    pub fn tick_stability(&mut self, text_changed: bool) -> Option<CommitAction> {
        if text_changed {
            self.stale_count = 0;
            self.pause_comma_injected = false;
            return None;
        }
        self.stale_count += 1;

        if self.stale_count >= STALE_COMMA_THRESHOLD
            && !self.pause_comma_injected
            && !self.draft_text.is_empty()
        {
            let last_char = self.draft_text.chars().last();
            let already_has_punct = last_char
                .map(|c| "，。！？；、".contains(c))
                .unwrap_or(false);
            if !already_has_punct {
                self.pause_comma_injected = true;
                let draft_was_len = self.draft_text.chars().count();
                // 确认当前所有 draft + 追加逗号
                self.confirmed_text.push_str(&self.draft_text);
                self.confirmed_text.push('，');
                self.draft_text.clear();
                debug!(
                    "[stability] inject comma → confirmed={:?}",
                    self.confirmed_text
                );
                return Some(CommitAction::UpdateDraft {
                    new_text: "，".to_string(),
                    delete_chars: 0,
                    confirm_chars: draft_was_len,
                });
            }
        }
        None
    }

    /// 构造草稿提交动作：滑动窗口文本融合。
    ///
    /// `window_partial` 是 ring buffer 当前识别结果（只覆盖最近约 3 秒音频）。
    /// 通过 overlap/prefix 匹配将其与已有 draft 融合。
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let window_partial = normalize_partial_text(text);
        if window_partial.is_empty() {
            return None;
        }

        // 首次提交：直接输出
        if self.draft_text.is_empty() && self.confirmed_text.is_empty() {
            debug!("[draft] first commit: {:?}", window_partial);
            self.draft_text = window_partial.clone();
            return Some(CommitAction::UpdateDraft {
                new_text: window_partial,
                delete_chars: 0,
                confirm_chars: 0,
            });
        }

        let draft_chars: Vec<char> = self.draft_text.chars().collect();
        let wp_chars: Vec<char> = window_partial.chars().collect();

        // 策略 1: 找 draft 后缀与 window_partial 前缀的最长重叠
        let overlap = suffix_prefix_overlap(&draft_chars, &wp_chars);

        if overlap > 0 {
            // draft 中重叠左边的部分 → 确认
            let confirm_count = draft_chars.len() - overlap;
            let to_confirm: String = draft_chars[..confirm_count].iter().collect();
            self.confirmed_text.push_str(&to_confirm);

            // window_partial 中重叠之后的部分 → 追加
            let append: String = wp_chars[overlap..].iter().collect();
            // 新 draft = window_partial 全部
            self.draft_text = window_partial.clone();

            debug!(
                "[draft] overlap={} confirm={:?} append={:?} → confirmed={:?} draft={:?}",
                overlap, to_confirm, append, self.confirmed_text, self.draft_text
            );

            if append.is_empty() && confirm_count == 0 {
                return None;
            }

            return Some(CommitAction::UpdateDraft {
                new_text: append,
                delete_chars: 0,
                confirm_chars: confirm_count,
            });
        }

        // 策略 2: 无 overlap → 用 common_prefix diff 只修改 draft 尾部
        let common = common_char_prefix_len(&draft_chars, &wp_chars);
        let delete_chars = draft_chars.len().saturating_sub(common);
        let new_suffix: String = wp_chars[common..].iter().collect();

        debug!(
            "[draft] no-overlap, prefix_common={} del={} type={:?} → draft_was={:?} draft_new={:?}",
            common, delete_chars, new_suffix, self.draft_text, window_partial
        );

        self.draft_text = window_partial;

        if delete_chars == 0 && new_suffix.is_empty() {
            return None;
        }

        Some(CommitAction::UpdateDraft {
            new_text: new_suffix,
            delete_chars,
            confirm_chars: 0,
        })
    }

    /// 构造清除草稿动作（仅删 draft_text，保留 confirmed_text 在屏幕上）。
    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        let draft_chars = self.draft_text.chars().count();
        if draft_chars == 0 {
            return None;
        }
        self.draft_text.clear();
        Some(CommitAction::ClearDraft {
            delete_chars: draft_chars,
        })
    }

    /// 从草稿过渡到最终文本。
    ///
    /// final_text 是全部音频的完整识别结果。
    /// 屏幕上当前显示 confirmed_text + draft_text。
    /// 计算 diff 后只修改尾部差异。
    pub fn prepare_final_from_draft(&mut self, final_text: &str) -> Option<CommitAction> {
        let full_committed = format!("{}{}", self.confirmed_text, self.draft_text);
        if full_committed.is_empty() {
            return None;
        }

        let normalized = final_text.replace('\n', " ").trim().to_string();
        if normalized.is_empty() {
            // 清除所有已输出的文字
            let total = full_committed.chars().count();
            self.confirmed_text.clear();
            self.draft_text.clear();
            return Some(CommitAction::ClearDraft {
                delete_chars: total,
            });
        }

        let full_chars: Vec<char> = full_committed.chars().collect();
        let final_chars: Vec<char> = normalized.chars().collect();
        let common = common_char_prefix_len(&full_chars, &final_chars);

        // 只能删 draft 部分，confirmed 不能删
        let confirmed_len = self.confirmed_text.chars().count();
        let effective_common = common.max(confirmed_len);
        let delete_chars = full_chars.len().saturating_sub(effective_common);
        let new_suffix: String = final_chars[effective_common..].iter().collect();

        debug!(
            "[final_from_draft] screen={:?}({}) final={:?}({}) common={} conf_len={} eff_common={} del={} type={:?}",
            full_committed, full_chars.len(),
            normalized, final_chars.len(),
            common, confirmed_len, effective_common, delete_chars, new_suffix
        );

        self.confirmed_text.clear();
        self.draft_text.clear();

        Some(CommitAction::CommitFinalFromDraft {
            new_text: new_suffix,
            delete_chars,
        })
    }
}

fn normalize_partial_text(text: &str) -> String {
    text.replace('\n', " ").trim().to_string()
}

/// 计算两个 char 切片的公共前缀长度。
fn common_char_prefix_len(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

/// 找 `a` 的后缀与 `b` 的前缀的最长重叠长度。
///
/// 例: a = ['很','好','我','们'], b = ['我','们','去','公','园']
/// → overlap = 2 ('我','们')
fn suffix_prefix_overlap(a: &[char], b: &[char]) -> usize {
    let max_overlap = a.len().min(b.len());
    for overlap_len in (1..=max_overlap).rev() {
        if a[a.len() - overlap_len..] == b[..overlap_len] {
            return overlap_len;
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
    fn draft_commit_first_call_types_all() {
        let mut manager = PartialResultManager::default();
        let action = manager.prepare_draft_commit("你好").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(new_text, "你好");
                assert_eq!(delete_chars, 0);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_appends_via_prefix() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        let action = manager.prepare_draft_commit("你好世界").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(new_text, "世界");
                assert_eq!(delete_chars, 0);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_replaces_tail() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界吗");
        let action = manager.prepare_draft_commit("你好世界啊").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(new_text, "啊");
                assert_eq!(delete_chars, 1);
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_sliding_window_overlap() {
        // 模拟 ring buffer 滑动：旧 partial 和新 partial 有重叠
        let mut manager = PartialResultManager::default();
        // 第一个窗口
        manager.prepare_draft_commit("今天天气很好");
        // 窗口滑动：新 partial 的前缀和旧 draft 的后缀重叠
        let action = manager.prepare_draft_commit("天气很好我们").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                // overlap("今天天气很好", "天气很好我们") = 4 ("天气很好")
                // confirm "今天" (2 chars), append "我们"
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "我们");
            }
            _ => panic!("expected UpdateDraft"),
        }
        // confirmed 应为 "今天"
        assert_eq!(manager.confirmed_text, "今天");
        assert_eq!(manager.draft_text, "天气很好我们");
    }

    #[test]
    fn draft_commit_multi_slide_keeps_confirmed() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        // 滑动 1
        manager.prepare_draft_commit("天气很好我们");
        assert_eq!(manager.confirmed_text, "今天");
        // 滑动 2
        manager.prepare_draft_commit("好我们去公园");
        assert_eq!(manager.confirmed_text, "今天天气很");
        assert_eq!(manager.draft_text, "好我们去公园");
    }

    #[test]
    fn draft_commit_no_overlap_uses_prefix_diff() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        // 完全不同的文本，无 overlap 也无 common prefix
        let action = manager.prepare_draft_commit("经典天地").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, 4);
                assert_eq!(new_text, "经典天地");
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_commit_identical_returns_none() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        assert!(manager.prepare_draft_commit("你好").is_none());
    }

    #[test]
    fn stability_injects_comma() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        for _ in 0..STALE_COMMA_THRESHOLD {
            let action = manager.tick_stability(false);
            if let Some(action) = action {
                match action {
                    CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                        assert_eq!(new_text, "，");
                        assert_eq!(delete_chars, 0);
                    }
                    _ => panic!("expected comma UpdateDraft"),
                }
                // 逗号注入后，draft 应被确认
                assert_eq!(manager.confirmed_text, "你好世界，");
                assert!(manager.draft_text.is_empty());
                return;
            }
        }
        panic!("comma not injected");
    }

    #[test]
    fn draft_clear_only_deletes_draft() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        manager.prepare_draft_commit("天气很好我们");
        // confirmed="今天", draft="天气很好我们"
        let action = manager.prepare_draft_clear().unwrap();
        match action {
            CommitAction::ClearDraft { delete_chars } => {
                assert_eq!(delete_chars, 6); // draft "天气很好我们" = 6 chars
            }
            _ => panic!("expected ClearDraft"),
        }
        // confirmed 保留
        assert_eq!(manager.confirmed_text, "今天");
    }

    #[test]
    fn draft_clear_returns_none_when_no_draft() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_draft_clear().is_none());
    }

    #[test]
    fn final_from_draft_smooth_transition() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界吗");
        let action = manager.prepare_final_from_draft("你好世界啊！").unwrap();
        match action {
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                // screen="你好世界吗"(5), final="你好世界啊！"(6), common=4
                // del=5-4=1, type="啊！"
                assert_eq!(delete_chars, 1);
                assert_eq!(new_text, "啊！");
            }
            _ => panic!("expected CommitFinalFromDraft"),
        }
    }

    #[test]
    fn final_from_draft_preserves_confirmed() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        manager.prepare_draft_commit("天气很好我们");
        // confirmed="今天", draft="天气很好我们", screen="今天天气很好我们"
        let action = manager.prepare_final_from_draft("今天天气很好我们去公园").unwrap();
        match action {
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                // screen="今天天气很好我们"(8), final="今天天气很好我们去公园"(11)
                // common=8, del=0, type="去公园"
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "去公园");
            }
            _ => panic!("expected CommitFinalFromDraft"),
        }
    }

    #[test]
    fn final_from_draft_returns_none_when_empty() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_final_from_draft("any text").is_none());
    }

    #[test]
    fn suffix_prefix_overlap_basic() {
        let a: Vec<char> = "今天天气很好".chars().collect();
        let b: Vec<char> = "天气很好我们".chars().collect();
        assert_eq!(suffix_prefix_overlap(&a, &b), 4);
    }

    #[test]
    fn suffix_prefix_overlap_no_match() {
        let a: Vec<char> = "你好".chars().collect();
        let b: Vec<char> = "世界".chars().collect();
        assert_eq!(suffix_prefix_overlap(&a, &b), 0);
    }

    #[test]
    fn suffix_prefix_overlap_full() {
        let a: Vec<char> = "你好".chars().collect();
        let b: Vec<char> = "你好世界".chars().collect();
        assert_eq!(suffix_prefix_overlap(&a, &b), 2);
    }
}
