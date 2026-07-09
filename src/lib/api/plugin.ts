import { invoke } from "@tauri-apps/api/core";
export async function togglePluginForTool(pluginName: string, toolId: string, enabled: boolean, kind: string) { return await invoke("toggle_plugin_for_tool", { pluginName, toolId, enabled, kind }); }
