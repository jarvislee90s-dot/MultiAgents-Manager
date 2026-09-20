# M6R–M9R · 验收测试清单（自动化 / computer-use / 人工三段 + Mac 回传追加）

> 本清单对应批次设计 `docs/superpowers/specs/2026-09-18-phase2-m6r-m9r-injection-hardening-design.md` 的 R1–R7：
> R1 探测补测（批外完成，证据归档 `research/refs/phase2-消息注入/`）· R2 Windows 注入引擎重写 · R3 队列与回执修复 · R4 审批补全（严格档）· R5 一键 resume · R6 macOS 匹配修复 · R7 杂项清理。
> 结构四段：**A 自动化已覆盖（免人工）→ B computer-use 可辅助项（agent 可代看）→ C 人工主场景清单（用户真机）→ D Mac 回传清单追加**。
> 基线：分支 `feat/phase2-injection`（Windows 侧本地提交链，未 push）。终态门禁见台账 `.superpowers/sdd/2026-09-18-phase2-injection-mainline/progress.md`。
> 体例对齐 `docs/release-notes/m7-m8-macos-live-checklist.md`：实测结论不回写设计文档，问题按降级路径处理并回报用户裁决。

---

## A. 自动化已覆盖（免人工）

> 以下各条均可在常规门禁复现（`cargo test` / `cargo test --test m9r_e2e -- --ignored` / `pnpm test`），
> 测试名以 `src-tauri/src` 与 `src-tauri/tests` 实际代码为准（本清单逐条核对过源码，非计划文档抄录）。

### A1 · 族规格 / 事件构造 / 节流判定（R2）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 族表指纹 / 8 脆弱常量钉值 / 回退规格 | `inject/families.rs` | `constants_match_probe`、`family_table_matches_probe`、`fallback_spec_matches_default` |
| 背压判定 / 预算随背压放大 / 分块计划 | `inject/families.rs` | `backpressure_rules`、`budget_scales_for_backpressure`、`chunk_plan_counts_and_flags` |
| 文本→事件（ASCII VK+scan / 非 ASCII UTF-16 字符流 / 😀 代理对真值） | `inject/engine.rs` | `text_records_pair_with_vk_scan`、`text_records_non_ascii_char_stream` |
| 回车 VK 形态 / 控制键域校验（域外 None=P2-2）/ VT 整条原子流 / 方向键按族钉值 | `inject/engine.rs` | `enter_is_vk_form`、`control_keys_and_domain`、`control_records_single_char_domain`、`vt_seq_is_char_stream`、`vk_arrow_for_crossterm`、`vk_arrow_records_pins_and_domain` |
| `\n`→字面 `\n` 归一 / 设备名同样归一 / 裸 CR / 摘要截断 | `inject/normalize.rs` | `newlines_become_literal_backslash_n`、`compose_normalizes_device_name_too`、`bare_cr_normalizes`、`summarize_truncates` 等 5 测 |
| 路由表纯核（小写精确匹配 / platform 词表） | `inject/routing.rs` | 6 测 |

families.rs 6 测 + engine.rs 18 测（含 macOS 构造，见 A6），共 24 测。

### A2 · Windows 通道执行层重写（R2）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 键域域外拒绝（先于锁/附加，不回退 P2-2） | `inject/windows_console.rs` | `unknown_key_rejected` |
| 族分派规格送达（A 族 VT 字符流 / B 族 VK） | `inject/windows_console.rs` | `key_records_for_family_dispatch` |
| WinKeyLayout 真实 FFI 对拍（VkKeyScanW/MapVirtualKeyW） | `inject/windows_console.rs` | `win_key_layout_ffi_contract` |
| paced_write 背压 / 真总预算 / 部分写续写 / 122 重试（实现层纪律，注释量化最坏持锁 ≈460s） | `inject/windows_console.rs` | 编译门禁 + 下方 #[ignore] 实机探针 |
| CONSOLE_OP 全局互斥 + AttachGuard RAII 全路径 FreeConsole 复位（P1-1/P1-2） | `inject/windows_console.rs` | 实现层纪律（panic/错误路径复位由 RAII 结构保证） |

`#[ignore]` 实机探针（Windows 本机真跑过，常规门禁只编译）：`ffi_hop_injects_into_fresh_console_cmd`（FFI 一跳落盘地面真值）、`occ_stuck_reports_freeze_receipt_probe`（判冻中文状态回执——「Ok 且完成」或「Err 含『选择模式』/『输入积压』回执关键词」二选一放宽断言；D8 裁决后不自动解冻，try_unfreeze 已随 d38dbd5 移除）、`read_input_tail_returns_recent_chars`（CONOUT$ 屏读）。

