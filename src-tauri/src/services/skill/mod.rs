// Skill 管理服务 - 安装、启用、禁用、子 Agent 分配

use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, openclaw::OpenClawAdapter, AgentAdapter};
use crate::linker;
use crate::database;
use log::info;

/// 获取工具的 skill 目录
fn get_tool_skill_dir(tool_id: &str) -> Option<std::path::PathBuf> {
    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        "openclaw" => Box::new(OpenClawAdapter),
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
    let ext = database::ExtensionRecord {
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
        is_native: false,
    };
    database::insert_extension(&ext)?;
    info!("Skill 安装到全局仓库: {}", name);
    Ok(())
}

/// 为工具启用 skill（创建 Layer 2 symlink）
pub fn enable_skill_for_tool(skill_name: &str, tool_id: &str) -> Result<(), String> {
    let layer2_path = crate::linker::layer2::link_skill_to_layer2(skill_name, tool_id)?;

    if let Some(tool_skill_dir) = get_tool_skill_dir(tool_id) {
        let _ = std::fs::create_dir_all(&tool_skill_dir);
        let tool_target = tool_skill_dir.join(skill_name);
        if tool_target.exists() || tool_target.is_symlink() {
            let _ = crate::linker::remove_link(&tool_target);
        }
        crate::linker::create_link(&layer2_path, &tool_target)?;
    }

    let ext_id = format!("skill-{}", skill_name);
    database::upsert_assignment(&ext_id, tool_id, true, "valid")?;
    info!("Skill {} 已为 {} 启用（Layer 2）", skill_name, tool_id);
    Ok(())
}

/// 为工具禁用 skill（移除 Layer 2 symlink + 工具目录 symlink）
pub fn disable_skill_for_tool(skill_name: &str, tool_id: &str) -> Result<(), String> {
    if let Some(tool_skill_dir) = get_tool_skill_dir(tool_id) {
        let tool_target = tool_skill_dir.join(skill_name);
        let _ = crate::linker::remove_link(&tool_target);
    }
    let _ = crate::linker::layer3::cleanup_layer3_on_tool_disable(skill_name, tool_id);
    crate::linker::layer2::unlink_skill_from_layer2(skill_name, tool_id)?;

    let ext_id = format!("skill-{}", skill_name);
    database::upsert_assignment(&ext_id, tool_id, false, "missing")?;
    info!("Skill {} 已为 {} 禁用", skill_name, tool_id);
    Ok(())
}

/// 检查 skill 是否在工具级范围内
pub fn is_skill_in_tool_range(skill_name: &str, tool_id: &str) -> bool {
    let ext_id = format!("skill-{}", skill_name);
    let assignments = crate::database::list_assignments(tool_id);
    assignments.iter().any(|a| a.extension_id == ext_id && a.enabled)
}

/// 为子 Agent 分配 skill（带约束检查，走 Layer 3）
pub fn assign_skill_to_subagent(skill_name: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    if !is_skill_in_tool_range(skill_name, tool_id) {
        return Err(format!("Skill {} 未在 {} 的工具级分配中启用，无法分配给子 Agent", skill_name, tool_id));
    }

    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        "openclaw" => Box::new(OpenClawAdapter),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };

    let has_subagent_dir = adapter.subagent_dir().is_some();

    if has_subagent_dir {
        crate::linker::layer3::link_skill_to_layer3(skill_name, tool_id, sub_agent_id)?;
        if let Some(skill_dir) = adapter.skill_dirs().into_iter().next() {
            let subagent_dir = skill_dir.join("subagents").join(sub_agent_id);
            let _ = std::fs::create_dir_all(&subagent_dir);
            let tool_target = subagent_dir.join(skill_name);
            let layer3_path = crate::linker::layer3::subagent_active_dir(tool_id, sub_agent_id).join(skill_name);
            if tool_target.exists() || tool_target.is_symlink() {
                let _ = crate::linker::remove_link(&tool_target);
            }
            crate::linker::create_link(&layer3_path, &tool_target)?;
        }
    }

    let ext_id = format!("skill-{}", skill_name);
    crate::database::upsert_assignment_with_subagent(&ext_id, tool_id, sub_agent_id, true, if has_subagent_dir { "valid" } else { "ui-only" })?;
    info!("Skill {} 已分配给子 Agent {}（{}）", skill_name, sub_agent_id, if has_subagent_dir { "Layer 3" } else { "UI-only" });
    Ok(())
}
