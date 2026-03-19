# SPEC 文档总入口（AI First）

最后更新：2026-03-19

此文档为 AI 编程工具提供统一导航，优先指向结构化、可追踪、稳定命名的技术文档。

## 1. 需求层

- `requirements/BRD.md`
- `requirements/PRD.md`
- `traceability/requirements-to-implementation.md`

## 2. 设计与架构层

- `design/system-design-v1.md`
- `architecture/technical-solution-v1.md`
- `architecture/status-bar-menu-sync-plan-v1.md`
- `architecture/performance-optimization-roadmap-v1.md`
- `adr/README.md`

## 3. 接口与运维层

- `api/README.md`
- `operations/runbook.md`

## 4. 变更与审计

- `changes/changelog-20260319.md`
- `PROMPT-QA-LOG.md`

## 5. 人类友好文档映射

- `human-doc/BRD.md`
- `human-doc/PRD.md`
- `human-doc/TECH.md`
- `human-doc/USER-GUIDE.md`
- `human-doc/CHANGE-LOG.md`

## 6. 验收与基线工具

- `scripts/run_acceptance.sh`
- `scripts/perf_baseline.py`

## 维护约束

1. 当文档路径或命名变化时，必须立即更新本入口。
2. 当设计或架构版本升级时，更新本入口中的版本号。
3. 每次代码迭代后，同步维护 `changes/` 与 `traceability/`。
4. 变更日志文件必须使用 `changelog-YYYYMMDD.md`。
5. 架构计划从“实施中”转为“已完成”时，需同步更新 `design/`、`architecture/`、`adr/` 与 `changes/` 的状态描述。
