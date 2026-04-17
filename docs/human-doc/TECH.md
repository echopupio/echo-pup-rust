# TECH（Human Readable）

最后更新：2026-04-17

## 技术方案摘要

当前技术方案分为两条主线：

- 已落地主线：采用“共享业务内核 + 双向 IPC”的实现策略。
- 业务动作（配置开关、编辑、下载等）统一在主进程执行。
- TUI 与状态栏只作为交互入口，避免各自维护一套逻辑。
- 配置与下载状态通过快照和事件回传，保持展示一致。

- 下一阶段主线：将语音识别从以 Whisper 为主的停录后整段转写，迁移为 `sherpa-onnx + SenseVoiceSmall` 的流式链路。
  - 模型常驻加载，避免每次录音冷启动。
  - partial 草稿与 final 最终结果拆开管理。
  - 首轮先保证 final 稳定插入，复杂草稿替换后续增强。

## 当前实施进展

- 已完成的代码级边界：
  - `AsrEngine` / `AsrSession` 抽象已建立。
  - `TextCommitBackend` 已建立，当前仍以 insert-only 为基线。
  - `RecognitionSession`、partial manager、final manager 已落地。
  - `AudioRecorder` 已支持 recent buffer 与增量读口，preview 不再依赖整段快照重复裁剪。
- 已完成的识别链路变化：
  - preview 路径已改为“增量音频块 -> `AsrSession` -> partial”。
  - final 路径已改为“统一 session -> `finalize()` -> 后处理 -> 提交”。
  - 配置已支持 `asr.backend = whisper | sherpa_sensevoice`，并支持 sherpa 初始化失败时自动回退到 Whisper。
- 当前 sherpa 状态：
  - 已接入 `sherpa-onnx` crate。
  - 已新增 `SherpaSenseVoiceEngine` 骨架。
  - 目前采用 `OfflineRecognizer + incremental session` 过渡实现，尚未证明等价于真流式 SenseVoice。

- Linux / Wayland 兼容主线：
  - 已确认当前热键实现依赖 `global-hotkey` 与 `rdev`，其中 `rdev` 在 Linux 上基于 X11，不适合作为 Wayland 主路径。
  - 已确认文本输入当前采用 `enigo`，Linux 下会回退到 `xdotool` / `wtype`。
  - 本轮新增文档决策：Wayland 下优先采用“桌面快捷键绑定 -> EchoPup 外部触发接口”的方案；`GlobalShortcuts` portal 仅作为后续增强路径。

## 关键改动

- 设计/技术文档统一为版本化文件（`system-design-v1`、`technical-solution-v1`）。
- 变更日志统一为日期化命名（`changelog-YYYYMMDD.md`）。
- 状态栏菜单同步与性能路线图归档到 `docs/architecture/`。
- 新增流式 ASR 迁移方案与 ADR：
  - `docs/architecture/streaming-asr-migration-plan-v1.md`
  - `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`
- 新增 Wayland 兼容方案与 ADR：
  - `docs/architecture/wayland-compatibility-plan-v1.md`
  - `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`

## 风险与应对

- 风险：双入口状态不同步。
  - 应对：动作在主进程串行执行，界面只消费状态。
- 风险：下载流程长尾失败。
  - 应对：保留续传、无进度超时、自动重试机制。
- 风险：流式 ASR 迁移涉及音频热路径、识别状态机与宿主输入兼容性。
  - 应对：先抽象接口与 insert-only 基线，再分阶段替换后端。
- 风险：当前 sherpa 只完成过渡型 session 包装，真实模型效果和延迟收益尚未实测。
  - 应对：先完成本机模型冒烟与固定 WAV/手工录音回归，再决定默认后端切换。
- 风险：Wayland 若继续沿用应用内全局热键思路，将长期与平台边界冲突。
  - 应对：把热键问题改造为 trigger backend 问题，由桌面环境负责快捷键绑定，应用负责业务动作。

## 后续实施顺序

1. 准备真实 `SenseVoiceSmall` 模型目录与 `tokens.txt`，切配置跑通 sherpa 冒烟。
2. 补固定 WAV 基线脚本或样例集，记录 Whisper 与 sherpa 的输出和耗时。
3. 做手工短句/长句录音回归，记录 `first_partial_ms`、`final_after_silence_ms`、热键释放到最终提交延迟。
4. 将 `main.rs` 中录音生命周期和 session orchestration 下沉到独立 `session_control` 模块。
5. 核验 sherpa Rust API 是否存在可用在线 SenseVoice 能力；若没有，明确继续过渡方案还是切换实现路径。
6. 在 sherpa 稳定后，再推进宿主草稿替换与更强的 partial/final 交互体验。
7. 为 Wayland 增加能力探测、外部触发入口与 README / runbook 指引。
