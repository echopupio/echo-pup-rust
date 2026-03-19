# 需求到实现追踪矩阵

最后更新：2026-03-19

| 需求 ID | 需求摘要 | 设计章节 | 代码位置 | 测试证据 | 状态 |
| --- | --- | --- | --- | --- | --- |
| R-001 | 后台服务单实例生命周期管理 | `design/system-design-v1.md` 第 2 节 | `src/main.rs`, `src/runtime.rs` | `cargo test` + 手工回归命令 | 已实现 |
| R-002 | UI 生命周期管理 | `design/system-design-v1.md` 第 3 节 | `src/main.rs`, `src/ui.rs` | 手工回归 `echopup ui *` | 已实现 |
| R-003 | 模型下载稳定性（续传/重试/超时） | `design/system-design-v1.md` 第 5 节 | `src/ui.rs` 下载模块 | `src/ui.rs` 内下载相关单测 | 已实现 |
| R-004 | 状态栏菜单与 TUI 功能对齐 | `architecture/status-bar-menu-sync-plan-v1.md` | `src/status_indicator.rs`, `src/ui.rs` | 菜单功能清单回归 | 进行中 |
| R-005 | 热键配置安全校验 | `design/system-design-v1.md` 第 5 节 | `src/hotkey/listener.rs`, `src/ui.rs` | hotkey 相关单测 | 已实现 |
| R-006 | 多通道反馈（状态栏/通知/提示音） | `design/system-design-v1.md` 第 1/4 节 | `src/main.rs`, `src/status_indicator.rs` | macOS 手工验证 | 已实现 |
| R-007 | 文档治理与同步机制 | `docs/README.md` | `docs/*` | 文档审计与变更日志 | 已实现 |

## 说明

- 当需求或实现变化时，及时维护本表。
- 使用稳定编号（`R-xxx`）避免跨文档漂移。
