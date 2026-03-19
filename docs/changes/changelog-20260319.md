# 变更日志（20260319）

最后更新：2026-03-19

文件命名规则：`changelog-YYYYMMDD.md`。

## 功能变化

- 状态栏菜单与 TUI 15 项能力已完成对齐（见 `docs/architecture/status-bar-menu-sync-plan-v1.md`）。
- 保持后台运行场景下的可感知反馈能力（状态栏、通知、提示音）作为持续目标。

## 技术变化

- 设计与技术主文档统一为版本化命名：
  - `docs/design/system-design-v1.md`
  - `docs/architecture/technical-solution-v1.md`
- 专项技术文档归并至 `docs/architecture/`：
  - `status-bar-menu-sync-plan-v1.md`
  - `performance-optimization-roadmap-v1.md`
- 菜单能力同步优化（1-6）完成：
  - 共享业务内核：`src/menu_core.rs`、`src/model_download.rs`
  - 状态栏双向 IPC：`src/status_indicator.rs` + `src/main.rs` 主循环动作回传
  - 状态栏菜单与下载进度展示：动作映射、快照回推、下载日志显示
  - 自动化验收脚本：`scripts/run_acceptance.sh`
  - 性能 P0 聚合脚本：`scripts/perf_baseline.py`
  - 告警治理：`cargo test` 噪音告警收敛（含 `cargo-clippy` cfg 与 dead code 噪音）

## 文档变化

- 建立并确认文档基线目录：`requirements/design/architecture/adr/api/operations/changes/traceability/human-doc`。
- 完成模板文档补实：
  - `docs/requirements/BRD.md`
  - `docs/human-doc/BRD.md`
  - `docs/human-doc/PRD.md`
  - `docs/human-doc/TECH.md`
  - `docs/human-doc/USER-GUIDE.md`
  - `docs/human-doc/CHANGE-LOG.md`
- 更新导航与入口文档：
  - `docs/README.md`
  - `docs/SPEC.md`
  - `README.md`
- 完成“计划态 -> 实现态”文档收敛：
  - `docs/design/system-design-v1.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/adr/0002-status-bar-menu-sync-ipc.md`
  - `docs/adr/README.md`

## 规范化清理

- 已移除不符合规范的旧文档，并将有效信息迁移到规范路径：
  - `docs/PRD.md`
  - `docs/ECHOPUP_AGENT_SPEC.md`
  - `docs/design/system-design.md`
  - `docs/architecture/technical-solution.md`
  - `docs/changes/changelog.md`
  - `docs/changes/changelog-261903.md`
  - `docs/adr/0001-template.md`
  - `docs/STATUS_BAR_MENU_SYNC_PLAN.md`
  - `docs/PERFORMANCE_OPTIMIZATION_ROADMAP.md`

## 风险与回滚提示

- 风险：若后续迭代未同步更新 `SPEC`、`traceability` 与 `changes`，仍可能出现事实漂移。
- 回滚：可通过 Git 历史恢复已删除文档；建议优先在规范文档中修复并保持单一事实来源。

## 本轮增量（状态栏交互与触发模式收敛）

对应提交：`cc2eca5`

### 功能变化

- 新增热键触发模式菜单：
  - 长按模式（长按 1 秒开始，松开结束）
  - 按压切换模式（长按 1 秒开始，再按结束）
- 状态栏“退出”行为调整为退出主进程，而非仅关闭 UI。
- 热键编辑改为弹窗按键捕获，支持实时回显、撤销、确认。
- LLM 配置改为单弹窗表单统一编辑并自动保存。
- Whisper 模型切换、热键/开关变更均自动保存并立即生效。
- 下载模型改为弹窗流程：选择模型后在弹窗内持续展示进度与日志，完成/失败后确认关闭。

### 技术变化

- 配置新增 `hotkey.trigger_mode`，并在主流程中热更新。
- 录音触发链路重构为状态机，修复“松开后偶发立即结束”的边界不稳定问题。
- 右 Ctrl 监听改为按压计数模型，降低按键事件抖动。
- 状态栏视觉升级：
  - 空闲态仅展示 logo（无背景胶囊）
  - 激活态采用边缘脉动胶囊（内部不闪烁）
  - 状态栏占位改为空闲/激活自适应宽度
- 图标资源标准化：
  - `assets/logo.png` 调整为 1024x1024
  - `assets/mic.png` 透明边距重排并居中
- 下载稳健性增强：
  - 无进度超时延长与分段写入修正
  - 代理失败自动回退直连
  - 失败后清理 0B 临时文件

### 文档变化

- 更新：
  - `docs/SPEC.md`
  - `docs/design/system-design-v1.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/traceability/requirements-to-implementation.md`
- 新增：
  - `docs/adr/0003-hotkey-trigger-mode-and-adaptive-indicator.md`
