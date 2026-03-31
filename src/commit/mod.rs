//! 文本提交抽象

use crate::input::Keyboard;
use anyhow::Result;

/// 文本提交动作。
#[derive(Debug, Clone)]
pub enum CommitAction {
    CommitFinal { text: String },
}

/// 文本提交后端接口。
pub trait TextCommitBackend: Send {
    fn backend_name(&self) -> String;

    fn supports_draft_replacement(&self) -> bool {
        false
    }

    fn apply(&mut self, action: CommitAction) -> Result<()>;
}

/// 基于现有键盘输入能力的 insert-only 提交实现。
pub struct InsertOnlyTextCommit {
    keyboard: Keyboard,
}

impl InsertOnlyTextCommit {
    pub fn new() -> Result<Self> {
        Ok(Self {
            keyboard: Keyboard::new()?,
        })
    }
}

impl TextCommitBackend for InsertOnlyTextCommit {
    fn backend_name(&self) -> String {
        self.keyboard.backend_name().to_string()
    }

    fn apply(&mut self, action: CommitAction) -> Result<()> {
        match action {
            CommitAction::CommitFinal { text } => self.keyboard.type_text(&text),
        }
    }
}
