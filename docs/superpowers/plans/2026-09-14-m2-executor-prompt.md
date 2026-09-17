# M2 执行提示词（交给执行 Agent；完成后由独立评审 review）

## 角色与对象

你是 MAM 项目的实施工程师，负责执行 M2 里程碑（远程接入骨架 + 局域网移动看板）。
工作目录：`/Users/jarvis/Documents/mam-worktree2`
执行对象：`docs/superpowers/plans/2026-09-14-m2-remote-access-board.md`（8 个任务，严格 Task 1 → Task 8 顺序，不跳步、不合并任务；计划含 2026-09-14 事后核查修订 15 处，以修订后文本为准）。

## 第〇步：分支与基线（必须最先做）

当前分支是 feat/m1-dsh-adapter（M1 正在走 PR，不要动它），工作区有未提交的 M2 文档。按序：

```bash
cd /Users/jarvis/Documents/mam-worktree2
git checkout -b feat/m2-remote-board   # 从当前 HEAD 切出（含 M1 代码——M2 的 sessions 端点依赖八工具聚合与单飞护栏）
git add docs/superpowers/plans/2026-09-14-m2-remote-access-board.md \
        docs/superpowers/plans/2026-09-14-m2-executor-prompt.md \
        docs/superpowers/specs/2026-09-12-remote-access-level1-design.md
git commit -m "docs(spec): M2 实施计划（含事后核查修订）+ spec 进度基线"
git fetch origin
# 若 origin/main 已包含 M1（PR 已合）：git rebase origin/main（冲突停下报告）
# 若未合：保持现基，M1 合并后再 rebase（记入完成报告）
```

## 铁律（违反任何一条：立即停止并报告，不要自行变通）

1. **TDD 顺序不可跳**：先写失败测试 → 跑确认失败 → 实现 → 跑确认通过 → conventional commit。
2. **范围红线（M2 只做接入骨架+局域网看板）**：不做审批配对/设备花名册 UI/隧道（quick/named）/SSE/推送/电源锁/托盘入口/APK——全部 M3-M5（计划 Global Constraints 有完整清单）。
3. **测试零污染**：不写真实 `~/.mam/mam.db`（用计划提供的 `DeviceStore::memory()` 注入）、不写 `~/.dsh`、不碰 M1 的 dsh 监控代码。
4. **不动宪法**：不修改 `docs/MASTER-PLAN.md` 与 `AGENTS.md`。
5. **构建顺序**：`cargo check/test` 前先 `pnpm build:mobile`（rust-embed 需 dist-mobile 存在；fresh clone 只有 .gitkeep 占位，此时 /m 返回 404 属预期——计划 Task 1 已布好占位）。
6. **测试位置**：前端测试一律放 `tests/mobile/`（vitest include 只收 tests/**，放 src/ 下是假绿）。
7. 每 Task 结束 `cargo test` + 涉前端时 `pnpm build && pnpm build:mobile && pnpm check:i18n` 全绿才 commit；**不 push**。
8. 中文注释、英文标识符（见 AGENTS.md）。

## 参考文档（按需查阅原文，勿凭记忆）

| 文档 | 用途 |
|---|---|
| `docs/superpowers/specs/2026-09-12-remote-access-level1-design.md` | 需求唯一来源（v5，P6 最简版/P7 直连/P8 + 里程碑表 + 进度附录） |
| `research/refs/phase1-接入层/一期实现要点-技术备忘.md` §B | 接入层实现要点（模块结构/API 面/配对语义） |
| `research/refs/phase1-接入层/dsh-remote-web-ui-源码摘要.md` | 配对七不变量（Task 2 的语义权威） |
| M1 分支的 `src-tauri/src/adapter/mod.rs` | `get_all_sessions`（sessions 端点直调它，R3 单飞护栏已就位） |

## 关键接口对齐点（以现有代码为准）

- `crate::database::dao::settings::{get_setting, set_setting}`（KV 字符串）
- `crate::database::connection::DB`（全局 `Lazy<Mutex<Connection>>`，lock() 返回 Result）
- `crate::adapter::get_all_sessions()`（sessions 端点唯一数据源，禁止复制聚合逻辑）

## Task 8 的边界

Step 1 门禁（cargo test/clippy + pnpm check 全量）照常执行；Step 2 的七场景局域网实测（手机扫码/token 一次性/停止/重启/TLS 门/桌面回归）**需要用户与手机配合——不要假装做过**，在完成报告里列为"待用户+评审执行的手动项"。

## 完成后输出（供独立评审 review）

按 Task 1–8 逐条报告：①完成状态（完成/部分+原因/停止+触发的铁律）②测试结果（命令+关键输出摘要，失败也如实贴）③commit hash 与 message ④与计划的偏离及理由（无则写"无"）。
最后附：`git log --oneline`（本分支全部提交）、`cargo test` 全量摘要、`pnpm build && pnpm build:mobile && pnpm check:i18n` 结果、分支基线状态（是否已 rebase 到含 M1 的 main）。
