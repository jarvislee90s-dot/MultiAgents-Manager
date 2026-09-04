#[cfg(target_os = "macos")]
mod applescript;
#[cfg(target_os = "macos")]
pub mod app_activation;
#[cfg(any(target_os = "macos", windows))]
pub mod deep_link;
#[cfg(target_os = "macos")]
mod iterm;
#[cfg(target_os = "macos")]
mod terminal_app;
#[cfg(target_os = "macos")]
mod tmux;
#[cfg(windows)]
pub mod win32;

/// 通过 PID 聚焦对应的终端/应用窗口（macOS TTY 链路 / Linux 不支持）。
/// Windows 不走此入口——commands/session.rs 直接调用 win32 判定链（resolve_and_focus）
#[cfg(not(windows))]
pub fn focus_terminal_for_pid(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // 获取进程的 TTY
        let tty = get_tty_for_pid(pid)?;

        // 依次尝试：tmux → iTerm2 → Terminal.app
        if tmux::focus_tmux_pane_by_tty(&tty).is_ok() {
            return Ok(());
        }
        if iterm::focus_iterm_by_tty(&tty).is_ok() {
            return Ok(());
        }
        terminal_app::focus_terminal_app_by_tty(&tty)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = pid;
        Err("当前平台不支持终端跳转".to_string())
    }
}

/// 通过 ps 命令获取进程的 TTY
#[cfg(target_os = "macos")]
fn get_tty_for_pid(pid: u32) -> Result<String, String> {
    use std::process::Command;
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "tty="])
        .output()
        .map_err(|e| format!("Failed to get TTY: {}", e))?;
    if output.status.success() {
        let tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tty.is_empty() || tty == "??" {
            Err("Process has no TTY (可能是桌面 APP)".to_string())
        } else {
            Ok(tty)
        }
    } else {
        Err("Failed to get TTY for process".to_string())
    }
}

/// APP 激活入口（W2）：优先深度链接直达会话 → pid 提取 bundle → 按工具枚举兜底。
/// 返回 Some(json) 表示跳转成功，None 表示无法激活
#[cfg(target_os = "macos")]
pub fn activate_agent_app(
    pid: u32,
    agent_type: Option<&str>,
    session_id: Option<&str>,
) -> Option<serde_json::Value> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    // 1) 第一顺位：深度链接直达 session（路由格式由 deep_link 模块决定，未探明则跳过）
    if let (Some(agent), Some(sid)) = (agent_type, session_id) {
        if let Some(url) = deep_link::session_url(agent, sid) {
            if deep_link::open_url(&url).is_ok() {
                return Some(serde_json::json!({ "type": "focused" }));
            }
        }
    }

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::Always),
    );

    // 2) pid 仍存活：取其 exe 提取 .app bundle 激活（pid=0/已退出时自然跳过）
    if pid > 0 {
        if let Some(proc) = system.process(sysinfo::Pid::from_u32(pid)) {
            if let Some(exe) = proc.exe() {
                if let Some(bundle) = app_activation::app_bundle_from_exe(&exe.to_string_lossy()) {
                    if app_activation::activate_app_bundle(&bundle).is_ok() {
                        return Some(serde_json::json!({ "type": "focused" }));
                    }
                }
            }
        }
    }

    // 3) pid 失效兜底：枚举该工具任一 App 形态进程，激活其宿主 bundle
    //    （自洽保证：未读卡存在 ⇒ 宿主进程必存活 ⇒ 此步必有目标）
    // pid 失效兜底要求明确的工具归属（spec W2「按工具降级」）；无工具信息则放弃兜底
    let target = agent_type.map(|a| a.to_lowercase())?;
    for proc in system.processes().values() {
        let Some(exe) = proc.exe() else { continue };
        let Some(bundle) = app_activation::app_bundle_from_exe(&exe.to_string_lossy()) else {
            continue;
        };
        if !app_activation::bundle_matches_agent_pub(&bundle.to_lowercase(), &target) {
            continue;
        }
        if app_activation::activate_app_bundle(&bundle).is_ok() {
            return Some(serde_json::json!({ "type": "focused" }));
        }
    }
    None
}
