# CatEcho Agent 实现规格

**项目名称：** CatEcho  
**项目类型：** 跨平台 AI 语音听写工具  
**主要语言：** Rust  
**核心流程：** 热键 -> 录音 -> Whisper 语音转文本 -> LLM 文本整理 -> 系统键盘输入

---

## 1. 项目目标

CatEcho 是一个跨平台 AI 语音输入工具。

目标用户体验：

```text
按住 F12
→ 说话
→ 本地 Whisper 语音识别
→ 可选的 LLM 改写 / 清理
→ 将最终文本输入到当前聚焦应用
```

主要使用场景：

* AI 编程
* 写 Prompt
* 写文档
* 写 commit message
* 在任意应用中进行通用语音输入

目标支持平台：

* Linux 优先
* macOS 第二阶段
* Windows 后续支持

---

## 2. 产品需求

### 2.1 MVP 需求

MVP 必须支持：

1. 全局热键监听
2. 按住说话录音
3. 本地 Whisper 转写
4. 可选的 LLM 文本清理
5. 将最终文本模拟输入到当前聚焦应用
6. 配置文件支持
7. 提供 `run` / `test` / `config` 相关 CLI 命令

### 2.2 MVP 非目标

MVP 暂不需要：

* 流式转写
* GUI
* 账号系统
* 云同步
* 高级设置界面
* 多设备同步

---

## 3. 功能流程

```text
用户按下 F12
→ 开始录制麦克风音频
→ 用户松开 F12
→ 停止录音
→ 对缓冲音频运行 Whisper 转写
→ 得到 raw_text
→ 可选地将 raw_text 发送给 LLM 改写模块
→ 得到 clean_text
→ 模拟键盘输入到当前应用
```

---

## 4. 技术架构

```text
Hotkey Listener
      │
      ▼
Audio Recorder
      │
      ▼
Whisper STT
      │
      ▼
LLM Rewrite
      │
      ▼
System Input
```

数据流：

```text
麦克风
→ PCM 音频缓冲
→ Whisper 转写
→ raw text
→ LLM 清理
→ final text
→ 键盘模拟输入
```

---

## 5. 推荐 Rust Crate

| 能力 | Crate |
| --- | --- |
| CLI | `clap` |
| 配置 | `serde`, `toml` |
| 日志 | `tracing`, `tracing-subscriber` |
| 音频采集 | `cpal` |
| WAV 写入/调试 | `hound` |
| Whisper 推理 | `whisper-rs` |
| HTTP 客户端 | `reqwest` |
| JSON | `serde_json` |
| 全局热键 | `global-hotkey` |
| 键盘输入模拟 | `enigo` |
| 错误处理 | `anyhow`, `thiserror` |
| 异步运行时 | `tokio` |
| 路径/目录 | `dirs` |

---

## 6. 完整 Cargo.toml

```toml
[package]
name = "cat-echo"
version = "0.1.0"
edition = "2021"
authors = ["CatEcho"]
description = "Cross-platform AI voice dictation tool powered by Whisper + LLM + system typing"
license = "MIT"

[dependencies]
anyhow = "1"
thiserror = "1"

clap = { version = "4", features = ["derive"] }

serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
dirs = "5"

tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }

tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }

cpal = "0.15"
hound = "3"

global-hotkey = "0.5"
enigo = "0.2"

whisper-rs = "0.11"
```

说明：

* 版本号在实际实现时可能需要小幅调整。
* 在 Linux 上，部分 crate 可能依赖 ALSA / X11 / Wayland 等系统开发库。
* `whisper-rs` 依赖底层原生编译能力，可能需要按平台做额外调优。

---

## 7. 项目结构

```text
cat-echo/
├─ Cargo.toml
├─ README.md
├─ LICENSE
├─ .gitignore
├─ docs/
│  ├─ architecture.md
│  └─ agent_spec.md
├─ models/
│  └─ ggml-small.bin
├─ src/
│  ├─ main.rs
│  ├─ cli.rs
│  ├─ config.rs
│  ├─ errors.rs
│  ├─ app.rs
│  │
│  ├─ audio/
│  │  ├─ mod.rs
│  │  ├─ recorder.rs
│  │  └─ buffer.rs
│  │
│  ├─ hotkey/
│  │  ├─ mod.rs
│  │  └─ listener.rs
│  │
│  ├─ stt/
│  │  ├─ mod.rs
│  │  └─ whisper.rs
│  │
│  ├─ llm/
│  │  ├─ mod.rs
│  │  └─ rewrite.rs
│  │
│  ├─ input/
│  │  ├─ mod.rs
│  │  └─ keyboard.rs
│  │
│  └─ utils/
│     ├─ mod.rs
│     └─ paths.rs
└─ .catecho/
   └─ config.toml.example
```

