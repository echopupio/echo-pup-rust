# EchoPup

AI 语音输入工具 - 支持长按模式与按压切换模式，自动识别并输入文本

## 安装

### macOS (Homebrew)

```bash
brew install pupkit-labs/tap/echopup
```

### Linux (Shell Installer)

```bash
curl -fsSL https://raw.githubusercontent.com/pupkit-labs/echo-pup-rust/main/install.sh | bash
```

或指定版本：

```bash
curl -fsSL https://raw.githubusercontent.com/pupkit-labs/echo-pup-rust/main/install.sh | bash -s -- v0.0.1
```

### 从源码编译

```bash
cargo install --git https://github.com/pupkit-labs/echo-pup-rust.git
```

## 更新

```bash
# 内置更新命令（macOS 自动走 brew，Linux 自动下载替换）
echopup update

# 或手动更新
# macOS
brew upgrade pupkit-labs/tap/echopup

# Linux
curl -fsSL https://raw.githubusercontent.com/pupkit-labs/echo-pup-rust/main/install.sh | bash
```

## 功能特性

- 🎤 **语音输入** - 支持两种触发模式：长按模式 / 按压切换模式
- 🔄 **语音识别** - 使用本地 sherpa-onnx Paraformer 模型进行流式语音转文字
- ✨ **智能整理** - 可选 LLM 自动润色转写文本
- ⌨️ **自动输入** - 自动模拟键盘输入到当前应用
- ⌨️ **固定热键** - 使用 Control 键触发（左右 Ctrl 均可）
- 🧭 **状态栏管理** - LLM 表单、模型切换与下载弹窗
- 💾 **自动生效** - 菜单改动自动保存配置并立即热更新
- 🟠 **多通道反馈** - 状态栏、系统通知、提示音

## 环境要求

- macOS 或 Linux（需图形会话）
- Rust 1.70+
- Linux 常用依赖：
  - `pkg-config`
  - `libssl-dev`
  - `libasound2-dev`
  - `libnotify-bin`（桌面通知）
  - `pulseaudio-utils` 或 `alsa-utils`（提示音）
- macOS 需授予：
  - 麦克风权限
  - 辅助功能权限（用于模拟键盘输入）
  - 通知权限（通知来源通常显示为“脚本编辑器”）

## 快速开始

### 1. 安装依赖（Linux 示例）

```bash
sudo apt install pkg-config libssl-dev libasound2-dev libnotify-bin pulseaudio-utils
```

### 2. 下载语音识别模型

```bash
# 自动下载 Paraformer ASR 模型和标点恢复模型到 ~/.echopup/models/
echopup model
```

### 3. 编译运行

```bash
# 编译
cargo build --release

# 测试各模块
./target/release/echopup test

# 启动后台服务（默认行为）
./target/release/echopup

# 显式启动后台服务
./target/release/echopup start

# 查看后台状态
./target/release/echopup status

# 打开管理 TUI
./target/release/echopup ui

# 管理 TUI 生命周期
./target/release/echopup ui status
./target/release/echopup ui stop
./target/release/echopup ui restart
```

## 使用方法

1. 运行 `./target/release/echopup`（默认后台启动，单实例）
2. 需要管理配置和模型时，运行 `./target/release/echopup ui`（UI 也是单实例）
3. 触发方式按平台区分：
   - macOS：在需要输入文本的应用中长按 `Control` 键 1 秒（左右 `Ctrl` 均可）
   - Linux X11：在需要输入文本的应用中长按 `F6` 1 秒
   - Linux Wayland：首次运行会尝试自动创建系统快捷键 `F6 -> echopup trigger toggle`
4. 按触发模式结束录音并转写：
   - 长按模式：松开热键即结束并输入
   - 按压切换模式（默认）：松开后继续录音，下次按下结束并输入

### Linux / Wayland 说明

- 当前 Linux/Wayland 默认走“桌面快捷键 -> `echopup trigger ...` -> 后台服务”的外部触发路径。
- 在 GNOME Wayland 下，首次运行 `echopup` 会尝试自动创建系统快捷键 `F6`。
- 若自动创建失败，可手动将以下命令绑定到系统快捷键：
  - `echopup trigger toggle`
- 当前 Linux 文本输入在失败时会回退到 `wtype`，这是 Wayland 路径下更现实的文本提交方式之一。
- 关于 Wayland 下更优雅的热键与文本提交方案，请参考：
  - `docs/architecture/wayland-compatibility-plan-v1.md`
  - `docs/adr/0005-wayland-trigger-and-text-commit-strategy.md`

### 配置

默认配置文件：`~/.echopup/config.toml`

