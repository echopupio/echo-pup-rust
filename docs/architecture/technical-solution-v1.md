# 技术方案文档 v1 - echo-pup-rust

最后更新：2026-03-19

## 1. 背景与约束

- 业务约束：后台运行时必须保证用户能感知录音/识别状态。
- 技术约束：核心链路本地优先，Whisper 模型位于 `~/.echopup/models`。
- 平台约束：状态栏能力支持 macOS 与 Linux (GNOME/X11)。
- 工程约束：TUI 与状态栏不能维护两套独立业务逻辑。

## 2. 架构总览

进程与模块边界：
- 主进程（`echopup run`）：音频采集、转写、文本输入、配置与下载动作执行。
- TUI 进程（`echopup ui`）：交互式配置与下载管理。
- 状态栏子进程（`echopup status-indicator`）：状态展示与菜单交互入口。

当前重点改造结果（已落地）：
- 已提取共享菜单业务内核（`src/menu_core.rs` + `src/model_download.rs`），供 TUI 与状态栏共同调用。
- 状态栏通信已升级为双向 IPC，状态栏只负责展示/交互，主进程负责动作执行与状态回传。
- 热键触发链路已升级为“双模式状态机”（长按模式 / 按压切换模式），并将触发模式纳入配置热更新。
- 状态栏占位已升级为“空闲窄宽度 + 激活宽宽度”自适应策略，兼顾紧凑布局与激活态可视化表达。

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
| 状态栏 | Cocoa / ObjC FFI (macOS) / tray-icon + muda + gtk (Linux) | 原生 macOS 菜单栏能力 / Linux GNOME 托盘 | - |

## 4. Linux 托盘实现 (R-013)



> 注：Linux 状态栏实现使用 tray-icon + muda 库，支持 GNOME/X11 环境的系统托盘。需要系统依赖：libx11-dev, libgtk-3-dev, libayatana-appindicator3-dev, libglib2.0-dev。


## 5. 数据与接口契约

关键契约：
- 配置契约：`src/config/config.rs`（`Config` 及子结构）。
  - 新增：`hotkey.trigger_mode`（`hold_to_record` / `press_to_toggle`）。
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
  - 交互策略：复杂输入动作通过状态栏弹窗承载（热键捕获、LLM 表单、下载进度），确认后再回传主进程执行。

下载稳定性补充契约：
- 范围下载请求支持分段重试与续传（`.part`）。
- 检测到代理连接失败时自动回退直连请求（no-proxy 客户端）。
- 下载失败后自动清理 0B 临时文件，避免后续重试污染。

## 5. 实施里程碑状态（截至 2026-03-19）

- 里程碑 1（共享菜单内核重构）：已完成。
- 里程碑 2（状态栏双向 IPC 与菜单动作打通）：已完成。
- 里程碑 3（下载进度可视化与回归验收）：已完成（`./scripts/run_acceptance.sh`）。
- 里程碑 4（触发模式状态机 + 状态栏视觉收敛）：已完成。

## 6. 运维影响

- 发布：
  - 常规发布采用 `cargo build --release`
  - 推荐先在 macOS 环境进行状态栏回归
- 回滚：
  - 保留上一版二进制，必要时 `echopup stop` 后替换并重启
- 监控：
  - 以日志观察为主，重点关注热键、录音、识别、下载、IPC 错误
  - 下载异常排查时优先检查代理环境变量（`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`）及直连回退日志

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
