# 批次丁 · 全终端问答/审批/模式闭环 · 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。**待用户批准开工。** 前置：批次乙+丙+R1 已合并验收（HEAD 8fcef20 之后）。
> **Goal:** 补齐 codex/opencode 的问答状态链与卡片、kimi 计划审批卡、codex 计划待确认卡；注入加对话框守卫；斜杠命令豁免前缀；模式二维模型+四家回读全开；多选提交补完+自由文本输入统一。
> **依据档案：** 本轮实测 8 截图 + 审计表 + rollout 取证（2026-09-21 手工测试）；`research/refs/phase2-消息注入/2026-09-21-批次丙主线复审独立核实探测.md`；三家官方文档调研（opencode.ai/docs、developers.openai.com/codex、moonshotai.github.io/kimi-code，2026-09-21）。
> **红线：** 不 push、不动 main、不动宪法；六门禁全绿（cargo 用 CARGO_TARGET_DIR=target/gate-run、bin 须 --features hook-listener；vitest 终跑勿与 cargo 并行）；实机测试 #[ignore]；台账如实；每任务独立 commit（`feat(mfix-d): T<N> …`）。

## §1 问题清单与根因（本轮实测取证）

| # | 现象 | 根因 | 任务 |
|---|---|---|---|
| 1 | codex 问答卡不出（Question 1/3 待决，手机只有 JSON 折叠行，黄点） | codex 解析器把 function_call 一律当"运行中"，**没有"用户输入类工具→等待"识别**（claude 有、它没有）；卡片挂载锚红灯 → 永不挂载。问答端点本身支持 codex（纯消息扫描不看状态）——断点只在状态链+挂载条件 | 丁1/丁2 |
| 2 | opencode question 卡不出（黄点）、发数字不达意 | 同构：question 部件一律=运行中；且 opencode 无钩子通道。发"4"带前缀进对话框不达意 | 丁1/丁2/丁9 |
| 3 | kimi Ready-to-build 无卡 → 打字被当成选项（误批准） | kimi 状态红灯 ✓ 但审批卡要求映射表，kimi 不在默认映射表 → 无卡。wire 的 interaction(approval) 结构化数据现成（plan_review/Approve/Reject/Revise） | 丁3 |
| 4 | codex「Implement this plan?」无选项卡 | 三无对话框：不落盘、无钩子、无状态变化。修法=「计划提案后预期对话框」启发式+屏读解析（选项文本来自屏读非猜测） | 丁4 |
| 5 | codex 模式按钮变"选第一项"；待批准时切换变直发/打断回合 | 模式切换注入**零守卫**（审计 17:36-38 连点 17 次全落在待决对话框里） | 丁5 |
| 6 | claude 多选题：在 composer 打自由文本 → 选项 1 被勾选/反勾 | 对话框待决时 composer 注入的**结尾 Enter 切换高亮行**（K9 实证）；composer 不知道对话框在场 | 丁5/丁9 |
| 7 | claude 多选提交停在 Review 屏（卡片只显示「已发送按键」） | 多选提交序列未处理 Review 确认屏（K10 三段式的第三段） | 丁8 |
| 8 | 手打 `/permissons` 被前缀毁掉（codex 当问题回答了） | compose_injection 无条件加 `[mobile …] ` 前缀，斜杠必须顶格 | 丁6 |
| 9 | 切换模式后不知道当前模式（claude/kimi/codex） | 三家回读词表未校准（批次丙开放问题 3，用户升级为必做） | 丁7 |
| 10 | codex 多问题问答（Question 1/3 + ←→ 导航）未支持 | 键序档未含多问题导航 | 丁3 |
| 11 | codex 计划卡 markdown 渲染不完整（计划 label 已出 ✓） | 升格已通，渲染层对计划内容的 markdown 支持不全 | 丁8 |
| 12 | kimi 问答 TwoPhaseSelect 数字键可靠性未验 | R1 复审 Important-1 遗留 | 丁9 |

## §2 统一交互契约（四家一致的用户可见行为，实现细节按工具藏在键序档）