---

## 8. 配置规格

默认配置路径：

```text
~/.catecho/config.toml
```

示例配置：

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
enabled = true
provider = "openai"
model = "gpt-4o-mini"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[input]
typing_delay_ms = 5

[app]
log_level = "info"
save_debug_wav = false
```

行为规则：

* 如果禁用 LLM，则直接输入原始 Whisper 文本。
* 如果 LLM 调用失败，则回退到原始文本。
* 如果模型文件缺失，应用应返回友好的错误提示。
* 如果当前平台不支持热键后端，应用应明确失败原因。

---

## 9. CLI 规格

二进制名称：

```text
catecho
```

命令：

```text
catecho run
catecho doctor
catecho transcribe --file sample.wav
catecho config init
catecho config path
catecho version
```

### 预期命令行为

#### `catecho run`

启动后台热键监听和主应用循环。

#### `catecho doctor`

检查项：

* 配置是否存在
* Whisper 模型是否存在
* 麦克风是否可访问
* 热键后端是否可用
* 可选的 LLM 连通性

#### `catecho transcribe --file sample.wav`

对 WAV 文件运行 Whisper，并输出原始文本 / 清理后的文本。

#### `catecho config init`

如果默认配置文件不存在，则创建它。

#### `catecho config path`

输出配置文件路径。

---

## 10. 模块职责

### 10.1 `cli.rs`

职责：

* 解析 CLI 参数
* 分发命令
* 调用应用启动逻辑

### 10.2 `config.rs`

职责：

* 从磁盘加载配置
* 校验配置
* 创建默认配置

### 10.3 `audio/recorder.rs`

职责：

* 打开麦克风流
* 开始 / 停止缓冲
* 返回 PCM 音频帧

### 10.4 `hotkey/listener.rs`

职责：

* 注册全局热键
* 发出 `Press` 和 `Release` 事件
* 与应用状态机集成

### 10.5 `stt/whisper.rs`

职责：

* 加载 Whisper 模型
* 将音频缓冲转换为转写输入
* 执行转写
* 返回原始文本

### 10.6 `llm/rewrite.rs`

职责：

* 调用 LLM 提供方
* 应用清理 prompt
* 返回清理后的文本
* 出错时安全回退

### 10.7 `input/keyboard.rs`

职责：

* 将最终文本输入到当前聚焦应用
* 支持可配置的按键间延迟

### 10.8 `app.rs`

职责：

* 持有应用状态
* 编排 热键 -> 录音 -> 转写 -> 改写 -> 输入 整体流程
* 管理错误和日志

---

## 11. Rust 骨架代码

### `src/main.rs`

```rust
mod app;
mod cli;
mod config;
mod errors;

mod audio;
mod hotkey;
mod stt;
mod llm;
mod input;
mod utils;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    cli::run().await
}
```

---

### `src/cli.rs`

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "catecho")]
#[command(version)]
#[command(about = "CatEcho voice dictation tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Run,
    Doctor,
    Transcribe {
        #[arg(long)]
        file: String,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Version,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    Init,
    Path,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run => crate::app::run().await,
        Commands::Doctor => crate::app::doctor().await,
        Commands::Transcribe { file } => crate::app::transcribe_file(&file).await,
        Commands::Config { command } => match command {
            ConfigCommands::Init => crate::config::init_default_config(),
            ConfigCommands::Path => {
                println!("{}", crate::config::config_path()?.display());
                Ok(())
            }
        },
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}
```

---

