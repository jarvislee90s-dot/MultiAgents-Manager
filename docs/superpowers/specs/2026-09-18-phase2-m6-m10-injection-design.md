# 二期批次一 · 消息注入主线（M6–M10）设计 v1

> 上位文档：`docs/MASTER-PLAN.md`（宪法，D1–D18）→ `docs/superpowers/specs/2026-09-18-phase2-message-injection-design.md`（二期总 spec **v1.1**）。冲突时以宪法为准，其次二期 spec。
> 版本：2026-09-18 v1（本期裁决 1–10 适用；批次编排裁决：M6–M10 一体设计、一个实施计划、一个 PR）。
> 基线：PR #68 tip（含 M4 全量 + m4-tunnel-ux + m5-pin/m5-files；远程基座=访问密码制）——分支策略（续 #68 或待其合并后开新分支）在实施计划期定。
> 状态：待用户确认后进入实施计划（writing-plans）。

---

## 1. 背景与批次范围

一期「收消息」链路已闭环（M0–M5：八工具只读监控 → SSE 提醒 → PIN 配对 → 隧道 → 保活 → 托盘 + 访问密码制与文件池增强，PR #68 待合并）。二期总 spec v1.1 已确立 W1–W12 与 M6–M13 全景。

**本批次 = M6–M10（交接导出之前），一句话：手机发消息 → agent 真实收到并处理（终端注入 + 无头双通道），红卡可应答，Windows 同口径。**

| 里程碑 | 交付 | 依赖 |
|---|---|---|
| M6 | Windows 注入探测（报告+评审） | 无（与 M7 并行启动，**只 gate M10**） |
| M7 | macOS 终端注入全链（引擎/队列/路由/移动端/审计） | 一期底座 |
| M8 | 审批应答（统一选项卡 + TUI 按键路径） | M7 注入引擎 |
| M9 | 无头通道五家（含 claude 审批双向 + watchdog） | M7 发送端点/路由；与 M8 部分解耦 |
| M10 | Windows 注入实现 | M6 结论 GO/PARTIAL + M7 同口径组件 |

**不在本批次（边界）**：W9 交接导出（M11，含类型×进度模板体系）、W10 应急预案文档（M11）、W11 推送网关（M12）、W12 APK/manualChunks（M13，D15 触发条件制）；切模型结构化 UI、会话管理、ImportService（三期）；keystroke 正式功能（D7）。

## 2. 总体架构

```
手机会话详情页输入框（多行；回车换行、按钮发送）
   │ POST /m/api/session-send / session-approve（PIN 设备 cookie 过闸）
   ▼
注入路由表（宿主形态 → 通道 + 可见性预期）
   ├─ injector/（终端注入引擎）
   │    ├─ macOS：tmux send-keys -l / iTerm2 write text / Terminal do script
   │    └─ Windows：M6 探测定案（M10 实现）
   ├─ headless/（无头通道，AionCore 骨架）
   │    └─ zcode / claude / codex / kimi / opencode 五家 Spawner
   └─ 不可注入 → 输入框禁用 + 原因
   ▼
queue/（状态门控：黄入队 → 可输入态逐条 flush → 单会话串行）
   ▼
SQLite：写审计表（只增）+ 队列表（持久化）    SSE/轮询：回执与队列状态
```

**新增落位（实现细节以实施计划为准）**：`src-tauri/src/inject/`（引擎+队列+路由）、`src-tauri/src/headless/`（Spawner/注册表/watchdog）、DB 迁移（队列表+审计表）、`src/mobile/` 详情页输入区、设置页审计查看入口。**复用零新增**：PIN 门禁与设备指纹（M5）、SessionWatcher 跃迁（队列触发源）、`window/` tty/pane 定位（跳转同源）、adapter 宿主形态判定（路由输入）、SSE 与轮询降级。

**跨模块红线继承**：`store.with` 持 DB 锁不可重入（M4 实锤教训）；会话扫描预算契约不受影响（headless turn 产生的消息由既有读链路自然看见，不新增扫描路径）；单实例插件互斥与 co-tenant 端口隔离（E2E 期沿用 M4 经验）。

## 3. 任务分解（T1–T5 ↔ M6–M10）

### T1 · Windows 注入探测（M6，不占主线关键路径）

- **目标**：Windows 终端会话的**外部写入路径**定案——GO / PARTIAL（缩小范围）/ NO-GO（降级登记）。
- **候选路径**（探测对象：Windows Terminal 与经典 conhost 两类宿主；目标 CLI：claude / codex / kimi）：
  1. **ConPTY 附加写入**：`AttachConsole(pid)` + `WriteConsoleInput`（输入记录写入目标控制台输入缓冲区）——前置实证：jump marker 三轮验收已证 AttachConsole(pid) 对 claude ConPTY 附加成功（issue #43），本探测补「写入后 TUI 是否真实收到」；
  2. **Windows Terminal 专用通道**：WT 无官方脚本 API，探测 ConPTY 输入缓冲区在 WT 宿主下的可达性差异；
  3. **SendInput 全局键击**：仅登记为应急参照（等价 macOS keystroke，D7 定位，不作为正式路径）。
