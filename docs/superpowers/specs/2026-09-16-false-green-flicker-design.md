# 设计文档：会话假「完成」信号（假绿）治理 — Codex CLI / WorkBuddy / OpenCode

- 日期：2026-09-16
- 状态：设计定稿（全部决策已拍板，含 OpenCode 活体取证；待出实现计划）
- 工作分支：`fix/status-false-green`（基于 main `2b86907`）
- 关联问题：用户实测报告——Codex CLI 多轮工具调用场景，每轮结束输出中间消息时被误判「任务完成」转绿灯，触发宠物语音

---

## 1. 背景与症状

用户报告：Codex CLI 执行一个由多轮工具调用组成的任务时，只要模型完成一轮工具调用并回复一条消息，MAM 就把该会话判定为「任务完成」（绿灯），触发一次宠物 "done" 语音；随后下一轮工具调用开始，又翻回黄灯。一个多轮任务会触发多次假完成语音。

### 1.1 完整触发链路（已代码级验证）

```
rollout 尾部短暂停在中间 assistant 消息
  → codex_parser.rs 判 Idle（绿灯）                    [误判源]
  → 后端 sync_unread：上一轮非绿 → 本轮绿 = Insert 未读行  [边沿触发]
  → 前端 petStatus.ts：prevColor≠green → green = completion 事件 [边沿触发]
  → FoxbellPet.tsx：newCompletion → playVoice("done")    [语音]
```

三层都是边沿触发，任何一次黄→绿翻转都会全链路生效。

### 1.2 实证数据（2026-09-16 本机 rollout）

`~/.codex/sessions/2026/09/16/rollout-2026-09-16T21-03-06-*.jsonl`，单个 task（`task_started` 13:03:33 → `task_complete` 13:04:34，61 秒）内出现 **5 条中间 assistant 消息，每条后面都还有下一轮 function_call**：

```
11  13:03:36  msg/assistant   ← 中间消息
12  13:03:37  function_call    ← 任务还在继续
...
57  13:04:11  msg/assistant   ← 中间消息
58  13:04:14  function_call    ← 中间隔了整 3 秒
...
66  13:04:34  msg/assistant   ← 真正的收尾消息
69  13:04:34  ev:task_complete
```

中间消息与下一轮 function_call 落盘间隔 0~3 秒。MAM 每 3 秒轮询，落在窗内即产生一次假绿。

## 2. 根因

共享判定核 `derive_app_status`（`src-tauri/src/monitor/app_status.rs:79`）采用**尾部倒扫**模型：取最后一条有语义条目定状态，`AssistantMessage` 在尾即判 Idle。该模型假设「assistant 纯文本 = 回合完成信号」，但 Codex（及下文 WorkBuddy）在**同一回合内**会在工具调用轮次之间写中间文本消息，该假设不成立。判定时完全不感知回合窗口——尽管 `task_started`/`task_complete` 已被 `codex_entry_kind`（`codex_parser.rs:61-64`）解析为 `TurnStart`/`TurnEnd` 放进 kinds 序列，核只把它们当「最后一条」候选，从不作为**开闭对**读取。

历史脉络：判定核诞生于 2026-09-06（commit `e14ab47`，issue #6），当时修的是「第二轮工具执行期恒 Idle」（function_call 条目被整体跳过），尾扫模型解决了永久性误判，但「中间消息短暂占据尾部」的瞬时误判是其固有残留。2026-09-10 APP 路线（commit `9285c56`）迁移 SQLite 时拿到了 Codex 自维护的轮次生命周期表 `thread_turns.status`，加了 `inProgress → Processing` 强信号守卫；CLI rollout 路线未被回移同款守卫。

## 3. 全工具横评与策略

