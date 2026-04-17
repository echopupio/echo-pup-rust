# 项目朝议总账

本账由司徒主理，用于持续汇总项目进展、风险、阻塞、遗留事项与建议下一步。此处记录的是“当前局势”，不是 ADR，也不是逐行变更日志。

## 项目概览

- 项目名称：EchoPup (`echo-pup-rust`)
- 当前阶段：中文流式 ASR 迁移实施中，已完成边界抽象、增量音频读口与 backend 切换骨架
- 丞相：主代理（当前会话）
- 最近一次巡检：2026-03-31 10:53:00 +0800
- 当前技术栈：Rust；macOS / Linux 桌面语音输入工具

## 当前总评

- 进展摘要：`AsrEngine` / `AsrSession` / `TextCommitBackend` 已落地；`RecognitionSession` 已落地；音频热路径已具备增量读口；preview 与 final 都已切到统一 session 边界；配置已支持 `asr.backend = whisper | sherpa_sensevoice`，且 sherpa 失败可回退 Whisper。
- 主要风险：当前 sherpa SenseVoice 仍是 `OfflineRecognizer + incremental session` 过渡实现，尚未用真实模型与真实口述回归证明“明显快于 Whisper”。
- 主要阻塞：仓库尚未完成 sherpa 模型文件与 tokens 的真实接线验证；`main.rs` 仍承担过多 orchestration；尚未沉淀首字/句末延迟指标；Wayland 触发路径仍缺正式主方案实现。
- 下一步建议：优先完成真实 sherpa model smoke test、固定 WAV/手工回归与延迟指标采集，并推进 Wayland“桌面快捷键绑定 + 外部触发”主路径实施。

## 阶段状态

| 官员 | 当前状态 | 最近产物 | 下一步 |
| --- | --- | --- | --- |
| 丞相 | 在值 | 新版本周天子重开朝会 | 汇总三官回报并等候议题 |
| 大司礼 | 在值 | 文档与实施进度同步 | 继续把实现现状、风险和下一步回写到变更日志、总账、追踪矩阵 |
| 司空 | 在值 | `AsrEngine` / `AsrSession` / backend fallback 架构落地 | 下一步收缩 `main.rs` 编排，继续推进 session_control 与 metrics |
| 大司马 | 在阵 | 增量音频读口、session 化 preview/final、sherpa backend 骨架 | 下一步接真实 SenseVoiceSmall 模型并做口述回归 |
| 御史 | 待召 | 无 | 待出现实现或规则变更后纠察 |
| 司寇 | 待召 | 无 | 待进入固定 WAV / 手工录音 / 延迟指标验证后介入 |
| 司徒 | 在值 | 本账 | 持续跟踪迁移实施状态与下一阶段阻塞 |
| 太史 | 待召 | 无 | 待出现需长期沉淀事项后著录 |
| 少府 | 已回报 | 工具链与技能接缝巡看 | 已建议补 wrapper 或固定入口约定；历史 `.zhou-tianzi/` 残留已清理 |

## 风险台账

| 编号 | 等级 | 描述 | 责任官员 | 应对措施 | 状态 |
| --- | --- | --- | --- | --- | --- |
| R-001 | 低 | 已清理历史 `.zhou-tianzi/` 目录残影、根目录旧空目录和 ADR 候选冲突件，遗留混淆已解除 | 少府 / 大司礼 | 保持“已安装技能为准”的仓库规则，不再恢复旧载体 | 关闭 |
| R-002 | 中 | 治理动作仍依赖外部技能安装路径，项目内缺少稳定 wrapper 或统一入口约定 | 少府 | 评估新增项目内 wrapper，或约定统一环境变量/固定安装路径 | 打开 |
| R-003 | 中 | 大模型下载体验仍受限于当前静态并发 / 串行续传机制，尚未达到 aria2 风格高速下载目标 | 少府 / 大司马 | 已立项需求 `R-014`，后续按“分片级恢复 + 动态调度 + 稳定降级”推进 | 打开 |
| R-004 | 高 | 流式 ASR 迁移涉及音频热路径、partial/final 状态机与宿主输入兼容性，若边界不清会导致大范围回归 | 司空 / 大司马 / 司寇 | 先实施引擎抽象与 insert-only 基线，再分阶段接入 streaming、shadow mode 和宿主增强 | 打开 |
| R-005 | 高 | sherpa SenseVoice 当前仅以 offline recognizer 包装进 session，若官方 Rust API 侧缺少在线 SenseVoice 支持，则“真流式”能力可能需要改技术路线或继续过渡方案 | 司空 / 大司马 | 先验证现有 wrapper 的时延收益；并确认是否存在可用在线 API、其他 backend 或 FFI 方案 | 打开 |
| R-006 | 中 | 当前 backend 切换已支持配置化回退，但尚未通过真实模型文件与真实录音验证回退路径是否满足用户体验 | 大司马 / 司寇 | 准备 sherpa 模型目录，执行本机实测与回退验证 | 打开 |
| R-007 | 高 | Linux/Wayland 下若继续沿用应用内全局热键监听，会持续与平台安全边界冲突，造成“某些桌面可用、某些不可用、原因不透明”的支持成本 | 司空 / 大司马 / 司寇 | 采用“桌面快捷键绑定 + EchoPup 外部触发接口”作为主路径，并补能力探测与运行日志 | 打开 |

