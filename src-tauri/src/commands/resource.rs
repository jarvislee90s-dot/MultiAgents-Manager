// 资源管理命令

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionWithAssignments {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub source_path: String,
    pub suite: Option<String>,
    pub source_tool: Option<String>,
    pub tags: Option<String>,
    pub assignments: Vec<AssignmentSummary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentSummary {
    pub agent_tool_id: String,
    pub enabled: bool,
    pub link_status: String,
}

#[tauri::command]
pub fn list_extensions_with_assignments() -> Vec<ExtensionWithAssignments> {
    let extensions = crate::database::list_extensions();
    let assignments = crate::database::list_all_assignments();
    extensions.iter().map(|ext| {
        let ext_assignments: Vec<AssignmentSummary> = assignments.iter()
            .filter(|a| a.extension_id == ext.id)
            .map(|a| AssignmentSummary {
                agent_tool_id: a.agent_tool_id.clone(),
                enabled: a.enabled,
                link_status: a.link_status.clone(),
            })
            .collect();
        ExtensionWithAssignments {
            id: ext.id.clone(), kind: ext.kind.clone(), name: ext.name.clone(),
            description: ext.description.clone(), source_path: ext.source_path.clone(),
            suite: ext.suite.clone(), source_tool: ext.source_tool.clone(),
            tags: ext.tags.clone(), assignments: ext_assignments,
        }
    }).collect()
}

#[tauri::command]
pub fn scan_native_resources(tool_id: String) -> Vec<crate::database::NativeExtensionRecord> {
    let mut results = Vec::new();
    let skill_dir = match tool_id.as_str() {
        "claude" => Some(dirs::home_dir().unwrap_or_default().join(".claude").join("skills")),
        "codex" => Some(dirs::home_dir().unwrap_or_default().join(".codex").join("skills")),
        "opencode" => Some(dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
        "openclaw" => Some(dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills")),
        _ => None,
    };
    if let Some(dir) = skill_dir {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let existing = crate::database::list_extensions();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let ext_id = format!("skill-{}", name);
                        let exists = existing.iter().any(|e| e.id == ext_id);
                        if !exists {
                            results.push(crate::database::NativeExtensionRecord {
                                id: ext_id, kind: "skill".to_string(), name: name.clone(),
                                description: None, source_path: path.to_string_lossy().to_string(),
                                source_tool: tool_id.clone(), detected_at: chrono::Utc::now().to_rfc3339(),
                                imported: false,
                            });
                        }
                    }
                }
            }
        }
    }
    results
}

#[tauri::command]
pub fn import_native_resources(items: Vec<(String, String)>) -> crate::services::ImportStats {
    let mut imported = 0;
    let mut skipped = 0;
    for (source_path, name) in items {
        let path = std::path::Path::new(&source_path);
        if !path.exists() { skipped += 1; continue; }
        if let Err(e) = crate::linker::install_to_repo(path, &name) {
            log::warn!("导入 {} 失败: {}", name, e); skipped += 1; continue;
        }
        let ext = crate::database::ExtensionRecord {
            id: format!("skill-{}", name), kind: "skill".to_string(), name: name.clone(),
            description: None, source_path: source_path.clone(), source_url: None,
            version: None, tags: None, suite: None, source_tool: None, is_native: true,
        };
        let _ = crate::database::insert_extension(&ext);
        imported += 1;
    }
    crate::services::ImportStats {
        imported, newly_added: imported, skipped_dup: skipped, source_counts: vec![],
    }
}

#[tauri::command]
pub fn list_tool_resources(tool_id: String) -> serde_json::Value {
    let global = crate::database::list_extensions();
    let native = scan_native_resources(tool_id.clone());
    let assignments = crate::database::list_assignments(&tool_id);
    // 返回所有全局资源，带 assignment 状态标记，前端决定是否显示已禁用的
    let global_with_status: Vec<_> = global.iter()
        .map(|e| {
            let assignment = assignments.iter().find(|a| a.extension_id == e.id);
            serde_json::json!({
                "id": e.id,
                "kind": e.kind,
                "name": e.name,
                "description": e.description,
                "sourcePath": e.source_path,
                "sourceTool": e.source_tool,
                "suite": e.suite,
                "tags": e.tags,
                "assignments": assignment.map(|a| vec![serde_json::json!({
                    "agentToolId": a.agent_tool_id,
                    "enabled": a.enabled,
                    "linkStatus": a.link_status,
                })]).unwrap_or_default(),
            })
        })
        .collect();
    serde_json::json!({ "global": global_with_status, "native": native })
}

