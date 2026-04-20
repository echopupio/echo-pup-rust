# 系统设计文档 v1 - echo-pup-rust

最后更新：2026-03-19

## 1. 设计目标

本设计聚焦三项核心目标：
- 语音输入链路稳定：热键触发录音 -> 转写 -> 可选整理 -> 自动输入
- 后台运行可感知：状态栏、通知、提示音多通道反馈
- 多入口一致：TUI 与状态栏菜单共享同一业务规则，避免行为分叉
- 触发策略可切换：支持“长按模式”与“按压切换模式”，降低误触发与误停止

## 2. 用户流程

1. 用户执行 `echopup start` 后台启动语音输入服务。
2. 用户在任意应用长按热键 1 秒开始录音（阈值固定 1s）。
3. 用户根据触发模式完成停止：
   - 长按模式：松开即停止并开始识别。
   - 按压切换模式：松开继续录音，下次按下停止并开始识别。
4. 用户通过 `echopup ui` 或状态栏菜单修改配置、下载模型、切换模型；配置自动保存并立即生效。
5. 发生异常（热键不合法、下载失败、识别失败）时，界面和日志给出可定位提示。

## 3. 组件模型

| 组件 | 职责 | 对外接口 |
| --- | --- | --- |
| `src/main.rs` | 进程入口、命令路由、主流程编排 | `echopup run/start/stop/status/restart/ui` |
| `src/ui.rs` | TUI 菜单渲染与输入捕获（业务动作委托共享内核） | `run_ui(config_path)` |
| `src/status_indicator.rs` | macOS 菜单栏状态展示、菜单交互桥接、IPC 收发 | `StatusIndicatorClient` / `status-indicator` |
| `src/menu_core.rs` | 共享菜单业务内核：动作执行、配置内存态、快照与下载事件聚合 | `MenuCore` / `MenuAction` / `MenuSnapshot` |
| `src/model_download.rs` | 共享模型下载能力：断点续传、重试、进度与日志事件 | `start_model_download` / `DownloadEvent` |
| `src/config/config.rs` | 配置加载、默认值、保存落盘 | `Config::load/save/default` |
| `src/hotkey/listener.rs` | 热键监听与安全策略校验 | `HotkeyListener` / `validate_hotkey_config` |
| `src/audio/*` | 录音采集与缓冲 | `AudioRecorder` |
| `src/stt/*` | Whisper 转写与文本后处理 | `WhisperSTT` / `TextPostProcessor` |
| `src/input/keyboard.rs` | 模拟键盘输入 | `Keyboard::type_text` |
| `src/runtime.rs` | 单实例锁、后台进程控制、模型目录 | `InstanceGuard` / `model_dir` |

## 4. 数据流

核心语音链路：
1. 热键按下后进入“候选启动”状态，计时 1 秒。
2. 达到阈值后启动录音，并进入实时反馈态（状态栏边缘脉动、提示音、通知）。
3. 触发停止条件：
   - 长按模式：当前按键释放；
   - 按压切换模式：下一次按下；
   - 或 VAD 自动结束。
4. 停止后获取音频缓冲并进入转写。
5. Whisper 转写文本。
6. 可选 LLM 重写与谐音纠错。
7. 键盘输入到当前焦点应用。
8. 状态栏展示完成/失败，并按时自动回到空闲态。

配置链路：
1. UI / 状态栏读取配置快照。
2. 用户执行编辑或开关动作更新内存态。
3. 动作完成后自动写入 `~/.echopup/config.toml` 并立即热更新运行时（热键、LLM、Whisper、触发模式）。

菜单同步链路（状态栏 <-> 主进程）：
1. 主进程启动时初始化 `MenuCore`，并向状态栏下发初始 `MenuSnapshot`。
2. 状态栏菜单操作后，通过 IPC 回传 `ActionRequest`。
3. 主进程执行动作并回传 `ActionResult` 与最新快照。
4. 下载过程通过事件轮询驱动进度和日志刷新，状态栏与 TUI 共享同一状态源。
5. 对需要复杂输入的动作，状态栏使用弹窗形态（热键捕获、LLM 表单、模型下载），并仅在流程结束后关闭弹窗。

## 5. 异常与边界处理

- 异常场景：
  - 麦克风无数据或权限不足
  - Whisper 模型缺失或损坏
  - 热键配置非法或过宽
  - 下载无进度卡死或网络中断
- 恢复策略：
  - 关键错误写日志并推送可见提示
  - 下载失败自动重试，支持 `.part` 续传
  - 代理网络失败时自动切换直连重试，并清理 0B 临时文件
  - 热键配置变更前执行校验并阻断非法值

## 6. 可观测性与指标

当前以日志为主：
- 启动日志：配置、模块初始化、监听模式
- 关键状态：开始录音、结束录音、识别中、识别完成/失败
- 下载日志：请求范围、重试次数、已下载大小、保存结果
- 触发模式状态：按键按下/释放序列、启动阈值、停止防抖窗口
- 性能埋点：`stt_ms`、`llm_ms`、`postprocess_ms`、`type_ms`、`e2e_ms`

## 7. 测试策略

- 单元测试：
  - 热键解析与校验
  - `menu_core` 动作契约与下载事件处理
  - `status_indicator` 菜单 tag 到动作映射
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
