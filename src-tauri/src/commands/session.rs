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

/// 跳转成功 → 标记该会话已读（仅删除对应未读行；同工具其他未读卡保留，spec W4）
fn mark_read_on_jump(session_id: &Option<String>, agent_type: &Option<String>) {
    if let (Some(sid), Some(agent)) = (session_id, agent_type) {
        crate::database::dao::unread::delete(&agent.to_lowercase(), sid);
    }
}

#[tauri::command]
pub fn focus_session(
    pid: u32,
    session_id: Option<String>,
    agent_type: Option<String>,
    project_name: Option<String>,
    last_message: Option<String>,
    form: Option<String>,
    unread: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        let system = sysinfo::System::new_all();
        // unread 参数（未读标记）在 Windows 深度链接路径暂不消费（已读回标统一在
        // 跳转成功分支执行）；保留签名以与前端 JumpTarget.unread 对齐
        let _ = unread;
        // P1-1（Windows）：App 形态且带 sessionId 且工具为 workbuddy/codex 时，
        // 深度链接为第一顺位（session 级直达）：handler 校验通过 → 派发 → 前台验证成功
        // → 标已读返回；任一步失败无缝落回现有 resolve_and_focus → reactivate_tool_app 链路。
        // codex 由 handler 校验天然门控（无 handler 机器自动跳过，实测语义见 P1-2）
        if form.as_deref() == Some("app") {
            if let (Some(sid), Some(agent)) = (session_id.as_deref(), agent_type.as_deref()) {
                if matches!(agent, "workbuddy" | "codex")
                    && crate::window::deep_link::scheme_handler_exists(agent)
                {
                    if let Some(url) = crate::window::deep_link::session_url(agent, sid) {
                        if crate::window::deep_link::open_url(&url).is_ok()
                            && crate::window::win32::verify_foreground_tool(
                                &system,
                                agent,
                                2_000,
                                250,
                            )
                        {
                            mark_read_on_jump(&session_id, &agent_type);
                            return Ok(serde_json::json!({
                                "type": "focused",
                                "via": "deep-link"
                            }));
                        }
                    }
                }
            }
        }
        // marker 与 hook 注入的标题标记一致：MAM:<session_id 前 8 位>
        let marker = session_id
            .as_deref()
            .map(|id| format!("MAM:{}", id.chars().take(8).collect::<String>()));
        // 面板反推：当前所有运行会话的 (工具id, 项目名)，用于排除其他工具的终端窗口
        // （codex 终端标题=项目名，无 "codex" 关键词可静态认领）。进程扫描即可，无文件解析开销
        let running_projects = running_projects_from_processes(&system);
        match crate::window::win32::resolve_and_focus(
            &system,
            pid,
            marker.as_deref(),
            agent_type.as_deref(),
            project_name.as_deref(),
            last_message.as_deref(),
            &running_projects,
        ) {
            Ok(crate::window::win32::FocusOutcome::Focused) => {
                mark_read_on_jump(&session_id, &agent_type);
                Ok(serde_json::json!({ "type": "focused" }))
            }
            Ok(crate::window::win32::FocusOutcome::Ambiguous(windows)) => {
                Ok(serde_json::json!({ "type": "ambiguous", "windows": windows }))
            }
            Err(e) => {
                // pid 失效兜底（W2）：pid 已死时按工具激活宿主 APP 窗口
                let pid_dead = system.process(sysinfo::Pid::from_u32(pid)).is_none();
                if pid_dead
                    && crate::window::win32::reactivate_tool_app(&system, agent_type.as_deref())
                        .is_ok()
                {
                    mark_read_on_jump(&session_id, &agent_type);
                    Ok(serde_json::json!({ "type": "focused" }))
                } else {
                    Err(e)
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (project_name, last_message, form, unread);
        // CLI 形态：TTY 链路（tmux/iTerm2/Terminal.app）
        if crate::window::focus_terminal_for_pid(pid).is_ok() {
            mark_read_on_jump(&session_id, &agent_type);
            return Ok(serde_json::json!({ "type": "focused", "via": "tty" }));
        }
        // APP 形态 / pid 失效兜底：深度链接 → bundle 激活 → 按工具枚举（W2）。
        // via=app-fallback：CLI 会话 TTY 聚焦失败走到这里的 UX 提示依据（review M3）
        #[cfg(target_os = "macos")]
        if let Some(mut out) =
            crate::window::activate_agent_app(pid, agent_type.as_deref(), session_id.as_deref())
        {
            mark_read_on_jump(&session_id, &agent_type);
            out["via"] = serde_json::Value::String("app-fallback".into());
            return Ok(out);
        }
        // 非 macOS 桌面平台无 APP 激活链路，防未使用告警
        #[cfg(not(target_os = "macos"))]
        let _ = (agent_type, session_id);
        Err(format!(
            "无法聚焦目标（pid={}）：进程无 TTY 且未找到宿主 APP",
            pid
        ))
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

/// 从进程快照收集运行会话的 (工具id, 项目目录名)——仅进程扫描，无文件解析开销
#[cfg(windows)]
fn running_projects_from_processes(system: &sysinfo::System) -> Vec<(String, String)> {
    use crate::monitor::process as monitor_process;
    let mut v = Vec::new();
    for (agent, procs) in [
        ("claude", monitor_process::find_claude_processes(system)),
        ("codex", monitor_process::find_codex_processes(system)),
        ("opencode", monitor_process::find_opencode_processes(system)),
        ("openclaw", monitor_process::find_openclaw_processes(system)),
        ("kimi", monitor_process::find_kimi_processes(system)),
    ] {
        for p in procs {
            if let Some(name) = p
                .cwd
                .as_ref()
                .and_then(|c| c.file_name())
                .map(|n| n.to_string_lossy().to_string())
            {
                if !name.is_empty() {
                    v.push((agent.to_string(), name));
                }
            }
        }
    }
    v
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
