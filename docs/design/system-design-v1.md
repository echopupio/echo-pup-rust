# 系统设计文档 v1 - echo-pup-rust

最后更新：2026-03-19

## 1. 设计目标

本设计聚焦三项核心目标：
- 语音输入链路稳定：热键触发录音 -> 转写 -> 可选整理 -> 自动输入
- 后台运行可感知：状态栏、通知、提示音多通道反馈
- 多入口一致：TUI 与状态栏菜单共享同一业务规则，避免行为分叉

## 2. 用户流程

1. 用户执行 `echopup start` 后台启动语音输入服务。
2. 用户在任意应用按住热键开始录音，松开后触发识别与自动输入。
3. 用户通过 `echopup ui` 或状态栏菜单修改配置、下载模型、保存配置。
4. 发生异常（热键不合法、下载失败、识别失败）时，界面和日志给出可定位提示。

## 3. 组件模型

| 组件 | 职责 | 对外接口 |
| --- | --- | --- |
| `src/main.rs` | 进程入口、命令路由、主流程编排 | `echopup run/start/stop/status/restart/ui` |
| `src/ui.rs` | TUI 菜单、输入编辑、下载进度显示 | `run_ui(config_path)` |
| `src/status_indicator.rs` | macOS 菜单栏状态展示与交互承载 | `StatusIndicatorClient` / `status-indicator` |
| `src/config/config.rs` | 配置加载、默认值、保存落盘 | `Config::load/save/default` |
| `src/hotkey/listener.rs` | 热键监听与安全策略校验 | `HotkeyListener` / `validate_hotkey_config` |
| `src/audio/*` | 录音采集与缓冲 | `AudioRecorder` |
| `src/stt/*` | Whisper 转写与文本后处理 | `WhisperSTT` / `TextPostProcessor` |
| `src/input/keyboard.rs` | 模拟键盘输入 | `Keyboard::type_text` |
| `src/runtime.rs` | 单实例锁、后台进程控制、模型目录 | `InstanceGuard` / `model_dir` |

## 4. 数据流

核心语音链路：
1. 热键按下 -> 开始录音
2. 热键松开或 VAD 结束 -> 获取音频缓冲
3. Whisper 转写文本
4. 可选 LLM 重写与谐音纠错
5. 键盘输入到当前焦点应用
6. 反馈通道更新（状态栏、通知、提示音）

配置链路：
1. UI / 状态栏读取配置快照。
2. 用户执行编辑或开关动作更新内存态。
3. 用户触发“保存配置”后统一写入 `~/.echopup/config.toml`。

## 5. 异常与边界处理

- 异常场景：
  - 麦克风无数据或权限不足
  - Whisper 模型缺失或损坏
  - 热键配置非法或过宽
  - 下载无进度卡死或网络中断
- 恢复策略：
  - 关键错误写日志并推送可见提示
  - 下载失败自动重试，支持 `.part` 续传
  - 热键配置变更前执行校验并阻断非法值

## 6. 可观测性与指标

当前以日志为主：
- 启动日志：配置、模块初始化、监听模式
- 关键状态：开始录音、结束录音、识别中、识别完成/失败
- 下载日志：请求范围、重试次数、已下载大小、保存结果
- 性能埋点：`stt_ms`、`llm_ms`、`postprocess_ms`、`type_ms`、`e2e_ms`

## 7. 测试策略

- 单元测试：
  - 热键解析与校验
  - 菜单行为与下载事件处理
- 集成测试：
  - `run/start/stop/status/restart` 与 `ui *` 生命周期
- 端到端测试：
  - macOS 后台运行下热键录音与跨应用文本输入
  - 状态栏反馈与下载流程可视化校验

## 8. 关联文档

- `docs/requirements/PRD.md`
- `docs/architecture/technical-solution-v1.md`
- `docs/architecture/status-bar-menu-sync-plan-v1.md`
- `docs/architecture/performance-optimization-roadmap-v1.md`
- `docs/traceability/requirements-to-implementation.md`
