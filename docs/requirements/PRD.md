# 需求文档（PRD）- echo-pup-rust

最后更新：2026-04-17

## 1. 背景

EchoPup 是本地优先的语音输入工具，核心场景是用户按住热键说话、松开后自动输入文本。当前产品已经具备 `run/start/stop/status/restart`、`ui` 管理界面、模型下载、Linux/macOS 状态反馈能力以及基于 Whisper 的本地转写链路。

当前进入下一阶段迭代：在保持现有输入法主链路可用的前提下，将语音识别后端从以 Whisper 为主逐步迁移到 `sherpa-onnx + SenseVoiceSmall`，优先解决中文识别速度、首字延迟和流式体验问题。

## 2. 目标与非目标

- 目标：
  - 在后台运行场景下，用户可通过状态栏完成与 TUI 等价的管理操作
  - 保持配置、下载、热键策略在多入口下行为一致
  - 在 Linux/Wayland 下提供不依赖应用内全局监听的稳定触发路径
  - 保持模型下载稳定性（断点续传、超时失败自动重试）
  - 将中文语音输入主链路迁移为更低延迟的本地流式识别架构
  - 支持 partial 草稿与 final 最终结果的双阶段输出
- 非目标：
  - 不把菜单/界面能力扩展作为本轮主目标
  - 不在首轮迁移中强制实现所有宿主的复杂草稿替换
  - 不以远程 STT 替代本地离线路径

## 3. 用户角色与关键场景

- 角色 A：日常写作/沟通用户（后台运行，随时语音输入）
- 角色 B：维护者/调试者（需要查看状态、调整配置、下载模型）

## 4. 范围

- 必须有：
  - 状态栏菜单具备与 TUI 对齐的核心管理能力（开关、热键捕获、LLM 表单、模型切换、下载弹窗、配置子菜单、关于与退出）
  - 热键编辑支持按键捕获与安全校验（最多 3 键）
  - 下载进度与关键日志在状态栏菜单可见
  - 新 ASR 架构需支持常驻模型、流式 session 与 final 稳定提交
- 应该有：
  - 主进程与状态栏之间双向 IPC，避免逻辑重复
  - 文档基线目录规范化并可持续维护
  - partial 与 final 必须解耦，支持状态栏草稿展示
- 暂不做：
  - 跨平台菜单栏统一实现（先完成 macOS）
  - 宿主复杂草稿替换的全面兼容

## 5. 功能需求

| ID | 需求描述 | 优先级 | 来源 |
| --- | --- | --- | --- |
| R-001 | 支持单实例后台运行与状态查询（`start/stop/status/restart`） | 高 | 已有 CLI 能力 |
| R-002 | 支持 UI 生命周期管理（`echopup ui start/stop/status/restart`） | 高 | 已有 CLI 能力 |
| R-003 | 模型必须下载到 `~/.echopup/models`，支持断点续传与重试 | 高 | 运行稳定性 |
| R-004 | 状态栏菜单必须覆盖与 TUI 对齐的核心管理动作，并支持模式切换与弹窗交互 | 高 | 本迭代核心目标 |
| R-005 | 热键配置必须可校验，拒绝过宽或危险组合 | 高 | 可用性与安全性 |
| R-006 | 录音/识别过程必须有可感知反馈（状态栏、通知、提示音） | 中 | 后台可感知性 |
| R-007 | 文档结构需与代码演进同步，具备需求-设计-实现追踪 | 中 | 工程治理 |
| R-012 | 支持录音过程中实时输出已识别文本（流式转写预览） | 中 | 后续体验优化需求 |
| R-013 | 在 Linux 上实现状态栏菜单（支持 GNOME/X11），功能与 macOS 对齐 | 高 | 用户跨平台需求 |
| R-013.1 | Linux 热键编辑弹窗（输入自定义热键） | 高 | 对齐 macOS 功能 |
| R-013.2 | Linux LLM 配置弹窗（输入 API Key 等） | 高 | 对齐 macOS 功能 |
| R-013.3 | Linux 模型下载弹窗（显示下载进度） | 中 | 对齐 macOS 功能 |
| R-013.4 | Linux 打开配置文件夹 | 中 | 提升可用性 |
| R-013.5 | Linux 查看模型文件 | 低 | 提升可用性 |
| R-014 | 模型下载需支持 aria2 风格高速并发下载，在大模型场景显著缩短首次下载耗时，并在不支持多连接时自动降级 | 中 | 下载体验优化 |
| R-015 | 将本地 STT 主链路从 Whisper 平滑迁移到 `sherpa-onnx + SenseVoiceSmall`，优先优化中文实时识别，并支持 partial / final 双阶段输出 | 高 | 中文实时输入体验升级 |
| R-016 | 在 Linux/Wayland 下提供可解释的热键触发与文本提交兼容路径，避免继续将 X11 假设作为默认前提 | 高 | 用户实际使用反馈 + 平台约束 |
| R-016.1 | 运行时需识别会话类型（X11 / Wayland）及关键能力（portal / 命令后端）并输出可排障日志 | 高 | 平台兼容性 |
| R-016.2 | Wayland 下需支持桌面快捷键绑定到 EchoPup CLI / IPC 触发接口（如 `press/release/toggle`） | 高 | Wayland 主路径 |
| R-016.3 | Wayland 文本提交需显式采用 Wayland 兼容后端优先级，而非仅作为 X11 路径失败后的隐式 fallback | 中 | 稳定性与可解释性 |

## 6. 非功能需求

