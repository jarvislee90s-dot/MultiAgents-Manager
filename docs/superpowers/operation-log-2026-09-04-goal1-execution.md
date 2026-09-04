# 执行操作日志：WorkBuddy 适配 + APP 跳转与已读机制 + 工具勾选管理（Goal 1）

- 日期：2026-09-04
- 分支：`feat/workbuddy-app-jump-tool-toggle`（32 个 commit，未 push）
- 执行方式：subagent-driven-development（每任务独立实现者 + 独立评审者 + 评审修复循环）
- 本文为最终执行汇总的全文存档（同文本已尝试发送至「workbuddy 工具集成」窗口，受阻原因见第七节）

## 一、任务完成度表（Task 1-15）

| Task | 内容 | 状态 | Commit |
|---|---|---|---|
| 1 | petSuppressPopup 简化 + 气泡点击即清除 | ✅ | `1da636c`（前置 `a74e245` 修预存 prettier 漂移） |
| 2 | macOS APP bundle 提取与激活模块（TDD 5 测） | ✅ | `08c836b` |
| 3 | focus_session 接线 + 双平台 pid 失效兜底 | ✅ | `0a9621e` + 修复 `b903cff` |
| 4 | 深度链接探测（spike）+ 路由接线 + spec 回写 | ✅ | `5bd2a39` |
| 5 | WorkBuddy 骨架注册 | ✅ | `abbab42` |
| 6 | workbuddy_parser 完整实现（TDD 10 测） | ✅ | `34fc593` |
| 7 | 前端徽标/工具列/声音区接入 | ✅ | `cc9d8a6` |
| 8 | unread_sessions 表与 DAO（TDD 3 测） | ✅ | `235e4ee` |
| 9 | Session.unread + sync_unread_sessions + 宿主判定 | ✅ | `e8aa291` |
| 10 | Codex 每会话一卡聚合 + 心跳消失补偿 | ✅ | `5b2198a` |
| 11 | 跳转标记已读 + 未读卡 UI + i18n | ✅ | `dfd8d2a` |
| 12 | agent_tools DAO + 还原/重建服务 + IPC | ✅ | `7e9f04f` + 修复 `205f8e9` |
| 13 | 启用过滤生效（扫描/资源分布/toggle 守卫） | ✅ | `ab96d2d` |
| 14 | 设置页工具管理 UI（保存确认 + 离开拦截） | ✅ | `5137c10` |
| 15 | 后端下发工具列表 + 收尾 | ✅ | `fdc5d2b` |
| 收尾 | 计划勾选 + spec 审计修复 9 笔 + 终审修复 | ✅ | `7bb8653` `3d3c9d3` `b8bde43` `d80375d` `d2cfe60` `1d026f9` `2d9469b` `3a5021e` `2bc52b0` `db73227` |

三处「以实际代码为准」对照点均按真实代码对齐：

1. win32 `AllWindows.by_pid: HashMap<u32, Vec<(isize,String)>>`（计划的单层迭代改为双层）；
2. `LinkHealth{Valid,Dangling,NotLink,Missing}`（还原守卫收紧为 Valid|Dangling，Missing 跳过）；
3. MCP SSOT 实为 `~/.mam/mcp/<name>.json`（计划的 `ensure_repo_dir().join("mcp")` 会落错到 `~/.mam/skills/mcp`，已修正）；另发现 MCP 分配仅 assignment 行（`mcp-<name>` 前缀）、`toggle_plugin` 为 4 参。

Windows 代码经本机 `x86_64-pc-windows-gnu` target `cargo check/clippy -D warnings` 编译级验证；实机运行验证按计划注记留待 Windows。

## 二、门禁输出摘要（最终 HEAD `db73227` 复核）

- `cargo test`：**165 通过 / 0 失败**（新增 workbuddy 解析 10 测、unread DAO 3 测、Codex 聚合 2 测、host 8 测、restore 6 测、补偿 tempdir 5 测、未读合并 3 测等）
- `cargo clippy --all-targets -- -D warnings`：**0 警告**
- `pnpm check`（prettier + eslint + i18n 键齐平 + tsc + vite build）：**exit 0**
- `npx vitest run`：**59/59（17 文件）**（含按 W1 新语义更新的 pet 用例）
- `git status` 干净；i18n 中英键结构逐键比对零漂移

## 三、Spec 审计出入与修复（阶段二，10 处真出入全部以 spec 为准修复）

