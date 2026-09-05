// 宿主进程存活判定（spec W4）：宿主 = .app 包内的非会话运行时进程。
// 会话进程（codebuddy/Codex 框架进程）不参与判定——APP 崩溃后的孤儿会话进程
// 不得导致误判「宿主还活着」、未读卡不清理

/// exe 路径（已小写）是否为该工具的宿主进程
/// Windows 兼容（spec §7）：路径无 `.app` 包结构，分隔符归一化后按
/// 「可执行文件名」（最后一段）兜底判定（WorkBuddy.exe / chatgpt.exe）
pub fn is_host_process(exe_lower: &str, tool_id: &str) -> bool {
    // 分隔符归一化：Windows 路径（C:\...\WorkBuddy.exe）与 POSIX 统一按 / 分段
    let normalized = exe_lower.replace('\\', "/");
    // 最后一段 = 可执行文件名（两种平台通用；POSIX 下等价 basename）
    let file_name = normalized.rsplit('/').next().unwrap_or_default();
    match tool_id {
        "workbuddy" => {
            !exe_lower.contains("codebuddy") // 会话进程不判定为宿主（孤儿防线）
                && (normalized.contains("workbuddy.app/")
                    // Windows 安装形态（含 MSIX）：可执行名以 workbuddy 开头即宿主，
                    // 同时覆盖 macOS Helper 无包路径的场景
                    || file_name.starts_with("workbuddy"))
        }
        "codex" => {
            // 防御性决策：codex.exe 单独不构成宿主——Codex CLI / rollout 框架进程
            // 也可能叫 codex.exe，孤儿会话进程不得误判「宿主还活着」；仅 chatgpt(.exe) 算
            let chatgpt_exe = file_name == "chatgpt"
                || (file_name.starts_with("chatgpt") && file_name.ends_with(".exe"));
            chatgpt_exe
                || ((normalized.contains("chatgpt.app/") || normalized.contains("codex.app/"))
                    // 内嵌 Codex 框架进程（Contents/Frameworks/）= 会话运行时，不算宿主
                    && !normalized.contains("frameworks/"))
        }
        _ => false,
    }
}

/// 同 tool_host_alive，但复用外部传入的 System 快照（review F2：
/// adapter 的 SHARED_SYSTEM 每轮已带 exe 全量刷新，活跃卡过滤不得另起扫描）
pub fn tool_host_alive_in(system: &sysinfo::System, tool_id: &str) -> bool {
    system.processes().iter().any(|(_, p)| {
        p.exe()
            .map(|e| is_host_process(&e.to_string_lossy().to_lowercase(), tool_id))
            .unwrap_or(false)
    })
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
    tool_host_alive_in(&system, tool_id)
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

    // ---- Windows 形态（spec §7 跨平台硬性）：无 .app 包，靠可执行文件名兜底 ----

    #[test]
    fn workbuddy_windows_exe_is_host() {
        assert!(is_host_process(
            "c:\\users\\u\\appdata\\local\\programs\\workbuddy\\workbuddy.exe",
            "workbuddy"
        ));
    }

    #[test]
    fn codex_windows_chatgpt_exe_is_host() {
        assert!(is_host_process(
            "c:\\users\\u\\appdata\\local\\programs\\chatgpt\\chatgpt.exe",
            "codex"
        ));
    }

    #[test]
    fn codebuddy_windows_exe_is_not_workbuddy_host() {
        // Windows 会话进程 codebuddy.exe 同样不得判定为宿主（孤儿防线）
        assert!(!is_host_process(
            "c:\\users\\u\\appdata\\local\\programs\\codebuddy\\codebuddy.exe",
            "workbuddy"
        ));
    }

    /// review F2：tool_host_alive_in 复用外部快照；空快照（无进程）→ 不存活
    #[test]
    fn empty_system_snapshot_has_no_host() {
        let system = sysinfo::System::new();
        assert!(!tool_host_alive_in(&system, "workbuddy"));
        assert!(!tool_host_alive_in(&system, "codex"));
    }

    #[test]
    fn codex_exe_filename_alone_is_not_codex_host() {
        // 防御性决策：codex.exe 单独不构成宿主——Codex CLI / rollout 框架进程
        // 也可能叫 codex.exe，孤儿会话进程不得误判「宿主还活着」；仅 chatgpt(.exe) 算
        assert!(!is_host_process(
            "c:\\users\\u\\appdata\\local\\programs\\chatgpt\\codex.exe",
            "codex"
        ));
    }
}
