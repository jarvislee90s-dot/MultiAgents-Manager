# 批次丁 · 全终端问答/审批/模式闭环 · 实施计划（压缩版 v2）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**待用户批准开工。** 前置：批次乙+丙+R1 已合并验收。任务级粒度对齐仓库既有批次计划体例（甲/乙/丙）；每任务的文件/要点/验收如下，实现细节由实现者按代码地图自探。
> **Goal:** 四家终端的「终端在等用户 → 红灯 → 卡片 → 安全作答/切换 → 回显」全链闭环：补 codex/opencode 状态链、kimi 审批卡、codex 计划待确认卡、注入对话框守卫、签名后置+斜杠豁免、模式二维模型+回读全开、多选提交闭环与自由文本统一。
> **依据档案：** 三轮实测取证（2026-09-21：11 张截图 + 审计表 + rollout + 状态链代码勘察）+ 三家官方文档调研（opencode.ai/docs、developers.openai.com/codex（rust-v0.155.1 源码对齐）、moonshotai.github.io/kimi-code）+ 批次丙 R1 键序档机制。
> **红线：** 不 push、不动 main、不动宪法；六门禁全绿（cargo 用 CARGO_TARGET_DIR=target/gate-run、bin 须 --features hook-listener；vitest 终跑勿与 cargo 并行）；实机测试 #[ignore]；台账如实；每任务独立 commit（`feat(mfix-d): T<N> …`）。

## §1 问题清单与根因（三轮实测，全部取证）

| # | 现象 | 根因 | 任务 |
|---|---|---|---|
| 1 | codex 问答卡不出（Question 1/3 待决只有 JSON 折叠行，黄点） | 解析器无"用户输入类工具→等待"识别：function_call 一律=Processing；卡片挂载锚红灯 → 永不挂载（问答端点本身不看状态，断点只在状态链+挂载） | 丁T1 |
| 2 | opencode question 卡不出（黄点）、发数字不达意 | 同构：tool 部件一律=Running→Processing；且 opencode 无钩子通道，标记叠加不生效 | 丁T1 |
| 3 | kimi Ready-to-build 无卡 → 打字被当成选项（误批准实锤） | 状态红灯 ✓ 但审批卡要求映射表，kimi 不在默认映射表；wire 的 interaction(approval/plan_review) 结构化数据现成未用 | 丁T2 |
| 4 | codex「Implement this plan?」无选项卡 | 三无对话框（不落盘/无钩子/无状态变化）；修法=计划提案后「预期对话框」启发式+屏读解析 | 丁T2 |
| 5 | 模式按钮变"选第一项"；待批准时切换变直发/打断回合 | 模式切换注入零守卫（审计 17:36-38 连点 17 次全落进待决对话框） | 丁T3 |
| 6 | claude 多选题：composer 打自由文本 → 选项 1 被勾选/反勾 | 消息注入以回车结尾，多选对话框 **Enter=切换高亮行**（K9）；composer 不知道对话框在场 | 丁T3 |
| 7 | claude 多选提交停在 Review 屏（卡片只显示「已发送按键」） | 提交序列未处理 Review 确认屏（K10 第三段）；应为「以屏读为锚的阶段机闭环」而非一次性序列 | 丁T5 |
| 8 | 手打 `/permissons` 被前缀毁掉 | compose_injection 无条件加 `[mobile …] ` 前缀（rollout 10:04 实锤） | 丁T3 |
| 9 | 切换模式后不知道当前模式 | 三家回读词表未校准（批次丙开放问题 3，用户升级必做） | 丁T4 |
| 10 | codex 多问题问答（Question 1/3 + ←→）未支持 | 键序档未含多题导航 | 丁T2 |
| 11 | codex 计划卡 markdown 不完整（计划 label 已出 ✓） | 升格已通、渲染层支持不全；17:19 rollout 实录（行 140）做夹具 | 丁T5 |
| 12 | kimi 问答 TwoPhaseSelect 数字键可靠性未验 | R1 复审 Important-1 | 丁T2 |

## §2 统一交互契约（四家一致的用户可见行为）

