//! 7 个采集器的登记处与共用工具。
//! 每个采集器都遵守同一条纪律：**每个文件只读一遍**（经 CollectContext::read_incremental），
//! 在同一遍里把四桶/turn/工具/报错/时长一起算出来，只上报原始字段（口径计算在 semantics 等模块）。
pub mod claude;
pub mod codex;
pub mod dsh;
pub mod kimi;
pub mod opencode;
pub mod workbuddy;
pub mod zcode;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::collect::UsageCollector;
use super::delta::{CursorDelta, DetailDelta, DetailKey, SessionDimDelta, SourceDelta};
use super::model::{SourceKind, UsageBuckets, UsageSourceId};
use super::provider::ProviderRule;

/// 采集器登记表（登记顺序 = 采集顺序 = 契约 `UsageSourceStatus` 顺序）
///
/// **纪律（Global Constraints 20 / §3.2.2 阻塞 4）**：Task 12–17 每个采集器任务写完自己的
/// `collect()` 之后，**必须回来把 `Box::new(XxxCollector)` 加进本表**（每个任务都给了改后的
/// 完整函数体）。漏登记**不是编译错，而是静默不采**——`run_collection` 只遍历本表；
/// 每个采集器文件里的 `collector_is_registered_in_all()` 就地锁住自己那一行，
/// Task 17 另有一条 `all_seven_collectors_are_registered_in_order()` 锁全表。
pub fn all() -> Vec<Box<dyn UsageCollector>> {
    vec![
        Box::new(claude::ClaudeCollector),
        Box::new(codex::CodexCollector),
        Box::new(kimi::KimiCollector),
        Box::new(opencode::OpenCodeCollector),
        Box::new(workbuddy::WorkBuddyCollector),
        Box::new(zcode::ZCodeCollector),
        Box::new(dsh::DshCollector),
    ]
}

/// 文件标识（游标键）：**路径派生、在源内唯一**（评审 B5）。
///
/// 为什么不能用文件名 stem：kimi 的会话文件**全部叫 `wire.jsonl`**
/// （`<session>/agents/<main|agent-N>/wire.jsonl`，本机 82 个）→ stem 恒为 `"wire"`，
/// 而 `usage_cursor` 主键是 `(source_id, session_id=游标键)` → 82 个文件只留 1 行
/// （后写覆盖先写），其余 81 个文件每轮尾指纹不符 → `rescan = true` → **整文件重读重算
/// 并累加**，账目按采集轮次线性膨胀。workbuddy 的文件名是 `<session_id>.jsonl`（唯一）、
/// claude / codex 的 rollout 名含 UUID（唯一）——**唯一性不是文件名给的，是路径给的**。
///
/// 实现 = **相对本源根目录的路径**，分隔符归一为 `:`（Windows 反斜杠同样归一）；
/// 相对根而不是绝对路径，是为了用户搬 home 后游标仍然有效。
/// 调用方传各自源的根：claude = `~/.claude/projects`、codex = 两棵树的根、
/// kimi = **sessions 根**（不是单个会话目录，否则不同会话的同名 agent 目录仍会撞）、
/// workbuddy = `~/.workbuddy`。
pub fn file_key_of(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let key = rel
        .to_string_lossy()
        .replace(['/', '\\'], ":")
        .trim_start_matches(':')
        .to_string();
    if key.is_empty() {
        // 兜底（理论上不可达）：至少保证非空且带文件名
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string())
    } else {
        key
    }
}

