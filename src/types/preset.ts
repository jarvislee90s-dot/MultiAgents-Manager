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
