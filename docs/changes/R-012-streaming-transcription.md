# R-012: 流式转写预览

## 需求概述

**需求描述**: 录音过程中实时输出已识别文本（流式转写预览）

**验收标准**:
- 录音进行中可持续看到增量识别文本
- 停录后结果与增量内容一致

**优先级**: 中

---

## 技术分析

### 现有实现

| 模块 | 现状 |
|------|------|
| `src/stt/whisper.rs` | 使用 `full()` 一次性转写整个音频 |
| `src/main.rs` | 录音结束后才调用 `process_audio()` 转写 |
| `src/audio/recorder.rs` | 使用 `Arc<Mutex<Vec<f32>>>` 存储录音数据 |
| whisper-rs | 支持 `set_single_segment(true)` + `set_new_segment_callback` 进行流式转写 |

### 核心挑战

1. **Whisper 模型特性**: 需积累一定音频才能产出有效识别结果（首字延迟）
2. **线程安全**: WhisperSTT 使用内部 state，需处理并发访问
3. **状态栏更新频率**: 避免过于频繁更新导致 UI 卡顿
4. **增量 vs 最终结果一致性**: 增量文本可能与最终结果有差异

---

## 实现方案

### 方案 A: 后台线程增量转写（推荐）

在录音过程中，启动后台线程周期性获取 `audio_buffer` 快照并进行增量转写：

```
录音线程 → audio_buffer (共享)
              ↓ 周期性快照
         后台转写线程 → WhisperSTT::transcribe()
              ↓ 增量文本
         StatusIndicatorClient::send_snapshot()
              ↓
         状态栏显示部分文本
```

**优点**: 不影响主录音流程，实现相对简单
**缺点**: 会有重复计算（已识别部分可能被重复处理）

### 方案 B: Whisper 回调模式

使用 whisper-rs 的 `set_new_segment_callback` 回调：

```rust
// 设置单段模式
params.set_single_segment(true);

// 设置新段回调
params.set_new_segment_callback(|segment| {
    // 每次产生新段时调用
    let text = segment.text;
    // 发送到状态栏
});
```

**优点**: Whisper 原生支持，效率更高
**缺点**: 需要修改 WhisperSTT 内部实现

---

## 实现步骤

### 阶段 1: WhisperSTT 流式能力

- [ ] 1.1 在 `src/stt/whisper.rs` 添加 `transcribe_streaming()` 方法
- [ ] 1.2 使用 `set_single_segment(true)` 启用单段模式
- [ ] 1.3 实现或封装 `set_new_segment_callback` 回调
- [ ] 1.4 确保线程安全（考虑 Arc + Mutex 或专属转写线程）

### 阶段 2: 录音流程改造

- [ ] 2.1 在 `src/audio/recorder.rs` 添加获取快照方法 `get_snapshot()`
- [ ] 2.2 在 `src/main.rs` 录音开始时启动后台转写线程
- [ ] 2.3 后台线程周期性（如每 500ms）获取 audio_buffer 快照
- [ ] 2.4 调用流式转写，收集增量文本
- [ ] 2.5 录音结束时停止后台线程，合并最终结果

### 阶段 3: 状态栏显示

- [ ] 3.1 扩展 `IndicatorState` 添加 `TranscribingPartial { text: String }` 状态
- [ ] 3.2 修改 `send_snapshot()` 支持增量文本更新
- [ ] 3.3 在 macOS 状态栏显示部分文本
- [ ] 3.4 在 Linux 菜单状态行显示部分文本

### 阶段 4: 优化与测试

- [ ] 4.1 首字延迟优化（调整触发阈值）
- [ ] 4.2 增量文本与最终结果一致性验证
- [ ] 4.3 性能测试（CPU/内存）
- [ ] 4.4 手工验收测试

---

## 涉及代码文件

| 文件 | 改动说明 |
|------|----------|
| `src/stt/whisper.rs` | 添加流式转写方法 |
| `src/stt/mod.rs` | 导出新方法 |
| `src/audio/recorder.rs` | 添加 `get_snapshot()` |
| `src/main.rs` | 启动后台转写线程 |
| `src/status_indicator.rs` | 支持增量文本显示 |

---

## 依赖

- 无需新增 crate
- whisper-rs 0.16 已支持 `set_new_segment_callback`

---

## 风险与注意事项

1. **首字延迟**: Whisper 需要积累一定音频才能识别，首字可能延迟 1-3 秒
2. **重复计算**: 每次快照都包含已识别部分，需优化避免重复
3. **状态栏更新频率**: 建议限制更新频率（如最多每秒 2 次）
4. **用户体验**: 增量文本可能与最终结果不同，需向用户说明

---

## 参考资料

- [whisper-rs 文档](https://docs.rs/whisper-rs/0.16.0)
- whisper-rs 源码: `~/.cargo/registry/src/*/whisper-rs-0.16.0/src/whisper_params.rs`
- 现有转写流程: `src/main.rs::process_audio()`

---

*本文档最后更新: 2026-03-21*
