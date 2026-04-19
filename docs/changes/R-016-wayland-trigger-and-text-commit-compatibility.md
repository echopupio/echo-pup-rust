# R-016 Wayland 热键触发与文本提交兼容方案

最后更新：2026-04-18
状态：部分完成（Linux Wayland Phase 1 已落地）

## 1. 需求背景

用户在 Linux/Wayland 桌面环境中使用 EchoPup 时，发现现有“应用自己监听全局热键”的实现无法稳定工作，导致语音输入链路无法顺畅触发。

当前问题不是单一 bug，而是平台边界变化带来的架构问题：Wayland 不再默认允许普通应用像 X11 那样全局监听输入。

## 2. 目标

- 让 EchoPup 在 Wayland 下具备稳定、优雅、可解释的触发方式。
- 让 Wayland 文本提交路径具备明确的后端选择与排障说明。
- 保持 X11 / macOS 现有能力继续工作。
- 为后续 portal / libei / IME 路线预留扩展点。

## 3. 已核验事实摘要

- 当前代码：
  - 热键：`global-hotkey` + `rdev`
  - 文本输入：`enigo`，Linux 下回退 `xdotool` / `wtype`
- 上游事实：
  - `rdev` Linux `listen` 依赖 X11，不支持 Wayland
  - `enigo` Wayland / libei 仍属 experimental
  - `wtype` 是 Wayland 下典型文本注入路径
- 当前验证环境事实：
  - `XDG_SESSION_TYPE=wayland`
  - `XDG_CURRENT_DESKTOP=ubuntu:GNOME`
  - 可见 `RemoteDesktop` / `InputCapture` portal
  - 未观察到 `GlobalShortcuts` portal

## 4. 需求范围

### 必须有

- R-016.1：启动时识别会话类型与关键能力（Wayland/X11、portal、命令后端）
- R-016.2：提供外部触发入口（CLI 或 IPC）以承接 Wayland 快捷键绑定
- R-016.3：Wayland 下文本提交优先走明确的 Wayland 兼容路径，而不是仅依赖 X11 假设
- R-016.4：README / runbook / traceability 明确说明 Wayland 的主路径、限制与排障方法

### 应该有

- R-016.5：若运行环境支持 `GlobalShortcuts` portal，则允许后续增加 portal backend
- R-016.6：日志明确打印 trigger backend、text commit backend、关键能力探测结果

### 暂不做

- R-016.7：首轮不承诺所有 compositor 的应用内全局热键原生支持
- R-016.8：首轮不把 IBus / Fcitx5 输入法集成立即纳入实现范围

## 5. 推荐方案

### 热键触发

采用“**桌面绑定快捷键 -> EchoPup 外部触发接口**”作为 Wayland 主路径。

建议接口：

- `echopup trigger press`
- `echopup trigger release`
- `echopup trigger toggle`

### 文本提交

- X11：`enigo` -> `xdotool`
- Wayland：`wtype` 作为明确 fallback
- Future：`libei` / `RemoteDesktop.ConnectToEIS`

### 能力探测

建议在启动时记录：

- `XDG_SESSION_TYPE`
- `XDG_CURRENT_DESKTOP`
- `GlobalShortcuts` portal 是否存在
- `RemoteDesktop` portal 是否存在
- `wtype` / `xdotool` 是否存在

## 6. 验收标准

| 需求 ID | 验收标准 | 验证方式 |
| --- | --- | --- |
| R-016.1 | 能正确识别 X11 / Wayland 会话并输出后端与能力摘要 | 手工运行 + 日志检查 |
| R-016.2 | Wayland 下通过桌面快捷键绑定 CLI/IPC 后可稳定触发录音状态机 | GNOME / 其他 compositor 手工回归 |
| R-016.3 | Wayland 文本提交能明确使用 `wtype` 或其他已选后端，不再仅靠隐式 fallback | 手工回归 + 日志检查 |
| R-016.4 | README、runbook、traceability 与架构文档对 Wayland 路线表述一致 | 文档 review |
| R-016.5 | 若 portal 可用，backend 能识别并在失败时回退到 external trigger | 具备 portal 的环境手工回归 |

## 7. 关联文档

- `docs/architecture/wayland-compatibility-plan-v1.md`
- `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- `docs/requirements/PRD.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/operations/runbook.md`

## 8. 当前实现状态（2026-04-18）

### 已落地

- Linux Wayland 启动路径已切换为“桌面快捷键 -> `echopup trigger ...` -> 后台服务”的外部触发架构。
- 新增 `echopup trigger press|release|toggle` 子命令，Linux 后台服务通过本地 Unix socket 接收外部触发动作。
- Linux Wayland 会话下，运行时不再创建应用内全局热键监听器，避免继续依赖 X11 风格的键盘监听。
- GNOME Wayland 下，首次启动 `echopup` / `echopup start` 时会尝试自动创建系统快捷键：
  - `F6 -> echopup --config ~/.echopup/config.toml trigger toggle`
- Linux 默认触发键调整为 `F6`；macOS 现有 `ctrl` 逻辑保持不变。
- Linux 后台启动已改为独立会话启动，避免 `start` 拉起的后台进程被父终端或父进程生命周期连带终止。
- `doctor` 已补充 Wayland 会话说明，明确当前 trigger backend 与推荐快捷键绑定方式。

### 当前语义

- Linux Wayland 默认外部快捷键采用 `toggle` 语义。
- `hold_to_record` 仅在 `trigger press/release` 组合被调用时才能完整表达；普通 GNOME 自定义快捷键默认只适合 `toggle`。

### 已知边界

- `GlobalShortcuts` portal backend 仍未接入，当前实现优先保证 GNOME Wayland 可落地而不是追求 portal 优先。
- Linux 托盘图标显示仍受桌面环境与托盘支持情况影响，不属于本次 Wayland 触发链路改造的阻塞项。
