# 技术方案文档 v1 - echo-pup-rust

最后更新：2026-04-17

## 1. 背景与约束

- 业务约束：后台运行时必须保证用户能感知录音/识别状态。
- 技术约束：核心链路本地优先；当前主链路处于“Whisper 可用 + sherpa backend 已接入骨架”的过渡阶段，目标后端仍为 `sherpa-onnx + SenseVoiceSmall`。
- 平台约束：状态栏能力支持 macOS 与 Linux；Linux 下 X11 与 Wayland 的输入能力边界不同，不能继续把 X11 全局热键假设直接套用到 Wayland。
- 工程约束：TUI 与状态栏不能维护两套独立业务逻辑。
- 实时性约束：中文实时输入需优先降低首字延迟、句末 final 延迟和热键释放后的等待感。

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
- STT 迁移已进入代码实施：
  - `AsrEngine` / `AsrSession` / `TextCommitBackend` 已建立。
  - `RecognitionSession`、partial manager、final manager 已建立。
  - `AudioRecorder` 已具备 recent buffer 与增量读口。
  - preview 与 final 识别路径已统一走 session。
  - 配置已支持 `asr.backend` 与 `asr.sherpa.*`，并支持 sherpa 失败时回退 Whisper。

## 3. 技术选型

| 领域 | 选型 | 选择理由 | 备选方案 |
| --- | --- | --- | --- |
| CLI | `clap` | 子命令与参数定义清晰 | 手写解析 |
| 配置 | `serde + toml` | 配置结构化、可读性高 | JSON / YAML |
| 热键 | 当前：`global-hotkey` + `rdev`；Wayland 规划：桌面快捷键绑定 + EchoPup CLI / IPC 触发，portal 可用时再补 `GlobalShortcuts` backend | 兼顾现有 X11/macOS 路径与 Wayland 平台边界 | 继续仅靠应用内全局监听 |
| 音频 | `cpal`（项目内封装） | 跨平台音频采集 | 平台专用库 |
| STT | 当前：`whisper-rs`；规划：`sherpa-onnx + SenseVoiceSmall` | 保持本地离线前提下，逐步从整段识别迁移到更适合中文实时输入的流式链路 | 远程 STT API |
| HTTP 下载 | `reqwest` blocking | 简化下载与重试逻辑 | 外部脚本调用 |
| TUI | `ratatui + crossterm` | 终端 UI 生态成熟 | 自绘终端控件 |
| 状态栏 | Cocoa / ObjC FFI (macOS) / tray-icon + muda + gtk (Linux) | 原生 macOS 菜单栏能力 / Linux GNOME 托盘 | - |

Wayland 兼容补充：

- 当前仓库事实：`src/hotkey/listener.rs` 默认走 `global-hotkey`，`right_ctrl` 特判走 `rdev::listen`；`src/input/keyboard.rs` 默认先尝试 `enigo`，Linux 失败后回退 `xdotool` / `wtype`。
- 已核验约束：`rdev` Linux `listen` 基于 X11；`enigo` 的 Wayland / libei 路径仍属 experimental。
- 因此，Wayland 下推荐主路径不是“应用自己监听全局热键”，而是“桌面环境绑定快捷键 -> EchoPup 外部触发接口”。
- 当运行环境存在 `GlobalShortcuts` portal 时，可作为后续增强 backend；当前 Ubuntu GNOME Wayland 验证环境未观测到该接口。

## 4. Linux 托盘实现 (R-013)



> 注：Linux 状态栏实现使用 tray-icon + muda 库，支持 GNOME/X11 环境的系统托盘。需要系统依赖：libx11-dev, libgtk-3-dev, libayatana-appindicator3-dev, libglib2.0-dev。


## 5. 下一阶段 STT 演进（R-015）

目标演进：

- 将识别主链路从“停录后整段转写”演进为“常驻引擎 + streaming session + partial/final 双阶段输出”。
- 保留 Whisper 作为过渡期回退与对照路径。
- 将第三方识别后端隔离在 `src/asr/backends/*`，业务层统一依赖项目内 trait。