- **探测矩阵**：宿主（WT/conhost）× 目标 CLI × 行为（单行+回车、`\n` 归一长串、中文/emoji、`[mobile]` 前缀、非聚焦窗口写入）。
- **交付**：报告入 `research/refs/phase2-消息注入/`（沿一期 M0 模式）+ 独立评审落盘。
- **排期**：与 T2 并行启动；结论仅 gate T5。

### T2 · macOS 终端注入全链（M7，首个可验收版本）

- **注入引擎**：三通道（tmux `send-keys -t <pane> -l --` + Enter；iTerm2 `write text`（newline NO 先文本后回车两步）；Terminal.app `do script`（转义反斜杠/引号））；**多行 `\n` 归一纯函数**（换行符→字面 `\n`，单测锁定）；`[mobile <设备花名>] ` 前缀；定位复用 `window/`（tty/pane），定位失败=明确失败回执不入队。
- **状态门控队列**：SQLite 队列表（设备/时间戳/内容）；黄入队，**可输入态**（红·等待 / 绿·完成空闲）逐条 flush（SessionWatcher 跃迁驱动）；一次一条、等下一可输入态=单会话串行；红·中断挂起+移动端明示；可查可撤回；持久化重启不丢。**队列仅作用终端注入通道**（无头按 turn 串行）。
- **路由表**：宿主形态→通道决策+可见性预期（实时/刷新后/不可见）返回移动端；不可注入（WorkBuddy/dsh 待评/APP 形态）输入框禁用+原因；已开 TUI 优先终端注入（无头对已开 TUI 会分叉不可见）。
- **移动端**：详情页输入框（多行，回车换行、按钮发送）+ 队列状态指示（排队第 N 条/已送达/失败重试）+ 回执语义（已送达终端=注入动作成功；agent 处理由状态流体现）；斜杠命令原样放行。
- **写审计**：表只增不改（设备/时间/会话/通道/动作/摘要/结果）；设置页查看入口。
- **验收**：三通道真实会话各实测（手机→终端出现 `[mobile]` 输入）；黄→可输入态 flush 实测；审计逐条可查；不可注入卡禁用；一期零回归（全量门禁）。

### T3 · 审批应答（M8，统一选项卡 + TUI 按键路径）

- **统一 UI**：红卡结构化选项卡（批准/拒绝/选项 N）——通道无关的移动端交互件。
- **TUI 路径**：Claude/Codex 按键映射**探测入库**（提示形态→键位）+ **版本探针**（CLI 版本变化提示复核）；映射缺失降级为普通发消息（不发错误键位）；选择→注入按键（复用 T2 引擎）。
- **审计**：action=approve/reject 与内容摘要入审计表。
- **claude 无头双向**：`--permission-prompt-tool stdio` 的 `control_request{can_use_tool}` → 选项卡 → `control_response`（AionCore 实证机制）——**随 T4 无头 spawn 落地**（本任务预留选项卡→控制消息的分派接口）。
- **验收**：Claude/Codex 红卡一键批准/拒绝→终端按键生效（实机）；降级路径可用；审计可查。

### T4 · 无头通道五家（M9）

- **进程骨架（AionCore 移植）**：Spawner trait、进程注册表、空闲挂起+重生（WakeRecipe：claude `--resume` 等）、版本门控、`process_group` + `kill_on_drop`；一会话一进程、单会话同时至多一个 turn。
- **五家通道与回执**：
  - `zcode --resume <id> --prompt <text> --cwd <项目> --json`——**在册工作区校验**（D6 判定 F），桌面刷新后可见；
  - `claude -p --resume <session> --output-format stream-json` + `--permission-prompt-tool stdio`（审批双向，接 T3 选项卡）；
  - `codex exec resume <id>`——审批策略驱动（spawn 给定 policy，turn 不因审批阻塞；拒绝事件回流回执）；
  - `kimi -p -S <id>` / `opencode run`——回执与审批行为**首任务逐家活体探测**（未证项）；
  - 回执统一：turn 完成=末条 assistant 消息 + token 用量。
- **watchdog**：turn 超时（默认可配置）自动回收进程+超时回执——与「双向应答 / 策略驱动」共同保证**无头永不静默卡死**。
- **验收**：ZCode 在册无头发消息→桌面刷新后可见（判定 F 复现）；五家回执齐；claude 审批双向实机；挂起/重生可用。

### T5 · Windows 注入实现（M10，条件执行）

