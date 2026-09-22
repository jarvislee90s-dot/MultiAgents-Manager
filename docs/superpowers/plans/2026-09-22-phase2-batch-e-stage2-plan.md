# 批次戊 · Stage 2 编码计划（任务 × 定案表 × TDD 夹具 × 文件地图）

> 2026-09-22 定稿。依据：设计文稿 `2026-09-22-phase2-batch-e-design.md`（§0 裁1–裁20）+ 差距矩阵 `2026-09-22-phase2-batch-e-gap-matrix.md`（§6.3 裁决/§6.4 实测键序/§6.5 守卫原则/§6.6 矛盾审计）+ 键序大词典**清淤版**（单一口径）+ 定案表汇总 `research/refs/phase2-消息注入/2026-09-22-戊探定案表汇总.md`。
> **执行序（矩阵 §6.6 结论）：先改 A 类矛盾（T1–T5），再补 B 类新行为（T6–T12），T13 收口。**
> 执行方式：subagent-driven-development——每任务一个实现子代理（不并行）→ **单评审员一轮**（规格+质量合并）→ Important/Critical 由实现者修复后同评审员复审。TDD：夹具先行失败测试 → 实现 → 端到端 `#[ignore]`。
> 红线：不 push、不动 main、不动 `docs/MASTER-PLAN.md` 与两份批次戊文档的 §0 裁决；六门禁全绿（cargo 一律 `CARGO_TARGET_DIR=target/gate-run` 且在 `src-tauri/` 下；bin 须 `--features hook-listener`；vitest 终跑勿与 cargo 并行）；实机项一律 `#[ignore]`；台账如实；每任务独立 commit（`feat(mfix-e): T<N> …` 风格，文档 `docs(mfix-e): …`）；Esc 类注入单次+屏读确认；opencode Ctrl+C 一律禁注（裁19）。

## §0 编号映射（废止旧两套戊编号）

| 本计划 | 设计文稿 §4 旧编号 | gap 矩阵 §3 旧编号 | 覆盖 |
|---|---|---|---|
| S2-T1 | —（§6.6-A1 新增） | — | 插队撤回窗口防护 |
| S2-T2 | 戊5（部分）+§6.6-A2 新增 | 戊4（部分）+§6.6-A2 新增 | claude 审批数字优先双路径（A2） |
| S2-T3 | 戊6 | 戊5 | 权限菜单解析加固（A4/N6/N2②） |
| S2-T4 | 戊6（部分）+戊7（部分） | 戊5（部分）+戊8 | codex Default 回读+守卫前置（A5/N2①③） |
| S2-T5 | 戊5 | 戊4 | 对话框解析锚点+布局契约（N5/裁12） |
| S2-T6 | 戊3 | 戊3 | kimi 问答键序重构（N7/A3） |
| S2-T7 | 戊2+戊4（codex 面） | 戊1 | codex 多题卡+Tab 备注（N1/issue#78） |
| S2-T8 | 戊1+戊4（opencode 面） | 戊2 | opencode 多题/多选卡（N4） |
| S2-T9 | 戊4 | — | 三家自由作答统一（issue #78 关闭） |
| S2-T10 | 戊8+范围扩展（opencode 扩入） | —（矩阵 §2「其他」插队行） | 插队扩展三工具+队列语义 |
| S2-T11 | 戊7 | 戊6 | 回读校准+档位高亮（N8） |
| S2-T12 | 戊9 | 戊7 | 配色统一蓝系+composer 占位（裁11） |
| S2-T13 | 丁T6' | 戊9 | 收口 |

依赖：T9 依赖 T6/T7/T8；T10 依赖 T1；T11 的 kimi 权限档回读依赖 T3（回执锚同一解析）；其余独立。T1 必须最先（唯一现行代码级危害）。

## §1 夹具库（TDD 输入，先建后开工）

全部取自探测档案「裁15 逐行原文」节，拷入**仓库根** `tests/fixtures/e-stage2/`（跨语言共享先例=`tests/fixtures/plan_pending_cases.json`：Rust 侧经 `../tests/fixtures/e-stage2/` 读、vitest 同路径读）：

