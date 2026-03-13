# TypechoAI Agent Spec

**Project Name:** TypechoAI
**Project Type:** Cross-platform AI Voice Dictation Tool
**Primary Language:** Rust
**Core Workflow:** Hotkey → Record → Whisper STT → LLM Rewrite → System Typing

---

## 1. Project Goal

TypechoAI is a cross-platform AI voice dictation tool.

Target user experience:

```text
Hold F12
→ speak
→ local Whisper speech-to-text
→ optional LLM rewrite / cleanup
→ type final text into the current focused application
```

Primary use cases:

* AI coding
* writing prompts
* writing documentation
* writing commit messages
* general voice typing in any app

Supported targets:

* Linux first
* macOS second
* Windows later

---

## 2. Product Requirements

### 2.1 MVP Requirements

The MVP must support:

1. Global hotkey listening
2. Push-to-talk recording
3. Local Whisper transcription
4. Optional LLM text cleanup
5. Simulated keyboard typing into the focused app
6. Config file support
7. CLI commands for run / test / config

### 2.2 Non-Goals for MVP

The MVP does **not** need:

* streaming transcription
* GUI
* account system
* cloud sync
* advanced settings UI
* multi-device sync

---

## 3. Functional Flow

```text
User presses F12
→ start recording microphone audio
→ user releases F12
→ stop recording
→ run Whisper transcription on buffered audio
→ get raw_text
→ optionally send raw_text to LLM rewrite module
→ get clean_text
→ simulate typing into active application
```

---

## 4. Technical Architecture

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

Data flow:

```text
Microphone
→ PCM audio buffer
→ Whisper transcription
→ raw text
→ LLM cleanup
→ final text
→ keyboard simulation
```

---

## 5. Recommended Rust Crates

| Capability        | Crate                           |
| ----------------- | ------------------------------- |
| CLI               | `clap`                          |
| Config            | `serde`, `toml`                 |
| Logging           | `tracing`, `tracing-subscriber` |
| Audio capture     | `cpal`                          |
| WAV writing/debug | `hound`                         |
| Whisper inference | `whisper-rs`                    |
| HTTP client       | `reqwest`                       |
| JSON              | `serde_json`                    |
| Hotkey            | `global-hotkey`                 |
| Keyboard input    | `enigo`                         |
| Error handling    | `anyhow`, `thiserror`           |
| Async runtime     | `tokio`                         |
| Paths/directories | `dirs`                          |

---

## 6. Full Cargo.toml

```toml
[package]
name = "typechoai"
version = "0.1.0"
edition = "2021"
authors = ["TypechoAI"]
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

Notes:

* Version numbers may need minor adjustment during implementation.
* For Linux, some crates may require system packages such as ALSA / X11 / Wayland development libraries.
* `whisper-rs` depends on underlying native compilation and may need tuning per platform.

---

## 7. Project Structure

```text
typechoai/
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
└─ .typechoai/
   └─ config.toml.example
```

---

## 8. Config Specification

Default config path:

```text
~/.typechoai/config.toml
```

Example config:

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

Behavior rules:

* If LLM is disabled, raw Whisper text is typed directly.
* If LLM fails, fallback to raw text.
* If model file is missing, app should return a friendly error.
* If hotkey backend is unsupported on the current platform, app should fail clearly.

---

## 9. CLI Specification

Binary name:

```text
typecho
```

Commands:

```text
typecho run
typecho doctor
typecho transcribe --file sample.wav
typecho config init
typecho config path
typecho version
```

### Expected command behavior

#### `typecho run`

Starts background hotkey listener and main app loop.

#### `typecho doctor`

Checks:

* config exists
* Whisper model exists
* microphone is accessible
* hotkey backend seems available
* optional LLM connectivity

#### `typecho transcribe --file sample.wav`

Runs Whisper on a WAV file and prints raw / cleaned text.

#### `typecho config init`

Creates default config file if missing.

#### `typecho config path`

Prints the config path.

---

## 10. Module Responsibilities

### 10.1 `cli.rs`

Responsibilities:

* parse CLI args
* dispatch commands
* call app bootstrap

### 10.2 `config.rs`

Responsibilities:

* load config from disk
* validate config
* create default config

### 10.3 `audio/recorder.rs`

Responsibilities:

* open microphone stream
* start / stop buffering
* return PCM audio frames

### 10.4 `hotkey/listener.rs`

Responsibilities:

* register global hotkey
* emit `Press` and `Release` events
* integrate with application state machine

### 10.5 `stt/whisper.rs`

Responsibilities:

* load Whisper model
* convert audio buffer to transcription input
* run transcription
* return raw text

### 10.6 `llm/rewrite.rs`

Responsibilities:

* call LLM provider
* apply cleanup prompt
* return cleaned text
* fallback safely on errors

### 10.7 `input/keyboard.rs`

Responsibilities:

* type final text to focused application
* configurable delay between keystrokes

### 10.8 `app.rs`

Responsibilities:

* own app state
* orchestrate hotkey → recording → transcription → rewrite → typing
* manage errors and logging

---

## 11. Rust Skeleton

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
#[command(name = "typecho")]
#[command(version)]
#[command(about = "TypechoAI voice dictation tool")]
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
    Ok(home.join(".typechoai").join("config.toml"))
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

    println!("Starting TypechoAI...");
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
pub enum TypechoError {
    #[error("microphone not available")]
    MicrophoneUnavailable,

    #[error("whisper model not found")]
    ModelNotFound,

    #[error("hotkey registration failed")]
    HotkeyRegistrationFailed,
}
```