| 工具/路线 | 判定模型 | 中间消息假绿风险 | 本轮策略 |
|---|---|---|---|
| **Codex CLI**（rollout JSONL） | 共享尾扫核 | **实锤**（§1.2 实证） | **加回合守卫**（§4.1） |
| **Codex APP**（app-server SQLite） | 尾扫核 + `inProgress` 强信号 | 已守卫（`codex_thread_parser.rs:338`） | 不动，作为语义参照 |
| **WorkBuddy**（JSONL） | 共享尾扫核，**格式无任何轮次信号** | **本机数据证实同款中间消息模式**（最新会话尾部 `msg/assistant → function_call → …` 多轮交错） | **完成防抖 10s**（§4.2） |
| **ZCode**（SQLite） | 尾扫核 + `step-finish(reason=tool-calls)`→ToolCall 守卫 | 半免疫：步骤间已守卫（`zcode_parser.rs:668-676`），残余仅部件流式落盘窗口（未证实） | 不动，残余暴露记录于 §7 |
| **Claude**（JSONL） | 消息模型：assistant 带 `tool_use` → Processing（`status.rs:127`） | 免疫（文字+tool_use 同条消息，纯文本即回合结束） | 不动 |
| **Kimi**（wire.jsonl 事件流） | 事件模型：`tool_calls` 判据 + `turn.ended` 边界（`kimi_parser.rs:429`） | 免疫 | 不动 |
| **OpenCode**（SQLite） | `last_role` + 60s 新鲜窗 + CPU（`opencode_parser.rs:363`） | 延迟衰减变体：中间消息先红 60s，模型停顿 >60s 才假绿 | **加 step 边界守卫**（§4.3，ZCode 同构，活体取证完成） |
| **OpenClaw** | 纯 CPU 启发式 | 另一类误报源（API 等待期 CPU 低） | 不动（超出本轮范围，§7 记 follow-up） |
| **dsh**（事件流 + 锁） | 轮次事实 `has_open_turn`（`dsh/status.rs`） | 免疫，正面教材 | 不动，作为 Codex 守卫的参考实现 |

方案总纲两条轨道：**有精确信号的用信号**（Codex CLI 的开闭对、OpenCode 的 tool part），**没信号的用防抖**（WorkBuddy）。

## 4. 修复设计

### 4.1 Codex CLI：回合守卫

**目标**：回合（task）仍在进行中时，中间 assistant 消息不得触发 Idle；只有回合真正结束（`task_complete` 落盘）才转绿。

**实现位置**：`app_status.rs` 新增共享纯函数 + `codex_parser.rs` 消费，`derive_app_status` 契约不变（核仍是「最后一条说了什么」，回合感知是叠加在核之上的仲裁层，WorkBuddy/ZCode 等无信号消费者不受影响）。

**已定方案 A（2026-09-16 用户拍板；方案 B 为已否决备选，见本节末尾）**：

- **方案 A（开闭对，dsh 同款，选定）**：扫描 kinds 序列，最后一个 `TurnStart` 晚于最后一个 `TurnEnd` → 回合开。仅当「derive 结果为 Idle 且尾部语义条目是 `AssistantMessage` 且回合开」时改判 Processing。
  - 超长回合（>500 行尾读窗口）时 `TurnStart` 滚出窗口 → 回合状态不可证 → 回退现状（用户已拍板：保持现状 + 文档化，见 §8 决策 2）。已知限制：长回合中途仍可能假绿（记入 §7）。
  - 既有测试 `assistant_message_tail_is_idle`（夹具无 task_started/task_complete 开闭对）在方案 A 下语义不变（窗口内无边界事件 → 不仲裁 → Idle 照旧），无适配负担——这是 A 相对 B 的实现简洁性优势。

**不参与仲裁的路径**：`UserMessage` 在尾 → Thinking 原样；`ToolCall`/`TurnStart` 在尾 → Processing 原样；`TurnEnd` 在尾 → Idle 原样。

**测试**（方案 A）：
- 真实样本夹具化（§1.2 序列）：中间消息在尾 → Processing；追加 function_call → Processing；追加收尾消息 + task_complete → Idle。
- 两轮同文件：第一轮完成后第二轮运行中 → Processing（回归 issue #6 主场景）。
- 回合正常完成 → Idle 立即（无防抖延迟）。
- 窗口内无开闭对（既有夹具形态）→ 语义不变（Idle 照旧），锁定 A 的回退行为。

**已否决备选——方案 B（不可证完成即不绿）**：`AssistantMessage` 在尾且位于窗口内最后一个 `TurnEnd` 之后（或窗口内无 `TurnEnd`）→ 保持 Processing。优点：超长回合中途也不假绿、绿灯零延迟；缺点：依赖「每回合必写 `task_complete`」这一协议假设，且反转 `assistant_message_tail_is_idle` 既有语义。用户裁决取 A（行为可预期性优先，超窗场景接受已知限制）。