// **W-13 结构锁（部分覆盖，诚实标注见下）**：`record_project` 的**实际调用次数**计数器。
// 线程局部（`cargo test` 每个用例各占一个线程 → 用例之间零串味），且只在 `#[cfg(test)]`
// 下存在——生产构建**零开销、零行为差异**。
//
// 它锁住的是：**调用点被搬回「逐记录」**（把 `session()` 里那次 `record_project_for(cwd)`
// 挪到函数首句）→ 计数值随记录数增长 →
// `record_project_is_called_once_per_session_not_per_record` 立刻红。
// W-13 的失败模式是**纯性能**的（codex 真机 226,035 行 / 2.21 GB 上叠数十万次
// `canonicalize`），输出值一模一样、没有第二条外部可观测通路（计划原文给 W-13 的锁本就是
// 「评审逐源核对调用点」，见 KNOWN-PLAN-DEFECTS W-13）→ 本计数器是**部分覆盖**：
// 它抓不到「绕过 `record_project_for` 直接调 `project::record_project`」的改法，那一种只能靠读码。
// （写成 `///` 文档注释会触发 `unused_doc_comments`——门禁 `-D warnings` 下即 error，故用行注释。）
#[cfg(test)]
thread_local! {
    pub(crate) static RECORD_PROJECT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 单文件读取失败是否**可跳过**（W-06 / D-24；**采集器共用，只有这一份实现**）。
///
/// 只有「文件已消失」这一种——它是**枚举与读取之间的轮转竞态**（`SCAN.collect` 拿到清单之后、
/// `read_incremental` 打开之前文件被轮转/删除），该文件下一轮已不在清单里，跳过不会永久漏数。
///
/// **不能只看 `ErrorKind::NotFound`**（Task 11 的 M-4）：`session_scan::collect_inner` 用
/// `DirEntry::metadata()`（**不跟随**符号链接）列文件，而 `read_incremental` 的 `stat_of` 用
/// `fs::metadata`（**跟随**）→ 一个**悬空符号链接** `x.jsonl` 会永远躺在清单里、每轮 `NotFound`、
/// 每轮被「跳过」（**持久泄漏**：这个源永远读不完整，且没有任何信号）。
/// 所以 `NotFound` 时**再核一次链接本身**：连链接都没了才算「已消失」，否则整源 `Err`。
///
/// 其余错误（mtime 不可得 → `InvalidData`、权限、IO 故障）都是「**这个源读不了**」的信号，
/// 必须整源 `Err` 落进 `UsageSourceStatus.errorCode`，**不得**只 `log::warn!`——否则用户在
/// UI 上看到的是「这个源没有用量」，而不是「这个源读不了」（W-06 补充条款）。
///
/// **为什么提到本模块**（Task 12 评审 Important #3，裁决 B）：这条判据决定「静默跳过 vs 整源
/// 响亮失败」，claude / codex **各存一份逐字拷贝**的漂移（例如某侧将来补 EACCES、另一侧没补）
/// 会把「读不了」重新降级成「这个源没有用量」，而且**不会有任何测试报红**——正是 GC 6
/// 「口径层唯一」要防的。故收进采集器共用层，两个采集器共用同一份。
pub(crate) fn file_read_failure_is_benign(path: &Path, e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
        && matches!(
            std::fs::symlink_metadata(path),
            Err(ref pe) if pe.kind() == std::io::ErrorKind::NotFound
        )
}

/// 会话级项目归属的**唯一调用点**（W-13）：`record_project` 内含一次 `canonicalize`
/// （必然读文件系统，是本域唯一的非纯函数）。包一层的目的只有一个——给上面那条
/// 「调用点是否还在会话级」的结构锁一个可计数的落点。
///
/// **计数口径（M-3，fix round 1/5）**：只统计 `cwd` **非空白**的调用——`record_project`
/// 自身对空白/缺失 cwd 直接短路（不 canonicalize），故它不构成 W-13 要压住的那笔 syscall。
/// 因此本计数器**证明不了**「空白 cwd 的记录不会读到文件系统」（那是 `project.rs` 的内部分支），
/// 也**不写**这类断言——只有「同一会话的后续记录不再重复调用」这一条是可伪证的。
fn record_project_for(cwd: Option<&str>) -> super::project::RecordProject {
    #[cfg(test)]
    if matches!(cwd.map(str::trim), Some(s) if !s.is_empty()) {
        RECORD_PROJECT_CALLS.with(|c| c.set(c.get() + 1));
    }
    super::project::record_project(cwd)
}

/// 单遍扫描的累计器：把同一 (会话, 小时, 模型, 供应商) 的增量与计数归并到一条明细
pub struct DeltaBuilder {
    source: UsageSourceId,
    pub rules: Vec<ProviderRule>,
    details: HashMap<DetailKey, DetailDelta>,
    sessions: HashMap<String, SessionDimDelta>,
    cursors: Vec<CursorDelta>,
    /// 供应商 → 三态（D8）：采集器经 `provider_of` 记录，`detail()` 建行时自动填
    kind_by_provider: HashMap<String, SourceKind>,
    pub parsed_files: i64,
    pub new_records: i64,
}

impl DeltaBuilder {
    pub fn new(source: UsageSourceId, rules: Vec<ProviderRule>) -> Self {
        Self {
            source,
            rules,
            details: HashMap::new(),
            sessions: HashMap::new(),
            cursors: Vec::new(),
            kind_by_provider: HashMap::new(),
            parsed_files: 0,
            new_records: 0,
        }
    }

    /// **采集器解析供应商的唯一入口**：内部就是
    /// `provider::resolve_provider(self.source, explicit, model, &self.rules)`，
    /// 外加记录三态（供 `detail()` 落 `provider_kind`）。采集器里出现的
    /// `resolve_provider(UsageSourceId::X, explicit, model, &b.rules)` 一律改用本方法。
    pub fn provider_of(&mut self, explicit: Option<&str>, model: &str) -> (String, SourceKind) {
        let (provider, kind) =
            super::provider::resolve_provider(self.source, explicit, model, &self.rules);
        self.kind_by_provider.insert(provider.clone(), kind);
        (provider, kind)
    }

    /// 取（或建）一条明细，供调用方直接累加（**同一遍扫描内**同时写四桶与计数）。
    /// **项目归属一律取自 `key.project_key`（记录级）**——由采集器按该条记录的 cwd 填好；
    /// 这里**严禁**回查 `self.sessions`（会话级近似）：那会让同一会话的两个 cwd 折叠成一行、
    /// 且与日聚合口径分叉（R2/D21 规则④）。
    pub fn detail(&mut self, key: DetailKey, ts_ms: i64) -> &mut DetailDelta {
        let source_id = self.source.db_id().to_string();
        let provider_kind = self
            .kind_by_provider
            .get(&key.provider)
            .copied()
            .unwrap_or(SourceKind::Unknown);
        self.details
            .entry(key.clone())
            .or_insert_with(|| DetailDelta {
                source_id,
                provider_kind,
                key,
                buckets: UsageBuckets::default(),
                request_total: 0,
                requests: 0,
                user_est: None,
                cache_semantics: super::semantics::CacheSemantics::Exclusive,
                counters: Default::default(),
                ts_ms,
            })
    }

    /// 登记/更新会话维度（D21 四列 + D17 分层血缘）。
    /// **项目列是会话级近似**：一个会话跨多项目时只保留**首个带 cwd 的记录**的项目
    /// （记录级归属在 `DetailKey.project_key` 上，不靠本表）；无 cwd 的记录不得覆盖它。
    /// 其余字段按「有则补、无则留」合并，`last_seen_at` 取 max（乱序批次不得改小）。
    ///
    /// **`is_subagent` 的口径（裁决 B，fix round 1/5；`plan-mandated` 的 `|=` 已作废）**：
    /// 该位的含义是「**本会话自身就是子代理会话**」（D17 的计数分层建立在这个含义上），
    /// **不是**「本会话里出现过侧链记录」。因此本参数是**记录级**输入，合并规则必须是
    /// **AND**：只有「该会话的每一条记录都是侧链」才置真。
    /// 为什么 `|=` 是错的：claude 的侧链记录与父会话**同文件同 sessionId**，用 OR 会让
    /// 「用过子代理的父会话」整条变成子代理会话 → 唯一消费者 Task 18 按该位**硬过滤** →
    /// 父会话**连会话数带 turn 数一起消失**（不是「只扣掉子代理那部分」）。
    /// 跨轮保持（增量读只看得到文件中段）由采集器的 `state_json` 负责，见
    /// `claude::ClaudeFileState::note_record_sidechain`。
    ///
    /// **W-13（本域唯一的非纯函数调用纪律）**：`record_project` 内含一次 `canonicalize`
    /// （必然读文件系统）→ **每个会话最多调一次**。任务书原文把它放在本函数**首句**
    /// （= 逐记录调用），与 W-13「逐记录一律用零 FS 的 `project_key_of`」直接冲突；
    /// 这里按 W-13 落到**唯一真正需要它的分支**：① 新建会话行；② 既有会话行补上首个带 cwd
    /// 的项目。其余记录（同一会话的后续记录）走**零 FS** 路径。
    /// **输出与任务书等价**（`record_project` 的返回值只在这两处被采纳；空白 cwd 的记录：
    /// 任务书会调一次但结果同样被丢弃，本实现直接不调——两者产出逐字相同，差别只在调用次数）。
    /// 见报告「偏差申报」。
    /// 会话维度登记。**证据单调性由采集器负责，DAO 的 `MAX` 只做物理合并**（A-4 / Important-2
    /// 的归属文档）。
    ///
    /// 把这条分界写明，免得两处互相指望：
    /// * **采集器**负责「本轮的证据是什么」——同一会话的多条记录、多个判据源（如 dsh 的
    ///   投影缓存 vs 日志头）必须在**进到这里之前**收敛成**单一决定性取值**
    ///   （见 dsh 的 `JudgeTally::observe`：投影缓存权威、日志兜底，不一致时按优先级取值
    ///   并带计数告警）。本条的 `is_subagent` 用 **AND** 合并
    ///   （`cur.is_subagent &= is_subagent`，裁决 B）：「本会话每一条记录都是侧链」才置真
    ///   —— 用 `|=` 会让「用过子代理的父会话」整条变成子代理会话。
    /// * **DAO** 的 `usage_session` upsert 对 `is_subagent` 取 `MAX`（旧值 OR 新值）：那是
    ///   **物理合并**（幂等写入 / 多批 upsert 不互相抹掉），**不是**语义上的证据单调性
    ///   —— 它**下不来**：某轮落了 true，后续轮次即使证据变成 false 也不会回落。
    ///   故「真→假」的振荡必须在**采集器**侧就被判成单一取值（本仓 claude / dsh 两处已如此），
    ///   不能指望 DAO 兜。
    #[allow(clippy::too_many_arguments)]
    pub fn session(
        &mut self,
        session_id: &str,
        cwd: Option<&str>,
        title: Option<String>,
        is_subagent: bool,
        parent: Option<String>,
        originator: Option<String>,
        last_seen_at: i64,
    ) {
        match self.sessions.get_mut(session_id) {
            Some(cur) => {
                // 只接受「首个带 cwd 的记录」（以 project_path_raw 是否为空为判据）
                if cur.project_path_raw.is_empty() {
                    if let Some(raw) = cwd.map(str::trim).filter(|s| !s.is_empty()) {
                        let p = record_project_for(Some(raw));
                        cur.project_key = p.key;
                        cur.project_label = p.label;
                        cur.project_path_raw = p.path_raw;
                        cur.project_realpath = p.realpath;
                    }
                }
                if cur.title.is_none() {
                    cur.title = title;
                }
                // **裁决 B**：AND——「本会话每一条记录都是侧链」才置真。用 `|=` 会让
                // 「用过子代理的父会话」整条变成子代理会话 → Task 18 硬过滤 → 父会话消失。
                cur.is_subagent &= is_subagent;
                if cur.parent_session_id.is_none() {
                    cur.parent_session_id = parent;
                }
                if cur.originator.is_none() {
                    cur.originator = originator;
                }
                cur.last_seen_at = cur.last_seen_at.max(last_seen_at);
            }
            None => {
                let p = record_project_for(cwd);
                self.sessions.insert(
                    session_id.to_string(),
                    SessionDimDelta {
                        session_id: session_id.to_string(),
                        project_key: p.key,
                        project_label: p.label,
                        project_path_raw: p.path_raw,
                        project_realpath: p.realpath,
                        title,
                        is_subagent,
                        parent_session_id: parent,
                        originator,
                        last_seen_at,
                    },
                );
            }
        }
    }

    pub fn push_cursor(&mut self, c: CursorDelta) {
        self.cursors.push(c);
    }

    pub fn finish(self) -> SourceDelta {
        SourceDelta {
            details: self.details.into_values().collect(),
            sessions: self.sessions.into_values().collect(),
            cursors: self.cursors,
            parsed_files: self.parsed_files,
            new_records: self.new_records,
        }
    }
}

/// 工具耗时配对状态（claude `sourceToolAssistantUUID`→`uuid`、codex `call_id`→
/// `function_call_output`、workbuddy 时间戳配对共用）。**必须进游标 state_json**：
/// 增量读从文件中段开始时，上一轮的未配对调用还在等待回执。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PendingTools {
    pub open: BTreeMap<String, (i64, String)>,
    /// 因容量上限被丢弃的未配对调用数（**不静默丢**：计数随 `state_json` 落库、并 `log::warn!`）
    #[serde(default)]
    pub dropped: i64,
}

