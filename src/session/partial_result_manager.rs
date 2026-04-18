use crate::commit::CommitAction;
use tracing::debug;

/// partial 更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialUpdate {
    pub text: String,
    pub status_text: String,
}

/// 纯追加模式的 partial 结果管理器。
///
/// ASR 的 partial 结果来自一个固定长度的 ring buffer（约 3 秒）。
/// 管理器通过 overlap 检测，每次只追加新增部分到屏幕，**从不退格删除**。
/// 录音结束时由 final 结果做一次修正。
///
/// 数据模型：
/// - `last_window_text`: 上一次 poll 的 window 识别结果
/// - `total_appended`:   所有已追加到屏幕的文字总和
#[derive(Debug, Default)]
pub struct PartialResultManager {
    latest_text: String,
    /// 上一次 poll 的 window partial 结果
    last_window_text: String,
    /// 已追加到屏幕的全部文字
    total_appended: String,
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
        self.last_window_text.clear();
        self.total_appended.clear();
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
            && !self.total_appended.is_empty()
        {
            let last_char = self.total_appended.chars().last();
            let already_has_punct = last_char
                .map(|c| "，。！？；、".contains(c))
                .unwrap_or(false);
            if !already_has_punct {
                self.pause_comma_injected = true;
                self.total_appended.push('，');
                debug!(
                    "[stability] inject comma → total={:?}",
                    self.total_appended
                );
                return Some(CommitAction::UpdateDraft {
                    new_text: "，".to_string(),
                    delete_chars: 0,
                    confirm_chars: 0,
                });
            }
        }
        None
    }

    /// 构造草稿提交动作：纯追加模式，永不退格。
    ///
    /// 通过 overlap 或 prefix 匹配找出新增部分，只追加不删除。
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let window = normalize_partial_text(text);
        if window.is_empty() {
            return None;
        }

        // 首次提交：直接输出
        if self.last_window_text.is_empty() {
            debug!("[draft] first window: {:?}", window);
            self.last_window_text = window.clone();
            self.total_appended = window.clone();
            return Some(CommitAction::UpdateDraft {
                new_text: window,
                delete_chars: 0,
                confirm_chars: 0,
            });
        }

        // 同一窗口内文本变化：用 common prefix 只追加新增部分
        let last_chars: Vec<char> = self.last_window_text.chars().collect();
        let win_chars: Vec<char> = window.chars().collect();

        // 先尝试 overlap（窗口滑动场景）
        let overlap = suffix_prefix_overlap(&last_chars, &win_chars);
        let append = if overlap > 0 {
            // 窗口滑动：旧窗口的后缀和新窗口的前缀重叠
            let new_part: String = win_chars[overlap..].iter().collect();
            debug!(
                "[draft] overlap={} append={:?} last={:?} win={:?}",
                overlap, new_part, self.last_window_text, window
            );
            new_part
        } else {
            // 同窗口内文本变化：用 common prefix
            let common = common_char_prefix_len(&last_chars, &win_chars);
            if common >= last_chars.len() {
                // 新文本是旧文本的超集 → 只追加多出的部分
                let new_part: String = win_chars[common..].iter().collect();
                debug!(
                    "[draft] extend prefix_common={} append={:?}",
                    common, new_part
                );
                new_part
            } else {
                // 旧文本尾部被修改 — 但纯追加模式不删除
                // 忽略已输出的部分，只追加新窗口中超出旧文本的内容
                if win_chars.len() > last_chars.len() {
                    let new_part: String =
                        win_chars[last_chars.len()..].iter().collect();
                    debug!(
                        "[draft] tail-changed, append excess={:?} last_len={} win_len={}",
                        new_part,
                        last_chars.len(),
                        win_chars.len()
                    );
                    new_part
                } else {
                    // 新窗口更短或等长但内容不同 → 跳过
                    debug!(
                        "[draft] skip: win shorter/same-len but different last={:?} win={:?}",
                        self.last_window_text, window
                    );
                    self.last_window_text = window;
                    return None;
                }
            }
        };

        self.last_window_text = window;

        if append.is_empty() {
            return None;
        }

        self.total_appended.push_str(&append);

        Some(CommitAction::UpdateDraft {
            new_text: append,
            delete_chars: 0,
            confirm_chars: 0,
        })
    }

    /// 纯追加模式下不清除已输出文字，返回 None。
    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        None
    }

    /// 从已追加文字过渡到最终文本。
    ///
    /// 找 total_appended 和 final_text 的公共前缀，
    /// 删掉 total_appended 尾部的错误字符，追加 final 的剩余部分。
    pub fn prepare_final_from_draft(&mut self, final_text: &str) -> Option<CommitAction> {
        if self.total_appended.is_empty() {
            return None;
        }

        let normalized = final_text.replace('\n', " ").trim().to_string();
        if normalized.is_empty() {
            // final 为空但已经输出了文字 — 需要全部退格
            let total = self.total_appended.chars().count();
            self.total_appended.clear();
            self.last_window_text.clear();
            return Some(CommitAction::CommitFinalFromDraft {
                new_text: String::new(),
                delete_chars: total,
            });
        }

        let appended_chars: Vec<char> = self.total_appended.chars().collect();
        let final_chars: Vec<char> = normalized.chars().collect();
        let common = common_char_prefix_len(&appended_chars, &final_chars);

        let delete_chars = appended_chars.len().saturating_sub(common);
        let new_suffix: String = final_chars[common..].iter().collect();

        debug!(
            "[final_from_draft] appended={:?}({}) final={:?}({}) common={} del={} type={:?}",
            self.total_appended,
            appended_chars.len(),
            normalized,
            final_chars.len(),
            common,
            delete_chars,
            new_suffix
        );

        self.total_appended.clear();
        self.last_window_text.clear();

        if delete_chars == 0 && new_suffix.is_empty() {
            return None;
        }

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
    fn draft_first_commit_types_all() {
        let mut manager = PartialResultManager::default();
        let action = manager.prepare_draft_commit("你好").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(new_text, "你好");
                assert_eq!(delete_chars, 0);
            }
            _ => panic!("expected UpdateDraft"),
        }
        assert_eq!(manager.total_appended, "你好");
    }

    #[test]
    fn draft_appends_when_text_extends() {
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
        assert_eq!(manager.total_appended, "你好世界");
    }

    #[test]
    fn draft_sliding_window_overlap() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        let action = manager.prepare_draft_commit("天气很好我们").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "我们");
            }
            _ => panic!("expected UpdateDraft"),
        }
        assert_eq!(manager.total_appended, "今天天气很好我们");
    }

    #[test]
    fn draft_multi_slide() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        manager.prepare_draft_commit("天气很好我们");
        assert_eq!(manager.total_appended, "今天天气很好我们");
        manager.prepare_draft_commit("好我们去公园");
        assert_eq!(manager.total_appended, "今天天气很好我们去公园");
    }

    #[test]
    fn draft_tail_changed_appends_excess() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        // 尾部改了但长度增加 → 追加超出部分
        let action = manager.prepare_draft_commit("你好天地啊").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "啊"); // 只追加第5个字
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_identical_returns_none() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        assert!(manager.prepare_draft_commit("你好").is_none());
    }

    #[test]
    fn draft_shorter_window_skips() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界");
        // 新窗口更短且无overlap → 跳过
        assert!(manager.prepare_draft_commit("经典").is_none());
    }

    #[test]
    fn draft_clear_returns_none() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        assert!(manager.prepare_draft_clear().is_none());
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
                assert_eq!(manager.total_appended, "你好世界，");
                return;
            }
        }
        panic!("comma not injected");
    }

    #[test]
    fn final_from_draft_appends_only() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好");
        manager.prepare_draft_commit("天气很好我们");
        // total_appended = "今天天气很好我们"
        let action = manager
            .prepare_final_from_draft("今天天气很好我们去公园")
            .unwrap();
        match action {
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text, "去公园");
            }
            _ => panic!("expected CommitFinalFromDraft"),
        }
    }

    #[test]
    fn final_from_draft_fixes_tail() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气好嘛");
        // total = "今天天气好嘛", final = "今天天气很好"
        let action = manager
            .prepare_final_from_draft("今天天气很好")
            .unwrap();
        match action {
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                // common = 4 ("今天天气"), del = 6-4 = 2 ("好嘛"), type = "很好"
                assert_eq!(delete_chars, 2);
                assert_eq!(new_text, "很好");
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
