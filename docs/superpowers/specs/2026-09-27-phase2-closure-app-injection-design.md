# 二期收尾总设计 · APP 类消息注入 + 剩余里程碑编排 v1

> 日期：2026-09-27 · 状态：**总设计定稿待审**（探测批次计划与实现批次计划均另立文档，不在本 spec 内）
> 基线：`origin/main` `78bf114`（v0.5.0-beta.1 已发版；M6–M9 注入主线 + M6R–M9R 加固已交付）；工作分支 `feat/phase2-closure-app-injection`
> 上位文档：`docs/MASTER-PLAN.md`（宪法，F2.3/F2.4/D6/D14 相关）；`docs/superpowers/specs/2026-09-18-phase2-message-injection-design.md`（二期注入总 spec，本批 = 其 M11 里程碑的提前与扩容 + 收尾编排）
> 文档关系（用户 2026-09-27 裁决）：**本 spec（总设计）→ 探测阶段计划（单独立）→ 探测结果回传 → 修订本 spec → 实现批次计划（单独立）**

## 1. 背景与目标

v0.5.0-beta.1 交付了四家终端（Claude Code / Codex CLI / OpenCode / Kimi Code）的手机端消息注入主线（引擎、排队、审批、问答、模式切换、附件、resume）。**APP 形态工具（ZCode / Codex APP / WorkBuddy）至今不可注入**——路由表对 APP 形态一律判「无外部写 API」。本批目标是把注入能力扩展到 APP 形态，并编排二期收尾的剩余工作（M10 交接导出、M11 无头通道、远程新建会话 Phase C），使二期进入可收官状态。

三项新证据（2026-09-27 本机实测 + 官方调研）表明 APP 形态的写通道是存在的，旧「黑盒」结论需要重探：

1. **ZCode**：APP 内嵌 CLI（`resources/glm/zcode.cjs`，0.16.9）有完整无头面——`--prompt` / `--resume sess_xxx` / `--cwd` / `--json` / `--attach` / `--mode` / `--surface terminal|desktop`；用户实机验证无头会话可运行且 APP 重启后可见（与既有 spike 判定 F 一致）。
2. **Codex**：官方 2026-08 新增 `codex queue --thread <id> --message <text>`（本机 0.156.1 在场），走共享 daemon 的 `thread/queue/add`——**与桌面 APP 自身的 follow-up 排队同机制**，线程发现范围含桌面（Atlas/ChatGPT）会话；`codex exec resume` 追加 APP 会话 rollout 已被官方 issue #28259 实证（索引刷新不保证，已知限制）。APP 与 CLI 共用 `~/.codex` 会话文件（adapter 在产口径）。
3. **WorkBuddy**：会话心跳文件暴露每会话 `url`/`endpoint`（如 `http://127.0.0.1:35923`），内嵌运行时为 `cli/bin/codebuddy`（cwd `workbuddy-host-cli`）；官方调研确认 **CodeBuddy CLI 是独立一等产品**（npm `@tencent-ai/codebuddy-code`），带 `--serve` HTTP API（含 `POST /jobs/:id/reply` 向运行中任务追加消息）、`-p --resume` 无头、ACP、SDK；官方文档明示 WorkBuddy = 「使用 CodeBuddy 引擎的应用」。心跳端点很可能就是内嵌 codebuddy 的 serve 实例。

## 2. 裁决记录（2026-09-27 用户对齐，实施不得重议）

