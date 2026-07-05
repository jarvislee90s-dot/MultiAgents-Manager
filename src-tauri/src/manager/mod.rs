// Manager 统一入口 — 按 ExtensionKind 分发到 linker/mcp.rs/plugin.rs
// Skill 只能通过预设组分配（无单独 toggle），MCP/Plugin 可单独 toggle

pub mod mcp;
pub mod preset;

use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, AgentAdapter};
use crate::linker;
use crate::store;
use log::info;

/// 获取工具的 skill 目录
fn get_tool_skill_dir(tool_id: &str) -> Option<std::path::PathBuf> {
    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        _ => return None,
    };
    adapter.skill_dirs().into_iter().next()
}

/// 安装 skill 到全局仓库
pub fn install_skill(source_path: &str, name: &str) -> Result<(), String> {
    let source = std::path::Path::new(source_path);
    if !source.exists() {
        return Err(format!("源路径不存在: {}", source_path));
    }
    linker::install_to_repo(source, name)?;
    let ext = store::ExtensionRecord {
        id: format!("skill-{}", name),
        kind: "skill".to_string(),
        name: name.to_string(),
        description: None,
        source_path: source_path.to_string(),
        source_url: None,
        version: None,
        tags: None,
        suite: None,
        source_tool: None,
    };
    store::insert_extension(&ext)?;
    info!("Skill 安装到全局仓库: {}", name);
    Ok(())
}

/// 为工具启用 skill（创建 symlink）
pub fn enable_skill_for_tool(skill_name: &str, tool_id: &str) -> Result<(), String> {
    let repo = linker::ensure_repo_dir();
    let source = repo.join(skill_name);
    if !source.exists() {
        return Err(format!("Skill 不在全局仓库中: {}", skill_name));
    }
    let target_dir = get_tool_skill_dir(tool_id)
        .ok_or(format!("工具 {} 无 skill 目录", tool_id))?;
    let _ = std::fs::create_dir_all(&target_dir);
    let target = target_dir.join(skill_name);
    linker::create_link(&source, &target)?;
    let ext_id = format!("skill-{}", skill_name);
    store::upsert_assignment(&ext_id, tool_id, true, "valid")?;
    info!("Skill {} 已为 {} 启用", skill_name, tool_id);
    Ok(())
}

/// 为工具禁用 skill（移除 symlink）
pub fn disable_skill_for_tool(skill_name: &str, tool_id: &str) -> Result<(), String> {
    let target_dir = get_tool_skill_dir(tool_id)
        .ok_or(format!("工具 {} 无 skill 目录", tool_id))?;
    let target = target_dir.join(skill_name);
    linker::remove_link(&target)?;
    let ext_id = format!("skill-{}", skill_name);
    store::upsert_assignment(&ext_id, tool_id, false, "missing")?;
    info!("Skill {} 已为 {} 禁用", skill_name, tool_id);
    Ok(())
}

/// 为工具启用/禁用 MCP 服务器
pub fn toggle_mcp(mcp_name: &str, tool_id: &str, enabled: bool) -> Result<(), String> {
    if enabled {
        // 从全局仓库读取 MCP 配置
        let repo = dirs::home_dir().unwrap_or_default().join(".mam").join("mcp");
        let config_path = repo.join(format!("{}.json", mcp_name));
        if !config_path.exists() {
            return Err(format!("MCP 配置不在全局仓库中: {}", mcp_name));
        }
        let content = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
        let config: mcp::McpConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        mcp::write_mcp(tool_id, mcp_name, &config)?;
    } else {
        mcp::remove_mcp(tool_id, mcp_name)?;
    }
    let ext_id = format!("mcp-{}", mcp_name);
    store::upsert_assignment(&ext_id, tool_id, enabled, if enabled { "valid" } else { "missing" })?;
    info!("MCP {} 已为 {} {}", mcp_name, tool_id, if enabled { "启用" } else { "禁用" });
    Ok(())
}

// ===== 子 Agent 检测（US6）=====

