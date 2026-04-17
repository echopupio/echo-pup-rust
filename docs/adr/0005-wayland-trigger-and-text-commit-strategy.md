# ADR 0005 - Wayland 触发与文本提交采用“桌面绑定 + 外部触发”为主路径

日期：2026-04-17
状态：已采纳

## 背景

EchoPup 当前热键触发基于 `global-hotkey` 与 `rdev`，文本提交基于 `enigo`，Linux 下失败后回退到 `xdotool` / `wtype`。

经核验：

1. `rdev` 在 Linux 上的 `listen` 基于 X11，不支持 Wayland。
2. `enigo` 的 Wayland / libei 路径仍属 experimental。
3. 当前 Ubuntu GNOME Wayland 验证环境中存在 `RemoteDesktop` / `InputCapture` portal，但未观察到 `GlobalShortcuts` portal。

因此，“应用自己监听全局热键”不能作为 Wayland 主路径。

## 决策

1. 在 Wayland 下，EchoPup 的推荐触发模式改为：
   - **桌面环境或 compositor 绑定快捷键**
   - **EchoPup 通过 CLI/IPC 接收 `press/release/toggle` 触发动作**
2. `GlobalShortcuts` portal 若在运行环境中可用，可作为后续增强 backend；但不作为当前唯一依赖。
3. X11 / macOS 现有热键实现保留，不因本决策直接废弃。
4. Wayland 文本提交短期继续采用 `wtype` 作为明确 fallback，后续再评估 `libei` / `RemoteDesktop.ConnectToEIS`。
5. 长期若 Linux/Wayland 成为核心使用面，再评估输入法框架（IBus / Fcitx5）路线。

## 备选方案与取舍

### 方案 A：继续强化 `global-hotkey` / `rdev`

未选原因：

- 与 Wayland 安全边界不一致；
- `rdev` 在 Linux/Wayland 下已知不可用；
- 即使局部可工作，也无法给出稳定产品承诺。

### 方案 B：仅依赖 `GlobalShortcuts` portal

未选原因：

- 当前验证环境未暴露该接口；
- 桌面环境覆盖不完整；
- 缺乏 portal 不可用时的 fallback。

### 方案 C：直接转向输入法框架

未选原因：

- 路线更重，实施周期明显更长；
- 当前阶段先解决“可触发、可输入、可解释”的产品问题更重要。

## 影响

### 正向影响

- Wayland 主路径更加符合平台边界；
- 用户能通过桌面快捷键稳定使用 EchoPup，而不是依赖应用偷听全局输入；
- 为 portal / libei / IME 路线预留了可演进边界。

### 负向影响

- 需要额外设计 CLI/IPC 触发接口；
- 用户初次配置桌面快捷键的门槛会上升；
- 文档、runbook、README 必须同步说明平台差异。

## 落地位置

- 触发架构：
  - `src/hotkey/listener.rs`
  - `src/main.rs`
  - `src/config/config.rs`
- 文本提交：
  - `src/input/keyboard.rs`
  - `src/commit/mod.rs`
- 文档：
  - `docs/architecture/wayland-compatibility-plan-v1.md`
  - `docs/requirements/PRD.md`
  - `docs/operations/runbook.md`
  - `README.md`

## 后续约束

1. 任何“Wayland 已支持什么”的对外表述，都必须以实际 backend 能力和验证结果为准。
2. 引入 portal 或 libei backend 时，必须同时提供能力探测、失败回退与日志说明。
3. 不得再把 `rdev` 的 Linux 能力描述为对 Wayland 通用有效。

## 关联文档

- `docs/architecture/wayland-compatibility-plan-v1.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`
- `docs/traceability/requirements-to-implementation.md`
