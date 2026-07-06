// Layer 2：工具级激活目录管理
// ~/.mam/active/<tool>/ 存放该工具已启用的 skill 链接

use std::path::PathBuf;

/// 获取 Layer 2 基础目录
pub fn active_base_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mam").join("active")
}

/// 获取指定工具的 Layer 2 目录
pub fn tool_active_dir(tool_id: &str) -> PathBuf {
    active_base_dir().join(tool_id)
}

/// 确保 Layer 2 目录存在
pub fn ensure_tool_active_dir(tool_id: &str) -> PathBuf {
    let dir = tool_active_dir(tool_id);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// 创建 Layer 2 symlink：从 Layer 1 源文件 → Layer 2 工具目录
pub fn link_skill_to_layer2(skill_name: &str, tool_id: &str) -> Result<PathBuf, String> {
    let repo = super::ensure_repo_dir();
    let source = repo.join(skill_name);
    if !source.exists() {
        return Err(format!("Skill 不在全局仓库: {}", skill_name));
    }
    let layer2_dir = ensure_tool_active_dir(tool_id);
    let target = layer2_dir.join(skill_name);
    super::create_link(&source, &target)?;
    Ok(target)
}

/// 从 Layer 2 移除 skill 链接
pub fn unlink_skill_from_layer2(skill_name: &str, tool_id: &str) -> Result<(), String> {
    let target = tool_active_dir(tool_id).join(skill_name);
    super::remove_link(&target)
}

/// 列出工具在 Layer 2 中已启用的 skill
pub fn list_layer2_skills(tool_id: &str) -> Vec<String> {
    let dir = tool_active_dir(tool_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_active_dir() {
        let dir = tool_active_dir("claude");
        assert!(dir.to_string_lossy().contains(".mam/active/claude"));
    }

    #[test]
    fn test_ensure_tool_active_dir() {
        let dir = ensure_tool_active_dir("test_tool");
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
