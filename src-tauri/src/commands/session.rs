// 会话相关命令

use crate::adapter;
use crate::session::SessionsResponse;

#[tauri::command]
pub fn get_all_sessions(app: tauri::AppHandle) -> SessionsResponse {
    let response = adapter::get_all_sessions();
    let has_processing = response.sessions.iter().any(|s| {
        matches!(s.status, crate::session::SessionStatus::Processing
            | crate::session::SessionStatus::Thinking
            | crate::session::SessionStatus::Compacting)
    });
    crate::plugins::system_tray::update_tray_status(
        &app, response.waiting_count, response.total_count, has_processing,
    );
    let preset_count = crate::database::list_presets().len();
    let last_count = crate::database::get_setting("last_preset_count").and_then(|s| s.parse::<usize>().ok()).unwrap_or(usize::MAX);
    if preset_count != last_count {
        let _ = crate::plugins::system_tray::update_tray_with_presets(&app);
        crate::database::set_setting("last_preset_count", &preset_count.to_string());
    }
    response
}

#[tauri::command]
pub fn focus_session(pid: u32) -> Result<(), String> {
    crate::window::focus_terminal_for_pid(pid)
}

#[tauri::command]
pub fn kill_session(pid: u32) -> Result<(), String> {
    use sysinfo::{Pid, Signal};
    if let Some(process) = sysinfo::System::new_all().process(Pid::from_u32(pid)) {
        process.kill_with(Signal::Term);
        Ok(())
    } else {
        Err(format!("进程 {} 不存在", pid))
    }
}
