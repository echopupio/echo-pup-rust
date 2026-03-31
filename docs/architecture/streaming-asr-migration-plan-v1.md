# 流式 ASR 迁移方案 v1 - sherpa-onnx + SenseVoiceSmall

最后更新：2026-03-31

## 实施进展（截至 2026-03-31）

当前方案已从“纯规划”进入“代码实施中”：

- 已完成：
  - `AsrEngine` / `AsrSession` / `TextCommitBackend` 抽象落地。
  - `RecognitionSession`、partial manager、final manager 落地。
  - `AudioRecorder` 增加 recent buffer、增量读口与 preview 本地窗口推进。
  - preview 识别链路改为增量音频喂 `AsrSession`。
  - final 识别链路统一改为 `AsrSession::finalize()`。
  - 配置新增 `asr.backend` 和 `asr.sherpa.*`，允许 sherpa 初始化失败时自动回退到 Whisper。
  - `sherpa-onnx` crate 已接入，`SherpaSenseVoiceEngine` 已以 `OfflineRecognizer + incremental session` 形式实现第一版 backend 骨架。
- 仍未完成：
  - 真实 `SenseVoiceSmall` 模型与 `tokens.txt` 的本机冒烟。
  - 固定 WAV 基线与手工口述回归。
  - `first_partial_ms` / `final_after_silence_ms` / 热键释放到最终提交延迟的量化记录。
  - `main.rs` 中 session orchestration 的独立模块化。
  - 真正在线 SenseVoice 能力确认；当前 sherpa 方案仍属过渡实现。

### 紧接着要做什么

1. 准备并配置真实 `SenseVoiceSmall` 模型目录，完成 sherpa backend 首次本机录音验证。
2. 增加固定 WAV 基线和手工录音回归，明确 Whisper 与 sherpa 的输出差异与时延。
3. 把录音生命周期与 preview/final 编排从 `main.rs` 抽出为 `session_control`。
4. 判断是否继续沿用当前 sherpa 过渡方案，还是切换到具备真在线能力的 backend / FFI 路径。
5. 在识别链路稳定后，再推进宿主草稿替换和更丰富的 partial/final 交互策略。

## 1. 背景与目标

当前项目的语音输入主链路已可用，但现状仍以 `Whisper` 停录后整段转写为主，主要瓶颈集中在：

- 首字延迟高，中文口述时等待感明显。
- “实时预览”仍依赖对录音快照反复重跑识别，重复计算较重。
- 录音、转写、partial 展示、final 提交和键盘输入耦合在主流程中，后续替换 ASR 后端风险较高。
- 当前输入层以简单文本插入为主，不具备稳定的草稿替换能力。

本方案的目标是将本地 STT 主链路迁移为：

- Rust
- sherpa-onnx
- SenseVoiceSmall
- 中文优先
- 更低延迟的本地流式识别
- 支持“partial 草稿 + final 最终提交”的双阶段体验

本方案强调“平滑迁移、边界先行、逐步替换”，不追求一次性重写。

## 2. 当前实现问题盘点

当前代码中的关键现状：

- `src/main.rs` 仍以 `process_audio()` 串行执行“转写 -> LLM -> 纠错 -> 键盘输入”。
- `src/audio/recorder.rs` 使用 `Mutex<Vec<f32>>` 持有整段录音，`get_snapshot()` 与 `stop()` 都会 clone 整段缓冲并按需重采样。
- `src/stt/whisper.rs` 已提供增量与 callback 形式的包装，但底层仍是整段 `full()` 推理。
- `src/input/keyboard.rs` 仅提供 `type_text()` 插入能力，缺少宿主级草稿替换能力。

由此导致的直接结果：

- partial 体验依赖快照轮询与重复推理，不适合真正低延迟链路。
- 录音线程与识别线程的边界不清晰，后续接入流式引擎时容易把阻塞带回热路径。
- `partial` 与 `final` 没有独立状态机，后续要实现“边说边出字，结束再修正”会遇到去重、替换和光标稳定性问题。

## 3. 总体迁移策略

