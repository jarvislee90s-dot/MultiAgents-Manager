// 深度链接跳转（W2 第一顺位）：session 级直达依赖 URL scheme 路由格式，
// Task 4 已于 2026-09-04 探测探明 workbuddy / codex 两套路由（见 session_url 文档），
// 其余未探明工具返回 None，走 APP 级保底
#[cfg(target_os = "macos")]
fn open_url_macos(url: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("open url failed: {}", e))
}

#[cfg(windows)]
fn open_url_windows(url: &str) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("start url failed: {}", e))
}

/// 打开外部 URL（跨平台）
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return open_url_macos(url);
    #[cfg(windows)]
    return open_url_windows(url);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = url;
        Err("当前平台不支持 URL 打开".to_string())
    }
}

/// 该工具的「直达具体会话」深度链接；未探明的工具返回 None 走 APP 级保底。
///
/// 路由格式来源（探测日期 2026-09-04，`strings <App>.app/Contents/Resources/app.asar` 证据）：
/// - WorkBuddy：`workbuddy://chat/<sessionId>` —— app.asar 中存在
///   `windowManager.handleOpenUrl(\`workbuddy://chat/${encodeURIComponent(sessionId)}\`)`
///   及文档字面量 `workbuddy://chat/{sessionId} deep link`（sessionId 为 UUID 形态，
///   不含保留字符，无需 percent-encode，保持简单确定）。
/// - Codex：`codex://threads/<threadId>` —— ChatGPT.app 的 app.asar 中存在
///   copy-link 处理器模板 `codex://threads/${i}`（解构出 `{threadId:i}`）。
///
/// 注意：Codex 的 threadId 与 MAM 使用的 rollout-session UUID 大概率同源，
/// 但仅能通过 GUI 点击实测确认；若实测直达失败（APP 打开但停留在原界面），
/// 按 plan Step 3 回退规则将 codex 分支改回 None。
pub fn session_url(agent_type: &str, session_id: &str) -> Option<String> {
    match agent_type {
        // 实测（2026-09-04，WorkBuddy app.asar）：workbuddy://chat/<sessionId>
        "workbuddy" => Some(format!("workbuddy://chat/{}", session_id)),
        // 实测（2026-09-04，ChatGPT.app app.asar）：codex://threads/<threadId>
        "codex" => Some(format!("codex://threads/{}", session_id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbuddy_session_url_format() {
        assert_eq!(
            session_url("workbuddy", "4b36e102-8fdc-4b47-9a92-0d1ec7f3a111"),
            Some("workbuddy://chat/4b36e102-8fdc-4b47-9a92-0d1ec7f3a111".to_string())
        );
    }

    #[test]
    fn codex_session_url_format() {
        assert_eq!(
            session_url("codex", "0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f"),
            Some("codex://threads/0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f".to_string())
        );
    }

    #[test]
    fn unknown_tool_returns_none() {
        assert_eq!(session_url("claude", "abc-123"), None);
        assert_eq!(session_url("", "abc-123"), None);
    }
}