### 4.2 WorkBuddy：完成防抖（10 秒）

**目标**：WorkBuddy 格式无任何轮次边界信号，无法精确判定；用时间防抖拦截瞬态假绿。

**语义**：`derive_status_from_tail` 结果为 Idle 且尾部语义条目为 `AssistantMessage` 时——文件 mtime 年龄 < `GREEN_DEBOUNCE_MS`（10_000）→ 保持 Processing；≥ 10s → Idle。其余状态（Thinking/Processing）原样。

**实现位置**：`workbuddy_parser.rs` 构卡层（现有 mtime 叠加点 `workbuddy_parser.rs:384-393` 旁），纯时间叠加，不动 L2 缓存契约（内容摘要仍为纯内容产物）。

**接口调整**：防抖需感知「Idle 是否由 assistant 尾导出」。在 `app_status.rs` 新增共享纯函数 `tail_semantic_kind(&[AppEntryKind]) -> Option<AppEntryKind>`（与 §4.1 守卫函数同文件，便于单测），`derive_status_from_tail` 内部改用它定位尾部语义条目；WorkBuddy 构卡层以「derive 结果 + 尾部语义条目」共同决定防抖。`derive_status_from_tail` 对外签名不变，波及面仅限内部实现与新增单测。

**权衡**（用户已接纳）：所有真完成绿灯/语音延迟 10 秒到达；10s 窗口内模型若停顿超 10s 再继续（重推理长停顿），仍可能漏放一次假绿——防抖是启发式，不追求全防。

**与 300s 停更叠加的顺序**：防抖在前（assistant 尾 + 新鲜 → 拉回 Processing），`overlay_mtime_stale` 在后。真完成场景 10s 后即为 Idle，不会落入 300s 降级路径。

**测试**：assistant 尾 + mtime 9.9s → Processing / 10.1s → Idle；`function_call` 尾不受防抖影响（照常 Processing）；user 尾照常 Thinking；防抖只作用于 assistant 尾导出的 Idle，不影响其他 Idle 来源。

### 4.3 OpenCode：step 边界守卫（ZCode 同构）

**目标**：消除「中间停顿 >60s 后假绿」的延迟衰减变体。

**现状**：`determine_opencode_status`（`opencode_parser.rs:363`）只看 last_role + 60s 窗 + CPU，完全不读 parts。中间消息场景：assistant 尾 + 60s 内红（Waiting）、超 60s 绿（Idle）——若模型/编排器 60s 后续跑则假绿。

**活体取证（2026-09-16，opencode v1.18.22，用户配合实测一轮工具任务）**——原「实现期前置取证」任务已消解，证据如下：

