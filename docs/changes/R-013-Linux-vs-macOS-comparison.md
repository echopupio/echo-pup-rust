# R-013: Linux 与 macOS 状态栏实现对比

## 实现概述

### macOS 状态栏 (约 650 行核心逻辑)

| 模块 | 实现方式 | 说明 |
|------|----------|------|
| 托盘图标 | Cocoa NSStatusBar | 原生系统托盘 |
| 菜单 | Cocoa NSMenu | ObjC 运行时 |
| 弹窗 | 自定义 NSWindow | 热键捕获、LLM 表单、模型下载 |
| 事件循环 | NSApplication runLoop | Cocoa 事件驱动 |
| IPC | stdin/stdout 管道 | Parent/Child 消息 |
| 状态动画 | CALayer 脉冲效果 | 录音/识别中动态效果 |

### Linux 状态栏 (约 150 行核心逻辑)

| 模块 | 实现方式 | 说明 |
|------|----------|------|
| 托盘图标 | tray-icon crate | 跨平台托盘库 |
| 菜单 | muda crate | 跨平台菜单库 |
| 弹窗 | **未实现** | 依赖 macOS 自定义窗口 |
| 事件循环 | gtk::main_iteration_do | GTK 事件轮询 |
| IPC | stdin 管道 | 复用 Parent/Child 消息 |
| 状态动画 | **未实现** | 静态图标 |

---

## 功能对比矩阵

| 功能 | macOS | Linux (R-013) | 差异说明 |
|------|-------|---------------|----------|
| 状态显示 (Idle/Recording/Transcribing) | ✅ | ✅ | macOS 有动态图标，Linux 静态 |
| 状态栏标题 | ✅ | ✅ | 均显示状态中文 |
| LLM 开关 | ✅ | ✅ | |
| 文本纠错开关 | ✅ | ✅ | |
| VAD 开关 | ✅ | ✅ | |
| 录音触发模式切换 | ✅ | ✅ | |
| 编辑热键 (弹窗) | ✅ | ❌ | Linux 需实现 |
| 编辑 LLM 配置 (弹窗) | ✅ | ❌ | Linux 需实现 |
| 切换 Whisper 模型 | ✅ | ✅ | macOS 弹窗选择，Linux 子菜单 |
| 下载模型 (弹窗) | ✅ | ❌ | Linux 需实现 |
| 重载配置 | ✅ | ✅ | |
| 查看模型文件 | ✅ | ❌ | Linux 可用 notify/open |
| 打开配置文件夹 | ✅ | ❌ | Linux 可用 notify/open |
| 关于 | ✅ | ❌ | Linux 可跳过 |
| 退出 | ✅ | ✅ | |

---

## 代码结构差异

### macOS 核心架构

```
run_status_indicator_process()
    ├── spawn_stdin_reader()         → IPC 读取
    ├── create_menu()                → NSMenu 构建
    ├── NSStatusBar::systemStatusBar → 托盘图标
    └── runLoop (主循环)
        ├── menu_rx.try_recv()       → 处理菜单点击
        ├── rx.try_recv()            → 处理 IPC 消息
        ├── NSApp.nextEvent...      → Cocoa 事件
        └── 自动状态恢复定时器
```

### Linux 核心架构

```
run_status_indicator_process()
    ├── spawn_linux_stdin_reader()  → IPC 读取
    ├── build_linux_menu()          → muda::Menu 构建
    ├── TrayIconBuilder::new()      → 托盘图标
    └── gtk 主循环
        ├── muda::MenuEvent::recv   → 处理菜单点击
        ├── rx.try_recv()           → 处理 IPC 消息
        └── gtk::events_pending()   → GTK 事件
```

---

## 待完善功能 (Linux)

### 高优先级

1. **热键编辑弹窗** - 用户需要通过弹窗输入自定义热键
   - macOS 使用自定义 NSWindow
   - Linux 可考虑使用 gtk::Dialog 或简易命令行输入

2. **LLM 配置弹窗** - 用户需要输入 API Key 等配置
   - macOS 使用自定义 NSWindow + 表单
   - Linux 可考虑使用 gtk::Entry 或简易命令行输入

3. **模型下载弹窗** - 显示下载进度
   - macOS 使用自定义 NSWindow + 进度条
   - Linux 可考虑使用托盘菜单内联或 notify 通知

### 中优先级

4. **打开配置文件夹** - 使用 `xdg-open` 或 `notify-send`
5. **查看模型文件** - 使用文件管理器打开模型目录
6. **关于对话框** - 使用 `zenity` 或跳过

---

## 技术依赖

### Linux 系统依赖

```bash
sudo apt install libx11-dev libgtk-3-dev libayatana-appindicator3-dev libglib2.0-dev
```

### Linux Rust 依赖 (Cargo.toml)

```toml
[target.'cfg(target_os = "linux")'.dependencies]
tray-icon = "0.21"
muda = "0.14"
gtk = "0.18"
```

---

## IPC 协议 (共用)

### Parent → Child 消息

```rust
enum ParentMessage {
    SetState { state: IndicatorState },
    SetSnapshot { snapshot: MenuSnapshot },
    SetActionResult { result: MenuActionResult },
    Exit,
}
```

### Child → Parent 消息

```rust
enum ChildMessage {
    ActionRequest { action: MenuAction },
}
```

---

*本文档最后更新: 2026-03-23*