1. **卡片四型**：问答卡（单选即答/多选勾选+提交/多题导航）/ 审批卡（二元降级、N 选项屏读、计划批准导航确认、kimi 计划审批）/ 计划卡（markdown 正文）与计划文件卡（路径+预览）/ 模式栏（当前模式回显+切换组）。
2. **计划双卡政策**：正文卡与文件卡**各自能上就都上**（正文来自消息或可读文件；文件卡来自已知路径）——claude 正文✓+文件✓（.claude/plans 已豁免）、codex 正文✓+文件✗（计划不落文件）、kimi 正文✓（读 plan 文件渲染，路径在 wire/工具结果，目录已豁免）+文件✓、opencode 视形态。目标：大维度上行为通用，不为强求一致而砍掉某家已有的能力。
3. **点选语义**：单选点选项=作答；多选点选项=仅勾选，必须显式提交；**提交是阶段机闭环**——以屏读状态为锚（题目 n/Review 屏/确认屏）逐段推进直至终态确认，序列未走完卡片显示进行中，不得以「已发送按键」了事。
4. **自由文本双入口**：问答卡内嵌自由文本输入框（主入口）；对话框待决时 composer 自动转「作为回答发送」模式（防呆底线——在哪打字都不许误触选项）。点选项的本质=「替用户把内容填入并提交」，与默认项确认明确区分。
5. **签名规则**：普通消息 `[mobile 设备名]` 签名**后置到消息末尾**（正文可读性优先，溯源不变）；`/` 开头命令消息**裸注入不带货**（前后附加都会破坏命令/参数），溯源走审计 `action=slash`+设备名。
6. **模式栏**：二维工具（codex/kimi）=「模式组 Default/Plan」+「权限组三档」两组按钮（斜杠直达，权限为两段式：命令→菜单→屏读定位→导航确认）；单轴工具（claude/opencode）=一组循环+当前模式回显。切换后必须回显结果（回读失败显示"请人工核对"）。
7. **对话框在场 = 注入红线**：控制类注入（模式切换/斜杠）前屏读检测，检测到待决对话框一律拒绝并提示。
8. 兜底渲染原则不变。

## §3 任务分解（5+收口）

### 丁T1 · 红灯与状态链（问题 1/2）
**Files**: `src-tauri/src/monitor/codex_parser.rs`、`opencode_parser.rs`、`src/mobile/SessionDetail.tsx`、各测试
- [ ] codex_parser：待决 `request_user_input`（function_call 按 name 匹配且无对应 output）→ Waiting；APP/SQLite 路径同核。
- [ ] opencode_parser：尾部 question part（无 answers）→ Waiting（替代一律 Running）。
- [ ] QuestionCard 挂载放宽：非结束态挂载+可用性自隐（问答端点本不看状态；成本=详情页每轮一个 GET）。
- [ ] 回归锁：「运行中不误红灯」（已答/历史问题不触发）。
- 验收：codex/opencode 问题待决 → 看板红灯 + 详情页问答卡出现；回答后回落。

### 丁T2 · 缺失的卡片（问题 3/4/10/12）
**Files**: `src-tauri/src/remote/api.rs`、`inject/question.rs`、wire 解析（kimi interaction）、`src/mobile/` 卡片、`inject/dialog.rs`
- [ ] kimi 计划审批卡：wire `interaction.request`（plan_review）→ 卡片（Approve/Reject/Revise + 状态回显）；注入走导航确认（R1 序列，从解析高亮算步进）。
- [ ] kimi 计划正文卡：读 plan 文件渲染 markdown（路径取 wire/工具结果；目录豁免批次丙已在）——与计划文件卡并存（§2.2 双卡政策）。
- [ ] codex「计划待确认」卡：计划提案消息（proposed_plan，已有识别）→「预期对话框」态 → 详情页提示条+「检查终端对话框」按钮 → 屏读解析 Implement-this-plan 选项 → N 选项卡（codex=数字直选档）+ 计划全文聚合。
- [ ] codex 多问题键序（Question n/N + ←→ 导航 + 每题 enter）——`#[ignore]` 实机探测定案后实现。
- [ ] kimi 问答 TwoPhaseSelect 数字可靠性实测（R1 复审 Important-1）→ 键序分族定案。
- 验收：kimi Ready-to-build 三钮卡且 Reject 必达；codex 计划待确认卡可见可点；多问题可答。

