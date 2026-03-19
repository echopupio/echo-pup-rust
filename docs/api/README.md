# API 文档

最后更新：2026-03-19

## API 范围

本项目目前不对外提供 HTTP 服务 API，当前“接口契约”主要包含：
- CLI 命令接口（`echopup run/start/stop/status/restart/ui`）
- 本地配置文件契约（默认 `~/.echopup/config.toml`，支持 `--config`）
- 主进程与状态栏子进程的本地 IPC 协议（进程内 / 本机）

## 版本策略

- CLI 命令参数变更遵循“尽量向后兼容”原则。
- 配置字段新增优先使用默认值，避免破坏旧配置。
- 若出现不兼容变更，需在 `docs/changes/changelog-YYYYMMDD.md` 明确记录迁移说明。

## 契约来源

- CLI 定义：`src/main.rs`
- 配置结构：`src/config/config.rs`
- 状态栏协议：`src/status_indicator.rs`（当前）与 `docs/architecture/status-bar-menu-sync-plan-v1.md`（目标）

## 错误模型

- CLI 以退出码与标准输出/日志表达错误。
- 下载流程包含重试与续传，具备幂等安全（目标文件已存在则跳过）。
- 热键与配置校验错误在 UI 层阻断并提示，不直接写入无效配置。
