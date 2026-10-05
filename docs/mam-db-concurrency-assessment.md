# mam.db 并发能力评估与提请（用量功能触发，未实施）

- 日期：2026-10-03
- 状态：**仅评估与提请**。说明书 §7.2 明文：该改动会改变整个 `mam.db` 的既有行为，
  属【默认裁定】，必须由用户裁决后才可实施。
- **本任务零生产代码改动**：`mam.db` 的 **WAL / `busy_timeout` / 独立连接一律未实施**
  （GC 2 的禁令项，一行未动）。本文件只做评估与提请；可执行验收在
  `src-tauri/tests/usage_concurrency_test.rs`。
- 触发背景：用量采集需要在 3 秒会话轮询之外写库。当前实现（计划①）已经用
  「分批短事务 + 批间让出」把单批锁持有压到毫秒级（见 Task 4 / Task 22 断言），
  **不等 WAL 也能满足验收**；本评估回答的是「要不要进一步做」。

## 0. 结论速览

| 问题 | 结论 | 依据 |
|---|---|---|
| 现状会不会挡住 3 秒轮询？ | **不会**。写库期间轮询单次取锁最长等待 **13 ms**（空载 5 次 12.9–18.9 ms；负载下另观测到最高 **127 ms**），到 500 ms 上界仍有 **≥3.9×** 余量 | §2.1 / §2.2 / §2.3 |
| 反例（不分批）代价多大？ | 一个事务写 4,000 行 → 轮询最长等待 **355 ms**（**单次取样**；本机 4 次观测 355–2440 ms，现状的 **27×–188×**，倍率不稳定见 §2.3） | §2.1 / §2.2 / §2.3 |
| 500 ms 阈值能不能拦住反例？ | **拦不住**（355 ms < 500 ms）。真正的守门人是 GC 2 的**结构断言**（≤200 行/事务），见 §3 | §3 |
| 要不要上 WAL + 独立连接？ | **不建议现在做**（收益已被分批短事务吸收，代价是全库语义变更）。将来若出现写放大来源再单独立项 | §4 / §5 |
| 有没有该顺手修的同域缺陷？ | 有 3 条**只登记、不在本任务实施**：W-05（清理失败静默降级）、W-52（immutable 回退无探针）、W-53（DAO 的 `let _ =`） | §1 / §6 |

---

## 1. 现状（代码事实，逐条已读源码核实）

### 1.1 `mam.db` 只有一把锁、一个连接、无 WAL、无 `busy_timeout`

- `src-tauri/src/database/connection.rs:21` 全局单连接 `pub static DB: Lazy<Mutex<Connection>>`，
  初始化时 `Connection::open(<MAM_HOME>/.mam/mam.db)` 后直接 `schema::init`；
- **全仓 `src/database/` 内没有任何 `journal_mode` / `busy_timeout` 语句**
  （grep 只命中 `migration.rs` 的 `PRAGMA table_info(...)` 形参探测，与日志模式无关）
  → `mam.db` 是默认的 **rollback journal** 模式、默认 `busy_timeout = 0`；
- 所有读写（3 秒轮询、托盘、宠物、远程、用量账本、用量查询）**共用这一把进程内 `Mutex` 与这一个连接**。
  写事务在提交前持有 `Mutex` → 同进程内的任何其它读者都要排队等锁。

> 注：`src/monitor/sqlite.rs:18 / :34` 的 `busy_timeout(1000)` 是**工具私有库**
> （opencode.db / workbuddy.db / zcode.db）只读连接的设置，与 `mam.db` **无关**——
> 不要把它读成「mam.db 已有 busy_timeout」。

### 1.2 用量落库已经用「分批短事务 + 批间让出」压住锁窗口（GC 2）

- `src/services/usage/ledger.rs:23` `LEDGER_BATCH_ROWS = 200`、`:25` `LEDGER_YIELD_MS = 1`；
- `ledger.rs:86-113` `apply_delta`：逐批 `DB.lock()` → 一个短事务 → 作用域结束即 **drop 锁** →
  `sleep(1ms)` 让出，再进下一批；
- `ledger.rs:150-161` `validate_batches` 在**写之前**硬校验「单批 ≤ 200 行」，超限整轮 `Err`
  （游标不推进、下一轮重试）；
- `ledger.rs:163-177` `write_one_batch` + D-16 的行数比对：四类行写入行数与计划行数不符 → 该批回滚；
- 因此**单次取锁窗口的上界 = 一个 ≤200 行事务的时长**，而不是「一轮采集」的时长。

### 1.3 查询层全程持全局锁（Task 18 的 M-2，本轮读码复核）

