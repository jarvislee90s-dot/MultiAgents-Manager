//! 用量域值对象：冻结契约 §2 的 Rust 镜像（字段名与类型逐字一致）。
//! `Option` 的两种用法是刻意的：
//!   * 契约写 `field?: T`  → `Option<T>` + skip_serializing_if（缺省即不出现）
//!   * 契约写 `field: T | null` → `Option<T>` 且**必须**序列化出 null
//!     （compare / userEst / WorkSummary 全字段 / topTool / topToolMs / longestTurnPerTool 值）
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageRangePreset {
    Last5h,
    Today,
    Last7d,
    Last30d,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRange {
    pub preset: UsageRangePreset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageGroupBy {
    Tool,
    Project,
    Provider,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubagentMode {
    Include,
    ParentsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MiniBarRange {
    Today,
    Last5h,
    Last7d,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Measured,
    Inferred,
    Unknown,
}

impl SourceKind {
    /// 落库形态（usage_detail / usage_daily 的 provider_kind 列）
    pub fn as_db(&self) -> &'static str {
        match self {
            SourceKind::Measured => "measured",
            SourceKind::Inferred => "inferred",
            SourceKind::Unknown => "unknown",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "measured" => SourceKind::Measured,
            "inferred" => SourceKind::Inferred,
            _ => SourceKind::Unknown,
        }
    }
    /// 合并两个三态：取"更强"者（measured > inferred > unknown）——日聚合折叠用
    pub fn strongest(self, other: SourceKind) -> SourceKind {
        let rank = |k: SourceKind| match k {
            SourceKind::Measured => 2,
            SourceKind::Inferred => 1,
            SourceKind::Unknown => 0,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBuckets {
    pub input_fresh: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output: i64,
}

impl UsageBuckets {
    pub fn add(&mut self, other: &UsageBuckets) {
        self.input_fresh += other.input_fresh;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.output += other.output;
    }
    pub fn total(&self) -> i64 {
        self.input_fresh + self.cache_read + self.cache_write + self.output
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetrics {
    pub request_total: i64,
    /// 0–1 分数（UI 负责百分号格式化；参考实现这里有 0-1 与 0-100 混用的历史坑）
    pub cache_hit_rate: f64,
    /// 用户输入(估)：不可得源为 null（**不加 skip**，必须出 null）
    pub user_est: Option<i64>,
    pub requests: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub key: String,
    pub label: String,
    pub buckets: UsageBuckets,
    pub metrics: UsageMetrics,
    pub source_kind: SourceKind,
    pub is_subagent: bool,
}

/// 可得性条目可覆盖的指标（契约 §2 的**完整**枚举，共 12 值）。
/// 本计划的 `availability_table()` 只为「跨源口径不可比 / 需逐源说明」的那些出条目
/// （turn / errorModel / errorTurn / errorTool / interrupted / longestTurn / toolCalls / userEst）；
/// 其余值（sessions / toolAvgMs / topTool / topToolMs）留在枚举里供 ② 或后续按需登记——
/// 契约明文：**未列出的 WorkSummary 字段不可得时直接给 `null`，不强制配 availability 条目**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageMetric {
    Sessions,
    Turn,
    ErrorModel,
    ErrorTurn,
    ErrorTool,
    Interrupted,
    LongestTurn,
    ToolCalls,
    ToolAvgMs,
    TopTool,
    TopToolMs,
    UserEst,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageAvailability {
    pub metric: UsageMetric,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_source: Option<BTreeMap<String, bool>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    pub key: String,
    pub label: String,
    pub buckets: UsageBuckets,
    pub metrics: UsageMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareBlock {
    /// 上一等长周期的完整四桶（差值由前端算，后端不给 delta）
    pub prev_buckets: UsageBuckets,
    /// 上一等长周期的完整派生口径
    pub prev_metrics: UsageMetrics,
}

/// 浮窗第 4 行「本会话 X」：**最近有活动的会话**（不是当前打开的会话），
/// 用量按**当前 range** 统计（契约 §2）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSessionUsage {
    pub source_id: String,
    pub session_id: String,
    /// 会话标题（会话维度表；可为 null）
    pub title: Option<String>,
    pub buckets: UsageBuckets,
    pub metrics: UsageMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopTool {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopToolMs {
    pub name: String,
    pub ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongestTurn {
    pub p50: i64,
    pub max: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSummary {
    /// D17：父子分层后的会话数（**不含**子代理会话）
    pub sessions: Option<i64>,
    /// D16：只在按工具维度，**不合计**
    pub turns_per_tool: BTreeMap<String, Option<i64>>,
    /// D18：三层分列（互不包含，不得相加）
    pub error_model: Option<i64>,
    pub error_turn: Option<i64>,
    pub error_tool: Option<i64>,
    /// D18：用户主动打断单列，不计入错误
    pub interrupted: Option<i64>,
    pub tool_calls: Option<i64>,
    pub tool_avg_ms: Option<i64>,
    pub top_tool: Option<TopTool>,
    pub top_tool_ms: Option<TopToolMs>,
    /// D19：逐工具 p50 + 最长，**不跨工具合计**；值为 null = 该工具不可得
    pub longest_turn_per_tool: BTreeMap<String, Option<LongestTurn>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDashboard {
    pub range: UsageRange,
    pub group_by: UsageGroupBy,
    pub rows: Vec<UsageRow>,
    pub totals: UsageMetrics,
    pub totals_buckets: UsageBuckets,
    /// = totals.requestTotal + totalsBuckets.output（说明书 P1 第 2 条）
    pub hero: i64,
    pub trend: Vec<TrendPoint>,
    pub compare: Option<CompareBlock>,
    /// 最近有活动的会话（按当前 range 统计）；无会话 → null（契约 §2）
    pub recent_session: Option<RecentSessionUsage>,
    pub work_summary: WorkSummary,
    pub availability: Vec<UsageAvailability>,
    pub collected_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCard {
    pub tool_id: String,
    pub tool_label: String,
    pub buckets: UsageBuckets,
    pub metrics: UsageMetrics,
    /// 卡内每行 = 「供应商 / 模型」
    pub rows: Vec<UsageRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecords {
    pub range: UsageRange,
    pub group_by: UsageGroupBy,
    pub cards: Vec<UsageCard>,
    pub availability: Vec<UsageAvailability>,
    pub collected_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageFilters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub providers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    /// 默认 "include"（token 含子代理，D17）；"parentsOnly" 把子代理会话整体剔除
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_mode: Option<SubagentMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSettings {
    pub enabled: bool,
    pub mini_bar_range: MiniBarRange,
    pub mini_bar_tool_rows: i64,
    pub detail_retention_days: i64,
    pub collect_interval_min: i64,
    /// 用户自维护「模型名/前缀 → 供应商」规则（JSON 文本）
    pub provider_map_rules: String,
    pub export_quote: String,
    pub export_pose: String,
}

impl Default for UsageSettings {
    /// **说明书 §P7** 的默认值：开 / 今日 / **3 条**（浮窗第 3 行最多列几个工具）/ 90 天 / 10 分钟 / 空规则 / 空评语 / 随机姿态
    fn default() -> Self {
        Self {
            enabled: true,
            mini_bar_range: MiniBarRange::Today,
            mini_bar_tool_rows: 3,
            detail_retention_days: 90,
            collect_interval_min: 10,
            provider_map_rules: String::new(),
            export_quote: String::new(),
            export_pose: "random".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UsageSettingsPatch {
    pub enabled: Option<bool>,
    pub mini_bar_range: Option<MiniBarRange>,
    pub mini_bar_tool_rows: Option<i64>,
    pub detail_retention_days: Option<i64>,
    pub collect_interval_min: Option<i64>,
    pub provider_map_rules: Option<String>,
    pub export_quote: Option<String>,
    pub export_pose: Option<String>,
}

/// 采集源 id。**序列化形态必须是契约 §2 写的小写工具 id**
/// （`"claude" | "codex" | "kimi" | "opencode" | "workbuddy" | "zcode" | "dsh"`）——
/// 用 `lowercase` 而非 `camelCase`：后者会把 `WorkBuddy`/`ZCode` 变成 `workBuddy`/`zCode`，
/// 与契约逐字不符（且前端 `UsageSourceStatus.sourceId` 的类型联合会对不上）。
/// 同时与 `adapter::TOOL_IDS` / `AgentType::tool_id()` 同一套字符串（附 D.1：不新造 id）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSourceId {
    Claude,
    Codex,
    Kimi,
    OpenCode,
    WorkBuddy,
    ZCode,
    Dsh,
}

impl UsageSourceId {
    /// 落库 / 采集器注册 / 分组键统一用 `TOOL_IDS` 形态（adapter/mod.rs:384），
    /// 不新造 id 字符串（附 D.1）
    pub fn db_id(&self) -> &'static str {
        match self {
            UsageSourceId::Claude => "claude",
            UsageSourceId::Codex => "codex",
            UsageSourceId::Kimi => "kimi",
            UsageSourceId::OpenCode => "opencode",
            UsageSourceId::WorkBuddy => "workbuddy",
            UsageSourceId::ZCode => "zcode",
            UsageSourceId::Dsh => "dsh",
        }
    }
    pub const ALL: [UsageSourceId; 7] = [
        UsageSourceId::Claude,
        UsageSourceId::Codex,
        UsageSourceId::Kimi,
        UsageSourceId::OpenCode,
        UsageSourceId::WorkBuddy,
        UsageSourceId::ZCode,
        UsageSourceId::Dsh,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSourceStatus {
    pub source_id: UsageSourceId,
    pub ok: bool,
    pub parsed_files: i64,
    pub new_records: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageCollectResult {
    pub collected_at: i64,
    pub duration_ms: i64,
    pub sources: Vec<UsageSourceStatus>,
    pub total_new_records: i64,
}
