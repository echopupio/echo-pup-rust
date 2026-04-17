# 变更日志（20260417）

最后更新：2026-04-17

文件命名规则：`changelog-YYYYMMDD.md`。

## 本轮主题

- 基于真实代码与当前 Ubuntu GNOME Wayland 环境核验，为 EchoPup 沉淀 Wayland 热键触发与文本提交兼容方案。
- 将 Wayland 议题从“零散兼容性现象”提升为正式需求、架构方案与 ADR。
- 使用 doc-doc 文档治理思路回写项目文档基线，补齐索引、追踪、运维与人类可读文档映射。

## 文档变化

- 新增架构方案：
  - `docs/architecture/wayland-compatibility-plan-v1.md`
- 新增 ADR：
  - `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- 新增需求条目：
  - `docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`
- 更新基线文档：
  - `docs/README.md`
  - `docs/SPEC.md`
  - `docs/requirements/BRD.md`
  - `docs/requirements/PRD.md`
  - `docs/architecture/technical-solution-v1.md`
  - `docs/traceability/requirements-to-implementation.md`
  - `docs/operations/runbook.md`
  - `docs/setup/environment-resources.md`
  - `docs/human-doc/TECH.md`
  - `docs/reports/project-court-ledger.md`
  - `docs/PROMPT-QA-LOG.md`
  - `README.md`

## 事实结论摘要

- 当前代码中：
  - 热键监听主要依赖 `global-hotkey` 与 `rdev`
  - 文本提交主要依赖 `enigo`，Linux 下回退 `xdotool` / `wtype`
- 已核验：
  - `rdev` Linux `listen` 基于 X11，不支持 Wayland
  - `enigo` 的 Wayland / libei 路径仍属 experimental
  - `wtype` 是 Wayland 文本提交的现实路径之一
- 当前验证环境（Ubuntu GNOME / Wayland）观测到：
  - 有 `RemoteDesktop` / `InputCapture` portal
  - 未观察到 `GlobalShortcuts` portal

## 决策摘要

- Wayland 下不再把“应用自己监听全局热键”作为主路径。
- 推荐主路径改为：
  - 桌面环境 / compositor 绑定快捷键
  - EchoPup 提供 CLI / IPC 外部触发接口
- Wayland 文本提交短期继续保留 `wtype` 作为明确 fallback。
- `GlobalShortcuts` portal 与 `libei` 路线保留为后续增强方向。

## 后续动作

- 增加 trigger backend 抽象。
- 为 Wayland 增加 `press/release/toggle` 外部触发入口。
- 在启动日志中输出会话类型、portal 能力、trigger backend、text commit backend。
- 补 GNOME / KDE / Sway / Hyprland 的绑定文档或示例。
