// Skill 管理命令

/// 手动安装返回（Task 17）：success + frontmatter 专属预填建议。
/// `Result<(), String>` 改为结构体是破坏性 TS 变更，调用方（src/lib/api/skill.ts）
/// 已同步类型；目前前端消费方均忽略返回值，形状仅保证类型检查通过
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub success: bool,
    pub suggestion: Option<crate::services::resource::frontmatter::FrontmatterSuggestion>,
}

#[tauri::command]
pub fn list_repo_skills() -> Vec<String> {
    crate::linker::list_repo_skills()
}

#[tauri::command]
pub fn install_skill(
    source_path: String,
    name: String,
    overwrite: Option<bool>,
) -> Result<ImportOutcome, String> {
    crate::services::install_skill(&source_path, &name, overwrite.unwrap_or(false))?;
    // 建议计算失败（SKILL.md 读不到等）不阻断安装结果 → None（spec §6：只建议不强制；
    // 2026-09-15 裁决：手动导入弹提示当场确认，由前端呈现 Dialog）
    let suggestion = crate::services::resource::frontmatter::suggest_for_skill(&name);
    Ok(ImportOutcome {
        success: true,
        suggestion,
    })
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
