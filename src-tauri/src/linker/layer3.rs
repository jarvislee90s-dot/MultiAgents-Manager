// Layer 3：子 Agent 级激活目录管理
// ~/.mam/active/<tool>/<subagent>/ 存放子 Agent 已启用的 skill 链接
// 仅 Hermes 和 OpenCode 等支持子 Agent 独立 skill 目录的工具有此层
// Claude Code 和 Codex CLI 不支持子 Agent 独立目录，此层对其为"仅 UI 记录"

use std::path::PathBuf;

/// 获取指定工具+子 Agent 的 Layer 3 目录
pub fn subagent_active_dir(tool_id: &str, sub_agent_id: &str) -> PathBuf {
    super::layer2::tool_active_dir(tool_id).join(sub_agent_id)
}

/// 确保 Layer 3 目录存在
pub fn ensure_subagent_active_dir(tool_id: &str, sub_agent_id: &str) -> PathBuf {
    let dir = subagent_active_dir(tool_id, sub_agent_id);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 创建 Layer 3 symlink：从 Layer 1 源文件 → Layer 3 子 Agent 目录
/// 约束：Layer 3 只能链接 Layer 2 中已存在的 skill（工具级范围的子集）
pub fn link_skill_to_layer3(skill_name: &str, tool_id: &str, sub_agent_id: &str) -> Result<PathBuf, String> {
    // 检查工具级是否已启用
    let layer2_skills = super::layer2::list_layer2_skills(tool_id);
    if !layer2_skills.contains(&skill_name.to_string()) {
        return Err(format!(
            "Skill {} 未在 {} 的工具级分配中启用，无法分配给子 Agent {}",
            skill_name, tool_id, sub_agent_id
        ));
    }

    let repo = super::ensure_repo_dir();
    let source = repo.join(skill_name);
    let layer3_dir = ensure_subagent_active_dir(tool_id, sub_agent_id);
    let target = layer3_dir.join(skill_name);
    super::create_link(&source, &target)?;
    Ok(target)
}

/// 从 Layer 3 移除 skill 链接
pub fn unlink_skill_from_layer3(skill_name: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    let target = subagent_active_dir(tool_id, sub_agent_id).join(skill_name);
    super::remove_link(&target)
}

/// 列出子 Agent 在 Layer 3 中已启用的 skill
pub fn list_layer3_skills(tool_id: &str, sub_agent_id: &str) -> Vec<String> {
    let dir = subagent_active_dir(tool_id, sub_agent_id);
    let mut skills = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() || path.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    skills.push(name.to_string());
                }
            }
        }
    }
    skills.sort();
    skills
}

/// 当工具级禁用 skill 时，自动从所有子 Agent 中移除
pub fn cleanup_layer3_on_tool_disable(skill_name: &str, tool_id: &str) -> Result<(), String> {
    let tool_dir = super::layer2::tool_active_dir(tool_id);
    if !tool_dir.exists() {
        return Ok(());
    }
    let subagents: Vec<String> = std::fs::read_dir(&tool_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.is_dir() && !path.is_symlink() {
                e.file_name().to_str().map(String::from)
            } else {
                None
            }
        })
        .collect();

    for sub in &subagents {
        let target = subagent_active_dir(tool_id, sub).join(skill_name);
        if target.exists() || target.is_symlink() {
            let _ = super::remove_link(&target);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_active_dir() {
        let dir = subagent_active_dir("opencode", "researcher");
        assert!(dir.to_string_lossy().contains(".mam/active/opencode/researcher"));
    }
}
