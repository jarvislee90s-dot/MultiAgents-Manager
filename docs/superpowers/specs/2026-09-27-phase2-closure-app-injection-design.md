# 二期收尾总设计 · APP 类注入 + 无头通道 + 交接导出 v2（需求说明书）

> 日期：2026-09-27 · 状态：**总设计定稿待审**（探测批次计划与实现批次计划均另立文档）
> 基线：`origin/main` `78bf114`（v0.5.0-beta.1；M6–M9 注入主线 + M6R–M9R 加固已交付）；工作分支 `feat/phase2-closure-app-injection`
> 上位文档：`docs/MASTER-PLAN.md`（宪法）；`docs/superpowers/specs/2026-09-18-phase2-message-injection-design.md`（二期注入总 spec，本批 = 其 W7/M11 提前扩容 + W9/M10 落地 + 收尾编排；本 spec 功能点编号 H 系，与旧 spec W 系映射见附录 C）
> 文档关系（用户 2026-09-27 裁决）：**本 spec（总设计）→ 探测阶段计划（单独立）→ 探测结果回传 → 修订本 spec → 实现批次计划（单独立）**

## 1. 目标与范围

**目标**：把「手机发消息」从四家终端 TUI 扩展到 APP 形态（ZCode / Codex APP / WorkBuddy），建成无头通道公共底座（含生命周期控制与 zcode 无头新建会话），落地 M10 交接导出（含交接配置方式），补 M7 macOS 终验，使二期进入可收官状态。

**范围**：上述全部 + OpenClaw/dsh 写通道轻探测 + （裁决门定）claude/kimi/opencode 三家 CLI 无头。

**非范围（不做清单）**：
- 常驻连接池、zcode/codex app-server attach（三期 F3.3；codex 单写者锁 open issue #47193 跟踪）
- 切换模型选择器 UI、原生 APP 壳（三期 F3.1 / F3.9；本批仅探测取证斜杠命令等效性）
- 用户自定义交接模板（三期后，裁决 10；本期只做预设矩阵）
- OpenClaw gateway 深做 / dsh 插件生态展开（宪法 D14 另评口径不变）
- 任意命令/参数拼接暴露（宪法红线；仅固定 CLI 命令形态）
- 四家 CLI 的远程新建会话（独立轨道 `probe/remote-session-create`，本批只做 zcode 无头新建——该轨道明确排除的 APP 形态）
- 推送网关 / APK 壳（三期收尾，裁决 16）

## 2. 裁决记录（2026-09-27 用户对齐，实施不得重议）

| # | 裁决 | 内容 |
|---|---|---|
| 1 | 工具范围 | 核心三家 **zcode / codex APP / WorkBuddy** 全链；OpenClaw / dsh 各半天级轻探测（只判「有没有写通道」） |
| 2 | zcode 验收线 | **判定 F 基线**（手机实时回执 + APP 重启/刷新后可见）；`--surface desktop` 等免费升级项探测到即升 |
| 3 | codex APP 通道 | app-server 协议优先 → 落地为 **`codex queue` 主通道**（app-server `thread/queue/add` 的 CLI 形态）+ `exec resume` 兜底；直接 attach 桌面线程不可行（单写者锁，#47193/#25914/#33556 跟踪） |
| 4 | WorkBuddy 路线 | 以官方调研定路线（已回）：**端点 API（路线 A）/ 独立 codebuddy CLI 复刻（路线 B）/ 如实登记（路线 C）**，PW 探测后回填定案 |
| 5 | 三期项 | 切换模型 / 原生 APP 维持三期；本批仅加「斜杠命令经无头通道等效性」探测取证。**宪法零改动** |
| 6 | Mac 段 | 跑 ① 本批 APP 类探测 + ② M7 终验补课；③ session-create Mac 段已在跑不重复，仅交叉验证 1–2 核心格；报告全文回传 |
| 7 | 实现范围 | **探测完再裁**：M11 三家 CLI 无头是否并入实现批，等探测报告回传后用户裁决 |
| 8 | 无头架构 | **方案 A · 一次性进程 per turn**：每条消息 spawn 一次无头命令，回执归一后进程退出；不采纳 AionCore 空闲挂起/重生；claude 审批双向（`--permission-prompt-tool stdio`）为「进程存活至 turn 结束」特例；常驻池留三期 |
| 9 | 无头开启/关闭语义 | = **进程生命周期控制**（turn 结束即退、watchdog、移动端可中止）+ **zcode 无头新建会话**；**不做**每工具分开关的设置页多级开关 |
| 10 | 无头通道默认态 | **默认关，显式开启**（设置页远程区单一总开关；理由：无头 = 绕过终端可视确认直接驱动 Agent，风险高于终端注入，zcode `--mode` 审批档需知情选择） |