| # | 严重度 | 出入 | 修复 |
|---|---|---|---|
| D1 | 高 | `is_host_process` 只匹配 `.app/` 路径，Windows 恒 false → 未读卡在 Windows 无法保留 | 分隔符归一 + exe 文件名兜底判定，8 个双平台 fixture 测试 |
| D2 | 中 | sync 每轮刷新 `turned_green_at` → 24h 过期窗口持续后推，违 spec「转绿时间」语义 | ON CONFLICT 仅更新展示字段，保留首绿时间戳（TDD 先红后绿） |
| D3 | 中 | Ambiguous 窗口选择器跳转成功不标已读 | 前端 `focusHwnd` 成功后 `mark_session_read`（看板/宠物/通知铃三路径） |
| D4 | 中 | SSOT 缺失跳过仅 log 未入结果（spec §9 逐项报告） | `ApplyResult.skipped` + `RestoreOutcome` 三态 + 前端 toast 报告 |
| D5 | 中 | WorkBuddy 状态推导未叠加 mtime 300s 阈值 | `overlay_mtime_stale`（Processing 过期→Waiting），与 Codex 语义一致 |
| D6 | 中低 | Codex 聚合卡沿用 CLI 60s 阈值（重构丢失重解析） | 未认领文件按 App 形态重解析后聚合（seam + 120s fixture 测试） |
| D7 | 低 | 未读卡排序不保证排后 | `session_sort_cmp` 增加 unread 末位权重（测试锁定） |
| D8 | 低 | ToolIcon workbuddy 回退 Claude 图标（错品牌） | 腾讯蓝占位 WorkBuddyIcon（含 vitest） |
| D9 | 低 | 补偿/合并零测试 | 抽取 `_in` 纯核 + 5 个 tempdir 测试 + `build_unread_cards` 纯函数 3 测 |
| D10 | 低 | win32 `TOOL_CLAIM_KEYWORDS` 缺 workbuddy | 补 `("workbuddy", &["workbuddy"])` + 测试 |

非出入备案：未设独立 `list_unread_sessions` IPC（计划架构等价简化，已回写 spec §5）；90s 心跳阈值（spec 授权 2~3 倍轮询周期）；还原采用先暂存后删链（强于 spec 文面的安全序）。Task 4 探测结论（`workbuddy://chat/<id>`、`codex://threads/<id>`，均来自 asar 源码级证据）已回写 spec §9 风险表。

## 四、Self-Review（最终全分支评审）发现与修复

4 项重点全部通过：私有格式解析零 panic 面、cfg 分支两平台完整、i18n 中英零漂移、无调试残留（worktree 干净）。发现 1 项 Important 跨任务集成缝隙并修复：已停用工具会被 W4 心跳消失补偿「复活」出未读卡并触发通知（违反 W5 彻底隐藏）——三层纵深防御：补偿入口 `get_tool_enabled` 门禁 + 停用时清 `LAST_SEEN_SESSIONS` + `build_unread_cards` 注入启用谓词（新增测试 `disabled_tool_row_is_dropped`）。修复后复审裁决 **READY TO MERGE**。

## 五、GUI 端到端测试结果（阶段四）

环境阻塞两项（均超出可自主处置范围）：

1. **屏幕锁定**：显示器休眠后 Mac 进入锁屏（截图证据 `/tmp/mam-screen2.png`；`/tmp/mam-screen.png` 为锁屏前全黑帧）。无凭据不可解锁，所有视觉类清单项无法验证。
2. **系统代理故障**：`scutil --proxy` 显示 HTTP/HTTPS/SOCKS 指向 `127.0.0.1:29290` 但无进程监听，WorkBuddy 任务无法联网（`codebuddy -p "只回复 ok"` 返回 `502 connect ECONNREFUSED 127.0.0.1:29290`），无法产生真实会话驱动未读/补偿/宿主退出链路的实机验证。

已完成的后端级实机验证（MAM dev 实跑 12 分钟）：

- ✅ MAM dev 启动成功；`WorkBuddy: 0 processes, 0 sessions` —— 心跳过滤在真机正确生效（`--serve` 的 `interactive-8979` 与心跳过期的 prewarm `11952` 均被排除）
- ✅ `Codex: 2 sessions from 1 processes` —— 会话监控管线对既有 Codex CLI 会话正常出卡
- ✅ 运行全程 0 panic / 0 ERROR
- ✅ 深度链接 OS 级实测：`open "codex://threads/<uuid>"` 与 `open "workbuddy://chat/<uuid>"` 派发成功；两 APP Info.plist scheme 注册静态确认（ChatGPT→`codex`、WorkBuddy→`workbuddy`）；AppleScript `activate application` 派发成功
- ✅ 锁屏后轮询日志静默属预期（WebView 隐藏时 react-query 暂停轮询，`refetchIntervalInBackground:false`），进程存活

因阻塞未能验证的视觉项（建议解锁后按 spec §8 清单补测）：会话卡黄→绿流转、未读绿卡点与 X 关闭、点击卡片跳转视觉确认、MAM 重启未读保留、退出 WorkBuddy 卡片清理、宠物气泡静默与点击清除、工具管理保存确认弹窗与离开三选拦截、还原前后 `ls -la` 链接快照对比、beforeunload 关窗拦截。