### A3 · 判冻窗口（R2）

- 判定口径：占用>0 且**相邻采样无下降**（逐样本口径，质量评审 C1 定案）持续 ≥ `OCC_ABNORMAL_MS`(5s) 才判冻结——慢消费者合法 drain 不误报；常量钉值在 `families::constants_match_probe`。
- 自动化覆盖形态 = A2 末条的 `#[ignore]` 实机探针（判冻回执，含放宽断言）；**WT 宿主判冻检测行为未实机验证**（§8.3 已知限制；D8 裁决 2026-09-19 后不自动介入，恢复=人工点窗）——实机验收落点见 C-12。

### A4 · 确认层（R2 / 裁决 A1）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| stamp 纯核（trim_end→24 字符 / 多字节边界 / 只查 user 角色 / 空戳恒否） | `inject/confirm.rs` | `stamp_logic`、`stamp_of_multibyte_boundary`、`stamp_hit_filters_to_user_role`、`empty_stamp_never_hits`、`stamp_found_in_user_messages` |
| 直发确认端到端（未中→Failed 中文文案；注入失败短路 probe） | `inject/queue.rs` | `direct_confirm_failure_returns_err`、`flush_one_end_to_end_confirm_failure`、`inject_failure_short_circuits_confirm_probe` |
| 插队 best-effort（屏读/drains 不可用不 Gate） | `inject/queue.rs` | `jump_receipt_best_effort_when_drain_unavailable`、`jump_delivery_ignores_confirm_probe`、`session_stamp_hit_misses_when_read_fails` |

### A5 · 队列生命周期 / 端点回执四态（R3 / 裁决 19）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 四态分流（绿直发 / 黄排队 / jump 越态 / 会话消失挂起） | `inject/queue.rs` | `running_session_regular_path_holds_queue_head`、`waiting_session_flushes_queue_head`、`jump_delivers_even_when_running`、`missing_session_suspends_even_on_jump` |
| 停服冻结 / 重启对账 / 周期兜底（纯 SQL 计数门） | `inject/queue.rs` | `stop_freezes_flush_loop`、`reconcile_flushes_pending_on_restart_like_state`、`periodic_sweep_gated_by_pending_count`、`periodic_sweep_flushes_when_pending_present` |
| 端点回执精确映射（Sent→delivered / Failed→failed / Deferred·Suspended→queued） | `remote/server.rs` | `send_delivers_when_input_ready`、`send_reports_inject_failure`、`send_queues_when_running`、`queue_jump_suspended_returns_queued`、`send_info_matrix` |
| 撤回 / 插队 in-flight 守卫（P2-6，双端） | `remote/server.rs` | `queue_jump_busy_inflight_returns_failed`、`queue_retract_busy_inflight_returns_failed`、`send_input_ready_busy_inflight_falls_back_to_queue` |

### A6 · 版本探测超时垫片（R3 灰1）与审批映射纯核（R4）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 探测 3s 超时→kill→None 入缓存；cmd /c 垫片回退；非 UTF-8 输出 lossy | `inject/approve.rs` | `probe_version_times_out_and_caches_none`、`probe_version_via_cmd_shim` |
| 映射 KV 三态（缺省/损坏/合法）/ 默认表覆盖 / codex 源短语 markers / claude plan 追加 / 大小写不敏感 detect / 漂移规则 / 选项表 | `inject/approve.rs` | `load_mappings_from_three_states`、`default_mappings_cover_claude_codex`、`codex_markers_are_source_phrases`、`claude_plan_marker_added`、`detect_case_insensitive_marker`、`drift_rules`、`option_lookup` |

### A7 · 严格档（R4）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| probe_pending 双端点 fail-closed + **零按键断言**（未取证版本不出键） | `remote/server.rs` | `probe_pending_strict_policy` |
| 非严格档 reason=null；detect 未命中仍给 hint | `remote/server.rs` | `non_strict_unavailable_reason_is_null`、`probe_pending_hint_even_on_detect_miss` |
| 红卡端点（选项下发不含键位 / 批准 / 拒绝审计 / 守卫 / 无映射 404） | `remote/server.rs` | `approve_options_available`、`approve_options_not_waiting_or_no_hit`、`approve_options_no_mapping`、`approve_sends_key`、`approve_reject_audits_reject`、`approve_guards`、`audit_action_vocab` |