## 3. 现状与证据基线（三条新证据，2026-09-27）

1. **zcode**：APP 内嵌 CLI（`resources/glm/zcode.cjs`，本机 0.16.9）无头面本机实证——`--prompt` / `--resume sess_xxx` / `--cwd` / `--json` / `--attach` / `--mode build|edit|plan|yolo` / `--surface terminal|desktop`；旧探测（3.11.2）证 zcode:// 深链无会话级路由；用户实机 + 0.16.5 spike = 判定 F（APP 重启后可见）。运行形态：`ELECTRON_RUN_AS_NODE=1 ZCode.exe <安装目录>/resources/glm/zcode.cjs …`（Windows）。
2. **codex**：官方 2026-08 新增 `codex queue --thread <UUID或会话名> --message <文本>`（本机 0.156.1 在场，含 `--image`/`--model`），走共享 daemon `thread/queue/add`——**与桌面 APP 自身 follow-up 排队同机制**，线程发现含桌面（Atlas/ChatGPT）会话（openai/codex PR #39092）；`codex exec resume` 追加 APP 会话 rollout 被 issue #28259 实证（索引刷新不保证）；0.155.1+ 引入会话单写者锁（#46652/#47193）。
3. **WorkBuddy**：会话心跳（`~/.workbuddy/sessions/<PID>.json`）暴露每会话 `url/endpoint`（如 `http://127.0.0.1:35923`），内嵌运行时 `cli/bin/codebuddy`（cwd `workbuddy-host-cli`）；官方调研：CodeBuddy CLI 为独立一等产品（npm `@tencent-ai/codebuddy-code`），带 `-p/--print` 无头、`--resume`、`--serve` HTTP API（`POST /jobs/:id/reply` 向运行中任务追加消息、`/api/openapi.json` 规范）、ACP、SDK；官方文档明示 WorkBuddy = 「使用 CodeBuddy 引擎的应用」且 `CODEBUDDY_CONFIG_DIR` 隔离共存。

## 4. 写通道总体架构

三族通道 + 一套底座：

```
移动端发消息（/m/api/v1/session-send，PIN+gate 复用）
        │
   路由表（W3 扩展：会话宿主形态 × 工具 → 通道 + 可见性预期）
        │
 ┌──────┼──────────────┬─────────────────┐
 │ 终端注入（在产）    │ spawn 型无头（新）│ HTTP 型无头（新，WorkBuddy 路线 A）
 │ claude/codex TUI/   │ zcode.cjs --prompt│ 心跳 url + token →
 │ kimi/opencode TUI   │ codex queue/exec  │ POST /jobs/:id/reply
 │                     │ claude -p / kimi  │ （待 PW 探测定案）
 │                     │ -p / opencode run │
 │                     │ codebuddy -p（B） │
 └──────┴──────────────┴─────────────────┘
        │
 通道适配层：统一回执结构 {status, sessionId, lastAssistant, tokens, durationMs, stage?, reason?}
        │
 公共底座：无头总开关（默认关）· 按会话串行 · watchdog · 取消 · 版本门控 · 审计(action=headless)
```

- **spawn 型**：spawn 命令 → 流式读 stdout（`--json`/stream-json）→ 回执 → 进程退出；
- **HTTP 型**：端点调用 → 响应归一（同一回执结构）；
- **终端注入型**：在产四家不动，与无头互不路由到同一会话（W3 既有裁决）。

## 5. 第一部分 · APP 类注入与无头通道（H1–H10）

### H1 · zcode 无头发消息（在册会话）

