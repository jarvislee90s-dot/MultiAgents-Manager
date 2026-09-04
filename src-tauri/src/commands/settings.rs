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

/// 工具勾选列表（含 managed 标志：是否存在启用的分配）
#[tauri::command]
pub fn get_tool_settings() -> Vec<crate::services::tool_settings::ToolSetting> {
    crate::services::tool_settings::get_tool_settings()
}

/// 批量保存工具勾选（取消勾选清理 / 重新勾选重建，返回明细）
#[tauri::command]
pub fn update_tool_settings(
    changes: Vec<crate::services::tool_settings::ToolSettingChange>,
) -> crate::services::tool_settings::ApplyResult {
    crate::services::tool_settings::apply_tool_changes(changes)
}

/// 前端工具列下发项（资源视图 / 设置声音区的唯一样式来源）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledTool {
    pub id: String,
    pub label: String,
}

/// 前端工具列的唯一下发源（W5：勾选状态驱动，替代三处硬编码 TOOLS）
#[tauri::command]
pub fn list_enabled_tools() -> Vec<EnabledTool> {
    crate::adapter::TOOL_IDS
        .iter()
        .filter(|id| crate::database::dao::agent_tool::get_tool_enabled(id))
        .filter_map(|id| {
            crate::adapter::adapter_by_id(id).map(|a| EnabledTool {
                id: id.to_string(),
                label: a.name().to_string(),
            })
        })
        .collect()
}