`src/services/usage/query.rs:408-418` 的 `dashboard()`（`DB.lock()` 在 `:413`）是
`if let Ok(conn) = DB.lock() { dashboard_with_conn(&conn, …) }` —— **整个看板计算都在锁内**。
其中对 `usage_session` 做**全表读**（`dao::usage::load_sessions_conn(conn, None)`，`dao/usage.rs:661`）的调用点：

| # | 调用点（当前行号） | 何时发生 |
|---|---|---|
| 1 | `query.rs:199`（`key_label_dict`，被当期取数 `load_rows_for_keys` 调用） | 每次 dashboard **必然** |
| 2 | `query.rs:217`（`session_subagent_flags`） | 每次 dashboard **必然** |
| 3 | `query.rs:817`（`recent_session_with`） | 每次 dashboard **必然** |
| 4 | `query.rs:199`（同一函数，被**上一周期**取数再次调用） | 仅当有环比周期时 |

即：**读取点 3 处、单次 dashboard 实际读 4 次**（M-2 记的 4 次在数量上成立；
`query.rs:376/378/443/511` 是修轮前的旧行号，现为 `:199/:217/:817`）。
`usage_session` 的行量级 = 「每个源每个会话一行」（台账 Task 12 登记：codex 一源 **425** 个 rollout
文件 → 该源数百行），且**全程不许写库插队**——这是「查询侧」在 WAL 议题上的真实成本面。

### 1.4 保留期清理（W-04 / W-05）

- `ledger.rs:375-381` `purge_expired`：取全局 `DB` 锁后执行
  `dao::usage::purge_detail_before_conn`（`dao/usage.rs:697-707` 的**一条无上限 `DELETE`**），
  没有分批、没有批间让出 → 它的锁窗口随「积压行数」线性增长（本任务已把它纳入测量，见 §2.4）；
- **W-05（静默降级，登记不改）**：该 DAO 在 `DELETE` 失败时只 `log::warn!` 后 `return 0`，
  `purge_expired` 于是返回 `Ok(0)` —— 与「本来就没有行可清」**完全不可区分**，保留期不变量会静默失效。
  本任务用触发器造了真实删失败，把它变成**可执行事实**（见 §2.4 的 `[measure-extra]` 与测试用例
  `purge_expired_lock_window_and_silent_degradation`）。**收紧它要让 DAO 返回 `Result` 或失败标志
  → 动冻结形状，属另一轮裁决，本任务不实施。**

### 1.5 W-52：`open_usage_db` 的 immutable 回退分支**没有** `probe_readable`

`src/monitor/sqlite.rs:47-51`：

```rust
pub fn open_usage_db(path: &Path) -> Option<Connection> {
    open_readonly_with_timeout(path)
        .filter(probe_readable)                 // ← ro 分支有探针
        .or_else(|| open_readonly_immutable(path))  // ← immutable 回退**没有**探针
}
```

后果（Task 20 修复轮实测）：把**非 SQLite 内容**写进 `<temp>/.workbuddy/workbuddy.db` 时，
`open_readonly_immutable` 会把 NOTADB 文件**惰性**开成 `Some(conn)`，到第一句 SQL
（`load_sessions` 的 `prepare`）才炸。
**行为判定：响亮失败**（`ok=false` + `usage-source-io`），不是静默少算 ⇒ **不算缺陷**。
**语义张力**：契约里 `usage-source-db-open` 的定义是「源 SQLite **打不开**（含 immutable 回退失败）」，
而这条路径报的是 `usage-source-io`。若要判成 db-open，需在回退分支后**也**探一次
——**那是接口/口径决定，属 ①→② 复核，不在执行侧自行改**（登记）。

### 1.6 W-53：`dao/settings.rs` 的 `let _ =` 仍在

`src/database/dao/settings.rs:12-32`（`let _ =` 在 `:28`）：

```rust
pub fn set_setting(key: &str, value: &str) {
    let conn = DB.lock().unwrap();
    set_setting_conn(&conn, key, value);
}
pub(crate) fn set_setting_conn(conn: &Connection, key: &str, value: &str) {
    let _ = conn.execute("INSERT OR REPLACE INTO settings …");   // ← 写失败被吞
}
```

Task 20 的裁决 F 只修了**用量设置**那一侧（`services/usage/settings.rs::save` 返回 `Result` +
写后回读，命令层以既有码 `usage-db-failed` 报错）；**DAO 这一层的 `let _ =` 原样未动** →
其它走 `set_setting` 的功能（`ui_theme` 等）**仍可能静默丢写**。
**根治需 DAO 返回 `Result`（波及多调用方）→ 属另一轮裁决，登记。**

### 1.7 两条与「要不要上独立连接」直接相关的既有结论（Task 19 / 20）