/// 未配对工具调用的容量上限（**M-6，fix round 1/5**）。
/// `open` 会随 `state_json` 落库并**随会话寿命单调增长**：被用户打断 / 无回执的调用永远等不到
/// `finish`（真机样本恰好是 `"open":{}` 故本机不发作——那是运气，不是结构）。
/// 正常跨轮配对的窗口只有一轮（回执紧随其后），64 远超所需。
pub const PENDING_TOOLS_CAP: usize = 64;

impl PendingTools {
    /// 超上限 → 按 `ts_ms` **最早**的丢弃（直到回到上限内），每丢一条 `dropped += 1`。
    /// 只丢最早项：跨轮配对的正常需求（回执通常在本轮或下一轮）不受影响。
    fn evict_overflow(&mut self) {
        while self.open.len() > PENDING_TOOLS_CAP {
            let Some(oldest) = self
                .open
                .iter()
                .min_by_key(|(_, (ts, _))| *ts)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.open.remove(&oldest);
            self.dropped += 1;
            log::warn!(
                "usage: 未配对工具调用超上限 {}，丢弃最早项 {oldest}（累计丢弃 {}）",
                PENDING_TOOLS_CAP,
                self.dropped
            );
        }
    }

    pub fn start(&mut self, call_id: &str, ts_ms: i64, name: &str) {
        self.open
            .insert(call_id.to_string(), (ts_ms, name.to_string()));
        self.evict_overflow();
    }
    /// **只在没有同键未配对项时**登记（claude：一条 assistant 记录理论上可带多个 `tool_use`，
    /// 而配对键是**记录级 `uuid`**——此时只留第一个参与耗时配对，计数仍各自累加；
    /// 直接 `start` 会让后一个覆盖前一个，耗时归属随记录内顺序漂移）。
    pub fn start_if_absent(&mut self, call_id: &str, ts_ms: i64, name: &str) {
        self.open
            .entry(call_id.to_string())
            .or_insert((ts_ms, name.to_string()));
        self.evict_overflow();
    }
    /// 回执到达：返回 (耗时毫秒, 工具名)；无配对（跨轮丢失 / 无回执的 2/71）→ None
    pub fn finish(&mut self, call_id: &str, ts_ms: i64) -> Option<(i64, String)> {
        let (start, name) = self.open.remove(call_id)?;
        Some(((ts_ms - start).max(0), name))
    }

