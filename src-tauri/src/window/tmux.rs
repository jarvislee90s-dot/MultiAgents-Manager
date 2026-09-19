use super::applescript::execute_applescript;
use super::{iterm, terminal_app};
use std::process::Command;

/// tmux 全局 pane 清单（`list-panes -a` + 既有格式串）：每行
/// `#{pane_tty} #{session_name}:#{window_index}.#{pane_index}`。
/// tmux 不存在 / 命令失败 → None（调用方降级下一通道）。
/// 注入链（inject/engine `find_tmux_pane`）与聚焦链（本模块）共用同一份
/// Command+格式串（P3 Task 7）与同一份匹配纯函数（Task 9：匹配归一
/// `inject::engine::parse_panes_find`，双份 contains 匹配逻辑就此消灭）
pub(crate) fn list_panes_lines() -> Option<Vec<String>> {
    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_tty} #{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
    )
}

/// 通过 TTY 匹配并聚焦 tmux pane。匹配归一 `inject::engine::parse_panes_find`
/// （Task 9，P2-3）：全路径相等（杜绝 contains/ends_with 的 ttys005 撞 ttys0050
/// 前缀撞号），与注入侧同一份逻辑；入参裸后缀先归一 `/dev/` 全路径。
pub fn focus_tmux_pane_by_tty(tty: &str) -> Result<(), String> {
    let lines = list_panes_lines().ok_or_else(|| "tmux not running or no sessions".to_string())?;
    let target = crate::inject::engine::parse_panes_find(&lines, &super::normalize_dev_tty(tty))
        .ok_or("Pane not found in tmux")?;
    let _ = Command::new("tmux")
        .args(["select-window", "-t", &target])
        .output();
    let _ = Command::new("tmux")
        .args(["select-pane", "-t", &target])
        .output();
    focus_tmux_client_terminal()?;
    Ok(())
}

fn focus_tmux_client_terminal() -> Result<(), String> {
    let output = Command::new("tmux")
        .args(["display-message", "-p", "#{client_tty}"])
        .output()
        .map_err(|e| format!("Failed to get tmux client tty: {}", e))?;
    if !output.status.success() {
        return focus_any_terminal_with_tmux();
    }
    let client_tty = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if client_tty.is_empty() {
        return focus_any_terminal_with_tmux();
    }
    let tty_name = client_tty.split('/').next_back().unwrap_or(&client_tty);
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
