# EchoPup 性能优化路线图 v1（2026-03）

最后更新：2026-03-31

## 1. 目标

在不明显牺牲准确率的前提下，持续降低“首个 partial 出现时间”“最后有效语音到 final 的时间”“松键到出字的体感延迟”，并保证后台稳定性。

目标分层：
- `T0`：主观响应更快
- `T1`：P50 < 1.0s，P95 < 1.8s（不含远程 LLM）
- `T2`：进入可用的流式体验（首字延迟显著下降）
- `T3`：具备“partial 草稿 + final 最终结果”的稳定双阶段输出

## 2. 当前状态盘点

### 2.1 已完成

- 已支持性能档位：`accurate / balanced / fast`
- 已支持线程自动模式：`n_threads = "auto"`
- 已有关键耗时埋点日志：`stt_ms`、`llm_ms`、`postprocess_ms`、`type_ms`、`e2e_ms`
- 模型下载具备稳定性机制：断点续传、无进度超时失败、自动重试
- 已完成流式 ASR 迁移方案与 ADR 沉淀，进入实施前准备阶段

### 2.2 进行中

- 状态栏菜单与 TUI 管理能力同步（见 `docs/architecture/status-bar-menu-sync-plan-v1.md`）
- 后台可感知反馈增强（状态栏 + 通知 + 提示音）

### 2.3 待改进

- 缺少标准化聚合报表（按机型/配置自动统计 P50/P95）
- LLM 路径仍为串行阻塞，长尾场景会抬高总体延迟
- 当前 partial 仍依赖对音频快照反复重跑，未形成真正的流式 session
- 音频缓冲当前以整段 `Vec<f32>` clone + 重采样为主，不适合低延迟热路径

## 3. 后续优化方向

### P0：观测与基线（短期，实施前）

- 增加日志聚合脚本，自动汇总 P50/P95
- 建立“机型 × 档位 × 延迟”基线表
- 新增 `first_partial_ms`、`final_after_silence_ms`、`audio_backlog_ms` 指标
- 当前实现：`scripts/perf_baseline.py`
  - 近 N 次统计：`./scripts/perf_baseline.py --limit 200`
  - 基线导出：`./scripts/perf_baseline.py --profile balanced --export-csv ./artifacts/perf-baseline.csv`

验收：
- 一条命令可导出近 N 次识别延迟统计
- 不同档位延迟与准确率可对比

### P1：引擎抽象与常驻模型（短中期）

- 提炼 `AsrEngine` / `AsrSession` / `TextCommitBackend`
- 模型 preload + warmup，避免首次热键冷启动
- 保留 Whisper 回退能力，并为 sherpa backend 预留切换开关

验收：
- 在不改变用户现有操作方式的情况下，引擎接口可切换
- 冷启动首轮录音不再受模型加载阻塞

### P2：流式链路重构（中期）

- 将音频链路改为 ring buffer + 16k 帧流
- 引入 streaming session、partial/final 双阶段输出
- partial 先稳定在状态栏/浮层，不直接要求宿主草稿替换

验收：
- 首字延迟明显下降
- final 输出稳定，不出现重复上屏

### P3：LLM 路径降级/异步化（中期）

- 提供“速度优先”模式：默认直接输出 STT + 规则纠错结果
- LLM 可异步补写或仅在显式开启时阻塞执行

验收：
- 默认路径长尾显著下降
- LLM 开关策略对用户可理解且可控

### P4：宿主草稿替换与高级体验（中长期）

- 按宿主能力增加 draft replace
- 探索“稳定前缀渐进提交”与真正 preedit 的兼容层

验收：
- 草稿替换对普通宿主有稳定降级路径
- 不因回改导致光标错乱或重复输入

## 4. 风险与取舍

- 速度提升与准确率存在天然权衡，需要档位化管理预期
- LLM 异步化会引入一致性问题（是否回写、何时覆盖）
- 流式方案复杂度高，必须先完成边界抽象与 shadow mode，再主链路切换

## 5. 关联文档

- `docs/requirements/PRD.md`
- `docs/design/system-design-v1.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/architecture/streaming-asr-migration-plan-v1.md`
- `docs/architecture/status-bar-menu-sync-plan-v1.md`
