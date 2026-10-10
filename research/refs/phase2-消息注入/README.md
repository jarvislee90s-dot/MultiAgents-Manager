# phase2-消息注入 · 调研参考库（本地，不入库）

> 本目录是二期（消息注入）的调研沉淀入口。宪法附 B 指向 `research/refs/phase2-消息注入/`；本 Windows 工作树于 2026-09-18 建立（Mac 侧主库若有同名目录请合并本索引）。

## 文档索引

| 日期 | 文档 | 一句话结论 |
|---|---|---|
| 2026-09-18 | [参考架构-输入投递机制调研](2026-09-18-参考架构-输入投递机制调研.md) | 七系统源码实证：宿主/协议/复用器三模型，无一家注入「别人家已开终端」；Windows 注入唯一同场景先例 = cc-discord-remote |
| 2026-09-18 | [Windows控制台写入语义与消费墙复核](2026-09-18-Windows控制台写入语义与消费墙复核.md) | **推翻 M6「消费墙」机制学结论**：OS 输入缓冲无界、122 非本 API 语义（三重源码）；M6 方法论硬伤清单 + 10 项补测实验 + 正确注入姿势清单 |
| 2026-09-18 | [codex-claude审批模式与取证手册](2026-09-18-codex-claude审批模式与取证手册.md) | codex 0.154 源码级审批原文与键位矩阵（y/esc/n/a/p）；Task 13 失败根因=auto 档工作区写自动放行；30 分钟确定性取证手册；M11 无头审批协议通道 |
| 2026-09-18 | [M6R-windows注入探测报告-v2](2026-09-18-M6R-windows注入探测报告-v2.md) | M6R 全量实证：消费墙不存在、两族定案、按家分支（claude/kimi=VT 方向键、codex=VK） |
| 2026-09-19 | [opencode-windows注入规格](2026-09-19-opencode-windows注入规格.md) | A 族 0x0208；SQLite 对账；慢消费者必须背压节流（固定节流丢提交回车） |
| 2026-09-19 | [zcode-APP形态首触调查与互通矩阵](2026-09-19-zcode-APP形态首触调查与互通矩阵.md) | **APP 类判定**（无 TUI 包/无控制台）；zcode 另带完整 CLI（`-p` 无头 + `app-server` NDJSON 协议，均实测可驱动）；**在册只影响 ZCode 桌面侧栏显示，不影响无头驱动/落盘/MAM 出卡**；resume 补正为 `--resume <id>`/`-c`（原记 None 系 CLI 不在 PATH 所致） |
| 2026-09-20 | [问卷交互跨工具矩阵](2026-09-20-问卷交互跨工具矩阵.md) | 四家「向用户提问」机制：opencode/kimi/claude 三家 GO（存储+键击双实测）、codex GO 带复验条件（relay 403）；四家问题数据全会话文件直读，T10 问答卡不依赖屏读 |
| 2026-09-21 | [claude-askuserquestion-按键语义探测](2026-09-21-claude-askuserquestion-按键语义探测.md) | K1–K11 键位定案（单选数字直选即交无回车/Esc=拒答/多选三段式/自由文本先定位）；**pending tool_use 在 relay 环境不落盘（真实交互会话已证伪——会落盘，通道 B 可用）**；附带发现 ConIn.ps1 缺 CharSet=Unicode（CJK 封送必败，技能库原件待同步）与 claude 应用层 CJK 乱码疑云（F-2，待验） |
| 2026-09-21 | [审批对话框N选项屏读解析实机探测](2026-09-21-审批对话框N选项屏读解析实机探测.md) | **三类 N 选一对话框全部实机触发并解析**（claude 计划批准 / codex Implement this plan / kimi Ready to build）；**抓获两个实现缺陷**：① 光标标记前缀 `›`(codex)/`❯`(claude)/`▶`(kimi) 使首项永不匹配 → 整框降级二元卡或簇错位拼接正文列表（已修+回归锁）；② kimi 确认键是 **Enter** 而非矩阵记载的第二个数字（已修+回归锁）。**另发现**：claude 计划批准框**数字键无效**（须 ↓+Enter）——与 AUQ 语义不同。**R1 复评已修（2026-09-21）**：claude 无效 + **kimi 数字通道不可依赖（可能误批准，安全缺陷）** → 审批侧引入**按对话框模型的键序档**（`DigitDirect`=codex / `NavigateConfirm`=claude·kimi），导航序列按**解析到的当前高亮位**算循环距离 → `[↓×k, Enter]`（无高亮则拒绝）；本机端到端复验通过（目标=2 → 状态栏 `manual mode on`）。另修：降级态**防重警示脚注**（R1-3）+ `dialog:N` 审计归 `approve`（R1-4） |
| 2026-09-21 | [四家模式切换shift-tab实机探测](2026-09-21-四家模式切换shift-tab实机探测.md) | **四家 shift+tab 全部生效**（VK_TAB + SHIFT_PRESSED 修饰位，底栏均明示 `shift+tab to cycle`）；**opencode 档位屏读回读实测正确**（`Build`↔`Plan` 循环 4 次，实现侧 `parse_mode_from_screen` 对 5 份真实快照 5/5 正确）；**偏差**：codex 实测亦用 shift+tab（非仅斜杠命令）；三家底栏档位文本实际可读而实现侧偏保守（均申报待裁决） |
| 2026-09-21 | [claude打断式插队Esc语义链实机探测](2026-09-21-claude打断式插队Esc语义链实机探测.md) | **确证 T9 的「Esc 优先序」**：busy 注入正文 → Enter 落**内部队列**（底栏原文 `Press up to edit queued messages` = 计划书问题 10 现场证据）；**Esc 中断后队列消息被自动取出开新回合**（`Interrupted · What should Claude do instead?` → `✢ Photosynthesizing…`，消息不丢）；K2 的「禁止数字后补 Esc」是问答场景禁令，与插队场景语义相反但依据同一事实（Esc=中断 busy 回合） |
| 2026-09-21 | [codex-request_user_input-键位实机补测](2026-09-21-codex-request_user_input-键位实机补测.md) | **补齐矩阵遗留条件项**（relay 403 已通）：codex 数字单键**即选即交**（三取样，落盘 ≈300ms）、↓+Enter 亦可、**Esc=中断整个回合**（非 claude 的 is_error 拒答）、Tab=备注、←→ 切题；**确认屏数字无效只认 Enter/Esc**（推翻源码级 B 级记载）；pending `function_call` **即落盘**（≈43s 窗口，通道 B 实时可用）→ MAM 侧 codex 由只读**升格**为可作答（cancel 仍拒） |
| 2026-09-21 | [happy项目审批与模式切换调研](2026-09-21-happy项目审批与模式切换调研.md) | **「远程端复刻终端审批/提问/plan 交互」的 UI 与状态机标准答案**（SDK 托管架构不可抄、UI 全可抄：多档批准按钮/兜底渲染/ExitPlanMode 三联动/统一模式枚举+各工具映射/永不自动批准清单）；三期协议路线参考实现；批次丙 §2 基准与 T5/T6/T8 据此起草 |
| 2026-09-21 | [claude-notification-message-取证](2026-09-21-claude-notification-message-取证.md) | **推翻计划书 §T1 的 message 判据假设**：AUQ 待答与真实审批的 `Notification(permission_prompt)` 的 message **逐字相同**（`Claude needs your permission`）→ 判据必须落在 **`PermissionRequest.tool_name`**（AUQ 时=`AskUserQuestion`+完整 tool_input）；且事件文件覆盖写使 Notification 常为唯一可见事件 → helper 需把未决 AUQ 进入信号**承接**过那一跳（30s 窗） |
| 2026-09-22 | [codex-kimi权限菜单与FullAccess三段式-用户实机取证](2026-09-22-codex-kimi权限菜单与FullAccess三段式-用户实机取证.md) | **用户手工实机取证**（屏幕逐字转抄），**推翻丁T4 的「权限菜单无编号」假设**：codex 菜单**有编号**（`1. Read Only`…`› 2. Ask for approval (current)`…`4. Full Access`）→ 原行首匹配判据在真机定位 **0 项**、codex 权限切换完全不可用；kimi **无编号**（缩进 + `❯`，每档下跟一行描述）→ 评审提出的「连续成簇+紧邻标题」**会误伤 kimi**。另实锤：**Full Access 有第三段确认框**（`Enable full access?`，1/2/3 档无）；claude 的 `/permissions` 是**色块高亮**（打印看不出选中，不可走屏读定位）、**opencode 无该命令** → 两家单轴 shift+tab 设计获反证。结论落地：定位改**关键词包含 + 命中≥2 即跳过**（编号不再是判据）、导航改**闭环**（每步复核位移恰好 1 行）、补第三段确认与成功回执核验 |
| 2026-09-22 | [codex权限菜单三段式屏读对账与D20时序标定](2026-09-22-codex权限菜单三段式屏读对账与D20时序标定.md) | **agent 自 spawn conhost 实机探测**（八条铁律），对丁T4 收尾后的判据做真机对账：① 权限菜单定位 **4/4 项**、一致性闸通过（关键词判据在真机可用）；② 光标标记实测 **U+203A `›`**（与 `CURSOR_MARKERS` 一致）；③ **闭环导航「位移恰好 1 项」成立**——**探测器一度按屏上行号测量得 +2 而误判「实现会恒中止」，经喂实现侧同一判据复核为项索引 +1（教训：屏上「行」与判据侧「项」是两套坐标，每项占 2 行）**；④ 方向键**VK 形态生效、VT 序列无效**（B 族，与实现侧族规格一致）；⑤ **Full Access 第三段确认框**真机弹出且确认框解析唯一肯定项成功；⑥ 终态 **`• Permissions updated to Full Access`**。**D20 时序标定**：菜单 260ms / 确认框 140ms / 回执 264ms（均 ≪ 1500ms 窗）；**推论并标注待复验**——用户观察②「切换太快」的主因更可能是「投递后不等就发下一个键」（旧值 600ms 亦够），窗放大是加固 |
| 2026-09-22 | [戊探定案表汇总](2026-09-22-戊探定案表汇总.md) | **批次戊 Stage 1 一页纸定案表**（四工具×全部已测交互：注入序列→预期屏读→回执/落账判据 + 〔矛盾〕清单）——Stage 2 编码的键序档输入，用户过目用 |
| 2026-09-22 | [戊探A-opencode多选多题键序](2026-09-22-戊探A-opencode多选多题键序.md) | opencode 1.18.32 问答全链定案：**enter=真 toggle（空格两形态均无效——用户猜测不成立）**、数字=toggle、tab 前向循环（新发现 h/l 切页）、own answer=↓定位→enter 开输入行→打字→enter（裸打字被当导航/勾选）；落账 answers=二维数组按题序+`multiple` 字段；勾选态属性可检（0x000B/0x000F）；esc 取消=error+dismissed |
| 2026-09-22 | [戊探B-kimi问答键序定案](2026-09-22-戊探B-kimi问答键序定案.md) | kimi 2.0.2 问答定案：单选数字可靠收口（char/VK 各 3/3 直达 Review）；多选切勾三通道全成立（**数字/回车/空格**）；tab 一次直达 Submit、确认键三通道全通；Other 全链 ✓；落账 resolved+method；方法发现：注入器 UnicodeChar 域携控制字符=假阴性源 |
| 2026-09-22 | [戊探C-codexTab备注全链](2026-09-22-戊探C-codexTab备注全链.md) | codex 0.155.1 Tab 备注定案：就地备注行 `› Add notes`；打字 VK 携带字符逐字精确到达；落账=`answers.<qid>.answers` 第二元素 `user_note: <全文>`；备注态 ←/→ 不切题、Enter=提交当前项+备注（无未答确认屏）；多题每题 [Tab→备注→Enter] |
| 2026-09-22 | [戊探D-权限菜单屏读底料](2026-09-22-戊探D-权限菜单屏读底料.md) | 权限菜单三态底料（codex+kimi）：codex 底锚定+短语锚、**busy 菜单照常打开**（官方「运行中不可用」UI 层不成立）、**问答待决=菜单打不开+Enter 吃掉交默认答案**（原生层未复现 N2 混屏，留对账）；kimi 两行组+五锚充分（**簇判据可退役**）、不自洽触发行=`Never interrupts you;…`、待决叠加=吞键+误选污染待决态 |
| 2026-09-22 | [戊探E-claude对话框选项锚点底料](2026-09-22-戊探E-claude对话框选项锚点底料.md) | N5 误纳复现（正文编号簇 vs 真簇同屏）+ 锚点定案：**离底栏最近合格簇优先 2/2、标题行逐字节稳定**；误纳夹具固化（30 行原文）；数字键 ×5 零效果（裁决加固） |
| 2026-09-22 | [戊探F-打断式插队扩展底料](2026-09-22-戊探F-打断式插队扩展底料.md) | 三工具插队语义链定案：claude 丁T9 复现+新 hint `ctrl+x ctrl+s`+**早 Esc（≤~8s 无输出）=取消发送**；codex 单次 Esc=立即中断+草稿保留**需手动 Enter**（`<turn_aborted>` 落账）；kimi=**排队制不丢**（回合结束 50–86ms 自动开新回合，修订「busy=丢失」）；Ctrl+S 注入 ×4 零效〔矛盾〕 |
| 2026-09-22 | [戊探G-D20拍数回填](2026-09-22-戊探G-D20拍数回填.md) | D20 拍数实测：Esc→单帧停 ≈250/269ms、**稳定判据主读数 ≈353/374ms**、0/3 瞬态复发 → **建议 TURN_STOP_POLL_TOTAL_MS 维持 3000、TURN_STOP_STABLE_FRAMES 维持 2**（只写建议未改代码）；附带破案 CJK 注入 0x8007007A=CLR 封送伪影（Rust FFI 6/6 正常） |
| 2026-09-24 | [claude多选多题键序-用户实机取证](2026-09-24-claude多选多题键序-用户实机取证.md) | **用户手工实机取证（claude 2.1.278，npm 2026-09-22 自 2.1.251 升级）**：多选屏**空格=切勾、数字=无反应**（推翻 K8，三证并记不裁决）、↑↓=题内移动、←→=切题（`Type something`/`Next` 两行不生效）、`Next`+回车=下一题、Submit 页 1/esc；底栏改版 `Tab/Arrow keys to navigate`=**版本漂移实锤**；多题页签 `☒/☐`=已答态非当前页。实现口径：toggle 不依赖数字，走闭环导航+空格+屏读校验翻转（形态不被消费即干净中止） |
| 2026-10-09 | [codex-160-问答屏读底料](../../../../docs/superpowers/specs/2026-10-08-codex-160-question-屏读底料.md) | codex 0.160.0 问答面板四闸门①活体取证（**随实施分支入库** `docs/superpowers/specs/`——research/ 在 gitignore，本行只作指向）：S1–S4+A–H 定案；**箭头族注入全无效→vim j/k 走位、h/l 切题**；数字=选中+推进原子（末题禁用）；空格 toggle（已答换选非原子——焦点消歧注记）；notes 文本不随卷提交→自由作答收缩为具名中止；确认屏 0.160.0 未复现（unverified 构造词形）；摘要屏/归零词形/footer 三段四段逐字 |

## 关联材料

- M6 探测原始包：`%USERPROFILE%\mam-probe\`（报告/20 脚本/evidence；⚠️ conin.log 已被 13:00 重跑覆盖）+ `mam-probe-m6.zip`
- M6 报告结论修正对照表：见《Windows控制台写入语义》§七
- cc-discord-remote 长度上限核实结论（2026-09-18 clone 实证）：**注入方向无任何长度上限**——限制全在 Discord 平台层（2000/25MB），>2000 长文转附件后仍全文注入；80 字符/50ms 分块 + Enter 丢失回查 + 屏幕稳定确认，详见《Windows控制台写入语义》§四

## 对设计的当前输入（2026-09-18 头脑风暴进行中）

1. Windows 注入引擎按 cc-discord-remote 配方重写（80 字符/50ms 分块、只发 keydown、VT 序列发方向键、写前侦察控制台模式、软背压）——先跑修正版补测再动引擎。
2. 通道决策模型：进程存活→注入；未开窗→无头（协议模型）；长消息→文件指针（落 markdown+短指令）——待用户裁决主通道定位。
3. 审批映射：codex 触发词换源码原文短语；键位 y/n 恰好正确待取证确认。