### `src/config.rs`

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub audio: AudioConfig,
    pub whisper: WhisperConfig,
    pub llm: LlmConfig,
    pub input: InputConfig,
    pub app: AppConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    pub model_path: String,
    pub translate: bool,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub api_base: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputConfig {
    pub typing_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub log_level: String,
    pub save_debug_wav: bool,
}

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("home directory not found")?;
    Ok(home.join(".catecho").join("config.toml"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let cfg: Config = toml::from_str(&content)?;
    Ok(cfg)
}

pub fn init_default_config() -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if path.exists() {
        println!("Config already exists: {}", path.display());
        return Ok(());
    }

    let default = Config {
        hotkey: HotkeyConfig { key: "F12".into() },
        audio: AudioConfig { sample_rate: 16000, channels: 1 },
        whisper: WhisperConfig {
            model_path: "models/ggml-small.bin".into(),
            translate: false,
            language: "zh".into(),
        },
        llm: LlmConfig {
            enabled: true,
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            api_base: "https://api.openai.com/v1".into(),
            api_key_env: "OPENAI_API_KEY".into(),
        },
        input: InputConfig { typing_delay_ms: 5 },
        app: AppConfig {
            log_level: "info".into(),
            save_debug_wav: false,
        },
    };

    let content = toml::to_string_pretty(&default)?;
    fs::write(&path, content)?;
    println!("Created config: {}", path.display());
    Ok(())
}
```

---

### `src/app.rs`

```rust
use anyhow::Result;

pub async fn run() -> Result<()> {
    let cfg = crate::config::load()?;

    println!("Starting CatEcho...");
    println!("Hotkey: {}", cfg.hotkey.key);

    // TODO:
    // 1. initialize whisper engine
    // 2. initialize recorder
    // 3. initialize hotkey listener
    // 4. on press => start recording
    // 5. on release => stop recording, transcribe, rewrite, type text

    Ok(())
}

pub async fn doctor() -> Result<()> {
    let cfg = crate::config::load()?;
    println!("Config OK: {:?}", cfg.hotkey.key);

    // TODO:
    // check model path
    // check mic
    // check hotkey registration
    // check optional API env

    Ok(())
}

pub async fn transcribe_file(file: &str) -> Result<()> {
    let cfg = crate::config::load()?;
    let whisper = crate::stt::whisper::WhisperEngine::new(&cfg.whisper.model_path)?;
    let raw = whisper.transcribe_wav(file)?;
    println!("RAW:\n{}", raw);

    if cfg.llm.enabled {
        let cleaned = crate::llm::rewrite::rewrite_text(&cfg, &raw).await?;
        println!("\nCLEAN:\n{}", cleaned);
    }

    Ok(())
}
```

---

### `src/audio/mod.rs`

```rust
pub mod recorder;
pub mod buffer;
```

### `src/audio/buffer.rs`

```rust
#[derive(Default, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
}

impl AudioBuffer {
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn push_samples(&mut self, data: &[f32]) {
        self.samples.extend_from_slice(data);
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}
```

### `src/audio/recorder.rs`

```rust
use anyhow::Result;
use crate::audio::buffer::AudioBuffer;

pub struct Recorder {
    pub sample_rate: u32,
    pub channels: u16,
}

impl Recorder {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self { sample_rate, channels }
    }

    pub fn start(&mut self) -> Result<()> {
        // TODO: open cpal input stream and begin buffering samples
        Ok(())
    }

    pub fn stop(&mut self) -> Result<AudioBuffer> {
        // TODO: stop stream and return captured samples
        Ok(AudioBuffer::default())
    }
}
```

---

### `src/stt/mod.rs`

```rust
pub mod whisper;
```

### `src/stt/whisper.rs`

```rust
use anyhow::Result;

pub struct WhisperEngine {
    model_path: String,
}

impl WhisperEngine {
    pub fn new(model_path: &str) -> Result<Self> {
        // TODO: initialize whisper-rs context
        Ok(Self {
            model_path: model_path.to_string(),
        })
    }

    pub fn transcribe_buffer(&self, _samples: &[f32]) -> Result<String> {
        // TODO: run whisper inference on PCM buffer
        Ok("placeholder transcription".to_string())
    }

    pub fn transcribe_wav(&self, _path: &str) -> Result<String> {
        // TODO: load wav and transcribe
        Ok("placeholder wav transcription".to_string())
    }
}
```

---

### `src/llm/mod.rs`

```rust
pub mod rewrite;
```

### `src/llm/rewrite.rs`

```rust
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::json;

