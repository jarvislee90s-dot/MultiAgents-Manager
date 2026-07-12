// 预设组命令

use crate::database::PresetRecord;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetApplyResult {
    pub success_count: usize,
    pub failures: Vec<String>,
    pub conflicts: Vec<String>,
}

#[tauri::command]
pub fn create_preset(name: String, items: Vec<(String, String)>) -> Result<String, String> {
    crate::database::create_preset(&name, &items)
}

#[tauri::command]
pub fn delete_preset(preset_id: String) -> Result<(), String> {
    crate::database::delete_preset(&preset_id)
}

#[tauri::command]
pub fn list_presets() -> Vec<PresetRecord> {
    crate::database::list_presets()
}

#[tauri::command]
pub fn apply_preset(preset_id: String, tool_id: String) -> PresetApplyResult {
    let result = crate::services::preset::apply_preset(&preset_id, &tool_id);
    PresetApplyResult { success_count: result.success, failures: result.failures, conflicts: result.conflicts }
}

#[tauri::command]
pub fn deactivate_preset(preset_id: String, tool_id: String) -> Result<(), String> {
    crate::services::preset::deactivate_preset(&preset_id, &tool_id)
}

#[tauri::command]
pub fn apply_preset_to_subagent(preset_id: String, tool_id: String, sub_agent_id: String) -> PresetApplyResult {
    let result = crate::services::preset::apply_preset_to_subagent(&preset_id, &tool_id, &sub_agent_id);
    PresetApplyResult { success_count: result.success, failures: result.failures, conflicts: result.conflicts }
}

#[tauri::command]
pub fn deactivate_preset_from_subagent(preset_id: String, tool_id: String, sub_agent_id: String) -> Result<(), String> {
    crate::services::preset::deactivate_preset_from_subagent(&preset_id, &tool_id, &sub_agent_id)
}
