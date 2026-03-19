# 技术方案文档 v1 - echo-pup-rust

最后更新：2026-03-19

## 1. 背景与约束

- 业务约束：后台运行时必须保证用户能感知录音/识别状态。
- 技术约束：核心链路本地优先，Whisper 模型位于 `~/.echopup/models`。
- 平台约束：当前状态栏能力优先支持 macOS，Linux 菜单栏不在本轮范围。
- 工程约束：TUI 与状态栏不能维护两套独立业务逻辑。

## 2. 架构总览

进程与模块边界：
- 主进程（`echopup run`）：音频采集、转写、文本输入、配置与下载动作执行。
- TUI 进程（`echopup ui`）：交互式配置与下载管理。
- 状态栏子进程（`echopup status-indicator`）：状态展示与菜单交互入口。

当前重点改造结果（已落地）：
- 已提取共享菜单业务内核（`src/menu_core.rs` + `src/model_download.rs`），供 TUI 与状态栏共同调用。
- 状态栏通信已升级为双向 IPC，状态栏只负责展示/交互，主进程负责动作执行与状态回传。

## 3. 技术选型

| 领域 | 选型 | 选择理由 | 备选方案 |
| --- | --- | --- | --- |
| CLI | `clap` | 子命令与参数定义清晰 | 手写解析 |
| 配置 | `serde + toml` | 配置结构化、可读性高 | JSON / YAML |
| 热键 | `global-hotkey` + `rdev` | 跨平台兼容与低层监听补充 | 平台专用 API |
| 音频 | `cpal`（项目内封装） | 跨平台音频采集 | 平台专用库 |
| STT | `whisper-rs` | 本地推理、离线可用 | 远程 STT API |
| HTTP 下载 | `reqwest` blocking | 简化下载与重试逻辑 | 外部脚本调用 |
| TUI | `ratatui + crossterm` | 终端 UI 生态成熟 | 自绘终端控件 |
| 状态栏 | Cocoa / ObjC FFI | 原生 macOS 菜单栏能力 | 第三方跨平台托盘库 |

## 4. 数据与接口契约

关键契约：
- 配置契约：`src/config/config.rs`（`Config` 及子结构）。
- 运行时文件：
  - `~/.echopup/config.toml`（默认配置路径，可通过 `--config` 覆盖）
  - `~/.echopup/models/*.bin`
  - `~/.echopup/echopup.lock`
  - `~/.echopup/echopup.log`
- 子进程通信契约：
  - 已落地协议：NDJSON（stdin/stdout）。
  - 主进程 -> 状态栏：`SetState`、`SetSnapshot`、`SetActionResult`、`Exit`。
  - 状态栏 -> 主进程：`ActionRequest`（承载 `MenuAction`）。
  - 兼容策略：保留旧状态行协议解析，避免升级期间协议中断。

## 5. 实施里程碑状态（截至 2026-03-19）

- 里程碑 1（共享菜单内核重构）：已完成。
- 里程碑 2（状态栏双向 IPC 与菜单动作打通）：已完成。
- 里程碑 3（下载进度可视化与回归验收）：已完成（`./scripts/run_acceptance.sh`）。

## 6. 运维影响

- 发布：
  - 常规发布采用 `cargo build --release`
  - 推荐先在 macOS 环境进行状态栏回归
- 回滚：
  - 保留上一版二进制，必要时 `echopup stop` 后替换并重启
- 监控：
  - 以日志观察为主，重点关注热键、录音、识别、下载、IPC 错误

## 7. 安全与合规

- API Key 通过环境变量读取，不写入仓库。
- 热键策略限制过宽配置，避免吞键影响正常输入。
- 本地模型与本地推理优先，减少外发数据依赖。

## 8. 关联文档

- `docs/design/system-design-v1.md`
- `docs/architecture/status-bar-menu-sync-plan-v1.md`
- `docs/architecture/performance-optimization-roadmap-v1.md`
- `docs/requirements/PRD.md`
- `docs/operations/runbook.md`
