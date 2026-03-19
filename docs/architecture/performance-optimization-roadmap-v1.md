# EchoPup 性能优化路线图 v1（2026-03）

最后更新：2026-03-19

## 1. 目标

在不明显牺牲准确率的前提下，持续降低“松键到出字”的体感延迟，并保证后台稳定性。

目标分层：
- `T0`：主观响应更快
- `T1`：P50 < 1.0s，P95 < 1.8s（不含远程 LLM）
- `T2`：逐步靠近流式体验（首字延迟显著下降）

## 2. 当前状态盘点

### 2.1 已完成

- 已支持性能档位：`accurate / balanced / fast`
- 已支持线程自动模式：`n_threads = "auto"`
- 已有关键耗时埋点日志：`stt_ms`、`llm_ms`、`postprocess_ms`、`type_ms`、`e2e_ms`
- 模型下载具备稳定性机制：断点续传、无进度超时失败、自动重试

### 2.2 进行中

- 状态栏菜单与 TUI 管理能力同步（见 `docs/architecture/status-bar-menu-sync-plan-v1.md`）
- 后台可感知反馈增强（状态栏 + 通知 + 提示音）

### 2.3 待改进

- 缺少标准化聚合报表（按机型/配置自动统计 P50/P95）
- LLM 路径仍为串行阻塞，长尾场景会抬高总体延迟
- 尚未引入流式转写链路

## 3. 后续优化方向

### P0：观测与基线（短期）

- 增加日志聚合脚本，自动汇总 P50/P95
- 建立“机型 × 档位 × 延迟”基线表
- 当前实现：`scripts/perf_baseline.py`
  - 近 N 次统计：`./scripts/perf_baseline.py --limit 200`
  - 基线导出：`./scripts/perf_baseline.py --profile balanced --export-csv ./artifacts/perf-baseline.csv`

验收：
- 一条命令可导出近 N 次识别延迟统计
- 不同档位延迟与准确率可对比

### P1：LLM 路径降级/异步化（中期）

- 提供“速度优先”模式：默认直接输出 STT + 规则纠错结果
- LLM 可异步补写或仅在显式开启时阻塞执行

验收：
- 默认路径长尾显著下降
- LLM 开关策略对用户可理解且可控

### P2：流式识别 PoC（中长期）

- 评估分段推理与增量输出
- 探索“先草稿后修正”的体验设计

验收：
- 首字延迟明显下降
- 最终文本质量不劣于当前离线方案

## 4. 风险与取舍

- 速度提升与准确率存在天然权衡，需要档位化管理预期
- LLM 异步化会引入一致性问题（是否回写、何时覆盖）
- 流式方案复杂度高，建议先 PoC 再主链路替换

## 5. 关联文档

- `docs/requirements/PRD.md`
- `docs/design/system-design-v1.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/architecture/status-bar-menu-sync-plan-v1.md`
