# Wayland 兼容方案 v1 - 热键触发与文本提交

最后更新：2026-04-17

## 1. 背景

用户在 Linux/Wayland 桌面环境中使用 EchoPup 构建产物时，发现“应用自己识别全局热键”路径不可用，导致语音输入主链路无法稳定触发。本方案用于把该问题从“桌面偶发兼容性问题”升级为明确的产品与架构议题，并给出可实施、可验证、可回退的方案。

本方案只记录已核验事实与基于事实的实施建议，不把尚未实现的能力描述为既有事实。

## 2. 已核验事实

### 2.1 仓库当前实现事实

基于当前仓库代码（2026-04-17 检查）：

- 热键监听：
  - `Cargo.toml` 使用 `global-hotkey = "0.7"`
  - `src/hotkey/listener.rs` 中：
    - 常规热键默认走 `global-hotkey`
    - `right_ctrl` 特判走 `rdev::listen`
- 文本提交：
  - `src/input/keyboard.rs` 先尝试 `enigo`
  - Linux 下失败后回退到：
    - `xdotool`
    - `wtype`
- 文本提交抽象：
  - `src/commit/mod.rs` 已有 `TextCommitBackend`
  - 当前 `InsertOnlyTextCommit` 仍以“模拟键盘输入”作为统一实现基线

### 2.2 上游依赖事实

已核验的上游约束：

- `rdev` README 明确说明：Linux 的 `listen` 基于 X11，**不会在 Wayland 下工作**。
- `enigo` README 明确说明：
  - 默认 Linux 主路径仍以 X11 为主；
  - Linux Wayland / libei 路径存在，但为 **experimental**，且依赖 feature flag 与运行环境支持。
- `wtype` README 明确将自己定义为“`xdotool type for wayland`”，即 Wayland 下的文本注入工具路径。
- XDG Desktop Portal 文档中存在：
  - `org.freedesktop.portal.GlobalShortcuts`
  - `org.freedesktop.portal.RemoteDesktop`
- `RemoteDesktop` v2 文档明确存在：
  - `NotifyKeyboardKeycode`
  - `NotifyKeyboardKeysym`
  - `ConnectToEIS`

### 2.3 当前验证机器的环境事实

在当前验证环境执行过以下检查：

```bash
date '+%Y-%m-%d %H:%M:%S %z'
echo "$XDG_SESSION_TYPE"
echo "$XDG_CURRENT_DESKTOP"
gdbus introspect --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop
```

观察结果：

- 会话类型：`XDG_SESSION_TYPE=wayland`
- 桌面环境：`XDG_CURRENT_DESKTOP=ubuntu:GNOME`
- 已暴露 portal 接口：
  - `org.freedesktop.portal.RemoteDesktop`
  - `org.freedesktop.portal.InputCapture`
- 当前**未观察到** `org.freedesktop.portal.GlobalShortcuts`
- `RemoteDesktop` portal 版本：`2`
- `InputCapture` 的 `SupportedCapabilities` 返回值：`15`

> 说明：上面的 portal 观察结果只代表当前 Ubuntu GNOME Wayland 验证机，不等价于“所有 Wayland 桌面都如此”。

## 3. 问题归因

### 3.1 热键问题的根因

当前实现默认假设“应用自身可以直接全局监听键盘事件”。

该假设在 X11 下通常成立，但在 Wayland 下并不稳固，原因包括：

1. Wayland 的安全模型不鼓励普通应用直接全局窃听按键。
2. 当前用于 `right_ctrl` 的 `rdev::listen` 在 Linux 上明确依赖 X11。
3. `global-hotkey` 在 Wayland 下是否可用，取决于桌面环境、compositor 与实现路径；不能作为通用可靠基础。
4. 当前验证环境中没有暴露 `GlobalShortcuts` portal，因此应用不能依赖该 portal 作为当前 GNOME/Wayland 的唯一解法。

结论：

> 在 Wayland 下，把“应用自己监听全局热键”作为主路径并不优雅，也不可靠。

### 3.2 文本提交问题的根因

当前项目的文本提交仍以“模拟键盘输入”为中心。该路径在 Linux 上可以分裂成三类：

1. X11：`enigo` / `xdotool`
2. Wayland virtual keyboard：`wtype`
3. 更受控的 Wayland / portal / EIS / libei 路径：尚未在项目内落地

结论：

> 当前文本提交在 Wayland 下并非完全不可用，但缺少明确的平台策略、能力探测和后端优先级定义。

## 4. 设计目标

本轮 Wayland 方案的目标不是“实现所有桌面环境上的完美全局快捷键生态”，而是：

