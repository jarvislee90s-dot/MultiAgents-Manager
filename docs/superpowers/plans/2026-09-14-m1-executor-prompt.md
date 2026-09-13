# M1 执行提示词（交给执行 Agent；执行完成后由独立评审负责 review）

## 角色与对象

你是 MAM 项目的实施工程师，负责执行 M1 里程碑（dsh 监控底座）的实施计划。
工作目录：`/Users/jarvis/Documents/mam-worktree2`。
执行对象：`docs/superpowers/plans/2026-09-14-m1-dsh-monitoring-adapter.md`（10 个任务，严格按 Task 1 → Task 10 顺序执行，不跳步、不合并任务）。

## 第〇步：同步主分支（必须最先做，云端 main 已更新）

本地当前有未提交的设计文档，按以下顺序处理（防止丢失）：

```bash
cd /Users/jarvis/Documents/mam-worktree2
git add AGENTS.md docs/MASTER-PLAN.md \
  docs/superpowers/specs/2026-09-12-remote-access-level1-design.md \
  docs/superpowers/plans/2026-09-14-m1-dsh-monitoring-adapter.md \
  docs/superpowers/plans/2026-09-14-m1-executor-prompt.md
git commit -m "docs(spec): 一期设计 v4 定稿 + M1 实施计划 + 项目宪法条款"
git fetch origin
git pull --rebase origin main
# rebase 冲突时：仅 docs 文本冲突可手工合并保留双方内容；代码/配置冲突一律停下报告
git checkout -b feat/m1-dsh-adapter   # 全部开发工作在此分支
```

## 铁律（违反任何一条：立即停止并报告，不要自行变通）

1. **TDD 顺序不可跳**：每个任务先写失败测试 → 跑确认失败 → 实现 → 跑确认通过 → conventional commit。
2. **只读红线**：不写 `~/.dsh` 下任何文件；不调用 dsh 的 HTTP API；不安装任何 dsh 插件。
3. **不动宪法**：不修改 `docs/MASTER-PLAN.md`（修改需用户在场明确同意）。
4. **探测事实冲突即停**：实现中发现与 `research/dsh-probe-2026-09-13-report.md`（事实 F1–F16）不符的现象，停下记录现象与证据，**不要猜测 dsh 行为、不要绕过**。
5. 每个任务结束 `cargo test` 全绿才 commit；**不 push**（评审通过后由用户决定）。
6. 代码注释用中文、标识符用英文（项目规范，见 AGENTS.md）。

## 参考文档（按需查阅原文，勿凭记忆）

| 文档 | 用途 |
|---|---|
| `docs/superpowers/specs/2026-09-12-remote-access-level1-design.md` | 需求唯一来源（v4 定稿，P1–P12 + 证据台账） |
| `research/dsh-probe-2026-09-13-report.md` | dsh 探测事实 F1–F16（技术事实唯一来源） |
| `research/dsh-probe-2026-09-13-review.md` | 独立评审（其修正已并入计划，冲突时以此为准） |
| `research/refs/phase1-接入层/一期实现要点-技术备忘.md` | 实现要点（§A = dsh adapter） |
| `research/dsh-probe-2026-09-13-evidence/sample*.sanitized.jsonl` | 5 个黄金测试夹具（Task 3 拷入 `src-tauri/tests/fixtures/dsh/`） |

## 计划已注明的两处对齐点（以现有代码为准，照抄现有惯例）

1. `crate::database::connection` 的取连接函数名/返回类型（看 dao 其他调用方怎么写）。
2. `crate::monitor::project::project_name_from_path` 的真实签名（zcode_parser 有同款调用可抄）。

## Task 10 的边界

验收任务中：Step 1（全量检查 cargo test/clippy/pnpm build/lint）照常执行；Step 2–5 涉及真机手动场景（五状态实测/跳转点击/回归观察），**不要假装做过**——在完成报告里列为"待用户+评审执行的手动项"。

## 完成后输出（供独立评审 review）

按 Task 1–10 逐条报告：

1. 完成状态（完成 / 部分完成+原因 / 停止+触发的那条铁律）
2. 测试结果（跑过的命令 + 关键输出摘要，失败也要如实贴）
3. commit hash 与 message
4. 与计划的任何偏离及理由（没有写"无"）

最后附三样：`git log --oneline`（本分支全部提交）、`cargo test` 全量摘要（含 dsh 相关测试数）、`pnpm build` 结果。
