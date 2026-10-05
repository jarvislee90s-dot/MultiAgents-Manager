//! dsh 采集器（**投影缓存 + 原始 zstd 日志**双数据源，说明书 §5.1 / §9.5 / 矩阵 §6.2）。
//!
//! * **投影缓存** `storages/session_projcache/sessions/*.json`（真机 170 文件，四桶 **170/170 恒存在**）
//!   → 四桶 / turns / steps / toolMs / 模型 / 供应商 / 子代理分层。**会话级累计值 → 取差值**；
//! * **原始会话日志** `sessions/<proj>/<dir>/session[.vN].jsonl.zstd`（多帧 zstd，复用既有
//!   `monitor::dsh::{log,decode}` 解码器）→ 报错三层 / 用户打断单列 / 工具调用次数 / **turn 时长样本**
//!   （最长单 turn 由样本算出）—— 投影缓存拿不到这几类，且真机只有一部分会话留有原始日志
//!   （矩阵口径 19/99；本机复核 172 个会话目录中 171 个可解码），不可得的部分按空态，**绝不填 0**。
//!   **逐工具**维度（`tool_stats`：每个工具的次数与耗时）**在同一次扫描里产出**——
//!   配对键是 `tool/call.data.callId` ↔ **`tool/result.data.message.source.callId`**
//!   （真机 173 文件全量 14,584/14,584，配对率 100.0%；**不是**任务书写的顶层
//!   `data.toolCallId`，真机 0 条 —— 见 `result_call_id`），配不上的只计次数、不计耗时；
//! * `val.last == null`（真机 10/170）＝本源 TotalOnly 等价情形 → 只用 totals，不参与逐轮；
//! * 四桶**无 reasoningTokens**（dsh 特有）；字段名 `uncachedInputTokens` 即互斥 → 声明 `Exclusive`。
//!
//! ## W-30：`is_subagent` **按文件自身身份**判定（不得走记录级 OR）
//! dsh 的父/子会话**各成一个投影缓存文件**（矩阵 §3.5），子代理身份就写在文件自己的
//! `record.rows.subagent.val.identity` 上。真机 170 文件实测三种形态：
//! `val.identity` 非空（113 个**本文件就是子代理会话**）、`val = {}`（57 个父会话：**行在场**、
//! 值为空对象）、以及旧快照连该行都没有（缺行 = 父）。**只有第一种**才是子代理会话；
//! 「行在场」不是判据，「文件里出现过侧链」更不是（claude 那次 `|=` 把用过子代理的父会话
//! 整条判成子代理 → 唯一消费者按该位硬过滤 → 父会话连会话数带 turn 数一起消失）。
//! 锁：`subagent_identity_comes_from_the_file_not_from_the_row_presence`（混合夹具：
//! 真机父形态 + 真机子形态同场，父必须 `false`、子必须 `true`）。
//!
//! ## GC 4 / W-15：文件读取只经 `CollectContext::read_incremental`
//! 投影缓存是**整份重写**的 JSON 文档（不是只追加的日志），所以它的字节水位没有意义：
//! 每份游标一律把 `byte_offset` 置 0 → 文件代际一变就从 0 全量读回整份文档（正是「一遍扫描」），
//! 而 `(mtime, size)` 未变时仍走 L2 快速路（零读取）。**不得**改成真实 byte_offset：
//! 同尺寸重写/尾部追加会让 append 路只回吐半份 JSON（静默少算）。
//! 断言：`stats.reads` / `stats.bytes_read` / `stats.repeat_reads`（W-15：只断言「值对了」
//! 不等于「路径对了」）。
//!
//! **继承的盲区（如实登记，不得静默）**：`read_incremental` 的快速路只比 `(mtime_ms, size)`，
//! 因此「**同一毫秒内 + 同一大小**」的重写会被判「未变」而整轮跳过 —— 这是 stream/cursor 层的
//! **已登记盲区**（`cursor.rs` 模块文档 + `stream.rs` 的同名锁），本采集器**继承**它。
//! 生产暴露面的量级：要漏掉一次重写，必须让「上一轮游标里记录的 mtime」与本轮的重写时刻落在
//! **同一毫秒**（= 采集读该文件的那一刻与前后两次写入全挤进同一毫秒）；采集轮次是分钟级间隔，
//! 概率 ~0。**夹具纪律**：不得在亚毫秒内做「同尺寸重写」（T16 第一版夹具即因此 flake）。
//! 锁：`same_millisecond_same_size_rewrite_is_the_inherited_fast_path_blind_spot`。
//!
//! **原始日志侧的读取不经过 `read_incremental`**（偏差申报）：zstd 是二进制负载，
//! 而 `read_incremental` 的产物是 `Vec<String>`（`from_utf8_lossy` 切行）——让它去读压缩帧
//! 只会把负载撕成 U+FFFD，且无 `\n` 的文件连一行都切不出来。该侧改用 GC 5 明令复用的
//! `SessionFileScan`（namespace = `scan_namespace(Dsh)` 派生 + `-log`）做 `(mtime,size)`
//! 内容摘要缓存：**同一代际只解码一次**（比「每轮一遍」更强），代际未变则零解码。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{file_read_failure_is_benign, DeltaBuilder};
use crate::monitor::session_scan::SessionFileScan;
use crate::services::usage::collect::{scan_namespace, CollectContext, UsageCollector};
// **不得**导入 `CursorDelta`：本文件从头到尾没有显式写它的类型名（构造游标时一路类型推断），
// 导进来就是 `unused_imports`，而门禁是 `-D warnings`（任务书 Step 3 的导入清单里带着它，
// 属 D-10 / D-25 同族的「导入清单与代码不自洽」；见报告「偏差申报」）。
use crate::services::usage::delta::{DetailKey, SourceDelta};
use crate::services::usage::error::UsageError;
use crate::services::usage::model::UsageSourceId;
use crate::services::usage::project::project_key_of;
use crate::services::usage::range::{hour_key_of, SourceTz};
use crate::services::usage::semantics::{default_policy, normalize, resolve_semantics, RawUsage};

/// 本源 `SessionFileScan` 的 namespace（GC 5 / **W-19**）：**必须**由 `scan_namespace(sid)`
/// 派生，不得手写字符串字面量——同 ns 混存别的产物类型会让 `Arc::downcast` 失配、
/// 缓存静默失效（每轮重新解析，正是 L2 要省掉的那笔开销）。
const SCAN_NAMESPACE: &str = scan_namespace(UsageSourceId::Dsh);

/// 原始日志侧的 L2 namespace = `scan_namespace(Dsh)` + `-log`。
/// 为什么要有独立后缀：投影缓存侧不建 `SessionFileScan`（它的 (mtime,size) 判定由
/// `read_incremental` 的游标负责），但**万一**将来有人给投影缓存也建一个，同 ns 混存
/// 两种产物类型就正上面那条静默失效。派生方式：`concat!` 不折叠 `const fn` 调用，
/// 故运行时拼一次、`OnceLock` 缓存（值恒定，之后零成本）。
fn log_scan_namespace() -> &'static str {
    static NS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NS.get_or_init(|| format!("{SCAN_NAMESPACE}-log")).as_str()
}

const TZ: SourceTz = SourceTz::HostLocal;

pub struct DshCollector;

/// 续读状态：投影缓存是**累计值**，必须记住上次快照才能算差值
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct DshState {
    total: (i64, i64, i64, i64),
    turns: i64,
    steps: i64,
    tool_ms: i64,
}

impl DshState {
    /// 「还没有任何累计快照」= 首轮（或游标里的状态不可解析）。
    /// 归桶策略据此分岔：首轮这一笔是**全部历史**（归 `createdAt`），之后是**增量**（归当轮）。
    fn is_empty(&self) -> bool {
        self.total == (0, 0, 0, 0) && self.turns == 0 && self.steps == 0 && self.tool_ms == 0
    }
}

impl UsageCollector for DshCollector {
    fn source_id(&self) -> UsageSourceId {
        UsageSourceId::Dsh
    }
    fn collect(&self, ctx: &CollectContext<'_>) -> Result<SourceDelta, UsageError> {
        DshCollector::collect_with_home(ctx, &crate::monitor::dsh::dsh_home_with(ctx.home))
    }
}

