---
name: ai-report-archive
description: Use when performing any project task that creates, changes, reviews, tests, diagnoses, researches, or documents work. Creates and maintains date-classified AI process records in 报告/ai_docs and archives non-product AI code in 报告/ai_codes.
---

# AI 报告归档

执行任何项目任务时，先读取项目根目录的 `AGENTS.md`，并将其作为最高优先级的项目归档规则。

## 强制归档流程

1. 在实际修改或执行关键操作前，检查工作区上下文，并创建计划文档：`报告/ai_docs/计划/YYYY-MM-DD/YYYY-MM-DD-任务主题.md`。
2. 任务执行期间持续更新操作文档：`报告/ai_docs/操作/YYYY-MM-DD/YYYY-MM-DD-任务主题.md`。
3. 交付前创建审计、测试和报告文档，分别位于同一日期下的 `审计`、`测试`、`报告` 目录。
4. 即使某阶段不适用或无法执行，也创建对应文档，说明原因、影响和后续处理。
5. 需求澄清、调研、关键决策、阻塞和复盘分别归档到 `需求`、`调研`、`决策`、`问题记录`、`复盘` 目录；按实际发生情况创建。

## 命名与内容

1. 日期采用任务开始日 `YYYY-MM-DD`；同日同类同名文件追加两位序号。
2. 每份文档顶部记录任务名称、日期、状态、负责人和关联文件。
3. 记录事实和脱敏摘要，不记录密钥、令牌、密码、私有连接串或个人数据。
4. 使用 UTF-8（无 BOM）和 LF 换行。

## AI 辅助代码

AI 编写且不属于项目产品、测试、构建或部署所必需的代码，保存到：

`报告/ai_codes/YYYY-MM-DD/<两位序号-任务主题>/`

每个目录必须有 `README.md`，说明用途、运行方式、输入输出、依赖及是否可安全删除。不得让此类代码被项目自动构建、发布或运行。

## 完成检查

交付前确认：

1. 计划、操作、审计、测试、报告五类文档均已创建且与实际一致。
2. 文档中的路径、变更与测试结果可追溯。
3. 不存在敏感信息、伪造结果或未说明的阻塞。