| 夹具文件 | 来源档案 | 用途 |
|---|---|---|
| `claude-approve-dialog-e2.txt` / `-e1.txt` | 戊探E（误纳复现 30 行 + 干净对照） | T5 锚点解析、T2 渲染等待门 |
| `claude-recall-state.txt` / `claude-interrupted-state.txt` | 戊探F claude 段（撤回态=消息回输入行；中断态=`Interrupted` 标记） | T1 撤回窗口判据 |
| `codex-perm-menu-idle.txt` / `-busy.txt` | 戊探D codex 段（4 份 dump 行号一致；busy 混屏含滚动正文） | T3/T4 |
| `kimi-perm-menu-{permission,yolo,auto}.txt` + `kimi-perm-receipt.txt` | 戊探D kimi 段 + K-1 回执实录 | T3/T11 |
| `kimi-question-multiselect.txt` / `kimi-question-multi-q.txt` / `kimi-question-review.txt` | 用户 K-5 实录（多选题屏/多题屏/Review 汇总屏）+ 戊探B 单题 Review 屏 | T6 |
| `codex-notes-inline.txt` + rollout `user_note` 行 | 戊探C | T7 |
| `opencode-confirm-page.txt` / `-checked.txt` | 戊探A（Confirm 页/勾选态） | T8 |

## §2 任务详情

### S2-T1 · 插队撤回窗口防护（A1，最高优先）
- **定案输入**：词典 §1「Esc（busy 中）②」——已发出且无输出的消息 Esc=撤回回**输入行**（不清空）；矩阵 §6.6-A1。
- **改动**：`src-tauri/src/inject/queue.rs`——`interrupt_first` 在 `wait_turn_stopped` 通过后、投递正文前，屏读检查**输入行非空**（claude 输入行判据：底栏 `❯ ` 提示符后有残留文本）：非空=疑似撤回态 → **中止投递**+回执「未投递：终端输入行有残留内容（可能是被撤回的消息），请人工确认」（不自动清空——清空键未实测，保守）。
- **判据陷阱**：claude 屏上 `❯` **同时**出现在输入行与队列展示区（如 `❯ <排队消息>`、`❯ Press up to edit queued messages`）——非空判据必须区分两者（如：排除已知 hint 串后取**最底部** ❯ 行 + 相对状态栏位置；以两份夹具驱动定案，不得只匹配「任意 ❯ 行有尾文」）。
- **TDD**：①撤回态夹具（输入行含文本+无 busy 串）→ 不投递+回执如实；②中断态夹具（`Interrupted` 标记+空输入行）→ 正常投递；③既有 5+6 例投递时机锁全绿；④变异：去掉非空检查 → ①红。
- **DoD**：上述全绿 + C-25/C-47 验收判据更新（含撤回防护语义）。

### S2-T2 · claude 审批数字优先双路径 + 渲染等待门（A2/裁18）
- **定案输入**：词典 §1「计划批准框·数字字符」（用户终裁：数字直接选中；引擎规则：注入前屏读确认选项簇已渲染）。
- **改动**：`inject/approve.rs` 键序档 + `inject/dialog.rs`——claude 档改 **DigitFirstWithVerify**：①注入前屏读确认选项簇已渲染（复用对话框解析，N 项齐才算就绪，未就绪在 `MENU_POLL_TOTAL_MS` 窗内轮询）；②发数字；③屏读验证生效（对话框消失/模式词/plan 卡）；④未生效 → 既有 NavigateConfirm 导航回退。**kimi 审批档 NavigateConfirm 维持不动**（数字禁令，加回归锁）。
- **TDD**：数字成功（夹具对话框→注入→消失）/ 数字无效转导航 / 未渲染不注入（空窗夹具）/ kimi 维持导航锁 ×2。
- **DoD**：`#[ignore]` 实机端到端（claude 计划批准框走数字路径成功）。

### S2-T3 · 权限菜单解析加固·codex+kimi（A4/N6/N2②）
- **定案输入**：词典 §2「权限菜单三态」「busy 换档生效」+ §3「权限菜单定位」（五锚/两行组）——戊探D 两段。
- **改动**：`inject/mode.rs` `locate_menu_items` 分家——**codex**：标题 `Update Model Permissions` ↔ footer `Press enter to confirm or esc to go back` 短语窗 + 编号项模式（折行不产新项）+ `(current)`；**kimi**：标题 `Select permission mode` ↔ footer 窗 + **两行组**（标签行+缩进描述行归属）+ `▶`(U+25B6) 光标 + `← current`；关键词簇判据退役（删除，不留兜底——依据=戊探D kimi 段五锚实证 + N6 失败行归因；词典已按裁20 清为单一口径）。
- **TDD**：kimi 三档定位 + 「总是询问」跨折行可达（N6 失败行 `Never interrupts you;…` 正确归属 Never Ask 描述行）；codex 4 项定位 + busy 混屏 dump（滚动正文 row≤18）不污染菜单块；折行归属锁。
- **DoD**：kimi「总是询问」`#[ignore]` 实机切换成功（回执 `Permission mode: Always Ask`）。