impl DshCollector {
    pub fn collect_with_home(
        ctx: &CollectContext<'_>,
        home: &std::path::Path,
    ) -> Result<SourceDelta, UsageError> {
        // **供应商规则从 ctx 注入**（§3.2.3 FIX-6）：lib 单测构建注入空规则 → 零 DB；
        // 生产/集成构建才回落到 settings::load()。采集器不得直连设置层。
        let rules = ctx.provider_rules();
        let mut b = DeltaBuilder::new(UsageSourceId::Dsh, rules);
        // ---- A. 投影缓存（四桶/turns/steps/toolMs/模型/供应商/子代理）----
        let dir = home.join("storages/session_projcache/sessions");
        // **不得**在投影缓存目录缺失时提前 `return`（任务书原文如此，会把 B 段一起短路）：
        // 原始日志可能仍在——B4 的夹具就是「只有日志、没有投影缓存」这种形态，真机也存在
        // 「日志在、投影缓存被清理/尚未落盘」的窗口。目录缺失只跳过 A 段。
        //
        // **M2（fix round 1）**：目录级读失败原来被 `.ok()` 静默吞掉。按 `ErrorKind` 区分
        //（D-24 的口径）：`NotFound` = 「未装 dsh / 缓存被清理」这一种可静默；
        // 其余（EACCES / EIO）是「这个源读不了」→ **整源响亮失败**（落 `errorCode`，别让用户
        // 以为是「这个源没有用量」）。
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => Some(e),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(UsageError::new(
                    "usage-source-io",
                    format!("dsh 投影缓存目录 {} 读取失败: {e}", dir.display()),
                ))
            }
        };
        // **M1（fix round 1）**：结构性缺 `modelSelection` 行的会话数（真机 63/171）。
        // 这些行按**空模型 + Unknown 供应商**落库（数据 > 分组整洁，见模块头与报告），
        // 但**不得静默**：本轮结束前给一条带数量的告警。
        let mut no_model_sessions = 0i64;
        // 「会话 → (模型, 供应商)」：投影缓存是唯一带这两个字段的一侧，
        // B 段（原始日志）靠它补位，免得报错/工具计数落进 `model=""` 的幽灵行
        let mut model_by_session: HashMap<String, (String, String)> = HashMap::new();
        for e in entries.into_iter().flatten().flatten() {
            let path = e.path();
            if path.extension().map(|x| x != "json").unwrap_or(true) {
                continue;
            }
            let session_id = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if session_id.is_empty() {
                continue;
            }
            // 游标键 = 文件 stem（目录扁平，`<sessionId>.json` 在源内唯一）
            let file_key = session_id.clone();
            let prev = ctx.cursor_of(&file_key).cloned();
            // **GC 4：文件读取只经唯一入口** `CollectContext::read_incremental`
            //（它同时统计 reads / bytes_read / repeat_reads，W-15 的断言就落在这里）
            let read = match ctx.read_incremental(&path, &file_key) {
                Ok(r) => r,
                Err(err) => {
                    // **W-06 / D-24 / D-38**：只有「枚举与读取之间的消失竞态」才允许跳过；
                    // 其余读失败（权限 / IO / mtime 不可得）必须整源响亮失败，落
                    // `UsageSourceStatus.errorCode`——否则用户看到的是「这个源没有用量」。
                    if file_read_failure_is_benign(&path, &err) {
                        log::warn!(
                            "usage/dsh: 投影缓存 {} 读取失败（文件已消失，跳过本轮）: {err}",
                            path.display()
                        );
                        continue;
                    }
                    return Err(UsageError::new(
                        "usage-source-io",
                        format!("dsh 投影缓存 {} 读取失败: {err}", path.display()),
                    ));
                }
            };
            if read.lines.is_empty() {
                if read.bytes_read == 0 {
                    // `(mtime,size)` 未变 → L2 快速路（零读取）→ 本轮无新数据，游标原样推回
                    b.push_cursor(read.next);
                } else {
                    // 读了字节却切不出一行完整行 = 文件不以 '\n' 收尾（撕裂写 / 非文本）。
                    // **不得**当成「没有数据」静默跳过（GC 7 / D-23 同族）：告警 + **不推游标**，
                    // 下一轮仍从 0 重读同一份文档（不推进就不会永久漏）。
                    log::warn!(
                        "usage/dsh: 投影缓存 {} 没有以换行收尾（撕裂写？），本轮跳过且不推进游标",
                        path.display()
                    );
                }
                continue;
            }
            // 投影缓存是**整份重写**的多行 JSON 文档（真机 170/170 都以 '\n' 收尾）：
            // 行切分产物按 '\n' 拼回正文即可（JSON 不在乎行尾空白）。
            let text = read.lines.join("\n");
            let Ok(rec) = serde_json::from_str::<Value>(&text) else {
                // 形状不认识：响亮但**不冻结整源**（一个坏文件不该让其余文件本轮不入账，
                // 与 D-24 的取舍一致）；不推游标 → 下轮重试。
                log::warn!(
                    "usage/dsh: 投影缓存 {} JSON 解析失败（本轮跳过且不推进游标）",
                    path.display()
                );
                continue;
            };
            b.parsed_files += 1;
            let rows = rec.pointer("/record/rows").cloned().unwrap_or(Value::Null);
            let cwd = rec
                .pointer("/record/identity/cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // **W-30：子代理分层按「本文件自身身份」判定**（不得走记录级 OR，也不得拿
            // 「行在场」当判据）。真机 170 个投影文件实测三种形态：
            // ① `subagent.val.identity` 非空 → **113 个文件本身就是子代理会话**；
            // ② `subagent.val = {}` → **57 个父会话**（行在场、值是空对象）；
            // ③ 旧快照连该行都没有 → 父会话。
            // 「行在场 ⇒ 子代理」的退化实现会把 170/170 全判成子代理 → 消费层按该位硬过滤
            // → 父会话连会话数带 turn 数一起消失（与 claude 那次 `|=` 是同一族错误的两个入口）。
            let is_sub = rows
                .pointer("/subagent/val/identity")
                .map(|id| id.as_object().map(|o| !o.is_empty()).unwrap_or(false))
                .unwrap_or(false);
            // 模型/供应商：modelSelection.val.lastUsed（真机 99/170 有行且都有值）
            let last_used = rows.pointer("/modelSelection/val/lastUsed");
            let model = last_used
                .and_then(|v| v.get("model"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let explicit = last_used
                .and_then(|v| v.get("provider"))
                .and_then(|v| v.as_str());
            if model.is_empty() {
                no_model_sessions += 1;
            }
            // **供应商唯一入口**（GC 6 / B6）：dsh 有同对象 `.provider` 字段
            // → 有值即 Measured；无值落 Unknown（**不是**空串推断）
            let (provider, _) = b.provider_of(explicit, model);
            // 累计四桶 + turns/steps/toolMs
            let tot = rows
                .pointer("/tokenUsage/val/totals")
                .cloned()
                .unwrap_or(Value::Null);
            let now_total = (
                tot.get("uncachedInputTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                tot.get("outputTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                tot.get("cacheReadTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                tot.get("cacheWriteTokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            );
            let stats = rows
                .pointer("/sessionStats/val")
                .cloned()
                .unwrap_or(Value::Null);
            let now_turns = stats.get("turns").and_then(|v| v.as_i64()).unwrap_or(0);
            let now_steps = stats.get("steps").and_then(|v| v.as_i64()).unwrap_or(0);
            let now_tool_ms = stats.get("toolMs").and_then(|v| v.as_i64()).unwrap_or(0);
            // 差值（累计 → 增量）：**必须从游标里读回上次快照**，只写不读就是每轮重复计
            //（D-34 在 opencode 上的形态）。回退（重写/重置）时按「本轮 = 当前值」计。
            let prev_state: DshState = prev
                .as_ref()
                .and_then(|c| serde_json::from_str(&c.state_json).ok())
                .unwrap_or_default();
            // **裁决 B（fix round 1）：时间桶必须来自会话自己的时间，不是采集时刻。**
            // 首轮（账本为空 / 本文件还没有累计快照）这一笔是**全部历史累计**；若按采集时刻归桶，
            // 用户第一次跑就会把整个历史（只读冒烟实测 input 53.1M / cache_read 2.64G / output 12.5M）
            // 塞进「当轮那一小时」，hero（近 24 小时）与小时趋势被历史淹没，而且这些行**永久**留在错的小时。
            // 链：`record.identity.createdAt`（真机实测是 ms epoch）→ 该文件**真实 mtime**
            // → 都拿不到才 `ctx.now_ms` + 告警。
            let first_snapshot = prev_state.is_empty();
            let ts = if first_snapshot {
                match rec
                    .pointer("/record/identity/createdAt")
                    .and_then(|v| v.as_i64())
                    .filter(|v| *v > 0)
                {
                    Some(t) => t,
                    None if read.next.mtime_ms > 0 => read.next.mtime_ms,
                    None => {
                        log::warn!(
                            "usage/dsh: 会话 {session_id} 既无 createdAt 也无文件 mtime，归桶退回采集时刻"
                        );
                        ctx.now_ms
                    }
                }
            } else {
                // **唯一把采集时刻当主判据的地方**（其余三处 `ctx.now_ms` 都是「真值拿不到」
                // 时的兜底，两处还带 warn；已在模块头与报告「偏差申报」点名）：
                // 有快照之后的差值记的是「自上次采集以来」发生的事，归**当轮**才是对的
                //（opencode 的 `session.time_updated` 在活跃会话上同理 ≈ 当轮）。
                // 把增量也钉在创建时刻会让会话寿命越长、趋势越歪。
                ctx.now_ms
            };
            let prev = prev_state;
            let diff = |cur: i64, old: i64| if cur >= old { cur - old } else { cur };
            let d_total = (
                diff(now_total.0, prev.total.0),
                diff(now_total.1, prev.total.1),
                diff(now_total.2, prev.total.2),
                diff(now_total.3, prev.total.3),
            );
            let hour = hour_key_of(ts, &TZ);
            let key = DetailKey {
                session_id: session_id.clone(),
                hour_key: hour.clone(),
                day_key: hour.chars().take(10).collect(),
                // 记录级项目键：dsh 的归属来自 record.identity.cwd（99/99 存在，9 个去重值）
                project_key: project_key_of(Some(cwd)),
                model: model.to_string(),
                // 任务书原文这里直接 `provider,`（把 `String` move 进 key），但同一个
                // `provider` 后面还要写进 `model_by_session` → **E0382**（借用已移动的值）。
                // **最小编译修复**：边界处 `clone()`，判据与语义一字未改。
                provider: provider.clone(),
            };
            let raw = RawUsage {
                input_raw: d_total.0,
                cache_read: d_total.2,
                cache_write: d_total.3,
                output: d_total.1,
                reasoning: 0,
                total_raw: None, // 无 total 判据 → 声明互斥（字段名 uncachedInputTokens 即互斥）
            };
            let sem = resolve_semantics(default_policy(UsageSourceId::Dsh), &raw);
            let d_steps = diff(now_steps, prev.steps);
            let d_turns = diff(now_turns, prev.turns);
            let d_tool_ms = diff(now_tool_ms, prev.tool_ms);
            // 零差值不写行（同 opencode）：否则第二轮起每轮 upsert 一条全 0 明细
            let changed = d_total != (0, 0, 0, 0) || d_steps != 0 || d_turns != 0 || d_tool_ms != 0;
            if changed {
                b.new_records += 1; // 本条记录确实带来了新数据
                if let Some(n) = normalize(&raw, sem) {
                    let d = b.detail(key.clone(), ts);
                    d.buckets.add(&n.buckets);
                    d.request_total += n.request_total;
                }
                let d = b.detail(key, ts);
                d.requests += d_steps; // 请求数 = LLM 调用步数（steps）
                d.counters.turns += d_turns;
                d.counters.tool_ms += d_tool_ms;
            }
            // **fix round 2 Minor#3**：会话维度的 `last_seen_at` 用**该投影缓存文件的真实 mtime**
            //（= 这个会话最后一次被写入的时间），**不是** `ts` —— `ts` 在首轮是 `createdAt`，
            // 把它当「最后见到」会让「只由投影缓存代表、此后不再变化」的会话永远停在创建时刻
            //（DAO 取 MAX，一旦写入就回不来）。mtime 正是本文件回退链里已经信任的那个真值；
            // 取不到（理论不可达：`stat_of` 拿不到 mtime 时读早就 Err 了）才退回 ts。
            let last_seen = if read.next.mtime_ms > 0 {
                read.next.mtime_ms
            } else {
                ts
            };
            b.session(&session_id, Some(cwd), None, is_sub, None, None, last_seen);
            // 游标：真实代际（`mtime_ms` / `file_size` 取自**读取前**那次 stat —— C3/W-23：
            // 不得写 token 累计数那种语义无关的垃圾值，也不得把失败伪装成 0）+ 累计快照。
            let mut cur = read.next;
            cur.session_id = session_id.clone();
            cur.last_cumulative = Some(now_total.0 + now_total.1 + now_total.2 + now_total.3);
            // **投影缓存的字节水位语义（有意为之，不得改）**：它是**整份重写**的 JSON 文档，
            // 不是只追加的日志 —— 记真实 `byte_offset` 会让「同尺寸重写 / 尾部追加」命中
            // append 快速判定，只回吐半份 JSON（静默少算）。恒置 0 的效果是：
            // * `(mtime,size)` 未变 → L2 快速路，零读取（不变语义照旧）；
            // * 代际一变 → 从 0 全量读回**整份文档**（正是「一遍扫描」要的），
            //   增量 = 新累计 − 游标 state_json 里的旧累计。
            // `fingerprint` 同时清空 = 「没有已消费前缀」，任何将来按 `fingerprint_mismatch`
            // 判定的路径都会往**重扫**这个安全方向倒。
            cur.byte_offset = 0;
            cur.ordinal = 0;
            cur.fingerprint = String::new();
            cur.state_json = serde_json::to_string(&DshState {
                total: now_total,
                turns: now_turns,
                steps: now_steps,
                tool_ms: now_tool_ms,
            })
            .unwrap_or_default();
            b.push_cursor(cur);
            model_by_session.insert(session_id.clone(), (model.to_string(), provider.clone()));
        }
        if no_model_sessions > 0 {
            log::warn!(
                "usage/dsh: 本轮 {no_model_sessions} 个投影缓存文件结构性缺 modelSelection 行 \
                 → 四桶照常入账、明细行按空模型 + Unknown 供应商（真机 63/171，矩阵 §5.1/§C.2；不得静默）"
            );
        }
        // ---- B. 原始会话日志（报错三层 / 最长单 turn / 逐工具）----
        scan_raw_logs(ctx, home, &mut b, &model_by_session)?;
        Ok(b.finish())
    }
}

/// 原始日志扫描（复用既有 `monitor::dsh::log` 的三代际选择与 zstd 解码）。
///
/// **一遍扫描产出全部指标**：`LogDigest::of` 在**一次**遍历里同时产出
/// 报错三层 / 用户打断 / turn 时长样本 / **逐工具（次数 + 耗时）** —— 绝不为新指标再解一遍。
/// 这个**扫描产物整体**进 L2 缓存（`(mtime,size)` 未变 → 连解析都不做）：稳态下一份未变的日志
/// 是**零解码、零解析**（GC 3/5），由 `parsed_files == 0` 的断言钉住
///（fix round 1 裁决 C：原来缓存的是解压后的**正文**，每轮仍要把全部事件重新 `parse_events` 一遍）。
fn scan_raw_logs(
    ctx: &CollectContext<'_>,
    home: &std::path::Path,
    b: &mut DeltaBuilder,
    model_by_session: &HashMap<String, (String, String)>,
) -> Result<(), UsageError> {
    let sessions_root = home.join("sessions");
    // **M2（fix round 1）**：根目录读失败按 `ErrorKind` 区分 —— `NotFound` = 未装 dsh /
    // 会话日志从未落盘（正常空态）；其余（EACCES / EIO）是「这个源读不了」→ **整源响亮失败**。
    let projects = match std::fs::read_dir(&sessions_root) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(UsageError::new(
                "usage-source-io",
                format!(
                    "dsh 会话日志根目录 {} 读取失败: {e}",
                    sessions_root.display()
                ),
            ))
        }
    };
    let mut live = std::collections::HashSet::new();
    // L2 内容摘要缓存的句柄（GC 5：namespace 由 `scan_namespace(Dsh)` 派生 + `-log`）——
    // `SessionFileScan` 只是个零成本句柄（真正的缓存在 `session_scan` 的全局注册表里），
    // 每轮现造一个即可。
    let log_scan = SessionFileScan::new(log_scan_namespace());
    // 聚合告警计数（M1 / 工具配对失败）：**不得静默**，但也不按会话刷屏
    let mut unpaired = 0i64;
    let mut no_model_rows = 0i64;
    for p in projects.flatten() {
        let ppath = p.path();
        if !ppath.is_dir() {
            continue;
        }
        let dirs = match std::fs::read_dir(&ppath) {
            Ok(d) => d,
            // 目录在枚举与读取之间消失 = 真竞态 → 跳过（D-24 唯一允许静默的 kind）
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            // 单个项目目录读不了：**响亮到日志**、不冻结整源（一个子目录不该让整个源本轮不入账；
            // 与 `kimi.rs` 的家族口径同形，已登记）
            Err(e) => {
                log::warn!(
                    "usage/dsh: 会话项目目录 {} 读取失败（跳过该项目）: {e}",
                    ppath.display()
                );
                continue;
            }
        };
        for sd in dirs.flatten() {
            let spath = sd.path();
            if !spath.is_dir() {
                continue;
            }
            let Some((_v, gen_path)) = crate::monitor::dsh::log::generation_logs(&spath).pop()
            else {
                continue;
            };
            live.insert(gen_path.clone());
            // 预过滤反模式禁令（AGENTS.md L3-4）：内容未变 → 不打开、不解压、不解析。
            // `parsed_now` 记「本轮真的解了码」（W-50 的文件计数口径）。
            let parsed_now = std::cell::Cell::new(false);
            let digest = log_scan.parse(&gen_path, |p| {
                parsed_now.set(true);
                p.parent()
                    .and_then(crate::monitor::dsh::log::read_best_generation)
                    .and_then(|r| LogDigest::of(&r))
            });
            let Some(digest) = digest.as_ref() else {
                // 解不开（截断 / 非 zstd / 空文件）：**响亮但不冻结整源**——一个坏文件不该让
                // 其余会话本轮不入账（与 D-24 的取舍一致）；该会话的报错/工具指标按**空态**，
                // 绝不填 0（GC 7 / §9.5 的「部分覆盖」）。
                // **重试时机（M6，措辞已更正）**：`get_or_parse` 把 `None` **也**按 `(mtime,size)`
                // 缓存 → 该文件是「**下次变化时**才重试」，不是每轮重试。
                // 这是有意保留的（坏文件不该每轮重复解压 + 刷屏），**不是**「不推进游标所以下轮重试」。
                log::warn!(
                    "usage/dsh: 原始日志无法解码（本次跳过，不产出报错/工具指标）: {}",
                    gen_path.display()
                );
                continue;
            };
            if parsed_now.get() {
                b.parsed_files += 1;
            }
            let header = &digest.header;
            let facts = &digest.facts;
            let cwd = header.cwd.clone().unwrap_or_default();
            b.session(
                &header.id,
                Some(&cwd),
                None,
                crate::monitor::dsh::log::is_subagent(header),
                header.parent_session.clone(),
                None,
                // 会话维度的 last_seen_at 用日志自己的最后活动时间（裁决 B 同源），
                // 不用采集时刻
                facts.last_event_ms.unwrap_or(ctx.now_ms),
            );
            // 事实计数水位（键带 `log:` 前缀，避免与投影缓存的同名会话键互相覆盖）
            let log_key = format!("log:{}", header.id);
            // **D-41 同族（跨轮持久化模型 / 供应商）**：原始日志侧不产生模型字段，
            // 若本轮的投影缓存不可得（被清理 / 该文件结构性缺 `modelSelection` 行），
            // 必须沿用**上一轮已知**的模型与供应商——否则这些报错/工具计数会落进
            // `model=""` / `provider=""` 的幽灵行（进明细表与日聚合主键、UI 显空名）。
            let prev: DshLogState = ctx
                .cursor_of(&log_key)
                .and_then(|c| serde_json::from_str(&c.state_json).ok())
                .unwrap_or_default();
            let (model, provider) = match model_by_session.get(&header.id) {
                Some((m, p)) if !m.is_empty() => (m.clone(), p.clone()),
                // 本轮的投影缓存没给出模型：退回游标里的上一轮值（结构性不可得时两边都空）
                _ => (prev.model.clone(), prev.provider.clone()),
            };
            let diff = |cur: i64, old: i64| if cur >= old { cur - old } else { cur };
            // **M3 / W-49（fix round 1）**：水位按**代际**而不只按会话 —— 真机 7/172 个会话目录
            // 有多代际文件（v2+v3 / plain+v3 / v3+v4）。代际一变：
            // * 新文件若是旧水位的**超集**（各项计数都不减）→ 差值恰好是「切换期间新增的事件」，照常计；
            // * 否则（截断 / 重建）→ **对齐水位而不计数**。旧语义「回退按整份重计」会把新文件里
            //   **已有的**全部报错/工具/样本再计一遍（重复计入）。
            let gen_changed = prev.gen != 0 && prev.gen != digest.version;
            let superset = facts.error_model >= prev.error_model
                && facts.error_turn >= prev.error_turn
                && facts.error_tool >= prev.error_tool
                && facts.interrupted >= prev.interrupted
                && facts.tool_calls >= prev.tool_calls
                && (facts.turn_ms.len() as i64) >= prev.turn_samples;
            let align_only = gen_changed && !superset;
            if align_only {
                log::warn!(
                    "usage/dsh: 会话 {} 的日志代际 {} → {} 不是旧水位的超集 → 只对齐水位、本轮不计任何事实",
                    header.id,
                    prev.gen,
                    digest.version
                );
            }
            let now_samples = facts.turn_ms.len();
            let mut new_samples: Vec<i64> = if align_only {
                Vec::new()
            } else if now_samples as i64 >= prev.turn_samples {
                // 只取**新增的尾部样本**（日志只追加）；水位回退时按整份重计
                let from = prev.turn_samples.max(0) as usize;
                facts.turn_ms[from.min(now_samples)..].to_vec()
            } else {
                facts.turn_ms.clone()
            };
            let mut d_error_model = diff(facts.error_model, prev.error_model);
            let mut d_error_turn = diff(facts.error_turn, prev.error_turn);
            let mut d_error_tool = diff(facts.error_tool, prev.error_tool);
            let mut d_interrupted = diff(facts.interrupted, prev.interrupted);
            let mut d_tool_calls = diff(facts.tool_calls, prev.tool_calls);
            let mut d_unpaired = diff(facts.tool_unpaired, prev.tool_unpaired);
            // 逐工具：次数与耗时**都**按差值算（同一个累计水位）。
            // **绝不只填次数**：那会让该工具的 ms 变成假 0（GC 7）。
            let mut d_tool_stats: Vec<(String, (i64, i64))> = Vec::new();
            if !align_only {
                for (name, (cnt, ms)) in &facts.tool_stats {
                    let (pc, pm) = prev.tool_stats.get(name).copied().unwrap_or((0, 0));
                    let (dc, dm) = (diff(*cnt, pc), diff(*ms, pm));
                    if dc != 0 || dm != 0 {
                        d_tool_stats.push((name.clone(), (dc, dm)));
                    }
                }
            }
            if align_only {
                d_error_model = 0;
                d_error_turn = 0;
                d_error_tool = 0;
                d_interrupted = 0;
                d_tool_calls = 0;
                d_unpaired = 0;
                new_samples.clear();
                d_tool_stats.clear();
            }
            let new_total = d_error_model
                + d_error_turn
                + d_error_tool
                + d_interrupted
                + d_tool_calls
                + new_samples.len() as i64;
            unpaired += d_unpaired;
            // **fix round 2 裁决 A（行为性、静默永久丢数）**：落行闸门原来只看 `new_total`，
            // 而**迟到回执**恰好是「回执是该轮**唯一**新增事实」的形态（真机单工具最长 3.1 h
            // → 跨轮必然发生）：第 N 轮读到 `tool/call`（无回执）记下 `(1,0)` 并**推进水位**；
            // 第 N+1 轮回执到达（成功工具 → 无 error_tool、turn 未结束、无新调用）⇒ `new_total == 0`
            // ⇒ 不落行，**但水位已把这段 ms 记走**；第 N+2 轮 turn/end 落行时 `dm` 已是 0
            // ⇒ **该次工具耗时永久丢失**。
            // 所以「逐工具耗时」必须与 `new_total` 并列成为落行条件（codex 在回执事件上
            // 无条件 `b.detail(...)`，同款）。
            // 注意 `new_records` **仍按事实数**（此处 +0）：它记的是「新增事实」，
            // 而这一轮新增的是**已计过次数的调用的耗时**，不是新事实 —— Task 23B 的
            // 「零差值轮 newRecords == 0」语义不受影响。
            let has_ms_only = d_tool_stats.iter().any(|(_, (_, dm))| *dm != 0);
            if new_total > 0 || has_ms_only {
                b.new_records += new_total;
                // **裁决 B（fix round 1）**：原始日志侧归桶用**日志自己的时间** ——
                // 最后一条带时间的事件 → 该代际文件的真实 mtime → 都拿不到才采集时刻 + 告警。
                // 绝不用采集时刻当唯一来源（首轮会把整个历史的报错/工具/时长记进当轮小时桶）。
                let ts = match facts.last_event_ms {
                    Some(t) => t,
                    None if digest.mtime_ms > 0 => digest.mtime_ms,
                    None => {
                        log::warn!(
                            "usage/dsh: 会话 {} 的日志既无事件时间戳也无文件 mtime，归桶退回采集时刻",
                            header.id
                        );
                        ctx.now_ms
                    }
                };
                let hour = hour_key_of(ts, &TZ);
                if model.is_empty() {
                    no_model_rows += 1;
                }
                // 供应商一律经唯一入口（GC 6）——**包括**从游标里继承来的这一路：
                // 空模型时 `provider_of` 落 (`""`, Unknown)，与投影缓存侧同一口径。
                let (provider, _) = b.provider_of(Some(&provider), &model);
                let key = DetailKey {
                    session_id: header.id.clone(),
                    hour_key: hour.clone(),
                    day_key: hour.chars().take(10).collect(),
                    project_key: project_key_of(Some(&cwd)),
                    // `clone()`：同一个 `model` 后面还要写进跨轮水位（D-41），直接 move 是 E0382
                    model: model.clone(),
                    provider,
                };
                let d = b.detail(key, ts);
                d.counters.error_model += d_error_model;
                d.counters.error_turn += d_error_turn;
                d.counters.error_tool += d_error_tool;
                d.counters.interrupted += d_interrupted;
                d.counters.tool_calls += d_tool_calls;
                d.counters.turn_ms.extend(new_samples);
                for (name, (dc, dm)) in d_tool_stats {
                    let e = d.counters.tool_stats.entry(name).or_default();
                    e.0 += dc;
                    e.1 += dm;
                }
            }
            // 日志侧游标 = **事实计数水位**（键 `log:<session>`，与投影缓存侧的会话键分开存，
            // 因为两侧的「上次快照」语义完全不同）。
            // `byte_offset` / `ordinal` / `file_size` / `mtime_ms` / `fingerprint` **一律 0**：
            // 本游标**不按字节判定**（变化判定由 L2 的 (mtime,size) 摘要缓存 + 这里的计数差值负责），
            // 0 = 「不适用」，**不是**「文件大小/mtime 是 0」——不得往这几个字段填任何看起来像
            // 真值的数（D-23 一族：把「不适用 / 不可得」伪装成「真值」）。
            let mut cur = ctx.cursor_of(&log_key).cloned().unwrap_or_default();
            cur.session_id = log_key;
            cur.state_json = serde_json::to_string(&DshLogState {
                // **M3**：记住产出这份水位的**代际**
                gen: digest.version,
                error_model: facts.error_model,
                error_turn: facts.error_turn,
                error_tool: facts.error_tool,
                interrupted: facts.interrupted,
                tool_calls: facts.tool_calls,
                turn_samples: now_samples as i64,
                tool_stats: facts.tool_stats.clone(),
                tool_unpaired: facts.tool_unpaired,
                // **D-41**：模型 / 供应商随水位跨轮持久化（本项目其它源同款纪律）
                model,
                provider,
            })
            .unwrap_or_default();
            b.push_cursor(cur);
        }
    }
    log_scan.retain_existing(&live);
    if unpaired > 0 {
        log::warn!(
            "usage/dsh: 本轮 {unpaired} 次工具调用拿不到耗时（无 callId / 无回执 / 任一侧无时间戳）\
             → 只计次数、不计耗时（不写假 0，GC 7）"
        );
    }
    if no_model_rows > 0 {
        log::warn!(
            "usage/dsh: 本轮 {no_model_rows} 个会话的原始日志事实落在空模型行 \
             （该会话投影缓存结构性缺 modelSelection，矩阵 §5.1/§C.2；不得静默）"
        );
    }
    Ok(())
}

/// 原始日志侧的**事实计数水位**（B4）。与投影缓存侧的 `DshState` **分开**存
/// （游标键 `log:<session_id>`），因为两侧的「上次快照」语义不同。
#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
struct DshLogState {
    /// **M3**：产出这份水位的日志**代际**（`GenerationRead.version`）。
    /// 0 = 旧版游标（没有这个字段）→ 不做代际判定，行为与修复前一致。
    #[serde(default)]
    gen: i64,
    error_model: i64,
    error_turn: i64,
    error_tool: i64,
    interrupted: i64,
    tool_calls: i64,
    /// `facts.turn_ms` 的样本条数（只取新增尾部样本）
    turn_samples: i64,
    /// **逐工具累计水位**：工具名 → (调用次数, 累计耗时毫秒)
    #[serde(default)]
    tool_stats: std::collections::BTreeMap<String, (i64, i64)>,
    /// 计数了但拿不到耗时的工具调用数（无 callId / 无回执 / 任一侧无 time）
    #[serde(default)]
    tool_unpaired: i64,
    /// **D-41 同族**：该会话最后一次已知的模型 / 供应商（跨轮持久化）。
    /// 原始日志侧不产生模型字段，而 63/171 个投影文件**结构性**地缺 `modelSelection` 行 ——
    /// 有值就必须记住，免得下一轮把报错/工具计数落进 `model=""` 幽灵行。
    #[serde(default)]
    model: String,
    #[serde(default)]
    provider: String,
}

/// L2 缓存的原始日志**扫描产物**（纯内容产物：同一份文本必然产出同一份产物，
/// 不含任何随轮次变化的东西——这是它能进 `(mtime,size)` 摘要缓存的前提）。
#[derive(Debug, Clone)]
struct LogDigest {
    /// 代际号（`session.vN` 的 N；`session.jsonl` = 0）
    version: i64,
    /// 该代际文件的 mtime（毫秒）：B 段归桶的第二顺位来源
    mtime_ms: i64,
    header: crate::monitor::dsh::log::DshHeader,
    facts: LogFacts,
}

impl LogDigest {
    /// **一遍扫描**（GC 4「一次扫描产出全部指标」）：报错三层 / 打断 / turn 时长样本 /
    /// 逐工具（次数 + 耗时）全在同一次 `parse_events` 之上产出。
    ///
    /// 逐工具口径（真机实测）：配对键 `tool/call.data.callId` ↔
    /// `tool/result.data.message.source.callId`（**不是同一个字段名，也不是同一层**，见
    /// `result_call_id` 的文档），耗时 = `tool/result.time − tool/call.time`；
    /// 真机 `callId` **全不重复**（没有 codex 式相邻重放）。
    fn of(read: &crate::monitor::dsh::log::GenerationRead) -> Option<Self> {
        use crate::monitor::dsh::log::{parse_events, parse_header};
        let header = parse_header(&read.text)?;
        let mut facts = LogFacts::default();
        let mut open_start: Option<i64> = None;
        // 本文件内的 未回执调用表：callId → (工具名, 调用时刻)
        let mut open_calls: HashMap<String, (String, i64)> = HashMap::new();
        for e in parse_events(&read.text) {
            if let Some(t) = e.time {
                facts.last_event_ms = Some(facts.last_event_ms.map_or(t, |m| m.max(t)));
            }
            match e.kind.as_str() {
                "turn/start" => open_start = e.time,
                "turn/end" => {
                    if let (Some(s), Some(end)) = (open_start.take(), e.time) {
                        facts.turn_ms.push((end - s).max(0));
                    }
                    match e
                        .data
                        .pointer("/reason/kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                    {
                        "error" => facts.error_turn += 1,
                        "aborted" | "interrupted" => facts.interrupted += 1,
                        _ => {}
                    }
                    if e.data.pointer("/reason/error/code").is_some() {
                        facts.error_model += 1;
                    }
                }
                "llm/retry" => facts.error_model += 1,
                "tool/call" => {
                    facts.tool_calls += 1;
                    // 工具名缺失时落 `unknown` 桶（真机 14,148/14,148 全部带 `name`；
                    // 与 claude 采集器同款兜底）——**这样 Σ tool_stats.0 才恒等于 tool_calls**
                    let name = e
                        .data
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("unknown")
                        .to_string();
                    facts.tool_stats.entry(name.clone()).or_insert((0, 0)).0 += 1;
                    match (e.data.get("callId").and_then(|v| v.as_str()), e.time) {
                        (Some(id), Some(t)) => {
                            open_calls.insert(id.to_string(), (name, t));
                        }
                        // 没有 callId 或没有时间戳 → 次数照记、耗时不计（下面计数 + 告警）
                        _ => facts.tool_unpaired += 1,
                    }
                }
                "tool/result" => {
                    // 工具失败判据：真机 `data.error` 是在场对象 `{name, code}`
                    //（406/406 全为 dict，无 null、无其它形状）
                    let is_err = e.data.get("error").map(|x| !x.is_null()).unwrap_or(false);
                    if is_err {
                        facts.error_tool += 1;
                    }
                    if let Some(id) = result_call_id(&e.data) {
                        if let Some((name, t0)) = open_calls.remove(id) {
                            match e.time {
                                Some(t1) => {
                                    facts.tool_stats.entry(name).or_insert((0, 0)).1 +=
                                        (t1 - t0).max(0);
                                }
                                // 回执没有时间戳 → 同样只计次数、不计耗时
                                None => facts.tool_unpaired += 1,
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // 整份日志读完仍没有回执的调用（被用户打断 / 无回执）→ 计入 unpaired（计数 + 告警，不写假 0）
        facts.tool_unpaired += open_calls.len() as i64;
        Some(LogDigest {
            version: read.version,
            mtime_ms: read.mtime_ms,
            header,
            facts,
        })
    }
}

/// 回执侧的配对键解析（**真机形态优先**，逐层兜底）。
///
/// 真机 173 文件全量实测：**14,584/14,584** 条 `tool/result` 的键在
/// `data.message.source.callId`（对象形态 `{"kind":"tool","callId":"call_…"}`），
/// 配对率 **14,552/14,554 = 100.0%**（矩阵旧口径 1,299/1,302）；
/// 而**任务书正文写的顶层 `data.toolCallId` 真机一个都没有**（探测文档里的旧样本形态
/// `data.message.toolCallId` 也只有 72 条）——照抄扁平形态会让配对率恒 0、
/// 逐工具耗时**全 0**，而任务书夹具恰好把它放在顶层 → 用例全绿（D-29 / D-36 同族假绿）。
/// 三层都认（新形态 → 旧形态 → 任务书形态），只为形状演进兜底，**不以它们为主判据**。
fn result_call_id(data: &Value) -> Option<&str> {
    data.pointer("/message/source/callId")
        .and_then(|v| v.as_str())
        .or_else(|| data.pointer("/message/toolCallId").and_then(|v| v.as_str()))
        .or_else(|| data.get("toolCallId").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
}

/// 一份原始日志的**累计事实**（纯内容产物，整份进 L2 缓存）
#[derive(Debug, Default, Clone)]
struct LogFacts {
    turn_ms: Vec<i64>,
    error_model: i64,
    error_turn: i64,
    error_tool: i64,
    interrupted: i64,
    tool_calls: i64,
    /// 逐工具累计：工具名 → (调用次数, 累计耗时毫秒)。
    /// **只有能配对的调用才计耗时**（配不上/缺时间戳的只计次数，见 `tool_unpaired`）——
    /// 绝不写「假 0 耗时」（GC 7）。
    tool_stats: std::collections::BTreeMap<String, (i64, i64)>,
    /// 该日志里**最后一条带时间的事件**的毫秒时间戳（裁决 B：归桶用真实活动时间）
    last_event_ms: Option<i64>,
    /// 计数了但拿不到耗时的工具调用数
    tool_unpaired: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::usage::caps::caps_of;
    use crate::services::usage::collect::{CollectContext, CollectStats};
    use crate::services::usage::delta::CursorDelta;
    use crate::services::usage::model::SourceKind;
    use serde_json::json;
    use std::collections::HashMap;

    /// **本文件所有用例串行**（家族先例：`monitor::kimi_parser` 的 `HOME_LOCK`，见 D-30）。
    ///
    /// 理由：日志侧的 L2 缓存是**进程级全局注册表**（`monitor::session_scan::registry`），
    /// 而 `retain_existing(&live)` 会清掉**同 namespace 里不在本轮 live 集合中的条目** ——
    /// 并行跑的另一个用例的本轮 live 集合里当然没有本用例的临时文件 → 顺手把本用例的缓存条目
    /// 清掉 → 「未变代际零解码零解析」这类**路径类断言**（裁决 C）会随机变红（实测：
    /// 单跑 5/5 绿、全文件并行跑时偶发红）。**生产侧不存在这个问题**：一个源一个采集器，
    /// 进程内只有一份 live 集合。锁本身不改变任何生产行为。
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 投影缓存里 `subagent` 行的**三种真机形态**（W-30）。
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum SubAgentRow {
        /// 连该行都没有（旧快照文件）
        Missing,
        /// **真机父会话形态**：行在场、`val` 是**空对象** `{}`（真机 57/170）
        ParentEmpty,
        /// **真机子会话形态**：`val.identity` 非空（真机 113/170）
        ChildIdentity,
    }

    /// 写一个投影缓存文件。**真机形态**：`version:7` + 多行缩进 JSON + 行尾 `\n`
    /// （真机 170/170 都是如此）。采集器经 `read_incremental` 读它——
    /// **没有行尾换行 = 一行完整行都切不出来**，夹具必须同形（D-29 同族：夹具绕开真机形态 = 假绿）。
    #[allow(clippy::too_many_arguments)]
    fn projcache(
        home: &std::path::Path,
        id: &str,
        cwd: &str,
        totals: (i64, i64, i64, i64),
        turns: i64,
        steps: i64,
        tool_ms: i64,
        subagent: SubAgentRow,
        model: Option<(&str, &str)>,
    ) {
        projcache_full(
            home, id, cwd, 1, totals, turns, steps, tool_ms, subagent, model,
        );
    }

    /// 同上，但显式给 `record.identity.createdAt`（**裁决 B 的归桶锁**要它跟采集时刻分处不同小时）
    #[allow(clippy::too_many_arguments)]
    fn projcache_full(
        home: &std::path::Path,
        id: &str,
        cwd: &str,
        created_at: i64,
        totals: (i64, i64, i64, i64),
        turns: i64,
        steps: i64,
        tool_ms: i64,
        subagent: SubAgentRow,
        model: Option<(&str, &str)>,
    ) {
        let d = home.join("storages/session_projcache/sessions");
        std::fs::create_dir_all(&d).unwrap();
        // `val.last == null`（真机 10/170）＝本源 TotalOnly 等价情形 → 只走 totals
        let mut rows = json!({
            "tokenUsage": {"ver":2,"seq":1,"val":{"totals":{
                "uncachedInputTokens":totals.0,"outputTokens":totals.1,
                "cacheReadTokens":totals.2,"cacheWriteTokens":totals.3},
                "last": null}},
            "sessionStats": {"ver":1,"seq":1,"val":{"turns":turns,"steps":steps,"toolMs":tool_ms}}
        });
        if let Some((p, m)) = model {
            rows["modelSelection"] =
                json!({"ver":1,"seq":1,"val":{"lastUsed":{"provider":p,"model":m}}});
        }
        match subagent {
            SubAgentRow::Missing => {}
            SubAgentRow::ParentEmpty => {
                rows["subagent"] = json!({"ver":1,"seq":1,"val":{}});
            }
            SubAgentRow::ChildIdentity => {
                rows["subagent"] =
                    json!({"ver":1,"seq":1,"val":{"identity":{"mode":"continuable","label":"x"}}});
            }
        }
        let rec = json!({"version":7,"record":{"identity":{"formatVersion":3,"createdAt":created_at,"cwd":cwd},"rows":rows}});
        let body = format!("{}\n", serde_json::to_string_pretty(&rec).unwrap());
        std::fs::write(d.join(format!("{id}.json")), body).unwrap();
    }

    /// 行 ⇒ `\n` 结尾的 JSONL 正文
    fn log_body(events: &[serde_json::Value]) -> String {
        events
            .iter()
            .map(|e| format!("{}\n", serde_json::to_string(e).unwrap()))
            .collect()
    }

    /// 写一份**真机代际布局**的原始日志：`sessions/<proj>/<dir>/session.v3.jsonl.zstd`
    /// （与既有 `monitor/dsh` 日志测试同款 zstd 编码）。`events[0]` 必须是 session 头。
    fn write_log(
        home: &std::path::Path,
        proj: &str,
        dir: &str,
        events: &[serde_json::Value],
    ) -> std::path::PathBuf {
        let sess_dir = home.join("sessions").join(proj).join(dir);
        std::fs::create_dir_all(&sess_dir).unwrap();
        let p = sess_dir.join("session.v3.jsonl.zstd");
        let frame = zstd::stream::encode_all(log_body(events).as_bytes(), 3).unwrap();
        std::fs::write(&p, frame).unwrap();
        p
    }

    /// 追加**一帧** zstd（真机 `.zstd` 就是多帧拼接，`decode_zstd_frames` 专为它存在）
    fn append_log(path: &std::path::Path, events: &[serde_json::Value]) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(&zstd::stream::encode_all(log_body(events).as_bytes(), 3).unwrap())
            .unwrap();
    }

    /// 采集一轮（home 注入版）
    fn run(
        home: &std::path::Path,
        cursors: &HashMap<String, CursorDelta>,
    ) -> (SourceDelta, CollectStats) {
        let ctx = CollectContext::new(home, 1_700_000_000_000, cursors);
        let delta = DshCollector::collect_with_home(&ctx, home).unwrap();
        (delta, ctx.stats())
    }

    fn cursors_of(d: &SourceDelta) -> HashMap<String, CursorDelta> {
        d.cursors
            .iter()
            .map(|c| (c.session_id.clone(), c.clone()))
            .collect()
    }

    #[test]
    fn cumulative_diff_subagent_layering_and_missing_model_row() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "par",
            "/p/A",
            (100, 10, 50, 0),
            3,
            9,
            1000,
            SubAgentRow::Missing,
            Some(("ollama-pro", "deepseek-v4.1-flash")),
        );
        projcache(
            dir.path(),
            "sub",
            "/p/A",
            (20, 5, 0, 0),
            1,
            2,
            300,
            SubAgentRow::ChildIdentity,
            None,
        );
        // 空游标表必须先绑成变量：`&HashMap::new()` 是**临时值**，借用进 `ctx` 就是
        // E0716（temporary value dropped while borrowed）——语句结束即释放。
        let no_cursors = HashMap::new();
        let ctx = CollectContext::new(dir.path(), 1_700_000_000_000, &no_cursors);
        let d1 = DshCollector::collect_with_home(&ctx, dir.path()).unwrap();
        // 首轮 = 全量（无上次累计）
        let out: i64 = d1.details.iter().map(|d| d.buckets.output).sum();
        assert_eq!(out, 15, "10 + 5：首轮是全量累计");
        // 子代理分层（D17）：47/99 文件本身是子代理会话 → 会话维度 is_subagent=true
        assert!(
            d1.sessions
                .iter()
                .find(|s| s.session_id == "sub")
                .unwrap()
                .is_subagent
        );
        assert!(
            !d1.sessions
                .iter()
                .find(|s| s.session_id == "par")
                .unwrap()
                .is_subagent
        );
        // 缺 modelSelection 行（真机 63/170）→ 模型/供应商为空，但四桶与 turns 照常
        let sub = d1
            .details
            .iter()
            .find(|d| d.key.session_id == "sub")
            .unwrap();
        assert_eq!(sub.key.model, "");
        assert_eq!(sub.counters.turns, 1);
        // 供应商不可得 → 空串 + **未知态**（经 provider_of 记录；不得猜、不得填 "unknown" 字面量）
        assert_eq!(sub.key.provider, "");
        assert_eq!(sub.provider_kind, SourceKind::Unknown);
        // **四桶映射 + 语义**：`uncachedInputTokens` 即互斥 → Exclusive（本机无 total 判据），
        // 请求输入 = 未缓存 + 缓存读 + 缓存写（命中率由查询层算，采集器不得自己算，GC 6/W-9）
        let par = d1
            .details
            .iter()
            .find(|d| d.key.session_id == "par")
            .unwrap();
        assert_eq!(par.buckets.input_fresh, 100);
        assert_eq!(par.buckets.cache_read, 50);
        assert_eq!(par.buckets.cache_write, 0);
        assert_eq!(par.request_total, 150, "Exclusive：100 + 50 + 0");
        // turns / steps / toolMs（真机 99/99 存在；turns 与原始 turn/start 精确相等）
        assert_eq!(par.counters.turns, 3);
        assert_eq!(par.counters.tool_ms, 1000);
        assert_eq!(
            par.requests, 9,
            "dsh 无逐请求字段 → 请求数用 sessionStats.steps（LLM 调用步数）"
        );
        // 供应商经唯一入口解析出的三态必须落进明细行（GC 6：不得退化成 unknown）
        assert_eq!(par.key.provider, "ollama-pro");
        assert_eq!(par.provider_kind, SourceKind::Measured);
        // W-50：文件型源 → parsed_files 记**本轮真读到的文件数**（不是会话数）
        assert_eq!(d1.parsed_files, 2);
        // 二轮：文件更新（累计增长）→ **只计差值**，不重复计
        projcache(
            dir.path(),
            "par",
            "/p/A",
            (150, 25, 60, 0),
            4,
            11,
            1500,
            SubAgentRow::Missing,
            Some(("ollama-pro", "deepseek-v4.1-flash")),
        );
        let cursors: HashMap<_, _> = d1
            .cursors
            .iter()
            .map(|c| (c.session_id.clone(), c.clone()))
            .collect();
        let ctx2 = CollectContext::new(dir.path(), 1_700_000_000_000 + 60_000, &cursors);
        let d2 = DshCollector::collect_with_home(&ctx2, dir.path()).unwrap();
        let out2: i64 = d2.details.iter().map(|d| d.buckets.output).sum();
        assert_eq!(out2, 15, "差值：输出 25 - 10 = 15");
        let par2 = d2
            .details
            .iter()
            .find(|d| d.key.session_id == "par")
            .unwrap();
        assert_eq!(par2.counters.turns, 1, "turns 差值 4 - 3 = 1");
        assert_eq!(par2.counters.tool_ms, 500, "toolMs 差值 1500 - 1000");
        assert_eq!(par2.requests, 2, "steps 差值 11 - 9");
    }

    /// **C3 回归**：投影缓存游标的 `mtime_ms` / `file_size` 必须是**真实文件元数据**。
    /// 旧实现写的是 `now_total.0`（token 累计数）——一个语义完全无关的垃圾值：
    /// 任何将来按 `file_unchanged()` 语义使用该列的代码都会得到错误结论。
    #[test]
    fn projcache_cursor_carries_real_mtime_and_size_not_token_total() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "par",
            "/p/A",
            (100, 10, 50, 0),
            3,
            9,
            1000,
            SubAgentRow::Missing,
            Some(("ollama-pro", "deepseek-v4-flash")),
        );
        let path = dir
            .path()
            .join("storages/session_projcache/sessions/par.json");
        let md = std::fs::metadata(&path).unwrap();
        let real_mtime = md
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let real_size = md.len() as i64;

        // 空游标表必须先绑成变量：`&HashMap::new()` 是**临时值**，借用进 `ctx` 就是
        // E0716（temporary value dropped while borrowed）——语句结束即释放。
        let no_cursors = HashMap::new();
        let ctx = CollectContext::new(dir.path(), 1_700_000_000_000, &no_cursors);
        let d1 = DshCollector::collect_with_home(&ctx, dir.path()).unwrap();
        let cur = d1.cursors.iter().find(|c| c.session_id == "par").unwrap();
        assert_eq!(cur.file_size, real_size, "file_size 必须是真实字节数");
        assert!(
            (cur.mtime_ms - real_mtime).abs() <= 2000,
            "mtime_ms 必须是真实文件 mtime（旧实现写的是 token 累计数 100）：{}",
            cur.mtime_ms
        );
        assert_ne!(cur.mtime_ms, 100, "不得再把 token 累计数当 mtime（C3）");
        // **投影缓存的水位语义（有意为之，锁住）**：整份重写的 JSON 文档没有可用的字节水位 →
        // `byte_offset` 恒 0（配合 `(mtime,size)` 变化即从 0 全量重读）；**不得**记真实水位。
        assert_eq!(
            cur.byte_offset, 0,
            "整份重写文档不得记字节水位（否则 append 路只回吐半份 JSON）"
        );
    }

    /// **B4 回归**：原始日志侧的报错/工具/时长**必须做增量**。
    /// 旧实现每轮把 `facts` 原样再加一遍（游标既不读也不写）→ `error_model` / `error_turn` /
    /// `error_tool` / `interrupted` / `tool_calls` / `turn_ms` 随采集轮次线性膨胀。
    /// fixture 用真实代际布局：`sessions/<proj>/<dir>/session.v3.jsonl.zstd`
    /// （`zstd::stream::encode_all` 与既有 dsh 日志测试同款编码）。
    #[test]
    fn raw_log_second_round_adds_nothing() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let sess_dir = dir.path().join("sessions/proj-a/session-raw-1");
        std::fs::create_dir_all(&sess_dir).unwrap();
        let body = [
            r#"{"type":"session","version":3,"id":"raw-1","cwd":"/p/A"}"#,
            r#"{"type":"turn/start","time":1000}"#,
            r#"{"type":"tool/call","time":1100,"data":{"name":"Bash"}}"#,
            r#"{"type":"tool/result","time":1200,"data":{"error":{"code":"E1"}}}"#,
            r#"{"type":"turn/end","time":2000,"data":{"reason":{"kind":"error","error":{"code":"X"}}}}"#,
        ]
        .join("\n")
            + "\n";
        let frame = zstd::stream::encode_all(body.as_bytes(), 3).unwrap();
        std::fs::write(sess_dir.join("session.v3.jsonl.zstd"), frame).unwrap();

        // 空游标表必须先绑成变量：`&HashMap::new()` 是**临时值**，借用进 `ctx` 就是
        // E0716（temporary value dropped while borrowed）——语句结束即释放。
        let no_cursors = HashMap::new();
        let ctx1 = CollectContext::new(dir.path(), 1_700_000_000_000, &no_cursors);
        let d1 = DshCollector::collect_with_home(&ctx1, dir.path()).unwrap();
        let raw1 = d1
            .details
            .iter()
            .find(|d| d.key.session_id == "raw-1")
            .expect("原始日志必须产出报错/工具指标");
        assert_eq!(raw1.counters.error_tool, 1, "tool/result.error");
        assert_eq!(raw1.counters.error_turn, 1, "turn/end reason.kind=error");
        assert_eq!(raw1.counters.tool_calls, 1);
        assert_eq!(
            raw1.counters.turn_ms,
            vec![1000],
            "turn/start→turn/end 时长"
        );
        assert!(
            d1.cursors.iter().any(|c| c.session_id == "log:raw-1"),
            "原始日志侧必须落一条 `log:<session>` 事实水位游标（旧实现一条都没有）"
        );
        assert!(d1.new_records > 0, "首轮必须有新增（否则用例是空跑）");
        // W-50：原始日志侧记的同样是**文件数**（本轮真解码的文件数；未变代际不计）
        assert_eq!(d1.parsed_files, 1);
        // 该会话没有投影缓存 → 模型/供应商**结构性不可得**（真机 63/170 是这种文件）：
        // 计数照常入账（绝不因为拿不到模型而丢 tokens/报错），模型与供应商留空 + 未知态。
        assert_eq!(raw1.key.model, "");
        assert_eq!(raw1.key.provider, "");
        assert_eq!(raw1.provider_kind, SourceKind::Unknown);

        // 第二轮：同一份日志 → 一个事实都不能重复计
        let cursors: HashMap<_, _> = d1
            .cursors
            .iter()
            .map(|c| (c.session_id.clone(), c.clone()))
            .collect();
        let ctx2 = CollectContext::new(dir.path(), 1_700_000_000_000 + 60_000, &cursors);
        let d2 = DshCollector::collect_with_home(&ctx2, dir.path()).unwrap();
        let counters2: i64 = d2
            .details
            .iter()
            .map(|d| {
                d.counters.error_model
                    + d.counters.error_turn
                    + d.counters.error_tool
                    + d.counters.interrupted
                    + d.counters.tool_calls
                    + d.counters.turn_ms.len() as i64
            })
            .sum();
        assert_eq!(
            counters2, 0,
            "第二轮不得重复计入任何报错/工具事实（旧实现每轮再加一遍）"
        );
    }

    /// **W-30 语义锁（混合夹具）**：父/子**按文件自身身份**判定，三种真机形态同场。
    ///
    /// 真机 170 文件实测：`subagent.val.identity` 非空 **113**（本文件就是子代理会话）、
    /// `subagent.val = {}` **57**（父会话：**行在场**、值为空对象）、旧快照无该行。
    /// 「行在场 ⇒ 子代理」的退化实现会把 **170/170** 全判成子代理（真机后果：Task 18 按该位
    /// 硬过滤 → 父会话连会话数带 turn 数一起消失）；本用例把父会话钉成 `false` 即可变红。
    ///
    /// 同时钉住「token 总量含子代理」：两侧的四桶都必须照常入账（不得因为分层而丢数）。
    #[test]
    fn subagent_identity_comes_from_the_file_not_from_the_row_presence() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "p-parent",
            "/p/A",
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        projcache(
            dir.path(),
            "c-child",
            "/p/A",
            (20, 5, 0, 0),
            1,
            1,
            0,
            SubAgentRow::ChildIdentity,
            None,
        );
        projcache(
            dir.path(),
            "p-old",
            "/p/A",
            (7, 3, 0, 0),
            1,
            1,
            0,
            SubAgentRow::Missing,
            None,
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        let sub = |id: &str| {
            d1.sessions
                .iter()
                .find(|s| s.session_id == id)
                .unwrap_or_else(|| panic!("缺会话 {id}"))
                .is_subagent
        };
        assert!(
            !sub("p-parent"),
            "真机父形态 `subagent.val = {{}}` 不是子代理（行在场 ≠ 子代理）"
        );
        assert!(
            sub("c-child"),
            "`subagent.val.identity` 非空 = 本文件就是子代理会话"
        );
        assert!(!sub("p-old"), "缺 `subagent` 行的旧快照也是父会话");
        // 前提断言（防假绿）：三个文件都产出了明细行，上面的判定不是「空跑出来的 false」
        assert_eq!(d1.details.len(), 3);
        // token 总量含子代理：子会话的产出必须照常入账（分层只影响计数类，不影响四桶）
        assert_eq!(
            d1.details.iter().map(|d| d.buckets.output).sum::<i64>(),
            18,
            "10 + 5 + 3：分层不得吃掉任何一侧的 token"
        );
    }

    /// **GC 4 / W-15 锁**：投影缓存的文件读取**只经** `CollectContext::read_incremental`，
    /// 且「一遍扫描」不能只看结果对不对——把 `reads` / `bytes_read` 一起钉住：
    /// * 首轮：每文件各进一次读取入口，`bytes_read` = 各文件真实字节数之和；
    /// * 第二轮未变更：仍各进一次入口（L2 快速路），但 **零字节读取**、零新增、parsed_files 归零。
    #[test]
    fn projection_cache_reads_go_through_the_single_entry_point_once_per_round() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "s1",
            "/p/A",
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        projcache(
            dir.path(),
            "s2",
            "/p/B",
            (200, 20, 60, 0),
            2,
            2,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        let sizes: i64 = ["s1", "s2"]
            .iter()
            .map(|id| {
                std::fs::metadata(
                    dir.path()
                        .join(format!("storages/session_projcache/sessions/{id}.json")),
                )
                .unwrap()
                .len() as i64
            })
            .sum();
        let no_cursors = HashMap::new();
        let (d1, s1) = run(dir.path(), &no_cursors);
        assert_eq!(s1.reads, 2, "两个文件各进一次读取入口（GC 4：唯一入口）");
        assert_eq!(s1.bytes_read, sizes as u64, "首读必须真读（不是快速路）");
        assert_eq!(s1.repeat_reads, 0, "一遍扫描纪律");
        assert_eq!(d1.parsed_files, 2);
        assert_eq!(d1.new_records, 2, "两个文件都带来了新数据");

        let cursors = cursors_of(&d1);
        let (d2, s2) = run(dir.path(), &cursors);
        assert_eq!(s2.reads, 2, "未变文件仍各进一次入口（只是走 L2 快速路）");
        assert_eq!(
            s2.bytes_read, 0,
            "未变文件必须零字节读取（W-15：把 bytes_read 也纳入断言）"
        );
        assert_eq!(s2.repeat_reads, 0);
        assert_eq!(
            d2.parsed_files, 0,
            "未变更文件不计入「本轮真读到的文件数」（W-50）"
        );
        assert_eq!(d2.new_records, 0);
        assert!(d2.details.is_empty(), "不得重复计入");
    }

    /// **B4 补强**：日志**增长**时只计新增尾部（不是每轮重算整份），且跨轮状态真的被读回。
    #[test]
    fn log_growth_counts_only_the_new_tail() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let p = write_log(
            dir.path(),
            "proj-a",
            "raw-2",
            &[
                json!({"type":"session","version":3,"id":"raw-2","cwd":"/p/A"}),
                json!({"type":"turn/start","time":1000}),
                json!({"type":"tool/call","time":1100,"data":{"name":"Bash"}}),
                json!({"type":"turn/end","time":2000,"data":{"reason":{"kind":"completed"}}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, s1) = run(dir.path(), &no_cursors);
        let one = |d: &SourceDelta| {
            d.details
                .iter()
                .find(|x| x.key.session_id == "raw-2")
                .map(|x| {
                    (
                        x.counters.tool_calls,
                        x.counters.turn_ms.clone(),
                        x.counters.error_turn,
                    )
                })
        };
        assert_eq!(one(&d1), Some((1, vec![1000], 0)));
        assert_eq!(s1.repeat_reads, 0);
        let c1 = &d1
            .cursors
            .iter()
            .find(|c| c.session_id == "log:raw-2")
            .unwrap()
            .clone();
        assert!(
            c1.state_json.contains("\"tool_calls\":1"),
            "前提：事实水位必须落库：{}",
            c1.state_json
        );

        // 第二轮：日志追加一帧（真机 .zstd 多帧）→ 只计新增的那条工具调用与那一轮时长
        append_log(
            &p,
            &[
                json!({"type":"tool/call","time":3000,"data":{"name":"Read"}}),
                json!({"type":"turn/start","time":3000}),
                json!({"type":"turn/end","time":3500,"data":{"reason":{"kind":"aborted"}}}),
            ],
        );
        let (d2, _) = run(dir.path(), &cursors_of(&d1));
        assert_eq!(
            one(&d2),
            Some((1, vec![500], 0)),
            "新增尾部：工具 1 次 + 本轮 500ms；aborted 计入 interrupted 而不是 error_turn"
        );
        assert_eq!(
            d2.details
                .iter()
                .map(|x| x.counters.interrupted)
                .sum::<i64>(),
            1
        );
        // 第三轮：文件未变 → 零新增（L2 未变代际**不重复解码、也不重复解析**）
        let (d3, s3) = run(dir.path(), &cursors_of(&d2));
        assert!(d3.details.is_empty(), "未变代际不得再产出任何事实");
        assert_eq!(d3.new_records, 0);
        assert_eq!(s3.repeat_reads, 0);
        assert_eq!(
            d3.parsed_files, 0,
            "**裁决 C（fix round 1）**：未变代际的日志必须零解码零解析 —— 摘掉日志侧 L2 \
             （每轮直接 read_best_generation）时它会变成 1（W-15：只断「值对了」没断「路径对了」）"
        );
    }

    /// **D-41 同族（跨轮持久化模型 / 供应商）**：原始日志侧**不产生**模型字段，
    /// 若某轮投影缓存不可得（被清理 / 缺行），不得把该会话的报错/工具事实落进
    /// `model=""` 幽灵行——必须沿用**上一轮已知**的模型与供应商。
    ///
    /// 触发面是真的：投影缓存是**缓存**（可被清理），而原始日志才是权威；
    /// 而本机 63/170 个投影文件**结构性地**没有 `modelSelection` 行（不是「本轮还没读到」）。
    #[test]
    fn log_side_model_survives_a_missing_projection_cache() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "s1",
            "/p/A",
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            Some(("ollama-pro", "deepseek-v4.1-flash")),
        );
        let p = write_log(
            dir.path(),
            "proj-a",
            "s1",
            &[
                json!({"type":"session","version":3,"id":"s1","cwd":"/p/A"}),
                json!({"type":"tool/call","time":1100,"data":{"name":"Bash"}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        let rows: Vec<_> = d1
            .details
            .iter()
            .filter(|d| d.key.session_id == "s1")
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "投影缓存与原始日志同一会话同一小时 → 合并成一行"
        );
        assert_eq!(rows[0].key.model, "deepseek-v4.1-flash");
        assert_eq!(rows[0].key.provider, "ollama-pro");
        assert_eq!(rows[0].provider_kind, SourceKind::Measured);
        assert_eq!(rows[0].counters.tool_calls, 1);

        // 第二轮：投影缓存被清理（缓存可被清），日志又长了一条工具调用
        std::fs::remove_file(
            dir.path()
                .join("storages/session_projcache/sessions/s1.json"),
        )
        .unwrap();
        append_log(
            &p,
            &[json!({"type":"tool/call","time":3000,"data":{"name":"Read"}})],
        );
        let (d2, _) = run(dir.path(), &cursors_of(&d1));
        let rows2: Vec<_> = d2
            .details
            .iter()
            .filter(|d| d.key.session_id == "s1")
            .collect();
        assert_eq!(
            rows2.len(),
            1,
            "不得落出第二条（空模型）幽灵行：{:?}",
            rows2.iter().map(|r| r.key.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            rows2[0].key.model, "deepseek-v4.1-flash",
            "投影缓存不可得时必须沿用游标里跨轮持久化的模型（D-41：绝不写 model=\"\" 行）"
        );
        assert_eq!(rows2[0].key.provider, "ollama-pro");
        assert_eq!(rows2[0].counters.tool_calls, 1);
    }

    /// **GC 10（隐私白名单）锁**：原始日志里含 prompt 正文 / 工具参数原文，
    /// 账本里**只允许**结构化数字与静态文本（会话 id / 项目路径 / 模型名）。
    /// 本用例把正文哨兵塞进日志的三个真实位置（`turnOutline` 行、`tool/call.arguments`、
    /// `user/message` 文本），再把整份 `SourceDelta` 序列化，断言**一个字节都没有漏进来**。
    #[test]
    fn prompt_text_never_reaches_the_ledger() {
        let _serial = serial();
        const SECRET: &str = "PROMPT-BODY-SECRET-9f2c";
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "s1",
            "/p/A",
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        write_log(
            dir.path(),
            "proj-a",
            "s1",
            &[
                json!({"type":"session","version":3,"id":"s1","cwd":"/p/A"}),
                json!({"type":"user/message","time":10,"data":{"text":SECRET}}),
                json!({"type":"tool/call","time":1100,"data":{"name":"Bash","callId":"c1","arguments":SECRET}}),
                json!({"type":"tool/result","time":1200,"data":{"toolCallId":"c1","error":{"name":"X","code":"E1"}}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        assert!(
            d1.new_records > 0,
            "前提：本轮真的有产出（否则本用例是空跑）"
        );
        let dump = format!("{:?}", d1.details)
            + &format!("{:?}", d1.sessions)
            + &format!("{:?}", d1.cursors);
        assert!(
            !dump.contains("PROMPT-BODY-SECRET"),
            "prompt 正文 / 工具参数原文**不得**进入任何字段（GC 10）"
        );
        assert!(
            d1.sessions.iter().all(|s| s.title.is_none()),
            "标题也不采（投影缓存的 title/titleInput 一律不读）"
        );
    }

    /// **边界**：dsh 未安装（home 下既无投影缓存也无会话日志）→ 零产物的 `Ok`，
    /// **不是**失败（该源在 `UsageSourceStatus` 上是 `ok=true` + 零文件）。
    #[test]
    fn no_dsh_home_yields_a_zero_product_delta() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let no_cursors = HashMap::new();
        let (d, s) = run(dir.path(), &no_cursors);
        assert!(d.details.is_empty());
        assert!(d.sessions.is_empty());
        assert!(d.cursors.is_empty());
        assert_eq!(d.parsed_files, 0);
        assert_eq!(d.new_records, 0);
        assert_eq!(s.reads, 0, "没有文件 → 一次读取都不该发生");
        assert_eq!(s.bytes_read, 0);
    }

    /// **边界**：原始日志解不开（截断 / 非 zstd / 空文件）→ 该文件不产出指标，
    /// 但**不得冻结整源**：同一轮里其它会话的日志照常入账。
    /// （与 D-24 的取舍一致：一个坏文件不该让其余文件本轮不入账；但也不能静默。）
    #[test]
    fn undecodable_log_is_skipped_without_freezing_the_other_sessions() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let bad_dir = dir.path().join("sessions/proj-a/broken");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(
            bad_dir.join("session.v3.jsonl.zstd"),
            b"not a zstd frame at all",
        )
        .unwrap();
        write_log(
            dir.path(),
            "proj-a",
            "good",
            &[
                json!({"type":"session","version":3,"id":"good","cwd":"/p/A"}),
                json!({"type":"tool/call","time":1100,"data":{"name":"Bash"}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d, _) = run(dir.path(), &no_cursors);
        assert!(
            d.details.iter().any(|x| x.key.session_id == "good"),
            "坏文件不得冻结整源：好的会话必须照常入账"
        );
        assert!(
            !d.details.iter().any(|x| x.key.session_id == "broken"),
            "解不开的日志不得产出任何指标（按空态，不填 0 —— GC 7）"
        );
        assert_eq!(d.parsed_files, 1, "解不开的文件不算「本轮真读到的文件」");
    }

    /// **W-19 锁**：`SessionFileScan` 的 namespace 必须由 `scan_namespace(sid)` **派生**
    /// （不得手写字符串字面量），两套缓存（投影缓存 / 原始日志）之间也必须各自独立——
    /// 同 ns 混存两种产物类型会让 `Arc::downcast` 失配、缓存静默失效（GC 5 的原始动机）。
    #[test]
    fn scan_namespace_is_derived_from_the_source_id() {
        let _serial = serial();
        assert_eq!(SCAN_NAMESPACE, scan_namespace(UsageSourceId::Dsh));
        assert_eq!(SCAN_NAMESPACE, "usage-dsh");
        assert_eq!(
            log_scan_namespace(),
            format!("{}-log", scan_namespace(UsageSourceId::Dsh)),
            "原始日志侧的 ns 必须与 scan_namespace 的值自洽（W-19 / D-1 先例）"
        );
        assert_ne!(log_scan_namespace(), SCAN_NAMESPACE, "两套缓存不得同 ns");
        assert!(log_scan_namespace().starts_with("usage-"));
        // 不得复用 monitor/ 既有 ns（同 ns 混存会让 downcast 失配）
        assert_ne!(log_scan_namespace(), "dsh-log");
        assert_ne!(SCAN_NAMESPACE, scan_namespace(UsageSourceId::Claude));
    }

    /// **D-24 / D-38 同族**：单文件读失败里只有「**文件已消失**」这一种可以跳过，
    /// 其余（权限 / IO 故障 / mtime 不可得）必须**整源响亮失败** —— 落进
    /// `UsageSourceStatus.errorCode`，否则用户在 UI 上看到的是「这个源没有用量」，
    /// 而不是「这个源读不了」。
    #[cfg(unix)]
    #[test]
    fn unreadable_projection_cache_fails_the_whole_source_loudly() {
        let _serial = serial();
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        projcache(
            dir.path(),
            "s1",
            "/p/A",
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        let p = dir
            .path()
            .join("storages/session_projcache/sessions/s1.json");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o000)).unwrap();
        // 前提断言（防假绿）：权限真的生效了——本进程读它必须失败
        assert!(std::fs::read(&p).is_err(), "前提：0000 权限下必须读不了");
        let no_cursors = HashMap::new();
        let ctx = CollectContext::new(dir.path(), 1_700_000_000_000, &no_cursors);
        let err = DshCollector::collect_with_home(&ctx, dir.path()).unwrap_err();
        assert_eq!(
            err.code, "usage-source-io",
            "读不了必须整源响亮失败：{err:?}"
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).ok();
    }

    /// **继承的盲区（如实登记，不是本任务新缺陷）**：投影缓存走 `read_incremental`，
    /// 而它的快速路 `file_unchanged` 只比 `(mtime_ms, size)` —— **同一毫秒内 + 同一大小**的
    /// 重写会被判「未变」→ 整轮零字节读取、增量漏掉。这是 stream/cursor 层的**已登记盲区**
    ///（`cursor.rs` 模块文档「已知盲区」+ `stream.rs` 的同名锁），本采集器**继承**它。
    ///
    /// 生产暴露面的量级（模块头同款分析）：要漏掉一次重写，必须让「上一轮游标里记录的 mtime」
    /// 与本轮的重写时刻落在**同一毫秒** —— 即采集读该文件的那一刻与前后两次写入全挤进同一毫秒；
    /// 采集轮次间隔是分钟级，概率 ~0。**夹具纪律**：不要在亚毫秒内做「同尺寸重写」，
    /// 否则会随机踩中它（本任务 T16 的第一版夹具就是这么 flake 的）。
    ///
    /// 本用例用 `set_modified` **确定性地**复现该盲区，把它写死（免得将来有人以为它是新 bug，
    /// 也免得有人用「夹具恰好不触发」当成它不存在）。要彻底关掉它只能改**冻结的**增量读契约，
    /// 或给投影缓存另起一套「每轮必读」的入口（后者违反 GC 4 的唯一入口）。
    #[test]
    fn same_millisecond_same_size_rewrite_is_the_inherited_fast_path_blind_spot() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        projcache_full(
            dir.path(),
            "s1",
            "/p/A",
            1,
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        let path = dir
            .path()
            .join("storages/session_projcache/sessions/s1.json");
        let first_len = std::fs::metadata(&path).unwrap().len();
        let fixed =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_000);
        let set_mtime = |p: &std::path::Path, t: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        set_mtime(&path, fixed);
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        assert_eq!(d1.details.len(), 1, "前提：首轮必须产出明细行");

        // 同尺寸重写（数字位数完全相同）+ **同一毫秒** mtime
        projcache_full(
            dir.path(),
            "s1",
            "/p/A",
            1,
            (150, 25, 60, 0),
            2,
            3,
            0,
            SubAgentRow::ParentEmpty,
            None,
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            first_len,
            "前提：两次写入必须**同尺寸**（否则踩不到这条快速路）"
        );
        set_mtime(&path, fixed);
        let (d2, s2) = run(dir.path(), &cursors_of(&d1));
        assert_eq!(
            s2.bytes_read, 0,
            "前提：命中 (mtime,size) 快速路（零字节读取）"
        );
        assert!(
            d2.details.is_empty(),
            "继承的盲区：同毫秒 + 同尺寸的重写会被判「未变」→ 本轮不计增量（已登记，非新缺陷）"
        );
    }

    /// **裁决 A（fix round 1，`plan-mandated`）**：dsh 的**逐工具**维度必须落地 ——
    /// spec §5.1 dsh 行把「逐工具耗时」与报错次数、最长单 turn **并列**为「投影缓存拿不到、
    /// 必须读原始日志」，§8.6 记它属**部分覆盖**（`available=true + reason=部分覆盖`）。
    /// 配对键是 `tool/call.data.callId` ↔ **`tool/result.data.message.source.callId`**
    ///（真机 173 文件全量 14,584/14,584 都是这一层；**不是同一个字段名、也不是同一层**；
    /// 旧样本 `data.message.toolCallId` 与任务书写的顶层 `data.toolCallId` 只是兜底），
    /// 耗时 = `tool/result.time − tool/call.time`；两者就在**同一次**扫描里（GC 4：绝不二次读取）。
    ///
    /// 两条不变量：① `Σ tool_stats.0 == tool_calls`（兄弟源同款，也防「0 次调用却有耗时」）；
    /// ② **绝不只填次数**（那会让该工具的 ms 变成假 0，GC 7）；配不上回执的只计次数、不计耗时。
    #[test]
    fn tool_stats_carries_counts_and_durations_and_sums_to_tool_calls() {
        let _serial = serial();
        const T0: i64 = 1_680_000_000_000;
        let dir = tempfile::tempdir().unwrap();
        write_log(
            dir.path(),
            "proj-a",
            "raw-3",
            &[
                json!({"type":"session","version":3,"id":"raw-3","cwd":"/p/A"}),
                json!({"type":"tool/call","time":T0,"data":{"name":"Bash","callId":"c1","arguments":"ls"}}),
                json!({"type":"tool/call","time":T0 + 1_000,"data":{"name":"Read","callId":"c2"}}),
                json!({"type":"tool/call","time":T0 + 3_000,"data":{"name":"Edit","callId":"c3"}}),
                json!({"type":"tool/call","time":T0 + 4_000,"data":{"name":"Write","callId":"c4"}}),
                // **真机主形态**（173 文件全量 14,584/14,584）：键在
                // `data.message.source.callId`，**不在** data 顶层 ——
                // 任务书正文写的顶层 `data.toolCallId` 真机**一个都没有**（D-29/D-36 同族）
                json!({"type":"tool/result","time":T0 + 500,
                       "data":{"message":{"role":"user","source":{"kind":"tool","callId":"c1"},"content":[]}}}),
                // 旧样本形态（探测文档里的 v3-era，真机 72 条）：`data.message.toolCallId`
                json!({"type":"tool/result","time":T0 + 1_250,
                       "data":{"message":{"role":"tool","toolCallId":"c2"}}}),
                // 任务书正文的扁平形态（兜底）：`data.toolCallId`
                json!({"type":"tool/result","time":T0 + 3_100,"data":{"toolCallId":"c3"}}),
                // 第四次调用**没有回执**：次数照记、耗时不计（不写假 0）
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        let row = d1
            .details
            .iter()
            .find(|x| x.key.session_id == "raw-3")
            .expect("原始日志必须产出逐工具指标");
        assert_eq!(row.counters.tool_calls, 4);
        assert_eq!(
            row.counters.tool_stats.get("Bash").copied(),
            Some((1, 500)),
            "真机主形态 `data.message.source.callId` 必须配上（顶层 `data.toolCallId` 一个都没有）"
        );
        assert_eq!(
            row.counters.tool_stats.get("Read").copied(),
            Some((1, 250)),
            "旧样本形态 `data.message.toolCallId` 兜底"
        );
        assert_eq!(
            row.counters.tool_stats.get("Edit").copied(),
            Some((1, 100)),
            "任务书扁平形态 `data.toolCallId` 兜底"
        );
        assert_eq!(
            row.counters.tool_stats.get("Write").copied(),
            Some((1, 0)),
            "配不上回执的调用：**次数照记、耗时不计**（宁可少一个耗时，不写假 0）"
        );
        assert_eq!(
            row.counters
                .tool_stats
                .values()
                .map(|(n, _)| *n)
                .sum::<i64>(),
            row.counters.tool_calls,
            "Σ tool_stats.0 必须等于 tool_calls（兄弟源同款不变量）"
        );
        // 前提断言（防假绿）：耗时真的配对算出来了（不是恒 0 的退化实现）
        assert_eq!(
            row.counters
                .tool_stats
                .values()
                .map(|(_, ms)| *ms)
                .sum::<i64>(),
            850,
            "500 + 250 + 100（三种回执形态各配一条；无回执的那条不计）"
        );
        assert_eq!(row.counters.tool_stats.len(), 4, "四个工具各一条");
    }

    /// **fix round 2 裁决 A（行为性：静默永久丢数）**：迟到回执的逐工具耗时**必须**落行。
    ///
    /// 形态（真机必然发生：单工具最长 3.1 h）：第 N 轮只有 `tool/call`（无回执）→ 记 `(1,0)`
    /// 并推进水位；第 N+1 轮**只有它的回执**到达（成功工具 → 无 `error_tool`、turn 未结束、
    /// 无新调用）⇒ `new_total == 0` —— 旧闸门 `if new_total > 0` 会**不落行**，而水位已把这段
    /// ms 记走 → 第 N+2 轮 `dm` 已是 0 ⇒ **该次耗时永久丢失**。
    ///
    /// 锁：三轮夹具，断言该工具的耗时**恰好出现一次**（少了 = 丢；多了 = 重），
    /// 且三轮的 ms 之和恒等于真实耗时。
    #[test]
    fn late_result_duration_survives_the_new_total_gate() {
        let _serial = serial();
        const T0: i64 = 1_680_000_000_000;
        let dir = tempfile::tempdir().unwrap();
        let p = write_log(
            dir.path(),
            "proj-a",
            "raw-6",
            &[
                json!({"type":"session","version":3,"id":"raw-6","cwd":"/p/A"}),
                json!({"type":"tool/call","time":T0,"data":{"name":"Bash","callId":"c1"}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        let bash = |d: &SourceDelta| {
            d.details
                .iter()
                .find_map(|x| x.counters.tool_stats.get("Bash").copied())
        };
        assert_eq!(bash(&d1), Some((1, 0)), "第 N 轮：只有调用、没有回执");
        assert_eq!(
            d1.details.len(),
            1,
            "前提：这一轮有事实（tool_calls）→ 落行"
        );

        // 第 N+1 轮：回执到达，**它是本轮唯一新增事实**（new_total == 0）
        append_log(
            &p,
            &[json!({"type":"tool/result","time":T0 + 500,
                     "data":{"message":{"role":"user","source":{"kind":"tool","callId":"c1"},"content":[]}}})],
        );
        let (d2, _) = run(dir.path(), &cursors_of(&d1));
        assert_eq!(
            d2.details.len(),
            1,
            "迟到回执必须落行（旧闸门只看 new_total 时这里会是 0 行 → ms 永久丢失）"
        );
        assert_eq!(
            bash(&d2),
            Some((0, 500)),
            "这一行只带耗时（次数差值 0），耗时 500ms"
        );
        assert_eq!(
            d2.new_records, 0,
            "`new_records` 仍按**事实数**：本轮没有新事实"
        );

        // 第 N+2 轮：turn/end（新的报错/时长事实）→ **不得**把那段 ms 再算一遍
        append_log(
            &p,
            &[
                json!({"type":"turn/start","time":T0 + 600}),
                json!({"type":"turn/end","time":T0 + 1_100,"data":{"reason":{"kind":"completed"}}}),
            ],
        );
        let (d3, _) = run(dir.path(), &cursors_of(&d2));
        assert_eq!(
            bash(&d3),
            None,
            "耗时已经在上一条 delta 里给过 → 这一轮不得再给（多了 = 重复计）"
        );
        // 「恰好一次」的等价表述：三轮里给出 ms 的轮数 == 1，且总量 == 500
        let with_ms = [&d1, &d2, &d3]
            .iter()
            .filter(|d| {
                d.details
                    .iter()
                    .any(|x| x.counters.tool_stats.values().any(|(_, ms)| *ms != 0))
            })
            .count();
        let total_ms: i64 = [&d1, &d2, &d3]
            .iter()
            .flat_map(|d| d.details.iter())
            .map(|x| {
                x.counters
                    .tool_stats
                    .values()
                    .map(|(_, ms)| *ms)
                    .sum::<i64>()
            })
            .sum();
        assert_eq!(
            with_ms, 1,
            "该工具的耗时必须**恰好出现一次**（少了=丢、多了=重）"
        );
        assert_eq!(total_ms, 500, "三轮合计恒等于真实耗时");
    }

    /// **裁决 B（fix round 1）**：首轮（账本为空）那一笔是**全部历史累计**，必须归到会话自己的
    /// `record.identity.createdAt` 桶，**不是采集时刻** —— 否则用户第一次跑就把整个历史
    /// （只读冒烟实测 input 53.1M / cache_read 2.64G）塞进「当轮那一小时」，
    /// hero（近 24 小时）与小时趋势被历史淹没，而且这些行**永久**留在错的小时。
    /// 第二轮只带增量 → 归**当轮**（模块头与报告「偏差申报」已点名这一处有意使用 `ctx.now_ms`）。
    #[test]
    fn first_round_buckets_by_created_at_not_collection_time() {
        let _serial = serial();
        const CREATED: i64 = 1_690_000_000_000;
        const NOW: i64 = 1_700_000_000_000;
        let h_created = hour_key_of(CREATED, &TZ);
        let h_now = hour_key_of(NOW + 60_000, &TZ);
        // 前提断言（防假绿）：两个候选桶必须真的不同（否则本用例在任何时区都不可伪证）
        assert_ne!(
            h_created, h_now,
            "前提：createdAt 与采集时刻必须分处不同小时"
        );

        let dir = tempfile::tempdir().unwrap();
        projcache_full(
            dir.path(),
            "s1",
            "/p/A",
            CREATED,
            (100, 10, 50, 0),
            1,
            1,
            0,
            SubAgentRow::ParentEmpty,
            Some(("ollama-pro", "deepseek-v4.1-flash")),
        );
        let no_cursors = HashMap::new();
        let ctx = CollectContext::new(dir.path(), NOW, &no_cursors);
        let d1 = DshCollector::collect_with_home(&ctx, dir.path()).unwrap();
        let row = d1
            .details
            .iter()
            .find(|x| x.key.session_id == "s1")
            .unwrap();
        assert_eq!(
            row.key.hour_key, h_created,
            "首轮的历史累计必须归会话 createdAt 桶（按采集时刻归桶会落在 {h_now}）"
        );
        // 前提断言：游标里记的是**真实** mtime（归桶的第二顺位来源就是它）
        let c1 = d1.cursors.iter().find(|c| c.session_id == "s1").unwrap();
        assert!(c1.mtime_ms > 0, "游标必须带真实 mtime");
        // **fix round 2 Minor#3**：会话维度的 `last_seen_at` 必须用**文件真实 mtime**，
        // 不是 `ts`（首轮的 ts = createdAt）—— DAO 取 MAX，一旦把创建时刻写成「最后见到」
        // 就再也回不来，此后不再变化的会话会永远停在创建时刻。
        let md = std::fs::metadata(
            dir.path()
                .join("storages/session_projcache/sessions/s1.json"),
        )
        .unwrap();
        let real_mtime = md
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let sess = d1.sessions.iter().find(|x| x.session_id == "s1").unwrap();
        assert_eq!(
            sess.last_seen_at, real_mtime,
            "last_seen_at 必须是该投影缓存文件的真实 mtime"
        );
        assert_ne!(
            sess.last_seen_at, CREATED,
            "不得把会话创建时刻当成「最后见到」（DAO MAX 一旦写入就回不来）"
        );

        // 第二轮：文件更新（只带增量）→ 归**当轮**。
        // 注意：这里的数字**故意换成长度不同的**（1500/250/600、turns 11、steps 13）——
        // 投影缓存走 `read_incremental`，而它的快速路只比 `(mtime_ms, size)`：
        // 「同毫秒 + 同尺寸」的重写会被判「未变」而整轮跳过（继承的已登记盲区，
        // 见模块头与 `same_millisecond_same_size_rewrite_is_the_inherited_fast_path_blind_spot`）。
        // 本用例要锁的是**归桶**，不该被那个盲区搅成 flake。
        projcache_full(
            dir.path(),
            "s1",
            "/p/A",
            CREATED,
            (1500, 250, 600, 0),
            11,
            13,
            0,
            SubAgentRow::ParentEmpty,
            Some(("ollama-pro", "deepseek-v4.1-flash")),
        );
        // 借用不能挂在临时值上（E0716）：先绑变量
        let cursors1 = cursors_of(&d1);
        let ctx2 = CollectContext::new(dir.path(), NOW + 60_000, &cursors1);
        let d2 = DshCollector::collect_with_home(&ctx2, dir.path()).unwrap();
        let row2 = d2
            .details
            .iter()
            .find(|x| x.key.session_id == "s1")
            .unwrap();
        assert_eq!(
            row2.key.hour_key, h_now,
            "增量归**当轮**（有意为之：把增量也钉在创建时刻会让会话寿命越长、趋势越歪）"
        );
        assert_ne!(row2.key.hour_key, h_created, "增量不得再落回创建桶");
    }

    /// **裁决 B（原始日志侧）**：归桶用**日志自己的时间** —— 最后一条带时间的事件
    /// （退路是该代际文件的真实 mtime），**不是采集时刻**。
    #[test]
    fn log_facts_bucket_by_the_last_event_time_not_collection_time() {
        let _serial = serial();
        const EVENT: i64 = 1_680_000_000_000;
        const NOW: i64 = 1_700_000_000_000;
        let h_event = hour_key_of(EVENT, &TZ);
        assert_ne!(
            h_event,
            hour_key_of(NOW, &TZ),
            "前提：事件时刻与采集时刻必须分处不同小时"
        );
        let dir = tempfile::tempdir().unwrap();
        write_log(
            dir.path(),
            "proj-a",
            "raw-4",
            &[
                json!({"type":"session","version":3,"id":"raw-4","cwd":"/p/A"}),
                json!({"type":"tool/call","time":EVENT - 2_000,"data":{"name":"Bash","callId":"c1"}}),
                json!({"type":"tool/result","time":EVENT - 1_000,"data":{"toolCallId":"c1"}}),
                json!({"type":"turn/start","time":EVENT - 500}),
                json!({"type":"turn/end","time":EVENT,"data":{"reason":{"kind":"completed"}}}),
            ],
        );
        let no_cursors = HashMap::new();
        let ctx = CollectContext::new(dir.path(), NOW, &no_cursors);
        let d1 = DshCollector::collect_with_home(&ctx, dir.path()).unwrap();
        let row = d1
            .details
            .iter()
            .find(|x| x.key.session_id == "raw-4")
            .unwrap();
        assert_eq!(row.counters.tool_calls, 1, "前提：本轮真的产出了事实");
        assert_eq!(
            row.key.hour_key,
            h_event,
            "日志侧事实必须归日志自己的时间（用采集时刻会落在 {}）",
            hour_key_of(NOW, &TZ)
        );
    }

    /// **M3 / W-49（fix round 1）**：日志水位按**代际**记（真机 7/172 个会话目录有多代际文件）。
    /// ① 新代际是旧水位的**超集** → 差值恰好是「切换期间新增的事件」，照常计（不得整份丢掉）；
    /// ② 新代际**不是**超集（截断 / 重建）→ 只对齐水位、本轮一个事实都不计
    ///   （旧语义「回退按整份重计」会把新文件里已有的全部报错/工具/样本再计一遍）；
    /// ③ 对齐之后水位要能继续正常工作（不是冻死）。
    #[test]
    fn generation_switch_counts_a_superset_tail_and_aligns_a_non_superset() {
        let _serial = serial();
        const T0: i64 = 1_680_000_000_000;
        let dir = tempfile::tempdir().unwrap();
        let sess_dir = dir.path().join("sessions/proj-a/raw-5");
        let v3 = write_log(
            dir.path(),
            "proj-a",
            "raw-5",
            &[
                json!({"type":"session","version":3,"id":"raw-5","cwd":"/p/A"}),
                json!({"type":"tool/call","time":T0,"data":{"name":"Bash","callId":"c1"}}),
                json!({"type":"tool/result","time":T0 + 500,"data":{"toolCallId":"c1"}}),
            ],
        );
        let no_cursors = HashMap::new();
        let (d1, _) = run(dir.path(), &no_cursors);
        assert_eq!(
            d1.details
                .iter()
                .map(|x| x.counters.tool_calls)
                .sum::<i64>(),
            1
        );
        let c1 = d1
            .cursors
            .iter()
            .find(|c| c.session_id == "log:raw-5")
            .expect("前提：必须落日志侧水位")
            .clone();
        assert!(
            c1.state_json.contains("\"gen\":3"),
            "前提：水位必须记住代际号：{}",
            c1.state_json
        );

        // ① v4 = v3 的内容 + 一条新工具调用（**超集**）→ 只计新增的那一条
        let v4 = sess_dir.join("session.v4.jsonl.zstd");
        let mut body = std::fs::read(&v3).unwrap();
        body.extend(
            zstd::stream::encode_all(
                log_body(&[
                    json!({"type":"tool/call","time":T0 + 2_000,"data":{"name":"Read","callId":"c2"}}),
                    json!({"type":"tool/result","time":T0 + 2_500,"data":{"toolCallId":"c2"}}),
                ])
                .as_bytes(),
                3,
            )
            .unwrap(),
        );
        std::fs::write(&v4, body).unwrap();
        let (d2, _) = run(dir.path(), &cursors_of(&d1));
        assert_eq!(
            d2.details
                .iter()
                .map(|x| x.counters.tool_calls)
                .sum::<i64>(),
            1,
            "代际切换 + 超集：只计「切换期间新增的事件」，旧事实一个都不能重计"
        );
        assert_eq!(
            d2.details
                .iter()
                .find_map(|x| x.counters.tool_stats.get("Read").copied()),
            Some((1, 500)),
            "新代际里新增的工具调用（含耗时）必须照常入账"
        );

        // ② v5 = **更小**的文件（非超集）→ 只对齐水位、不计任何事实
        let v5 = sess_dir.join("session.v5.jsonl.zstd");
        std::fs::write(
            &v5,
            zstd::stream::encode_all(
                log_body(&[
                    json!({"type":"session","version":5,"id":"raw-5","cwd":"/p/A"}),
                    json!({"type":"tool/call","time":T0,"data":{"name":"Bash","callId":"c1"}}),
                ])
                .as_bytes(),
                3,
            )
            .unwrap(),
        )
        .unwrap();
        let (d3, _) = run(dir.path(), &cursors_of(&d2));
        assert!(
            d3.details.is_empty(),
            "非超集的代际切换：本轮不得计任何事实"
        );
        assert_eq!(d3.new_records, 0);

        // ③ 对齐之后水位继续可用：v6 = v5 的内容 + 一条新调用（超集）→ 只计新增那一条
        let v6 = sess_dir.join("session.v6.jsonl.zstd");
        let mut body = std::fs::read(&v5).unwrap();
        body.extend(
            zstd::stream::encode_all(
                log_body(&[
                    json!({"type":"tool/call","time":T0 + 5_000,"data":{"name":"Write","callId":"c9"}}),
                    json!({"type":"tool/result","time":T0 + 5_100,"data":{"toolCallId":"c9"}}),
                ])
                .as_bytes(),
                3,
            )
            .unwrap(),
        );
        std::fs::write(&v6, body).unwrap();
        let (d4, _) = run(dir.path(), &cursors_of(&d3));
        assert_eq!(
            d4.details
                .iter()
                .map(|x| x.counters.tool_calls)
                .sum::<i64>(),
            1,
            "对齐之后水位必须继续正常推进（不是冻死）"
        );
        assert_eq!(
            d4.details
                .iter()
                .find_map(|x| x.counters.tool_stats.get("Write").copied()),
            Some((1, 100))
        );
    }

    /// **M2（fix round 1）**：目录级读失败不得被静默吞。
    /// `NotFound` = 未装 dsh / 从未落盘（正常空态）；其余（权限 / IO）= 「这个源读不了」
    /// → **整源响亮失败**（落 `UsageSourceStatus.errorCode`，与 D-24 同口径）。
    #[cfg(unix)]
    #[test]
    fn unreadable_root_directories_fail_the_whole_source_loudly() {
        let _serial = serial();
        use std::os::unix::fs::PermissionsExt;
        let no_cursors = HashMap::new();

        // ① 投影缓存目录列不出来
        let dir = tempfile::tempdir().unwrap();
        let pc = dir.path().join("storages/session_projcache/sessions");
        std::fs::create_dir_all(&pc).unwrap();
        std::fs::set_permissions(&pc, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            std::fs::read_dir(&pc).is_err(),
            "前提：0000 权限下目录必须列不出来"
        );
        let ctx = CollectContext::new(dir.path(), 1_700_000_000_000, &no_cursors);
        let err = DshCollector::collect_with_home(&ctx, dir.path()).unwrap_err();
        assert_eq!(
            err.code, "usage-source-io",
            "投影缓存目录读不了必须整源失败：{err:?}"
        );
        std::fs::set_permissions(&pc, std::fs::Permissions::from_mode(0o755)).ok();

        // ② 会话日志根目录列不出来（投影缓存不存在 → 正常空态，必须继续走到 B 段）
        let dir2 = tempfile::tempdir().unwrap();
        let root = dir2.path().join("sessions");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            std::fs::read_dir(&root).is_err(),
            "前提：0000 权限下目录必须列不出来"
        );
        let ctx2 = CollectContext::new(dir2.path(), 1_700_000_000_000, &no_cursors);
        let err2 = DshCollector::collect_with_home(&ctx2, dir2.path()).unwrap_err();
        assert_eq!(
            err2.code, "usage-source-io",
            "会话日志根目录读不了必须整源失败：{err2:?}"
        );
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).ok();
    }

    /// **结构性锁（fix round 2 实测教训）**：本文件**每一个** `#[test]` 的函数体都必须
    /// 以 `let _serial = serial();` 开头 —— 日志侧 L2 是进程级全局注册表，任何调用
    /// `collect` 的用例都会经 `retain_existing(&live)` 清掉**同 namespace 里不在它本轮
    /// live 集合中的条目**；漏掉锁的用例会把**并行跑的**用例的缓存条目清掉 →
    /// 对方的「未变代际零解码」断言随机变红（fix round 2 就是这样偶发红 2/15：新加的
    /// `late_result_duration_survives_the_new_total_gate` 漏了锁）。
    /// 用**源码自省**把这条纪律变成机械锁：漏一个 → 本用例立刻红。
    #[test]
    fn every_test_in_this_file_takes_the_serial_lock() {
        let _serial = serial();
        let src = include_str!("dsh.rs");
        let lines: Vec<&str> = src.lines().collect();
        let mut missing: Vec<usize> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() != "#[test]" {
                continue;
            }
            let window = lines.iter().skip(i + 1).take(4);
            if !window.into_iter().any(|l| l.contains("serial()")) {
                missing.push(i + 1);
            }
        }
        assert!(
            missing.is_empty(),
            "这些 #[test]（行号）没有取串行锁 `let _serial = serial();`：{missing:?}"
        );
    }

    /// **W-19 措辞更正（M5，fix round 1）**：本用例只是「派生值两两不撞」的**自证**
    /// （`scan_namespace` = `"usage-"+db_id()`、`db_id` 互异 ⇒ 必然成立），
    /// 它**抓不到**「给 `SessionFileScan::new` 传硬编码 ns」。真正生效的是每个采集器文件里
    /// 自己的 `assert_eq!(SCAN_NAMESPACE, scan_namespace(sid))` 与 `log_scan_namespace()` 的自洽断言。
    #[test]
    fn all_seven_namespaces_are_distinct_and_prefixed() {
        let _serial = serial();
        let nss: Vec<&str> = UsageSourceId::ALL
            .iter()
            .map(|s| scan_namespace(*s))
            .collect();
        for ns in &nss {
            assert!(
                ns.starts_with("usage-"),
                "namespace 必须带 usage- 前缀：{ns}"
            );
        }
        let uniq: std::collections::HashSet<&&str> = nss.iter().collect();
        assert_eq!(
            uniq.len(),
            nss.len(),
            "七源 namespace 必须互不相同：{nss:?}"
        );
    }

    /// **登记锁（独立验证复核 §3.2.2 阻塞 4）**：漏登记不是编译错——`run_collection`
    /// 只遍历 `collectors::all()`，这个源会被静默跳过。
    #[test]
    fn collector_is_registered_in_all() {
        let _serial = serial();
        let ids: Vec<UsageSourceId> = crate::services::usage::collectors::all()
            .iter()
            .map(|c| c.source_id())
            .collect();
        assert!(
            ids.contains(&UsageSourceId::Dsh),
            "dsh 采集器必须登记进 collectors::all()，实际登记：{ids:?}"
        );
    }

    /// **全表锁（§3.2.2 阻塞 4 + W-19 + W-27）**：
    /// ① 七源必须**齐全**且**顺序 = `UsageSourceId::ALL`**（= 契约 `UsageSourceStatus` 的展示顺序）；
    /// ② 逐源断言 `c.caps() == caps_of(c.source_id())`（**W-27**：把「不许覆写 caps」从文档约束
    ///    变成机械锁——`caps.rs` 明写「采集器与查询层共用同一张表，杜绝两边漂移」，
    ///    而可覆写点正是漂移的唯一入口）；
    /// ③（W-19 的 ns 断言已拆到 `all_seven_namespaces_are_distinct_and_prefixed`，M5：措辞降级）。
    /// 少一个源在 `run_collection` 里不会报错，只会让 Task 24 的人工验收面少一行、
    /// Task 23B 的 `sources.len() == 7` 变红。
    #[test]
    fn all_seven_collectors_are_registered_in_order() {
        let _serial = serial();
        let cols = crate::services::usage::collectors::all();
        let ids: Vec<UsageSourceId> = cols.iter().map(|c| c.source_id()).collect();
        assert_eq!(
            ids,
            UsageSourceId::ALL.to_vec(),
            "登记表必须恰好是七个源且顺序与 UsageSourceId::ALL 一致"
        );
        // W-27：`caps()` 是**可覆写点**，生产采集器一律不许自造第二张可得性表
        for c in &cols {
            assert_eq!(
                c.caps(),
                caps_of(c.source_id()),
                "{} 覆写了 caps()（W-27：查询层与采集层必须共用 caps.rs 的同一张表）",
                c.source_id().db_id()
            );
        }
        // W-19 的「七源 ns 互不相同」已拆到同文件的
        // `all_seven_namespaces_are_distinct_and_prefixed`（**M5**：那段是派生值的自证，
        // 抓不到「给 SessionFileScan::new 传硬编码 ns」，措辞已降级）。
    }
}
