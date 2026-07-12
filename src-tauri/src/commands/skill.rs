// Skill 管理命令

#[tauri::command]
pub fn list_repo_skills() -> Vec<String> {
    crate::linker::list_repo_skills()
}

#[tauri::command]
pub fn install_skill(source_path: String, name: String) -> Result<(), String> {
    crate::services::install_skill(&source_path, &name)
}

#[tauri::command]
pub fn rescan_skills() -> crate::services::ImportStats {
    crate::services::auto_import_extensions(true)
}

#[tauri::command]
pub fn assign_skill_to_subagent(skill_name: String, tool_id: String, sub_agent_id: String) -> Result<(), String> {
    crate::services::assign_skill_to_subagent(&skill_name, &tool_id, &sub_agent_id)
}
