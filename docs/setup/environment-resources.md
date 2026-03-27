# 环境与资源说明

本文件由少府主理，用于记录项目的工具链、环境要求、常用命令、模板与资源清单。

## 一、基础环境

- 操作系统：macOS 或 Linux（Linux 需图形会话）
- Shell：POSIX shell；项目脚本以 `bash` 为主，日常命令兼容 `zsh`
- 语言运行时：Rust 1.70+
- 包管理器：Cargo
- 构建工具：`cargo build` / `cargo build --release`
- 测试工具：`cargo test`、`./target/release/echopup test`、`./scripts/run_acceptance.sh`

## 二、常用命令

| 类型 | 命令 | 用途 | 维护官员 |
| --- | --- | --- | --- |
| 初始化百官 | `<zhou-tianzi-skill-root>/scripts/init_zhou_tianzi.sh .` | 建立治理骨架 | 丞相 / 少府 |
| 巡牧总账 | `<zhou-tianzi-skill-root>/scripts/inspect_court_state.sh .` | 状态巡检 | 司徒 |
| 编译构建 | `cargo build --release` | 生成发布构建 | 大司马 / 少府 |
| 单元与集成测试 | `cargo test` | 基础回归验证 | 司寇 |
| 运行内建自测 | `./target/release/echopup test` | 模块级功能检查 | 司寇 |
| 验收脚本 | `./scripts/run_acceptance.sh` | 状态栏菜单与 TUI 对齐验收 | 司寇 |
| 性能基线 | `./scripts/perf_baseline.py --limit 200` | 汇总最近性能埋点 | 司寇 / 少府 |
| 文档治理 | `<zhou-tianzi-skill-root>/libu/scripts/quick_command.py audit .` | 礼部体检与同步 | 大司礼 |

## 三、模板与资产

- ADR 模板：`docs/templates/adr-template.md`
- 巡检模板：`docs/templates/report-template.md`
- 项目评审入口：`docs/reviews/README.md`
- 会话总账与名册：`docs/reports/`

## 四、维护约束

- 新增工具链前，先写明用途、版本、依赖与回退方式。
- 少府负责器用与资源，不代替司空做架构裁决。
- 任何会影响协作方式的命令、模板或项目原生自动化脚本变动，都应同步更新本文件。