---

## 12. Module Pseudocode

### 12.1 App State Machine

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

### 12.2 Audio Recorder Pseudocode

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

### 12.3 Whisper Transcription Pseudocode

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

### 12.4 LLM Rewrite Pseudocode

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

### 12.5 Keyboard Typing Pseudocode

```text
focus stays on current app
simulate text entry
optionally insert with small per-char delay
if backend fails:
    return user-friendly error
```

---

## 13. Implementation Notes by Platform

### Linux

Potential system dependencies:

```bash
sudo apt install libasound2-dev pkg-config build-essential
```

Potential typing limitations:

* X11 typing is generally easier
* Wayland may restrict synthetic input in some environments

MVP recommendation:

* prioritize Ubuntu + X11 first
* document Wayland limitations explicitly

### macOS

Notes:

* may require Accessibility permissions for keyboard simulation
* hotkey registration and keyboard injection may require user approval

### Windows

Notes:

* add after Linux/macOS MVP stabilizes
* keyboard simulation uses different APIs under the hood

---

## 14. CloudCode / Codex Execution Plan

This section is written specifically for AI coding agents.

### Phase 1: Bootstrap Project

Tasks:

1. initialize Rust project
2. create module structure
3. fill `Cargo.toml`
4. implement CLI parsing
5. implement config loading

Expected output:

* builds successfully
* `typecho config init`
* `typecho config path`
* `typecho version`

### Phase 2: Implement Whisper File Transcription

Tasks:

1. integrate `whisper-rs`
2. load model file
3. implement `typecho transcribe --file sample.wav`
4. print raw transcription
5. optionally print rewritten transcription

Expected output:

* can transcribe local WAV files
* can validate model loading

### Phase 3: Implement Audio Recording

Tasks:

1. integrate `cpal`
2. record microphone audio
3. buffer audio samples
4. support start / stop recording
5. optional WAV debug dump

Expected output:

* record from microphone
* can save or inspect captured audio

### Phase 4: Implement Hotkey Integration

Tasks:

1. integrate `global-hotkey`
2. bind F12
3. on press start recorder
4. on release stop recorder
5. log event sequence

Expected output:

* physical hotkey controls recording lifecycle

### Phase 5: Implement System Typing

Tasks:

1. integrate `enigo`
2. type arbitrary text to focused app
3. test against text editor
4. add failure handling

Expected output:

* sample text appears in focused app

### Phase 6: Full Pipeline Integration

Tasks:

1. wire hotkey + recording + whisper + rewrite + typing
2. add fallback if rewrite fails
3. add logging and diagnostics
4. validate end-to-end workflow

Expected output:

* hold F12
* speak
* release
* transcribed and cleaned text types into active app

### Phase 7: Hardening

Tasks:

1. improve error messages
2. add doctor checks
3. add config validation
4. optimize startup and model loading
5. add graceful shutdown

Expected output:

* stable MVP ready for real usage

---

## 15. Agent Rules for Implementation

AI coding agents should follow these rules:

1. Keep changes modular.
2. Prefer compiling code frequently.
3. Use minimal placeholder logic first, then replace iteratively.
4. Do not attempt streaming transcription in MVP.
5. Keep Linux support first-class.
6. Treat Wayland restrictions as a known limitation, not an implementation failure.
7. Ensure LLM rewrite is optional and can be disabled.
8. Always fall back to raw Whisper output if rewrite fails.
9. Avoid unnecessary abstraction in MVP.
10. Prioritize end-to-end usability over architectural perfection.

---

## 16. Acceptance Criteria

The MVP is complete when all of the following are true:

* `cargo build --release` succeeds
* `typecho config init` works
* `typecho doctor` reports useful diagnostics
* `typecho transcribe --file sample.wav` returns text
* pressing F12 starts recording
* releasing F12 stops recording
* speech is transcribed locally with Whisper
* LLM rewrite can be enabled/disabled
* final text is typed into the active application
* failure cases are understandable and recoverable

---

## 17. Future Extensions

Post-MVP roadmap:

1. streaming transcription
2. local LLM rewrite via Ollama
3. smarter prompt-mode for AI coding
4. clipboard paste fallback
5. tray icon / lightweight GUI
6. multilingual auto-detection
7. model auto-download
8. punctuation mode / code mode
9. noise suppression
10. word-by-word live preview

---

## 18. Suggested First Deliverable

The first implementation milestone should be:

```text
CLI + config + Whisper WAV transcription
```

Reason:

* easiest to validate
* isolates model integration first
* reduces debugging complexity before microphone/hotkey/system-input integration

---

## 19. Suggested GitHub README Tagline

```text
TypechoAI — Cross-platform AI voice dictation powered by Whisper, LLM cleanup, and system typing.
```

---

## 20. Final Summary

TypechoAI is a Rust-based AI voice dictation tool with this workflow:

```text
Hotkey
→ Record audio
→ Whisper transcription
→ LLM cleanup
→ Type text into current app
```

Recommended MVP priorities:

```text
1. CLI + config
2. Whisper file transcription
3. microphone recording
4. hotkey
5. system typing
6. full integration
```

This spec is intentionally designed so AI coding agents can implement the project incrementally and verify progress at each step.