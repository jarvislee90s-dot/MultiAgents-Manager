// 资源管理服务 - 自动扫描导入 skills 和 plugins

use crate::linker;

/// SKILL.md 元数据
struct SkillMeta {
    name: String,
    description: Option<String>,
}

/// 从 SKILL.md 提取 name 和 description（YAML front matter）
fn parse_skill_meta(skill_md_path: &std::path::Path) -> Option<SkillMeta> {
    let content = std::fs::read_to_string(skill_md_path).ok()?;
    let front_matter = if let Some(after) = content.strip_prefix("---") {
        &after[..after.find("---")?]
    } else {
        &content[..]
    };
    let name = front_matter.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("name:").map(|v| {
                v.trim().trim_matches(char::from(34)).to_string()
            })
        })?;
    if name.is_empty() { return None; }
    let description = front_matter.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("description:").map(|v| {
                v.trim().trim_matches(char::from(34)).to_string()
            })
        }).filter(|s| !s.is_empty());
    Some(SkillMeta { name, description })
}

/// 检测套件名称
fn detect_suite(skill_name: &str, skill_path: &std::path::Path, skills_root: &std::path::Path) -> Option<String> {
    if let Ok(relative) = skill_path.strip_prefix(skills_root) {
        let components: Vec<_> = relative.components().collect();
        if components.len() > 1 {
            return components[0].as_os_str().to_str().map(String::from);
        }
    }
    if let Some(dash_pos) = skill_name.find('-') {
        let prefix = &skill_name[..dash_pos];
        let known_suites = ["speckit"];
        if known_suites.contains(&prefix) {
            return Some(prefix.to_string());
        }
    }
    None
}

/// 递归扫描目录下的所有 SKILL.md 文件
fn scan_skills_recursive(dir: &std::path::Path, skills_root: &std::path::Path) -> Vec<(std::path::PathBuf, String)> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    if let Some(meta) = parse_skill_meta(&skill_md) {
                        let _ = detect_suite(&meta.name, &path, skills_root);
                        results.push((path.clone(), meta.name));
                    }
                }
                results.extend(scan_skills_recursive(&path, skills_root));
            }
        }
    }
    results
}

/// 技能导入统计
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStats {
    pub imported: usize,
    pub newly_added: usize,
    pub skipped_dup: usize,
    pub source_counts: Vec<(String, usize)>,
}

/// 从源路径推断 skill 所属工具
fn detect_source_tool(source_path: &str) -> Option<String> {
    let path = std::path::Path::new(source_path);
    for tool_id in ["claude", "codex", "opencode", "openclaw"] {
        if let Some(dir) = crate::adapter::primary_skill_dir(tool_id) {
            if path.starts_with(&dir) {
                return Some(tool_id.to_string());
            }
        }
    }
    None
}

/// 为已导入但尚未建立工具链接的 skill 补链（尊重用户显式禁用）
pub fn sync_imported_skill_links() {
    for ext in crate::database::list_extensions() {
        if ext.kind != "skill" {
            continue;
        }

        let source_tool = ext.source_tool.clone()
            .or_else(|| detect_source_tool(&ext.source_path));
        let Some(tool_id) = source_tool else {
            continue;
        };

        let assignments = crate::database::list_assignments(&tool_id);
        if assignments.iter().any(|a| a.extension_id == ext.id && !a.enabled) {
            continue;
        }

        let already_linked = crate::adapter::primary_skill_dir(&tool_id)
            .map(|dir| dir.join(&ext.name).is_symlink())
            .unwrap_or(false);
        if already_linked {
            continue;
        }

        if let Err(e) = crate::services::enable_skill_for_tool(&ext.name, &tool_id) {
            log::warn!("补链 {} 到 {} 失败: {}", ext.name, tool_id, e);
        }
    }

    // 兼容历史数据：assignment 表里可能已有 skill 记录，但 extensions 表没有对应行
    let mut assignments: Vec<_> = crate::database::list_all_assignments();
    // 先建顶层套件链接，再补嵌套子 skill，避免父目录先被创建成真实目录
    assignments.sort_by_key(|a| a.extension_id.matches('/').count());
    for assignment in assignments {
        if !assignment.enabled {
            continue;
        }
        let Some(skill_name) = assignment.extension_id.strip_prefix("skill-") else {
            continue;
        };
        let repo_skill = crate::linker::ensure_repo_dir().join(skill_name);
        if !repo_skill.exists() {
            continue;
        }

        let already_linked = crate::adapter::primary_skill_dir(&assignment.agent_tool_id)
            .map(|dir| dir.join(skill_name).is_symlink())
            .unwrap_or(false);
        if already_linked {
            continue;
        }

        if let Err(e) = crate::services::enable_skill_for_tool(skill_name, &assignment.agent_tool_id) {
            log::warn!("补链 {} 到 {} 失败: {}", skill_name, assignment.agent_tool_id, e);
        }
    }
}

