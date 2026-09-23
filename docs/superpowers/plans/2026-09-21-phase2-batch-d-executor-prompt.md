# 批次丁 · 执行提示词（交执行 agent 的完整指令，2026-09-21）

> 用法：把本文件 `---` 之后的全文作为执行 agent 的任务指令。计划 SSOT 与依据档案见文内指向。批次丙评审意见已全部收敛为本批任务；**本批完成并通过主线复审后，乙丙丁三批合并验收**。

---

[$subagent-driven-development](C:\Users\bunny\.agents\skills\subagent-driven-development\SKILL.md)

你是 MAM 项目「批次丁（全终端问答/审批/模式闭环）」执行 agent。工作目录
E:\LLMproject\Github\mam-worktree2（分支 feat/phase2-injection，基线=开工时 HEAD，计划提交 199ee54
之后）。前三批已合并验收；本批解决手工测试暴露的问答/审批/模式交互问题。

任务书（唯一执行 SSOT）：docs/superpowers/plans/2026-09-21-phase2-batch-d-plan.md（v3）。
**§0 用户裁决清单（裁1-10）是刚性需求，实现者不得重新设计**；§2 统一交互契约是四家一致行为的
成文规范；§3 任务分解 T1–T5+收口；§1 问题×根因已全部取证（勿重新调研）。

## 执行方法（评审环节按用户指令精简）

每任务：派一个实现 subagent（TDD：先写失败测试再实现）→ 派**一个**评审 subagent 做**合并评审**
（规格符合性 + 代码质量一轮完成：先对照任务书逐条给规格结论，再给质量分级 Issues
Critical/Important/Minor 与 Assessment）→ Important/Critical 由实现者修复后同一评审员复审 →
通过即下一任务。实现 subagent 不并行；评审员不信实现者报告、以代码为准。

## 红线

不 push、不动 main、**不改 docs/MASTER-PLAN.md 与批次丁计划文档**；六门禁全绿（cargo 一律
CARGO_TARGET_DIR=target/gate-run 且在 src-tauri/ 下执行；bin 测试须 --features hook-listener；
vitest 终跑勿与 cargo 并行）；实机测试一律 #[ignore]；台账如实；每任务独立 commit
（`feat(mfix-d): T<N> …` 风格）；§0 裁决与 §2 契约是刚性规范，§5 边界不得越界。

## 任务顺序

丁T1 红灯与状态链（codex/opencode 待决问题→Waiting+挂载放宽）→ 丁T2 缺失的卡片（kimi 审批
卡/kimi 计划正文卡/codex 计划待确认卡/多问题与键序探测定案）→ 丁T3 注入守卫与签名后置（对话框
在场检测+模式切换与 composer 两接入+stamp 适配+审计 slash）→ 丁T4 模式二维与回读全开（§2.6 规
格表照做）→ 丁T5 提交闭环与渲染（阶段机提交+卡内自由文本输入框+codex markdown）→ 丁T6 收口
（六门禁+验收清单 G 节+台账批次丁节+handover 新档+停下等主线 review）。

## 关键环境事实（开工先核对，勿重新调研）

- 状态链断点已在代码层定位：codex_parser 把 function_call 一律映射 Processing（无 Waiting 分支）、
  opencode_parser 把 tool 部件一律映射 Running；claude 的同款机制参照 monitor/status.rs 的
  is_waiting_for_user_input。
- kimi 计划审批的数据源在 wire `interaction.request`（plan_review/ExitPlanMode，无 questions[]）；
  kimi plan 文件路径在工具结果/系统提醒的 `Plan file:` 行；kimi plans 目录豁免已上线（批次丙 T7）。
- codex 计划提案=rollout 里 `<proposed_plan>` 包裹消息（17:19 实录=rollout-…01a0c191…jsonl 行 140，
  作丁T5 渲染夹具；该条升格未触发的断点待你核对）。
- 对话框屏读解析与导航确认安全序列已存在（批次丙 T5/R1：inject/dialog.rs 的
  parse_dialog_options/navigation_sequence），丁T3/T4/T5 复用勿重写。
- 签名后置有个必防的真 bug：确认戳若取注入文本尾部，后置签名=同设备消息共享尾→跨消息假命中——
  丁T3 的 stamp 适配与「两条同设备不同正文互不命中」测试是安全项，不可省。
- 模式切换斜杠路径已是裸注入（无前缀）；composer 的 [mobile] 前缀在 compose_injection（无任何豁免）。
- 用户已过 codex 信任门；测试一律用你自己 spawn 的新会话，不碰用户自有会话。

## 实机探测与验证

下列各项须 #[ignore] 实机定案/验证（遵守 .agents/skills/win-console-inject-probe 八条铁律：只碰
自己 spawn 的进程、临时目录带 run-id、日志+截图+会话文件三重证据、结束清场、结论不超证据）：
丁T2 的 codex 多问题导航键序与 kimi 问答数字可靠性；丁T3 的三场景（待决点模式钮→拒/composer→
自由作答/移动端斜杠→触发+审计）；丁T4 的四家模式切换与回读逐例；丁T5 的 claude Review 屏闭环与
各工具自由作答序列。新探测档案落 research/refs/phase2-消息注入/ 并同步该目录 README.md 索引。

## 台账与收口

`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md` 文末追加「批次丁」节（任务
×commit×评审×门禁×自裁决清单，体例对齐批次丙节）+ 同目录 handover 新档。丁T6 收口：六门禁终跑
+ 验收清单 H 节（批次丁人工项，体例对齐 E 节）+ 停下等主线 review。

## 执行纪律

中途与计划/代码现实冲突 → 停下上台账按最小偏差处理并在简报显式申报；不得单方面修改批次丁计划
文档（判据被实机证伪等情形照批次丙 T1 先例：改实现+三处留痕+台账申报）；§0 裁决清单如需变动必须
停下等用户，不得自行调整。
