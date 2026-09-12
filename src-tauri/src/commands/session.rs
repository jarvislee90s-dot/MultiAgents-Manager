// 会话相关命令

use crate::adapter;
use crate::session::SessionsResponse;
use tauri::Emitter;

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

/// 跳转成功 → 标记该会话已读（仅删除对应未读行；同工具其他未读卡保留，spec W4）。
/// T1：删行后广播 session-read，宠物等辅助窗口据此同步已读置位（卡片行为与看板一致）
fn mark_read_on_jump(
    app: &tauri::AppHandle,
    session_id: &Option<String>,
    agent_type: &Option<String>,
) {
    if let (Some(sid), Some(agent)) = (session_id, agent_type) {
        crate::database::dao::unread::delete(&agent.to_lowercase(), sid);
        // issue #35-1：已读墓碑——缓存失忆（长间隙清缓存 / MAM 重启）后
        // Insert 边沿与补偿据此不再复活已读未读卡
        crate::database::dao::unread::mark_read(&agent.to_lowercase(), sid);
        let _ = app.emit(
            "session-read",
            serde_json::json!({ "agentType": agent, "sessionId": sid }),
        );
    }
}

// 参数即 Tauri IPC 契约（前端 invoke 逐名传参），不宜打包为结构体
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn focus_session(
    app: tauri::AppHandle,
    pid: u32,
    session_id: Option<String>,
    agent_type: Option<String>,
    project_name: Option<String>,
    last_message: Option<String>,
    title: Option<String>,
    form: Option<String>,
    unread: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(windows)]
    {
        // mut：P2-1 验证轮询/兜底前会在原地刷新进程表（快照不再是一次性照片）
        let mut system = sysinfo::System::new_all();
        // unread 参数（未读标记）在 Windows 深度链接路径暂不消费（已读回标统一在
        // 跳转成功分支执行）；保留签名以与前端 JumpTarget.unread 对齐
        let _ = unread;
        // P1-1（Windows）：App 形态且带 sessionId 且工具为 workbuddy/codex 时，
        // 深度链接为第一顺位（session 级直达）：handler 校验通过 → 派发 → 前台验证成功
        // → 标已读返回；任一步失败无缝落回现有 resolve_and_focus → reactivate_tool_app 链路。
        // codex 由 handler 校验天然门控（无 handler 机器自动跳过，实测语义见 P1-2）。
        // zcode 不在此列（2026-09-09 实机验收后移出）：zcode://workspace/open 每次
        // 无条件弹信任确认、落点为新会话 composer（非定位已有会话）、每次拉起一个
        // 转发进程——跳转改为下方 resolve_and_focus 的 pid 单窗口路径直接聚焦唯一
        // 窗口（ZCode 单窗口多标签，卡片携带宿主 pid，窗口零歧义），失败由
        // reactivate_tool_app 兜底（zcode 已在其谓词与认领关键字内）
        if form.as_deref() == Some("app") {
            if let (Some(sid), Some(agent)) = (session_id.as_deref(), agent_type.as_deref()) {
                if matches!(agent, "workbuddy" | "codex")
                    && crate::window::deep_link::scheme_handler_exists(agent)
                {
                    let url = crate::window::deep_link::session_url(agent, sid);
                    if let Some(url) = url {
                        if crate::window::deep_link::open_url(&url).is_ok() {
                            if crate::window::win32::verify_foreground_tool(
                                &mut system,
                                agent,
                                2_000,
                                250,
                            ) {
                                mark_read_on_jump(&app, &session_id, &agent_type);
                                return Ok(serde_json::json!({
                                    "type": "focused",
                                    "via": "deep-link"
                                }));
                            }
                            // P2-1（B 兜底保险，issue #34）：派发已发生但前台验证未确认
                            // ——宿主可能恰在验证窗口后被冷启动。落保底链路前重刷快照，
                            // 使 running_projects / resolve_and_focus / reactivate_
                            // tool_app 能认出派发期间新出现的宿主进程；everything 全量
                            // 刷新，下游消费的 cwd/exe 均取最新
                            system.refresh_processes_specifics(
                                sysinfo::ProcessesToUpdate::All,
                                true,
                                sysinfo::ProcessRefreshKind::everything(),
                            );
                        }
                    }
                }
            }
        }
        // marker 与按需注入 helper 贴的标题标记一致：MAM:<session_id 剥连字符后
        // 前 12 位>。口径两处互引（改动须同步）：本处（匹配侧）/ mam-marker
        // helper（src/bin/mam-marker.rs，注入侧）。8 位对 codex UUIDv7 只编码
        // 65.5s 粒度、同分钟双开撞车（实测 2026-09-08），12 位不撞；必须先剥
        // 连字符——UUID 第 9 位即 '-'，直接 take(12) 会切进分隔符
        let marker = session_id.as_deref().map(|id| {
            format!(
                "MAM:{}",
                id.chars()
                    .filter(|c| *c != '-')
                    .take(12)
                    .collect::<String>()
            )
        });
        // 面板反推：当前所有运行会话的 (工具id, 项目名)，用于排除其他工具的终端窗口
        // （codex 终端标题=项目名，无 "codex" 关键词可静态认领）。进程扫描即可，无文件解析开销
        let running_projects = running_projects_from_processes(&system);
        // 配对不确定门（issue #48）：同工具同项目 ≥2 个进程在跑时，卡片 pid 与会话的
        // 启发式配对（kimi=wire mtime 最新 / opencode=time_updated DESC）可能互换——
        // 跳转禁用一切演绎锁定（单窗口即锁/幸存者推理），只认正向证据，否则交选择器。
        // 项目名比对忽略大小写（Windows 路径不区分大小写），与 running_projects 的
        // cwd file_name 同源
        let require_evidence = {
            let agent = agent_type.as_deref().unwrap_or_default().to_lowercase();
            match project_name.as_deref().map(str::to_lowercase) {
                Some(p) if !p.is_empty() => {
                    running_projects
                        .iter()
                        .filter(|(a, pr)| a.to_lowercase() == agent && pr.to_lowercase() == p)
                        .count()
                        >= 2
                }
                _ => false,
            }
        };
        // 按需注入（spec 2026-09-12 §3.2）：claude/codex CLI 会话在判定链前贴
        // marker，① 层即可精确命中。**配对不确定时必须一并禁用**（第四轮 3c 实证）：
        // 注入目标取自卡片 pid，pid 本身可能配对交叉 → marker 贴到兄弟会话的终端，
        // ① 层随之高置信锁错窗（marker 命中构成自证循环，不是正向证据）。禁用后
        // 该场景回落双命中仲裁/选择器，"锁对或选择器、绝不锁错"契约恢复。
        // helper 缺失/失败/超时仍然全部静默回落
        if on_demand_injection_allowed(agent_type.as_deref(), form.as_deref(), require_evidence) {
            if let Some(sid) = session_id.as_deref() {
                if let Some(m) = marker.as_deref() {
                    crate::window::win32::inject_marker_on_demand(sid, pid, m);
                }
            }
        }
        let hints = crate::window::win32::JumpHints {
            session_marker: marker.as_deref(),
            agent_keyword: agent_type.as_deref(),
            project_name: project_name.as_deref(),
            last_message: last_message.as_deref(),
            title: title.as_deref(),
            require_positive_evidence: require_evidence,
        };
        match crate::window::win32::resolve_and_focus(&system, pid, &hints, &running_projects) {
            Ok(crate::window::win32::FocusOutcome::Focused) => {
                // 聚焦成功清痕（round-5）：剥掉本次注入的 marker，标题回到干净态
                //（下次跳转重新注入，按需注入本就是一次性模式）。清除范围与注入的
                // 工具/形态门一致；Ambiguous 选择器分支不清——marker 还在候选标题上，
                // 供用户辨认，关选择器后自然过期/下次注入时被剥
                if on_demand_marker_applies(agent_type.as_deref(), form.as_deref()) {
                    crate::window::win32::clear_marker_after_focus(pid);
                }
                mark_read_on_jump(&app, &session_id, &agent_type);
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
                    mark_read_on_jump(&app, &session_id, &agent_type);
                    Ok(serde_json::json!({ "type": "focused" }))
                } else {
                    Err(e)
                }
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (project_name, last_message, title, form, unread);
        // CLI 形态：TTY 链路（tmux/iTerm2/Terminal.app）
        if crate::window::focus_terminal_for_pid(pid).is_ok() {
            mark_read_on_jump(&app, &session_id, &agent_type);
            return Ok(serde_json::json!({ "type": "focused", "via": "tty" }));
        }
        // APP 形态 / pid 失效兜底：深度链接 → bundle 激活 → 按工具枚举（W2）。
        // via=app-fallback：CLI 会话 TTY 聚焦失败走到这里的 UX 提示依据（review M3）
        #[cfg(target_os = "macos")]
        if let Some(mut out) =
            crate::window::activate_agent_app(pid, agent_type.as_deref(), session_id.as_deref())
        {
            // P1-2（review）：macOS 深链无前台验证，路由成败不可证 → 不标已读
            //（宁可让用户再点一次/X 关闭，不可误删未读卡）；bundle/枚举兜底激活
            // 仍按 spec §5「兜底激活同样算跳转成功」回标已读
            let via = out.get("via").and_then(|v| v.as_str());
            if macos_deep_link_marks_read(via) {
                mark_read_on_jump(&app, &session_id, &agent_type);
                out["via"] = serde_json::Value::String("app-fallback".into());
            }
            return Ok(out);
        }
        // 非 macOS 桌面平台无 APP 激活链路，防未使用告警
        #[cfg(not(target_os = "macos"))]
        let _ = (agent_type, session_id);
        // 措辞兼容两种场景：pid 存活的 CLI 会话（有 TTY 但聚焦失败，P2-2 后不再做
        // APP 兜底）与 pid=0/已死（TTY 无从聚焦 + 宿主 APP 枚举未命中）
        Err(format!(
            "无法聚焦目标（pid={}）：终端与宿主 APP 均未能聚焦",
            pid
        ))
    }
}

