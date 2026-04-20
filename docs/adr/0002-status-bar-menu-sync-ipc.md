# ADR 0002 - 状态栏菜单与 TUI 功能同步采用“共享业务内核 + 双向 IPC”

日期：2026-03-19
状态：已采纳

## 背景

当前项目同时存在两个管理入口：
- 终端管理界面（`echopup ui` / `src/ui.rs`）
- macOS 状态栏指示器（`src/status_indicator.rs`）

随着状态栏交互能力增强，如果继续在两个入口分别实现菜单动作，容易出现：
- 配置行为不一致
- 下载逻辑重复与回归风险
- 后续迭代维护成本高

## 决策

采用以下架构策略：
1. 抽离共享菜单业务内核（动作执行、配置校验、下载任务状态）
2. 状态栏进程只负责展示与交互，不直接执行业务动作
3. 主进程与状态栏之间采用双向 IPC 传递动作请求、执行结果与状态快照

## 备选方案与取舍

1. 方案 A：TUI 与状态栏各自独立实现菜单逻辑  
   未选原因：重复代码多，行为易分叉，测试与回归成本高。

2. 方案 B：仅保留 TUI，状态栏只展示状态不提供菜单  
   未选原因：后台运行用户无法便捷管理配置与下载，交互效率低。

3. 方案 C：共享内核 + 双向 IPC（选定）  
   选择原因：职责清晰，单一真相源，便于扩展与回归测试。

## 影响

- 正向影响：
  - 管理动作一致性更高
  - 业务逻辑复用，维护成本下降
  - 状态栏可实现接近 TUI 的完整能力

- 负向影响：
  - 需要引入 IPC 协议与状态同步逻辑
  - 初期重构工作量较高

## 落地情况（2026-03-19）

- 共享业务内核已落地：
  - `src/menu_core.rs`
  - `src/model_download.rs`
- 状态栏双向 IPC 已落地：
  - `src/status_indicator.rs`（动作请求与结果回传）
  - `src/main.rs`（菜单动作执行与快照下发）
- TUI 已切换为共享内核：
  - `src/ui.rs`

## 后续约束

1. 新增菜单项时，先扩展 `MenuAction` 与 `MenuSnapshot`，再分别接入 TUI 与状态栏。
2. 状态栏只保留展示/交互职责，不直接承载业务规则。
3. 每次菜单相关改动需同步 `docs/changes/` 与 `docs/traceability/`。

## 关联文档

- `docs/architecture/status-bar-menu-sync-plan-v1.md`
- `docs/requirements/PRD.md`
- `docs/design/system-design-v1.md`
- `docs/architecture/technical-solution-v1.md`