    /// **无配对键**型回执的兜底（workbuddy：`function_call_result` 不带任何键时）：
    /// 取**最早**（按调用时间戳）一个未配对调用收尾（时间戳配对，真机 69/71）。
    ///
    /// **为什么不是 `self.open.keys().next()`**：`open` 是 `BTreeMap<String, _>`，
    /// 键序是 **call id 的字典序**——直接取首键会配到一个可能**更晚**的调用上，
    /// 耗时算错（甚至被 `.max(0)` 夹成 0）。判据与 `evict_overflow` 的「丢最早」同一条时间序。
    pub fn finish_oldest(&mut self, ts_ms: i64) -> Option<(i64, String)> {
        let id = self
            .open
            .iter()
            .min_by_key(|(_, (ts, _))| *ts)
            .map(|(k, _)| k.clone())?;
        self.finish(&id, ts_ms)
    }
}

// ==== usage 自省锁：以下为排除区（测试代码），勿删勿复制 ====
#[cfg(test)]
mod tests {
    use super::*;

    /// **B5 回归（键的形状）**：游标键 = **相对本源根目录的路径**，分隔符归一为 `:`。
    /// 用文件名 stem（或绝对路径 / 反斜杠不归一）都过不了这几条。
    #[test]
    fn file_key_is_the_path_relative_to_the_source_root() {
        let root = Path::new("/home/u/.claude/projects");
        assert_eq!(
            file_key_of(root, &root.join("-p-A").join("sess-1.jsonl")),
            "-p-A:sess-1.jsonl",
            "键必须相对本源根目录（绝对路径会让用户搬 home 后游标失效）"
        );
        // 分隔符归一：Windows 反斜杠同样归 `:`（同一份代码跨平台产出同一把键）
        assert_eq!(
            file_key_of(
                root,
                Path::new("/home/u/.claude/projects/win\\sub\\wire.jsonl")
            ),
            "win:sub:wire.jsonl"
        );
        // Windows 形态的盘符根（用正斜杠写，两个平台的分量解析一致）
        assert_eq!(
            file_key_of(Path::new("C:/root"), Path::new("C:/root/a/b/wire.jsonl")),
            "a:b:wire.jsonl"
        );
    }

