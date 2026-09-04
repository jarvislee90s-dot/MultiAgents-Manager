// 宿主进程存活判定（spec W4）：宿主 = .app 包内的非会话运行时进程。
// 会话进程（codebuddy/Codex 框架进程）不参与判定——APP 崩溃后的孤儿会话进程
// 不得导致误判「宿主还活着」、未读卡不清理

/// exe 路径（已小写）是否为该工具的宿主进程
pub fn is_host_process(exe_lower: &str, tool_id: &str) -> bool {
    match tool_id {
        "workbuddy" => {
            exe_lower.contains("workbuddy.app/") && !exe_lower.contains("codebuddy")
        }
        "codex" => {
            (exe_lower.contains("chatgpt.app/") || exe_lower.contains("codex.app/"))
                && !exe_lower.contains("frameworks") // 主进程在 Contents/MacOS 下
        }
        _ => false,
    }
}

/// 该工具宿主 APP 是否存活（独立 sysinfo 扫描，用未过滤的原始进程集）
pub fn tool_host_alive(tool_id: &str) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::Always),
    );
    system.processes().iter().any(|(_, p)| {
        p.exe()
            .map(|e| is_host_process(&e.to_string_lossy().to_lowercase(), tool_id))
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbuddy_host_is_electron_not_codebuddy() {
        assert!(is_host_process(
            "/applications/workbuddy.app/contents/macos/electron",
            "workbuddy"
        ));
        assert!(is_host_process(
            "/applications/workbuddy.app/contents/frameworks/workbuddy helper.app/contents/macos/workbuddy helper",
            "workbuddy"
        ));
        // 会话进程：不判定为宿主（孤儿防线）
        assert!(!is_host_process(
            "/applications/workbuddy.app/contents/resources/app.asar.unpacked/cli/bin/codebuddy",
            "workbuddy"
        ));
        assert!(!is_host_process("/usr/local/bin/codebuddy", "workbuddy"));
        // 其他 APP 不匹配
        assert!(!is_host_process(
            "/applications/chatgpt.app/contents/macos/chatgpt",
            "workbuddy"
        ));
    }

    #[test]
    fn codex_host_is_chatgpt_main() {
        assert!(is_host_process(
            "/applications/chatgpt.app/contents/macos/chatgpt",
            "codex"
        ));
        // 内嵌 Codex 框架进程 = 会话运行时，不算宿主
        assert!(!is_host_process(
            "/applications/chatgpt.app/contents/frameworks/codex",
            "codex"
        ));
    }
}