- **采集只在后台线程、且只有 `usage_collect` 一条入口**（GC 3）：
  `collect.rs:475-482` 的 `spawn_initial_collection()` 起线程 → `sleep(3s)` → `collect(false)`；
  IPC 侧走 `spawn_blocking`。**3 秒会话轮询（`get_all_sessions`）零采集**
  （Task 20 / W-22 的可执行断言 + 本任务 `session_poll_never_triggers_collection` 的字面 0 断言）。
  ⇒ 冲突面**只有**「前台读（轮询 / 看板）vs 后台写（采集落库）」这一对。
- **锁序铁律（W-26）**：持 `DB` 锁时**只能**调 `collect::cached_result()` / `collect_calls()`
  （二者不碰飞行锁），**绝不能**调 `collect()` / `collect_with()` / `run_collection()`
  —— 否则是 `COLLECT_FLIGHT → DB` 与 `DB → cached_result()` 的 ABBA
  （`std::sync::Mutex` 不可重入、无超时 → 整个 app 的 DB 访问一起挂死）。
  **本任务把这条变成机械锁**：`tests/usage_concurrency_test.rs` 的
  `this_file_never_calls_collection_entries` 用 `concat!` 拼串自省（防自指假绿），
  变异实测（插一行 `collect::collect(false)`）→ **真红**。
  ⇒ **若将来上独立连接，这条锁序必须重审**：连接从「一把」变「两把」之后，
  ABBA 的判据也跟着变，不能只把 `DB.lock()` 换成 `DB2.lock()` 了事。
- 导出侧（Task 19/21）：`usage_export_csv` 的 CSV 构建仍在 `DB` 锁内（`query.rs:709-717`），
  落盘（Task 21 的 `export_save_text` / `export_save_bytes`）**只写文件系统、不碰 `mam.db`**
  → 对连接层零影响，不改变本评估的结论。

---

## 2. 实测数据（本机，测试方法可复现）

### 2.1 表格（数字**照抄**测试输出，不手算、不估计）

| 场景 | 单批 200 行事务耗时 | 轮询单次取锁最长等待 |
|---|---|---|
| **现状**（分批短事务，rollback journal） | **15 ms**（上限；本轮 23 批、最大批 189 行） | **13 ms** |
| **假设：一个事务写 4,000 行（反例）** | **358 ms**（该大事务自身持锁上限，含 `commit`） | **355 ms** |

> ⚠️ **两行的取样性质不同，且两侧都随机器负载浮动（Minor 5，fix round 1 补注，不得省略）**：
> * **现状**：空载 5 次复跑 = 12.9–18.9 ms（写库侧单批持锁 15–21 ms）；本机同时有另一个窗口
>   在编译/跑测试时另 4 次 = **21 / 26.5 / 50 / 127 ms**（写库侧 23–130 ms）。
> * **反例**：**单次取样**，本机 4 次观测 = 轮询最长等待 **355 / 367 / 979 / 2440 ms**
>   （大事务持锁 **358 / 369 / 627 / 1469 ms**）；**评审独立复跑同 binary 得 1590 ms**。
> * ⇒ **倍率不可当结论**（同负载下 355/127 ≈ 2.8×，跨负载 2440/13 ≈ 188×）。
>   **可依赖的是结构事实**：现状的单次锁窗口 = **一个 ≤200 行事务**（本机 15–130 ms），
>   反例 = **整整一轮 4,000 行**（358–1469 ms）——行数差 20×，观测差与它同量级；
>   而现状到 500 ms 上界仍留 **≥3.9×** 余量（最坏观测 127 ms，上界断言在本机从未接近）。

**口径（W-50：跨行/跨场景口径必须写明，否则数字不可比）**：

1. 「单批事务耗时」= 写库侧测得的 **锁窗口上限**：从 `DB.lock()` **取到锁之后**到 `commit`
   之后（反例）／从取锁前到 guard drop 之后（现状的 `stats.max_hold_ms`，含取锁等待，
   但本轮测试是串行上下文，两者近似相等）。**不含**批间 `sleep(1ms)` 让出。
2. 「轮询单次取锁最长等待」= 200 次 `database::get_tool_enabled("claude")`（取同一把全局 `DB` 锁
   的真实读路径）里最慢的一次，含取锁 + 一次极小查询；采样间隔 ≈3 ms，模拟前端 3 秒轮询的取锁行为。
3. 夹具规模：现状 = 每轮 2,000 条明细（64 个会话行键）；反例 = 一个事务 4,000 条明细。
   **这不是真机 codex 全量（226,035 行）的量级**，绝对数字只用于**同机同比**（见 §2.5 局限）。
