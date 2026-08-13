---
name: dated-reporting
description: Use when planning, implementing, reviewing, testing, auditing, or reporting any project task that must produce date-organized records under 报告/ and place non-project AI helper code under 报告/ai_codes/.
---

# Dated Reporting Workflow

Apply this skill to every substantive task in this project. The root `AGENTS.md` is authoritative; this skill provides the operational checklist and document structure.

## Start a task

1. Determine the local task-start date in `YYYY-MM-DD` form.
2. Choose `报告/YYYY-MM-DD/` for the first task of that date. If it already represents another task, create `报告/YYYY-MM-DD/NN-简短任务名/` with the next two-digit sequence.
3. Before modifying project files, create `01-规划.md`. State the user request, scope, assumptions, risks, intended changes, and verification plan.
4. When the work needs scripts, parsers, migration helpers, diagnostics, or other code that is not part of the delivered project, write it only below `报告/ai_codes/YYYY-MM-DD/NN-任务主题/`. Include a `README.md` describing its purpose and deletion safety.

## During work

Maintain `02-操作.md` with material actions only: inspected context, files added/changed, decisions, commands that matter, failures, retries, and deviations from the plan. Do not include secrets or full noisy command output.

## Close a task

Create or update all of these documents even when their outcome is "not applicable":

| File | Required content |
| --- | --- |
| `03-审计.md` | Scope check, changed-file review, regressions, security/privacy concerns, unresolved risks. |
| `04-测试.md` | Commands or manual checks, expected behavior, observed result, skipped validation and reason. |
| `05-报告.md` | Outcome, deliverables, constraints, known issues, and useful next action. |

Each report begins with a compact metadata block containing task name, date, status, owner, and associated files. Keep documents factual, concise, UTF-8 without BOM, and LF-terminated.

## Completion gate

Do not claim completion until the report documents correspond to actual work and the verification status is explicit. If work is blocked, record the blocker and required user decision in both `04-测试.md` (where relevant) and `05-报告.md`.