- **需求**：手机对 ZCode APP 的**在册会话**发消息，无头执行一个 turn，手机实时收回执；APP 侧按判定 F 可见（刷新/重启后），不谎报。
- **输入**：session_id（`sess_...`）+ 消息文本 + 设备花名（gate 过闸时记录）；会话所属项目目录（读链路已有）。
- **输出与效果**：版本门控探针通过后，spawn
  `ELECTRON_RUN_AS_NODE=1 <ZCode.exe> <安装目录>/resources/glm/zcode.cjs --prompt "<文本> [mobile <花名>]" --resume <sess_id> --cwd <项目> --mode <档位> --json`
  → stdout JSON 解析 → 归一回执（末条 assistant 摘要 + token 用量 + 耗时 + sessionId）→ 移动端回执卡；回执附可见性提示文案「ZCode 应用重启/刷新后可见」（若 PZ 探测升级到实时/刷新即更新文案与路由元数据）。消息落库读链路自然上板（手机/桌面 MAM 可见）。
- **边界**：**在册工作区限定**（F2.3/D6——工作区不在册 → 明确回执拒绝，不盲发）；`--mode` 默认收紧档（PZ 定案具体档；yolo 需用户显式选择）；同会话同时至多一个无头 turn（进行中 → 排队回执）；APP 正在同会话跑 turn 时的行为（排队 / 如实报冲突）按 PZ 并发探测结论定案；多行 `\n` 归一与 W4 同口径；斜杠命令放行语义与终端注入一致（PZ 取证）。

### H2 · codex APP 托管会话发消息

- **需求**：手机对 **Codex 桌面 APP 托管**的会话发消息；CLI TUI 托管的会话维持既有终端注入不动（双形态分派）。
- **输入**：session_id + 文本 + 花名；MAM session id ↔ codex thread id 映射（adapter 层定案，PC 探测回填映射规则）。
- **输出与效果**：主通道 `codex queue --thread <thread_id> --message "<文本> [mobile <花名>]"`——投递回执 = queue 命令退出码与输出（投递成功信号）；**被消费与执行结果信号**按 PC 探测定案（候选：会话文件追加侦测 / queue 输出流 / 心跳）；可见性 = APP 原生排队（预期实时，PC 确认）。daemon 不在场 → 探测定案处置（引导开启 / MAM 侧 `app-server daemon bootstrap` / 如实报错）。兜底通道 `codex exec resume <id> - <文本>`（PC 实测 0.156.1 单写者锁后行为；索引滞后如实提示）。
- **边界**：不 attach 桌面线程（单写者锁，跟踪 #47193）；APP 内审批 UI 是 codex 自己的（本批不做 APP 会话的审批应答——W6 的 TUI 路径与无头策略路径都不适用于 queue 通道，如实标注）；queue 通道 experimental 状态入版本门控。

### H3 · WorkBuddy 注入（三路线，PW 探测后定案一条）

- **需求**：手机对 WorkBuddy 会话发消息；通道存续性以探测为准，探不通就如实登记（工具差异如实呈现原则）。
- **输入**：session_id（心跳 `interactive-<PID>` → codebuddy 会话 id 的映射在 PW 探测定案）+ 文本 + 花名。
- **输出与效果**（三路线）：
  - **路线 A · 内嵌端点**：定位内嵌 codebuddy 配置目录与 serve 端点 token → 调用定案端点（候选 `POST /jobs/:id/reply` 或 openapi 揭示的等价物）投递 → HTTP 响应 + 会话文件追加佐证回执；可见性 = APP 内实时（若端点即宿主运行时）。
  - **路线 B · 独立 CLI 复刻**：npm 独立 `codebuddy`（2.158）指向 WorkBuddy 配置目录，`codebuddy -p --resume <id> "<文本>"` 写入 WB 会话存储；可见性按 PW 实测（预期类判定 F：APP 刷新/重启后）；版本偏差（内嵌 2.115）入版本门控。
  - **路线 C · 登记**：A/B 均不可行 → README 矩阵与移动端如实标注不可注入 + 依据留档。
- **边界**：只读 WorkBuddy 配置目录定位信息 + 调用端点，**不修改 WorkBuddy 安装本体**；不逆向破解鉴权（token 找不到 = 路线 A 不成立）；WorkBuddy 官方 IM 远程助理通道（微信/企微）不在 MAM 集成范围。

### H4 · OpenClaw / dsh 写通道轻探测

- **需求**：各半天级探测，只出**一格结论**：「存在可集成写通道（是/否）+ 依据 + 集成形态草图」。
- **输入**：OpenClaw sessions CLI / gateway API 面（本地 gateway 在场实测）；dsh web 宿主 HTTP API / 插件生态线索（自有部署实测）。
- **输出与效果**：结论入裁决门议程；「是」→ 追加为后续批次功能点（届时补需求节），「否」→ 延后表登记复核时机。
- **边界**：不展开实现；dsh 宪法 D14 另评口径不变。