- **消息/部件结构**：每个模型 step = 一条独立 assistant 消息，部件序 `step-start → [reasoning | tool | text] → step-finish`；一轮 = 一至多个 step。
- **step-finish.reason 词汇表（本机两值全观测）**：`tool-calls`（本步因要调工具而结束，后续还有动作）/ `stop`（回合结束）。与 ZCode 解析器的映射（`zcode_parser.rs:668-676`）及 opencode issue 生态（[opencode #14972](https://github.com/anomalyco/opencode/issues/14972) 中 `finish_reason: tool_calls` 标准值）三源交叉一致。
- **tool 部件真实形状**：`{"type":"tool","tool":"bash","callID":"call_…","state":{"status":"completed","input":{"command":"…"}}}`——tool 部件会落盘，本守卫设计不依赖它（增强项）。
- **时序金证**：step1 `step-finish(tool-calls)` 落盘到 step2 `step-start` 落盘间隔 **4.3 秒**——此窗口内尾部即 `step-finish(tool-calls)`，恰是守卫要覆盖的「步骤间空窗」；step2 回答 text 落盘到 `step-finish(stop)` 间隔 5.1 秒（部件流式窗口，守卫同样覆盖）。

**设计**（与 ZCode `part_entry_kind` 完全同构）：读取会话尾部部件（查询路径已有，`opencode_parser.rs:312`），映射 `step-finish(reason)` 后判定：

- 尾部语义部件 = `step-finish(reason="tool-calls")` → 后续还有动作 → **Processing**（覆盖现 Waiting/Idle 分支）
- 尾部语义部件 = `step-finish(reason="stop")` → 回合结束 → 维持现行为（60s 窗内 Waiting / 超窗 Idle）
- 尾部无 `step-finish`（步骤进行中：`step-start`/`reasoning`/`text`/`tool` 在尾）→ **Processing**
- 增强（非必需）：尾部 `tool` 部件 `state.status` 为未完成值 → Processing；本机仅观测到 `completed`，未完成值词汇表（`running`/`pending`）实现期以防御性不等式（≠ `completed`/`error`）处理

**残余已知限制**：外部编排器 team-mode 形态（AionUi/omo "Sisyphus"，本机 `ses_fae18a2f` 等老会话实证）只落 `text`/`patch` 部件、无 step 部件 → 无 step 信号时回退现行为，编排器续跑的假绿窗口保留（记入 §7）。

**测试**（以本节取证样本为夹具）：`step-finish(tool-calls)` 在尾 → Processing；`step-finish(stop)` 在尾 → 现行为；`text` 尾无 step-finish（流式窗口）→ Processing；无 step 部件的老库会话 → 现行为降级；user 消息在尾 → 现行为（Processing/Thinking 语义不变）。

## 5. 明确不动清单

| 对象 | 理由 |
|---|---|
| Codex APP 路线 | `inProgress` 强信号已覆盖，作为 §4.1 语义参照 |
| ZCode | `step-finish(tool-calls)` 守卫已覆盖步骤间场景；流式落盘窗口未证实，不值得预防性改动 |
| Claude / Kimi | 消息内工具调用判据 + 轮次边界事件，结构免疫 |
| dsh | 轮次事实模型，正面教材 |
| OpenClaw | 纯 CPU 启发式的误报是另一族问题（无消息模型可依），单独立项 |
| 共享核 `derive_app_status` 契约 | 保持「最后一条说了什么」定位；回合感知以叠加仲裁层实现（§4.1），避免波及无信号消费者 |
| 前端（petStatus / FoxbellPet / 未读池） | 边沿触发链路本身正确，源头状态正确后无需改动 |

## 6. 测试与验收

1. 单测：§4.1/§4.2/§4.3 各自的测试清单全部落地（夹具优先取自本机真实数据脱敏）。
2. 回归：`cd src-tauri && cargo test` 全绿；`clippy -D warnings` 干净。重点回归组：`app_status` 既有判定核测试、codex `app_status_fixture_tests`（issue #6 样本）、WorkBuddy `derive_status_from_tail` 系列、OpenCode `status_tests`。
3. 手工验收：Codex CLI 跑一个 3+ 轮工具任务，全程黄灯、只在 `task_complete` 后一次绿灯一次语音；WorkBuddy 完成任务观察绿灯延迟约 10s 出现。

## 7. 已知限制与 follow-up

- **WorkBuddy 防抖漏防**：模型中间停顿 >10s 仍会漏放一次假绿（启发式上限）。
- **ZCode 流式落盘窗口**：text part 已写、step-finish 未写的瞬态窗口未证实是否存在假绿；待有实感症状再立项。
- **OpenCode 编排器续跑**：team-mode 外部编排形态（无 step 部件）无库内回合信号，假绿窗口保留。
- **OpenClaw CPU 误报族**：API 等待期 CPU 低可能假绿/假红，与本轮不同根因，单独立项。
- **方案 A 专属限制**：>500 行长回合窗口内无开闭对 → 回退现状可能假绿（用户裁决接受，§8 决策 2/5）。

## 8. 决策记录

| # | 决策点 | 结论 | 日期 |
|---|---|---|---|
| 1 | WorkBuddy 防抖思路是否接纳 | 接纳 | 2026-09-16 |
| 2 | Codex CLI 超窗（>500 行）回退策略 | 保持现状 + 文档化（方案 A 语境） | 2026-09-16 |
| 3 | WorkBuddy 防抖窗口 N | 10 秒（常量可调） | 2026-09-16 |
| 4 | OpenCode 延迟衰减变体是否本轮处理 | 本轮一并修（§4.3） | 2026-09-16 |
| 5 | Codex CLI 守卫方案 A vs B | **方案 A（开闭对，dsh 同款）**；B 记录为已否决备选 | 2026-09-16 |