| # | 裁决 | 内容 |
|---|---|---|
| 1 | 工具范围 | 核心三家 **zcode / codex APP / WorkBuddy** 全链；**OpenClaw / dsh 各排半天级轻探测**（只判「有没有写通道」，有则追加实现，无则如实登记） |
| 2 | zcode 验收线 | **判定 F 基线**：手机实时回执 + APP 重启/刷新后可见即验收；同批探测 `--surface desktop` 等**免费升级**项，探到即升 |
| 3 | codex APP 通道 | **app-server 协议优先**（用户选定）——经调研落地为：**`codex queue` 为主通道**（它本身就是 app-server 协议 `thread/queue/add` 的 CLI 形态），`exec resume` 兜底；直接 attach 桌面线程不可行（单写者锁，#47193/#25914/#33556 三个 open issue 在求此能力，登记跟踪） |
| 4 | WorkBuddy 路线 | **以官方调研定路线**（调研已完成，见 §6 探测批 PW 系双路线：端点 API + 独立 CLI 复刻，探测后回填定案） |
| 5 | 三期项 | 切换模型（F3.1）/ 原生 APP（F3.9）**维持三期**；本批仅加「斜杠命令经无头通道等效性」探测取证（近乎零成本，为三期铺路）。**宪法零改动** |
| 6 | Mac 段 | Mac 跑 ① 本批 APP 类探测 + ② M7 终验补课（tmux/iTerm2/Terminal 三通道实机）；③ session-create Mac 段**已在跑不重复**，仅交叉验证 1–2 个核心格子；报告全文回传 |
| 7 | 实现范围 | **探测完再裁**——M11 的 claude/kimi/opencode 无头是否并入实现批，等探测报告回传后用户裁决（本 spec 只定无头底座为五家同构设计，扩不扩三家是配置级裁决） |
| 8 | 无头架构 | **方案 A · 一次性进程 per turn**：每条远程消息 spawn 一次无头命令，回执归一后进程退出；常驻连接池/app-server attach 留三期 F3.3。claude 审批双向（`--permission-prompt-tool stdio`）作为「进程存活到 turn 结束」的特例归入 A |

## 3. 二期账本现状与收尾编排

| 里程碑 | 状态 | 本批关系 |
|---|---|---|
| M6–M9 + M6R–M9R（注入主线与加固） | ✅ 已交付（v0.5.0-beta.1） | 底座复用（引擎/排队/审计/门禁/PIN） |
| 远程新建会话 | 🔄 设计定稿 + Phase P 探测计划已挂 `probe/remote-session-create` 分支（Mac 段在跑） | 独立轨道，不并入本批；探测纪律沿用其八条总纪律 |
| **M11 无头通道** | ⬜ 未开始 | **本批提前并扩容**（APP 形态 + 裁决门定是否含三家 CLI）；编号建议沿用 M11 |
| M10 交接导出 | ⬜ 未开始 | 不并入本批（依赖 M7 注入通道已在产，不依赖无头）；排在无头批后 |
| 推送网关 / APK 壳 | 三期收尾（裁决 16） | 不动 |

收尾顺序（建议）：**Phase P+ 探测批 → 裁决门 → M11 无头批（APP 类 + 视裁决三家 CLI）→ M10 交接导出 → 远程新建会话 Phase C → 二期收官**。

## 4. 范围与非目标

**范围**：APP 形态三家工具的手机端发消息全链（通道探测→实现→回执→审计→README 矩阵更新）；OpenClaw/dsh 写通道轻探测；无头公共底座（五家同构）；M7 macOS 终验补课；macOS/Windows 双端口覆盖。

**非目标（不做清单）**：
- 常驻连接池、zcode app-server / codex app-server attach（三期 F3.3；codex 侧单写者锁 open issue #47193 登记跟踪，官方落地即切）
- 切换模型选择器 UI、原生 APP 壳（三期 F3.1 / F3.9）
- zcode 深链打开无头会话（旧探测已证 3.11.2 无会话级路由；仅在新版本探测中复核一次）
- OpenClaw gateway 深做 / dsh 插件生态（轻探测外不展开，宪法 D14 另评口径不变）
- 任意命令/参数拼接暴露（宪法红线，仅固定 CLI 命令形态）
- 远程新建会话（独立轨道，见 §3）

## 5. 通道总表（「探测定案」栏待 Phase P+ 回填）

