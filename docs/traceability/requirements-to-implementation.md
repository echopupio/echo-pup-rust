# 需求到实现追踪矩阵

最后更新：2026-03-19

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
| R-009 | 录音触发在按下/释放边界上保持稳定，不因抖动误停止 | `architecture/technical-solution-v1.md` 第 2/4 节 | `src/main.rs`, `src/hotkey/listener.rs` | `cargo test -q` + 手工回归 | 已实现 |
| R-010 | 状态栏空闲态紧凑、激活态边缘脉动反馈且支持自适应占位 | `design/system-design-v1.md` 第 4 节 | `src/status_indicator.rs`, `assets/logo.png`, `assets/mic.png` | macOS 手工验证 + `cargo test -q` | 已实现 |
| R-011 | 下载在代理异常环境下可回退直连并清理异常临时文件 | `architecture/technical-solution-v1.md` 第 4 节 | `src/model_download.rs` | `cargo test -q` + 下载日志核对 | 已实现 |
| R-012 | 录音过程中实时输出识别文本（流式转写预览） | `docs/changes/R-012-streaming-transcription.md` | `src/main.rs`, `src/stt/whisper.rs`, `src/audio/recorder.rs` | `cargo build` | 已实现 |
| R-013 | Linux 状态栏菜单（GNOME/X11） | `design/system-design-v1.md` 第 5 节 | `src/status_indicator.rs` | `cargo build` | 已实现 |

## 说明

- 当需求或实现变化时，及时维护本表。
- 使用稳定编号（`R-xxx`）避免跨文档漂移。
