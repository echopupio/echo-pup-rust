# TypechoAI

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
# 在项目目录下
mkdir -p models
./scripts/download_model.sh small
```

### 3. 编译运行

```bash
# 编译
cargo build --release

# 测试各模块
./target/release/typechoai test

# 运行
./target/release/typechoai run
```

## 使用方法

1. 运行 `./target/release/typechoai run`
2. 在需要输入文本的应用中，按住 **F12**（默认热键）
3. 对着麦克风说话
4. 松开 F12，识别文本将自动输入

### 配置

默认配置文件: `~/.typechoai/config.toml`

```toml
[hotkey]
key = "F12"

[audio]
sample_rate = 16000
channels = 1

[whisper]
model_path = "models/ggml-small.bin"
translate = false
language = "zh"

[llm]
enabled = false
provider = "openai"
model = "gpt-4o-mini"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
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
typechoai - AI Voice Dictation Tool

Usage: typechoai [OPTIONS] [COMMAND]

Commands:
  run              运行语音输入
  test             测试各模块
  config           配置管理
  download-model   下载 Whisper 模型

Options:
  -c, --config <CONFIG>  配置文件路径 [default: ~/.typechoai/config.toml]
  -h, --help            显示帮助信息
  -V, --version         显示版本信息
```

## 项目结构

```
typecho_ai/
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
├── models/           # 模型文件目录
└── Cargo.toml
```

## 常见问题

### Q: 键盘输入失败
A: 确保在图形界面环境下运行，键盘模拟需要 X11

### Q: Whisper 模型加载失败
A: 检查模型文件是否存在于 models/ 目录

### Q: 录音没有声音
A: 检查麦克风权限和系统音频配置

## License

MIT