### A8 · 一键 resume（R5）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 命令表定案（claude `--resume` / codex `resume` / kimi `--session` / opencode `--session`；未装工具 None） | `inject/resume.rs` | `resume_command_table` |
| Windows spawn 构造（wt 优先 / conhost+CREATE_NEW_CONSOLE 回退一次） | `inject/resume.rs` | `build_spawn_command_windows`、`windows_wt_failure_falls_back_to_conhost_once` |
| macOS 双通道脚本（cd+命令+聚焦+转义；哨兵先于 spawn） | `inject/resume.rs` | `macos_scripts_carry_cd_focus_and_escaping`、`macos_dual_channel_dispatches_via_seam`、`core_open_sentinels_before_spawn`、`core_open_dispatches_spawn_on_windows` |
| 端点（200 opening+审计 open / 404 no_cwd / 404 no_resume_command / 404 no_session+坏请求） | `remote/server.rs` | `session_open_endpoint_opens_and_audits`、`session_open_endpoint_no_cwd`、`session_open_endpoint_no_resume_command`、`session_open_endpoint_no_session_and_bad_request` |
| 端点失败回执（200 failed+审计 failed: / spawn 失败 / macOS TCC 指引形态，M2） | `remote/server.rs` | `session_open_endpoint_spawn_failure_reports_failed`、`session_open_endpoint_macos_tcc_guidance_reports_failed` |
| osascript 错误分类（-1743 TCC→中文指引 / 其他→stderr 摘要截断，M2 纯函数跨平台可测） | `inject/resume.rs` | `classify_resume_error_maps_tcc_to_guidance`、`classify_resume_error_other_keeps_summary` |
| **F1 macOS 死窗治理（2026-09-20，mac-reverify §四-A）**：通道顺序 Terminal.app 优先、iTerm2 次选；出手后效果回查（3s 窗按 resume 命令特征查子进程，未命中转次选；双败 failed 回执+审计 open failed:*——「死窗」入账） | `inject/resume.rs` | `macos_dual_channel_dispatches_via_seam`（新序）、`macos_terminal_dead_window_check_miss_falls_to_iterm2`、`macos_both_channels_fail_merged_with_channel_labels`、`resume_effect_in_snapshot_pure` |

### A9 · macOS 匹配构造（R6）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| iTerm2 全路径相等匹配（`is`，防 ttys 前缀撞号） | `inject/engine.rs` | `iterm_scripts_use_exact_tty` |
| tmux `parse_panes_find` 纯函数 + 前缀撞号回归锁（ttys100/ttys1000 互不误配） | `inject/engine.rs` | `tmux_pane_match_is_full_path` |
| Terminal.app 遍历全部标签页（do script in t / 先选后发） | `inject/engine.rs` | `terminal_scripts_traverse_tabs`、`terminal_script_uses_do_script`、`terminal_send_key_script_activate_and_keystroke` |
| iTerm2 两步 write / 按键分派 / tmux 字面与回车 / AppleScript 转义 | `inject/engine.rs` | `iterm_script_finds_tty_and_writes_without_newline`、`iterm_send_key_script_dispatch`、`tmux_literal_and_enter`、`tmux_key_args_literal_vs_named`、`applescript_escape_backslash_and_quotes` |
| `normalize_dev_tty`（裸后缀→/dev/ 全路径；Windows 可测 3 断言） | `window/mod.rs` | `normalize_dev_tty_tests` 模块 |

### A10 · 前端（R3/R7，vitest 562/562 · 73 文件）

| 覆盖点 | 位置 | 代表测试 |
|---|---|---|
| 对账恢复（jump/retract 失败→fetchQueue 复核→在队恢复/不在队中性 gone/复核失败保守恢复） | `tests/mobile/MessageComposer.test.tsx` | `jump_busy_restores_queued_view`、`retract_failure_same_reconcile`、`retract gone 中性收敛（评审必须2）`、`retract busy 分流（评审必须1）` |
| 「投递中…」chip（await 全程不空白） | `tests/mobile/MessageComposer.test.tsx` | `delivering_chip_during_await` |
| 10000 上限双保险（与后端 MAX_SEND_CHARS 对齐） | `tests/mobile/MessageComposer.test.tsx` | `maxlength_guard` |
| 排队/插队/撤回/轮询/位次 0 容忍/sending 期按钮加闸 | `tests/mobile/MessageComposer.test.tsx` | `排队态：回执「排队中 第1位」…`、`立即发送（插队）…`、`排队态 3s 轮询刷新队位…`、`queued position=0 容忍`、`sending 期间排队按钮加闸（评审必须3）` |
| 严格档（后端下发 reason→卡片只渲染提示条不渲染按键组）/ 降级文案 / drift | `tests/mobile/ApproveCard.test.tsx` | `probe_pending_options_show_hint_only`、`ApiError 404 no_mapping…`、`drift=true…` |
| resume 分诊（failed 文案上屏 / opening 提示 / 404 no_cwd 分诊 / 无 cwd 禁用镜像） | `tests/mobile/SessionDetail.test.tsx` | `评审 C1：200 failed 回执不得当成功…`、`200 opening → 成功提示条…`、`404 no_cwd → 按错误码分診中文文案…`、`无项目目录 → 按钮禁用 + 原因…` |
| 审计页 | `tests/settings/AuditLogSection.test.tsx` | 失败态不伪装空态等 |

