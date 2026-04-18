use crate::commit::CommitAction;
use tracing::debug;

/// partial 更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialUpdate {
    pub text: String,
    pub status_text: String,
}

/// Spinner + 尾部预览模式的 partial 结果管理器。
///
/// 录音过程中，在光标处显示：旋转符号 + 最新 4 个识别文字。
/// 每次 ASR 更新时，只需退格固定数量的字符（最多 5 个）。
/// 录音结束时退格 5 个字符，输入完整最终文本。
///
/// 屏幕上的显示格式：`⠋今天天气` （1 个 spinner + 最多 4 个中文字）
#[derive(Debug)]
pub struct PartialResultManager {
    latest_text: String,
    /// 当前 spinner 帧索引
    spinner_frame: usize,
    /// 当前屏幕上显示的字符数（spinner + tail）
    displayed_chars: usize,
    /// 当前显示的尾部文字
    latest_tail: String,
}

impl Default for PartialResultManager {
    fn default() -> Self {
        Self {
            latest_text: String::new(),
            spinner_frame: 0,
            displayed_chars: 0,
            latest_tail: String::new(),
        }
    }
}

/// 频谱跳动帧（2 个 braille 字符，半角宽度，更窄更精致）
const SPINNER_FRAMES: &[&str] = &[
    "⡀⠄",
    "⠆⡀",
    "⡆⠒",
    "⠒⡆",
    "⠄⡀",
    "⡀⠆",
];
/// 每个 spinner 帧的字符数（所有帧等长）
#[cfg(test)]
const SPINNER_CHAR_COUNT: usize = 2;
const TAIL_CHAR_COUNT: usize = 5;

impl PartialResultManager {
    pub fn clear(&mut self) {
        self.latest_text.clear();
        self.spinner_frame = 0;
        self.displayed_chars = 0;
        self.latest_tail.clear();
    }

