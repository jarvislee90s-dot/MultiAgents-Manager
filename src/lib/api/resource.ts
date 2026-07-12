import { invoke } from "@tauri-apps/api/core";
import type { ImportStats } from "@/types/extension";
export async function listExtensionsWithAssignments() { return await invoke("list_extensions_with_assignments"); }
export async function scanNativeResources(toolId: string) { return await invoke("scan_native_resources", { toolId }); }
export async function importNativeResources(items: [string, string][]) { return await invoke<ImportStats>("import_native_resources", { items }); }
export async function listToolResources(toolId: string) { return await invoke("list_tool_resources", { toolId }); }
export async function checkPresetCompatibility(presetId: string, toolId: string) { return await invoke("check_preset_compatibility", { presetId, toolId }); }