4. 构建形态：`cargo test`（debug）、临时目录里的 `mam.db`（`support::setup()` 重定向 HOME/MAM_HOME）。
5. 单位：毫秒（`as_millis()` 截断）。µs 级原始值见 §2.2 的 `[measure-extra]`。

### 2.2 数据来源（原始输出照抄）

```bash
# 现状两行（测试内 println! 输出）
cd src-tauri && cargo test --test usage_concurrency_test -- --test-threads=1 --nocapture
#   [measure] 现状（分批短事务）: 单批 200 行事务耗时上限 = 15 ms | 批数 = 23 | 最大批行数 = 189
#   [measure] 现状（分批短事务）: 轮询单次取锁最长等待 = 13 ms
# 反例行
cd src-tauri && cargo test --test usage_concurrency_test -- --ignored --nocapture counterexample
#   [measure] 反例（一个事务写 4,000 行）: 轮询单次取锁最长等待 = 355 ms
```

**现状（原文照抄，2026-10-03 本机）**：

```
running 7 tests
test counterexample_single_transaction_blocks_polling ... ignored, 反例测量：故意制造秒级锁持有，只在填评估表时手动跑
test every_test_in_this_file_takes_the_serial_lock ... ok
test ledger_transactions_are_bounded ... [measure] 现状（分批短事务）: 单批 200 行事务耗时上限 = 15 ms | 批数 = 23 | 最大批行数 = 189
ok
test polling_lock_wait_stays_bounded_while_writing ... [measure] 现状（分批短事务）: 轮询单次取锁最长等待 = 13 ms
[measure-extra] 现状: 轮询窗口内写库完成轮数 = 16 | 失败轮数 = 0 | 累计计划行数 = 34017 | 每轮批数 = 12 | 写库侧单批持锁上限 = 16 ms | try_lock 命中「别人正持锁」= 104/200 | 轮询最长等待 = 13705 µs
ok
test purge_expired_lock_window_and_silent_degradation ... ignored, 清理路径的锁窗口测量（W-04）+ 静默降级特征化（W-05）：只在填评估表时手动跑
test session_poll_never_triggers_collection ... ok
test this_file_never_calls_collection_entries ... ok

test result: ok. 5 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 3.49s
```

**反例（原文照抄）**：

```
running 1 test
test counterexample_single_transaction_blocks_polling has been running for over 60 seconds
[measure] 反例（一个事务写 4,000 行）: 轮询单次取锁最长等待 = 355 ms
[measure-extra] 反例: 大事务自身持锁上限 = 358 ms（含 commit，不含取锁等待）| 轮数 = 200（轮询窗口内 199）| try_lock 命中「别人正持锁」= 193/200 | 单事务执行行数 × 轮数 = 800000 | 库内 SUM(input_fresh) 增量 = 800000
test counterexample_single_transaction_blocks_polling ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 64.99s
```

**结论性对比**：现状 **13 ms** vs 反例 **355 ms** = **27×**（反例是**单次取样**；把至今所有有效观测
列全后区间是 355–2440 ms，**27× 只是最小观测倍数**，见 §2.3）。任何一次观测里反例都比现状
高 **1–2 个数量级** →「分批短事务」确实是当前满足验收的**充分**手段；但**不要引用固定倍率**。

### 2.3 复跑稳定性与观测波动

**现状侧（空载 5 连跑 + 负载下 4 次补测）**：

| 量 | 空载 5 连跑 | 负载下 4 次补测 |
|---|---|---|
| 轮询单次取锁最长等待（现状） | 12.9 – 18.9 ms | 21 / 26.5 / 50 / 127 ms |
| 写库侧单批持锁上限（现状） | 15 – 21 ms | 23 / 29 / 54 / 130 ms |
| `try_lock` 命中「别人正持锁」 | 102 – 140 / 200 | 159 – 179 / 200 |
| 轮询窗口内写库完成轮数 | 恒定 16 | 恒定 16 |

**没有观察到 flake**；`try_lock` 命中的量级（≈50% 占空比）说明并发**真的**发生，
而不是「什么都没发生时查询不慢」（见 §3 的前提断言设计）。

**反例侧（Minor 5，fix round 1）：从来没有 5 连跑，是单次取样，且明显随负载浮动**。
把至今**所有有效观测**列全（口径：同一 binary、同一命令、同一台机器；差异来自机器负载——
本任务执行期间这台 checkout 上还有另一个窗口在编译 / 跑测试）：

| 观测 | 轮询单次取锁最长等待 | 大事务自身持锁上限 |
|---|---|---|
| 本任务第 1 次 | 367 ms | 369 ms |
| 本任务 canonical（§2.2 照抄的那一次） | 355 ms | 358 ms |
| 本任务第 4 次 | 979 ms | 627 ms |
| 本任务第 5 次 | **2440 ms** | **1469 ms** |
| **评审独立复跑**（fix round 1 反馈） | **1590 ms** | （未提供） |