核心边界：

- `audio_capture` / `audio_resample`：只处理音频，不感知识别模型。
- `vad`：负责语音边界与 endpoint 候选，不直接决定文本提交。
- `asr_engine`：只产出统一 `RecognitionEvent`，不直接操作 UI 或输入法宿主。
- `partial_result_manager` / `final_result_manager`：分别处理草稿与最终文本状态。
- `text_commit`：提供 insert-only 基线与后续 draft-replace 扩展点。

实施原则：

1. 先抽象接口与状态机，再替换引擎。
2. 先把 partial 显示稳定在状态栏或浮层，再逐步增强宿主输入体验。
3. 所有性能收益必须通过结构化指标验证，而不是仅靠主观体感。

当前实现状态补充：

- 已完成：
  - 运行时 backend 选择与 Whisper 回退。
  - partial 预览从 batch 调用切到 `AsrSession`。
  - final 结果统一走 `AsrSession::finalize()`。
  - sherpa backend 骨架与 `sherpa-onnx` 依赖已接入。
- 未完成：
  - sherpa 真实模型文件的本机验证。
  - 真正在线 SenseVoice 能力确认。
  - `session_control` 模块化抽离。
  - 固定 WAV / 手工录音 / 指标采集闭环。

详细方案见：

- `docs/architecture/streaming-asr-migration-plan-v1.md`
- `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`

## 6. 数据与接口契约

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

Wayland 兼容新增契约（规划中）：
- trigger backend 需显式区分：
  - `global_hotkey`（现有 X11 / macOS）
  - `external_trigger`（Wayland 主路径）
  - `portal_global_shortcuts`（可选增强路径）
- text commit backend 需显式区分：
  - X11：`enigo` / `xdotool`
  - Wayland：`wtype`（短期明确 fallback）
  - Future：`libei` / portal-backed backend
- 启动日志需记录：
  - `XDG_SESSION_TYPE`
  - `XDG_CURRENT_DESKTOP`
  - portal 能力摘要
  - trigger backend / text commit backend

## 7. 实施里程碑状态（截至 2026-03-31）

- 里程碑 1（共享菜单内核重构）：已完成。
- 里程碑 2（状态栏双向 IPC 与菜单动作打通）：已完成。
- 里程碑 3（下载进度可视化与回归验收）：已完成（`./scripts/run_acceptance.sh`）。
- 里程碑 4（触发模式状态机 + 状态栏视觉收敛）：已完成。
- 里程碑 5（流式 ASR 迁移方案、ADR 与追踪基线）：已完成。
- 里程碑 6（ASR/session 边界与增量音频入口）：已完成。
- 里程碑 7（backend 选择、sherpa backend 骨架、统一 final session）：已完成第一阶段，待真实模型与指标验证。

## 8. 运维影响

- 发布：
  - 常规发布采用 `cargo build --release`
  - 推荐先在 macOS 环境进行状态栏回归
- 回滚：
  - 保留上一版二进制，必要时 `echopup stop` 后替换并重启
- 监控：
  - 以日志观察为主，重点关注热键、录音、识别、下载、IPC 错误
  - 下载异常排查时优先检查代理环境变量（`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`）及直连回退日志

## 9. 安全与合规

- API Key 通过环境变量读取，不写入仓库。
- 热键策略限制过宽配置，避免吞键影响正常输入。
- 本地模型与本地推理优先，减少外发数据依赖。
- 新增 ASR 后端时需同步审查模型文件来源、校验信息和运行时 provider 配置。

## 10. 关联文档

- `docs/design/system-design-v1.md`
- `docs/architecture/wayland-compatibility-plan-v1.md`
- `docs/architecture/streaming-asr-migration-plan-v1.md`
- `docs/architecture/status-bar-menu-sync-plan-v1.md`
- `docs/architecture/performance-optimization-roadmap-v1.md`
- `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`
- `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- `docs/requirements/PRD.md`
- `docs/operations/runbook.md`