/// 检测工具的子 Agent 列表
pub fn detect_subagents(tool_id: &str) -> Vec<String> {
    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        _ => return Vec::new(),
    };
    if let Some(dir) = adapter.subagent_dir() {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                return entries.flatten()
                    .filter_map(|e| {
                        e.path().file_stem()
                            .and_then(|s| s.to_str())
                            .map(String::from)
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}


// ===== 子 Agent 级分配约束（T058/T059）=====

/// 检查 skill 是否在工具级范围内
pub fn is_skill_in_tool_range(skill_name: &str, tool_id: &str) -> bool {
    let ext_id = format!("skill-{}", skill_name);
    let assignments = crate::store::list_assignments(tool_id);
    assignments.iter().any(|a| a.extension_id == ext_id && a.enabled)
}

/// 为子 Agent 分配 skill（带约束检查）
pub fn assign_skill_to_subagent(skill_name: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    // 约束：必须在工具级范围内
    if !is_skill_in_tool_range(skill_name, tool_id) {
        return Err(format!("Skill {} 未在 {} 的工具级分配中启用，无法分配给子 Agent", skill_name, tool_id));
    }

    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };

    let repo = linker::ensure_repo_dir();
    let source = repo.join(skill_name);
    if !source.exists() {
        return Err(format!("Skill 不在全局仓库中: {}", skill_name));
    }

    // 子 Agent 目录：工具 skill 目录下的子目录
    if let Some(skill_dir) = adapter.skill_dirs().into_iter().next() {
        let subagent_dir = skill_dir.join("subagents").join(sub_agent_id);
        let _ = std::fs::create_dir_all(&subagent_dir);
        let target = subagent_dir.join(skill_name);
        linker::create_link(&source, &target)?;

        let ext_id = format!("skill-{}", skill_name);
        crate::store::upsert_assignment(&ext_id, tool_id, true, "valid")?;
        info!("Skill {} 已分配给子 Agent {}", skill_name, sub_agent_id);
    }
    Ok(())
}


// ===== 首次启动自动导入已有 skills =====

/// SKILL.md 元数据
struct SkillMeta {
    name: String,
    description: Option<String>,
}

/// 从 SKILL.md 提取 name 和 description（YAML front matter）
fn parse_skill_meta(skill_md_path: &std::path::Path) -> Option<SkillMeta> {
    let content = std::fs::read_to_string(skill_md_path).ok()?;
    
    // 提取 YAML front matter（--- 之间的内容）
    let front_matter = if content.starts_with("---") {
        let after = &content[3..];
        if let Some(end) = after.find("---") {
            &after[..end]
        } else {
            return None;
        }
    } else {
        // 没有标准 front matter，尝试从全文提取
        &content[..]
    };

    // 提取 name: 字段
    let name = front_matter.lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("name:").map(|v| {
                v.trim().trim_matches(char::from(34)).to_string()
            })
        })?;

    if name.is_empty() { return None; }

    // 提取 description: 字段
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
    // 方法 1：如果路径深度 > skills_root + 1，说明有父目录（套件）
    if let Ok(relative) = skill_path.strip_prefix(skills_root) {
        let components: Vec<_> = relative.components().collect();
        if components.len() > 1 {
            // 有父目录 → 套件名 = 第一级目录名
            return components[0].as_os_str().to_str().map(String::from);
        }
    }
    // 方法 2：从名称前缀推断（如 speckit-constitution → speckit）
    if let Some(dash_pos) = skill_name.find('-') {
        let prefix = &skill_name[..dash_pos];
        // 只对已知套件前缀推断（避免把 "chinese-code" 误判为 "chinese" 套件）
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
                // 检查是否有 SKILL.md
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    if let Some(meta) = parse_skill_meta(&skill_md) {
                        let suite = detect_suite(&meta.name, &path, skills_root);
                        results.push((path.clone(), meta.name));
                        // 如果有套件名，保存到 meta（通过返回值传递不了，直接记录）
                        // 这里简化：返回 (path, name)，suite 在导入时重新检测
                        let _ = suite;
                    }
                }
                // 递归扫描子目录（即使当前目录有 SKILL.md，也可能有子技能）
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
    pub skipped_dup: usize,
    /// 每个工具找到的 skill 数 [(tool_id, count)]
    pub source_counts: Vec<(String, usize)>,
}

/// 扫描各工具的 skill 目录，递归导入到全局仓库（含去重）
/// force=true 时跳过"已导入过"检查，用于前端"重新扫描"按钮
pub fn auto_import_skills(force: bool) -> ImportStats {
    let _repo = linker::ensure_repo_dir();

    if !force {
        // 首次启动：已导入过就跳过
        let existing = crate::store::list_extensions();
        if !existing.is_empty() {
            log::debug!("Skills already imported ({} in DB), skipping auto-import", existing.len());
            return ImportStats { imported: 0, skipped_dup: 0, source_counts: Vec::new() };
        }
    }

    // 各工具的 skill 目录
    let skill_sources = [
        ("claude", dirs::home_dir().unwrap_or_default().join(".claude").join("skills")),
        ("codex", dirs::home_dir().unwrap_or_default().join(".agents").join("skills")),
        ("opencode", dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
    ];

    // 收集所有 skill：name → (path, source_tool, suite)
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
            // 去重：同名 skill 只导入一次
            if seen_names.contains(skill_name) {
                skipped_dup += 1;
                log::debug!("跳过重复 skill: {} (来自 {})", skill_name, tool_id);
                continue;
            }
            seen_names.insert(skill_name.clone());

            // 提取元数据
            let meta = parse_skill_meta(&skill_path.join("SKILL.md"));
            let description = meta.as_ref().and_then(|m| m.description.clone());
            let suite = detect_suite(skill_name, skill_path, skills_dir);

            // 复制到全局仓库
            if let Err(e) = linker::install_to_repo(skill_path, skill_name) {
                log::warn!("导入 skill {} 失败: {}", skill_name, e);
                continue;
            }

            // 记录到数据库
            let ext = crate::store::ExtensionRecord {
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
            };
            let _ = crate::store::insert_extension(&ext);
            imported += 1;
        }
    }

    if imported > 0 {
        log::info!("首次导入完成: {} 个 skill (跳过 {} 个重复)", imported, skipped_dup);
    }
    ImportStats { imported, skipped_dup, source_counts }
}