→ **倍率不可用**（受负载影响：355/127 ≈ 2.8× 到 2440/13 ≈ 188× 都出现过），
但**结论不受影响**，因为结论**不建立在倍率上**：
现状的单次锁窗口被 **GC 2 的 ≤200 行/事务**结构性钉死（`validate_batches` + 行数断言），
反例则是「一整轮一个大事务」——**这是设计差异，不是测量差异**；
实测只是它的量级投影（现状 15–130 ms vs 反例 358–1469 ms，与 20× 的行数差同量级）。
（已作废的观测：M7 变异那次的 `1 ms` 用的是**私有连接**、不再持全局锁，不是本场景的样本。）
（口径提醒：本机在执行本任务期间**同时**有另一个窗口在编译/跑测试，「空载」与「负载」两组数据
来自不同时段，**不可互相做倍率比较**。）

### 2.4 顺带量出来的两条（W-04 与 W-39）

**W-04（清理路径的锁窗口，Task 22 被点名的动作）**：

```
[measure] purge_expired（单条无上限 DELETE，删除 5000 行）: 锁窗口 = 13 ms
[measure-extra] W-05: 撤掉阻塞后同一条 DELETE 删掉 2000 行（同一个 0 的两种含义在此分叉）
```

- 口径：`ledger::purge_expired(7, now_ms())` 的**整段墙钟**（取锁 + 一条 `DELETE`），
  删除 **5,000** 行（每行一个独立行键）；**不是**「大积压」场景。
- 同一条路径在**删失败**时（`BEFORE DELETE` 触发器 `RAISE(ABORT)`）返回 **`Ok(0)`**，
  而库里 2,000 行一行未少；撤掉触发器后同一条 `DELETE` 又把它们全删掉
  → **W-05「失败 ≡ 无行可清」是可执行事实，不是推测**。

**W-39（跨小时工具调用的 `(0, ms>0)` 条目）**：

1. **真机账本查询（只读）** —— `sqlite3 -readonly ~/.mam/mam.db`：

```
usage_detail 总行数|0
usage_detail 中 tool_calls=0 且 tool_ms>0 的行|0
usage_cursor 行数|0
usage_daily 行数|0
usage_session 行数|0
```

   → **真机账本当前 0 行**（与 Task 18 的登记一致：真正的采集要等应用启动后才会写），
     因此**无法在真实采集数据上确认**该形态是否存在。

2. **库层构造确认（临时探针，跑完即删）** —— 用 `support::open_ledger_db("w39")` 私有库，
   按 Task 17 登记的跨小时语义写两轮（计数落 09 时、耗时落 10 时），再查库：

```
[w39] 两轮落库行数 = 2/2
[w39] 库里: hour=2026-10-03T09 tool_calls=1 tool_ms=0 tool_stats={"Bash":[1,0]}
[w39] 库里: hour=2026-10-03T10 tool_calls=0 tool_ms=2500 tool_stats={"Bash":[0,2500]}
[w39] tool_calls=0 且 tool_ms>0 的明细行 = 1
[w39] tool_stats 里 (0, ms>0) 条目的行数 = 1
```

   → **形态在库层确实会出现**（`tool_calls=0 / tool_ms=2500 / tool_stats={"Bash":[0,2500]}`），
     且 DAO 的同键累加救不了跨键（两行主键不同）。触发条件已在 Task 17 登记
     （真机单工具最长 3.1h ⇒ 必然跨小时）；**消除它需要「跨小时 A/B 行合并」= 口径变更 → ①→② 复核**。

### 2.5 局限（不得把「推理上应该没问题」写成「实测没问题」）

- 上述数字来自 **4,000 行量级的夹具 + debug 构建 + 临时目录库**。真机 codex 单轮是
  226,035 行 / 2.21 GB 的量级，**单批 200 行的时长在真机上只会更长**（慢盘、库更大）；
  本表**不能**当作「真机轮询等待就是 13 ms」来引用，只能当作**同机同构建下的量级对比**。
- 真机全量采集的并发实测**未做**（需要跑 1–3 分钟的 codex 全量扫描），属 Task 23B/24 的范围。

---

## 3. 阈值灵敏度与回归锁（诚实披露：500 ms 拦不住反例）

本任务的可执行验收在 `src-tauri/tests/usage_concurrency_test.rs`：共 **8 个 `#[test]`** =
任务书点名的 3 条 + 3 条自省/静态锁（进日常门禁）+ 2 条 `#[ignore]` 测量。三条硬约束的**真实**守门人如下：

