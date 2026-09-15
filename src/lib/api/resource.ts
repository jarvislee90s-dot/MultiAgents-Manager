import { invoke } from "@tauri-apps/api/core";
import type { ImportStats, SsotResources } from "@/types/extension";
import type { DriftItem, ReconcileOutcome } from "@/types/preset";
export async function listExtensionsWithAssignments() {
  return await invoke("list_extensions_with_assignments");
}
export async function scanNativeResources(toolId: string) {
  return await invoke("scan_native_resources", { toolId });
}
export async function importNativeResources(items: [string, string, string][]) {
  return await invoke<ImportStats>("import_native_resources", { items });
}
export async function listToolResources(toolId: string) {
  return await invoke("list_tool_resources", { toolId });
}
export async function listSsotResources() {
  return await invoke<SsotResources>("list_ssot_resources");
}
export async function detectDuplicateSkills(toolId: string) {
  return await invoke<string[]>("detect_duplicate_skills", { toolId });
}
export async function cleanupDuplicateSkills(toolId: string, names: string[]) {
  return await invoke("cleanup_duplicate_skills", { toolId, names });
}
export async function checkSkillTargetType(toolId: string, skillName: string) {
  return await invoke<string>("check_skill_target_type", { toolId, skillName });
}
export async function disableSkillForTool(toolId: string, skillName: string) {
  return await invoke<string>("disable_skill_for_tool", { toolId, skillName });
}
export async function enableSkillForTool(skillName: string, toolId: string) {
  return await invoke("enable_skill_for_tool_cmd", { skillName, toolId });
}
export async function importMcpToSsot(mcpName: string) {
  return await invoke("import_mcp_to_ssot", { mcpName });
}
export async function saveMcpConfig(
  name: string,
  command: string,
  args: string[],
  env: Record<string, string>
) {
  return await invoke("save_mcp_config", { name, command, args, env });
}

// —— 账本-磁盘对账（spec §13；命令在 src-tauri/src/commands/resource.rs）——

// 扫描 enabled 工具的 L1-L4 漂移清单（disabled 工具按名册语义排除）
export async function scanLedgerDrift(): Promise<DriftItem[]> {
  return await invoke<DriftItem[]>("scan_ledger_drift");
}
// 单条处置。mode: "a" 账本为准修磁盘 | "b" 磁盘为准回写账本；
// 「暂不处理」= 前端收起该行（纯 state），绝不调后端
export async function reconcileItem(item: DriftItem, mode: "a" | "b"): Promise<ReconcileOutcome> {
  return await invoke<ReconcileOutcome>("reconcile_item", { item, mode });
}
// 批量处置：该工具全部漂移逐条按同一 mode 处置（L4 恒 needs_manual；单条失败不中断），
// 返回按扫描顺序的 outcome 数组（message 前缀 "ext_id @ tool_id" 可回映行）
export async function reconcileToolBatch(
  toolId: string,
  mode: "a" | "b"
): Promise<ReconcileOutcome[]> {
  return await invoke<ReconcileOutcome[]>("reconcile_tool_batch", { toolId, mode });
}
