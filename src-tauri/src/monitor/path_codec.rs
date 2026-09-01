// Claude projects 目录名编解码 — Claude Code 专用的路径 <-> 目录名转换
// 仅 Claude 解析器使用；其他工具有各自的会话定位方式（Codex 递归 rollout、
// OpenCode SQLite、OpenClaw JSON、Kimi session_index），不依赖此模块

/// 将路径转换为 Claude projects 目录名
/// Claude Code 规则：路径中每个非 ASCII 字母数字字符（分隔符、盘符冒号、点、空格、非 ASCII）
/// 逐字符替换为 '-'
/// Unix: /Users/x/proj -> -Users-x-proj；Windows: C:\Users\x\proj -> C--Users-x-proj
pub(crate) fn convert_path_to_dir_name(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// 将 Claude projects 目录名还原为路径
pub(crate) fn convert_dir_name_to_path(dir_name: &str) -> String {
    // Windows 盘符目录名（如 C--Users-bunny）→ C:\Users\bunny
    // 注意：目录名中 '.' 与 '-' 不可区分，还原结果仅作兜底显示，精确 cwd 以 jsonl 内记录为准
    // 实现说明：rest 由 skip(2) 得到（形如 "-Users-bunny"），首字符 '-' 替换为 '\\' 后即路径分隔符，
    // 因此格式串只需 "{}:{}"，若写成 "{}:\\{}" 会在盘符后产生双反斜杠
    let mut chars = dir_name.chars();
    if let (Some(first), Some(second)) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() && second == '-' {
            let rest: String = dir_name.chars().skip(2).collect();
            return format!("{}:{}", first, rest.replace('-', "\\"));
        }
    }
    let name = dir_name.strip_prefix('-').unwrap_or(dir_name);
    let parts: Vec<&str> = name.split('-').collect();
    if parts.is_empty() {
        return String::new();
    }
    let projects_idx = parts
        .iter()
        .position(|&p| p == "Projects" || p == "UnityProjects");
    if let Some(idx) = projects_idx {
        let path_parts = &parts[..=idx];
        let project_parts = &parts[idx + 1..];
        let mut path = String::from("/");
        path.push_str(&path_parts.join("/"));
        if !project_parts.is_empty() {
            path.push('/');
            let mut segments: Vec<String> = Vec::new();
            let mut current = String::new();
            let mut in_hidden = false;
            for part in project_parts {
                if part.is_empty() {
                    if !current.is_empty() {
                        segments.push(current);
                        current = String::new();
                    }
                    in_hidden = true;
                } else if in_hidden {
                    if current.is_empty() {
                        current = format!(".{}", part);
                    } else {
                        segments.push(current);
                        current = part.to_string();
                    }
                } else {
                    if current.is_empty() {
                        current = part.to_string();
                    } else {
                        current.push('-');
                        current.push_str(part);
                    }
                }
            }
            if !current.is_empty() {
                segments.push(current);
            }
            path.push_str(&segments.join("/"));
        }
        path
    } else {
        format!("/{}", name.replace('-', "/"))
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_dir_name_to_path, convert_path_to_dir_name};

    #[test]
    fn unix_paths_keep_old_behavior() {
        assert_eq!(convert_path_to_dir_name("/Users/x/proj"), "-Users-x-proj");
        assert_eq!(
            convert_path_to_dir_name("/Users/x/.agents/skills"),
            "-Users-x--agents-skills"
        );
    }

    #[test]
    fn windows_paths() {
        assert_eq!(
            convert_path_to_dir_name("C:\\Users\\bunny\\Desktop"),
            "C--Users-bunny-Desktop"
        );
        assert_eq!(
            convert_path_to_dir_name("C:\\Users\\bunny\\.agents\\skills\\extract-report"),
            "C--Users-bunny--agents-skills-extract-report"
        );
        // 非 ASCII 字符逐字符替换为 '-'：分隔符 1 个 + 2 个中文 = 3 个 '-'
        // （注意：计划原文预期 2 个 '-' 与实际逐字符规则不符，已按规则修正为 3 个）
        assert_eq!(
            convert_path_to_dir_name("C:\\Users\\bunny\\Desktop\\桌面"),
            "C--Users-bunny-Desktop---"
        );
    }

    #[test]
    fn windows_drive_letter() {
        assert_eq!(
            convert_dir_name_to_path("C--Users-bunny-Desktop"),
            "C:\\Users\\bunny\\Desktop"
        );
    }

    #[test]
    fn unix_keeps_old_behavior() {
        assert_eq!(convert_dir_name_to_path("-Users-x-proj"), "/Users/x/proj");
    }
}
