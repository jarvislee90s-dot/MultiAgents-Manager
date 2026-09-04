// 设置、工具检测、子 Agent 命令

use crate::database::SubAgentRecord;

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

/// 手动关闭未读卡（X 按钮）→ 标记已读（删除未读行）
#[tauri::command]
pub fn mark_session_read(agent_type: String, session_id: String) {
    crate::database::dao::unread::delete(&agent_type.to_lowercase(), &session_id);
}
