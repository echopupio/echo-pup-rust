//! 识别会话状态管理

mod final_result_manager;
mod partial_result_manager;

pub use final_result_manager::FinalResultManager;
pub use partial_result_manager::{PartialResultManager, PartialUpdate};

use crate::commit::CommitAction;

/// 当前录音会话的最小状态集合。
///
/// 第二阶段先把 partial / final 状态从 `main.rs` 抽离出来，
/// 后续再继续演进成更完整的 streaming session。
#[derive(Debug, Default)]
pub struct RecognitionSession {
    partials: PartialResultManager,
    finals: FinalResultManager,
}

impl RecognitionSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.partials.clear();
        self.finals.clear();
    }

    pub fn clear_partials(&mut self) {
        self.partials.clear();
    }

    pub fn update_partial(&mut self, text: &str) -> Option<PartialUpdate> {
        self.partials.update(text)
    }

    pub fn prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction> {
        self.partials.prepare_draft_commit(text)
    }

    pub fn tick_stability(&mut self, text_changed: bool) -> Option<CommitAction> {
        self.partials.tick_stability(text_changed)
    }

    pub fn prepare_draft_clear(&mut self) -> Option<CommitAction> {
        self.partials.prepare_draft_clear()
    }

    pub fn prepare_final_commit(&mut self, text: &str) -> Option<CommitAction> {
        // 如果有活跃草稿，平滑过渡到最终文本（只替换尾部差异）
        if let Some(action) = self.partials.prepare_final_from_draft(text) {
            return Some(action);
        }
        // 无草稿时，走普通 CommitFinal 路径
        self.finals.prepare_commit(text)
    }
}