### H5 · 无头通道总开关

- **需求**：无头注入（H1–H3、H10）**默认关闭，显式开启**（裁决 10）；用户知情后才暴露该安全面。
- **输入**：设置页远程区「无头注入」开关（**单一总开关**，裁决 9 不做每工具分开关）。
- **输出与效果**：开关状态持久化 settings 表并随 `remote_status` 下发；关闭态下移动端对无头路由会话的发送入口**置灰 + 标因**（「无头通道未开启，请在电脑端 MAM 设置中开启」）；开启动作弹一次性安全说明（无头 = 绕过终端可视确认直接驱动 Agent；zcode 审批档位选择）；开关翻转写审计。
- **边界**：不影响终端注入四家（不经此开关）；远程总开关关闭时无头入口自然一并不可达（gate 在前）。

### H6 · 无头进程生命周期控制（裁决 9 用户点名功能点）

- **需求**：无头进程的开启（spawn）与关闭（回收）全程**受控、可见、可中止**；turn 生命周期 = 进程生命周期（裁决 8）。
- **输出与效果**：
  - **自然路径**：spawn → 流式回执 → 进程自然退出 → 回执终态；
  - **超时**：watchdog（默认值 PZ/PC 探测回填，设置可配）到点 kill **进程树**（Windows `taskkill /T`，避免孤儿子进程）+ 分阶段失败回执（stage=timeout）+ 可重试标注；
  - **用户中止**：移动端回执卡「取消」按钮（turn 进行中可见）→ 主动 kill 进程树 + 审计 `action=headless_cancel` + 回执终态「已取消」；与 watchdog 互斥（先到者生效，回执注明由谁终止）；
  - **崩溃**：非零退出 → 回执含退出码 + stderr 尾行；**不自动重试**（重发是用户动作，回执卡一键重发按钮）；连续崩溃 N 次后的通道熔断提示（N 探测批建议）；
  - **MAM 退出/重启**：在飞无头进程优雅关闭（各工具对 kill 的落盘行为 = 探测项——重点 zcode SQLite 半 turn 落盘、codex queue 幂等性）；MAM 重启后无孤儿进程残留自检；
  - **全局并发上限**：同时无头 turn 数上限（默认 2，可配——防手机端连点多会话打爆机器；超额请求即时排队回执含全局队列位置）。
  - **配置落点**：watchdog 超时与并发上限随 H5 开关同住设置页远程区「无头」子区（开关 + 超时 + 并发上限三件，不另开页面）。
- **边界**：不采纳空闲挂起/崩溃重生（AionCore 骨架中被裁决 8 排除的两项）；取消粒度 = turn 级（不提供「暂停」）。

### H7 · 无头审批与权限档

- **需求**：无头 turn 不因审批静默卡死（W6 既有三机制必居其一承诺沿用）；权限档位用户知情可控。
- **输出与效果**：**claude** = `--permission-prompt-tool stdio` 双向（control_request → 移动端审批卡 → control_response；进程存活至 turn 结束——裁决 8 特例）；**codex** = 策略驱动（spawn 时给定 approval policy，turn 不阻塞；拒绝/需批准事件回流回执，可调策略重试）；**zcode** = `--mode` 档位（build/edit/plan/yolo ↔ MAM 展示名映射，PZ 定案；默认收紧档，yolo 显式）；**watchdog 兜底**覆盖全部 spawn 型通道；queue 通道（H2）的投递命令本身即短命进程，超时保护仅覆盖投递阶段，turn 执行阶段归 codex APP 自身（如实标注，不承诺三机制）。
- **边界**：codex queue 通道无审批面（H2 边界重申）；本批不做权限档的移动端主动切换（三期 F3.1），仅 spawn 档位选择 + 展示。

### H8 · zcode 无头新建会话（裁决 9 用户点名功能点）

