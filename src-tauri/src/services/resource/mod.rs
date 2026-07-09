// 资源管理服务 - 自动扫描导入 skills 和 plugins

use crate::linker;
use log::info;

/// SKILL.md 元数据
struct SkillMeta {
    name: String,
    description: Option<String>,
}

/// 从 SKILL.md 提取 name 和 description（YAML front matter）
fn parse_skill_meta(skill_md_path: &std::path::Path) -> Option<SkillMeta> {
    let content = std::fs::read_to_string(skill_md_path).ok()?;
    let front_matter = if content.starts_with("---") {
        let after = &content[3..];
        if let Some(end) = after.find("---") {
            &after[..end]
        } else {
            return None;
        }
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

    let skill_sources = [
        ("claude", dirs::home_dir().unwrap_or_default().join(".claude").join("skills")),
        ("codex", dirs::home_dir().unwrap_or_default().join(".codex").join("skills")),
        ("opencode", dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
        ("openclaw", dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills")),
    ];

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
