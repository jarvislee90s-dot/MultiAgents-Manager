// 资源管理命令

/// 递归扫描目录，找到所有直接包含 SKILL.md 的子目录
/// 返回相对路径列表（如 "brainstorming", "superpowers/brainstorming"）
fn scan_skill_dirs(base: &std::path::Path) -> Vec<String> {
    let mut results = Vec::new();
    fn recurse(dir: &std::path::Path, base: &std::path::Path, results: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') { continue; }
                    if path.join("SKILL.md").exists() {
                        if let Ok(rel) = path.strip_prefix(base) {
                            results.push(rel.to_string_lossy().to_string());
                        }
                    } else {
                        recurse(&path, base, results);
                    }
                }
            }
        }
    }
    recurse(base, base, &mut results);
    results.sort();
    results
}

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
        "codex" => Some(dirs::home_dir().unwrap_or_default().join(".agents").join("skills")),
        "opencode" => Some(dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
        "openclaw" => Some(dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills")),
        _ => None,
    };
    if let Some(dir) = skill_dir {
        if dir.exists() {
            let existing = crate::database::list_extensions();
            let skill_names = scan_skill_dirs(&dir);
            for name in skill_names {
                let path = dir.join(&name);
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

    // 补充 SSOT 仓库中已有但未在 DB extensions 中的 skill
    let mam_skills = dirs::home_dir().unwrap_or_default().join(".mam").join("skills");
    let ssot_skill_names = scan_skill_dirs(&mam_skills);
    let mut global_with_status: Vec<_> = global.iter()
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

    // 补充 SSOT 中的 skill（不在 DB extensions 里的）
    for name in &ssot_skill_names {
        let ext_id = format!("skill-{}", name);
        if !global_with_status.iter().any(|g| g["id"].as_str() == Some(&ext_id)) {
            let assignment = assignments.iter().find(|a| a.extension_id == ext_id);
            global_with_status.push(serde_json::json!({
                "id": ext_id,
                "kind": "skill",
                "name": name,
                "description": null,
                "sourcePath": mam_skills.join(name).to_string_lossy(),
                "sourceTool": null,
                "suite": null,
                "tags": null,
                "assignments": assignment.map(|a| vec![serde_json::json!({
                    "agentToolId": a.agent_tool_id,
                    "enabled": a.enabled,
                    "linkStatus": a.link_status,
                })]).unwrap_or_default(),
            }));
        }
    }

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

    // 构建工具 → skill 目录映射，用于检测原生生效的 skill
    let tool_skill_dirs: Vec<(&str, std::path::PathBuf)> = {
        use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, openclaw::OpenClawAdapter, AgentAdapter};
        let adapters: Vec<(Box<dyn AgentAdapter>, &str)> = vec![
            (Box::new(ClaudeAdapter), "claude"),
            (Box::new(CodexAdapter), "codex"),
            (Box::new(OpenCodeAdapter), "opencode"),
            (Box::new(OpenClawAdapter), "openclaw"),
        ];
        adapters.into_iter()
            .filter_map(|(a, id)| a.skill_dirs().into_iter().next().map(|d| (id, d)))
            .collect()
    };

    let scan_skills = |dir: &std::path::Path| -> Vec<SsotResource> {
        let names = scan_skill_dirs(dir);
        names.into_iter().map(|name| {
            let ext_id = format!("skill-{}", name);
            // 1) DB 中有 enabled=true 的记录
            let mut enabled_tools: Vec<String> = assignments.iter()
                .filter(|a| a.extension_id == ext_id && a.enabled)
                .map(|a| a.agent_tool_id.clone())
                .collect();
            // 2) 补充：检查各工具原生 skill 目录中是否存在（非符号链接的实际目录也算已生效）
            for (tool_id, tool_dir) in &tool_skill_dirs {
                if enabled_tools.iter().any(|t| t == tool_id) { continue; }
                if tool_dir.join(&name).exists() {
                    enabled_tools.push(tool_id.to_string());
                }
            }
            SsotResource { name, kind: "skill".to_string(), enabled_tools }
        }).collect()
    };

    // 构建工具 → MCP 配置路径映射，用于扫描各工具已有的 MCP 服务器
    let tool_mcp_configs: Vec<(&str, std::path::PathBuf, crate::adapter::McpFormat)> = {
        use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, openclaw::OpenClawAdapter, AgentAdapter};
        let adapters: Vec<(Box<dyn AgentAdapter>, &str)> = vec![
            (Box::new(ClaudeAdapter), "claude"),
            (Box::new(CodexAdapter), "codex"),
            (Box::new(OpenCodeAdapter), "opencode"),
            (Box::new(OpenClawAdapter), "openclaw"),
        ];
        adapters.into_iter()
            .filter_map(|(a, id)| {
                let path = a.mcp_config_path()?;
                Some((id, path, a.mcp_format()))
            })
            .collect()
    };

    // MCP 扫描：从各工具配置文件中提取 MCP 服务器列表 + DB assignment
    let scan_mcp = || -> Vec<SsotResource> {
        let mut all_mcps: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for (tool_id, config_path, format) in &tool_mcp_configs {
            let content = std::fs::read_to_string(config_path).unwrap_or_default();
            let servers: serde_json::Value = match format {
                crate::adapter::McpFormat::Json | crate::adapter::McpFormat::Jsonc => {
                    serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
                }
                crate::adapter::McpFormat::Toml => {
                    let toml_val: Result<toml::Value, _> = content.parse();
                    toml_val.map(|v| {
                        let json_str = serde_json::to_string(&v).unwrap_or_default();
                        serde_json::from_str(&json_str).unwrap_or(serde_json::json!({}))
                    }).unwrap_or(serde_json::json!({}))
                }
            };
            let mcp_obj = servers.get("mcpServers")
                .or_else(|| servers.get("mcp_servers"))
                .or_else(|| servers.get("mcp"))
                .and_then(|v| v.as_object());
            if let Some(obj) = mcp_obj {
                for name in obj.keys() {
                    all_mcps.entry(name.clone()).or_default().push(tool_id.to_string());
                }
            }
        }
        // 合并 DB assignment
        for assignment in &assignments {
            if assignment.extension_id.starts_with("mcp-") && assignment.enabled {
                let name = assignment.extension_id.strip_prefix("mcp-").unwrap_or("");
                all_mcps.entry(name.to_string()).or_default().push(assignment.agent_tool_id.clone());
            }
        }
        let mut resources: Vec<SsotResource> = all_mcps.into_iter().map(|(name, tools)| {
            SsotResource { name, kind: "mcp".to_string(), enabled_tools: tools }
        }).collect();
        resources.sort_by(|a, b| a.name.cmp(&b.name));
        resources
    };

    let scan_simple = |dir: &std::path::Path, kind: &str| -> Vec<SsotResource> {
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
                resources.push(SsotResource { name, kind: kind.to_string(), enabled_tools });
            }
        }
        resources.sort_by(|a, b| a.name.cmp(&b.name));
        resources
    };

    SsotResources {
        skills: scan_skills(&mam.join("skills")),
        mcp: scan_mcp(),
        plugins: scan_simple(&mam.join("plugins"), "plugin"),
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
    let ssot_skills = scan_skill_dirs(&repo);
    for name in ssot_skills {
        let tool_path = tool_skill_dir.join(&name);
        if tool_path.exists() && !tool_path.is_symlink() {
            duplicates.push(name);
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

/// 检查 skill 在工具目录中的类型：symlink | native | missing
#[tauri::command]
pub fn check_skill_target_type(tool_id: String, skill_name: String) -> String {
    let tool_skill_dir = match tool_id.as_str() {
        "claude" => dirs::home_dir().unwrap_or_default().join(".claude").join("skills"),
        "codex" => dirs::home_dir().unwrap_or_default().join(".agents").join("skills"),
        "opencode" => dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills"),
        "openclaw" => dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills"),
        _ => return "missing".to_string(),
    };
    let target = tool_skill_dir.join(&skill_name);
    if !target.exists() {
        "missing".to_string()
    } else if target.is_symlink() {
        "symlink".to_string()
    } else {
        "native".to_string()
    }
}

/// 取消 skill 的工具配置：移至回收站 + 更新 DB
#[tauri::command]
pub fn disable_skill_for_tool(tool_id: String, skill_name: String) -> Result<String, String> {
    let tool_skill_dir = match tool_id.as_str() {
        "claude" => dirs::home_dir().unwrap_or_default().join(".claude").join("skills"),
        "codex" => dirs::home_dir().unwrap_or_default().join(".agents").join("skills"),
        "opencode" => dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills"),
        "openclaw" => dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills"),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };
    let target = tool_skill_dir.join(&skill_name);
    if !target.exists() {
        return Err("目标路径不存在".to_string());
    }

    let target_type = if target.is_symlink() { "symlink" } else { "native" };

    // 尝试用 trash 命令移至回收站
    let result = std::process::Command::new("trash")
        .arg(&target)
        .output();

    match result {
        Ok(output) if output.status.success() => {
            let _ = crate::linker::layer3::cleanup_layer3_on_tool_disable(&skill_name, &tool_id);
            let _ = crate::linker::layer2::unlink_skill_from_layer2(&skill_name, &tool_id);
            let ext_id = format!("skill-{}", skill_name);
            let _ = crate::database::upsert_assignment(&ext_id, &tool_id, false, "missing");
            Ok(target_type.to_string())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("移入回收站失败: {}", stderr))
        }
        Err(e) => {
            log::warn!("trash 命令不可用，回退到直接删除: {}", e);
            crate::linker::remove_link(&target)?;
            let _ = crate::linker::layer3::cleanup_layer3_on_tool_disable(&skill_name, &tool_id);
            let _ = crate::linker::layer2::unlink_skill_from_layer2(&skill_name, &tool_id);
            let ext_id = format!("skill-{}", skill_name);
            let _ = crate::database::upsert_assignment(&ext_id, &tool_id, false, "missing");
            Ok(format!("{}-fallback-rm", target_type))
        }
    }
}

/// 为工具启用 skill（创建符号链接 + DB 记录）
#[tauri::command]
pub fn enable_skill_for_tool_cmd(skill_name: String, tool_id: String) -> Result<(), String> {
    crate::services::enable_skill_for_tool(&skill_name, &tool_id)
}
