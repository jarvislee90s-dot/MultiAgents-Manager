// 预设组 v2 类型（与 Rust PresetRecord / PresetApplyResult / RestoreResult / ApplyPreview 对齐，serde camelCase）

/** 预设条目（extension_id + kind 二元组的展开，extensionName 由后端回填展示用） */
export interface PresetItem {
  extensionId: string;
  kind: string;
  extensionName: string;
}

/** 预设组记录：scope = universal（全工具可用）| tool（仅 boundTool 可见） */
export interface PresetRecord {
  id: string;
  name: string;
  description: string;
  scope: "universal" | "tool";
  boundTool: string | null;
  items: PresetItem[];
}

/** 应用结果（含独占清扫产物：暂存 / 停用 / 原生回补） */
export interface PresetApplyResult {
  successCount: number;
  failures: string[];
  conflicts: string[];
  stashed: string[];
  disabled: string[];
  restoredNative: string[];
}

/** 恢复默认结果（回补 MAM 链接 + 回移暂存的原生资源） */
export interface RestoreResult {
  restoredMam: string[];
  restoredNative: string[];
  conflicts: string[];
}

/** 应用预览（确认弹窗 dry-run 数据源） */
export interface ApplyPreview {
  toEnable: string[];
  filtered: string[];
  toDisable: string[];
  toStash: string[];
  residentExempt: string[];
}

// —— 一致性体检（spec §13，Task 14/15 检测+呈现侧，serde camelCase）——

/** 账本-磁盘漂移条目（Rust reconcile::DriftItem）；kind = "L1"|"L2"|"L3"|"L4" */
export interface DriftItem {
  toolId: string;
  kind: "L1" | "L2" | "L3" | "L4";
  extensionId: string;
  path: string;
}

/** 单条对账处置结果（Rust reconcile::ReconcileOutcome）：needs_manual 升级人工；
 *  extensionId/toolId 为结构化行键定位（终审 Minor #4），message 只供人读不解析 */
export interface ReconcileOutcome {
  fixed: boolean;
  needsManual: boolean;
  extensionId: string;
  toolId: string;
  message: string;
}

/** 暂存账本条目（Rust database::StashEntryRecord）；restoredAt = null 即未恢复 */
export interface StashEntryRecord {
  id: number;
  toolId: string;
  skillName: string;
  stashedPath: string;
  originalPath: string;
  createdAt: string;
  restoredAt: string | null;
}

/** 空目录条目（Rust reconcile::EmptyDirItem）：owner = "mam"（~/.mam/skills）
 *  | "tool:<id>"（该工具 primary skill 目录）；path 为绝对路径 */
export interface EmptyDirItem {
  owner: string;
  path: string;
}

/** 预设健康聚合（Rust preset::PresetHealth）：三源合一 + 空目录（wave33），无持久化每次现算 */
export interface PresetHealth {
  invariants: string[];
  stashPending: StashEntryRecord[];
  drift: DriftItem[];
  emptyDirs: EmptyDirItem[];
}
