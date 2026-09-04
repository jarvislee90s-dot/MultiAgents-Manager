// 深度链接跳转（W2 第一顺位）：session 级直达依赖 URL scheme 路由格式，
// Task 4 已于 2026-09-04 探测探明 workbuddy / codex 两套路由（见 session_url 文档），
// 其余未探明工具返回 None，走 APP 级保底
//
// P1-2（A+B 组合）：派发前校验 handler 存在性 + 派发后前台验证，杜绝「spawn 成功
// → 误标已读」——URL 协议派发是异步无回执的 OS 交互，spawn 成功 ≠ 存在 handler 且
// 完成路由（实测 codex:// 仅有协议标记、无 handler）
#[cfg(target_os = "macos")]
fn open_url_macos(url: &str) -> Result<(), String> {
    // open 在无 handler 时会报错退出（非零退出码）→ 视为派发失败（P1-2 A 的 macOS 侧）
    let output = std::process::Command::new("open")
        .arg(url)
        .output()
        .map_err(|e| format!("open url failed: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "open url 退出码非零（可能无 handler）：{}",
            output.status
        ))
    }
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

/// Windows 注册表查询 scheme 的 shell\open\command 默认值（可注入闭包的纯判定核心）。
/// 依次查 HKCU\Software\Classes\<scheme> 与 HKCR\<scheme>，任一存在非空默认值 → true。
/// 实测语义（2026-09-04，附录 A）：workbuddy 两处均有 handler、codex 两处均无
/// （仅 URL Protocol 标记，无 shell\open\command）→ codex 在无 handler 机器上被天然门控
#[cfg(windows)]
fn scheme_handler_exists_in<F>(scheme: &str, read_default: &F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    ["HKCU\\Software\\Classes", "HKCR"]
        .iter()
        .any(|root| {
            let key = format!(r"{root}\{}\shell\open\command", scheme);
            read_default(&key)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false)
        })
}

/// Windows 查注册表默认值（HKCU\Software\Classes\<scheme>\shell\open\command 与
/// HKCR\<scheme>\shell\open\command），任一非空 → handler 存在。注册表打开失败视为无 handler
#[cfg(windows)]
pub fn scheme_handler_exists(scheme: &str) -> bool {
    scheme_handler_exists_in(scheme, &|key| {
        use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER};
        use winreg::RegKey;
        // 先试 HKCU（用户级覆盖），再试 HKCR
        let (hive, subkey) = key
            .strip_prefix("HKCU\\Software\\Classes\\")
            .map(|rest| (HKEY_CURRENT_USER, rest))
            .or_else(|| key.strip_prefix("HKCR\\").map(|rest| (HKEY_CLASSES_ROOT, rest)))?;
        let root = RegKey::predef(hive);
        root.open_subkey(subkey)
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
    })
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

    // ---- P1-2 A：派发前 handler 校验（纯函数注入闭包，Windows） ----

    #[cfg(windows)]
    mod scheme_handler_tests {
        use super::super::scheme_handler_exists_in;

        /// 构造注册表模拟：key → 默认值
        fn reg(
            scheme: &str,
            hkcu: Option<&str>,
            hkcr: Option<&str>,
        ) -> impl Fn(&str) -> Option<String> {
            let hkcu_key = format!(r"HKCU\Software\Classes\{scheme}\shell\open\command");
            let hkcr_key = format!(r"HKCR\{scheme}\shell\open\command");
            let hkcu = hkcu.map(|v| v.to_string());
            let hkcr = hkcr.map(|v| v.to_string());
            move |key: &str| {
                if key == hkcu_key {
                    hkcu.clone()
                } else if key == hkcr_key {
                    hkcr.clone()
                } else {
                    None
                }
            }
        }

        #[test]
        fn handler_in_either_hive_is_found() {
            // 实测（附录 A）：workbuddy 在 HKCR/HKCU 均有 handler → true
            let both = reg(
                "workbuddy",
                Some(r#""D:\Program Files\WorkBuddy\WorkBuddy.exe" "%1""#),
                Some(r#""D:\Program Files\WorkBuddy\WorkBuddy.exe" "%1""#),
            );
            assert!(scheme_handler_exists_in("workbuddy", &both));
            // 仅 HKCU 有 → true（用户级覆盖）
            let only_hkcu = reg("workbuddy", Some("\"x\" \"%1\""), None);
            assert!(scheme_handler_exists_in("workbuddy", &only_hkcu));
            // 仅 HKCR 有 → true
            let only_hkcr = reg("workbuddy", None, Some("\"x\" \"%1\""));
            assert!(scheme_handler_exists_in("workbuddy", &only_hkcr));
        }

        #[test]
        fn no_handler_in_any_hive_is_not_found() {
            // 实测（附录 A）：codex 两处均无 handler（仅 URL Protocol 标记）→ false
            let none = reg("codex", None, None);
            assert!(!scheme_handler_exists_in("codex", &none));
        }

        #[test]
        fn empty_command_value_counts_as_no_handler() {
            // 命令默认值为空串 → 视为无 handler（防御：注册表值存在但空）
            let empty = reg("workbuddy", Some(""), Some(""));
            assert!(!scheme_handler_exists_in("workbuddy", &empty));
        }

        #[test]
        fn missing_scheme_is_not_found() {
            // 未注册 scheme → 两 key 都不存在 → false
            let missing = reg("ghost", None, None);
            assert!(!scheme_handler_exists_in("ghost", &missing));
        }
    }
}
