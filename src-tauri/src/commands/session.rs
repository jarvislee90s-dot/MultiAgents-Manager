// 会话相关命令

use crate::adapter;
use crate::session::SessionsResponse;

#[tauri::command]
pub fn get_all_sessions(app: tauri::AppHandle) -> SessionsResponse {
    let response = adapter::get_all_sessions();
    let has_processing = response.sessions.iter().any(|s| {
        matches!(
            s.status,
            crate::session::SessionStatus::Processing
                | crate::session::SessionStatus::Thinking
                | crate::session::SessionStatus::Compacting
        )
    });
    crate::plugins::system_tray::update_tray_status(
        &app,
        response.waiting_count,
        response.total_count,
        has_processing,
    );
    let preset_count = crate::database::list_presets().len();
    let last_count = crate::database::get_setting("last_preset_count")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    if preset_count != last_count {
        let _ = crate::plugins::system_tray::update_tray_with_presets(&app);
        crate::database::set_setting("last_preset_count", &preset_count.to_string());
    }
    response
}

#[tauri::command]
pub fn focus_session(
    pid: u32,
    session_id: Option<String>,
    agent_type: Option<String>,
    project_name: Option<String>,
) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        let system = sysinfo::System::new_all();
        // marker 与 hook 注入的标题标记一致：MAM:<session_id 前 8 位>
        let marker = session_id
            .as_deref()
            .map(|id| format!("MAM:{}", id.chars().take(8).collect::<String>()));
        match crate::window::win32::resolve_and_focus(
            &system,
            pid,
            marker.as_deref(),
            agent_type.as_deref(),
            project_name.as_deref(),
        ) {
            Ok(crate::window::win32::FocusOutcome::Focused) => {
                Ok(serde_json::json!({ "type": "focused" }))
            }
            Ok(crate::window::win32::FocusOutcome::Ambiguous(windows)) => {
                Ok(serde_json::json!({ "type": "ambiguous", "windows": windows }))
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (session_id, agent_type, project_name);
        crate::window::focus_terminal_for_pid(pid).map(|_| serde_json::json!({ "type": "focused" }))
    }
}

/// 窗口选择器点选后按句柄聚焦
#[tauri::command]
pub fn focus_hwnd(hwnd: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::window::win32::focus_hwnd(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        Err("当前平台不支持".to_string())
    }
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
