// 用量域值对象（契约 §2 的 TS 镜像）。注意可选性：契约写 `?:` 的字段在 Rust 侧
// skip_serializing_if → 前端可能**缺键**；契约写 `| null` 的字段一定出现且可为 null。
export type UsageRangePreset = "last5h" | "today" | "last7d" | "last30d" | "custom";
export interface UsageRange {
  preset: UsageRangePreset;
  from?: string;
  to?: string;
}
export type UsageGroupBy = "tool" | "project" | "provider" | "model";
/**
 * 记录页（`usage_records`）的 groupBy **只接受 `tool` | `project`**（W6/D6/D7）：
 * 它决定**卡片**维度，卡内行恒为「供应商 / 模型」。传 provider/model → `usage-groupby-invalid`。
 * 大看板（`usage_dashboard`）与 CSV（`usage_export_csv`）仍用完整的 `UsageGroupBy` 四值。
 */
export type UsageRecordsGroupBy = Extract<UsageGroupBy, "tool" | "project">;
export interface UsageBuckets {
  inputFresh: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
}
export interface UsageMetrics {
  requestTotal: number;
  /** 0–1 分数（UI 负责百分号格式化） */
  cacheHitRate: number;
  /** 用户输入(估)；不可得源为 null */
  userEst: number | null;
  requests: number;
}
export type UsageSourceKind = "measured" | "inferred" | "unknown";
export interface UsageRow {
  key: string;
  /**
   * 展示名。**供应商维度且供应商不可得时，这里是 i18n 键** `usage.label.unknownProvider`
   * （W5：Rust 侧不硬编码中文）→ ② 用 `t(label)` 渲染；其余维度是原文。
   */
  label: string;
  buckets: UsageBuckets;
  metrics: UsageMetrics;
  sourceKind: UsageSourceKind;
  /**
   * D17 计数分层标记：`true` = 该组**只由子代理会话贡献**、`false` = 含父会话
   * （以上两者只在**小时档**成立）；`null` = **本档位算不出**（日档与记录页卡内行——
   * 两者的行维度都不含 `session_id`，见契约 §2 2026-10-03 裁决）。
   * 遇 `null` **不显示**子代理标记，**不得**回退成 `false` 的「非子代理」语义。
   */
  isSubagent: boolean | null;
}
export type UsageMetric =
  | "sessions"
  | "turn"
  | "errorModel"
  | "errorTurn"
  | "errorTool"
  | "interrupted"
  | "longestTurn"
  | "toolCalls"
  | "toolAvgMs"
  | "topTool"
  | "topToolMs"
  | "userEst";
/**
 * 别名：计划② 的消费点按 `UsageAvailabilityMetric` 这个名字 import。
 * **同一联合、只此一处定义**（避免两侧各造一个同名不同形的类型）——
 * ② 不得在 `src/lib/api/usage.ts` 或别处再声明同名联合（形状以本文件为准）。
 */
export type UsageAvailabilityMetric = UsageMetric;
export interface UsageAvailability {
  metric: UsageMetric;
  available: boolean;
  reason?: string;
  perSource?: Record<string, boolean>;
}
export interface TrendPoint {
  key: string;
  label: string;
  buckets: UsageBuckets;
  metrics: UsageMetrics;
}
export interface CompareBlock {
  prevBuckets: UsageBuckets;
  prevMetrics: UsageMetrics;
}
export interface RecentSessionUsage {
  sourceId: string;
  sessionId: string;
  title: string | null;
  buckets: UsageBuckets;
  metrics: UsageMetrics;
}
export interface LongestTurn {
  p50: number;
  max: number;
}
export interface WorkSummary {
  sessions: number | null;
  turnsPerTool: Record<string, number | null>;
  errorModel: number | null;
  errorTurn: number | null;
  errorTool: number | null;
  interrupted: number | null;
  toolCalls: number | null;
  toolAvgMs: number | null;
  topTool: { name: string; count: number } | null;
  topToolMs: { name: string; ms: number } | null;
  longestTurnPerTool: Record<string, LongestTurn | null>;
}
export interface UsageDashboard {
  range: UsageRange;
  groupBy: UsageGroupBy;
  rows: UsageRow[];
  totals: UsageMetrics;
  totalsBuckets: UsageBuckets;
  hero: number;
  trend: TrendPoint[];
  compare: CompareBlock | null;
  recentSession: RecentSessionUsage | null;
  workSummary: WorkSummary;
  availability: UsageAvailability[];
  collectedAt: number;
}
export interface UsageCard {
  toolId: string;
  toolLabel: string;
  buckets: UsageBuckets;
  metrics: UsageMetrics;
  rows: UsageRow[];
}
export interface UsageRecords {
  range: UsageRange;
  groupBy: UsageGroupBy;
  cards: UsageCard[];
  availability: UsageAvailability[];
  collectedAt: number;
}
export interface UsageFilters {
  toolIds?: string[];
  projects?: string[];
  providers?: string[];
  models?: string[];
  subagentMode?: "include" | "parentsOnly";
}
export interface UsageSettings {
  enabled: boolean;
  miniBarRange: "today" | "last5h" | "last7d";
  /** 浮窗第 3 行最多列几个工具（**条数**，不是行数）；默认 3（说明书 §P7） */
  miniBarToolRows: number;
  detailRetentionDays: number;
  collectIntervalMin: number;
  providerMapRules: string;
  exportQuote: string;
  exportPose: string;
}
export type UsageSettingsPatch = Partial<UsageSettings>;
export type UsageSourceId =
  "claude" | "codex" | "kimi" | "opencode" | "workbuddy" | "zcode" | "dsh";
export interface UsageSourceStatus {
  sourceId: UsageSourceId;
  ok: boolean;
  parsedFiles: number;
  newRecords: number;
  errorCode?: string;
}
export interface UsageCollectResult {
  collectedAt: number;
  durationMs: number;
  sources: UsageSourceStatus[];
  totalNewRecords: number;
}