| 工具 × 形态 | 主通道 | 兜底 | APP 可见性预期 | 定案状态 |
|---|---|---|---|---|
| zcode（APP） | `ELECTRON_RUN_AS_NODE=1 ZCode.exe zcode.cjs --prompt <text> --resume <sess_id> --cwd <项目> --json` | — | 判定 F（重启/刷新后）；`--surface desktop` 探测升级 | 待 PZ |
| codex（APP 托管） | `codex queue --thread <id> --message <text>` | `codex exec resume <id>` | 预期 = APP 原生排队（实时） | 待 PC |
| codex（CLI TUI） | 既有终端注入（不动） | — | 实时 | ✅ 在产 |
| WorkBuddy（APP） | 待定：内嵌 serve 端点 `POST /jobs/:id/reply` | 独立 codebuddy CLI 指向 WB 配置目录 `-p --resume`；再兜底如实登记 | 待探测 | 待 PW |
| OpenClaw | 轻探测定案（sessions CLI / gateway API 面） | — | — | 待 PL |
| dsh（web 宿主） | 轻探测定案（web API / 插件生态线索） | — | — | 待 PL |
| claude（CLI 无头） | `claude -p --resume <session> --output-format stream-json`（+`--permission-prompt-tool stdio` 承接审批） | — | 读链路自然可见 | 裁决门定是否并入 |
| kimi（CLI 无头） | `kimi -p -S <id>` | — | 刷新后 | 裁决门定是否并入 |
| opencode（CLI 无头） | `opencode run` | — | 官方 web 实时 | 裁决门定是否并入 |

> zcode 命令行为 Windows 形态（`ELECTRON_RUN_AS_NODE=1` + `ZCode.exe` 转载 `zcode.cjs`）；macOS 的可执行入口与调用形态随 Mac 段探测对齐后回填。

## 6. Phase P+ 探测批（结构级描述；任务级计划另立文档）

> 纪律沿用 `probe/remote-session-create` 探测计划的八条总纪律（run-id 临时目录、只碰自建会话、日志追加、结束清场、不修代码、结论不超证据、Windows 段先读 `win-console-inject-probe` 技能、环境变量隔离构造首装态）。探测独立于 MAM 应用运行。

- **Windows 段（本机执行）**：
  - **PZ · zcode 七问**：无头 resume 对 APP 在册会话全链；可见性三态（实时/刷新/重启）+ `--surface desktop` 差异；APP 运行中并发安全（SQLite 锁/同会话 turn 冲突）；`--json` 回执结构；`--mode` 审批语义（默认 yolo 风险定档）；斜杠命令等效性（/model 取证）；版本门控基线。
  - **PC · codex 五问**：`queue` 触达桌面 APP 托管会话（空闲/运行态）；queue 回执形态（投递成功/被消费信号）；`exec resume` 兜底实测（0.155.1+ 单写者锁后行为）；MAM session id ↔ thread id 映射；共享 daemon 前置条件（APP 关闭后 queue 可用性）。
  - **PW · WorkBuddy 五问**：内嵌 codebuddy 配置目录与 serve 端点 token 定位；端点 API 面（openapi.json，jobs/reply 在场性）；自建会话 `jobs/reply` 投递实测；独立 CLI（npm 2.158）指向 WB 配置目录 resume（对内嵌 2.115 的版本偏差）；心跳 url 与端口拓扑（每会话一端口 or 全局 daemon）。
  - **PL · 轻探测**：OpenClaw / dsh 各半天，只出「有/无写通道 + 依据」一格结论。
- **Mac 段（用户执行、报告全文回传）**：zcode APP on Mac（PZ 核心项对齐）；WorkBuddy on Mac（PW 路径对齐）；codex APP on Mac（PC 核心项对齐）；**M7 终验补课**（三通道注入实机，claude 必测）；session-create Mac 段核心项交叉验证（1–2 格，不与在跑任务重复）。

