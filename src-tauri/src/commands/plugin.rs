// 插件管理命令

#[tauri::command]
pub fn toggle_plugin_for_tool(plugin_name: String, tool_id: String, enabled: bool, kind: String) -> Result<(), String> {
    crate::services::plugin::toggle_plugin(&plugin_name, &tool_id, enabled, &kind)
}