pub async fn rewrite_text(cfg: &crate::config::Config, raw: &str) -> Result<String> {
    if !cfg.llm.enabled {
        return Ok(raw.to_string());
    }

    let api_key = std::env::var(&cfg.llm.api_key_env)
        .with_context(|| format!("missing env var {}", cfg.llm.api_key_env))?;

    let prompt = format!(
        "Rewrite the following speech recognition text.\n\
         Rules:\n\
         - remove filler words\n\
         - add punctuation\n\
         - keep original meaning\n\
         - do not add new content\n\n\
         Text:\n{}",
        raw
    );

    let body = json!({
        "model": cfg.llm.model,
        "messages": [
            {"role": "user", "content": prompt}
        ]
    });

    let client = Client::new();
    let resp = client
        .post(format!("{}/chat/completions", cfg.llm.api_base))
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    let value: serde_json::Value = resp.json().await?;
    let text = value["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or(raw)
        .to_string();

    Ok(text)
}
```

---

### `src/input/mod.rs`

```rust
pub mod keyboard;
```

### `src/input/keyboard.rs`

```rust
use anyhow::Result;
use enigo::{Enigo, Keyboard, Settings};

pub fn type_text(text: &str) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.text(text)?;
    Ok(())
}
```

---

### `src/hotkey/mod.rs`

```rust
pub mod listener;
```

### `src/hotkey/listener.rs`

```rust
use anyhow::Result;

pub struct HotkeyListener;

impl HotkeyListener {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn run<F1, F2>(&self, _on_press: F1, _on_release: F2) -> Result<()>
    where
        F1: Fn() + Send + 'static,
        F2: Fn() + Send + 'static,
    {
        // TODO: register global hotkey using global-hotkey crate
        // TODO: map key press/release events
        Ok(())
    }
}
```

---

### `src/errors.rs`

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CatEchoError {
    #[error("microphone not available")]
    MicrophoneUnavailable,

    #[error("whisper model not found")]
    ModelNotFound,

    #[error("hotkey registration failed")]
    HotkeyRegistrationFailed,
}
```

---

## 12. 模块伪代码

### 12.1 应用状态机

```text
state = Idle

on_hotkey_press:
    if state == Idle:
        recorder.start()
        state = Recording

on_hotkey_release:
    if state == Recording:
        audio = recorder.stop()
        state = Processing

        raw_text = whisper.transcribe(audio)

        if llm_enabled:
            clean_text = llm.rewrite(raw_text)
        else:
            clean_text = raw_text

        if clean_text not empty:
            keyboard.type(clean_text)

        state = Idle
```

---

### 12.2 音频录制伪代码

```text
create shared buffer
create cpal input stream
for each input callback:
    if recording_enabled:
        convert input samples to f32
        append to shared buffer

start():
    clear buffer
    set recording_enabled = true
    play input stream

stop():
    set recording_enabled = false
    clone current buffer
    return buffer
```

---

### 12.3 Whisper 转写伪代码

```text
load whisper model once at startup

transcribe(samples):
    create whisper state
    configure params:
        language
        translate false
        no timestamps
    run full transcription on samples
    concatenate segment text
    return joined text
```

---

### 12.4 LLM 改写伪代码

```text
if llm disabled:
    return raw_text

build prompt:
    remove filler words
    add punctuation
    keep original meaning
    do not add content

send request to provider
extract text from response
if request fails:
    return raw_text
else:
    return cleaned_text
```

---

### 12.5 键盘输入伪代码

```text
focus stays on current app
simulate text entry
optionally insert with small per-char delay
if backend fails:
    return user-friendly error
```

---

## 13. 各平台实现说明

### Linux

可能需要的系统依赖：

```bash
sudo apt install libasound2-dev pkg-config build-essential
```

可能存在的输入限制：

* X11 下模拟输入通常更容易实现
* Wayland 在部分环境下可能限制合成输入

MVP 建议：

* 优先支持 Ubuntu + X11
* 明确记录 Wayland 的限制

### macOS

说明：

* 键盘输入模拟可能需要辅助功能权限
* 热键注册和键盘注入可能需要用户授权

### Windows

说明：

* 建议在 Linux/macOS MVP 稳定后再接入
* 键盘模拟在底层会使用不同 API

---

## 14. CloudCode / Codex 执行计划

本节专门写给 AI 编码代理。

### Phase 1: Bootstrap Project

任务：

1. 初始化 Rust 项目
2. 创建模块结构
3. 完善 `Cargo.toml`
4. 实现 CLI 参数解析
5. 实现配置加载