- **需求**：手机上直接**无头创建 zcode 新会话并注入首句**，APP 可见后接着用——补上「远程新建会话」轨道明确排除的 APP 形态一格。
- **输入**：项目路径（候选列表 = 在册工作区〔zcode recentProjects 口径，F3.8 前置感知〕∪ 看板快照项目 ∪ 手填完整路径——手填校验/黑名单/递归建目录复用 session-create spec §2 同款规则）+ 首句（可选，默认探针 `hi`——会话物化条件 PZ 复核）。
- **输出与效果**：spawn
  `ELECTRON_RUN_AS_NODE=1 <ZCode.exe> <…>/zcode.cjs --prompt "<首句> [mobile <花名>]" --cwd <项目> [--surface desktop] --mode <档> --json`
  （`--surface` 与新会话在 APP 任务列表出现条件的关系 = PZ 探测定案）→ 新 `sess_id` 回执 → 会话经读链路自然上板 → 可见性提示（判定 F 或升级态）；同项目已有活跃 zcode 会话 → 黄字提示（复用配对不确定门信号，不拦截）。**移动端入口**：新建表单的工具选择器新增 zcode 分组（本批独立交付；与四家 CLI 新建入口的 UI 融合留 session-create Phase C）。
- **边界**：仅 zcode（四家 CLI 新建会话在 session-create 轨道，Phase C 编码批时并入同一移动端入口 UI 但通道独立）；路径黑名单同源文件预览黑名单（session-create 口径）；不做无头「删除/归档」会话。

### H9 · 回执、审计与版本门控（横切）

- **需求**：无头写动作与终端注入同标准的回执诚实性、审计可查性、版本漂移防御。
- **输出与效果**：
  - **回执归一**：`{status: ok|queued|failed|cancelled, sessionId, lastAssistant(截断摘要), tokens?, durationMs, stage?, reason?}`；stage ∈ spawn/version_gate/timeout/crash/channel_error/dialog；移动端按 stage 分診文案；
  - **审计**：`action=headless`（及 `headless_cancel`）逐条入写审计表：设备、会话、通道、命令形态、内容摘要（对齐 W5 口径）、回执终态、耗时；设置页审计视图同口径可查；
  - **版本门控探针**：投递前校验各工具 flag 面（zcode `--resume/--prompt/--json` 在场 / codex `queue` 子命令在场 / codebuddy `--serve` 或 `-p --resume` 在场），结果缓存 + 版本变化提示复核，不盲发。
- **边界**：回执不回传消息全文（摘要口径）；探针失败=version_gate 拒发回执。

### H10 · claude / kimi / opencode 无头（条件功能点，裁决门 ⑦ 定并入否）

- **需求**：未开窗的 CLI 会话也能被驱动（W7 原需求，本期是否实现待裁决）。
- **输出与效果**（若并入）：`claude -p --resume <session> --output-format stream-json`（+审批双向）/ `kimi -p -S <id>` / `opencode run`，全部走 H5–H9 底座；回执与可见性同口径。
- **边界**：无头对已开 TUI 的会话默认不路由（W3）；probe 报告回传后本节升级为定案节。

## 6. 第二部分 · M10 交接导出与交接配置（W9 落地 + 配置功能点）

### M10-a · 交接导出双档（W9 全量定案落地）

按总 spec W9 已定案内容实现，**不重议**：HANDOFF v1 九段式骨架恒定；分类矩阵 10 格（编码/设计/文档 × 进度环节，封闭枚举）；提示词 = 公共骨架 1 + 类型模块 3 + 进度模块 10 组件拼装；模板三级选择（用户显式 > 自动预判启发式 > 公共兜底）；**双档执行**（注入自总结默认档：模板提示词经注入通道发给会话本体 → Agent 在项目根写 `MAM-HANDOFF-<短id>-<时间戳>.md` → MAM 检测落盘 → **自动收割转移**至 `~/.mam/handoffs/<规范名>.md` + SQLite 索引 + `.json` 附本，超时兜底规则摘要；规则摘要兜底档：零依赖离线，不可注入/黑盒/APP 形态会话适用）；强制保留项（失败路径清单 + 带预期结果的验证步骤）全格恒在。
**本批新增关联**：zcode/codex APP/WorkBuddy 会话注入落地后，其「交接」自动升级为注入自总结档（原仅规则摘要）——注入通道在本批**含无头通道**（模板提示词经 H1–H3 通道投递给会话本体），H1–H3 与 M10 的验收交叉点。

### M10-b · 交接配置方式（用户点名功能点）

- **需求**：交接行为的用户可配置面集中一处，明示默认值。
- **输入**：设置页「交接」区。
- **输出与效果**：
  - `.json` 无损附本开关（**默认开**，W9 定案）；
  - 注入自总结超时时长（默认 10 分钟，可配，超时自动降规则摘要并回执）；
  - **handoffs 保留期策略**（v1.5 裁决落地项）：保留时长档位（默认 90 天 / 180 天 / 永久）+ 手动「清理过期」按钮 + 当前占用体积展示；清理动作写审计；
  - 「打开交接目录」按钮（系统文件管理器）。
