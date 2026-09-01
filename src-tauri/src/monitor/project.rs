// 项目名提取与 cwd 形态校验 — 跨工具共享的展示/校验辅助

/// 校验 cwd 字符串形态：Unix 绝对路径（/ 开头）或 Windows 盘符路径（如 C:\... 或 c:/...）
pub(crate) fn is_valid_cwd(cwd: &str) -> bool {
    let bytes = cwd.as_bytes();
    cwd.starts_with('/') || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// 从项目路径提取项目名（跨平台：兼容 / 和 \ 分隔符）
pub(crate) fn project_name_from_path(project_path: &str) -> String {
    project_path
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or("Unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_cwd, project_name_from_path};

    #[test]
    fn accepts_unix_and_windows_absolute() {
        assert!(is_valid_cwd("/Users/x/proj"));
        assert!(is_valid_cwd("C:\\Users\\x"));
        assert!(is_valid_cwd("c:/Users/x"));
    }

    #[test]
    fn rejects_relative_and_empty() {
        assert!(!is_valid_cwd("relative/path"));
        assert!(!is_valid_cwd(""));
        assert!(!is_valid_cwd("C"));
    }

    #[test]
    fn cross_platform_basename() {
        assert_eq!(
            project_name_from_path("C:\\Users\\bunny\\Desktop"),
            "Desktop"
        );
        assert_eq!(project_name_from_path("/Users/x/proj"), "proj");
        assert_eq!(project_name_from_path("/"), "Unknown");
    }
}
