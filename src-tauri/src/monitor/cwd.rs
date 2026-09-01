// cwd 归一化与等价比较 — 进程 cwd ↔ 会话目录匹配的公共基础设施
// 各工具解析器（claude/codex/opencode/openclaw/kimi）共用，与具体工具协议无关

/// 归一化 cwd 字符串用于"进程 cwd ↔ 会话 cwd"匹配：
/// - 去尾部路径分隔符（Windows 下 sysinfo 返回的 cwd 带尾部反斜杠，如 "E:\x\y\"）
/// - 统一分隔符为正斜杠（OpenCode db 存正斜杠、sysinfo cwd 存反斜杠，两侧同规归一化后可比）
/// - Windows 下整体转小写（盘符/路径大小写随用户 cd 写法不同，文件系统实际不区分；
///   Unix 文件系统大小写敏感，保持原样）
/// - 根路径（"/"）归一化为空串，调用方按无有效 cwd 处理
pub(crate) fn normalize_cwd_for_match(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches(['/', '\\']).replace('\\', "/");
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// 判断两个 cwd 字符串归一化后是否指向同一目录（用于进程 cwd ↔ 会话 directory 匹配）
pub(crate) fn cwd_equivalent(a: &str, b: &str) -> bool {
    normalize_cwd_for_match(a) == normalize_cwd_for_match(b)
}

#[cfg(test)]
mod tests {
    use super::{cwd_equivalent, normalize_cwd_for_match};

    #[test]
    fn trims_trailing_separators() {
        // Windows 下 sysinfo 返回的 cwd 带尾部反斜杠；分隔符统一为正斜杠
        let expected = if cfg!(windows) { "e:/x/y" } else { "E:/x/y" };
        assert_eq!(normalize_cwd_for_match("E:\\x\\y\\"), expected);
        assert_eq!(normalize_cwd_for_match("E:\\x\\y"), expected);
        // 反斜杠与正斜杠输入等价（OpenCode db 存正斜杠，sysinfo 存反斜杠）
        assert_eq!(
            normalize_cwd_for_match("E:\\x\\y\\"),
            normalize_cwd_for_match("E:/x/y/")
        );
    }

    #[test]
    fn unix_paths_trim_only() {
        let expected = if cfg!(windows) {
            "/users/x/proj"
        } else {
            "/Users/x/proj"
        };
        assert_eq!(normalize_cwd_for_match("/Users/x/proj/"), expected);
        // Unix 路径中的反斜杠（罕见）也被统一为正斜杠，两侧同规不影响相等性
        assert_eq!(normalize_cwd_for_match("/Users/x/proj\\"), expected);
    }

    #[test]
    fn drive_letter_case_normalized_on_windows_only() {
        if cfg!(windows) {
            assert_eq!(
                normalize_cwd_for_match("E:\\X"),
                normalize_cwd_for_match("e:\\x")
            );
        }
    }

    #[test]
    fn root_normalizes_to_empty() {
        // 根路径归一化为空串，调用方按"无有效 cwd"处理（进入 unmatched 分支）
        assert_eq!(normalize_cwd_for_match("/"), "");
    }

    #[test]
    fn separator_direction_and_trailing_are_equivalent() {
        assert!(cwd_equivalent("E:\\LLMproject\\x\\", "E:/LLMproject/x"));
        // 大小写规则随平台（与 case_rules_follow_platform 一致）：Windows 不区分，Unix 区分
        if cfg!(windows) {
            assert!(cwd_equivalent("e:/llmproject/x", "E:\\LLMproject\\x\\"));
        } else {
            assert!(!cwd_equivalent("e:/llmproject/x", "E:\\LLMproject\\x\\"));
        }
    }

    #[test]
    fn case_rules_follow_platform() {
        if cfg!(windows) {
            assert!(cwd_equivalent("E:/X", "e:/x"));
        } else {
            assert!(!cwd_equivalent("/Users/X", "/Users/x"));
        }
    }

    #[test]
    fn different_paths_are_not_equivalent() {
        assert!(!cwd_equivalent("E:/a", "E:/b"));
        assert!(!cwd_equivalent("E:/a", "E:/a/sub"));
        assert!(!cwd_equivalent("", "E:/a"));
    }
}