- **边界**：不自定义模板/组件（裁决 10）；不动 MAM 之外的目录。

### M10-c · keystroke 应急预案文档（W10）

文档级交付（D7）：适用场景、前置条件、风险（抢焦点/前台/输入法/辅助功能权限）写入 docs；不实现、不暴露 UI。

## 7. 第三部分 · 收尾杂项

- **M7 macOS 终验补课**（Mac 段 ②）：tmux / iTerm2 / Terminal.app 三通道 × 四家注入实机（claude 必测，codex/kimi/opencode 量力）+ 黄排队 flush / 立即发送插队 / 锁屏注入可用性；产出终验清单（PASS/FAIL/不可达 + 证据）。
- **W13 B 兜底「发」半部补全**：H1–H3 落地后，「不可注入会话经无头驱动」路径自然成立，resume 窗口的边界条款（「B 兜底的发半部随 M11 补全」）在验收中关账。
- **远程新建会话 Phase C**：引用 `2026-09-27-remote-session-create-design.md`（独立轨道，Phase P 探测已在跑）；本批 H8 与其共享移动端「+ 新建会话」入口 UI 的信息架构（zcode 与四家 CLI 并列，通道各自独立），UI 融合在其 Phase C、不在本批。
- **README / CHANGELOG / 发版收尾**：功能矩阵注入列按实测定案更新（zcode/codex APP/WorkBuddy 从 ❌ 升 🧪 或如实维持）；CHANGELOG 版本化；发版流程沿用 v0.5.0-beta.1 流水线。

## 8. 里程碑与批次编排（C 系，顺序固定）

| # | 交付 | 出口标准 |
|---|---|---|
| C-P+ | 探测批（Windows 四系 PZ/PC/PW/PL + Mac 段） | 报告落盘 `research/refs/phase2-消息注入/`；裁决门三事（WorkBuddy 路线 / H10 并入否 / 各家验收线与超时定值）过用户 |
| C1 | 无头底座（H5/H6/H7/H9）+ zcode 注入（H1）+ codex APP 注入（H2） | 手机→zcode 在册会话全链（回执 + 判定 F 兑现）；codex APP queue 全链；开关默认关实测；取消/watchdog/审计逐条可查；`#[ignore]` E2E |
| C2 | WorkBuddy（H3，按裁决门定案） | 路线 A/B 全链 **或** 如实登记（含 README 更新），二选一交付 |
| C3 | H10 三家 CLI 无头（条件，按裁决门） | W7 口径：五家无头全绿 + claude 审批双向实机 |
| C4 | zcode 无头新建（H8）+ M10 交接导出（M10-a/b/c） | 无头新建全链；HANDOFF 双档实机（含 APP 会话升级注入自总结档）；交接设置区四项可用 |
| C5 | 收尾：M7 Mac 终验（若报告带回 FAIL 项则修复）+ README/CHANGELOG + 发版 | 终验清单绿；矩阵更新；版本发布 |

依赖关系：C1→C2/C3（底座先行）；C4 依赖 C1（H8 复用 zcode 通道与 M10-a 复用注入通道）；M7 终验与 C1–C4 可并行推进（Mac 段报告回传即入 C5）。

## 9. 输入输出总表

