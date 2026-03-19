# 文档索引

最后更新：2026-03-19

## 基线文档（主入口）

- `requirements/BRD.md` - 业务目标、边界与价值。
- `requirements/PRD.md` - 产品需求、范围与验收标准。
- `design/system-design-v1.md` - 系统行为、流程与组件设计。
- `architecture/technical-solution-v1.md` - 架构约束、选型与技术策略。
- `adr/` - 架构决策记录。
- `api/README.md` - 接口契约与版本兼容策略。
- `operations/runbook.md` - 发布、回滚与故障处置流程。
- `changes/changelog-20260319.md` - 本周期关键变更记录。
- `traceability/requirements-to-implementation.md` - 需求到实现与验证追踪。

## 专题文档

- `architecture/status-bar-menu-sync-plan-v1.md` - 状态栏菜单与 TUI 功能同步实施方案。
- `architecture/performance-optimization-roadmap-v1.md` - 性能优化路线图。
- `adr/0003-hotkey-trigger-mode-and-adaptive-indicator.md` - 热键双模式与状态栏自适应占位决策。

## 人类可读文档

- `human-doc/BRD.md`
- `human-doc/PRD.md`
- `human-doc/TECH.md`
- `human-doc/USER-GUIDE.md`
- `human-doc/CHANGE-LOG.md`

## 历史文档策略

- 旧版不规范文档已清理，不再并列保留。
- 历史版本统一通过 Git 提交历史追溯。

## 使用规则

1. 代码变更应在同一 PR 中同步更新相关文档。
2. 需求、设计、技术、变更四层文档应保持可追踪。
3. 设计与技术文档使用 `vN` 命名；变更日志使用 `YYYYMMDD` 命名。
4. 当文档路径变化时，必须同步更新本索引与 `docs/SPEC.md`。
