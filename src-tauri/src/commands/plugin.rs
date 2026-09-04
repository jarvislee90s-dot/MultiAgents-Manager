// 插件管理命令

#[tauri::command]
pub fn toggle_plugin_for_tool(
    plugin_name: String,
    tool_id: String,
    enabled: bool,
    kind: String,
) -> Result<(), String> {
    // W5：未勾选工具的 toggle 操作直接拒绝（数据保留在 DB）
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    crate::services::plugin::toggle_plugin(&plugin_name, &tool_id, enabled, &kind)
}
