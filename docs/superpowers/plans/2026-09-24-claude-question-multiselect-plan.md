# claude 问答卡与多选/多题键序复刻（2026-09-24）

> 分支 `feat/claude-auq-multiselect`（基线 main@78bf114 / v0.5.0-beta.1）。
> 性质：修缺陷 + 功能补齐批次。宪法（`docs/MASTER-PLAN.md`）无冲突，未改动。

## 0. 背景与根因

用户报告：claude 问答题在手机端被误判成「审批单选」（红色二元卡，允许=勾第 1 项、拒绝=取消），多选题/多题无法远程作答。三层根因：

1. **Bug A（阻断）**：`~/.claude/settings.json` 每个事件下 helper 与旧 bash 兜底**双注册**——两者同触发、双写同一事件文件，bash 只写基础字段且**后写覆盖** helper 的富载荷（`tool_name`/`tool_input` 丢失）→ AUQ 识别整体失效 → 裸 `Notification` 被通用审批映射接走 → 出审批卡。注册器的 `hooks_file_verified` 只查「规格命令在文件某处出现」，helper 在场即判已核验 → **跳过注册**，bash 残留永不清理。
2. **Bug B（功能）**：claude 多选 toggle 发**数字**（K8 旧证据），用户实机（**2.1.278**，npm 2026-09-22 自 2.1.251 升级）证明多选屏数字**完全无反应**——空格=切勾、↑↓=题内移动、←→=切题（`Type something`/`Next` 两行不生效）、`Next`+回车=下一题、Submit 页 1/esc。`space` 不在注入键域。
3. **Bug C（设计）**：二元审批卡红色并排小按钮，与问答卡（蓝色纵向编号列表）不同套。

**证据纪律**：全部新键序为用户手工实机取证（单样本），档案 `research/refs/phase2-消息注入/2026-09-24-claude多选多题键序-用户实机取证.md`（含 K8 冲突三证并记、版本漂移申报、空格注入形态待复验）。实现**全部走屏读闭环**：每键发前定位、发后核验，形态不符即干净中止——单样本风险收敛为「可读的失败」而非「答错」。

## 1. 改动清单

### Stage 0 —— 根修 + 取证归档

| 文件 | 改动 |
|---|---|
| `research/refs/phase2-消息注入/2026-09-24-claude多选多题键序-用户实机取证.md` | 新增：环境（2.1.278）、屏面转抄（页签栏 `← ☒ … ✔ Submit →`、无编号 `Next` 行、勾选框字形族 `[ ]`/`[✓]`/`[✔]`）、键序定案表、K8 冲突三证并记、复验条件 |
| `2026-09-22-键序大词典.md` §1 | claude 多选表按「用户实测推翻直接删」规则改写：数字→无反应、新增 空格/↑↓/←→/Next+回车 行 + 多题表单小节；版本口径注记 |
| `README.md`（research） | 索引加行 |
| `monitor/hooks.rs` | `hooks_file_verified` 升级为 JSON 解析级「每事件我方条目**恰好一条**且等于规格」判据；`register_hooks_in_file` 新增 `prune_extra_ours_entries`（跨条目只留第一条我方命令，掏空条目删除；用户命令永不触碰）；`register_all_hooks` 调用点传脚本路径。回归锁 ×3 |

### Stage 1 —— 单题多选闭环

| 文件 | 改动 |
|---|---|
| `inject/engine.rs` | `control_records` 增 `"space" => (0x20, 0x20)`（VK_SPACE+字符，与 enter/esc/tab 同形）；裸 `" "` 仍域外 |
| `inject/windows_console.rs` | 域错误文案补 space |
| `inject/question.rs` | ① 行块解析 `parse_question_rows`（以推进行为锚、向上收编号连续递减块——天然剔除滚回区杂行；分隔线下排除；勾选框按括号内容语义判）；② `run_toggle_stages` 闭环切勾（读屏定位 → 方向感知走位每步复核 → 空格 → **屏读核验翻转**，未翻转中止）；③ `run_submit_stages` 走位方向感知（新增 `up_steps`）+ 推进行标签集 {`Submit`,`Next`}（Next 精确相等防正文冒充）+ 第 1.5 段「已在 Review 屏直接确认」；④ Toggle 静态数字序列废止（显式 Err）；⑤ `action_supported` Toggle/Advance 门 |
| `remote/api.rs` | `StagePlan::ClaudeToggle{target}` + 派发臂 + `QuestionDispatch::ToggleDone{checked,verified}`（回执带屏读真值 `checked`）；段名常量 `toggle-row`；`stage_from_abort` 增切勾/切题关键词臂 |
| `mobile/ApproveCard.tsx` | 裁11 活口收口（用户裁决「完全并成一套」）：tone 恒 `question`、二元也走纵向编号列表（reject 徽标如实显示 `esc`）；PlanBody/plan-pending 的 rose 硬编码改 sky；警示脚注保留琥珀 |
| `mobile/QuestionCard.tsx` | 多选卡渲染勾选框字形 `[ ]`/`[✓]`（与终端同形）；toggle 回执带 `checked` 时以屏读真值同步本地态（不盲翻），缺失回落盲翻 |
| `mobile/api.ts` | stage 并集 + `toggle-row`/`advance`；`QuestionAnswerResult` + `checked`/`advanced` |

