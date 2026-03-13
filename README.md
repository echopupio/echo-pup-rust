# TypechoAI

TypechoAI 是一个跨平台 AI 语音输入工具，目标是把「按住热键说话」变成一条足够顺滑的输入链路：

```text
按住 F12
-> 说话
-> 本地 Whisper 语音识别
-> 可选 LLM 文本整理
-> 自动输入到当前光标所在应用
```

它面向需要高频文本输入的场景，例如 AI Coding、写 Prompt、写文档、写提交说明，以及任意应用中的通用语音输入。

## 项目目标

项目目标是实现一个以 Rust 为主的 CLI 工具，提供这条核心链路：

```text
Voice -> Text -> AI Rewrite -> System Typing
```

目标平台：

- Linux 优先
- macOS 次之
- Windows 后续支持

## MVP 范围

MVP 计划覆盖以下能力：

- 全局热键监听
- 按住说话录音
- 本地 Whisper 转写
- 可选 LLM 文本清理/润色
- 将最终文本自动输入到当前聚焦应用
- 配置文件支持
- CLI 命令支持 `run` / `test` / `config`

当前不在 MVP 范围内：

- 流式转写
- GUI
- 账号系统
- 云同步
- 高级设置界面

## 工作流程

```text
用户按下 F12
-> 开始录音
-> 用户松开 F12
-> 停止录音
-> Whisper 转写音频
-> 得到 raw_text
-> 可选 LLM 整理文本
-> 得到 clean_text
-> 模拟键盘输入到当前应用
```

## 技术方案

核心模块：

- Hotkey Listener
- Audio Recorder
- Whisper STT
- LLM Rewrite
- System Input

推荐 Rust crate：

| 能力 | 推荐 crate |
| --- | --- |
| CLI | `clap` |
| 配置 | `serde`, `toml` |
| 日志 | `tracing`, `tracing-subscriber` |
| 音频采集 | `cpal` |
| 音频调试 | `hound` |
| Whisper 推理 | `whisper-rs` |
| HTTP 请求 | `reqwest` |
| 全局热键 | `global-hotkey` |
| 键盘输入模拟 | `enigo` |
| 错误处理 | `anyhow`, `thiserror` |
| 异步运行时 | `tokio` |

## 计划中的项目结构

```text
typechoai/
├─ Cargo.toml
├─ README.md
├─ docs/
├─ models/
│  └─ ggml-small.bin
├─ src/
│  ├─ main.rs
│  ├─ cli.rs
│  ├─ config.rs
│  ├─ app.rs
│  ├─ audio/
│  ├─ hotkey/
│  ├─ stt/
│  ├─ llm/
│  ├─ input/
│  └─ utils/
└─ .typechoai/
   └─ config.toml.example
```

## 配置示例

默认配置路径：

```text
~/.typechoai/config.toml
```

示例：

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
```

## 仓库状态

当前仓库还处于设计/规划阶段，现有内容以产品文档和实现规格为主，代码骨架尚未落地完成。

如果你想快速了解设计细节，优先看下面两份文档：

- [产品需求文档](./docs/PRD.md)
- [Agent 实现规格](./docs/TYPECHOAI_AGENT_SPEC.md)

## Roadmap

- [ ] 搭建 Rust 工程骨架
- [ ] 完成 Linux 下的热键监听与录音链路
- [ ] 接入本地 Whisper 转写
- [ ] 接入可选 LLM 文本整理
- [ ] 完成系统级文本注入
- [ ] 增加配置、日志与调试命令
- [ ] 验证 macOS 支持
