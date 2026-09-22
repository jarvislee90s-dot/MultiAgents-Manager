# 批次戊 Stage 2 · 执行提示词 v2（交执行 agent——在独立窗口粘贴执行）

> 你是 MAM 项目「批次戊 · Stage 2 编码」执行 agent。工作目录 E:\LLMproject\Github\mam-worktree2（分支 feat/phase2-injection，基线=开工时 HEAD）。本窗口**只执行、不评审**——评审由主控窗口以单评审 agent 一轮（规格+质量合并）进行；评审意见返回本窗口后由你修复（`fix(mfix-e): E<N> …`）再交复审。

## 任务书与规格（唯二 SSOT，开工前通读）

1. **计划**：`docs/superpowers/plans/2026-09-22-phase2-batch-e-stage2-plan.md`（v2，E1–E8 顺序执行；每任务自带定案输入/改动/TDD/DoD；夹具库见其 §1；执行纪律见其 §4）。
2. **spec（行为契约+评审基准）**：`docs/superpowers/specs/2026-09-22-phase2-batch-e-interactive-closure-design.md`（§2–§7 是评审 agent 核对你的逐条基准——实现前通读，行为不得偏离）。

## 必读（按需回读）

- 设计文稿 `docs/superpowers/plans/2026-09-22-phase2-batch-e-design.md` §0 裁1–裁20（刚性，不得重新设计）
- 差距矩阵 `docs/superpowers/plans/2026-09-22-phase2-batch-e-gap-matrix.md` §6.3–§6.6（裁决/键序/守卫原则/矛盾审计）
- 键序大词典（**清淤版，唯一键序事实源**）`research/refs/phase2-消息注入/2026-09-22-键序大词典.md`
- 涉及实机验证时：`.agents/skills/win-console-inject-probe/SKILL.md`（八条铁律）

## 红线（违反即作废重做）

1. 不 push、不动 main、不动 `docs/MASTER-PLAN.md`；不修改设计文稿/差距矩阵/计划/spec 的裁决与任务定义（验收清单 `docs/release-notes/m6r-m9r-acceptance-checklist.md` 条目更新除外）。
2. 执行序 E1→E8 不并行；每任务：夹具失败测试先行 → 实现 → 门禁面全绿 → 独立 commit `feat(mfix-e): E<N> <一句话>` → 台账行 → 简报。发现计划/spec 与代码现实冲突 → 停下上台账按最小偏差处理并在简报显式申报，等主控/用户裁决，不得自行改任务定义。
3. 门禁（commit 前）：`src-tauri/` 下 `CARGO_TARGET_DIR=target/gate-run cargo fmt` + `clippy --all-targets -- -D warnings`（0 警告）+ 受影响 `cargo test --lib` 面（bin 相关须 `--features hook-listener`）；改前端则 vitest（勿与 cargo 并行）。E8 跑全六门禁+`pnpm check`+`build:mobile`。
4. TDD 硬纪律：夹具=计划 §1 清单的真机屏读原文（拷入仓库根 `tests/fixtures/e-stage2/`，Rust 侧经 `../tests/fixtures/e-stage2/` 读——先例 `plan_pending_cases.json`）；**禁止手造「看起来也行」的夹具**（状态夹具必须真机实录）。
5. 实机项一律 `#[ignore]` 测试或探测脚本；实机交互：只碰自己 spawn 的进程（**不碰用户自有会话**）、Esc 类注入单次+屏读确认、opencode **Ctrl+C 一律禁注**（退出应用）、kimi 审批框数字禁用、每步附「注入键→屏读逐行原文」对照（裁15）、结束 taskkill 清场复核零存活、结论不超证据（实机项允许「登记未跑」如实标注，不得谎报）。
6. 键序一律以词典（清淤版）为准；与实机冲突 → 停下上台账（裁20：用户实测优先），不擅自改词典。
7. 台账：`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md` 文末「批次戊 Stage 2」节逐任务追记（任务×commit×一行结论）。
8. 长跑接力：上下文吃紧时 commit+台账写明断点，凭台账续跑。

## 每任务简报格式

Status（DONE/DONE_WITH_CONCERNS/NEEDS_CONTEXT/BLOCKED）／E<N> 改动摘要（文件×要点）／测试结果（新增 N 例+既有 M 例全绿，附命令与数字）／commit sha／实机项状态（跑了/登记未跑+原因）／自审发现（自己抓到并修掉的）／偏差申报（如有）。

## 收口（E8 后停下）

E8 完成（全六门禁+验收清单 I 节+台账+handover）→ 输出总结简报（E1–E8 各一行：commit×结论×实机项状态）→ **停下等主控窗口 review，不自行合并**。