### Stage 2 —— claude 接入多题流（复用 1e14c56 地基）

| 文件 | 改动 |
|---|---|
| `inject/question.rs` | `run_advance_stages`（走位到 `Next`+回车+读屏分类：下一题页/Review 屏/中止；**已在 Review 屏=零按键 `advanced:false`**——← 回退未实测不乱试）；`multi_question_submit_supported` 放行 ClaudeFull；Advance 静态序列显式 Err |
| `remote/api.rs` | 三闸门纳入 claude（GET `multiQuestion`/`advance` 旗标、POST 多题闸门）；claude 单题载荷上的 advance → 400 bad_request（防误触）；`StagePlan::ClaudeAdvance` + 派发臂 + `AdvanceDone{advanced}` 回执；`bad_request` 归 400 |
| `mobile/QuestionCard.tsx` | advance 回执 `advanced:false` 时不推进（停在确认卡）；段名文案 advance |

## 2. 测试

- Rust：新增回归锁——hooks 双注册收敛 ×3、space 键域、行块解析 ×5（实机 dump/用户截图/滚回区剔除/单选空块/字形族）、闭环切勾 ×6（happy/走下/走上/未翻转中止/无焦点/目标缺席）、`toggle_sequence_is_no_longer_digit`（原 K8 锁改写）、端点级切勾 ×2、claude 多题 ×4（旗标+toggle/走位切题/Review 零键/已在 Review 直接确认）、`multi_questions` 拒绝载体换 zcode、`action_gates` 改写。三个改动模块全绿（inject 315 / server 146 / hooks 50）；全量 30 红均为 main 既有的 junction/symlink 环境测试（stash 验证与本批无关）。
- 前端：审批卡并蓝锁改写、勾选框字形 + `checked` 真值优先（含「回执仍 true 不翻回」反例）、盲翻回落。**825 全绿**；`pnpm check` 过。
- `#[ignore]` 实机占位：`claude_space_injection_form_live_probe`（空格形态复验指引——手工按键 ≠ 注入事件，回执「翻转」中止时的处置路径）。

## 3. 实机验收清单（待用户执行）

1. 启动 MAM 一次 → 读 `~/.claude/settings.json` 确认每事件只剩 helper 单条（bash 残留被自动清理）；
2. claude 问**多选单题** → 手机出蓝卡（不再有红色「等待批准」）→ 勾两项（卡面 `[✓]` 与终端同步）→ 提交 → Review 确认 → 会话文件出现 `Your questions have been answered: …`；
3. claude 问**三个问题（含多选）** → 逐题作答（多选子题勾选不切页）→「切换题目」逐题推进 → 确认卡「提交答案」→ 同上核验；
4. 二元审批卡与问答卡并排同形态（蓝、纵向编号）；
5. 若第 2 步回执报「空格已发出但屏读未见到勾选态翻转」→ 按 `claude_space_injection_form_live_probe` 指引复验空格形态（候选 char 形态）。

## 4. 已知边界（显式申报）

- ← **回退切题**未实现（用户实测该键在 `Type something`/`Next` 两行不生效，回退是复合动作未取证）；确认卡的「返回题目修改」在 claude 上为零按键不推进。
- 多选屏的 `Type something` 自由作答维持拒绝（复评 F6-3 口径不变）；`Chat about this` 行不触达。
- `families.rs` 的 `verified_with: "2.1.251"` 未改（A 族注入形态未在 2.1.278 复验——版本漂移只在取证档案申报）。
- 验收后应把「手机端→终端」逐键屏读对账追加进取证档案（≥3 取样升格「实测」）。