```toml
[hotkey]
trigger_mode = "press_to_toggle" # 或 "hold_to_record"

[audio]
sample_rate = 16000
channels = 1

[asr]
backend = "sherpa_paraformer"

[asr.sherpa_paraformer]
# 模型文件默认在 ~/.echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/
encoder_path = "~/.echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/encoder.onnx"
decoder_path = "~/.echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/decoder.onnx"
tokens_path = "~/.echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/tokens.txt"
provider = "cpu"
num_threads = 4  # 自动检测，上限 8

[punctuation]
enabled = true
model_path = "~/.echopup/models/punctuation/model.onnx"

[llm]
enabled = false
provider = "openai"
model = "gpt-4o-mini"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[text_correction]
enabled = true
homophone_map = { "公做" = "工作", "行好" = "型号" }

[feedback]
# macOS 状态栏反馈（在 Linux 上可保留该配置但当前不生效）
status_bar_enabled = true
# 录音开始/结束提示音（默认开启）
sound_enabled = true
# 启动时显示 macOS 通知设置提示（默认开启）
notify_tip_on_start = true
```

热键说明：
- 热键固定为 Control 键（左右 Ctrl 均可），不可更改

热键触发模式：
- `press_to_toggle`（默认）：长按 1 秒开始，再按一次结束
- `hold_to_record`：长按 1 秒开始，松开即结束

### 启用 LLM 整理

1. 设置环境变量：
   ```bash
   export OPENAI_API_KEY="your-api-key"
   ```

2. 修改配置启用 LLM：
   ```toml
   [llm]
   enabled = true
   ```

## 命令行选项

```bash
echopup - AI Voice Dictation Tool

Usage: echopup [OPTIONS] [COMMAND]

Commands:
  run              运行语音输入
  start            后台启动服务
  stop             停止后台服务
  status           查看后台服务状态
  restart          重启后台服务
  trigger          发送外部触发动作（Linux 桌面快捷键集成）
  ui               打开管理 TUI（仅管理，不执行语音输入）
  test             测试各模块
  config           配置管理
  model            管理语音识别模型（下载 / 打开目录）

Options:
  -c, --config <CONFIG>  配置文件路径 [default: ~/.echopup/config.toml]
  -h, --help             显示帮助信息
  -V, --version          显示版本信息
```

`ui` 子命令支持：`echopup ui start|stop|status|restart`（`echopup ui` 等价于 `echopup ui start`）。

## 项目结构

```text
echo-pup-rust/
├── src/
│   ├── main.rs            # 主程序入口
│   ├── audio/             # 音频录制模块
│   ├── asr/               # 语音识别模块（sherpa-onnx Paraformer）
│   ├── commit/            # 文本提交模块
│   ├── config/            # 配置管理模块
│   ├── hotkey/            # 热键监听模块
│   ├── input/             # 键盘输入模块
│   ├── llm/               # LLM 整理模块
│   ├── session/           # 会话管理模块
│   ├── model_download.rs  # 模型下载
│   ├── punctuation.rs     # 离线标点恢复
│   ├── menu_core.rs       # 状态栏菜单核心逻辑
│   ├── status_indicator.rs # macOS 状态栏指示器
│   ├── text_processor.rs  # 文本后处理
│   ├── runtime.rs         # 运行时工具
│   ├── trigger.rs         # 外部触发
│   └── linux_desktop.rs   # Linux 桌面集成
├── docs/
└── Cargo.toml

~/.echopup/
├── config.toml
├── echopup.lock
├── echopup-ui.pid
├── echopup.log
└── models/
```

## 文档

- 文档索引：`docs/README.md`
- 治理说明：`docs/governance/zhou-tianzi.md`
- 朝议总账：`docs/reports/project-court-ledger.md`
- 环境与资源：`docs/setup/environment-resources.md`
- 性能路线图：`docs/architecture/performance-optimization-roadmap-v1.md`
- 状态栏菜单同步方案：`docs/architecture/status-bar-menu-sync-plan-v1.md`

## 常见问题

### Q: 键盘输入失败
A: Linux 请确认在图形会话中运行并具备输入模拟能力；macOS 请在“系统设置 -> 隐私与安全性 -> 辅助功能”中授权 EchoPup。

### Q: 语音识别模型加载失败
A: 检查模型文件（encoder.onnx, decoder.onnx, tokens.txt）是否存在于 `~/.echopup/models/asr/sherpa-onnx-streaming-paraformer-bilingual-zh-en/`，可运行 `echopup model` 重新下载。

### Q: 录音没有声音
A: 检查麦克风权限、输入设备和系统音量；在 macOS 上确认应用麦克风授权已开启。

### Q: 后台运行时没有通知提示
A: macOS 需要通知权限；Linux 需要 `notify-send` 且在图形会话中（有 `DISPLAY` 或 `WAYLAND_DISPLAY`）。

### Q: 为什么 macOS 通知来源显示为“脚本编辑器”？
A: 当前通过 `osascript` 发送通知，这是系统行为。请在“系统设置 -> 通知 -> 脚本编辑器”中开启通知并选择“横幅”或“提醒”。

### Q: 菜单栏状态怎么理解？
A: 默认状态显示 logo（无闪烁背景）；录音中显示 mic + 橘红边缘脉动胶囊；识别中显示 mic + 橘黄边缘脉动胶囊；识别完成显示 mic + 绿色背景，约 1.5 秒后自动恢复默认状态。

## License

MIT
