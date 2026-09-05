// macOS APP 激活：从进程可执行路径提取 .app bundle 并激活（W2 保底路径）

/// 从可执行路径提取 .app bundle 根目录（取最内层 .app 段）
/// "/Applications/WorkBuddy.app/Contents/.../codebuddy" → "/Applications/WorkBuddy.app"
pub fn app_bundle_from_exe(exe: &str) -> Option<String> {
    let normalized = exe.replace('\\', "/");
    let idx = normalized.rfind(".app/")?;
    Some(normalized[..idx + ".app".len()].to_string())
}

/// bundle 路径（小写）是否属于该工具的宿主 APP（W2 pid 失效兜底的匹配规则）
///
/// 已知局限（暂不收严，issue #34 P2 侧注）：按 bundle 名后缀匹配而非完整路径或
/// bundle id，理论上同名后缀的无关应用（如 NotWorkBuddy.app 之于 workbuddy）
/// 会被误命中。当前仅 codex/ChatGPT 为 CLI+APP 双形态、WorkBuddy 为纯 APP 形态，
/// 误命中风险可接受；接入更多双形态/APP 形态工具时再评估收严口径。
fn bundle_matches_agent(bundle_lower: &str, agent_type: &str) -> bool {
    match agent_type {
        "codex" => bundle_lower.ends_with("chatgpt.app") || bundle_lower.ends_with("codex.app"),
        "workbuddy" => bundle_lower.ends_with("workbuddy.app"),
        // 其他工具暂无 APP 形态；新增 APP 类工具时在此补一行
        _ => false,
    }
}

/// 激活 APP（AppleScript，bundle 路径精确指定，避免同名歧义）
pub fn activate_app_bundle(bundle: &str) -> Result<(), String> {
    let script = format!("activate application \"{}\"", bundle.replace('\"', "\\\""));
    super::applescript::execute_applescript(&script)
}

/// 供 Task 3 使用的、按工具匹配 bundle 的公开包装（测试经此覆盖）
pub fn bundle_matches_agent_pub(bundle_lower: &str, agent_type: &str) -> bool {
    bundle_matches_agent(bundle_lower, agent_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_chatgpt_bundle_from_nested_codex() {
        assert_eq!(
            app_bundle_from_exe(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex.framework/Versions/A/Codex"
            )
            .as_deref(),
            Some("/Applications/ChatGPT.app")
        );
    }

    #[test]
    fn extracts_workbuddy_bundle() {
        assert_eq!(
            app_bundle_from_exe(
                "/Applications/WorkBuddy.app/Contents/Resources/app.asar.unpacked/cli/bin/codebuddy"
            )
            .as_deref(),
            Some("/Applications/WorkBuddy.app")
        );
    }

    #[test]
    fn returns_none_for_cli_path() {
        assert_eq!(app_bundle_from_exe("/usr/local/bin/codex"), None);
        assert_eq!(app_bundle_from_exe("/opt/homebrew/bin/claude"), None);
    }

    #[test]
    fn takes_last_app_segment_for_nested_apps() {
        // 路径含多个 .app 段时取最内层（离可执行文件最近的）
        assert_eq!(
            app_bundle_from_exe(
                "/Applications/WorkBuddy.app/Contents/Frameworks/Helper.app/Contents/MacOS/Helper"
            )
            .as_deref(),
            Some("/Applications/WorkBuddy.app/Contents/Frameworks/Helper.app")
        );
    }

    #[test]
    fn bundle_matches_agent_rules() {
        assert!(bundle_matches_agent("/applications/chatgpt.app", "codex"));
        assert!(bundle_matches_agent("/applications/codex.app", "codex"));
        assert!(!bundle_matches_agent(
            "/applications/workbuddy.app",
            "codex"
        ));
        assert!(bundle_matches_agent(
            "/applications/workbuddy.app",
            "workbuddy"
        ));
        assert!(!bundle_matches_agent("/applications/chatgpt.app", "claude"));
    }
}
