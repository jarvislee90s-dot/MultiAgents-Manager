export interface PresetItem {
  extensionId: string;
  kind: string;
  extensionName: string;
}

export interface PresetRecord {
  id: string;
  name: string;
  items: PresetItem[];
}

export interface PresetApplyResult {
  successCount: number;
  failures: string[];
  conflicts: string[];
}
