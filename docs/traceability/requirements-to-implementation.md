# 需求到实现追踪矩阵

最后更新：2026-04-17

| 需求 ID | 需求摘要 | 设计章节 | 代码位置 | 测试证据 | 状态 |
| --- | --- | --- | --- | --- | --- |
| R-001 | 后台服务单实例生命周期管理 | `design/system-design-v1.md` 第 2 节 | `src/main.rs`, `src/runtime.rs` | `cargo test` + 手工回归命令 | 已实现 |
| R-002 | UI 生命周期管理 | `design/system-design-v1.md` 第 3 节 | `src/main.rs`, `src/ui.rs` | 手工回归 `echopup ui *` | 已实现 |
| R-003 | 模型下载稳定性（续传/重试/超时） | `design/system-design-v1.md` 第 5 节 | `src/model_download.rs`, `src/menu_core.rs`, `src/ui.rs` | `model_download` / `menu_core` 单测 | 已实现 |
| R-004 | 状态栏菜单与 TUI 功能对齐 | `architecture/status-bar-menu-sync-plan-v1.md` | `src/menu_core.rs`, `src/status_indicator.rs`, `src/main.rs`, `src/ui.rs` | `./scripts/run_acceptance.sh` | 已实现 |
| R-005 | 热键配置安全校验 | `design/system-design-v1.md` 第 5 节 | `src/hotkey/listener.rs`, `src/ui.rs` | hotkey 相关单测 | 已实现 |
| R-006 | 多通道反馈（状态栏/通知/提示音） | `design/system-design-v1.md` 第 1/4 节 | `src/main.rs`, `src/status_indicator.rs` | macOS 手工验证 | 已实现 |
| R-007 | 文档治理与同步机制 | `docs/README.md` | `docs/*` | 文档审计与变更日志 | 已实现 |
| R-008 | 热键触发模式可切换（长按模式/按压切换模式）且阈值 1 秒 | `design/system-design-v1.md` 第 2/4 节 | `src/config/config.rs`, `src/menu_core.rs`, `src/main.rs`, `src/status_indicator.rs` | `menu_core` 单测 + `cargo test -q`（40 passed） | 已实现 |
| R-009 | 录音触发在按下/释放边界上保持稳定，不因抖动误停止 | `architecture/technical-solution-v1.md` 第 2 节；`docs/adr/0003-hotkey-trigger-mode-and-adaptive-indicator.md` | `src/main.rs`, `src/hotkey/listener.rs` | `cargo test -q` + 手工回归 | 已实现 |
| R-010 | 状态栏空闲态紧凑、激活态边缘脉动反馈且支持自适应占位 | `design/system-design-v1.md` 第 4 节 | `src/status_indicator.rs`, `assets/logo.png`, `assets/mic.png` | macOS 手工验证 + `cargo test -q` | 已实现 |
| R-011 | 下载在代理异常环境下可回退直连并清理异常临时文件 | `architecture/technical-solution-v1.md` 第 6 节 | `src/model_download.rs` | `cargo test -q` + 下载日志核对 | 已实现 |
| R-012 | 录音过程中实时输出识别文本（流式转写预览） | `docs/changes/R-012-streaming-transcription.md` | `src/main.rs`, `src/stt/whisper.rs`, `src/audio/recorder.rs` | `cargo build` | 已实现 |
| R-013 | Linux 状态栏菜单（GNOME/X11） | `docs/changes/R-013-Linux-vs-macOS-comparison.md` | `src/status_indicator.rs` | `review-20260323-01` | 已实现（含待修复项） |
| R-014 | 模型下载 aria2 风格高速并发下载 | `docs/changes/R-014-aria2-style-model-download.md` | `src/model_download.rs`, `src/menu_core.rs`, `src/status_indicator.rs`, `src/ui.rs` | 待实现：`cargo test -q` + 下载回归（并发 / 降级 / 恢复） | 规划中 |
| R-015 | 本地 STT 主链路迁移到 `sherpa-onnx + SenseVoiceSmall`，支持 partial / final 双阶段输出并优先优化中文实时性 | `docs/architecture/streaming-asr-migration-plan-v1.md`；`docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`；`docs/architecture/technical-solution-v1.md` 第 5/7 节 | 已落地：`src/asr/*`, `src/session/*`, `src/commit/*`, `src/audio/*`, `src/main.rs`；待继续：`src/vad/*`, `session_control` 抽离 | 已有证据：`cargo test -q`（56 passed）；待补：真实 SenseVoice 模型冒烟、固定 WAV 基线、手工口述回归、`first_partial_ms` / `final_after_silence_ms` 指标验证 | 实施中 |
| R-016 | Wayland 下提供可解释的热键触发与文本提交兼容路径 | `docs/architecture/wayland-compatibility-plan-v1.md`；`docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`；`docs/adr/0005-wayland-trigger-and-text-commit-strategy.md` | 当前事实：`src/hotkey/listener.rs`, `src/input/keyboard.rs`, `src/commit/mod.rs`, `src/main.rs`；待实现：外部触发入口、能力探测、backend 选择日志 | 已有证据：2026-04-17 代码与环境核验；待补：CLI/IPC 触发实现、Wayland 手工回归 | 规划中 |

## 说明

- 当需求或实现变化时，及时维护本表。
- 使用稳定编号（`R-xxx`）避免跨文档漂移。