### 3.1 原则

1. 先抽象边界，再替换引擎。
2. 先完成识别链路重构，再增强宿主输入体验。
3. 先保留 Whisper 后端作为回退，再默认切换到 sherpa + SenseVoice。
4. 文档、追踪矩阵、性能指标与回滚开关必须同步存在。

### 3.2 新旧方案关键差异

旧链路：

`热键 -> 录音 -> 停止 -> 整段 Whisper -> 后处理 -> 输入`

新链路：

`热键 -> 流式音频帧 -> VAD / endpoint -> 常驻 ASR session -> partial manager -> final manager -> text commit`

### 3.3 重构顺序

优先顺序建议如下：

1. 提炼 `AsrEngine` / `AsrSession` / `TextCommitBackend` 抽象。
2. 将 `main.rs` 中的 partial / final / commit 状态机拆入独立模块。
3. 重构音频链路为 ring buffer + 16k 帧流，但先继续兼容 Whisper。
4. 新增 sherpa-onnx + SenseVoiceSmall backend，并先做离线整段验证。
5. 再接入 streaming session 与 partial/final 双阶段输出。
6. 最后视宿主兼容性逐步增强“草稿替换”能力。

## 4. 模块架构设计

### 4.1 模块拆分

| 模块 | 职责 | 输入 | 输出 | 依赖边界 |
| --- | --- | --- | --- | --- |
| `audio_capture` | 从 CPAL 采集原始 PCM，保证回调轻量 | 麦克风原始帧 | 原始音频块 | 仅依赖采集设备 |
| `audio_resample` | 下混、增益、归一化、增量重采样到 16k mono | 原始音频块 | 16k 单声道帧 | 不依赖 VAD / ASR |
| `vad` | 语音活动检测、pre-roll、endpoint 候选 | 16k 音频帧 | `VadEvent` | 不直接提交文本 |
| `asr_engine` | 管理常驻模型、创建 session、驱动流式解码 | 16k 音频帧 | `RecognitionEvent` | 屏蔽第三方 API |
| `partial_result_manager` | partial 去抖、稳定前缀管理、展示节流 | partial 事件 | 可展示草稿 | 不直接写宿主 |
| `final_result_manager` | final 合并、去重、delta 计算、段落管理 | final 事件 | `CommitAction` | 不依赖具体输入后端 |
| `text_commit` | 将文本提交到当前光标或宿主草稿区 | `CommitAction` | 宿主文本变更 | 屏蔽 enigo / xdotool / wtype 差异 |
| `hotkey/session_control` | 管理录音生命周期、session 状态与热键模式 | 热键 / VAD / ASR 事件 | 状态迁移与调度 | 替代当前 `main.rs` 大编排 |
| `config` | 后端、模型、VAD、提交策略、性能参数 | TOML | typed config | 单一配置入口 |
| `logging/metrics` | 记录首字延迟、final 延迟、backlog、失败原因 | 各模块埋点 | 结构化日志 / 报表 | 不进入热路径决策 |

### 4.2 推荐目录结构

```text
src/
  app/
    orchestrator.rs
    metrics.rs
  audio/
    capture.rs
    frame.rs
    pipeline.rs
    resample.rs
    ring.rs
  asr/
    mod.rs
    traits.rs
    events.rs
    backends/
      whisper.rs
      sherpa_onnx.rs
    sensevoice/
      model.rs
      postprocess.rs
  vad/
    mod.rs
    detector.rs
    endpoint.rs
  session/
    recognition_session.rs
    session_control.rs
    partial_result_manager.rs
    final_result_manager.rs
  commit/
    mod.rs
    text_commit.rs
    insert_only.rs
    draft_replace.rs
  config/
    config.rs
    asr.rs
    vad.rs
    commit.rs
```

拆分原因：

- `audio`、`asr`、`session`、`commit` 分别沿采集、识别、状态机、输入四条轴演进，避免后续换引擎或换宿主时互相污染。
- `session` 独立后，`partial` / `final` 状态机可以脱离 `main.rs` 单独测试。
- `asr/backends` 下同时保留 Whisper 与 sherpa backend，可支持平滑回退与 shadow mode。

