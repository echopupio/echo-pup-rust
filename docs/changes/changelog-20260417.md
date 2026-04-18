# 变更日志（20260417）

最后更新：2026-04-17

文件命名规则：`changelog-YYYYMMDD.md`。

## 本轮主题

- 基于真实代码与当前 Ubuntu GNOME Wayland 环境核验，为 EchoPup 沉淀 Wayland 热键触发与文本提交兼容方案。
- 将 Wayland 议题从“零散兼容性现象”提升为正式需求、架构方案与 ADR。
- 使用 doc-doc 文档治理思路回写项目文档基线，补齐索引、追踪、运维与人类可读文档映射。

## 文档变化

- 新增架构方案：
  - `docs/architecture/wayland-compatibility-plan-v1.md`
- 新增 ADR：
  - `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- 新增需求条目：
  - `docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`
- 更新基线文档：
  - `docs/README.md`
  - `docs/SPEC.md`
  - `docs/requirements/BRD.md`
  - `docs/requirements/PRD.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/traceability/requirements-to-implementation.md`
  - `docs/operations/runbook.md`
  - `docs/setup/environment-resources.md`
  - `docs/human-doc/TECH.md`
  - `docs/reports/project-court-ledger.md`
  - `docs/PROMPT-QA-LOG.md`
  - `README.md`

## 事实结论摘要

- 当前代码中：
  - 热键监听主要依赖 `global-hotkey` 与 `rdev`
  - 文本提交主要依赖 `enigo`，Linux 下回退 `xdotool` / `wtype`
- 已核验：
  - `rdev` Linux `listen` 基于 X11，不支持 Wayland
  - `enigo` 的 Wayland / libei 路径仍属 experimental
  - `wtype` 是 Wayland 文本提交的现实路径之一
- 当前验证环境（Ubuntu GNOME / Wayland）观测到：
  - 有 `RemoteDesktop` / `InputCapture` portal
  - 未观察到 `GlobalShortcuts` portal

## 决策摘要

- Wayland 下不再把“应用自己监听全局热键”作为主路径。
- 推荐主路径改为：
  - 桌面环境 / compositor 绑定快捷键
  - EchoPup 提供 CLI / IPC 外部触发接口
- Wayland 文本提交短期继续保留 `wtype` 作为明确 fallback。
- `GlobalShortcuts` portal 与 `libei` 路线保留为后续增强方向。

## 后续动作

- 增加 trigger backend 抽象。
- 为 Wayland 增加 `press/release/toggle` 外部触发入口。
- 在启动日志中输出会话类型、portal 能力、trigger backend、text commit backend。
- 补 GNOME / KDE / Sway / Hyprland 的绑定文档或示例。

## 同日代码修复补记

- 修复 macOS / X11 低层热键监听在运行时切换时不能真正“停旧启新”的问题。
- 原因：
  - `right_ctrl` 与裸 `F1-F12` 都走 `rdev::listen`
  - 旧实现切换热键时只设置停止标记，但 `rdev::listen` 线程本身不会退出
  - 菜单栏把热键从 `ctrl/right_ctrl` 改成 `f1` 一类低层热键时，运行时可能继续受旧监听线程影响，表现为配置已保存但热键未自动生效
- 修复方式：
  - 将低层热键监听改为单一常驻线程
  - 运行时仅动态切换目标键，而不重复创建不可停止的 `rdev` 监听线程
- 验证：
  - `cargo test hotkey -- --nocapture`
  - 新增单测覆盖同一低层监听运行时内 `right_ctrl -> f1` 的目标切换

## ASR 解码修复

- 修复 sherpa-onnx OnlineRecognizer 解码不完整导致"未识别到有效录音"的问题。
- 原因：
  - `decode_audio()` 缺少 `stream.input_finished()` 调用
  - `recognizer.decode(&stream)` 仅调一次，未循环直到 `is_ready()` 返回 false
  - 导致绝大部分音频帧未被处理，识别结果为空
- 修复方式：
  - 送入音频后调用 `stream.input_finished()` 标记结束
  - 改为 `while recognizer.is_ready(&stream) { recognizer.decode(&stream); }` 循环
- 涉及文件：`src/asr/sherpa_paraformer.rs`

## 流式草稿提交 (Streaming Draft Commit)

- 新增"边说边出文字"功能：录音期间将 ASR 中间结果以草稿形式实时输入到光标处。
- 核心流程：预览线程每 500ms 拿到 partial 结果 → 退格删除旧草稿 → 输入新草稿 → 录音结束时删除草稿、输入 final 文本。
- 改动清单：
  - `src/input/keyboard.rs`：新增 `delete_backward(count)` 退格键支持（Enigo + Linux）
  - `src/commit/mod.rs`：新增 `UpdateDraft` / `ClearDraft` 变体；`InsertOnlyTextCommit` 实现草稿跟踪
  - `src/session/partial_result_manager.rs`：新增 `committed_char_count`、`prepare_draft_commit()`、`prepare_draft_clear()`
  - `src/session/mod.rs`：暴露草稿方法
  - `src/main.rs`：预览线程接入 `text_commit` 做实时草稿输入；stop 路径先清草稿再提交 final
  - `src/config/config.rs`：新增 `[commit] streaming_draft = true` 配置项（默认开启）
- 菜单开关：
  - `src/menu_core.rs`：新增 `ToggleStreamingDraft` action
  - `src/ui.rs`：TUI 新增流式草稿开关菜单项
  - `src/status_indicator.rs`：macOS 状态栏 + Linux 托盘均新增 CheckMenuItem
- 新增 4 个单测，全部 59 测试通过。
- 设计文档：`docs/design/streaming-draft-commit.md`
