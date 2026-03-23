# PROMPT-QA-LOG

最后更新：2026-03-23

用于审计人类提示词与 AI 执行摘要。

| 时间 | 人类输入摘要 | AI 执行动作摘要 | 关联文档/代码 |
| --- | --- | --- | --- |
| 2026-03-19 | 初始化文档基线 | 建立 `docs/` 基础结构并生成模板 | `docs/SPEC.md` |
| 2026-03-19 | 按规范整理文档并清理旧文档 | 迁移旧文档信息到规范路径，删除不规范旧文档，统一索引与命名规则 | `docs/README.md`, `docs/SPEC.md`, `docs/changes/changelog-20260319.md` |
| 2026-03-19 14:59 +0800 | 触发 `$doc-doc` 并提供技能规范，要求先展示菜单并在同意后沉淀最近会话；确认"是 10"保存最近 10 轮可见交互 | 按技能规则先返回菜单与会话沉淀确认项，收到授权后整理最近可见对话并追加审计记录，等待后续功能选择（自动探测/初始化/同步文档/迭代项目/归档当前文档） | `docs/PROMPT-QA-LOG.md` |
| 2026-03-23 15:00 +0800 | "使用 doc-doc 记录下这个评审报告"（R-013 Linux 弹窗代码评审） | 创建评审记录 `docs/reviews/review-20260323-01.md`，包含 1 个高、2 个中、2 个低级别 Findings；同步更新 `docs/traceability/requirements-to-implementation.md` 关联 R-013 | `docs/reviews/review-20260323-01.md`, `docs/traceability/requirements-to-implementation.md` |

## 记录规则

1. 只记录关键提示词与关键动作。
2. 描述要可追踪：至少能定位到文档或代码路径。
3. 避免写入敏感信息（密钥、隐私数据）。
