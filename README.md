# EchoPup

AI 语音输入工具 - 支持长按模式与按压切换模式，自动识别并输入文本

## 功能特性

- 🎤 **语音输入** - 支持两种触发模式：长按模式 / 按压切换模式
- 🔄 **语音识别** - 使用本地 Whisper 模型进行语音转文字
- ✨ **智能整理** - 可选 LLM 自动润色转写文本
- ⌨️ **自动输入** - 自动模拟键盘输入到当前应用
- ⚙️ **热键自定义** - 支持自定义触发热键（含安全校验）
- 🧭 **状态栏管理** - 热键捕获、LLM 表单、模型切换与下载弹窗
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

### 2. 下载 Whisper 模型

```bash
# 模型将下载到 ~/.echopup/models
./target/release/echopup download-model large-v3

# 或继续使用脚本
./scripts/download_model.sh large-v3
```

### 2.5 验收与性能基线（可选）

```bash
# 状态栏菜单与 TUI 对齐验收（自动化）
./scripts/run_acceptance.sh

# 聚合最近 200 条性能埋点（P50/P95）
./scripts/perf_baseline.py --limit 200

# 导出基线到 CSV（机型 × 档位）
./scripts/perf_baseline.py --profile balanced --export-csv ./artifacts/perf-baseline.csv
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
3. 在需要输入文本的应用中长按热键 1 秒（默认 `right_ctrl`）
4. 按触发模式结束录音并转写：
   - 长按模式：松开热键即结束并输入
   - 按压切换模式（默认）：松开后继续录音，下次按下结束并输入

### 配置

默认配置文件：`~/.echopup/config.toml`

```toml
[hotkey]
key = "right_ctrl"
trigger_mode = "press_to_toggle" # 或 "hold_to_record"

[audio]
sample_rate = 16000
channels = 1
vad_enabled = false
vad_silence_threshold_ms = 1500

[whisper]
# 可选: "accurate" / "balanced" / "fast"
# performance_profile = "balanced"
# 请使用绝对路径；默认在 ~/.echopup/models 下
model_path = "/Users/<user>/.echopup/models/ggml-large-v3.bin"
translate = false
language = "zh"
decoding_strategy = "beam_search"
beam_size = 5
greedy_best_of = 5
temperature = 0.0
no_context = true
suppress_nst = true
n_threads = "auto"
# initial_prompt = "可选热词：EchoPup, OpenAI, Rust"
hotwords = []

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

热键建议与限制：
- 推荐：`right_ctrl`（默认）
- 允许：单独 `F1-F24`，或至少包含 `ctrl/alt/super` 的组合键
- macOS 说明：`F1-F12` 可能被系统映射为媒体键，触发不稳定时建议改用 `F13-F24` 或 `ctrl+F*`
- 限制：最多 3 键；不支持仅 `shift+字母`；不允许普通输入键（如 `z`、`space`）单独作为热键

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
  ui               打开管理 TUI（仅管理，不执行语音输入）
  test             测试各模块
  config           配置管理
  download-model   下载 Whisper 模型

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
│   ├── main.rs         # 主程序入口
│   ├── audio/          # 音频录制模块
│   ├── config/         # 配置管理模块
│   ├── hotkey/         # 热键监听模块
│   ├── input/          # 键盘输入模块
│   ├── llm/            # LLM 整理模块
│   ├── stt/            # Whisper 转写模块
│   ├── ui.rs           # 终端管理 UI
│   └── status_indicator.rs # macOS 状态栏指示器
├── scripts/
│   ├── download_model.sh
│   ├── run_acceptance.sh
│   └── perf_baseline.py
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

### Q: Whisper 模型加载失败
A: 检查模型文件是否存在于 `~/.echopup/models/`，并确认 `whisper.model_path` 指向有效绝对路径。

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
