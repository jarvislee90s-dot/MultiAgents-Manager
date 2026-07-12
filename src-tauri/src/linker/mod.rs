// LinkerService — symlink (Unix) / Junction (Windows) 管理
// 移植自 skills-manager-jw linker.rs，简化为 MVP 所需功能

use log::debug;
pub mod detector;
pub mod layer2;
pub mod layer3;
use std::fs;
use std::path::{Path, PathBuf};
use fs2::FileExt;


/// 确保全局仓库目录存在，返回路径
pub fn ensure_repo_dir() -> PathBuf {
    let repo = dirs::home_dir().unwrap_or_default().join(".mam").join("skills");
    let _ = fs::create_dir_all(&repo);
    repo
}

/// 获取全局仓库中所有 skill 的名称
pub fn list_repo_skills() -> Vec<String> {
    let repo = ensure_repo_dir();
    let mut skills = Vec::new();
    if let Ok(entries) = fs::read_dir(&repo) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    skills.push(name.to_string());
                }
            }
        }
    }
    skills.sort();
    skills
}

/// 创建链接：source（全局仓库）→ target（工具 skill 目录）
pub fn create_link(source: &Path, target: &Path) -> Result<(), String> {
    // 如果目标已存在，先移除
    if target.exists() || target.is_symlink() {
        remove_link(target)?;
    }

    // 确保父目录存在
    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
            .map_err(|e| format!("创建 symlink 失败: {}", e))?;
    }

    #[cfg(windows)]
    {
        // Windows: 目录用 Junction，文件用 copy
        if source.is_dir() {
            // 使用 cmd 创建 Junction
            let source_str = source.to_string_lossy();
            let target_str = target.to_string_lossy();
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J", &target_str, &source_str])
                .output()
                .map_err(|e| format!("创建 Junction 失败: {}", e))?;
        } else {
            fs::copy(source, target).map_err(|e| format!("复制文件失败: {}", e))?;
        }
    }

    debug!("Link created: {:?} -> {:?}", target, source);
    Ok(())
}

/// 移除链接
pub fn remove_link(target: &Path) -> Result<(), String> {
    if target.is_symlink() {
        fs::remove_file(target).map_err(|e| format!("移除 symlink 失败: {}", e))?;
    } else if target.exists() {
        // 可能是 Junction 或实际目录
        fs::remove_dir_all(target).map_err(|e| format!("移除目录失败: {}", e))?;
    }
    Ok(())
}

/// 将原始目录替换为指向 SSOT 仓库的符号链接
///
/// 安全校验：target 必须存在且不是符号链接（防止重复清理）
/// 操作流程：删除 target 目录 → 创建符号链接 target → source
pub fn replace_with_symlink(source: &Path, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Err(format!("目标路径不存在: {}", target.display()));
    }
    if target.is_symlink() {
        return Err(format!("目标已是符号链接，无需清理: {}", target.display()));
    }
    if !source.exists() {
        return Err(format!("源路径不存在: {}", source.display()));
    }

    // 删除原始目录
    if target.is_dir() {
        std::fs::remove_dir_all(target)
            .map_err(|e| format!("删除原始目录失败: {}", e))?;
    } else {
        std::fs::remove_file(target)
            .map_err(|e| format!("删除原始文件失败: {}", e))?;
    }

    // 创建符号链接
    create_link(source, target)
}

/// 安装 skill 到全局仓库（从源路径复制）
/// 安全检查：验证源路径不在敏感目录内（防止路径穿越）
pub fn install_to_repo(source: &Path, name: &str) -> Result<(), String> {
    // 路径穿越检查：解析源路径，验证不在敏感目录内
    let canonical = source.canonicalize().map_err(|e| format!("路径解析失败: {}", e))?;
    let sensitive_paths = [
        ".ssh", ".gnupg", ".aws", ".kube", ".netrc",
        ".npmrc", ".docker", ".config/gcloud", ".config/gh",
    ];
    let home = dirs::home_dir().unwrap_or_default();
    for sensitive in &sensitive_paths {
        let sensitive_path = home.join(sensitive);
        if canonical.starts_with(&sensitive_path) {
            return Err(format!("安全检查失败：源路径在敏感目录内: {}", sensitive));
        }
    }
    // 验证目标名称不含路径穿越字符
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Skill 名称包含非法字符".to_string());
    }

    let repo = ensure_repo_dir();
    let dest = repo.join(name);

    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("清理旧目录失败: {}", e))?;
    }

    copy_dir_recursive(&canonical, &dest)?;
    Ok(())
}

/// 递归复制目录
/// 原子写入文件（write-to-temp + rename）
pub fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    fs::rename(&temp, path).map_err(|e| format!("重命名失败: {}", e))?;
    Ok(())
}

/// 对配置文件加排他锁后执行原子写入，防止多个 MAM 实例并发写同一配置
pub fn write_config_locked(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .read(true)
        .open(path)
        .map_err(|e| format!("打开配置文件失败: {}", e))?;
    file.lock_exclusive()
        .map_err(|e| format!("获取文件锁失败: {}", e))?;
    let result = (|| {
        let temp = path.with_extension("tmp");
        fs::write(&temp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
        fs::rename(&temp, path).map_err(|e| format!("重命名失败: {}", e))?;
        Ok(())
    })();
    let _ = file.unlock();
    result
}

pub fn copy_dir_recursive(source: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            fs::copy(&src_path, &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
