// 预设组 v2 API 封装：与 src-tauri/src/commands/preset.rs 一一对应（参数 camelCase 由 Tauri 自动映射 snake_case）
import { invoke } from "@tauri-apps/api/core";
import type { ActivePreset, ResourceBinding } from "@/types/extension";
import type { ApplyPreview, PresetApplyResult, PresetRecord, RestoreResult } from "@/types/preset";

export async function listPresets(): Promise<PresetRecord[]> {
  return await invoke<PresetRecord[]>("list_presets");
}
export async function getPreset(presetId: string): Promise<PresetRecord | null> {
  return await invoke<PresetRecord | null>("get_preset", { presetId });
}
export async function createPreset(
  name: string,
  items: [string, string][],
  description?: string,
  scope?: string,
  boundTool?: string
): Promise<string> {
  return await invoke<string>("create_preset", { name, items, description, scope, boundTool });
}
export async function updatePreset(
  id: string,
  name: string,
  description: string,
  scope: string,
  boundTool: string | null,
  items: [string, string][]
): Promise<void> {
  return await invoke("update_preset", { id, name, description, scope, boundTool, items });
}
export async function deletePreset(presetId: string): Promise<void> {
  return await invoke("delete_preset", { presetId });
}
export async function applyPreset(presetId: string, toolId: string): Promise<PresetApplyResult> {
  return await invoke<PresetApplyResult>("apply_preset", { presetId, toolId });
}
// 工具级 deactivate 封装已退役（T8）：v2 预设×工具开关的「关」走 restore_preset
// 恢复默认，不再直呼 deactivate_preset（Rust 命令保留，子 Agent 级 deactivatePresetFromSubagent 仍用旧命令）
export async function restorePreset(toolId: string): Promise<RestoreResult> {
  return await invoke<RestoreResult>("restore_preset", { toolId });
}
export async function getActivePreset(toolId: string): Promise<string | null> {
  return await invoke<string | null>("get_active_preset", { toolId });
}
export async function listActivePresets(): Promise<ActivePreset[]> {
  return await invoke<ActivePreset[]>("list_active_presets");
}
export async function getToolActiveResources(toolId: string): Promise<[string, string, string][]> {
  return await invoke<[string, string, string][]>("get_tool_active_resources", { toolId });
}
export async function previewApplyPreset(presetId: string, toolId: string): Promise<ApplyPreview> {
  return await invoke<ApplyPreview>("preview_apply_preset", { presetId, toolId });
}
export async function applyPresetToSubagent(
  presetId: string,
  toolId: string,
  subAgentId: string
): Promise<PresetApplyResult> {
  return await invoke<PresetApplyResult>("apply_preset_to_subagent", {
    presetId,
    toolId,
    subAgentId,
  });
}
export async function deactivatePresetFromSubagent(
  presetId: string,
  toolId: string,
  subAgentId: string
): Promise<void> {
  return await invoke("deactivate_preset_from_subagent", { presetId, toolId, subAgentId });
}

// —— 资源独占绑定 / 常驻豁免 ——

export async function setResourceBinding(
  extensionId: string,
  exclusiveTools: string[],
  reason?: string
): Promise<void> {
  return await invoke("set_resource_binding", { extensionId, exclusiveTools, reason });
}
export async function listResourceBindings(): Promise<ResourceBinding[]> {
  return await invoke<ResourceBinding[]>("list_resource_bindings");
}
export async function deleteResourceBinding(extensionId: string): Promise<void> {
  return await invoke("delete_resource_binding", { extensionId });
}
export async function setToolResident(
  toolId: string,
  extensionId: string,
  resident: boolean
): Promise<void> {
  return await invoke("set_tool_resident", { toolId, extensionId, resident });
}
export async function listToolResidents(toolId: string): Promise<string[]> {
  return await invoke<string[]>("list_tool_residents", { toolId });
}