| 判据 | 断言 | 真能变红的变异（已实跑，原始输出见 Task 22 报告 §5/§12） |
|---|---|---|
| 单事务 ≤ 200 行 | `max_batch_rows <= LEDGER_BATCH_ROWS` + `batches >= 20` + **常量钉住 `== 200`** | M2b（合并批次 + 停掉 `validate_batches`）→ `单事务行数 4000 超过上限 200` **RED**；M2a（把常量改成 100_000）→ 常量钉住断言 **RED** |
| 轮询上界 < 500 ms | `worst_ms < 500`（附 **9** 条前提断言） | M1（写库侧持锁多睡 700 ms）→ `轮询单次取锁最长等待 728ms` **RED**（数字照抄 `task-22-report.md` §5「M1」原始输出里的 `728957 µs`） |
| 采集不进轮询 | `collect_calls()` **字面上的 0**（扫描前后各断一次） | M3（往 `adapter::get_all_sessions` 插一行 `collect(false)`）→ `left: 1 / right: 0` **RED** |
| 采集不进轮询（**L1 逃逸面**） | `adapter_layer_never_references_collection_entries`：`src/adapter/**/*.rs` 静态扫采集入口 | M10（往**本机无进程**的 `kimi.rs` 插一行 `collect(false)`，L1 会跳过它）→ 静态锁 **RED**，而**同一次变异下行为式用例仍绿**（M10b，逃逸面当场复现）；对照 M10c（同样的变异放到**本机有进程**的 `claude.rs`）→ 行为式用例能抓到 |
| 前提：并发真的发生 | `DB.try_lock()` 的 `WouldBlock` 计数 > 0；写库侧 `writer_hold_ms > 0` | M6（探针改回紧贴释放锁之后）→ `0/200` **RED**；M7（反例改用私有连接）→ `0/200` **RED**；M8（`writer_hold_ms` 恒置 0）→ **RED** |
| 测试隔离 | 每条用例持 `serial()`（逐行 exact match 自省）+ **`serial()` 内部先取锁再改 env** | M4（摘掉一条 `serial()`）→ **RED**；M9（锁挪回函数末尾）→ **RED** |
| W-26 锁序 | 本文件**绝不出现**采集入口（`concat!` 拼串防自指假绿） | M5（插一行 `collect::collect(false)`）→ **RED** |

### 3.1 ⚠️ 必须如实说明的两条**灵敏度边界**

1. **500 ms 阈值拦不住本任务的反例**：反例（4,000 行单事务）只把轮询挡到 **355 ms**
   （< 500 ms）。也就是说，**如果有人把 `apply_delta` 的批次合并成一个大事务，
   「轮询上界」这条断言在本夹具规模下不会变红**。
   真正拦住它的是 **GC 2 的结构断言**：`validate_batches` 会让整轮 `Err`
   （测试 1 的 `.expect("落库必须成功")` 直接 panic），且 `max_batch_rows <= 200` + 常量钉住另有一道。
   → **500 ms 是「上界」，不是「反例检测器」；不要把它的通过读成「分批一定还在」。**
2. **自指式假绿（已被常量钉住断言堵住）**：所有行数判据都以 `LEDGER_BATCH_ROWS` 为基准，
   把常量从 200 改成 100_000 后**全部断言仍会全绿**（实跑 M2a′：把常量钉住断言也摘掉 → `ok`，
   而打印里已经是 `批数 = 2 | 最大批行数 = 4000`）。本任务因此**新增了**
   `assert_eq!(LEDGER_BATCH_ROWS, 200)` 与 `assert_eq!(LEDGER_YIELD_MS, 1)` 两条钉住断言。

### 3.2 前提断言（为什么本用例不是「串行跑两个函数」的假绿）

- 写库跑在**独立线程**的**生产入口**（`ledger::apply_delta`），轮询在主线程**同时**取锁计时；
- `DB.try_lock()` 返回 `WouldBlock` ⟺ 此刻有人**正持** DB 锁 —— 这是 Mutex 语义下**唯一**
  能直接观测「别人正在写」的方式。**探针采样时刻有讲究**：第一版紧贴轮询自己释放锁之后采样，
  实测 **`0/200` 命中（而同一轮轮询最长等待 19 ms、写库持锁上限 22 ms）** →
  那一刻锁刚被轮询放掉、写库线程还没被唤醒，采到的永远是「没人持锁」。
  改为「先让出 1 ms 再采样」后稳定在 **102–140/200**。这条教训（**假绿的计时用例**）已写进代码注释。