    fn next_spinner(&mut self) -> &'static str {
        let frame = SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()];
        self.spinner_frame += 1;
        frame
    }

    fn build_display(&mut self, tail: &str) -> String {
        let spinner = self.next_spinner();
        format!("{}{}", spinner, tail)
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

    /// 报告一次 poll 周期。无新文本时旋转 spinner。
    pub fn tick_stability(&mut self, text_changed: bool) -> Option<CommitAction> {
        if text_changed || self.displayed_chars == 0 {
            return None;
        }
        // 只更新 spinner 帧，保持 tail 不变
        let display = self.build_display(&self.latest_tail.clone());
        let delete = self.displayed_chars;
        self.displayed_chars = display.chars().count();
        let display_str = display.clone();
        debug!(
            "[tick] spinner rotate: delete={}, display='{}'",
            delete, display_str
        );
        Some(CommitAction::UpdateDraft {
            new_text: display,
            delete_chars: delete,
            confirm_chars: 0,
        })
    }

    /// 构造草稿提交动作：spinner + 最新 TAIL_CHAR_COUNT 个字。
    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        let window = normalize_partial_text(text);
        if window.is_empty() {
            return None;
        }

        let chars: Vec<char> = window.chars().collect();
        let tail_start = chars.len().saturating_sub(TAIL_CHAR_COUNT);
        let tail: String = chars[tail_start..].iter().collect();

        let display = self.build_display(&tail);
        let delete = self.displayed_chars;
        self.displayed_chars = display.chars().count();
        self.latest_tail = tail;

        let display_str = display.clone();
        let window_str = window.clone();
        debug!(
            "[draft] delete={}, display='{}', window='{}'",
            delete, display_str, window_str
        );

        Some(CommitAction::UpdateDraft {
            new_text: display,
            delete_chars: delete,
            confirm_chars: 0,
        })
    }

    /// 清除屏幕上的 spinner + tail。
    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        if self.displayed_chars == 0 {
            return None;
        }
        let delete = self.displayed_chars;
        self.displayed_chars = 0;
        self.latest_tail.clear();
        debug!("[draft_clear] delete={}", delete);
        Some(CommitAction::ClearDraft { delete_chars: delete })
    }

    /// 从 spinner + tail 过渡到最终文本。
    /// 退格删掉 spinner + tail，输入完整 final 文本。
    pub fn prepare_final_from_draft(&mut self, final_text: &str) -> Option<CommitAction> {
        if self.displayed_chars == 0 {
            return None;
        }

        let normalized = final_text.replace('\n', " ").trim().to_string();
        let delete = self.displayed_chars;

        let final_len = normalized.chars().count();
        debug!(
            "[final_from_draft] delete={} final='{}' (len={})",
            delete, &normalized, final_len
        );

        self.displayed_chars = 0;
        self.latest_tail.clear();
        self.latest_text.clear();

        Some(CommitAction::CommitFinalFromDraft {
            new_text: normalized,
            delete_chars: delete,
        })
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
    fn draft_first_commit_shows_spinner_and_tail() {
        let mut manager = PartialResultManager::default();
        let action = manager.prepare_draft_commit("你好世界").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, 0);
                // spinner(2) + 4 chars (text shorter than TAIL=5) = 6
                assert_eq!(new_text.chars().count(), SPINNER_CHAR_COUNT + 4);
                assert!(new_text.ends_with("你好世界"));
            }
            _ => panic!("expected UpdateDraft"),
        }
        assert_eq!(manager.displayed_chars, SPINNER_CHAR_COUNT + 4);
    }

    #[test]
    fn draft_short_text_shows_all() {
        let mut manager = PartialResultManager::default();
        let action = manager.prepare_draft_commit("你好").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, 0);
                assert_eq!(new_text.chars().count(), SPINNER_CHAR_COUNT + 2);
                assert!(new_text.ends_with("你好"));
            }
            _ => panic!("expected UpdateDraft"),
        }
        assert_eq!(manager.displayed_chars, SPINNER_CHAR_COUNT + 2);
    }

    #[test]
    fn draft_update_replaces_with_new_tail() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界真美丽");
        // text has 7 chars, tail=5 → first display: spinner(2) + "世界真美丽"(5) = 7
        let action = manager.prepare_draft_commit("今天天气很好我们").unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, SPINNER_CHAR_COUNT + 5);
                assert_eq!(new_text.chars().count(), SPINNER_CHAR_COUNT + 5);
                assert!(new_text.ends_with("很好我们")); // 最后5字: 气很好我们
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn draft_clear_removes_displayed() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界真美丽");
        let action = manager.prepare_draft_clear().unwrap();
        match action {
            CommitAction::ClearDraft { delete_chars } => {
                assert_eq!(delete_chars, SPINNER_CHAR_COUNT + 5);
            }
            _ => panic!("expected ClearDraft"),
        }
        assert_eq!(manager.displayed_chars, 0);
    }

    #[test]
    fn draft_clear_none_when_empty() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_draft_clear().is_none());
    }

    #[test]
    fn tick_rotates_spinner() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好世界真美丽");
        let action = manager.tick_stability(false).unwrap();
        match action {
            CommitAction::UpdateDraft { new_text, delete_chars, .. } => {
                assert_eq!(delete_chars, SPINNER_CHAR_COUNT + 5);
                assert_eq!(new_text.chars().count(), SPINNER_CHAR_COUNT + 5);
                assert!(new_text.ends_with("世界真美丽"));
            }
            _ => panic!("expected UpdateDraft"),
        }
    }

    #[test]
    fn tick_noop_when_text_changed() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("你好");
        assert!(manager.tick_stability(true).is_none());
    }

    #[test]
    fn tick_noop_when_nothing_displayed() {
        let mut manager = PartialResultManager::default();
        assert!(manager.tick_stability(false).is_none());
    }

    #[test]
    fn final_from_draft_replaces_all() {
        let mut manager = PartialResultManager::default();
        manager.prepare_draft_commit("今天天气很好我们去公园");
        // displayed = SPINNER_CHAR_COUNT + 5
        let action = manager
            .prepare_final_from_draft("今天天气很好，我们去公园。")
            .unwrap();
        match action {
            CommitAction::CommitFinalFromDraft { new_text, delete_chars } => {
                assert_eq!(delete_chars, SPINNER_CHAR_COUNT + 5);
                assert_eq!(new_text, "今天天气很好，我们去公园。");
            }
            _ => panic!("expected CommitFinalFromDraft"),
        }
    }

    #[test]
    fn final_from_draft_returns_none_when_empty() {
        let mut manager = PartialResultManager::default();
        assert!(manager.prepare_final_from_draft("any text").is_none());
    }
}