### A11 · E2E 七例（R2 全链 + 设计 B1 项四家矩阵 + F5 评审扩三例，`#[ignore]` 实机显式跑）

运行命令：

```text
cd src-tauri
cargo test --test m9r_e2e -- --ignored --nocapture --test-threads=1
```

执行纪律三条（2026-09-19 追加，固化本批 E2E 假执行教训——见台账收尾小批 P2 E2E 重跑记录）：

- 用绝对路径或在 `src-tauri/` 目录执行，勿依赖 shell 当前目录（后台管道下 CWD 错误会产生假执行）；
- 判定绿以**退出码**为准，勿以管道 tail 输出判绿（`cmd | tail` 的退出码是 tail 的）；
- 运行中的 MAM 应用会锁 `target/debug` 的 exe——用 `CARGO_TARGET_DIR=target/gate-run` 隔离跑测试（或先退出 MAM）。

前置：Windows 宿主 + conhost 控制台拓扑（项目技能 `win-console-inject-probe` 起会话法）；本机四家 CLI 且版本与族规格指纹一致（claude 2.1.251 / codex 0.154.0——按 npm 包版本口径，TUI 自报允许漂移 / kimi 2.0.0 / opencode 1.18.31）；零接触真实 `~/.mam`（内存库），只读真实 CLI 会话存储（A1 确认语义所需）。硬杀测试进程会残留探测终端需手动关。

| 用例 | 断言 | 首跑实绩（2026-09-19） | 验收复跑（2026-09-19，Task 13 终跑） |
|---|---|---|---|
| `e2e_engine_matrix_short` | 四家 stamp 全命中；背压旗标 opencode=true 其余 false | 76.6s，全过 | 四家 150 字符全过：背压旗标 opencode=true 其余 false；stamp 命中 0/17/2/1ms（顺序 claude/codex/kimi/opencode） |
| `e2e_engine_matrix_long` | claude 10k <15s；opencode 10k ≤460s 预算；双 stamp 命中 | claude 10k=2.1s / opencode 10k=106.3s，全过 | claude 10k=2.1s（2098ms）/ opencode 10k=86.6s（86639ms，快于首跑）；双 stamp 命中 1ms |
| `e2e_http_full_chain` | HTTP→路由→队列→引擎→确认→审计全链：delivered + stamp 命中 + 审计 send/flush 两行 | 39.7s，全过 | 200 `{"status":"delivered"}` + stamp 命中 0ms + 审计 send/flush 两行（channel=real） |
| `e2e_key_domain_and_enter` | codex VK 回车提交生效（rollout 命中）；域外键拒绝 Err | 19.0s，全过 | codex rollout 命中（1506ms）+ stamp 命中 12ms；域外「bad!」拒绝 Err |
| `e2e_wt_host_matrix`（F5） | WT 宿主 × 2000 字符 × claude/codex：written=2000、背压旗标双 false（2000 恰不超 `LONG_MSG_CHARS` 阈值）、双 stamp 命中 | 39.3s，全过 | written=2000 ×2；stamp 命中 2ms（claude）/9ms（codex）；WT 下 TUI 定位 tui/cmd 双命中（`wt -d` 落目录经 sysinfo cwd 标记匹配） |
| `e2e_codex_long_10k`（F5） | codex 10000 字符：背压=true（快消费者 10000>2000，`families::use_backpressure` 语义）、耗时 ≤460s 预算、stamp 命中 | 21.6s，全过 | written=10000、backpressure=true；注入耗时 2143ms（预算 460s，余量巨大）；stamp 命中 21ms |
| `e2e_cross_session_no_crosstalk`（F5） | claude+codex 双会话并行注入：各自 stamp 各自命中 + 对方会话存储零串扰 | 41.9s，全过（首跑 42.0s 同绿） | A/B written=2000、背压双 false；A stamp 0ms / B stamp 10ms；双向异戳零命中（并行触发、CONSOLE_OP 内部串行） |

