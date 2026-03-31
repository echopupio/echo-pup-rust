# 变更日志（20260331）

最后更新：2026-03-31

文件命名规则：`changelog-YYYYMMDD.md`。

## 本轮主题

- 沉淀“Whisper -> sherpa-onnx + SenseVoiceSmall”流式迁移方案，作为下一阶段语音输入主链路重构基线。
- 将迁移方向正式纳入 ADR、PRD、技术方案、性能路线图、追踪矩阵与治理台账。
- 启动代码实施：抽象 ASR/session 边界、重构 partial/final 状态、引入增量音频读口、接入可配置的 sherpa backend 骨架。

## 文档变化

- 新增迁移方案主文档：
  - `docs/architecture/streaming-asr-migration-plan-v1.md`
- 新增架构决策记录：
  - `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`
- 更新基线文档：
  - `docs/requirements/BRD.md`
  - `docs/requirements/PRD.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/architecture/performance-optimization-roadmap-v1.md`
  - `docs/traceability/requirements-to-implementation.md`
  - `docs/README.md`
  - `docs/SPEC.md`
  - `docs/adr/README.md`
  - `docs/reports/project-court-ledger.md`
  - `docs/human-doc/TECH.md`
- 本次继续更新实施进度与后续动作：
  - `docs/architecture/streaming-asr-migration-plan-v1.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/traceability/requirements-to-implementation.md`
  - `docs/reports/project-court-ledger.md`
  - `docs/human-doc/TECH.md`

## 方案要点

- 识别主链路目标后端调整为 `sherpa-onnx + SenseVoiceSmall`，中文优先、本地离线优先。
- 新架构采用“常驻引擎 + streaming session + partial/final 双阶段输出”。
- 迁移策略明确为分阶段推进，过渡期保留 Whisper 回退与对照能力。
- 文本提交以 `insert-only` 作为 MVP 基线，宿主草稿替换作为后续增强。

## 当前状态说明

- 当前已进入代码实施阶段，但仍处于“Whisper 主链路可用 + sherpa backend 已接骨架”的过渡期。
- 已落地内容：
  - `AsrEngine` / `AsrSession` / `TextCommitBackend` 抽象已从 `main.rs` 中剥离。
  - `RecognitionSession`、`PartialResultManager`、`FinalResultManager` 已建立，partial/final 不再完全散落在主流程字符串变量中。
  - `AudioRecorder` 已具备 recent buffer 与增量读口，preview 线程改为按增量音频块推进。
  - preview 识别链路已切到 `AsrSession`，不再直接按定时 batch API 调 Whisper。
  - final 路径已统一到 `AsrSession::finalize()`。
  - 配置新增 `asr.backend` 与 `asr.sherpa.*`，并支持 sherpa 初始化失败时回退 Whisper。
- 当前仍未完成的部分：
  - sherpa SenseVoice 仅接入了 `OfflineRecognizer + incremental session` 包装，尚未验证真实模型文件与口述延迟指标。
  - `main.rs` 仍承担较多 session orchestration，`session_control` 尚未独立模块化。
  - 宿主侧仍以 insert-only 为主，尚未进入草稿替换阶段。

## 风险与后续动作

- 风险：流式识别、partial/final 状态机与宿主输入兼容性会显著增加实现复杂度。
- 后续动作：
  - 准备真实 SenseVoiceSmall 模型目录与 tokens，完成 sherpa backend 首次本机实测。
  - 将 `main.rs` 中录音生命周期与 preview/final 编排抽出为独立 `session_control`。
  - 为 sherpa 链路补充固定 WAV 冒烟、手工短句/长句回归与 `first_partial_ms` / `final_after_silence_ms` 指标记录。
  - 明确当前 sherpa Rust API 是否支持在线 SenseVoice；若不支持，决定继续用 offline wrapper 过渡，还是切到其他可流式 backend。
  - 在验证 sherpa 稳定前，保留配置化 Whisper 回退与对照能力。