- 性能：常规语音输入链路需保持当前可接受延迟，不因菜单同步导致明显回归
- 性能：迁移后需以 `first_partial_ms`、`final_after_silence_ms`、`commit_ms` 等指标量化收益
- 安全：热键策略需限制“吞掉常用输入键”的风险；敏感信息仍通过环境变量读取
- 可靠性：下载场景支持无进度超时失败并自动重试；失败信息可追踪
- 合规：本地优先处理，尽量减少额外外部依赖
- 可维护性：ASR 后端依赖必须通过统一抽象封装，避免第三方 API 污染业务层

## 7. 验收标准

| 需求 ID | 验收标准 | 验证方式 |
| --- | --- | --- |
| R-001 | 后台命令行为与单实例策略符合预期 | 手工回归 `echopup start/stop/status/restart` |
| R-002 | `echopup ui` 子命令可完整管理 UI 进程 | 手工回归 `echopup ui *` |
| R-003 | 下载中断后可续传，长时间无进度会失败并自动重试 | 模拟网络波动 + 日志检查 |
| R-004 | 状态栏菜单与 TUI 核心动作一致，弹窗动作可完成并即时生效 | 功能清单逐项验收 |
| R-005 | 无效热键会被拒绝并给出明确提示 | 单元测试 + 手工输入校验 |
| R-006 | 录音开始/结束、识别中/完成具备可见或可听反馈 | macOS 手工验证（普通/全屏） |
| R-007 | 基线文档目录齐备，变更日志与追踪矩阵可更新 | 文档审计与 review |
| R-012 | 录音进行中可持续看到增量识别文本，停录后结果与增量内容一致 | 实时录音场景手工回归 + 日志校验 |
| R-013 | 在 GNOME/X11 环境下状态栏菜单可见，核心管理功能（开关）与 macOS 等效 | Linux 环境手工验收 |
| R-013.1 | 热键编辑弹窗可正常输入并保存 | Linux 手工验收 |
| R-013.2 | LLM 配置弹窗可输入并保存 API Key | Linux 手工验收 |
| R-013.3 | 模型下载弹窗显示下载进度 | Linux 手工验收 |
| R-013.4 | 点击可打开配置文件夹 | Linux 手工验收 |
| R-013.5 | 点击可打开模型文件夹 | Linux 手工验收 |
| R-014 | 在支持 `Range` 的服务端上可自动启用多连接并发下载；下载中断后支持分片级恢复或明确降级；下载过程中的进度与错误在 UI / 状态栏中可见 | `cargo test -q` + 大模型下载手工回归 + 中断恢复验证 |
| R-015 | 中文普通话短句首个 partial 延迟与 final 延迟显著优于当前 Whisper 路径；支持热键结束或静音结束后的 final 稳定提交；过渡期保留 Whisper 回退能力 | 固定 WAV 基线 + 手工口述回归 + 指标日志聚合 |
| R-016 | Wayland 会话下存在明确、可文档化的主触发路径；README、runbook 与架构文档对该路径表述一致 | Wayland 环境手工回归 + 文档 review |
| R-016.1 | 启动日志能输出会话类型、portal 能力、trigger backend 与 text commit backend | 手工运行 + 日志检查 |
| R-016.2 | 用户可通过桌面环境快捷键绑定到 EchoPup 外部触发接口，稳定启动/停止录音状态机 | GNOME / 其他 compositor 手工回归 |
| R-016.3 | Wayland 文本提交能明确使用 `wtype` 或后续指定 backend，并在失败时提供清晰说明 | 手工回归 + 日志检查 |

## 8. 风险与待确认问题

- 风险：
  - 状态栏与 TUI 并存可能导致状态不同步
  - 菜单交互与主进程通信失败时可能出现“点击无响应”
  - aria2 风格下载若引入 piece map / 随机写入，需重点关注恢复语义与文件一致性
  - 流式 ASR 迁移会同时触及音频链路、识别状态机和宿主提交链路，回归面较大
  - Wayland 兼容若继续围绕应用内全局监听修补，可能持续与平台边界冲突
- 待确认问题：
  - 状态栏菜单中下载日志展示采用“最近 N 条文本”还是“独立详情窗”形式
  - Linux 状态栏菜单已实现全部功能 (tray-icon + muda + gtk)，与 macOS 等效
  - insert-only 宿主与支持草稿替换的宿主，是否采用统一提交策略还是双策略并行
  - 目标 Wayland 桌面中，哪些环境暴露 `GlobalShortcuts` portal，哪些只能依赖外部触发路径

## 9. 关联文档

- 业务需求：`docs/requirements/BRD.md`
- 设计文档：`docs/design/system-design-v1.md`
- 技术方案：`docs/architecture/technical-solution-v1.md`
- Wayland 兼容方案：`docs/architecture/wayland-compatibility-plan-v1.md`
- 流式迁移方案：`docs/architecture/streaming-asr-migration-plan-v1.md`
- 专项方案：`docs/architecture/status-bar-menu-sync-plan-v1.md`
- 性能路线图：`docs/architecture/performance-optimization-roadmap-v1.md`
- 专项需求：`docs/changes/R-014-aria2-style-model-download.md`
- 专项需求：`docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`
- ADR：`docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`
- ADR：`docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- 变更日志：`docs/changes/changelog-20260331.md`
- 变更日志：`docs/changes/changelog-20260417.md`

## 10. 历史兼容说明

- 历史文档中曾出现 `doctor`、`transcribe`、`config path` 等旧命令描述。
- 以上内容不再作为当前实现基线，当前事实以本 PRD 与关联文档为准。
- 历史版本可通过 Git 提交历史追溯。
