# ADR 0004 - 本地 STT 主链路迁移至 sherpa-onnx + SenseVoiceSmall 并采用流式 partial/final 双阶段架构

日期：2026-03-31
状态：已采纳

## 背景

当前项目已经具备本地语音输入主链路，但现有实现仍以 Whisper 停录后整段转写为主，中文输入体验的主要问题集中在：

1. 首字延迟较高，实时性不足。
2. “流式预览”依赖对录音快照反复重跑识别，重复计算重。
3. 录音、转写、partial 展示、final 提交和宿主输入边界耦合，难以继续演进。

项目下一阶段的核心目标已明确为：在 Linux 优先、本地离线优先的前提下，将主链路迁移为 `Rust + sherpa-onnx + SenseVoiceSmall`，优先优化中文实时体验，并为未来“一边说一边出字 + 结束后再给出更完整 final”做架构铺垫。

## 决策

1. 本地 STT 主链路的目标后端从以 `whisper-rs` 为主，迁移为以 `sherpa-onnx + SenseVoiceSmall` 为主。
2. 识别架构从“停录后整段识别”演进为“常驻引擎 + 每次录音一个 streaming session + partial/final 双阶段输出”。
3. 业务层统一通过 `AsrEngine` / `AsrSession` / `RecognitionEvent` 接口访问识别能力，不直接依赖第三方后端类型。
4. 宿主输入默认采用 `insert-only` 稳定基线；草稿替换能力作为后续增强，不纳入首轮主路径切换前置条件。
5. 迁移采用阶段化方式推进，Whisper 后端在过渡期保留为回退与对照路径。

## 备选方案与取舍

1. 继续优化当前 Whisper 路径  
   未选原因：即使继续调参，停录后整段识别和快照重跑模式仍难满足低延迟中文输入法体验。

2. 直接切换远程流式 ASR  
   未选原因：不符合本地离线优先约束，也会带来网络不确定性与隐私边界变化。

3. 一次性重写整个输入法主流程  
   未选原因：回归面过大，难以在保持当前可用性的前提下逐步验证。

## 影响

- 正向影响：
  - 为中文短句和实时输入场景提供更低的首字与 final 延迟目标。
  - 通过统一抽象隔离第三方依赖，后续支持 Paraformer / Whisper 等后端更容易。
  - partial / final 状态机从主流程中解耦，便于单测、回归和性能观测。

- 负向影响：
  - 音频、识别、提交三条链路都要重构，短期内工程复杂度上升。
  - 宿主草稿替换能力在不同桌面环境中的兼容性仍存在不确定性。
  - 迁移期间需要维护双后端与 shadow mode，对日志与配置提出更高要求。

## 落地位置

- 方案文档：
  - `docs/architecture/streaming-asr-migration-plan-v1.md`
- 现有链路参考：
  - `src/main.rs`
  - `src/audio/recorder.rs`
  - `src/stt/whisper.rs`
  - `src/input/keyboard.rs`
- 规划中的代码落点：
  - `src/asr/*`
  - `src/session/*`
  - `src/commit/*`
  - `src/audio/*`
  - `src/vad/*`

## 后续约束

1. 在 sherpa 后端成为默认路径前，必须保留配置化回退到 Whisper 的能力。
2. partial 文本不得直接假定为最终文本；必须经过独立状态管理与去抖策略。
3. 任何宿主草稿替换功能上线前，必须先提供 insert-only 降级策略。
4. 性能目标、验收标准和追踪矩阵必须与迁移实现同步更新。

## 关联文档

- `docs/architecture/streaming-asr-migration-plan-v1.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/architecture/performance-optimization-roadmap-v1.md`
- `docs/requirements/PRD.md`
- `docs/traceability/requirements-to-implementation.md`
