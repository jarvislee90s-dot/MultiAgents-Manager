# 批次戊 Stage 2 · 执行提示词（交执行 agent 的完整指令）

> 你是 MAM 项目「批次戊 · Stage 2 编码」执行 agent。工作目录 E:\LLMproject\Github\mam-worktree2（分支 feat/phase2-injection，基线=开工时 HEAD，Stage 2 计划提交 3e6bcbf 之后）。
> 主控=评审员（单评审员一轮：规格符合性+代码质量合并）；每任务你交付后由主控 review，Important/Critical 由你修复后同评审复审，通过才进下一任务。

## 任务书（唯一 SSOT）

`docs/superpowers/plans/2026-09-22-phase2-batch-e-stage2-plan.md`——S2-T1..T13 顺序执行（第一波 T1–T5 矛盾修正、第二波 T6–T12 新能力、T13 收口）；每任务自带定案输入/文件地图/TDD/DoD。执行序与依赖见其 §0；夹具库见其 §1。

## 必读（开工前读完，之后按任务所需再读对应节）

1. Stage 2 计划全文（上文 SSOT）
2. 设计文稿 `docs/superpowers/plans/2026-09-22-phase2-batch-e-design.md` §0 裁1–裁20（刚性，不得重新设计）
3. 差距矩阵 `docs/superpowers/plans/2026-09-22-phase2-batch-e-gap-matrix.md` §6.3–§6.6（裁决/键序/守卫原则/矛盾审计=A 类任务的需求来源）
4. 键序大词典（**清淤版，唯一键序事实源**）`research/refs/phase2-消息注入/2026-09-22-键序大词典.md`
5. 涉及实机验证的任务：`.agents/skills/win-console-inject-probe/SKILL.md`（八条铁律）

## 红线（违反即作废重做）

1. 不 push、不动 main、不动 `docs/MASTER-PLAN.md`；不修改设计文稿/差距矩阵/Stage 2 计划的 §0 裁决与任务定义（发现计划与代码现实冲突 → 停下，上台账按最小偏差处理并在交付报告显式申报，等主控/用户裁决）。
2. 每任务独立 commit：`feat(mfix-e): T<N> <一句话>`；文档类 `docs(mfix-e): …`。commit 前 fmt + clippy 0 警告 + 受影响测试面全绿（cargo 一律 `CARGO_TARGET_DIR=target/gate-run` 且在 `src-tauri/` 下执行；bin 测试须 `--features hook-listener`；vitest 终跑勿与 cargo 并行）。T13 跑全六门禁。
3. TDD 硬纪律：每任务先写失败测试（夹具=计划 §1 清单的真机屏读原文，拷入仓库根 `tests/fixtures/e-stage2/`——Rust 侧经 `../tests/fixtures/e-stage2/` 读）→ 实现 → 全绿。**禁止手造「看起来也行」的夹具**（批次丁 H-4① 教训：状态夹具必须来自真机实录）。
4. 实机项一律 `#[ignore]` 测试或探测脚本；实机交互遵守：只碰自己 spawn 的进程/控制台（不碰用户自有会话）、Esc 类注入单次+屏读确认、opencode **Ctrl+C 一律禁注**（直接退出应用）、kimi 审批框数字禁用、每步附「注入键→屏读逐行原文」对照（裁15）、结束 taskkill 清场复核零存活、结论不超证据。
5. 键序一律以词典（清淤版）为准；任务中发现词典与实机冲突 → 停下上台账（裁20：用户实测优先），不擅自改词典。
6. 台账：`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md` 文末「批次戊 Stage 2」节逐任务追记（任务×commit×一行结论）；如实记录，不谎报（实机项允许「登记未跑」如实标注）。

## 每任务交付报告格式

Status（DONE / DONE_WITH_CONCERNS / NEEDS_CONTEXT / BLOCKED）／改动摘要（文件×要点）／测试结果（新增 N 例全绿+既有面无回归，附数字）／commit sha／实机项状态（跑了/登记未跑+原因）／自审发现（自己抓到并修掉的问题）／与计划的偏差申报（如有）。

## 收口（T13 后停下）

全六门禁+fmt/clippy/`pnpm check`/`build:mobile` → 验收清单 I 节（人工项从定案表生成）→ 台账+handover → **停下等主线 review，不自行合并**。
