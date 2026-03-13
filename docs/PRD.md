# TypechoAI

**AI Voice Dictation Tool (Whisper + LLM + System Typing)**

TypechoAI 是一个 **跨平台 AI 语音输入工具**，允许用户通过语音输入文本到任意应用程序。

核心体验：

```
按住 F12
↓
说话
↓
Whisper 语音识别
↓
LLM 自动整理文本
↓
自动输入到当前光标
```

主要使用场景：

* AI Coding（Cursor / Claude Code / Codex）
* 写 Prompt
* 写文档
* 写 commit message
* 语音输入任何文本

---

# 1 项目目标

实现一个 **跨平台 AI 语音输入 CLI 工具**：

```
TypechoAI
```

CLI 命令：

```
typecho
```

核心功能：

```
Voice → Text → AI Rewrite → System Typing
```

支持平台：

```
Linux
macOS
Windows（后续）
```

---

# 2 系统架构

整体架构：

```
Hotkey Listener
      │
      ▼
Audio Recorder
      │
      ▼
Whisper Speech-to-Text
      │
      ▼
LLM Text Rewriter
      │
      ▼
System Input Simulator
```

数据流：

```
Microphone
↓
Audio Buffer
↓
Whisper STT
↓
Raw Text
↓
LLM Rewrite
↓
Clean Text
↓
Keyboard Typing
```

---

# 3 技术栈（Rust）

推荐 Rust crate：

| 功能         | Crate           |
| ---------- | --------------- |
| 音频录制       | `cpal`          |
| 音频处理       | `hound`         |
| Whisper 推理 | `whisper-rs`    |
| HTTP 请求    | `reqwest`       |
| 系统输入       | `enigo`         |
| 全局热键       | `global-hotkey` |
| CLI        | `clap`          |
| 日志         | `tracing`       |

核心组合：

```
cpal
whisper-rs
reqwest
enigo
global-hotkey
```

---

# 4 项目结构

推荐项目结构：

```
typechoai/
│
├─ Cargo.toml
│
├─ src/
│   ├─ main.rs
│   ├─ config.rs
│
│   ├─ audio/
│   │   ├─ recorder.rs
│   │   └─ buffer.rs
│
│   ├─ stt/
│   │   └─ whisper.rs
│
│   ├─ llm/
│   │   └─ rewrite.rs
│
│   ├─ input/
│   │   └─ keyboard.rs
│
│   └─ hotkey/
│       └─ listener.rs
│
└─ models/
    └─ ggml-small.bin
```

模块职责：

| 模块     | 功能           |
| ------ | ------------ |
| audio  | 麦克风录音        |
| stt    | Whisper 语音识别 |
| llm    | 文本整理         |
| input  | 系统输入         |
| hotkey | 快捷键监听        |

---

# 5 功能模块设计

## 5.1 Hotkey Listener

监听全局快捷键。

默认：

```
F12
```

行为：

```
F12 press → start recording
F12 release → stop recording
```

Rust crate：

```
global-hotkey
```

接口：

```rust
start_listener(on_press, on_release)
```

---

## 5.2 Audio Recorder

负责从系统麦克风录音。

crate：

```
cpal
```

录音格式：

```
sample rate: 16000
channels: mono
format: i16
```

接口：

```rust
start_recording()

stop_recording()

get_audio_buffer()
```

---

## 5.3 Whisper Speech-to-Text

使用：

```
whisper-rs
```

底层：

```
whisper.cpp
```

加载模型：

```
ggml-small.bin
```

接口：

```rust
fn transcribe(audio_buffer) -> String
```

输出：

```
raw_text
```

示例：

```
帮我写一个python函数算斐波那契
```

---

# 5.4 LLM Rewrite

Whisper 输出通常：

```
没有标点
有口头语
语句不自然
```

需要 LLM 整理文本。

输入：

```
raw_text
```

输出：

```
clean_text
```

示例：

```
raw:
帮我写一个python函数算斐波那契

clean:
帮我写一个 Python 函数计算斐波那契数列。
```

实现：

```
reqwest → LLM API
```

支持：

```
OpenAI
Ollama
Local LLM
```

Prompt：

```
Rewrite the following speech recognition text.

Rules:
- remove filler words
- add punctuation
- keep original meaning
- do not add new content

Text:
{raw_text}
```

---

# 5.5 System Input

模拟键盘输入文本。

crate：

```
enigo
```

接口：

```rust
type_text(text)
```

行为：

```
simulate keyboard typing
```

效果：

```
文本出现在当前光标位置
```

---

# 6 配置系统

配置文件：

```
~/.typechoai/config.toml
```

示例：

```toml
[hotkey]
key = "F12"

[whisper]
model = "small"

[llm]
provider = "openai"
model = "gpt-4o-mini"

[input]
typing_delay = 5
```

---

# 7 跨平台设计

支持系统：

```
Linux
macOS
Windows
```

Rust 编译方式：

```
cargo build --release
```

发布版本：

| 平台          | Binary            |
| ----------- | ----------------- |
| Linux       | typecho-linux     |
| macOS ARM   | typecho-macos-arm |
| macOS Intel | typecho-macos     |
| Windows     | typecho.exe       |

Whisper 模型：

```
ggml models
```

跨平台通用。

---

# 8 性能目标

目标延迟：

| 模块          | 延迟      |
| ----------- | ------- |
| Whisper STT | < 600ms |
| LLM rewrite | < 500ms |
| 总延迟         | < 1.2s  |

体验目标：

```
接近 SuperWhisper
```

---

# 9 MVP 实现范围

第一版实现：

```
Hotkey listener
Audio recording
Whisper STT
LLM rewrite
System typing
```

流程：

```
F12 press
↓
record audio
↓
F12 release
↓
transcribe
↓
rewrite
↓
type text
```

---

# 10 AI Agent 任务拆分

AI 编程工具可以按模块实现。

### Task 1

实现：

```
audio recorder
```

crate：

```
cpal
```

---

### Task 2

实现：

```
whisper inference
```

crate：

```
whisper-rs
```

---

### Task 3

实现：

```
global hotkey listener
```

crate：

```
global-hotkey
```

---

### Task 4

实现：

```
LLM rewrite module
```

crate：

```
reqwest
```

---

### Task 5

实现：

```
system typing
```

crate：

```
enigo
```

---

# 11 用户体验

用户在：

```
Cursor
VSCode
Terminal
Browser
```

操作：

```
按住 F12
↓
说：
帮我写一个Rust函数解析JSON
```

结果：

```
帮我写一个 Rust 函数解析 JSON。
```

自动输入。

---

# 12 未来功能

未来可以增加：

### Streaming Whisper

```
实时语音识别
```

延迟：

```
200-400ms
```

---

### 本地 LLM

支持：

```
Ollama
```

实现：

```
完全离线
```

---

### Prompt Mode

专门用于 AI coding：

```
voice → prompt
```

例如：

```
帮我写一个Rust HTTP服务器
```

---

# 13 最终目标

构建一个：

```
Cross-platform AI Voice Dictation Tool
```

技术组合：

```
Whisper
+
LLM Rewrite
+
System Typing
```

目标体验：

```
SuperWhisper
+
AI Prompt Optimization
```

---

如果你愿意，我还可以 **再帮你做一个 AI Agent 专用版本**，把这个文档升级成：

```
TypechoAI Agent Spec
```

包含：

* 完整 `Cargo.toml`
* Rust skeleton
* 每个模块伪代码
* CloudCode / Codex 执行流程

这样 **Claude Code / Codex 基本可以一键生成整个项目**。