- 另有 **5 条**前提断言保证「写库真的在写」：`rounds_err == 0`、轮询窗口内完成轮数 ≥ 3、
  每轮落 ≥2,000 行、每轮 ≥10 批、**写库侧单批持锁上限 > 0**（Minor 4，fix round 1 补）。
  加上常量钉住、单批 ≤200 行、`try_lock` 命中与 `collect_calls() == 0`，该块共 **9 条前提断言**；
  落库失败 / 写库没占住锁窗口 / 两边没并发起来时，这些前提会**先**红。

---

## 4. 三个候选方案与代价

1. **维持现状（分批短事务 + 批间让出）**：零风险；代价是大批量首次采集时耗时略长
   （每批之间 1 ms 让出；一轮 20 万行 ≈ 1,000 批 ≈ 1 s 的让出开销，相对 codex 全量扫描
   1–3 分钟的耗时可以忽略）。代价面：**单批 ≤200 行**意味着**长事务不可能**——
   任何「要么整轮成功、要么整轮回滚」的新需求都会被这条挡住（现状：批级提交，
   失败时前几批已落盘 → 见 W-25 的重复计入风险，那是账本层设计缺口，与 WAL 无关）。
2. **仅加 `busy_timeout`**：对**单连接 `Mutex`** 模型几乎无收益——锁在**进程内**，
   竞争者排队的是 Rust 的 `std::sync::Mutex`，不是 SQLite 的忙等（`busy_timeout` 只管
   **跨连接/跨进程**的 SQLITE_BUSY）。风险低，收益也低。
   （真正的 `busy_timeout` 场景在**只读工具库**那一侧，`monitor/sqlite.rs` 已设 1000 ms。）
3. **独立连接 + WAL**：读不阻塞写、写不阻塞读；**但**：
   ① 需要为用量域开第二条连接与连接池策略（谁持哪把锁、跨域事务怎么办）；
   ② WAL 会在 `~/.mam/` 产生 `-wal` / `-shm` 文件 → **备份/清理/杀进程残留语义全变**
   （`mam.db` 的复制备份缺 `-wal` 会丢最近提交；进程被 kill 后 `-shm` 残留；用户手册/发布脚本要同步）；
   ③ 与既有「全局单连接」假设冲突的代码点需要逐个审计（事务边界、`PRAGMA`、`VACUUM`、
   任何依赖 `Connection` 唯一性的地方）——影响面**远超用量功能本身**；
   ④ 独立连接会**绕过**那把 `Mutex`，于是「写库不阻塞轮询」变成「两把锁 + SQLite 层并发」，
   需要重新证明（本次的分批让出结论**不能直接平移**）。

---

## 5. 建议

**维持方案 1**（分批短事务）。若将来出现新的写放大来源（如逐请求实时记账），再单独评估方案 3。

补充两条**不建议在本阶段做**的事（附理由）：

- **不要**为了「让轮询更快」去减小 `LEDGER_BATCH_ROWS`：它换来的是**事务数**上升与
  批间让出总时长线性增长，而现状 13 ms 的等待已经比 3 秒轮询周期小两个数量级；
- **不要**动 `purge_expired` 的锁窗口（13 ms / 5,000 行）除非先解决 W-05：
  在「失败被吞成 0 行」还存在的条件下给它加 `LIMIT` 分批，只会让**每批**都可能静默失败。

---

## 6. 提请裁决

请用户确认：

- [ ] **维持分批短事务（推荐）** —— 不实施 WAL / `busy_timeout` / 独立连接（GC 2 维持现状）
- [ ] **另开任务实施 WAL + 独立连接** —— 需先定义：连接归属与锁序、`-wal/-shm` 的备份与清理语义、
      受影响代码点审计清单、以及「绕过全局 `Mutex` 之后如何重新证明不阻塞轮询」

一并请裁（都与本文件的评估直接相关，但**不属本任务范围**）：

- [ ] W-05：是否让 `purge_expired` / DAO 把「删除失败」与「无行可清」区分开（需动 DAO 签名或形状）
- [ ] W-52：immutable 回退后是否补一次 `probe_readable`，并把该路径的码从 `usage-source-io`
      改成契约里的 `usage-source-db-open`
- [ ] W-53：是否让 `dao/settings.rs::set_setting_conn` 返回 `Result`（波及 `ui_theme` 等调用方）
- [ ] W-39：跨小时的 `(0, ms>0)` 工具条目是否需要「A/B 行合并」（口径变更，需回写契约）

---

## 7. 台账既有实测输入（**非本次复测**，引用自 `.superpowers/sdd/…/KNOWN-PLAN-DEFECTS.md`）

> 口径声明（W-50）：下表是**其它任务在真机上已取得**的数字，本任务只做了**读码核对**
> 与来源标注，**没有复测**；请勿把它们当成 Task 22 的实测产出。

