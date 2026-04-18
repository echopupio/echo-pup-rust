# 流式草稿提交设计方案 (Streaming Draft Commit)

> 日期: 2026-04-18
> 状态: 待实施

## 1. 问题

当前录音结束后才一次性输出文字。用户希望在录音过程中就能看到文字逐步出现在光标处（边说边出文字）。

## 2. 现状

- 预览线程每 500ms 调用 `poll_partial()` 拿到中间识别结果
- 中间结果只更新状态栏（"识别中: ..."），不输入到光标处
- `CommitAction` 仅有 `CommitFinal`，`Keyboard` 仅有 `type_text`，不能发退格键
- `text_commit` 不传入预览线程，预览线程无提交能力
- Trait 已预留 `supports_draft_replacement()` 钩子（返回 false）

## 3. 方案

### 核心思路

录音期间，将 partial 结果以"草稿"形式输入光标处；新 partial 来时，用退格键删掉旧草稿，再输入新草稿；录音结束时删掉草稿，输入 final 文本。

### 改动清单

#### 3.1 扩展 Keyboard — 支持退格键
- **文件**: `src/input/keyboard.rs`
- 新增 `delete_backward(&mut self, count: usize) -> Result<()>`
- Enigo 后端: 调用 `enigo.key(Key::Backspace, Click)` 循环 count 次
- Linux 后端: `xdotool key --repeat {count} BackSpace`
- Unavailable 后端: 返回 error

#### 3.2 扩展 CommitAction 枚举
- **文件**: `src/commit/mod.rs`
- 新增变体:
  - `UpdateDraft { new_text: String, delete_chars: usize }` — 删 N 字 + 输入新草稿
  - `ClearDraft { delete_chars: usize }` — 仅删草稿（不输入新文字）

#### 3.3 实现 draft commit 逻辑
- **文件**: `src/commit/mod.rs`
- `InsertOnlyTextCommit` 新增 `draft_char_count: usize` 字段
- `apply(UpdateDraft)`: 先 `delete_backward(delete_chars)` → 再 `type_text(new_text)` → 更新 `draft_char_count`
- `apply(ClearDraft)`: `delete_backward(delete_chars)` → 重置 `draft_char_count`
- `apply(CommitFinal)`: 如果 `draft_char_count > 0`，先清草稿再输入
- `supports_draft_replacement()` 返回 true

#### 3.4 扩展 RecognitionSession — 草稿状态跟踪
- **文件**: `src/session/partial_result_manager.rs` + `src/session/mod.rs`
- `PartialResultManager` 新增 `committed_char_count: usize` — 已提交到光标的草稿字符数
- 新增方法 `prepare_draft_commit(&mut self, text: &str) -> Option<CommitAction>`
  - 计算需要删的字符数 = `committed_char_count`
  - 返回 `UpdateDraft { new_text, delete_chars }`
  - 更新 `committed_char_count = new_text.chars().count()`
- 新增方法 `prepare_draft_clear(&mut self) -> Option<CommitAction>`
  - 如果 `committed_char_count > 0`，返回 `ClearDraft { delete_chars }`
  - 重置 `committed_char_count = 0`
- `RecognitionSession` 暴露这两个方法

#### 3.5 预览线程接入 text_commit
- **文件**: `src/main.rs`
- 将 `text_commit` 的 `Arc` clone 传入预览线程闭包
- 在 `poll_partial` 拿到结果后:
  - 调 `session.update_partial(text)` → 更新 UI（保持现有行为）
  - 如果 draft 功能开启: 调 `session.prepare_draft_commit(text)` → `text_commit.lock().apply(action)`
- 检查 `text_commit.lock().supports_draft_replacement()` 决定是否启用

#### 3.6 stop_recording_action 适配
- **文件**: `src/main.rs`
- 在 `process_audio` 之前，先调 `session.prepare_draft_clear()` → `text_commit.lock().apply(clear_action)` 清掉草稿
- 然后照常执行 final commit

#### 3.7 配置开关（可选）
- **文件**: `src/config/config.rs`
- 新增 `[commit]` section: `streaming_draft = true/false`（默认 false）
- 运行时根据此配置 + `supports_draft_replacement()` 双重判断

## 4. 依赖关系

```
keyboard-backspace → commit-action-draft → commit-impl-draft ─┬→ preview-thread-commit
                                         → session-draft-state ┤→ stop-action-adapt
                                                               └→ config-switch (无依赖)
```

## 5. 风险与注意事项

1. **退格兼容性**: 部分富文本编辑器（如 Notion/Figma）的退格行为不标准，可能导致草稿删除不干净。配置开关可让用户在不兼容场景下关闭。
2. **锁竞争**: 预览线程和 stop 回调都要锁 `text_commit`。由于 preview 线程 500ms 间隔且操作很快，竞争风险低。
3. **输入焦点切换**: 用户在录音期间切换了窗口，退格可能删错内容。可考虑在焦点变化时自动 clear draft（进阶优化，不在 MVP 中）。
4. **中文字符计数**: `chars().count()` 对中文正确，但需确认 enigo 的退格键在各平台上删的是 Unicode 字符而非字节。

## 6. 改动范围总览

| 文件 | 改动类型 |
|---|---|
| `src/input/keyboard.rs` | 新增方法 |
| `src/commit/mod.rs` | 扩展枚举 + 实现 |
| `src/session/partial_result_manager.rs` | 新增字段和方法 |
| `src/session/mod.rs` | 暴露新方法 |
| `src/main.rs` | 预览线程 + stop 回调改动 |
| `src/config/config.rs` | 新增配置项（可选） |
