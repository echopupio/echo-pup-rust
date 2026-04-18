//! 文本提交抽象

use crate::input::Keyboard;
use anyhow::Result;

/// 文本提交动作。
#[derive(Debug, Clone)]
pub enum CommitAction {
    /// 最终提交文本（录音结束后，无草稿在屏时使用）
    CommitFinal { text: String },
    /// 从草稿平滑过渡到最终文本：只替换尾部差异，不闪烁
    CommitFinalFromDraft {
        new_text: String,
        delete_chars: usize,
    },
    /// 更新草稿：先删旧草稿尾部再输入新后缀
    UpdateDraft {
        new_text: String,
        delete_chars: usize,
    },
    /// 清除草稿（不输入新文字）
    ClearDraft { delete_chars: usize },
}

/// 文本提交后端接口。
pub trait TextCommitBackend: Send {
    fn backend_name(&self) -> String;

    fn supports_draft_replacement(&self) -> bool {
        false
    }

    fn apply(&mut self, action: CommitAction) -> Result<()>;
}

/// 基于现有键盘输入能力的 insert-only 提交实现，支持草稿替换。
pub struct InsertOnlyTextCommit {
    keyboard: Keyboard,
    draft_char_count: usize,
}

impl InsertOnlyTextCommit {
    pub fn new() -> Result<Self> {
        Ok(Self {
            keyboard: Keyboard::new()?,
            draft_char_count: 0,
        })
    }
}

impl TextCommitBackend for InsertOnlyTextCommit {
    fn backend_name(&self) -> String {
        self.keyboard.backend_name().to_string()
    }

    fn supports_draft_replacement(&self) -> bool {
        true
    }

    fn apply(&mut self, action: CommitAction) -> Result<()> {
        match action {
            CommitAction::CommitFinal { text } => {
                if self.draft_char_count > 0 {
                    self.keyboard.delete_backward(self.draft_char_count)?;
                    self.draft_char_count = 0;
                }
                self.keyboard.type_text(&text)
            }
            CommitAction::CommitFinalFromDraft {
                new_text,
                delete_chars,
            } => {
                if delete_chars > 0 {
                    self.keyboard.delete_backward(delete_chars)?;
                }
                if !new_text.is_empty() {
                    self.keyboard.type_text(&new_text)?;
                }
                self.draft_char_count = 0;
                Ok(())
            }
            CommitAction::UpdateDraft {
                new_text,
                delete_chars,
            } => {
                if delete_chars > 0 {
                    self.keyboard.delete_backward(delete_chars)?;
                }
                self.keyboard.type_text(&new_text)?;
                self.draft_char_count =
                    self.draft_char_count - delete_chars + new_text.chars().count();
                Ok(())
            }
            CommitAction::ClearDraft { delete_chars } => {
                if delete_chars > 0 {
                    self.keyboard.delete_backward(delete_chars)?;
                }
                self.draft_char_count = 0;
                Ok(())
            }
        }
    }
}