## 5. sherpa-onnx 接入方案

### 5.1 设计原则

- 业务层禁止直接依赖 sherpa-onnx 类型。
- 第三方依赖必须通过 adapter 封装在 `src/asr/backends/sherpa_onnx.rs`。
- 允许初期使用现成 Rust 封装或 FFI wrapper，但对外只暴露项目内 trait。

### 5.2 引擎抽象

- `AsrEngine`：进程级、重量级、常驻对象，负责 preload / warmup / create_session。
- `AsrSession`：一次录音对应一个 session，负责 `accept_audio` / `poll` / `finalize`。
- `RecognitionEvent`：统一 partial / final / metrics / error 事件。

### 5.3 多后端扩展位

接口需要预留以下扩展：

- `AsrBackendKind`：`Whisper` / `SherpaSenseVoice` / `SherpaParaformer`
- `AsrCapabilities`：是否支持 streaming / partial / endpoint / timestamps
- `SessionOptions`：语言、热词、标点、ITN、调试开关
- `TranscriptMetadata`：置信度、时间戳、语言标签、特殊 token 信息

### 5.4 推荐接口草案

```rust
pub trait AsrEngine: Send + Sync {
    fn backend(&self) -> AsrBackendKind;
    fn capabilities(&self) -> &AsrCapabilities;
    fn preload(&self) -> Result<()>;
    fn warmup(&self) -> Result<()>;
    fn create_session(&self, opts: SessionOptions) -> Result<Box<dyn AsrSession>>;
}

pub trait AsrSession: Send {
    fn id(&self) -> SessionId;
    fn accept_audio(&mut self, pcm16k: &[f32]) -> Result<Vec<RecognitionEvent>>;
    fn poll(&mut self) -> Result<Vec<RecognitionEvent>>;
    fn finalize(&mut self, reason: FinalizeReason) -> Result<Vec<RecognitionEvent>>;
    fn reset(&mut self) -> Result<()>;
}
```

## 6. SenseVoiceSmall 集成设计

### 6.1 模型文件组织

建议目录：

```text
~/.echopup/models/asr/sensevoice-small/
  manifest.json
  model.onnx
  tokens.txt
  ...
```

`manifest.json` 至少记录：

- backend = `sherpa_onnx`
- model_family = `sensevoice_small`
- version
- sample_rate = `16000`
- quantization = `int8/fp16/fp32`
- sha256

### 6.2 生命周期

- 进程启动后后台 preload 模型。
- preload 后执行一次 300ms 到 500ms 静音 warmup。
- 每次按下热键只创建 streaming session，不重新初始化模型。
- 引擎生命周期随进程，session 生命周期随本次录音。

### 6.3 资源复用

- ONNX Runtime env / session 常驻。
- sherpa recognizer 常驻。
- 每个 session 仅分配自身 stream / decoder state。
- 复用 resampler 缓冲、partial manager 和 final manager，避免频繁分配。

### 6.4 中文后处理关注点

- 去掉模型输出中的语言 / 事件 / 情感类特殊 token。
- 统一中文标点为全角，英文内部分隔保留半角。
- 中英混输时做空格规范化，避免“中文 english中文”粘连。
- 数字、日期、金额优先走 ITN；模型不足时用规则补齐。
- 热词和专名通过配置或词典增强，不默认把 LLM 放进热路径。
- 命令词默认关闭，避免口述文本误触发控制行为。

## 7. 实时识别链路设计

完整链路：

1. 麦克风采集
2. CPAL 回调将原始 PCM 推入 ring buffer
3. `audio_worker` 下混 / 增益 / 增量重采样到 16k mono
4. 帧化为 10ms 基础帧
5. VAD 更新状态并给出 endpoint 候选
6. 每 40ms 小块送入 sherpa session
7. 每 160ms 左右拉取一次 partial
8. 静音或热键释放时拿 final
9. partial manager 生成草稿
10. final manager 生成 commit action
11. text commit 输出到宿主

推荐参数：

