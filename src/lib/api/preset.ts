import { invoke } from "@tauri-apps/api/core";
export async function listPresets() { return await invoke("list_presets"); }
export async function createPreset(name: string, items: [string, string][]) { return await invoke("create_preset", { name, items }); }
export async function deletePreset(presetId: string) { return await invoke("delete_preset", { presetId }); }
export async function applyPreset(presetId: string, toolId: string) { return await invoke("apply_preset", { presetId, toolId }); }
export async function deactivatePreset(presetId: string, toolId: string) { return await invoke("deactivate_preset", { presetId, toolId }); }
export async function applyPresetToSubagent(presetId: string, toolId: string, subAgentId: string) { return await invoke("apply_preset_to_subagent", { presetId, toolId, subAgentId }); }
export async function deactivatePresetFromSubagent(presetId: string, toolId: string, subAgentId: string) { return await invoke("deactivate_preset_from_subagent", { presetId, toolId, subAgentId }); }
