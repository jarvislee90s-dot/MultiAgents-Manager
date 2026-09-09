// 遗留 codex 技能链接检测/迁移 IPC 封装（spec §4.3，后端契约 Task 3 已冻结）
import { invoke } from "@tauri-apps/api/core";

// 处置模式：migrate = 迁移到 ~/.codex/skills（codex 私有目录接管）；
// keep = 保留为共享（链接改指 ~/.mam/skills，脱钩 codex 启停）
export type MigrationMode = "migrate" | "keep";

// 逐条迁移报告（与后端 MigrationItemReport serde 契约一致）
export interface MigrationItemReport {
  name: string;
  status: "ok" | "skipped" | "error";
  detail: string | null;
}

// 检测 ~/.agents/skills 下指向 codex 激活目录的 MAM 遗留链接（排序清单，空 = 无遗留）
export async function detectLegacyAgentsLinks(): Promise<string[]> {
  return await invoke<string[]>("detect_legacy_agents_links");
}

// 执行迁移/保留，返回逐条报告（单项失败不回滚已成功项，见 spec §6）
export async function migrateLegacyAgentsLinks(
  mode: MigrationMode
): Promise<MigrationItemReport[]> {
  return await invoke<MigrationItemReport[]>("migrate_legacy_agents_links", { mode });
}