- 标准采样率：16k mono
- 基础帧：10ms = 160 samples
- ASR 喂入块：40ms = 640 samples
- partial polling：160ms
- ring buffer 容量：2s 到 3s
- pre-roll：200ms

线程模型：

- CPAL input callback：只负责采集并 push ring，不做识别逻辑。
- `audio_worker`：同步线程，负责重采样、帧化和音频预处理。
- `vad/asr_worker`：可先共线程，后续再按压力拆分。
- `text_commit_worker`：单独线程，避免宿主输入阻塞识别。
- `session_control`：主流程调度线程，处理事件与状态机。

并发建议：

- 热路径优先使用同步线程 + 有界队列。
- `tokio` 保留给模型下载、未来网络能力、日志聚合，不进入音频实时链路。

## 8. partial 与 final 交互策略

### 8.1 统一内部状态

- `stable_prefix`
- `unstable_suffix`
- `revision`
- `final_segments`
- `committed_len`

### 8.2 理想宿主：支持草稿替换

- partial 文本以草稿形式显示到宿主。
- final 到来时替换当前草稿区域，不重复插入。
- 只有 `stable_prefix` 增长，或 `unstable_suffix` 连续稳定 2 次以上时才更新。

优点：

- 最接近输入法 preedit 体验。
- final 与 partial 一致性最好。

缺点：

- 需要宿主或平台支持 selection replace / preedit。
- 当前项目输入层尚不具备此能力。

### 8.3 普通宿主：只支持简单插入

MVP 默认采用此策略：

- partial 只显示到状态栏或轻量浮层。
- final 到来时一次性提交。
- 若后续开启“稳定前缀渐进提交”，只能追加稳定前缀，不能依赖删除回改。

优点：

- 对 X11 / Wayland / macOS 辅助功能场景更稳。
- 不会因反复删改导致光标错乱。

缺点：

- 宿主输入框内的“边说边出字”体验有限。

### 8.4 中文输入提交原则

立即提交：

- 热键释放
- endpoint 命中
- 稳定句末标点已经出现

延迟提交：

- 数字 / 时间 / 中英夹杂仍在改写
- 尾音未稳定
- insert-only 宿主且当前片段回改概率仍高

## 9. VAD 与断句策略

推荐默认参数：

- `frame_ms = 20`
- `hop_ms = 10`
- `pre_roll_ms = 200`
- `min_speech_ms = 120`
- `endpoint_silence_ms = 550`
- `short_utterance_endpoint_ms = 420`
- `max_segment_ms = 12000`
- `partial_emit_interval_ms = 160`
- `stable_partial_hold_ms = 240`

策略要点：

- VAD 负责语音边界，ASR endpoint 负责 final 时机，两者协同。
- 中文短句停顿通常较短，静音阈值不应过大。
- 连续说话时允许多个 final segment，避免单句无限增长。
- 超过最长时长时强制切段，并保留少量尾上下文衔接下一段。

## 10. 性能优化方案

### 10.1 第一阶段必须做

- preload + warmup，避免首次热键冷启动。
- 常驻引擎 + 每次录音仅创建 session。
- 音频链路改为帧流，取消整段 clone + 重采样。
- 识别线程与采集线程隔离。
- partial 轮询节流，避免过高频 decode。
- LLM 默认退出热路径，仅作为可选后处理。
- 记录 `first_partial_ms`、`final_after_silence_ms`、`audio_backlog_ms`、`commit_ms`。

### 10.2 第二阶段再做

- 更优重采样器
- 更强 VAD（如 Silero / WebRTC）
- int8 / fp16 多模型档位
- host 级草稿替换
- GPU provider 选项

## 11. 迁移步骤与回滚策略

