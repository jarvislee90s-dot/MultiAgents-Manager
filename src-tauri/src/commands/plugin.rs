// 插件管理命令

use tauri::{Builder, Runtime};

#[tauri::command]
pub fn toggle_plugin_for_tool(plugin_name: String, tool_id: String, enabled: bool, kind: String) -> Result<(), String> {
    crate::services::plugin::toggle_plugin(&plugin_name, &tool_id, enabled, &kind)
}

pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        toggle_plugin_for_tool
    ])
}
