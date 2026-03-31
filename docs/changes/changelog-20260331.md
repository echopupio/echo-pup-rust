# 变更日志（20260331）

最后更新：2026-03-31

文件命名规则：`changelog-YYYYMMDD.md`。

## 本轮主题

- 沉淀“Whisper -> sherpa-onnx + SenseVoiceSmall”流式迁移方案，作为下一阶段语音输入主链路重构基线。
- 将迁移方向正式纳入 ADR、PRD、技术方案、性能路线图、追踪矩阵与治理台账。

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

## 方案要点

- 识别主链路目标后端调整为 `sherpa-onnx + SenseVoiceSmall`，中文优先、本地离线优先。
- 新架构采用“常驻引擎 + streaming session + partial/final 双阶段输出”。
- 迁移策略明确为分阶段推进，过渡期保留 Whisper 回退与对照能力。
- 文本提交以 `insert-only` 作为 MVP 基线，宿主草稿替换作为后续增强。

## 当前状态说明

- 本轮变更为“文档与架构决策沉淀”，尚未完成代码实现切换。
- 现态代码仍以当前 Whisper 链路为准。

## 风险与后续动作

- 风险：流式识别、partial/final 状态机与宿主输入兼容性会显著增加实现复杂度。
- 后续动作：
  - 先抽象 `AsrEngine` / `AsrSession` / `TextCommitBackend`
  - 再重构音频帧流与 session 控制
  - 随后接入 sherpa + SenseVoiceSmall 并做 shadow mode 验证