- **前置**：T1 结论 GO（全量）或 PARTIAL（缩小宿主范围）。
- **实现**：按探测定案路径接入 `injector/`（与 macOS 同口径：`[mobile]` 前缀、`\n` 归一、队列、审计、回执、路由表 Windows 宿主分支）；Windows 宿主形态判定接入路由表。
- **降级**：NO-GO → 本任务降级登记为已知限制（二期 spec 附录 A 标注），批次其余照常交付，回报用户裁决是否另立替代方案。
- **验收**：Windows 实机（mam-win 场景）手机发消息→终端出现 `[mobile]` 输入；真机蜂窝复验；与 macOS 同口径的队列/审计行为。

## 4. 批次执行策略（一个批次尽量多做）

```
T1 探测 ────────────────┐（只 gate T5）
T2 macOS 注入全链 → T3 审批应答 → T4 无头五家 → T5 Windows 实现
```

- T1 与 T2 **并行启动**（探测在 Windows 实机进行，不占 macOS 主线人力/关键路径）；
- T5 排批次末尾：届时 T1 结论已定、T2–T4 的同口径组件（归一/队列/审计/回执）已沉淀，Windows 只做通道适配；
- 一份设计文档（本文）+ 一个实施计划 + 一个 PR；任一里程碑探测/实测不通过即走登记的降级路径，不阻塞批次内无依赖项。

## 5. 安全与横切

1. PIN 即信任（裁决 5）：写端点全部位于设备门禁后，无额外授权层；
2. 纯文本语义：不拼 shell 元字符；`-l` 字面量；注入内容不做事前过滤（裁决 4，审计留痕）；
3. 单会话串行 = 天然限速；写审计只增不改全覆盖；
4. 版本门控+探针+降级（宪法横切 6）：TUI 按键映射、无头 wire 行为、Windows 通道全部带版本登记（research/README 跟踪清单同步）；
5. watchdog 超时兜底：无头永不静默卡死；
6. 桌面体验不回退（横切 5）：注入/无头不触碰 monitor 扫描路径与预算契约；全量门禁六件套 + 一期回归 E2E。

## 6. 验收与证据

**批次总验收**（= 二期验收中交接导出之前的全部条目）：手机对 tmux/iTerm2/Terminal 会话发消息→`[mobile]` 标记入终端；黄排队、可输入态 flush；ZCode 在册无头发消息→桌面刷新可见（判定 F 复现）；Windows 终端注入同口径实机复验（T5 GO 时）。每任务另有上节所列专项出口标准。

**证据基线**（详见二期 spec §9）：三通道注入 A-（本机验证在库）；claude 无头双向 A-（AionCore 实证）；codex exec resume B（官方 #28259）；ZCode 无头 A（spike 判定 F，0.16.5）；kimi/opencode C（M9 首任务实测）；Windows 写入路径 A-→待证（M6）；TUI 按键映射 D（M8 探测入库）。

## 7. 风险与已知限制（预登记）

1. **wire 协议漂移**（claude stream-json / codex / kimi / opencode 非公开稳定 API）：版本门控+逐版本探针；破坏性变更时单家降级不影响其余；
2. **codex 无头索引滞后**：桌面列表不保证刷新（官方已知），详情/搜索可达即算可见——明示给用户；
3. **多行 `\n` 展示限制**：桌面终端与 MAM 详情页不换行展示（裁决 6 已接受，后续可升级）；
4. **Windows Terminal 探测不确定性**：WT 无官方脚本 API，ConPTY 可达性差异可能导致 PARTIAL/NO-GO——降级路径已定（T5 节）；
5. **E2E co-tenant 干扰**：探测与验收用临时端口隔离（M4 经验），不触碰用户自有隧道与实例；
6. **审批映射探测成本**：每 CLI 一次入库 + 版本探针维护，首批只做 Claude/Codex，其余跟进。

## 8. 与宪法 / 二期 spec 对照

| 宪法功能点 | 二期 spec | 本设计 | 里程碑 |
|---|---|---|---|
| F2.1 终端注入引擎（含 D18 Windows） | W1/W8 | T2（macOS）/ T1+T5（Windows） | M6/M7/M10 |
| F2.2 状态门控队列 | W2 | T2 队列节 | M7 |
| F2.3 ZCode 无头 / F2.4 通用无头 | W7 | T4 | M9 |
| F2.5 注入路由表 | W3 | T2 路由节 | M7 |
| F2.6 移动端发消息 | W4 | T2 移动端节 | M7 |
| F2.9 审批应答（D12 第二层） | W6 | T3（TUI+统一卡）/ T4（claude 双向+策略+watchdog） | M8/M9 |
| F2.10 写操作审计 | W5 | T2 审计节（T3/T4 复用） | M7–M9 |
| —（F2.7 导出 / F2.8 应急预案 / D17 移入项） | W9–W12 | **不在本批次** | M11–M13 |
