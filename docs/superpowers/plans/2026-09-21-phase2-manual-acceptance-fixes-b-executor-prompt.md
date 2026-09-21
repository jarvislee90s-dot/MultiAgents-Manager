# 批次丙 · 执行提示词（交执行 agent 的完整指令，2026-09-21）

> 用法：把本文件全文作为执行 agent 的任务指令。计划 SSOT 与依据档案见文内指向。批次乙评审过程中发现的问题已全部收敛为本批 §1 清单，**本批完成即批次乙视为评审通过（乙丙合并验收）**；执行完成后由主线 review agent 复审。

---

[$subagent-driven-development](C:\Users\bunny\.agents\skills\subagent-driven-development\SKILL.md)

你是 MAM 项目「手工验收修复批·批次丙（审批交互统一与 plan 全链）」执行 agent。工作目录
E:\LLMproject\Github\mam-worktree2（分支 feat/phase2-injection，基线=开工时 HEAD，计划提交 9e9684f
之后）。批次乙（T1–T8，收口 25f7bae）已交付；主线评审过程中以两轮实机测试（四终端 plan 模式 +
问答交互）暴露的问题已全部收敛为本批问题清单——**解决本批问题后，批次乙即视为评审通过，乙丙
合并验收**。执行完成后停下，由主线 review agent 复审。

任务书（唯一执行 SSOT）：docs/superpowers/plans/2026-09-21-phase2-manual-acceptance-fixes-b.md
（§1 十一项问题×根因取证对照 / §2 统一交互卡 UI 设计基准 / §3 任务分解 T1–T11 / §4 十条总验收）。
开工前通读任务书与三份依据档案（路径见任务书头部：claude 按键语义探测 K1–K11 / 问卷交互跨工具
矩阵 / happy 项目调研）。

执行方法：[$subagent-driven-development]——每任务派 fresh subagent 实现 → 规格符合性评审 → 代码
质量评审 → 通过后下一任务；实现 subagent 不并行。实机探测遵守项目技能
`.agents/skills/win-console-inject-probe` 八条铁律（不碰用户会话、临时目录、结束清进程、结论不超
证据）。

红线：不 push、不动 main、**不改 docs/MASTER-PLAN.md**；六门禁全绿（cargo 一律
CARGO_TARGET_DIR=target/gate-run；**bin 测试须 --features hook-listener**；vitest 终跑勿与 cargo
并行）；实机测试一律 #[ignore]；台账如实；每任务独立 commit（`feat(mfix-c): T<N> …` 风格，与
批次乙体例一致）。

执行顺序与依赖（任务书 §3 的推荐串行序）：
T1 幽灵审批标记（最高优先，图2/图3 红卡错乱根因）→ T2 helper 构建部署保障 → T3 问答卡形态化
匹配（codex/opencode 接入）→ T4 codex proposed_plan 剥标签 → T7 plan 文件豁免/识别/渲染 →
T5 通用 N 选项审批卡（屏读解析）→ T6 模式切换 → T8 审批点 plan 聚合（依赖 T4/T7）→ T9 claude
打断式插队（前置探测 Esc 语义）→ T10 统一交互卡 UI 改造（按任务书 §2 基准收敛全部卡）→ T11 收口。

关键环境事实（开工先核对，勿重新调研）：
- `~/.mam/bin/mam-hook-listener.exe` 是旧构建（00:45，早于批次乙 T8 提交 05:59），事件无 tool_name
  载荷——T2 完成并以新构建重启 MAM 后自愈；
- DB 里 claude 会话 0c41365d 挂着幽灵「等待审批」标记（取证样本）——测试一律用新会话，**不得写
  手动清理代码**（生产清理由既有清除族承担）；
- helper 事件载荷的实机验证是批次乙遗留未验段（T2 自检必须真实跑通一次 PreToolUse(AUQ) 载荷落盘）；
- 用户已过 codex 信任门（hooks 事件正常落盘）；claude 真实会话中 pending 的 AskUserQuestion
  tool_use 会即时落盘（通道 B 可用——与 2026-09-21 探测档案的 relay 环境结论相反，以真实为准）。

实机探测任务（T1 的 Notification.message 原文取证 / T5 三类对话框屏读解析 / T6 四家模式切换 /
T9 Esc 中断语义）：一律 #[ignore] 或探测脚本，遵守八条铁律；结论写实现注释并归档
research/refs/phase2-消息注入/（新探测档案同步更新该目录 README.md 索引）。

台账：`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md` 文末追加「批次丙」节
（任务×commit×评审×门禁×自裁决清单，体例对齐批次乙节）+ 同目录 handover 新档。T11 收口：六门禁
终跑 + 验收清单增补（C-23 N 选项审批 / C-24 模式切换 / C-25 打断式插队 / plan 文件预览扩展项，
体例对齐 m6r-m9r-acceptance-checklist.md）+ 停下等主线 review。

执行纪律：中途与计划/代码现实冲突 → 停下上台账按最小偏差处理并在简报显式申报；不得单方面修改
批次丙计划文档（口径变化只上台账）；任务书 §2.5「不做」与 §5「不入本计划」为边界，越界=违规。
