# EchoPup

AI 语音输入工具 - 按住热键说话，自动识别并输入文本

## 功能特性

- 🎤 **语音输入** - 按住热键说话，松开后自动输入
- 🔄 **语音识别** - 使用本地 Whisper 模型进行语音转文字
- ✨ **智能整理** - 可选 LLM 自动润色转写文本
- ⌨️ **自动输入** - 自动模拟键盘输入到当前应用
- ⚙️ **热键自定义** - 支持自定义触发热键

## 环境要求

- Linux (需要 X11)
- Rust 1.70+
- 系统依赖:
  - `pkg-config`
  - `libssl-dev`
  - `libasound2-dev`

## 快速开始

### 1. 安装系统依赖

```bash
sudo apt install pkg-config libssl-dev libasound2-dev
```

### 2. 下载 Whisper 模型

```bash
# 模型将下载到 ~/.echopup/models
./scripts/download_model.sh large-v3
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

1. 运行 `./target/release/echopup`（默认后台启动，且单实例）
2. 如需管理配置和模型，运行 `./target/release/echopup ui`（全局单实例，重复执行会接管到当前终端）
3. 在需要输入文本的应用中，按住右 Ctrl（默认热键，配置值 `right_ctrl`）
4. 对着麦克风说话
5. 松开右 Ctrl，识别文本将自动输入

### 配置

默认配置文件: `~/.echopup/config.toml`

```toml
[hotkey]
key = "right_ctrl"

[audio]
sample_rate = 16000
channels = 1

[whisper]
# 可选: "accurate" / "balanced" / "fast"
# performance_profile = "balanced"
model_path = "/home/<user>/.echopup/models/ggml-large-v3.bin"
translate = false
language = "zh"
decoding_strategy = "beam_search"
beam_size = 5
greedy_best_of = 5
temperature = 0.0
no_context = true
suppress_nst = true
n_threads = "auto"
# initial_prompt = "可选热词：EchoPup, OpenAI, Rust, ..."
hotwords = ["EchoPup", "OpenAI", "Rust"]

[llm]
enabled = false
provider = "openai"
model = "gpt-4o-mini"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[text_correction]
enabled = true
homophone_map = { "公做" = "工作", "行好" = "型号" }
```

### 启用 LLM 整理

1. 设置环境变量:
   ```bash
   export OPENAI_API_KEY="your-api-key"
   ```

2. 修改配置启用 LLM:
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
  -h, --help            显示帮助信息
  -V, --version         显示版本信息
```

`ui` 子命令支持：`echopup ui start|stop|status|restart`（`echopup ui` 等价于 `echopup ui start`）。

## 项目结构

```
echo-pup-rust/
├── src/
│   ├── main.rs        # 主程序入口
│   ├── audio/         # 音频录制模块
│   ├── config/        # 配置管理模块
│   ├── hotkey/       # 热键监听模块
│   ├── input/        # 键盘输入模块
│   ├── llm/          # LLM 整理模块
│   └── stt/          # Whisper 转写模块
├── scripts/
│   └── download_model.sh  # 下载模型脚本
└── Cargo.toml

~/.echopup/
├── config.toml
├── echopup.lock
├── echopup-ui.pid
└── models/            # 模型文件目录
```

## 性能优化计划

速度优化路线图文档：`docs/PERFORMANCE_OPTIMIZATION_ROADMAP.md`

## 常见问题

### Q: 键盘输入失败
A: 确保在图形界面环境下运行，键盘模拟需要 X11

### Q: Whisper 模型加载失败
A: 检查模型文件是否存在于 `~/.echopup/models/` 目录，且 `model_path` 配置正确

### Q: 录音没有声音
A: 检查麦克风权限和系统音频配置

## License

MIT