| 功能点 | 输入 | 输出 / 效果 |
|---|---|---|
| H1 zcode 无头发消息 | session_id + 文本 + 花名；在册校验 | zcode.cjs spawn 一次 turn；回执（末条 assistant/token/耗时）；判定 F 可见性提示；审计 |
| H2 codex APP 发消息 | session_id + 文本；thread id 映射 | `codex queue` 投递（exec resume 兜底）；APP 原生排队可见；投递/消费回执；daemon 处置 |
| H3 WorkBuddy 发消息 | session_id + 文本 | 路线 A 端点投递 / 路线 B 独立 CLI 复刻 / 路线 C 登记，三选一定案交付 |
| H4 OpenClaw/dsh 轻探测 | 本地 gateway / web 宿主 | 有/无写通道一格结论 + 依据 |
| H5 无头总开关 | 设置页远程区单开关 | 默认关；关闭置灰标因；开启一次性安全说明；审计 |
| H6 生命周期控制 | 回执卡取消 / watchdog / 进程退出 | turn=进程；超时 kill 树；取消审计；崩溃不自动重试；MAM 退出优雅关闭；全局并发上限 |
| H7 审批与权限档 | 无头 spawn 档位；审批事件 | claude 双向卡；策略驱动回执；zcode 档位映射（默认收紧）；watchdog 兜底 |
| H8 zcode 无头新建 | 项目路径（候选/手填）+ 首句 | 无头起会话 + 首句注入 + sess_id 回执 + 上板 + 可见性提示；黄字多实例提示 |
| H9 回执/审计/门控 | 全部无头动作 | 归一回执结构；action=headless 审计；flag 面探针 + 漂移提示 |
| H10 三家 CLI 无头（条件） | session_id + 文本 | claude/kimi/opencode 一次 turn + 回执（W7 口径） |
| M10-a 交接双档 | 会话卡「交接」+ 两级单选 | HANDOFF v1 + 收割转移 + 索引；注入自总结/规则摘要自动降档 |
| M10-b 交接配置 | 设置页交接区 | .json 附本开关（默认开）/ 自总结超时（默认 10min）/ 保留期档位 + 清理 / 打开目录 |
| M10-c 应急预案 | — | 文档级登记（D7） |

## 10. 非功能需求

- **安全**：PIN + gate + Host 双条件豁免复用（M5 在产）；无头命令固定形态无参数拼接（宪法红线）；敏感目录黑名单对 H8 手填路径生效；审计含开关状态。
- **可靠性**：无头 turn 永不静默卡死（watchdog 必在）；进程树级回收无孤儿；MAM 重启无残留自检。
- **性能**：无头 spawn 不占主线程（spawn_blocking 同款纪律）；版本探针结果缓存；全局并发上限防打爆。
- **诚实性**：可见性预期如实（判定 F 文案不谎报实时）；未投递/超时/取消如实回执；探不通的工具如实标注。
- **测试**：纯核单测（路由/回执归一/串行/门控）+ MSW 前端用例 + `#[ignore]` 实机 E2E（m9r 模式）；测试零网络、零真实用户数据目录。

## 11. 证据台账

| 项 | 等级 | 依据 / 验证点 |
|---|---|---|
| zcode 无头通道（H1/H8） | A- | CLI surface 本机实证（0.16.9 `--help` 全录）+ 判定 F（0.16.5 spike + 用户实机）；全链/并发/新建可见性 = PZ |
| codex queue（H2） | B+ | 本机 0.156.1 子命令在场 + PR #39092 机制（daemon 同源桌面排队）+ issue 佐证；触达/回执/daemon 前置 = PC |
| codex exec resume 兜底 | A- | issue #28259 实证；0.155.1+ 单写者锁后行为 = PC |
| codebuddy CLI/serve（H3） | B | 官方文档（npm/--serve/jobs:reply/--resume/CODEBUDDY_CONFIG_DIR 共存说明）；本机端点实测 = PW |
| WorkBuddy 端点=serve 假设 | C+ | 心跳 url/endpoint + cwd host-cli + 官方「WB 使用 CodeBuddy 引擎」；API 面 = PW |
| OpenClaw/dsh 写通道（H4） | D | 无本机证据；PL 出一格结论 |
| claude/kimi/opencode 无头（H10） | B/C | 官方 flag 面（旧 spec W7 台账沿用）；并入否 = 裁决门 |
| 交接导出（M10-a） | B | W9.1 决策表 12 项调研定档（四家 compact 一手核对）；组合模板 = C4 实机验证 |
| macOS 三通道（C5） | A-（代码） | 引擎 macOS 执行层在产；实机终验 = Mac 段 ② |

## 12. 风险与已知限制（预登记）

