// 资源管理命令

use tauri::{Builder, Runtime};

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
    let global_filtered: Vec<_> = global.iter()
        .filter(|e| assignments.iter().any(|a| a.extension_id == e.id && a.enabled))
        .collect();
    serde_json::json!({ "global": global_filtered, "native": native })
}

#[tauri::command]
pub fn check_preset_compatibility(preset_id: String, tool_id: String) -> crate::services::preset::CompatibilityReport {
    crate::services::preset::check_compatibility(&preset_id, &tool_id)
}

pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        list_extensions_with_assignments, scan_native_resources, import_native_resources,
        list_tool_resources, check_preset_compatibility
    ])
}