1. 让 EchoPup 在 Wayland 下具备**可用、优雅、可解释**的触发方式；
2. 不把 X11 假设继续硬套到 Wayland；
3. 把“触发入口”和“文本提交后端”抽象清楚，为后续 portal / libei / 输入法路线预留空间；
4. 保持 X11 与 macOS 现有能力可继续工作；
5. 形成一套能写进 README / Runbook / Traceability 的产品级事实基线。

## 5. 方案比较

### 方案 A：继续依赖应用内全局热键监听

做法：继续围绕 `global-hotkey` / `rdev` 修补。

不选原因：

- 与 Wayland 安全模型冲突；
- `rdev` 在 Linux/Wayland 下明确不成立；
- 即使局部桌面可用，也难形成稳定产品承诺；
- 会持续制造“某些发行版可用、某些不可用、原因不透明”的支持成本。

### 方案 B：优先采用 XDG GlobalShortcuts Portal

做法：如果运行环境提供 `org.freedesktop.portal.GlobalShortcuts`，由应用通过 portal 创建 session、绑定快捷键并接收 Activated/Deactivated。

优点：

- 方向正确，符合桌面平台边界；
- 比 compositor 私有接口更标准化；
- 语义上最接近“应用自己注册全局快捷键”。

限制：

- 当前 Ubuntu GNOME 验证机未暴露该接口；
- 不适合作为当前阶段唯一主路径；
- 仍需要无该 portal 时的 fallback。

### 方案 C：把快捷键绑定交给桌面环境 / compositor，应用只暴露触发接口（推荐主路径）

做法：

- EchoPup 提供显式触发命令或本地 IPC：
  - `echopup trigger press`
  - `echopup trigger release`
  - `echopup trigger toggle`
- GNOME / KDE / Sway / Hyprland 等由用户或安装器完成快捷键绑定
- 应用收到动作后复用当前录音状态机

优点：

- 符合 Wayland 思路；
- 跨 compositor 的通用性最好；
- 权责清晰：桌面环境负责热键，应用负责业务动作；
- 易于文档化与支持。

限制：

- 首次使用需要做一次快捷键绑定；
- “长按开始、松开结束”需要用 press/release 双动作或替代交互来表达；
- 用户体验不如 portal 原生注册那样“全自动”。

### 方案 D：GNOME/KDE/compositor 私有集成

做法：针对具体桌面环境分别做 shell extension、脚本生成器或配置写入。

定位：

- 适合作为后续增强层；
- 不适合作为当前通用基线。

## 6. 采纳方案

本方案采纳以下组合策略：

1. **Wayland 热键主路径**：采用“桌面环境绑定快捷键 -> EchoPup CLI/IPC 触发”的外部触发模式。
2. **Wayland 热键增强路径**：运行环境若存在 `GlobalShortcuts` portal，则允许后续增加 portal backend。
3. **X11 / macOS 现有路径**：保留当前 `global-hotkey` / `rdev` / 平台既有策略，不在本轮文档改造中删除。
4. **Wayland 文本提交主路径**：短期继续保留 `wtype` 作为 Wayland fallback。
5. **Wayland 文本提交增强路径**：后续评估 `libei` / `RemoteDesktop.ConnectToEIS` 是否能以可接受 UX 成本进入项目。
6. **长期方向**：若产品长期重点转向 Linux/Wayland，可进一步评估 IBus / Fcitx5 输入法集成，而不再把“模拟按键”作为唯一终局方案。

## 7. 架构改造建议

### 7.1 热键触发层改造

新增抽象目标：

- `TriggerSource` 或等价概念，区分“触发事件来源”而不是只关注“热键监听器”。

建议划分：

- `GlobalHotkeyTriggerBackend`
  - 服务于现有 X11 / macOS 路径
- `ExternalCommandTriggerBackend`
  - 服务于 Wayland 主路径
  - 接收 CLI 或本地 IPC 的 `press/release/toggle`
- `PortalGlobalShortcutsTriggerBackend`
  - 仅在检测到 portal 可用时启用

建议改造位置：

- `src/hotkey/listener.rs`：从“唯一热键监听器”向“现有全局热键 backend”收缩
- `src/main.rs`：录音状态机从“只接热键事件”改为“接统一触发事件”
- `src/config/config.rs`：允许记录 trigger backend 偏好或自动探测结果

### 7.2 文本提交层改造

当前 `TextCommitBackend` 已存在，可继续沿用，但要补齐平台策略：

建议后端策略：