| 步骤 | 改动目标 | 预期产出 | 验证方式 | 回滚策略 |
| --- | --- | --- | --- | --- |
| 1 | 抽象现有 Whisper 引擎接口 | `AsrEngine` / `AsrSession` / `TextCommitBackend` | 编译通过，Whisper 行为不变 | 保留 legacy 入口 |
| 2 | 保留旧链路，新增 sherpa backend 空实现 | 可切换 backend 配置 | 基础启动与配置测试 | 切回 `backend=whisper` |
| 3 | 先做离线整段识别验证 | 固定 WAV 对照基线 | 中文短句正确率与时延对比 | 仅继续使用 Whisper final |
| 4 | 重构音频链路为 ring buffer + 帧流 | 新音频 pipeline | 不丢帧、不阻塞 | 保留旧 `AudioRecorder` 路径 |
| 5 | 接入 streaming session 与 partial 事件 | status 栏 partial | 首字与 partial 稳定性验证 | 关闭 streaming，只保留 batch final |
| 6 | 引入 final manager 与 insert-only 提交 | final 提交稳定 | 多宿主手工回归 | 回退为停录后一次性输入 |
| 7 | 开启 shadow mode 对比 Whisper | 精度/延迟对照日志 | 分析差异 | 关闭 shadow mode |
| 8 | 默认切换到 sherpa + SenseVoice | 新主链路上线 | 多日使用回归 | 配置一键回退 Whisper |

## 12. MVP 定义

MVP 应包括：

- 按快捷键开始录音
- 实时看到 partial（状态栏 / 浮层）
- 松开或静音后得到 final
- 将 final 稳定插入当前输入位置
- 中文普通话场景明显快于当前 Whisper 路径

MVP 不做：

- 宿主复杂草稿替换
- 命令词控制
- GPU provider
- LLM 热路径润色
- 多语言高级优化

## 13. 验收标准

- 首个 partial 延迟：
  - P50 <= 350ms
  - P95 <= 700ms
- 最后有效语音到 final：
  - P50 <= 500ms
  - P95 <= 900ms
- 热键释放到 final 提交宿主：
  - P50 <= 700ms
  - P95 <= 1200ms
- 资源占用：
  - 活跃识别时 CPU 常态 < 250%（6C/12T CPU-only 参考）
  - warm 后 RSS 目标 < 1.2GB，优先压到 800MB 以内
- partial 抖动控制：
  - UI 更新频率 <= 6Hz
  - 已展示的稳定前缀不回退
- 长句稳定性：
  - 连续说 60 秒不崩溃、不漏帧、不中断

## 14. 风险与坑点

- 流式结果不稳定：必须引入 `PartialResultManager` 去抖。
- partial/final 状态机混乱：内部状态需区分稳定前缀、未稳定后缀、final 段与已提交长度。
- 宿主输入兼容性差：insert-only 作为默认基线，草稿替换后续分阶段启用。
- 中文标点与英文空格异常：需要专门的 SenseVoice 后处理层。
- 音频线程阻塞导致漏帧：采集回调只 push ring，不做重采样和识别。
- 模型初始化过慢：必须 preload + warmup。

## 15. 代码骨架附录

```rust
pub enum RecognitionEvent {
    SessionStarted { session_id: u64 },
    Partial {
        session_id: u64,
        revision: u32,
        stable_prefix: String,
        unstable_suffix: String,
        raw_text: String,
    },
    Final {
        session_id: u64,
        segment_index: u32,
        text: String,
        reason: FinalizeReason,
    },
    Error {
        session_id: u64,
        message: String,
    },
}

pub struct RecognitionSession {
    pub id: u64,
    pub state: SessionState,
    pub partials: PartialResultManager,
    pub finals: FinalResultManager,
}

pub struct SherpaSenseVoiceEngine {
    cfg: SherpaOnnxConfig,
    caps: AsrCapabilities,
    runtime: std::sync::Arc<SherpaRuntime>,
}

pub trait TextCommitBackend: Send {
    fn supports_draft_replace(&self) -> bool;
    fn apply(&mut self, action: CommitAction) -> anyhow::Result<()>;
}
```

## 16. 关联文档

- `docs/adr/0004-streaming-asr-backend-migration-to-sherpa-sensevoice.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/architecture/performance-optimization-roadmap-v1.md`
- `docs/requirements/PRD.md`
- `docs/traceability/requirements-to-implementation.md`