## 六、遗留问题与建议

已按最终评审 triage 归类为 follow-up（均非合并阻塞）：deep_link `open` 派发即成功语义（误路由会误标已读——解锁后实测 codex 直达，失败则按预案回退 None）；聚合测试 mtime 全等 tie 用例；补偿先移除 pid 的窄竞态；DAO 静默写失败可补 log；sub-agent 级分配的工具层还原语义需单独设计；toggle-back 后保存按钮残留（UX）；`rebuildFailed` toast 补 i18n；未保存关窗拦截建议改 `onCloseRequested`；`tools=[]` 瞬态「全部停用」闪现；`as keyof` 声音类型强转建议共享 SSOT。另建议：解锁后补做视觉清单；Windows 实机验证 win32 近祖聚焦与 `reactivate_tool_app`；WorkBuddy 品牌图标素材替换占位 SVG。

结论：**建议合并**。台账与全部评审包在 `.superpowers/sdd/`（git-ignored），分支 32 个 commit 未推送。

## 七、阶段五发送受阻记录

目标：将本文发送至 ZCode 桌面端「workbuddy 工具集成」会话窗口。

受阻原因：执行时 Mac 处于**锁屏状态**（无凭据不可解锁）：

1. 前台切换被系统拒绝——`open_application activate=true` 对 ZCode（dev.zcode.app）返回 "the live target never became foreground"（锁屏使任何应用无法成为前台）；
2. ZCode 窗口内容 AX 树挂起——`get_app_state` 仅返回菜单项（重复菜单树，1000 元素全部为 menuitem），无法定位会话侧栏与聊天输入框；也无法凭标题栏文字匹配「workbuddy 工具集成」（无视觉、无内容树）；
3. 窗口截图不可用——MCP 窗口捕获与 `screencapture -l` 均被拒/失败（锁屏会话无法产生窗口像素）。

已尝试的替代方案：唤醒显示器（`caffeinate -u`，屏幕仍为锁屏）；无 window_id 观察主窗口；逐个窗口 `screencapture -l`；Quartz 枚举（python3 无 pyobjc）。

安全边界判断：在「严格只允许粘贴+发送这一次消息、不得对该会话做任何其他操作」的约束下，无法确认 ZCode 当前聚焦的会话即目标会话（可能误发到任意其他会话，包括本会话），盲发风险不可接受，故按完成标准受阻条款停发。

报告全文已存档于本文（`docs/superpowers/operation-log-2026-09-04-goal1-execution.md`）并已写入系统剪贴板（解锁后可直接 Cmd+V 使用）。

[$requesting-code-review](/Users/jarvis/.agents/skills/requesting-code-review/SKILL.md)

## 八、合并前修复轮（review 裁决 F1-F7，2026-09-04）

双评审 + 裁决：无 Critical，7 条 Important 合并前修复。逐条 TDD（先红后绿）+ 独立 commit（`fix(review):`）+ 全门禁。