    /// **B5 的命门**：同一个文件名在不同目录下必须得到**不同**的键。
    /// kimi 的会话文件**全部叫 `wire.jsonl`**（本机 82 个）——用文件名 stem 的实现会让
    /// 它们撞成一行游标（后写覆盖先写），其余文件每轮 `rescan = true` → 整文件重读重算
    /// 并累加，账目按轮次线性膨胀。
    #[test]
    fn same_file_name_in_different_directories_never_collides() {
        let root = Path::new("/home/u/.kimi-code/sessions");
        let a = file_key_of(root, &root.join("sess-A/agents/main/wire.jsonl"));
        let b = file_key_of(root, &root.join("sess-B/agents/main/wire.jsonl"));
        assert_eq!(a, "sess-A:agents:main:wire.jsonl");
        assert_eq!(b, "sess-B:agents:main:wire.jsonl");
        assert_ne!(a, b, "同名的 wire.jsonl 必须各自成键（stem 键会撞成一行）");
        assert_ne!(a, "wire", "键不得退化成文件名 stem");
        // kimi 的另一半：同一会话的 main / agent-N 目录同样不能撞
        assert_ne!(
            file_key_of(root, &root.join("sess-A/agents/main/wire.jsonl")),
            file_key_of(root, &root.join("sess-A/agents/agent-1/wire.jsonl"))
        );
    }

