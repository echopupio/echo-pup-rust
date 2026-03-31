# TECH（Human Readable）

最后更新：2026-03-31

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

## 关键改动

- 设计/技术文档统一为版本化文件（`system-design-v1`、`technical-solution-v1`）。
- 变更日志统一为日期化命名（`changelog-YYYYMMDD.md`）。
- 状态栏菜单同步与性能路线图归档到 `docs/architecture/`。
- 新增流式 ASR 迁移方案与 ADR：
  - `docs/architecture/streaming-asr-migration-plan-v1.md`
  - `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`

## 风险与应对

- 风险：双入口状态不同步。
  - 应对：动作在主进程串行执行，界面只消费状态。
- 风险：下载流程长尾失败。
  - 应对：保留续传、无进度超时、自动重试机制。
- 风险：流式 ASR 迁移涉及音频热路径、识别状态机与宿主输入兼容性。
  - 应对：先抽象接口与 insert-only 基线，再分阶段替换后端。
