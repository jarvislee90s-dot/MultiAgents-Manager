mod applescript;
mod iterm;
mod terminal_app;
mod tmux;

/// 终端窗口管理抽象
pub trait WindowManager {
    /// 聚焦到指定 PID 对应的终端窗口
    fn focus(&self, pid: u32) -> Result<(), String>;
}

/// 默认实现：按平台分发
pub struct DefaultWindowManager;

impl WindowManager for DefaultWindowManager {
    fn focus(&self, pid: u32) -> Result<(), String> {
        focus_terminal_for_pid(pid)
    }
}

/// 通过 PID 聚焦对应的终端窗口
pub fn focus_terminal_for_pid(pid: u32) -> Result<(), String> {
    // Wayland 降级检测
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        return Err("Wayland 环境不支持终端跳转".to_string());
    }

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

/// 通过 ps 命令获取进程的 TTY
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