    /// 兜底分支：`strip_prefix` 失败（path 不在 root 下）时不得产出空串——
    /// 空键会让该游标行与「根本没有键」不可区分；root == path 时退到文件名。
    #[test]
    fn key_is_never_empty_even_outside_the_source_root() {
        let root = Path::new("/home/u/.claude/projects");
        let outside = file_key_of(root, Path::new("/elsewhere/sess-1.jsonl"));
        assert!(!outside.is_empty(), "键不得为空：{outside:?}");
        assert!(
            outside.contains("sess-1.jsonl"),
            "兜底至少带文件名：{outside:?}"
        );
        assert_eq!(file_key_of(Path::new("/x"), Path::new("/x")), "x");
    }

    /// **W-13 结构锁**：`record_project`（内含 `canonicalize`，本域唯一的非纯函数）
    /// **每个会话只许调一次**——逐记录一律走零 FS 的 `project_key_of`。
    /// 真机量级：codex 226,035 行 / 2.21 GB，挂到每条记录上就是数十万次 syscall。
    /// 把 `session()` 里那次 `record_project_for(cwd)` 搬回函数首句（= 逐记录）→ 本用例立刻红。
    ///
    /// **部分覆盖（诚实标注）**：抓不到「绕过 `record_project_for` 直接调
    /// `project::record_project`」的改法——那种写法与正确实现在**任何输出上都等价**，
    /// 计划原文给的锁也只是「评审逐源核对调用点」（W-13）。
    #[test]
    fn record_project_is_called_once_per_session_not_per_record() {
        RECORD_PROJECT_CALLS.with(|c| c.set(0));
        let mut b = DeltaBuilder::new(UsageSourceId::Claude, Vec::new());
        // 同一会话的 100 条记录：只有第一条真正需要 canonicalize（会话级一次）
        for i in 0..100 {
            b.session("s1", Some("/p/A"), None, false, None, None, i);
        }
        assert_eq!(
            RECORD_PROJECT_CALLS.with(|c| c.get()),
            1,
            "同一会话的后续记录不得再读文件系统（W-13；逐记录调用 = 100 次 canonicalize）"
        );
        // 前提断言（防假绿）：计数器真的在计数——换个会话必须各算一次
        b.session("s2", Some("/p/B"), None, false, None, None, 0);
        assert_eq!(RECORD_PROJECT_CALLS.with(|c| c.get()), 2);
        // **M-3（fix round 1/5）**：原来这里还有两条「无 cwd → 计数器不涨」的断言——
        // 它们是**恒真**的（计数器自带 blank-cwd 守卫，`record_project_for` 内），
        // 任何实现都过得了，属不可伪证的假锁，已删。无 cwd 的可伪证面在**输出**上：
        b.session("s3", None, None, false, None, None, 0);
        let s3 = b.sessions.get("s3").unwrap();
        assert_eq!(
            s3.project_key, "unknown",
            "W-01：无 cwd 的键是 `unknown`，不是空串"
        );
        assert_eq!(s3.project_path_raw, "", "无 cwd 的 cwd 原文就是空");
        assert_eq!(
            s3.project_realpath, "",
            "无 cwd 不得产生 FS 派生的 realpath（record_project 对空 cwd 直接短路）"
        );
        RECORD_PROJECT_CALLS.with(|c| c.set(0));
    }