/// 扫描各工具的 skill 目录，递归导入到全局仓库（含去重）
pub fn auto_import_extensions(force: bool) -> ImportStats {
    let _repo = linker::ensure_repo_dir();
    let existing_before: std::collections::HashSet<String> = crate::database::list_extensions()
        .iter()
        .map(|e| e.name.clone())
        .collect();

    if !force && !existing_before.is_empty() {
        log::debug!("Skills already imported ({} in DB), skipping auto-import", existing_before.len());
        return ImportStats { imported: 0, newly_added: 0, skipped_dup: 0, source_counts: Vec::new() };
    }

    let skill_sources: Vec<(&str, std::path::PathBuf)> = [
        "claude", "codex", "opencode", "openclaw",
    ].into_iter().filter_map(|tool_id| {
        crate::adapter::primary_skill_dir(tool_id)
            .map(|dir| (tool_id, dir))
    }).collect();

    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut imported: usize = 0;
    let mut skipped_dup: usize = 0;
    let mut source_counts: Vec<(String, usize)> = Vec::new();

    for (tool_id, skills_dir) in &skill_sources {
        if !skills_dir.exists() { continue; }
        let found = scan_skills_recursive(skills_dir, skills_dir);
        log::info!("扫描 {} ({}): 找到 {} 个 SKILL.md", tool_id, skills_dir.display(), found.len());
        source_counts.push((tool_id.to_string(), found.len()));

        for (skill_path, skill_name) in &found {
            if seen_names.contains(skill_name) {
                skipped_dup += 1;
                continue;
            }
            seen_names.insert(skill_name.clone());

            let meta = parse_skill_meta(&skill_path.join("SKILL.md"));
            let description = meta.as_ref().and_then(|m| m.description.clone());
            let suite = detect_suite(skill_name, skill_path, skills_dir);

            if let Err(e) = linker::install_to_repo(skill_path, skill_name) {
                log::warn!("导入 skill {} 失败: {}", skill_name, e);
                continue;
            }

            let ext = crate::database::ExtensionRecord {
                id: format!("skill-{}", skill_name),
                kind: "skill".to_string(),
                name: skill_name.clone(),
                description: description.clone(),
                source_path: skill_path.to_string_lossy().to_string(),
                source_url: None,
                version: None,
                tags: Some(tool_id.to_string()),
                suite: suite.clone(),
                source_tool: Some(tool_id.to_string()),
                is_native: false,
            };
            let _ = crate::database::insert_extension(&ext);
            // 默认按来源工具自动创建工具目录链接，让 harness 立即可用
            if let Err(e) = crate::services::enable_skill_for_tool(skill_name, tool_id) {
                log::warn!("导入 {} 后为 {} 创建链接失败: {}", skill_name, tool_id, e);
            }
            imported += 1;
        }
    }

    // Plugin 扫描
    let plugin_sources = [
        ("claude", dirs::home_dir().unwrap_or_default().join(".claude").join("plugins")),
        ("codex", dirs::home_dir().unwrap_or_default().join(".codex").join("plugins")),
        ("opencode", dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("plugins")),
        ("openclaw", dirs::home_dir().unwrap_or_default().join(".openclaw").join("plugins")),
    ];

    for (tool_id, plugins_dir) in &plugin_sources {
        if !plugins_dir.exists() { continue; }
        if let Ok(entries) = std::fs::read_dir(plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if seen_names.contains(&name) { continue; }
                seen_names.insert(name.clone());

                let kind = if path.is_dir() { "file" } else { "config" };
                let plugin_repo = dirs::home_dir().unwrap_or_default().join(".mam").join("plugins");
                let _ = std::fs::create_dir_all(&plugin_repo);
                let dest = plugin_repo.join(&name);
                if dest.exists() {
                    let _ = std::fs::remove_dir_all(&dest);
                }
                if path.is_dir() {
                    let _ = crate::linker::copy_dir_recursive(&path, &dest);
                } else {
                    let _ = std::fs::copy(&path, &dest);
                }

                let ext = crate::database::ExtensionRecord {
                    id: format!("plugin-{}", name),
                    kind: "plugin".to_string(),
                    name: name.clone(),
                    description: None,
                    source_path: path.to_string_lossy().to_string(),
                    source_url: None,
                    version: None,
                    tags: Some(kind.to_string()),
                    suite: None,
                    source_tool: Some(tool_id.to_string()),
                    is_native: false,
                };
                let _ = crate::database::insert_extension(&ext);
                imported += 1;
            }
        }
    }

    let existing_after: std::collections::HashSet<String> = crate::database::list_extensions()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    let newly_added = existing_after.difference(&existing_before).count();

    if imported > 0 {
        log::info!("扫描完成: 处理 {} 个（新增 {} 个，跳过 {} 个重复）", imported, newly_added, skipped_dup);
    }
    ImportStats { imported, newly_added, skipped_dup, source_counts }
}
