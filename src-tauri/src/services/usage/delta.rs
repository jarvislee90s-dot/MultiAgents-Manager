//! 采集产物类型：采集器唯一的输出形状（口径层的输入）。
//! 采集器**只上报原始字段与计数**，不算命中率、不做项目归一（那些在口径层）。
use std::collections::BTreeMap;

// `SourceKind` 必须显式导入：`DetailDelta.provider_kind` 与 `DailyDelta.provider_kind` 都用它，
// 只导 `UsageBuckets` 会 E0412（评审 M1——照抄即编译失败）。
use super::model::{SourceKind, UsageBuckets};
use super::semantics::CacheSemantics;

/// 明细行键：小时桶 + **记录级项目键** + 模型 + 供应商（day_key 由 hour_key 派生）。
/// `project_key` **必须进键**（R2/D21 规则④）：一个会话文件可跨多个项目，键里没有它就必然
/// 把同一会话同一小时的两个 cwd 折叠成一行；它同时也是明细表主键的一部分，内存键与库键一致。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DetailKey {
    pub session_id: String,
    /// "YYYY-MM-DDTHH"
    pub hour_key: String,
    /// "YYYY-MM-DD"（= hour_key 前 10 字符）
    pub day_key: String,
    /// **记录级**项目键（D21：`projectName.toLowerCase()`；无 cwd → `"unknown"`）。
    /// 由采集器按**该条记录的 cwd** 填（`project::project_key_of(cwd)`），
    /// **不得**在 `DeltaBuilder::detail()` 里回退查会话维度表（那是会话级近似，两档会分叉）。
    pub project_key: String,
    pub model: String,
    pub provider: String,
}

/// 计数类与时长样本（工作小结区，D15–D19）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionCounters {
    /// 该批新增的 turn 数（口径逐源见 research/七源字段可得性矩阵.md §3.1）
    pub turns: i64,
    /// D18 三层分列（互不包含，绝不合并）
    pub error_model: i64,
    pub error_turn: i64,
    pub error_tool: i64,
    /// D18 用户主动打断单列（不计入错误）
    pub interrupted: i64,
    pub tool_calls: i64,
    pub tool_ms: i64,
    /// 单 turn 时长样本（毫秒；p50 与最长都由它算）
    pub turn_ms: Vec<i64>,
    /// 工具名 → (调用次数, 累计毫秒)
    pub tool_stats: BTreeMap<String, (i64, i64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetailDelta {
    /// 采集源 id（`UsageSourceId::db_id()` 形态）：DAO 的落库列，采集器填自己那一个
    pub source_id: String,
    /// 供应商归因三态（D8）：由采集器经 `DeltaBuilder::provider_of` 记录，落库后 UI 可见区分
    pub provider_kind: SourceKind,
    /// 行身份（含**记录级** `project_key`，见 `DetailKey`）——项目归属随它落库并可分组
    pub key: DetailKey,
    /// 规范四桶（已按该条语义归一化）
    pub buckets: UsageBuckets,
    /// 该条语义下的「请求输入」（Exclusive 含缓存写、Subset 等于输入原文）
    pub request_total: i64,
    pub requests: i64,
    /// 用户输入(估)：**`None` = 该源读不到用户文本 = 不可得**（落库写 NULL，绝不写 0）。
    /// 有值的源（claude/codex/kimi）与无值的源在**小时档与日档**都按此语义读取（R1）
    pub user_est: Option<i64>,
    pub cache_semantics: CacheSemantics,
    pub counters: SessionCounters,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DailyDelta {
    pub day_key: String,
    pub source_id: String,
    /// **记录级**项目键（= 折叠时用的 `DetailDelta.key.project_key`，D21 规则④）
    pub project_key: String,
    pub provider: String,
    /// 三态取本组内"最强"者（measured > inferred > unknown）
    pub provider_kind: SourceKind,
    pub model: String,
    pub buckets: UsageBuckets,
    pub request_total: i64,
    pub requests: i64,
    /// 用户输入(估)：**全组都不可得才是 `None`**，任一行有值就累加（R1；日档必须出真值）
    pub user_est: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDimDelta {
    pub session_id: String,
    /// D21 分组键 = `projectName.toLowerCase()`
    pub project_key: String,
    /// D21 显示名 = `projectName` 原文（`project_name_from_path` 的 basename 结果）
    pub project_label: String,
    /// 记录内 cwd 原文（各源形态不同：claude 小写正斜杠 / zcode 原生反斜杠 / workbuddy 可能空串）
    pub project_path_raw: String,
    /// realpath 规范化路径（**不作 UI 用途**，只为日后修归类时无需重扫）
    pub project_realpath: String,
    pub title: Option<String>,
    pub is_subagent: bool,
    pub parent_session_id: Option<String>,
    pub originator: Option<String>,
    pub last_seen_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CursorDelta {
    pub session_id: String,
    /// 已消费前缀末尾 ≤64 字节的 sha256（截断/重写检测）
    pub fingerprint: String,
    /// 已消费字节水位（只在完整行边界推进）
    pub byte_offset: i64,
    /// 已消费完整行数（字节双水位的第二维）
    pub ordinal: i64,
    /// 累计型源的上次累计值（取差值算增量）
    pub last_cumulative: Option<i64>,
    pub mtime_ms: i64,
    pub file_size: i64,
    /// 采集器续读状态（serde JSON 小对象；文件型源用它跨轮保住模型继承 / 未配对工具调用 /
    /// 相邻去重上一条——增量读从文件中段开始时没有它就会算错）
    pub state_json: String,
}

/// 一个源一轮采集的全部产物
#[derive(Debug, Clone, Default)]
pub struct SourceDelta {
    pub details: Vec<DetailDelta>,
    pub sessions: Vec<SessionDimDelta>,
    pub cursors: Vec<CursorDelta>,
    pub parsed_files: i64,
    pub new_records: i64,
}
