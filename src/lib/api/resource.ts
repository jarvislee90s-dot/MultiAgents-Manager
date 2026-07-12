import { invoke } from "@tauri-apps/api/core";
import type { ImportStats, SsotResources } from "@/types/extension";
export async function listExtensionsWithAssignments() { return await invoke("list_extensions_with_assignments"); }
export async function scanNativeResources(toolId: string) { return await invoke("scan_native_resources", { toolId }); }
export async function importNativeResources(items: [string, string][]) { return await invoke<ImportStats>("import_native_resources", { items }); }
export async function listToolResources(toolId: string) { return await invoke("list_tool_resources", { toolId }); }
export async function checkPresetCompatibility(presetId: string, toolId: string) { return await invoke("check_preset_compatibility", { presetId, toolId }); }
export async function listSsotResources() { return await invoke<SsotResources>("list_ssot_resources"); }
export async function detectDuplicateSkills(toolId: string) { return await invoke<string[]>("detect_duplicate_skills", { toolId }); }
export async function cleanupDuplicateSkills(toolId: string, names: string[]) { return await invoke("cleanup_duplicate_skills", { toolId, names }); }
export async function checkSkillTargetType(toolId: string, skillName: string) { return await invoke<string>("check_skill_target_type", { toolId, skillName }); }
export async function disableSkillForTool(toolId: string, skillName: string) { return await invoke<string>("disable_skill_for_tool", { toolId, skillName }); }
export async function enableSkillForTool(skillName: string, toolId: string) { return await invoke("enable_skill_for_tool_cmd", { skillName, toolId }); }
