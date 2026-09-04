// Skill 管理命令

#[tauri::command]
pub fn list_repo_skills() -> Vec<String> {
    crate::linker::list_repo_skills()
}

#[tauri::command]
pub fn install_skill(
    source_path: String,
    name: String,
    overwrite: Option<bool>,
) -> Result<(), String> {
    crate::services::install_skill(&source_path, &name, overwrite.unwrap_or(false))
}

#[tauri::command]
pub fn rescan_skills() -> crate::services::ImportStats {
    let stats = crate::services::auto_import_extensions(true);
    // rescan 同步触发补链与断链修复（spec 015 故事 6 场景 1）
    crate::services::sync_imported_skill_links();
    stats
}

#[tauri::command]
pub fn assign_skill_to_subagent(
    skill_name: String,
    tool_id: String,
    sub_agent_id: String,
) -> Result<(), String> {
    // W5：未勾选工具的分配操作直接拒绝（数据保留在 DB）
    crate::services::tool_settings::ensure_tool_enabled(&tool_id)?;
    crate::services::assign_skill_to_subagent(&skill_name, &tool_id, &sub_agent_id)
}
