// 数据管理命令（2026-09-20 用户要求）：桌面端「数据管理」卡片的数据源。
// 首版只管移动端附件（<项目>/.mam-attachments/<会话>/）——列出各项目占用 +
// 按项目清理。路径全部来自服务端索引（~/.mam/attachments-index.json，上传时
// 服务端写入），**不接受客户端任意路径**（清理目标必须命中索引记录）。

use serde::Serialize;
use std::path::{Path, PathBuf};

/// 单项目的附件占用统计
#[derive(Debug, Clone, Serialize)]
pub struct AttachmentProjectStats {
    /// 项目根（.mam-attachments 所在目录）
    pub project: String,
    pub files: u64,
    pub bytes: u64,
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// 列出含附件的项目（核心可测）：读索引按项目分组，逐项实扫目录算占用；
/// 目录已不存在（用户手动删）的项目仍列出（files=0），便于发现残留索引
fn list_attachment_projects_with(home: &Path) -> Vec<AttachmentProjectStats> {
    let index = crate::remote::attachments::read_index(home);
    let mut projects: Vec<String> = Vec::new();
    for entry in &index {
        if !projects
            .iter()
            .any(|p| p.eq_ignore_ascii_case(&entry.project))
        {
            projects.push(entry.project.clone());
        }
    }
    projects
        .into_iter()
        .map(|project| {
            let (files, bytes) =
                crate::remote::attachments::project_attachment_stats(Path::new(&project));
            AttachmentProjectStats {
                project,
                files,
                bytes,
            }
        })
        .collect()
}

/// 清理某项目的附件目录（核心可测）：整目录删除 + 索引剔除。
/// 安全：project 必须命中索引记录（服务端可信），否则拒绝——杜绝客户端
/// 传任意路径触发 remove_dir_all
fn clean_attachment_project_with(home: &Path, project: &str) -> Result<(), String> {
    let known = crate::remote::attachments::read_index(home)
        .iter()
        .any(|e| e.project.eq_ignore_ascii_case(project));
    if !known {
        return Err("unknown_project".to_string());
    }
    crate::remote::attachments::clean_project_attachments(Path::new(project))
        .map_err(|e| e.to_string())?;
    crate::remote::attachments::prune_index_for_project(home, project);
    Ok(())
}

/// 列出含附件的项目（桌面「数据管理」卡片数据源）
#[tauri::command]
pub fn list_attachment_projects() -> Vec<AttachmentProjectStats> {
    let home = home_dir();
    list_attachment_projects_with(&home)
}

/// 清理某项目的附件目录（整目录删除；索引同步剔除）
#[tauri::command]
pub fn clean_attachment_project(project: String) -> Result<(), String> {
    let home = home_dir();
    clean_attachment_project_with(&home, &project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tempdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mam-datamgmt-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn seed_attachment(home: &Path, project: &Path, session: &str, name: &str, bytes: &[u8]) {
        let _ = crate::remote::attachments::write_attachment(project, session, name, bytes);
        crate::remote::attachments::append_index_entry(
            home,
            &crate::remote::attachments::AttachmentIndexEntry {
                project: project.to_string_lossy().into_owned(),
                session: session.into(),
                tool: "claude".into(),
                name: crate::remote::attachments::sanitize_file_name(name),
                path: String::new(),
                size: bytes.len() as u64,
                ts: 1000,
            },
        );
    }

    #[test]
    fn list_groups_by_project_with_real_dir_sizes() {
        let home = tempdir();
        let proj_a = tempdir();
        let proj_b = tempdir();
        seed_attachment(&home, &proj_a, "s1", "a.png", b"12345");
        seed_attachment(&home, &proj_a, "s1", "b.txt", b"12");
        seed_attachment(&home, &proj_b, "s2", "c.md", b"123");
        let out = list_attachment_projects_with(&home);
        assert_eq!(out.len(), 2);
        let a = out.iter().find(|s| s.files == 2).expect("proj_a");
        assert_eq!(a.bytes, 7);
        let b = out.iter().find(|s| s.files == 1).expect("proj_b");
        assert_eq!(b.bytes, 3);
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&proj_a).ok();
        std::fs::remove_dir_all(&proj_b).ok();
    }

    #[test]
    fn clean_removes_dir_and_prunes_index() {
        let home = tempdir();
        let proj = tempdir();
        seed_attachment(&home, &proj, "s1", "a.png", b"abc");
        assert!(proj.join(".mam-attachments").exists());
        clean_attachment_project_with(&home, proj.to_string_lossy().as_ref()).unwrap();
        // 目录删除（含宿主空壳 .mam-attachments 移除）+ 索引剔空
        assert!(!proj.join(".mam-attachments").exists());
        assert!(crate::remote::attachments::read_index(&home).is_empty());
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&proj).ok();
    }

    #[test]
    fn clean_unknown_project_rejected() {
        let home = tempdir();
        let err = clean_attachment_project_with(&home, "E:/not-in-index").unwrap_err();
        assert_eq!(err, "unknown_project");
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn clean_is_idempotent_after_manual_delete() {
        let home = tempdir();
        let proj = tempdir();
        seed_attachment(&home, &proj, "s1", "a.png", b"abc");
        // 用户手动删掉项目 → 索引残留；清理仍应成功（目录不存在 = 幂等）并剔索引
        std::fs::remove_dir_all(proj.join(".mam-attachments")).unwrap();
        clean_attachment_project_with(&home, proj.to_string_lossy().as_ref()).unwrap();
        assert!(crate::remote::attachments::read_index(&home).is_empty());
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&proj).ok();
    }

    #[test]
    fn stats_count_files_only_at_project_root_path() {
        // 防御：统计只走 <project>/.mam-attachments，不误扫项目其他目录
        let home = tempdir();
        let proj = tempdir();
        std::fs::write(proj.join("unrelated.txt"), b"zz").unwrap();
        seed_attachment(&home, &proj, "s1", "a.png", b"1");
        let (files, _) = crate::remote::attachments::project_attachment_stats(&proj);
        assert_eq!(files, 1);
        std::fs::remove_dir_all(&home).ok();
        std::fs::remove_dir_all(&proj).ok();
    }
}
