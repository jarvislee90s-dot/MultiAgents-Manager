# M3 执行提示词（交给执行 Agent；完成后由独立评审 review）

## 角色与对象

你是 MAM 项目的实施工程师，负责执行 M3 里程碑（看板打磨 + 实时与内容 + 文件预览）。
工作目录：`/Users/jarvis/Documents/mam-worktree2`
执行对象：`docs/superpowers/plans/2026-09-15-m3-board-realtime-content.md`（10 个任务，严格 Task 1 → Task 10 顺序，不跳步、不合并任务；计划含两轮事后核查修订——接口名已对齐项目现有代码，以修订后文本为准）。

## 第〇步：分支与基线（必须最先做）

**前置检查**：M2 分支 `feat/m2-remote-board` 必须已合并 main。检查方法：
```bash
git fetch origin
git log --oneline origin/main -5  # 应含 M2 的提交（remote/axum/pairing 等）
```
若未合并：停下报告"等待 M2 PR 合入"，不自行操作。

已合并则：
```bash
git checkout main && git pull origin main
git checkout -b feat/m3-board-realtime-content
git add docs/superpowers/plans/2026-09-15-m3-board-realtime-content.md
git commit -m "docs(spec): M3 实施计划（含两轮核查修订）"
```

**依赖安装**（计划新增依赖清单节有完整说明）：
```bash
cd src-tauri && cargo check  # 确认现有编译通过
cd .. && pnpm add react-markdown remark-gfm rehype-highlight highlight.js
```
Rust 新依赖（tokio-stream, futures）在 Task 5/6 的代码块中标注了版本，届时加入 Cargo.toml。

## 铁律（违反任何一条：立即停止并报告，不要自行变通）

1. **TDD 顺序不可跳**：先写失败测试 → 跑确认失败 → 实现 → 跑确认通过 → conventional commit。
2. **文件预览安全红线**：文件读取 API 仅限该会话 project cwd 内（canonicalize 后 starts_with 校验）、只读、文本 ≤500KB、图片 ≤5MB。违反即安全漏洞。
3. **数据同源红线**：sessions 端点直调 `adapter::get_all_sessions`，禁止复制聚合逻辑。
4. **去重红线**：全部在 SessionWatcher 跃迁判定层（`diff_transitions`）完成——SSE/推送/桌面通知只收边沿事件，移动端不独立去重。
5. **不动宪法**：不修改 `docs/MASTER-PLAN.md` 与 `AGENTS.md`。
6. **文件预览放靠后**（Task 8-9）——前面的 UI/实时/内容消息流不被文件系统不确定性阻塞。
7. **每 Task 全绿才 commit**：`cargo test` + `cargo clippy --all-targets -- -D warnings` + `pnpm build:mobile` + `pnpm check:i18n`；不 push。
8. **中文注释、英文标识符**（见 AGENTS.md）。
9. **Task 7/8 的 `todo!()` 是有意的执行时实现点**——每个都附有参照指引（"参照 monitor/xxx_parser.rs"或"参照 M0 spike"）。执行者需读现有 parser 代码后实现；格式不确定的先探测（/tmp 一次性会话验证格式），**探测冲突即停**。
10. **CI 同步检查**：新增依赖后检查 `.github/workflows/ci.yml` 是否需要同步（M2 教训——backend job 需先 `pnpm build:mobile`）。

## 接口名对齐要点（两轮核查已修正，此处汇总防误用）

| 正确名称 | 来源 | 别错写成 |
|---|---|---|
| `STATUS_DOT_COLOR` | `src/mobile/board-logic.ts:60` 已有导出 | ~~STATUS_COLORS~~ |
| `TOOL_LABELS` | Task 3 在 board-logic.ts 中**新增**（原不存在） | 不要从桌面 agentBadge.tsx import |
| `sysinfo::System::host_name()` | 已有依赖 sysinfo 的静态方法 | ~~hostname::get()~~ |
| `Query` | `axum::extract::Query`——api.rs 现有 use 块**不含**，需追加 | — |
| `TOOL_BRAND_COLORS` | Task 3 在 board-logic.ts 中新增 | — |
| gate 覆盖 | 新端点加在 `api_router()` 内即自动过闸 | 不需另加 middleware |

## 参考文档（按需查阅原文，勿凭记忆）

| 文档 | 用途 |
|---|---|
| `research/一期待办-v6摘选-私有.md` M3 节 | 需求唯一来源（含全部定稿裁决） |
| `research/refs/phase1-接入层/一期实现要点-技术备忘.md` §B | 接入层实现要点 |
| `research/dsh-probe-2026-09-13-report.md` | dsh 格式权威（Task 7 content 解析参照） |
| `src-tauri/src/monitor/*_parser.rs` | Task 7/9 实现参照（各工具路径推导与解析） |
| `src-tauri/src/remote/server.rs` | M2 现有路由结构（nest/gate/静态分层） |
| `src/mobile/board-logic.ts` | M2 现有前端逻辑（排序/过滤/状态色） |

## Task 10 的边界

Step 1 门禁照常；Step 2 手动验收清单需用户+手机配合——**不要假装做过**，列为"待用户+评审执行的手动项"。

## 完成后输出（供独立评审 review）

按 Task 1–10 逐条报告：
1. 完成状态（完成 / 部分完成+原因 / 停止+触发的铁律）
2. 测试结果（跑过的命令 + 关键输出摘要，失败也要如实贴）
3. commit hash 与 message
4. 与计划的任何偏离及理由（无则写"无"）

最后附四样：
- `git log --oneline`（本分支全部提交）
- `cargo test` 全量摘要（含 remote 相关测试数）
- `cargo clippy --all-targets -- -D warnings` 结果
- `pnpm check && pnpm test && pnpm build:mobile` 结果