#[tauri::command]
pub fn check_preset_compatibility(preset_id: String, tool_id: String) -> crate::services::preset::CompatibilityReport {
    crate::services::preset::check_compatibility(&preset_id, &tool_id)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SsotResource {
    pub name: String,
    pub kind: String,
    pub enabled_tools: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SsotResources {
    pub skills: Vec<SsotResource>,
    pub mcp: Vec<SsotResource>,
    pub plugins: Vec<SsotResource>,
}

/// 扫描 SSOT 仓库目录，返回三类资源的完整清单
#[tauri::command]
pub fn list_ssot_resources() -> SsotResources {
    let mam = dirs::home_dir().unwrap_or_default().join(".mam");
    let assignments = crate::database::list_all_assignments();

    let scan_dir = |dir: &std::path::Path, kind: &str| -> Vec<SsotResource> {
        let mut resources = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') { continue; }
                let ext_id = format!("{}-{}", kind, name);
                let enabled_tools: Vec<String> = assignments.iter()
                    .filter(|a| a.extension_id == ext_id && a.enabled)
                    .map(|a| a.agent_tool_id.clone())
                    .collect();
                resources.push(SsotResource {
                    name,
                    kind: kind.to_string(),
                    enabled_tools,
                });
            }
        }
        resources.sort_by(|a, b| a.name.cmp(&b.name));
        resources
    };

    SsotResources {
        skills: scan_dir(&mam.join("skills"), "skill"),
        mcp: scan_dir(&mam.join("mcp"), "mcp"),
        plugins: scan_dir(&mam.join("plugins"), "plugin"),
    }
}

/// 检测指定工具下所有在 SSOT 和原始目录中都存在的重复 skill
#[tauri::command]
pub fn detect_duplicate_skills(tool_id: String) -> Vec<String> {
    let repo = dirs::home_dir().unwrap_or_default().join(".mam").join("skills");
    let tool_skill_dir = match tool_id.as_str() {
        "claude" => Some(dirs::home_dir().unwrap_or_default().join(".claude").join("skills")),
        "codex" => Some(dirs::home_dir().unwrap_or_default().join(".agents").join("skills")),
        "opencode" => Some(dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
        "openclaw" => Some(dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills")),
        _ => None,
    };

    let tool_skill_dir = match tool_skill_dir {
        Some(d) => d,
        None => return Vec::new(),
    };

    if !repo.exists() || !tool_skill_dir.exists() {
        return Vec::new();
    }

    let mut duplicates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&repo) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') { continue; }
            let tool_path = tool_skill_dir.join(&name);
            if tool_path.exists() && !tool_path.is_symlink() {
                duplicates.push(name);
            }
        }
    }
    duplicates.sort();
    duplicates
}

/// 清理指定工具下的重复 skill（delete 原始目录，替换为符号链接）
#[tauri::command]
pub fn cleanup_duplicate_skills(tool_id: String, names: Vec<String>) -> Result<(), String> {
    let repo = dirs::home_dir().unwrap_or_default().join(".mam").join("skills");
    let tool_skill_dir = match tool_id.as_str() {
        "claude" => dirs::home_dir().unwrap_or_default().join(".claude").join("skills"),
        "codex" => dirs::home_dir().unwrap_or_default().join(".agents").join("skills"),
        "opencode" => dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills"),
        "openclaw" => dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills"),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };

    let mut cleaned = 0;
    let mut errors = Vec::new();

    for name in &names {
        let ssot_path = repo.join(name);
        let tool_path = tool_skill_dir.join(name);

        match crate::linker::replace_with_symlink(&ssot_path, &tool_path) {
            Ok(()) => {
                let ext_id = format!("skill-{}", name);
                let _ = crate::database::upsert_assignment(&ext_id, &tool_id, true, "symlinked");
                cleaned += 1;
            }
            Err(e) => {
                log::warn!("清理 skill {} 失败: {}", name, e);
                errors.push(format!("{}: {}", name, e));
            }
        }
    }

    if !errors.is_empty() {
        Err(format!("部分清理失败 (成功 {}/{}): {}", cleaned, cleaned + errors.len(), errors.join("; ")))
    } else {
        Ok(())
    }
}