### 丁T3 · 注入守卫与签名后置（问题 5/6/8）
**Files**: `inject/dialog.rs`（在场检测复用）、`remote/api.rs`（mode 路径+session-send）、`inject/normalize.rs`、`database/dao/write_audit.rs`、`src/mobile/MessageComposer.tsx`
- [ ] 对话框在场检测原语（屏读可见窗口存在编号选项对话框）——工具无关单点实现。
- [ ] 接入①模式切换：注入前检测，在场 → 拒绝+「终端有待决对话框，请先处理」回执。
- [ ] 接入②composer：问答卡在场 → 发送转「作为回答发送」自由文本序列（placeholder/按钮态变化；后端问答注入已在此路径）。
- [ ] 签名后置：普通消息 compose 改 `[正文] [mobile 设备名]`（签名在末尾）；`/` 开头消息免签名裸注入；审计新增 `action=slash`+设备名（溯源补偿）；队列预览与「修改」剥签名逻辑从前缀改尾部。
- [ ] `#[ignore]` 实机：对话框待决点模式按钮 → 拒且有提示；composer 发送 → 走自由作答；移动端发 `/permissions` → 命令触发+审计 slash。
- 验收：图5/6/7 场景（模式按钮变选项、打字变勾选、斜杠失效）全部消失。

### 丁T4 · 模式二维与回读全开（问题 9）
**Files**: `inject/mode.rs`、`src/mobile/ModeBar.tsx`、`remote/api.rs`（mode 路径）
- [ ] 二维工具（codex/kimi）：模式组（Default/Plan）+ 权限组（三档）两组按钮；斜杠直达（kimi `/plan on|off`、`/permission`、`/yolo`、`/auto`；codex `/plan`、`/permissions`）——权限为两段式（命令→菜单渲染→屏读定位目标行→导航确认，复用 R1 机制）。
- [ ] 单轴工具（claude/opencode）：一组循环切换 + 当前模式回显（claude 环序表 acceptEdits→plan→auto→manual 已实测；opencode `Build` 改显「默认」）。
- [ ] 回读校准：claude/kimi/codex 底栏词表逐家屏快照回归（codex Default 靠缺席推断的口径处理）；回读失败显示「请人工核对」，不假装。
- [ ] 约束落地：codex `/plan` 运行中不可用 → 如实回执；枚举含 untrusted/on-failure 标 legacy（官方已退役）。
- 验收：四家切换后卡片回显当前模式；「模式未知」从常态变例外。

### 丁T5 · 提交闭环与渲染（问题 7/11）
**Files**: `inject/question.rs`、`src/mobile/QuestionCard.tsx`、content.rs 渲染、测试
- [ ] 阶段机提交：多选/多题提交以屏读状态为锚（题目 n→Review 屏→确认）逐段推进自动走完，终态未达如实回执；卡片进行中态。
- [ ] 问答卡内嵌自由文本输入框（各工具自由作答序列 `#[ignore]` 实机定案：claude 定位 Type something→文本→Enter；codex tab notes；opencode 选 own answer→文本→Enter；kimi Other/feedback）——与丁T3 composer 转向共用同一序列出口。
- [ ] codex 计划卡 markdown 补全（17:19 rollout 行 140 实录做夹具：`###` 标题/列表/粗体）；核对同条升格路径。
- 验收：多选勾选→提交→Review 自动确认一气呵成；卡内输入自由要求 → 以自由作答提交不错勾选项；codex 计划卡渲染完整。

### 丁T6 · 收口
- [ ] 六门禁全绿；验收清单 G 节（批次丁人工项）；台账批次丁节 + handover 新档；停下等主线 review。

## §4 总验收（八条）

①codex/opencode 问题待决红灯+问答卡可作答；②kimi 计划审批三钮卡 Reject 必达+计划正文卡；③codex 计划待确认卡可见可点且聚合计划全文；④对话框待决时模式按钮被拒有提示、composer 转自由作答（不错勾选）；⑤移动端斜杠命令可触发+审计留痕+普通消息签名后置溯源不变；⑥四家切换后回显当前模式（二维两组/单轴一组）；⑦claude 多选勾选→提交→Review 一气呵成；⑧codex 计划卡 markdown 完整+kimi 计划正文卡。六门禁全绿。

## §5 不入本计划

- kimi server API 程序化应答（用户未开 server；备查升级路径）；
- opencode 会话内权限切档（官方无此能力，权限属配置文件域）；
- codex Permission Profiles（beta）；协议路线（三期）；
- zcode/dsh/workbuddy/openclaw 交互卡（沿用既有判定）；
- codex 打断式插队（esc interrupt 底栏存在，需探测后另批统一）。