- X11：`enigo` -> `xdotool`
- Wayland：`enigo(wayland feature)` 或 `wtype`
- Future：`libei` / portal-backed backend

建议改造位置：

- `src/input/keyboard.rs`
  - 显式区分 `XDG_SESSION_TYPE=x11|wayland`
  - 将 `wtype` 标记为 Wayland 明确后端，而不是“失败后的偶然 fallback”
- `src/commit/mod.rs`
  - 继续保持上层与具体注入方式解耦

### 7.3 能力探测与日志

增加能力探测项：

- `XDG_SESSION_TYPE`
- `XDG_CURRENT_DESKTOP`
- 是否存在 `org.freedesktop.portal.GlobalShortcuts`
- 是否存在 `org.freedesktop.portal.RemoteDesktop`
- `wtype` / `xdotool` 是否可执行

建议在启动日志中输出：

- trigger backend
- text commit backend
- portal capability summary

这将显著降低未来定位“为什么这台机器不工作”的成本。

## 8. 推荐实施阶段

### 阶段 1：文档基线与能力探测

目标：把事实说清楚，不再把 Wayland 当作 X11 兼容层。

交付：

- 文档基线更新（本次）
- 启动阶段记录会话类型、桌面环境、文本提交后端
- README / runbook 明确 Wayland 现状与建议使用方式

### 阶段 2：外部触发接口落地

目标：让 Wayland 用户能通过桌面快捷键稳定触发 EchoPup。

建议交付：

- CLI：
  - `echopup trigger press`
  - `echopup trigger release`
  - `echopup trigger toggle`
- 或本地 socket / DBus 命令接口
- GNOME / KDE / Sway / Hyprland 示例配置文档

验收标准：

- Wayland 会话下不依赖应用自监听全局按键，也能启动/停止录音
- 状态机与现有 `hold_to_record` / `press_to_toggle` 语义能映射清楚

### 阶段 3：portal backend 探测与可选接入

目标：在支持 `GlobalShortcuts` portal 的环境中提供更原生路径。

建议交付：

- portal 可用性探测
- backend 自动选择或手动配置
- 失败时自动回退到 external trigger 模式

### 阶段 4：Wayland 文本提交增强

目标：从“能打字”提升到“更稳定、更可解释”。

建议交付：

- 明确 `wtype` 为 Wayland 主 fallback
- 评估 `libei` / `RemoteDesktop.ConnectToEIS`
- 输出一轮兼容性矩阵：GNOME / KDE / Sway / Hyprland

### 阶段 5：长期演进（可选）

目标：评估输入法框架路线是否值得进入产品化阶段。

方向：

- IBus / Fcitx5
- preedit / commit text
- 不再完全依赖模拟按键

## 9. 验证与验收建议

### 9.1 最低验证矩阵

- Ubuntu GNOME / Wayland
- Ubuntu GNOME / X11
- 至少一种 wlroots 路线（Sway 或 Hyprland）

### 9.2 关键验证项

- 热键触发：
  - 应用内热键模式在 X11 下继续可用
  - Wayland 下 external trigger 可用
- 文本提交：
  - X11 下 `enigo` / `xdotool` 可用
  - Wayland 下 `wtype` 可用
- 观察性：
  - 日志能说明当前使用的 trigger backend 与 text backend

### 9.3 需避免的错误承诺

以下表述在文档和 README 中应避免：

- “Linux 热键与 X11/macOS 等价可用”
- “Wayland 下已支持应用内全局热键”
- “所有 GNOME/KDE/Hyprland/Sway 环境都可直接使用现有热键逻辑”

## 10. 风险与取舍

### 正向收益

- 让 Wayland 路线从“碰运气”变成“有主路径、有增强路径、有长期方向”。
- 将平台事实前移到文档与架构层，减少错误假设。
- 不阻塞现有 X11 / macOS 路线继续工作。

### 新增成本

- 需要维护额外的 trigger backend 抽象；
- 需要为不同桌面环境提供绑定指引；
- CLI/IPC 触发与长按语义的映射需要设计清楚。

## 11. 关联文件与建议改造点

- 热键监听：`src/hotkey/listener.rs`
- 主状态机：`src/main.rs`
- 配置：`src/config/config.rs`
- 文本输入：`src/input/keyboard.rs`
- 文本提交抽象：`src/commit/mod.rs`
- 用户文档：`README.md`

## 12. 关联文档

- `docs/requirements/PRD.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/changes/R-016-wayland-trigger-and-text-commit-compatibility.md`
- `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`
- `docs/operations/runbook.md`
- `docs/traceability/requirements-to-implementation.md`