### S2-T4 · codex Default 回读 + 守卫前置收口（A5/N2①③/裁17）
- **改动**：①`inject/mode.rs` 回读——codex Default=底栏无模式词且无 `Plan mode` 字样的**缺席推断**；②守卫——`blocks_control_injection` 收窄为**仅拦「对话框在场」**（注入前单次屏读快照，D20 例外条款），busy 态一律放行（裁17）；③前端权限/模式按钮：问答待决 → 置灰+原因文案「问答未处理时不可切档（切档键会被问答框吞掉/误交答案）」；④kimi 待决态**含回车的整条注入硬拒绝**（斜杠命令也不行——Enter 会误选污染待决态）。
- **TDD**：busy 放行锁 ×2（codex/kimi）；待决拒绝锁 ×2；置灰原因文案组件测试；Default 缺席推断词表锁。
- **DoD**：N2 三连失败场景（20:02 审计）以新守卫路径回归——不可达+原因透出。

### S2-T5 · 对话框选项解析锚点 + 布局契约（N5/裁12）
- **改动**：①`inject/dialog.rs`——选项簇定位改「**离底栏最近的合格簇** + 标题行锚（`Claude has written up a plan and is ready to execute. Would you like to proceed?`）」，弃最长簇；②前端卡体**总高度上限 + 长内容默认折叠 + 点开**（裁12：任何卡片不得把 composer 顶出首屏）。
- **TDD**：e2 夹具（正文编号 3–8 项与真选项同屏）→ 解析出 3 个真选项；e1 对照不回归；布局组件测试（上限/折叠/展开）。
- **DoD**：误纳复现夹具永久进回归集。

### S2-T6 · kimi 问答键序重构（N7/A3）
- **定案输入**：词典 §3 问答节（清淤版）+ K-5 实录。
- **改动**：`inject/question.rs` 键配置——**多选**=切勾单次（数字/回车/空格任一）→ tab 切 Submit → '1'（char/VK）或 Enter 确认；**多题**=**DigitAdvance**（数字直选+自动推进，**无尾 Enter**——A3 禁令）；**Review 汇总屏**（`Ready to submit your answers?` + `[1] Submit [2] Cancel`）解析为提交判据；**Other**=行数字→打字→回车→Review；**单题 TwoPhaseSelect 维持**。
- **TDD**：多选切勾单次净效果锁（N7 双切抵消回归）；多题无尾 Enter 锁（尾 Enter 会误作用下一题——用 K-5 夹具两题屏证明）；Review 汇总屏解析；Other 全链；落账 `interaction.resolved`（method 字段）对账。
- **DoD**：`#[ignore]` 实机多选+多题各一全链。

### S2-T7 · codex 多题卡 + Tab 备注（N1/issue#78 codex 面）
- **改动**：①前端 `SessionDetail.tsx` 多题卡逐题渲染（题干+选项+已答态）；②后端键序：数字即选即交+自动推进 / `←→` 切题 / 末题 Enter=submit all / **Tab=备注文本框 toggle**（框内打字含数字入文本、框内 Esc 退回选项；多题每题 `[Tab→备注→Enter]`）；③落账对账 `answers.<qid>.answers` 第二元素 `user_note: <全文>`。
- **TDD**：键序档路由锁；备注 toggle 夹具（戊探C footer 变化）；多题卡渲染组件测试；rollout 对账摘录夹具。
- **DoD**：`#[ignore]` 实机 2 题全链（1 题带备注）。

### S2-T8 · opencode 多题/多选卡（N4）
- **改动**：①前端逐题卡（tabs 形态，`(○)/(✓)` 已答标记）；②后端键序：tab 前向切页（**h/l 亦是切页键——注入器打字前须确认不在题页裸打字**）/ ↑↓ 定位 / **enter 或数字=toggle**（空格禁用——无效键）/ Confirm 页 enter 提交 / own answer=↓定位→**enter 开输入行**→打字→enter（裸打字守卫）；③esc 取消=`state.status="error"`+dismissed 回执；④SQLite 落账对账（answers **二维数组按题序**+`multiple` 字段，副本查询——verify-oc 模式）。
- **TDD**：toggle 双向（勾/取消勾）锁；Confirm 页提交锁；own answer 开行守卫锁；落账二维数组对账。
- **DoD**：`#[ignore]` 实机多选+多题各一。

### S2-T9 · 三家自由作答统一（issue #78 关闭；依赖 T6/T7/T8）
- **改动**：composer freeText 分族序列全开——claude=既有（K4–K7 定位→打字→Enter）；kimi=Other 行；codex=Tab 备注；opencode=own answer 开行；卡内自由作答入口与 composer 双入口一致；占位文案一致性（只读卡不承诺发送）。
- **TDD**：四家 freeText 路由锁；composer 占位四态快照测试。
- **DoD**：issue #78 可关（三家自由作答实机各一）。

