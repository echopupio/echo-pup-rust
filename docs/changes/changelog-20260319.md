# 变更日志（20260319）

最后更新：2026-03-19

文件命名规则：`changelog-YYYYMMDD.md`。

## 功能变化

- 状态栏菜单与 TUI 功能同步需求进入实施阶段（见 `docs/architecture/status-bar-menu-sync-plan-v1.md`）。
- 保持后台运行场景下的可感知反馈能力（状态栏、通知、提示音）作为持续目标。

## 技术变化

- 设计与技术主文档统一为版本化命名：
  - `docs/design/system-design-v1.md`
  - `docs/architecture/technical-solution-v1.md`
- 专项技术文档归并至 `docs/architecture/`：
  - `status-bar-menu-sync-plan-v1.md`
  - `performance-optimization-roadmap-v1.md`

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
