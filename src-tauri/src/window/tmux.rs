use std::process::Command;
use super::applescript::execute_applescript;
use super::{iterm, terminal_app};

/// 通过 TTY 匹配并聚焦 tmux pane
pub fn focus_tmux_pane_by_tty(tty: &str) -> Result<(), String> {
    let output = Command::new("tmux")
        .args(["list-panes", "-a", "-F", "#{pane_tty} #{session_name}:#{window_index}.#{pane_index}"])
        .output()
        .map_err(|e| format!("Failed to run tmux: {}", e))?;
    if !output.status.success() {
        return Err("tmux not running or no sessions".to_string());
    }
    let panes = String::from_utf8_lossy(&output.stdout);
    for line in panes.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let pane_tty = parts[0];
            let target = parts[1];
            if pane_tty.contains(tty) || pane_tty.ends_with(tty) {
                let _ = Command::new("tmux").args(["select-window", "-t", target]).output();
                let _ = Command::new("tmux").args(["select-pane", "-t", target]).output();
                focus_tmux_client_terminal()?;
                return Ok(());
            }
        }
    }
    Err("Pane not found in tmux".to_string())
}

fn focus_tmux_client_terminal() -> Result<(), String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{client_tty}"])
        .output().map_err(|e| format!("Failed to get tmux client tty: {}", e))?;
    if !output.status.success() {
        return focus_any_terminal_with_tmux();
    }
    let client_tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if client_tty.is_empty() {
        return focus_any_terminal_with_tmux();
    }
    let tty_name = client_tty.split('/').last().unwrap_or(&client_tty);
    if iterm::focus_iterm_by_tty(tty_name).is_ok() {
        return Ok(());
    }
    if terminal_app::focus_terminal_app_by_tty(tty_name).is_ok() {
        return Ok(());
    }
    focus_any_terminal_with_tmux()
}

fn focus_any_terminal_with_tmux() -> Result<(), String> {
    let script = r#"
        tell application "System Events"
            if exists process "iTerm2" then
                tell application "iTerm2" to activate
                return "found"
            else if exists process "Terminal" then
                tell application "Terminal" to activate
                return "found"
            end if
        end tell
        return "not found"
    "#;
    execute_applescript(script)
}