| # | 风险 / 已知限制 | 处置 |
|---|---|---|
| 1 | WorkBuddy 端点无写 API 或 token 不可寻 | 路线 B 兜底 → 路线 C 如实登记 |
| 2 | codex queue 触达桌面会话无官方保证 | PC 实测；exec resume 兜底；双不通则登记并跟踪 #47193 |
| 3 | zcode `--mode` 默认 yolo 风险 | PZ 定档；实现默认收紧，yolo 显式选择 |
| 4 | codebuddy 内嵌 2.115 vs 独立 2.158 偏差 | PW 实测；版本门控拦截不兼容组合 |
| 5 | codex daemon 前置条件（APP 关闭场景） | PC 探测 bootstrap 可行性；如实报错兜底 |
| 6 | 无头与 APP 同会话写冲突 | PZ/PC 并发探测；按会话串行 + 冲突排队回执 |
| 7 | macOS 行为差异（路径/刷新/端点拓扑） | Mac 段对齐探测；差异如实登记 |
| 8 | codex queue / codebuddy serve 标 experimental/Beta | 版本门控 + 分阶段失败回执 + issue 跟踪清单（#28259/#47193/#25914/#33556） |
| 9 | kill 进程树对半 turn 的落盘损伤 | 探测各工具 kill 行为；取消回执提示「可能未完整落盘」 |
| 10 | zcode 无头新建会话 APP 不显示（surface/scope 条件未知） | PZ 探测出现条件；不满足则 H8 降级「仅手机/桌面 MAM 可见」如实提示 |

## 13. 与宪法 / 二期 spec 对照

- 本批 = 二期 spec W7（M11）提前扩容 + W9（M10）落地 + W13 边界条款关账；F2.3/F2.4/D6/D14/D18 全部沿用，**无宪法冲突，宪法零改动**。
- 裁决 5（三期项维持）与裁决 7（探测后裁范围）不触碰 `docs/MASTER-PLAN.md`；探测报告回传后修订本 spec 定案栏 + 在 `2026-09-18-phase2-message-injection-design.md` 追记版本行（届时提请用户确认）。
- README 矩阵更新如实呈现工具差异，不假装统一。

## 附录 A · 工具 × 通道矩阵（目标态；「定案」栏探测后回填）

| 工具 | 终端注入（在产） | 无头/APP 通道（本批） | 有头可见性 | 定案状态 |
|---|---|---|---|---|
| Claude Code | ✅ | H10 `-p --resume`（条件） | 注入实时 / 无头读链路可见 | 裁决门 |
| Codex CLI（TUI 托管） | ✅ | —（不路由无头） | 实时 | ✅ 在产 |
| Codex APP 托管 | — | H2 queue / exec resume 兜底 | APP 原生排队（预期实时） | 待 PC |
| OpenCode | ✅ | H10 `run`（条件） | 官方 web 实时 | 裁决门 |
| OpenClaw | — | H4 轻探测 | — | 待 PL |
| Kimi Code | ✅ | H10 `-p -S`（条件） | 刷新后 | 裁决门 |
| WorkBuddy | — | H3 三路线 | 待 PW | 待 PW |
| ZCode | — | H1/H8 zcode.cjs 无头 | 判定 F（升级待 PZ） | 待 PZ |
| dsh | — | H4 轻探测 | — | 待 PL |

## 附录 B · 功能点进度表（随批次更新）

| 功能点 | 里程碑 | 状态 |
|---|---|---|
| H5 无头总开关 / H6 生命周期 / H7 审批档 / H9 横切 | C1 | ⬜ |
| H1 zcode 注入 / H2 codex APP 注入 | C1 | ⬜ |
| H3 WorkBuddy（定案后） | C2 | ⬜ |
| H10 三家 CLI 无头（条件） | C3 | ⬜ |
| H8 zcode 无头新建 / M10-a 双档 / M10-b 配置 / M10-c 文档 | C4 | ⬜ |
| M7 Mac 终验 / README / 发版 | C5 | ⬜ |

## 附录 C · 与旧 spec W 编号映射

| 旧 spec（2026-09-18） | 本 spec | 备注 |
|---|---|---|
| W7 无头通道五家（M11） | H1/H2/H3/H10 + H5/H6/H7/H9 底座 | 提前 + 扩容 APP 形态；空闲挂起/重生被裁决 8 排除 |
| W6 审批应答（无头半部 W6'） | H7 | 三机制承诺沿用；codex queue 通道例外如实标注 |
| W8 Windows 终端注入 | —（在产不动） | M6R–M9R 已交付 |
| W9 交接导出（M10）+ W9.1 决策表 | M10-a/b | 全量定案沿用；新增配置功能点 M10-b |
| W10 keystroke 应急预案 | M10-c | 文档级不变 |
| W13 一键 resume（B 兜底发半部） | §7 关账条款 | H1–H3 落地即补全 |
| W11 推送网关 / W12 APK | —（三期收尾） | 裁决 16 不变 |