| 条目 | 已收集的实测输入 | 来源 | 本任务动作 |
|---|---|---|---|
| **W-16** | codex 真机**峰值 RSS 685 MiB**（最大单文件 250.9 MB）；成因是冻结接口 `IncrementalRead { lines: Vec<String> }` 把整文件物化 | Task 12 真机回归 | 未复测（需跑真机 codex 全量 1–3 分钟）；登记：若上 WAL/独立连接，**内存峰值与连接数无关**，这条不会被方案 3 缓解 |
| **W-47** | `probe_readable` 探针 SQL 在 **762 MB** zcode 库上**中位 4 µs**；`mode=ro` 打开 1.853 ms、`immutable` 打开 0.098 ms、`open_usage_db` 0.982 ms | Task 16 实测 | 未复测（本任务临时 home 内无 762 MB 库）；登记：探针成本可忽略，W-52 若补探针**不必担心开销** |
| **Task 18 M-2** | 单次 dashboard **4 次全表读 `usage_session`** 且**全程持全局 `DB` 锁** | Task 18 登记 | **读码复核成立**（`query.rs:199/:217/:817`；读取点 3 处、含环比时调用 4 次，见 §1.3）；真机量级未测（账本 0 行） |
| **W-38** | `turn_seen` 上限 **1024** 的 `state_json` 体积 | Task 12 登记 | 真机 `usage_cursor` **0 行 → 无实测**；给出**结构性上界估算**（口径见下）：真机 codex 的 `turn_id` / `session_id` 抽样均为 **36 字符 UUID**（样本：`~/.codex/sessions/2026/07/22/rollout-…jsonl`，`"turn_id":"019f7511-f16c-7233-b372-da272303b4ef"`）→ 每对 ≈ `["<36>","<36>"]` = **80 B**，1024 对 ≈ **80 KB/游标行**；425 个 rollout 文件即便全部打满 ≈ **34 MB**（上界，**估算非实测**） |
| **W-39** | 跨小时工具调用会产出 `(0, ms>0)` 条目 | Task 17 登记 | **已按本条要求查库**：真机账本 0 行（不可确认）+ 库层构造确认形态真实存在（原始输出见 §2.4） |
| **W-52** | `open_usage_db` 的 immutable 回退分支没有 `probe_readable` | Task 20 修复轮实测 | **读码核实**（`monitor/sqlite.rs:47-51`，见 §1.5）；行为判定「响亮失败、非缺陷」，语义张力待裁 |
| **W-53** | `dao/settings.rs` 的 `let _ =` 仍在 | Task 20 登记 | **读码核实**（`dao/settings.rs:27-32` 的 `let _ =`，见 §1.6）；根治属另一轮裁决 |
| **W-04 / W-05** | 清理路径：单条无上限 `DELETE` 在锁内；删失败静默返回 0 | Task 4 评审提出，动作点名 Task 22 | **本任务已测量并入表**（13 ms / 5,000 行）+ 触发器造真实失败的可执行事实（§2.4） |

---

## 8. 未测项清单（**单列，不得当成已测**）

| # | 未测项 | 原因 |
|---|---|---|
| 1 | 真机 codex 全量（226,035 行）单轮采集期间的轮询等待 | 需 1–3 分钟全量扫描，属 Task 23B/24；本任务只用 4,000 行夹具做同机同比 |
| 2 | 真机 `usage_cursor.state_json` 的实际字节数（W-38） | 真机账本 4 表 0 行；只有结构性上界估算 |
| 3 | `purge_expired` 在**大积压**（数十万行）下的锁窗口 | 本任务只造了 5,000 行；13 ms 不能外推到 20 万行（DELETE 成本随行数近似线性） |
| 4 | dashboard 的 4 次全表读在真机 `usage_session` 行数下的耗时 | 账本 0 行，测不出有意义数字；只有代码事实（§1.3） |
| 5 | WAL 方案的效果对比（读不阻塞写） | **本任务不实施 WAL**（GC 2），无对照可测 |
| 6 | W-16 的内存峰值复现 | 需真机 codex 全量；引用 Task 12 数字 |
| 7 | ~~W-14 的时区分叉告警（Task 12 登记）~~ **已随 A-1 作废（2026-10-05）** | **本未测项已无所指**：分叉本身与那个 `log::warn!`（`warn_tz_divergence_once`）**均已被 Q-4 裁决删除**（spec §P6「一律宿主本地」；A-1 落地，全仓 grep 0 命中）⇒ 不再登记。**现行性质级保证** = `src-tauri/src/services/usage/collectors/codex.rs:1312` 的 26 小时差**跨时区行为锁**（`detail_hour_key_is_host_local_even_when_source_timezone_is_present`） |