1. **卡片四种**：问答卡（单选即答/多选勾选+提交/多题导航）/ 审批卡（二元降级、N 选项屏读、计划批准导航确认）/ 计划卡（markdown 正文）与计划文件卡（路径+预览）/ 模式栏（当前模式回显+模式组与权限组切换）。
2. **点选语义**：单选点选项=作答；多选点选项=仅勾选，**必须显式点提交**；提交序列内部处理工具特有阶段（claude Review 屏、kimi Review、codex 每题提交+导航）——对用户透明，序列未走完卡片不得显示"已发送"了事。
3. **自由文本**：每张问答卡带输入框（"其他/补充说明"）；**对话框待决时 composer 自动转为「作为回答发送」模式**（placeholder 变化+发送走问答自由文本序列），不再裸注入。二者等效，双入口。
4. **模式栏**：二维工具（codex/kimi）分「模式组（Default/Plan）+ 权限组（三档）」两组按钮（斜杠直达）；单轴工具（claude/opencode）一组循环切换+当前模式回显。切换后必须回显结果模式（回读失败显示"请人工核对"，不假装）。
5. **对话框在场 = 注入红线**：任何控制类注入（模式切换/斜杠）前做屏读对话框检测，检测到待决对话框一律拒绝并提示；composer 直发在问答待决时转自由文本序列（见 3）。
6. 兜底渲染原则不变：解析失败降级+明示，会话永不被前端卡死。

## §3 任务分解

### T1 · 状态链补齐（codex/opencode 待决问题 → 红灯）
- [ ] codex_parser：待决 `request_user_input`（function_call 无对应 output，按 name 匹配）→ Waiting；有 output 即回落。APP/SQLite 路径同核。
- [ ] opencode_parser：尾部 question part（无 answers）→ Waiting（替代一律 Running）。
- [ ] 防误判：已答/历史问题不触发（有 result/answers 即排除）；回归测试含「运行中不误红灯」。
- 验收：codex/opencode 问题待决 → 看板红灯；回答后回落。

### T2 · 问答卡挂载放宽（数据形态门，非状态门）
- [ ] QuestionCard 挂载条件从 `status==="waiting"` 放宽为非结束态挂载，由组件可用性自隐（问答端点本就不看状态，纯消息形态判定）。
- [ ] 误判面分析落注释：已答问题（其后有 tool-result）被排除；正常运行的上下文不会凑出「未决 questions[] 形态」。
- 验收：codex/opencode 问题待决 → 详情页出问答卡（丁1 后红灯也出）。

### T3 · kimi 计划审批卡（不含 plan 正文渲染——计划文件卡已覆盖）
- [ ] 数据源=wire `interaction.request`（plan_review/ExitPlanMode）结构化字段 → 出「计划待确认卡」：Approve / Reject / Revise 三按钮 + 状态回显；注入走 R1 导航确认安全序列（从解析高亮算步进）。
- [ ] kimi 问答卡的键序分族按丁9 探测结论调整（TwoPhaseSelect 数字可靠性）。
- [ ] codex 多问题问答（Question 1/3 + ←→ 导航 + 每题 enter 提交）键序支持（`#[ignore]` 实机探测定序）。
- 验收：kimi Ready-to-build → 三钮卡；点 Reject 必须 Reject（本批安全验收核心）。

### T4 · codex「计划待确认」卡
- [ ] 预期对话框启发式：尾部出现计划提案（proposed_plan 消息，T4 已识别）→ 会话进入「计划待确认」预期态 → 详情页提示条+「检查终端对话框」按钮 → 屏读解析 Implement-this-plan 选项 → N 选项卡（R1 键序档：codex=数字直选）。
- [ ] 计划批准卡主体聚合计划全文（T8 机制复用）。
- 验收：codex 计划提案 → 手机可见「计划待确认」→ 检查 → 三选项按钮 → 点按生效。

### T5 · 注入守卫（全工具通用，一处实现两处接入）
- [ ] 对话框在场检测原语（复用 T5 屏读解析：可见窗口存在编号选项对话框即「在场」）。
- [ ] 接入点①模式切换：注入前检测，在场 → 拒绝 + 「终端有待决对话框，请先处理」回执。
- [ ] 接入点②composer 直发：问答卡在场（前端判定）→ 发送转「作为回答发送」自由文本序列（placeholder/按钮态变化）；后端侧问答应答注入本身已在卡片动作内。
- [ ] `#[ignore]` 实机：对话框待决时点模式按钮 → 被拒且有提示；composer 发送 → 走自由文本作答不再误勾选。
- 验收：图6/图7 场景不再发生（模式按钮不变选项、打字不变勾选）。