**裁决门（探测报告回传后，用户裁三件事）**：① WorkBuddy 路线定案（端点 / 独立 CLI / 登记）；② M11 三家 CLI 无头是否并入实现批；③ 各家可见性验收线与超时/watchdog 定值确认。裁决后**修订本 spec**（通道总表定案栏 + 里程碑表），再立实现批次计划。

## 7. 无头底座设计（裁决 8 · 方案 A）

一次性进程 per turn 的公共设施，全部挂在既有模块缝上：

- **通道适配层（HeadlessSpawner + 端点客户端）**：spawn 型通道（zcode / claude / kimi / opencode / codebuddy CLI）= spawn 无头命令 → 流式读 stdout（`--json` / stream-json）→ **回执归一** → 进程退出；HTTP 型通道（WorkBuddy serve 端点，若 PW 定案）= 端点调用 → 响应归一。两类共用**统一回执结构**（末条 assistant 摘要 + token 用量 + 耗时 + sessionId），各家解析器独立（zcode `--json` / codex queue 输出 / claude stream-json / kimi / opencode / codebuddy）。claude 审批双向 = 同一进程存活至 turn 结束，control_request → 移动端审批卡 → control_response。
- **按会话串行**（宪法 W7 v1.3 定死）：同会话同时至多一个无头 turn，进行中新请求返回排队回执（含队列位置），turn 结束自动依序执行。
- **watchdog 超时兜底**：turn 超时（默认值探测批回填）自动回收进程并回报分阶段失败原因 + 可重试标注。
- **版本门控探针**：投递前校验目标工具版本与 flag 面（zcode 0.16.x 在场 / codex `queue` 子命令在场 / codebuddy `--serve` 在场），漂移即提示复核，不盲发。
- **审计**：`action=headless` 逐条入写审计表（设备、会话、内容摘要、回执、耗时），与既有注入审计同表同口径。
- **排队衔接**：目标会话忙时沿既有黄排队语义；无头通道自身按会话串行，两层不冲突（终端注入与无头不路由到同一会话——W3 路由表既有裁决）。

## 8. 路由表与移动端

- **路由表（routing.rs）扩展**：APP 形态从「一律不可注入」改为按工具分派新通道（zcode 无头 / codex queue / WorkBuddy 待定案）；**可见性预期元数据**（实时 / 刷新后 / 重启后 / 不可见）随路由返回，移动端如实展示（如「消息已投递，ZCode 应用重启后可见」），不谎报。
- **codex 双形态分派**：CLI TUI 托管 → 既有终端注入（不动）；APP 托管 → 无头 queue。分派依据 = 会话宿主形态（adapter 在产判定）。
- **移动端**：无头路由会话的发送交互 = 发送中态（回执未归）→ 回执卡（末条 assistant 摘要 + token + 耗时）→ 失败分阶段原因 + 重试；不可注入工具（探测登记无果的）入口如实置灰并标原因。附件沿 `--attach` / `-i` / 端点参数路由（探测定案）。

## 9. 验收（实现批出口标准）

- 手机对 **zcode 在册会话**发消息 → 无头执行 → 回执（末条 assistant + token）→ APP 重启后可见（判定 F 兑现；若探测升级到刷新/实时则按升级线验收）。
- 手机对 **codex APP 托管会话**发消息 → queue 投递 → APP 内可见（按探测定案形态验收）。
- **WorkBuddy** 按裁决门定案验收；若登记不可注入，README/移动端如实标注并附依据。
- 审计逐条可查；同会话并发守卫实测（双请求第二笔排队回执）；watchdog 超时实测生效；版本门控探针对漂移工具如实拒发。
- 自动化：`#[ignore]` 实机 E2E（发消息 → 回执 → 落盘核验），复用 m9r_e2e 模式。
- README 功能矩阵与 CHANGELOG 更新（APP 类注入列如实呈现）。
- 双平台：Windows 实机全链；macOS 按 Mac 段探测结论同口径实现与编译门禁（实机复验随 Mac 段安排）。

