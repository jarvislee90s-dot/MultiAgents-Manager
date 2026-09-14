// 预设组命令

use crate::database::PresetRecord;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetApplyResult {
    pub success_count: usize,
    pub failures: Vec<String>,
    pub conflicts: Vec<String>,
    pub stashed: Vec<String>,
    pub disabled: Vec<String>,
    pub restored_native: Vec<String>,
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
pub fn apply_preset(preset_id: String, tool_id: String) -> Result<PresetApplyResult, String> {
    // review F4：停用工具的预设写操作一律拒绝（W5 生效范围）
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    let result = crate::services::preset::apply_preset(&preset_id, &tool_id)?;
    Ok(PresetApplyResult {
        success_count: result.success,
        failures: result.failures,
        conflicts: result.conflicts,
        stashed: result.stashed,
        disabled: result.disabled,
        restored_native: result.restored_native,
    })
}

#[tauri::command]
pub fn deactivate_preset(preset_id: String, tool_id: String) -> Result<(), String> {
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    crate::services::preset::deactivate_preset(&preset_id, &tool_id)
}

#[tauri::command]
pub fn apply_preset_to_subagent(
    preset_id: String,
    tool_id: String,
    sub_agent_id: String,
) -> Result<PresetApplyResult, String> {
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    let result =
        crate::services::preset::apply_preset_to_subagent(&preset_id, &tool_id, &sub_agent_id);
    Ok(PresetApplyResult {
        success_count: result.success,
        failures: result.failures,
        conflicts: result.conflicts,
        // 子 Agent 级应用不触发独占清扫/暂存（工具级专属语义），恒为空
        stashed: Vec::new(),
        disabled: Vec::new(),
        restored_native: Vec::new(),
    })
}

#[tauri::command]
pub fn deactivate_preset_from_subagent(
    preset_id: String,
    tool_id: String,
    sub_agent_id: String,
) -> Result<(), String> {
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    crate::services::preset::deactivate_preset_from_subagent(&preset_id, &tool_id, &sub_agent_id)
}