### T6 · 斜杠豁免（保溯源的折衷设计）
- [ ] `/` 开头的消息免 `[mobile …]` 前缀（斜杠=命令语义，任何附加文本都会破坏命令/参数）；**不改后缀签名**（斜杠命令带尾参会被当参数解析，破坏 `/plan on` 这类带参命令）。
- [ ] 溯源补偿：此类消息审计记独立 `action=slash` + 设备名（终端不留痕，MAM 审计页留痕——审计本就逐条带 device_name）。
- [ ] 确认戳语义适配：斜杠消息的 stamp 命中核验（claude 斜杠在会话文件的记录形态需核对，`#[ignore]` 实机一次）。
- 验收：移动端发 `/permissions`（codex）、`/plan on`（kimi）→ 命令触发；审计页可见 slash+设备。

### T7 · 模式二维模型 + 四家回读全开
- [ ] 统一模型：二维工具（codex/kimi）=「模式组 Default/Plan」+「权限组三档」分两组按钮，**斜杠直达**（kimi：`/plan on|off`、`/permission`、`/yolo`、`/auto`；codex：`/plan`、`/permissions`+picker 确认键——两段式注入）；单轴工具（claude/opencode）=一组循环切换+当前模式回显。
- [ ] 回读校准：claude 环序表（acceptEdits→plan→auto→manual 已实测）+ kimi 底栏词表 + codex 底栏词表（Default 靠缺席推断的口径处理）；`parse_mode_from_screen` 按家扩表，逐例屏快照回归。
- [ ] opencode `Build` 标签改显「默认」；codex 枚举如含 untrusted/on-failure 标 legacy（官方已退役）。
- [ ] 约束落地：codex `/plan` 运行中不可用（官方）→ 运行中点 Plan 的回执如实提示。
- 验收：四家切换后卡片回显当前模式；「模式未知」从常态变为例外。

### T8 · 多选提交补完 + codex 计划渲染
- [ ] claude 多选提交序列补 Review 屏处理（提交后屏读检测 Review 屏 → 自动 '1' 确认；检测不到如实回执）；卡片在序列完成前显示进行中态而非「已发送按键」了事。
- [ ] codex 计划卡 markdown 渲染补全（用 17:19 rollout 实录做夹具：`###` 标题、列表、粗体）；核对升格未触发的那条记录的解析路径。
- 验收：多选勾选→提交→Review 自动确认→回合推进一气呵成；codex 计划卡标题/列表完整渲染。

### T9 · 自由文本统一 + kimi 问答键序定案
- [ ] 问答卡内嵌自由文本输入框（提交=各工具自由作答序列：claude 定位 Type something→文本→Enter；codex tab notes；opencode 选 Type your own answer→文本→Enter；kimi Other/feedback——各 `#[ignore]` 实机定序）。
- [ ] kimi 问答对话框数字行为实测（R1 复审 Important-1）→ 键序分族定案。
- [ ] 与 T5 的 composer 转向共用同一序列出口。
- 验收：claude 多选题自由补充要求 → 经卡内输入框或 composer → 以自由作答提交，不错勾选项。

### T10 · 收口
- [ ] 六门禁全绿；验收清单 F 节（批次丁人工项）；台账记批 + handover；停下等 review。

## §4 总验收（八条）

①codex/opencode 问题待决红灯+问答卡出现、可作答；②kimi 计划审批三钮卡且 Reject 必达；③codex 计划待确认卡可见可点；④对话框待决时模式按钮被拒有提示、composer 转自由作答（不错勾选项）；⑤移动端斜杠命令可触发且审计留痕；⑥四家切换后回显当前模式；⑦claude 多选全链（勾选→提交→Review）一气呵成；⑧codex 计划卡 markdown 完整。六门禁全绿。

## §5 不入本计划

- kimi server API 程序化应答（用户未开 server；开产后另行升级——注入方案已满足功能）；
- opencode 会话内权限切档（官方无此能力，权限属配置文件域）；
- codex Permission Profiles（beta 自定义档）；
- 协议路线（claude SDK/codex app-server 托管）——三期；
- zcode/dsh/workbuddy/openclaw 交互卡（沿用既有判定）。