## 阻塞清单

| 编号 | 描述 | 依赖项 | 当前处理人 | 状态 |
| --- | --- | --- | --- | --- |
| B-001 | sherpa backend 已可编译，但尚未准备真实 `model.onnx` / `tokens.txt` 并完成本机冒烟 | 模型文件到位、配置切换、录音回归 | 大司马 / 司寇 | 打开 |
| B-002 | `main.rs` 仍承载录音生命周期和 session orchestration，后续继续扩展容易失控 | 抽离 `session_control`、收束 preview/final 线程编排 | 司空 / 大司马 | 打开 |
| B-003 | 尚未建立延迟指标记录与固定 WAV 基线，无法判断 sherpa 过渡实现是否达到切换条件 | 增加 perf logging / baseline 样例集 | 大司马 / 司寇 | 打开 |
| B-004 | Wayland 路径尚未实现正式 external trigger 入口与 backend 探测，用户仍需依赖当前不稳定的应用内热键方案 | trigger backend 抽象、CLI/IPC 入口、README/runbook 指引 | 司空 / 大司马 | 打开 |

## 巡检记录

### 初始化基线

- 时间：2026-03-27 13:38:44 +0800
- 结论：已按新版本周天子重新上朝并补回治理骨架，三官巡看已完成，旧版残留已清，当前待定稿并等候议题。
- 备注：本次通过临时符号链接完成初始化；少府复核后确认新版本脚本已修正 `/Users/<name>/<project>` 误判，后续可直接使用已安装技能中的巡检脚本追加条目。

### 中文流式 ASR 迁移方案沉淀

- 时间：2026-03-31 09:45:01 +0800
- 结论：已将 `Whisper -> sherpa-onnx + SenseVoiceSmall` 迁移方案沉淀到架构文档、ADR、PRD/BRD、性能路线图、追踪矩阵与索引，明确“先边界后替换、先 insert-only 后草稿替换、先保留 Whisper 回退”的实施原则。
- 备注：当前仍为文档决策与规划阶段，后续需按方案依次进入 `AsrEngine` 抽象、音频帧流重构、sherpa backend 接入与 shadow mode 验证。

### 中文流式 ASR 迁移实施推进

- 时间：2026-03-31 10:53:00 +0800
- 结论：已完成 `AsrEngine` / `AsrSession` / `TextCommitBackend` 抽象，`RecognitionSession` 与 partial/final manager 已落地；`AudioRecorder` 已支持 recent buffer 与增量读口；preview 与 final 已统一走 session；配置新增 `asr.backend` 与 `asr.sherpa.*`，并已接入 sherpa backend 骨架与 Whisper 回退。
- 备注：
  - 当前 sherpa 实现为 `OfflineRecognizer + incremental session` 过渡方案。
  - 下一阶段必须先完成真实模型与真实录音验证，再决定默认 backend 是否可切换。
  - 需要重点记录的后续动作：`session_control` 抽离、固定 WAV 基线、时延指标采集、sherpa 真流式能力核验。

### Wayland 触发与文本提交方案沉淀

- 时间：2026-04-17 10:42:42 +0800
- 结论：已基于当前代码、上游依赖说明与 Ubuntu GNOME Wayland 验证环境，沉淀 Wayland 兼容方案与 ADR。明确 EchoPup 在 Wayland 下不应再把“应用自己监听全局热键”作为主路径，而应采用“桌面快捷键绑定 + 外部触发接口”；文本提交短期继续保留 `wtype` 作为 Wayland fallback。
- 备注：
  - 当前验证环境可见 `RemoteDesktop` / `InputCapture` portal，但未观察到 `GlobalShortcuts` portal。
  - 后续需实现 trigger backend 抽象、CLI/IPC 触发入口、能力探测日志与 README/runbook 指引。