/// 按需注入工具/形态门（spec §3.2）：仅无可靠静态标题键的 claude/codex 且 CLI
/// 形态。kimi/opencode 标题键已实测够用，注入只会污染其标题
#[cfg(any(windows, test))]
fn on_demand_marker_applies(agent: Option<&str>, form: Option<&str>) -> bool {
    matches!(agent, Some("claude" | "codex")) && form != Some("app")
}

/// 按需注入总门 = 工具/形态门 ∧ ¬配对不确定。配对不确定（同工具同项目 ≥2 进程）
/// 时卡片 pid 可能与会话交叉，注入目标随之错配，marker 命中构成自证循环——
/// 第四轮 3c 实证 codex 同项目双开两次静默锁错，故此场景禁注入、回落既有层
#[cfg(any(windows, test))]
fn on_demand_injection_allowed(
    agent: Option<&str>,
    form: Option<&str>,
    pairing_ambiguous: bool,
) -> bool {
    on_demand_marker_applies(agent, form) && !pairing_ambiguous
}

/// macOS 深链成功是否回标已读（P1-2 判定核心，cfg(test) 使其 Windows 侧可测）：
/// via=deep-link → 不标（无前台验证，路由成败不可证）；其余（bundle/枚举兜底）→ 标
#[cfg(any(target_os = "macos", test))]
fn macos_deep_link_marks_read(via: Option<&str>) -> bool {
    via != Some("deep-link")
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

/// 手动关闭活跃 App 卡（T2 X 按钮，「暂离不提示」）：写入进程内 dismiss 集合，
/// 从看板与宠物隐藏；同一会话状态变化后 key 不匹配自然重现。
/// 不碰 unread_sessions 表（未读卡的 X 走 mark_session_read 已读语义）。
/// status 用前端 SessionStatus 字符串（serde lowercase，如 "waiting"），
/// 与 filter_dismissed_cards 的归一化（Debug 形态转小写）一致
#[tauri::command]
pub fn dismiss_session_card(agent_type: String, session_id: String, status: String) {
    crate::monitor::SESSION_DISMISALS.lock().unwrap().insert((
        agent_type.to_lowercase(),
        session_id,
        status.to_lowercase(),
    ));
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

#[cfg(test)]
mod macos_deep_link_read_tests {
    use super::macos_deep_link_marks_read;

    /// P1-2 回归锁：macOS 深链（无前台验证，路由成败不可证）成功也不得回标已读；
    /// bundle/枚举兜底激活（spec §5「同样算跳转成功」）与 TTY 直达照旧标已读
    #[test]
    fn deep_link_via_never_marks_read() {
        assert!(!macos_deep_link_marks_read(Some("deep-link")));
        assert!(macos_deep_link_marks_read(None)); // bundle/枚举兜底（无 via 标记）
        assert!(macos_deep_link_marks_read(Some("tty")));
        assert!(macos_deep_link_marks_read(Some("app-fallback")));
    }
}

#[cfg(test)]
mod on_demand_tests {
    // 门控契约（spec §3.2）：仅 claude/codex 且仅 CLI 形态注入；其余零开销跳过
    use super::{on_demand_injection_allowed, on_demand_marker_applies};

    #[test]
    fn applies_to_claude_and_codex_cli_only() {
        assert!(on_demand_marker_applies(Some("claude"), Some("cli")));
        assert!(on_demand_marker_applies(Some("codex"), Some("cli")));
        assert!(on_demand_marker_applies(Some("claude"), None)); // form 缺失按 CLI 处理
        assert!(!on_demand_marker_applies(Some("claude"), Some("app")));
        assert!(!on_demand_marker_applies(Some("kimi"), Some("cli"))); // 标题键够用
        assert!(!on_demand_marker_applies(Some("opencode"), Some("cli"))); // 同上
        assert!(!on_demand_marker_applies(None, Some("cli")));
    }

    #[test]
    fn pairing_ambiguous_disables_injection() {
        // 第四轮 3c 回归锁：同工具同项目双开（配对不确定）时禁用注入——
        // 错配 pid 注入会把 marker 贴到兄弟会话终端，① 层自证循环锁错窗
        assert!(on_demand_injection_allowed(
            Some("codex"),
            Some("cli"),
            false
        ));
        assert!(!on_demand_injection_allowed(
            Some("codex"),
            Some("cli"),
            true
        ));
        assert!(!on_demand_injection_allowed(
            Some("claude"),
            Some("cli"),
            true
        ));
        assert!(!on_demand_injection_allowed(
            Some("kimi"),
            Some("cli"),
            false
        ));
    }
}

#[cfg(test)]
mod marker_literal_tests {
    // 匹配侧 marker 构造回归锁（与本文件 focus_session 的内联构造逐字一致；
    // 口径内联于命令内，按互引纪律不抽取 helper，靠本锁镜像防漂移）。
    // 与注入侧锁成对：src/bin/mam-marker.rs marker_strips_hyphens_and_takes_12
    // 对同一 session id 断言同一字面量，两侧改动必须同步（互引：focus_session
    // marker 注释 / mam-marker.rs 模块注释）
    #[test]
    fn matching_side_marker_literal_matches_helper_side() {
        let session_id = "01a08083-5ca0-4948-8276-9a0b8c7d6e5f";
        let marker = format!(
            "MAM:{}",
            session_id
                .chars()
                .filter(|c| *c != '-')
                .take(12)
                .collect::<String>()
        );
        assert_eq!(marker, "MAM:01a080835ca0");
    }
}