| # | 发现 | 修复 | 测试 | Commit |
|---|------|------|------|--------|
| F1 | 已读冲销：sync 对活跃 Idle 会话电平 upsert，跳转已读删行后下轮重插 | 迁移语义：`unread_pool_action(prev, idle)`（adapter/mod.rs）利用 `session_status_cache` 上一轮状态，仅「非绿→绿」插入；持续绿走 `refresh_display`（UPDATE 不插入，转绿时间不滑动）；dao/session.rs 增自由 `find_status` | 纯函数边沿映射测试 + DAO「已删行不复活/转绿时间不滑动」测试 | `e19f3aa` |
| F2 | 孤儿 codebuddy（宿主 Electron 死、心跳新鲜）仍出活跃卡 | `filter_host_dead_cards`（adapter）对 App 形态活跃卡统一 `tool_host_alive_in` 过滤，复用 `SHARED_SYSTEM` 快照不另起扫描；`AgentProcess` 增 `exe` 字段；Codex 聚合宿主口径改 `codex_host_process`（与 `is_host_process` 一致，CLI 同名 exe/框架进程不算宿主） | 过滤器纯函数 2 测 + 宿主口径测试 + 空快照测试 | `721b890` |
| F3 | 深链第一顺位仅凭 agent_type+session_id 触发，CLI pid TTY 失败会误拉 ChatGPT.app | `should_try_deep_link(pid, pid_bundle)` 前置门：仅 pid=0（未读兜底）或 exe 可提取 .app bundle 才走深链；`activate_agent_app` 重排（快照先建、bundle 复用） | 3 个前置条件测试（含 CLI pid 回归锁） | `9bfff96` |
| F4 | preset 写命令无停用守卫；PresetList.tsx 第四处硬编码 TOOLS | `apply_preset`/`deactivate_preset`/`apply_preset_to_subagent`/`deactivate_preset_from_subagent` 加 `ensure_tool_enabled`（apply 类返回改 `Result`，前端 try/catch 透明）；`ensure_tool_enabled_conn`（内存库可测）；PresetList 改 `useEnabledToolsQuery` | 守卫连接变体测试（停用报错/缺行放行/重新启用放行） | `005c98c` |
| F5 | 重启后首见老未读卡重放历史通知 | `isFreshFirstSeenUnread` 新鲜度门控：仅 lastActivityAt（转绿时间）距今 ≤2 分钟才补通知，时间不可解析保守静默 | 6 个 vitest 用例（新鲜/超时/边界/非绿/非未读/不可解析） | `a9ec4a6` |
| F6 | Codex Phase 2 对全部未认领 rollout 二次 parse 后才过滤 24h 窗口（IO 放大） | `fresh_unclaimed_files` 纯函数（mtimes 注入）前移过滤，仅窗口内文件付出 App 形态重解析 IO | 新鲜过滤合成测试 + 认领排除测试改写（排除+存活双断言） | `2c7fc89` |
| F7 | 前端测试敞口 | 3 个 vitest 用例：①工具管理 toggle→dirty→确认弹窗分类文案→`update_tool_settings` 入参断言；②leave-guard「放弃更改」（不持久化 + dirty 重置）；③首见未读只通知一次（同卡重入不重发 + F5 老卡静默） | toolManagement.test.tsx（2）+ notifyOnce.test.ts（1） | `e5205a3` |

**门禁（最终 HEAD `e5205a3`）**：cargo test **176 通过 / 0 失败**（本轮 +11）；`cargo clippy --all-targets -- -D warnings` 0 警告；`pnpm check` exit 0；vitest **68/68（20 文件，本轮 +9）**；worktree 干净，未 push。

**spec §5/§6 语义自查**：①已读信号——F1 后「跳转已读删行」不再被电平重插，已读真正生效，且「变黄→删→再次转绿→重插」语义保留（prev=Thinking→Insert）；②生命周期——「状态进入绿色 → upsert」的迁移触发即 spec 字面语义；③宿主生命周期——「无 App 形态进程存活 → 全部卡片清理」在 F2 后覆盖活跃卡（此前只清未读池，为本次发现的真实缺口）；④W5 写命令报错——F4 补齐 preset 面；⑤F5 新鲜度门控为补偿通知意图（提醒本次转绿）的合理细化，不违背 spec；⑥F3 将深链第一顺位限定回 spec W2 的 APP 类会话场景。

## 九、Minor 修复轮（review 残留 4 条，2026-09-04）

| # | 发现 | 修复 | 测试 | Commit |
|---|------|------|------|--------|
| M1 | 补偿路径一次性复活：已读删行后 prewarm 回池，补偿仍无条件 upsert | `compensate_vanished_heartbeats_in` 增 `status_of` 注入：状态缓存记录「绿已被观测」（Idle/Finished）→ 跳过补插（行缺席 = 已读）；包装器传真实 `find_status` | 回归锁测试（已观测绿→不复活，观测表照常移除） | `88db24c` |
| M2 | ImportDialog.tsx 第五处硬编码 5 工具列 | 改 `useEnabledToolsQuery`（与 PresetList/资源视图同源，停用工具不出现） | pnpm check + 全量 vitest | `8a208a7` |
| M3 | CLI 兜底跳转 UX 困惑（设计内行为，评审建议加提示） | 后端 focus_session 非 Windows 分支打 `via` 标记（tty / app-fallback）；前端 useSessionJump 对 `form==="cli" && via==="app-fallback"` 弹一次性 `toast.info`；SessionCard 传 form；i18n 中英键 `sessions.appFallbackHint` | 3 个 vitest 用例（CLI 兜底提示 / TTY 直达不提示 / APP 会话不提示） | `a70f66b` |
| M4 | parser 测试字面量缩进错乱 | 全仓 `cargo fmt`（分支漂移一并收口，`cargo fmt --check` 归零） | 全量门禁 | `53859c7` |

**门禁（最终 HEAD `53859c7`）**：cargo test **177 通过 / 0 失败**（本轮 +1）；clippy 0 警告；`pnpm check` exit 0；vitest **71/71（21 文件，本轮 +3）**。

**已知 flaky**：`tests/pet/foxbell-cards.test.tsx` 歧义浮层用例曾出现一次 3s 超时抖动（隔离重跑与全量重跑均通过），疑为 timing 敏感等待，后续可加显式 waitFor。