预期产出：

* 项目可成功构建
* `catecho config init`
* `catecho config path`
* `catecho version`

### Phase 2: Implement Whisper File Transcription

任务：

1. 集成 `whisper-rs`
2. 加载模型文件
3. 实现 `catecho transcribe --file sample.wav`
4. 输出原始转写结果
5. 可选输出改写后的文本

预期产出：

* 能转写本地 WAV 文件
* 能验证模型加载是否正常

### Phase 3: Implement Audio Recording

任务：

1. 集成 `cpal`
2. 录制麦克风音频
3. 缓冲音频采样
4. 支持开始 / 停止录音
5. 可选导出 WAV 调试文件

预期产出：

* 能从麦克风录音
* 可以保存或检查采集到的音频

### Phase 4: Implement Hotkey Integration

任务：

1. 集成 `global-hotkey`
2. 绑定 F12
3. 按下时启动录音
4. 松开时停止录音
5. 记录事件序列日志

预期产出：

* 物理热键可以控制录音生命周期

### Phase 5: Implement System Typing

任务：

1. 集成 `enigo`
2. 将任意文本输入到当前聚焦应用
3. 在文本编辑器中测试
4. 增加失败处理

预期产出：

* 示例文本能出现在当前聚焦应用中

### Phase 6: Full Pipeline Integration

任务：

1. 串联热键 + 录音 + Whisper + 改写 + 输入
2. 在改写失败时增加回退逻辑
3. 增加日志和诊断
4. 验证端到端工作流

预期产出：

* 按住 F12
* 说话
* 松开
* 转写并清理后的文本自动输入到当前应用

### Phase 7: Hardening

任务：

1. 改善错误提示
2. 增加 doctor 检查
3. 增加配置校验
4. 优化启动速度和模型加载
5. 增加优雅退出

预期产出：

* MVP 足够稳定，可用于真实使用

---

## 15. 实现规则（给 Agent）

AI 编码代理应遵循以下规则：

1. 保持改动模块化。
2. 优先频繁编译验证。
3. 先实现最小占位逻辑，再逐步替换。
4. MVP 不要尝试流式转写。
5. 优先保障 Linux 支持。
6. 将 Wayland 限制视为已知平台限制，而不是实现失败。
7. LLM 改写必须是可选能力，且支持关闭。
8. 改写失败时必须始终回退到原始 Whisper 输出。
9. MVP 阶段避免不必要的抽象。
10. 端到端可用性优先于架构完美。

---

## 16. 验收标准

当以下条件全部满足时，MVP 视为完成：

* `cargo build --release` 成功
* `catecho config init` 可用
* `catecho doctor` 能输出有价值的诊断信息
* `catecho transcribe --file sample.wav` 能返回文本
* 按下 F12 会开始录音
* 松开 F12 会停止录音
* 语音可通过本地 Whisper 完成转写
* LLM 改写可开启/关闭
* 最终文本能自动输入到当前应用
* 失败场景可理解、可恢复

---

## 17. 后续扩展

MVP 之后的路线图：

1. 流式转写
2. 通过 Ollama 实现本地 LLM 改写
3. 面向 AI Coding 的更智能 prompt 模式
4. 剪贴板粘贴兜底方案
5. 托盘图标 / 轻量 GUI
6. 多语言自动识别
7. 模型自动下载
8. 标点模式 / 代码模式
9. 噪声抑制
10. 逐词实时预览

---

## 18. 建议的首个交付物

第一个实现里程碑建议是：

```text
CLI + config + Whisper WAV transcription
```

原因：

* 最容易验证
* 可以先隔离模型集成问题
* 在引入麦克风 / 热键 / 系统输入前，能先降低调试复杂度

---

## 19. 建议的 GitHub README 标语

```text
CatEcho — Cross-platform AI voice dictation powered by Whisper, LLM cleanup, and system typing.
```

---

## 20. 最终总结

CatEcho 是一个基于 Rust 的 AI 语音输入工具，工作流如下：

```text
热键
→ 录音
→ Whisper 转写
→ LLM 清理
→ 将文本输入到当前应用
```

推荐的 MVP 优先级：

```text
1. CLI + config
2. Whisper 文件转写
3. 麦克风录音
4. 热键
5. 系统输入
6. 全链路集成
```