## 10. 风险登记

| # | 风险 | 处置 |
|---|---|---|
| 1 | WorkBuddy 端点无写 API 或 token 不可寻 | 兜底独立 CLI 复刻；再兜底如实登记不可注入（工具差异如实呈现原则） |
| 2 | codex queue 对桌面会话触达性仅有机制佐证（无官方明示） | PC 探测实测；`exec resume` 兜底（#28259 实证）；双通道都不通则 codex APP 登记不可注入并跟踪 #47193 |
| 3 | zcode `--mode` 默认 yolo = 无审批直跑 | 探测定档；实现批默认收紧（build/edit 档起），yolo 需用户显式开启 |
| 4 | codebuddy 内嵌 2.115 vs 独立 2.158 目录/会话格式偏差 | PW 探测实测；版本门控探针拦截不兼容组合 |
| 5 | codex 共享 daemon 不在时 queue 失败 | PC 探测 daemon 前置条件（含 APP 关闭场景）；必要时 MAM 引导 `app-server daemon bootstrap` |
| 6 | 无头 turn 与 APP 内同会话操作互踩（写冲突） | PZ/PC 并发探测；按会话串行 + 冲突时排队回执如实告知 |
| 7 | macOS 侧行为差异（路径/刷新机制/端点拓扑） | Mac 段对齐探测；差异如实登记，不假设与 Windows 一致 |
| 8 | APP 未来版本通道漂移（codex queue experimental、codebuddy serve Beta） | 版本门控探针 + 失败分阶段原因 + 官方 issue 跟踪清单（#28259/#47193/#25914/#33556） |

## 11. 证据台账

| 项 | 等级 | 依据 / 验证点 |
|---|---|---|
| zcode 无头通道 | A- | CLI surface 本机实证（0.16.9 `--help` 全录）；判定 F 有 0.16.5 spike + 用户实机；全链与并发 = PZ 探测 |
| codex queue 通道 | B+ | 本机 0.156.1 子命令在场 + 官方 PR #39092 机制（daemon thread/queue/add，桌面同机制）+ issue 佐证；触达性 = PC 探测 |
| codex exec resume 兜底 | A- | issue #28259 实证（rollout 追加）；0.155.1+ 单写者锁后行为 = PC 探测 |
| codebuddy 独立 CLI / serve API | B | 官方文档（npm 包、--serve、jobs/reply、--resume -p、CODEBUDDY_CONFIG_DIR 与 WorkBuddy 共存说明）；本机实测 = PW 探测 |
| WorkBuddy 端点即 codebuddy serve | C+ | 心跳 url/endpoint 字段 + cwd host-cli + 官方「WorkBuddy 使用 CodeBuddy 引擎」；API 面 = PW 探测 |
| OpenClaw / dsh 写通道 | D | 无本机证据；PL 轻探测出「有/无」一格结论 |
| claude/kimi/opencode 无头 | B/C | 官方 flag 面（二期 spec W7 证据台账沿用）；并入与否 = 裁决门 |
| macOS 三通道注入 | A-（代码）| 引擎 macOS 执行层在产；实机终验 = Mac 段 ② |

## 12. 宪法与既有 spec 对齐声明

- 本批 = 二期 spec W7（M11）的**提前与扩容**，F2.3（zcode 无头）/ F2.4（codex 无头·app-server 候选）既有裁决全部沿用，无新宪法冲突。
- 裁决 5（三期项维持）与裁决 7（探测后裁范围）均不触碰 `docs/MASTER-PLAN.md`；若探测后需要修订二期 spec 里程碑表，按活账惯例在 `2026-09-18-phase2-message-injection-design.md` 追记版本行（届时随探测报告一并提请用户确认）。
- 本 spec 落地后的 README 矩阵更新如实呈现工具差异（不假装统一）。