    /// **M-6 回归（无上界族）**：`PendingTools.open` 会随 `state_json` 落库、并随会话寿命
    /// **单调增长**（被用户打断 / 无回执的调用永远等不到 `finish`）。真机样本恰好是
    /// `"open":{}` 故本机不发作——那是运气，不是结构。本用例钉住：有上界 + **丢弃有计数** +
    /// 按**最早**丢（不能把最新的配对需求丢掉）。
    #[test]
    fn pending_tools_are_bounded_and_drops_are_counted() {
        let mut t = PendingTools::default();
        for i in 0..(PENDING_TOOLS_CAP + 5) {
            t.start_if_absent(&format!("c{i}"), i as i64, "Bash");
        }
        assert_eq!(
            t.open.len(),
            PENDING_TOOLS_CAP,
            "未配对调用必须有上界（M-6：否则 state_json 随会话寿命单调增长）"
        );
        assert_eq!(t.dropped, 5, "丢弃必须**计数**，不得静默");
        assert!(!t.open.contains_key("c0"), "丢的必须是最早的（ts 最小）");
        assert!(t.open.contains_key(&format!("c{}", PENDING_TOOLS_CAP + 4)));
        // 前提断言（防假绿）：上限之内的配对照常工作（容量逻辑没有把正常路径也吃掉）
        let mut t2 = PendingTools::default();
        t2.start_if_absent("a", 1_000, "Bash");
        assert_eq!(t2.finish("a", 3_500), Some((2_500, "Bash".into())));
        assert_eq!(t2.dropped, 0, "未超限不得有任何丢弃");
        // `start` 与 `start_if_absent` 走同一套上限（两条入口都不能漏）
        let mut t3 = PendingTools::default();
        for i in 0..(PENDING_TOOLS_CAP + 1) {
            t3.start(&format!("s{i}"), i as i64, "Read");
        }
        assert_eq!(t3.open.len(), PENDING_TOOLS_CAP);
        assert_eq!(t3.dropped, 1);
    }

    /// **`finish_oldest` 的时间序**（Task 15 新增；workbuddy 的无键型回执兜底）：
    /// 必须取**最早**（按调用时间戳）一个未配对调用——`open` 是 `BTreeMap<String, _>`，
    /// 键序是 **call id 的字典序**，`keys().next()` 会配到一个可能更晚的调用上
    /// （耗时算错，甚至被 `.max(0)` 夹成 0）。
    #[test]
    fn finish_oldest_pairs_the_earliest_call_not_the_smallest_id() {
        let mut t = PendingTools::default();
        t.start("z_early", 1_000, "Bash");
        t.start("a_late", 5_000, "Read");
        // 前提断言（防假绿）：字典序最小的**不是**最早的那个（否则本用例不可伪证）
        assert_eq!(t.open.keys().next().map(String::as_str), Some("a_late"));
        assert_eq!(
            t.finish_oldest(6_000),
            Some((5_000, "Bash".into())),
            "最早 = ts 最小（不是 id 字典序最小）"
        );
        assert_eq!(
            t.finish_oldest(6_000),
            Some((1_000, "Read".into())),
            "剩下的那一个"
        );
        assert_eq!(t.finish_oldest(6_000), None, "空表 → None（不得 panic）");
    }
}
// ==== usage 自省锁：排除区结束，勿删勿复制 ====
