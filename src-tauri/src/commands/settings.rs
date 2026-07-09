// 设置、工具检测、子 Agent 命令

use crate::database::SubAgentRecord;
use tauri::{Builder, Runtime};

#[tauri::command]
pub fn get_setting(key: String) -> Option<String> {
    crate::database::get_setting(&key)
}

#[tauri::command]
pub fn set_setting(key: String, value: String) {
    crate::database::set_setting(&key, &value);
}

#[tauri::command]
pub fn detect_tools() -> Vec<crate::linker::detector::ToolDetection> {
    crate::linker::detector::detect_all_tools()
}

#[tauri::command]
pub fn detect_subagents(tool_id: String) -> Vec<String> {
    crate::services::detect_subagents(&tool_id)
}

#[tauri::command]
pub fn list_sub_agents(tool_id: String) -> Vec<SubAgentRecord> {
    crate::database::list_sub_agents(&tool_id)
}

pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        get_setting, set_setting, detect_tools, detect_subagents, list_sub_agents
    ])
}
