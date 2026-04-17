# 运维手册 - echo-pup-rust

最后更新：2026-04-17

## 1. 服务责任

- 主负责人：项目维护者
- 备份负责人：核心贡献者
- 值班沟通渠道：仓库 Issue / 内部 IM 群

## 2. 发布流程

1. 发布前置条件
   - `cargo test` 通过
   - 核心命令手工回归通过：`run/start/stop/status/restart`、`ui *`
2. 发布步骤
   - `cargo build --release`
   - 备份旧二进制
   - 部署新二进制到运行路径
3. 验证检查
   - `echopup status` 返回运行状态
   - 检查日志 `~/.echopup/echopup.log`
   - 验证热键触发录音与识别输入链路

## 3. 回滚流程

1. 触发条件
   - 新版本出现关键功能不可用（热键失效、无法识别、无法输入）
2. 回滚步骤
   - 执行 `echopup stop`
   - 还原上一版二进制
   - 执行 `echopup start`
3. 回滚后验证
   - 再次执行 `echopup status`
   - 验证日志与基础语音输入能力恢复

## 4. 事故处理

- 告警来源：
  - 用户反馈、日志错误、功能回归
- 严重级别：
  - S1：主功能不可用（无法录音/输入）
  - S2：部分能力异常（例如状态反馈异常）
- 升级路径：
  - 维护者初步定位 -> 必要时回滚 -> 补丁修复

## 5. SLO 与监控

- 可用性目标：
  - 日常场景下主链路可用（录音 -> 识别 -> 输入）
- 错误预算策略：
  - 以版本迭代为单位统计关键故障次数并复盘
- 核心看板：
  - 当前以本地日志为主，后续可引入结构化指标

## 6. 常用诊断命令

```bash
echopup status
echopup ui status
tail -n 200 ~/.echopup/echopup.log
echo "$XDG_SESSION_TYPE"
echo "$XDG_CURRENT_DESKTOP"
command -v wtype
command -v xdotool
gdbus introspect --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop
```

## 7. Wayland 排障要点

1. 先确认当前是否为 Wayland：
   - `echo "$XDG_SESSION_TYPE"`
2. 若是 Wayland，不要再默认假设应用内全局热键一定可用；优先检查当前是否已经采用桌面快捷键绑定到 EchoPup 的外部触发接口。
3. 文本提交失败时优先检查：
   - `wtype` 是否存在于 `PATH`
   - 日志中最终选中的 text commit backend 是什么
4. 若需检查 portal 能力，优先查看：
   - `org.freedesktop.portal.RemoteDesktop`
   - `org.freedesktop.portal.InputCapture`
   - `org.freedesktop.portal.GlobalShortcuts` 是否存在
5. 对外沟通时避免把“Wayland 无法识别热键”直接表述为单点 bug；优先按平台边界问题定位。