### S2-T10 · 插队扩展三工具 + 队列语义（依赖 T1）
- **定案输入**：词典 §1–§4 busy/插队行（清淤版）+ 裁16/裁19。
- **改动**：`inject/mode.rs` `supports_interrupt` 扩展 + `inject/queue.rs` 分家键序——**codex**=打字→Tab 入队→Esc 直插（回退=草稿+Esc+手动 Enter）；**opencode**=Esc 打断直插（草稿态）；**kimi**=busy 直接投递即**排队制**（回合结束自动开新回合，无需 Esc）+ 可选 **Ctrl+S 立即插队**（产品 Rust 引擎通道 `#[ignore]` 复验**通过才上**，不通过则只留排队制）；回执语义按**整队批量**（裁16）；**opencode Ctrl+C 全局禁注**（注入引擎层键黑名单，裁19）。
- **TDD**：四家插队键序档路由锁；Ctrl+C 黑名单锁；kimi 排队回执语义锁（不谎报 delivered）。
- **注**：opencode 插队全链（打字→Esc 后草稿去向/是否需手动提交）词典未定案——`#[ignore]` 实机首验时定案并回填词典，实现不得预填键序。
- **DoD**：codex/opencode `#[ignore]` 实机插队各一；kimi Ctrl+S 复验结论落档（无论过否）。

### S2-T11 · 回读校准 + 档位高亮（N8；kimi 权限档部分依赖 T3）
- **改动**：kimi 权限档回读=**切档回执锚 `Permission mode: <档名>`** + 菜单 `← current` 直读（三档词形已实录）；claude 环序四档词表（acceptEdits/plan/auto/manual 底栏词，丁复审 C 档案）；codex `(current)` 后缀直读（Default 缺席推断归 T4）；前端模式栏**当前档高亮** + 漂移口径（kimi 批准后自动切出 plan=已知漂移，如实显示）。
- **TDD**：三家回读词表锁（夹具=丁复审 C + K-1 回执）；高亮组件测试；漂移口径锁。
- **DoD**：四家模式栏与终端实际档一致（实机抽查各一）。

### S2-T12 · 配色统一蓝系 + composer 占位（裁11）
- **改动**：InteractiveCard tone——审批**对话框卡**（N 选项）并入 sky 蓝系（与问答卡同族）；**二元审批**（允许/拒绝、y/esc 类）保留红系警示（裁11 给的默认；如用户另有指示再调）；composer 占位一致性收尾。
- **TDD**：tone 映射快照测试；composer 占位四态快照。
- **DoD**：N3 关闭。

### S2-T13 · 收口
- 六门禁全绿（lib/bin `--features hook-listener`/preset_v2 基线比对/vitest/`pnpm check`/`build:mobile`）+ fmt/clippy 0 警告；验收清单 **I 节**（人工项从定案表生成，含 `#[ignore]` 实机项清单）；**C-4 旧口径统一**（A6：删「排空回执」批次乙表述，指向 C-47+T1 撤回防护）；**N2 守卫回归确认**（A5 验证）；台账+handover；停下等主线 review。

## §3 验收（对照设计文稿 §5）

- gap 矩阵 §2 四家对照全 ✅（逐格以本计划 DoD 为准）；卡死场景清零；四家卡同设计语言（裁11/12）；模式切换后回显当前档（T4/T11）；issue #78 关闭（T9）；六门禁全绿（T13）。
- 每任务交付物：代码+测试（含夹具）+`#[ignore]` 实机项登记+台账行+独立 commit。实机项允许「登记未跑」如实标注，不得谎报。

## §4 执行纪律（全局）

1. 实现子代理不并行；评审=单评审员一轮（规格+质量合并），Important/Critical 修复后同评审员复审。
2. 夹具必须来自真机屏读原文（§1 清单），禁止手造「看起来也行」的相邻态（批次丁 H-4① 教训）。
3. 词典（清淤版）为唯一键序事实源；任务中发现与词典冲突 → 停下上台账，按裁20 处置（用户实测优先，报主控/用户裁决）。
4. Esc 类注入单次+屏读确认；Ctrl+C（opencode）与 kimi 审批框数字为永久禁令。
5. 完成判据=DoD 全绿，不含「计划写完」。
6. 台账：`.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md` 文末追加「批次戊 Stage 2」节（任务×commit×结论一行一记录）；每任务 commit 前 fmt/clippy+受影响测试面全绿，收口（T13）跑全六门禁。
7. 夹具来源=探测档案正文内的「裁15 逐行原文」节（`research/refs/phase2-消息注入/2026-09-22-戊探*.md`，本地持久）；`%TEMP%` 探测目录不保证存活，仅作补充。