验收复跑合计 **4 passed / 0 failed · 273.73s**（全套实跑命令见上，`--test-threads=1`；执行序按字母序 long→short→http→key）。首跑全套连跑 293.82s；证据目录 `%USERPROFILE%\mam-probe-m6r\evidence\m9r-e2e\`（按 `<用例名>-<run-id>` 归档，run.log 随 run 累积）。慢用例（opencode 长文）按测试文件头「单例失败先隔离重跑再定因」纪律处理——**重跑即愈**为既定判定口径。F5 三例首跑合计 **3 passed / 0 failed · 102.82s**（2026-09-19，逐一显式跑；跨会话例为取证完整性连跑两次均绿）。

**矩阵口径说明（F5 扩三例后，如实标注已测面/未测面）**：已测面 = conhost×短文×四家（`e2e_engine_matrix_short`）+ 10k×claude/opencode（`e2e_engine_matrix_long`）+ WT×2000×claude/codex（`e2e_wt_host_matrix`）+ codex 10k（`e2e_codex_long_10k`）+ 跨会话并发（claude+codex，`e2e_cross_session_no_crosstalk`）。**未测面如实留白**：WT 宿主 × kimi/opencode、WT 宿主 × 10k 长文、conhost × kimi/opencode 10k、跨会话并发 × kimi/opencode——以上组合无自动化覆盖，验收时如有需要按 B/C 段人工项补看；WT 判冻检测行为仍为已知限制（§8.3，见 A3/C-12；D8 裁决后恢复=人工点窗，不自动介入）。

---

## B. computer-use 可辅助项（agent 可代看）

> 以下各项不需要手机/解锁等用户独占资源，可由 agent 用浏览器（过 PIN）+ 截图代看；每项产出截图留档。

| # | 项 | 前置 | 步骤 | 预期观察 |
|---|---|---|---|---|
| B-1 | 四家终端注入后行在框 | 四家 CLI 各起一个会话（WT 与 conhost 各一批） | 移动端/浏览器各发一条短消息 | 截图核验终端出现 `[mobile <设备名>] <消息>` 且整行在框内（WT+conhost 各一） |
| B-2 | resume 窗口聚焦 | 任一有 cwd 的会话 | 点「在电脑上打开」 | 新窗口打开且前台聚焦；截图留档 |
| B-3 | 移动端页面走查 | 浏览器过 PIN 进移动端 | 发送→排队→插队→撤回→审批卡逐态截图 | 各态 chip/文案与 A10 自动化断言一致 |
| B-4 | 审计设置页 | 完成上述操作 | 桌面设置 → 注入审计逐条截图 | 每条写动作（send/flush/jump/retract/approve/open）逐条可查、只增不改 |

---

## C. 人工主场景清单（用户真机）

> 主力场景优先；需真机手机（蜂窝复验另列用户项）与用户在场（审批触发、锁屏等）。
> 行为变化须知（评审修复批 F7⑩ + 收尾批 P1）：消息文本中的 Tab 及 C0/C1 控制字符与 DEL(0x7F) 会被滤除后注入（裸 ESC 不再直发终端、DEL 不再触发 B 族退格）；换行归一字面 \n 语义不变。

| # | 场景 | 前置 | 步骤 | 预期观察 |
|---|---|---|---|---|
| C-1 | 四家各发短消息 | claude/codex/kimi/opencode 各一会话 | 手机发一条短消息 | 送达回执（delivered）+ 桌面终端可见 `[mobile 设备名]` 行 |
| C-2 | 长消息 2000 字 | claude（快）与 opencode（慢）各一 | 发 2000 字消息 | claude 即时送达；opencode 先「投递中…」chip → 最终命中回执（慢消费者耗时见已知限制） |
| C-3 | 黄排队→转闲自动 flush | 运行中会话 | 发消息得「排队中 第1位」→ 等任务完成 | ≤1s 自动送达，chip 收敛 |
| C-4 | 立即发送插队 | claude 运行中 + 排队项 | 点「立即发送」 | **Windows**：busy 态插队 → 命中排空回执，或 Failed +「正文可能已写入输入行」防重文案——两者均算 PASS；**macOS**：busy 入草稿/缓冲、转闲后命中，按实机行为记录 |
| C-5 | 撤回 | 排队中条目 | 点「撤回」 | 条目消失，终端不出现该内容 |
| C-6 | **关闭远程→队列冻结→重开续跑** | 有待发排队项 | 关闭远程开关 → 重开 | 关闭期间队列冻结不投递；重开后对账自动补投（裁决 19） |
| C-7 | MAM 重启→待发自动补投 | 有 PENDING 条目 | 重启 MAM 应用 | 启动对账 flush 补投待发项 |
| C-8 | 审批 codex（Read Only 档） | codex 沙箱调 Read Only 档触发审批框 | 红卡点「允许」/「拒绝」 | 终端收到 y/esc（Windows 取证键位），批准/拒绝生效；审计 action=approve/reject |
| C-9 | 审批 claude 计划模式 | claude 计划模式出计划 | 红卡点批准执行 | 计划被批准开始执行（「1」键，Windows 取证一致） |
| C-10 | resume 双端各一次（≥2 家工具） | 桌面端 + 移动端各一次 | 对 ≥2 家工具点「在电脑上打开」 | 新窗口打开、cwd 正确、前台聚焦；审计 action=open（仅移动端触发落账；桌面端 Tauri 路径不写审计——audit 需设备身份）。**F1 后 macOS 口径（2026-09-20）**：Terminal.app 优先出手 + 3s 效果回查，死窗自动转 iTerm2、双败回 failed 回执+审计 failed——macOS 实测记录新序与回查行为（iTerm2 死窗已知问题见 mac-reverify §四-A，上游升级复测后回翻） |
| C-11 | 审计页逐条可查 | 完成上述操作 | 设置 → 注入审计 | 逐条对应，无缺漏 |
| C-12 | WT/conhost 判冻与人工点窗恢复 | 任一 CLI 会话制造输出停顿/选择模式（如终端内文本框选） | 注入长文制造背压与停顿：注入回执出现「目标终端疑似进入选择模式…」中文状态回执（D8 裁决 2026-09-19：不自动介入）→ 用户点一下该终端窗口清除选择模式 → 重发 | 状态回执如实出现，点窗后重发成功、正文最终命中；**WT 宿主判冻检测行为未实机验证（OccWatch 探针为 #[ignore]，§8.3）——异常即记录不判 FAIL** |
| C-13 | 滞留→屏读→补按回车恢复路径 | 任一快消费者会话（E10 尾字符丢失场景：正文已打入但尾字符/提交回车丢失） | 直发后若确认失败回执出现：终端可见已打内容但未提交——人工按一次回车即提交；或等确认层自动屏读回查补按（500ms 轮询 3s 窗，`confirm::RECHECK_MS`） | 滞留内容最终提交进会话（会话文件见戳），不重复双投；确认回执与实际落盘一致 |
| C-14 | 锁屏场景注入/通知可达性（原 M7 遗留补充项） | Windows 锁屏态（命令备好于 mam-probe 报告 §6；macOS 侧锁屏项见 m7-m8 清单 #5） | 锁屏后远程发消息，观察注入与通知可达 | 占位：沿用既有验收口径，未在本批新增内容 |
| C-15 | 真机蜂窝网络复验（原用户项） | 外网蜂窝网络手机 + Windows 实机 | 蜂窝网络下远程发消息走全链 | 占位：沿用既有验收口径，未在本批新增内容 |
| C-16 | codex hooks 触发复验（**F3 PascalCase 修复后**，M1A 前置；**T2 后注册面扩为 8 键**） | codex 0.155.x 在场 | 跑 `cargo test --lib monitor::hooks::codex_pascal -- --ignored`（注册写入真实 ~/.codex/hooks.json）→ 跑一次真实 codex 交互会话 | hooks.json 出现 PascalCase 8 键（六状态键 + `PermissionRequest`/`Interrupt`，T2）且旧 camelCase 键被迁移清除；会话期间 `~/.mam/events/<session_id>.json` 出现（hook 真触发）。自动化形态：`cargo test --lib monitor::hooks::codex_hook_events_really_fire -- --ignored`（T2 沙箱自检，tempdir CODEX_HOME/MAM_HOME）。调研锚点：`research/refs/phase2-消息注入/2026-09-19-审批事件钩子通道调研.md`（0.155.1 键名 PascalCase 源码证据 §3.2；M1A 红卡=钩子信号+固定键位） |
| C-17 | codex hooks 信任门（**T2 新增**，C-8 根因③） | codex 0.155.x 在场，MAM 已注册 hooks（先跑 C-16 步骤①） | 开 codex TUI → 输入 `/hooks` | MAM 钩子条目以 **Untrusted** 列出 → 逐条审阅并信任（trust 后 hash 落用户层 config，仅需一次）→ 重跑一次会话确认 `~/.mam/events/<session_id>.json` 出现；**未信任前钩子不触发**（事件文件缺席为预期行为，不算 FAIL）；重启 codex 后信任态保持（无需重复信任）。注册侧引导：MAM 每次注册成功 log::warn 提示本流程 |
| C-18 | **claude hooks 触发复验（F8 新增）** | claude 在场（**无需登录**——实测未登录态钩子照常触发）；前置=debug helper 已构建 | 跑 `cargo test --lib monitor::hooks::claude_hook_events_really_fire -- --ignored` | 沙箱自检（tempdir `--settings` 文件 + MAM_HOME 重定向）：注册后跑真实 `claude -p` 一回合，`~/.mam/events/<session_id>.json` 落盘且形态合法（sid 白名单/UUID 形态、event 非空）。**实测已过（2026-09-21，1.1s）**。注意：claude **无信任门**（与 codex 的 C-17 不同族）、**无 `CLAUDE_CONFIG_DIR` 语义**（沙箱靠 `--settings <file>`）——两条均为本批实机取证结论 |
| C-19 | **kimi hooks 触发复验（F8 新增）** | kimi 已装且 `~/.kimi-code/config.toml` 有可用模型；前置=debug helper 已构建 | 跑 `cargo test --lib monitor::hooks::kimi_hook_events_really_fire -- --ignored` | 沙箱自检（tempdir `KIMI_CODE_HOME`，config.toml 由真实配置只读复制后追加 `[[hooks]]`）：事件落盘且 **sid 为 `session_<uuid>` 形态**（下划线前缀——F8 修复点，旧白名单拒收该形态致 kimi 事件全量静默丢弃）。**实测已过（2026-09-21，2.3s）**。注意：**审批事件（PermissionRequest/PermissionResult）在无头 `-p` 模式不触发**（实测仅 SessionStart/UserPromptSubmit/Stop 触发），故自检注册面额外并入三个生命周期事件作管道探针——审批事件真触发归人工交互会话（C-20 矩阵） |
| C-20 | **三家实机矩阵（claude / codex / kimi；F8 就绪）** | 三家 CLI 在场；codex 需先过信任门（C-17）；helper 已 debug 构建 | ① 三家各跑一条 `--ignored` 自检（C-16/C-18/C-19，自动化形态）；② **人工交互会话**（三家各一）：claude 触发权限提示、codex 触发审批框、kimi 触发审批请求，期间观察 `~/.mam/events/<session_id>.json` 内容与看板红卡 | ① 三条自检全绿（claude/kimi 本批实测已过；codex 至信任门前一步=事件不落盘为**设计内**，见 C-17）；② 人工会话中审批进入事件落盘 → 看板该会话**强制 Waiting 红卡**（T4 接铃铛）→ 手机上批准/拒绝 → 对应事件（claude PostToolUse/Stop、codex PostToolUse/Stop、kimi **PermissionResult**）清除标记 → 卡片回落绿灯（**F1 修复点**：kimi 清除链此前漏接 PermissionResult，红灯永不落）。**人工项归用户执行**（agent 不代做交互会话） |

> **F8 台账追记（2026-09-21，三家矩阵真实状态）**：
> ① **claude** = 全链实测通（C-18 自检 1.1s 过，事件落盘含 SessionEnd/Stop 等；print
> 模式 stdin 为空但生命周期事件照常触发，helper 管道不需 stdin 亦可判事件名——实测
> SessionEnd 事件文件 sid/event 齐备）；
> ② **kimi** = 全链实测通（C-19 自检 2.3s 过；并发现并修复 session_id 下划线白名单
> 缺陷，见 `hook_listener::session_id_allowed`）；
> ③ **codex** = 链路通至信任门前一步（自检实跑：exec 会话正常完成、事件不落盘
> =untrusted 钩子不触发的**设计内行为**，C-17）；信任后即达与 claude/kimi 同档。
> 三家共用的 helper 薄管道、白名单、注册/迁移逻辑均已有 tempdir 单测覆盖（非
> 实机面），实机面差异集中在「各 CLI 的事件触发策略」与「codex 的信任门」两处。

---

## D. Mac 回传清单追加

> 以下各项同时追加于 `docs/release-notes/m7-m8-macos-live-checklist.md` 尾部，供 macOS 实机逐条验证。
> Windows 侧已由单测锁构造行为（A6/A9），Mac 侧验证的是**实机语义**。
> 判定与实测结论以本节为 SSOT，m7-m8 节为执行镜像勿单边修订。

| # | 项 | 前置 | 步骤 | 预期观察 / 判定点 |
|---|---|---|---|---|
| D-1 | 三通道长消息 + 确认机制 | tmux/iTerm2/Terminal 各跑真实 CLI | 直发长消息（>500 字）；再测插队排空+草稿 | 三通道均送达；直发以会话文件命中回执；插队 busy 入草稿、转闲命中；**macOS 无 CONOUT$ 屏读**——确认层「无屏读即 Failed」的降级语义可用性需实机确认（直发失败文案是否可接受/重试不双投） |
| D-2 | 相等匹配（ttys 前缀相近双会话互不串扰） | iTerm2 开两个 tty 前缀相近的会话（如 ttys001/ttys001x 形态） | 对其一发消息 | 只有目标会话收到；另一会话无扰动（R6 相等匹配实机复核） |
| D-3 | Terminal.app 后台标签页注入 | Terminal.app 多标签页，目标会话在**非前台**标签 | 对后台标签会话发消息 | `do script in t` 后台执行语义成立：内容进目标 tab，不切换前台标签 |
| D-4 | 审批 codex/claude 复验 | codex（Read Only 档触发审批）/ claude（权限请求+计划模式） | 红卡批准/拒绝各一次 | 按键生效（codex y/esc、claude 1/esc——以 Windows 取证口径为基准核对 Mac 端一致）；**codex Mac 版本键位漂移需复核** |
| D-5 | resume（iTerm2/Terminal 双通道） | 有 cwd 的会话 ≥2 家工具 | 点「在电脑上打开」 | 开窗 + `cd <cwd> && <resume 命令>` + 完成后置前聚焦均成立 |
| D-6 | **activate→keystroke 时序竞争** | Terminal.app 会话触发审批红卡 | 点批准（activate 后立即 keystroke） | 若实机偶发丢键/键落错窗 → 按预案在 activate 后加 `delay 0.2` 复验（降级预案已备案） |
| D-7 | **Terminal raise 语义裁决** | Terminal.app 多窗口 | 观察按键/审批应答时的升窗行为 | 二选一裁决：升窗不升标签 vs 纯后台（当前实现先选 tab 后 activate，行为以实机观察为准回报用户定案） |
| D-8 | **iTerm2/Terminal `tty of s/t` 全路径口径确认** | iTerm2 与 Terminal.app 各一 | 实机打印 `tty of (sessions/TAB)` 取值 | 确认返回 /dev/ttysNNN 全路径（相等匹配的匹配键前提；若返裸后缀则 normalize_dev_tty 兜底生效） |
| D-9 | **normalize_dev_tty 实机验证** | macOS 终端跑 `ps -o tty` | 观察输出口径 | `ps` 返裸后缀（如 `ttys001`）时归一为 `/dev/ttys001` 后匹配成功（Windows 侧已单测锁构造，实机口径待验证） |

> **M2 追记（Mac 验收 D-5 根因①处置，2026-09-19）**：开发版（未签名、无 bundle id）
> 首跑 resume 前需在 **系统设置 > 隐私与安全性 > 自动化** 中手动允许 MAM 控制
> Terminal / iTerm2 一次（TCC 不自动弹窗）。未授权时 osascript 以 -1743 失败——
> spawn 路径现**等待完成并捕获 stderr/退出码**，失败回执 200 `{"status":"failed"}`
> +「macOS 自动化授权缺失」指引、审计记 `open failed:*`，不再出现 audit `open ok`
> 但无窗的账实背离。

> **M1B 追记（macOS 审批红卡降级口径，2026-09-19 用户裁决定案；适用 D-4 / D-6 及 M8 映射项 M8-1/M8-2）**：
> **macOS 审批红卡暂不可达，降级路径（普通发送键位字符→排队→插队投递）Mac 实测等效可用，M8-1/M8-2 经降级全部达效**。
> 根因两处（任一独立成立即可卡死红卡，见 Mac 报告 §四-A：`docs/release-notes/mac-acceptance-report-55f37e7.md`）：
> ① 审批等待期会话状态被判 **processing** 而非 Waiting（`available = status==Waiting && 映射 && detect(last_message)`）；
> ② 审批提示是 TUI 层覆盖物**不落会话文件**，Windows 靠 CONOUT$ 屏读拿到，macOS 无屏读。
> 降级链实测证据：claude 键位「1」经降级投递提示被正确应答、命令执行；codex 键位「y」经降级投递批准生效
> （`~/Desktop/mam-accept-codex-test.txt` 落盘）；`POST /session-approve` 被 409 not_waiting 守卫正确拦截
> （不向未等待会话发错误键位）。判定落账：D-4=PASS（有漂移记录，y/esc、1 键位不变）/
> D-6=不适用（红卡不可达，approve 的 keystroke 路径无实机触发样本，未观察丢键）/
> M8-1=部分（经降级达效）/ M8-2=PASS（经降级）。
