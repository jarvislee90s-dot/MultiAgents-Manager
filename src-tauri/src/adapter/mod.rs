// AgentAdapter trait + 枚举 + 会话发现调度器
// 移植自 agent-sessions agent/mod.rs，扩展支持 Codex CLI/APP 和 OpenCode

pub mod claude;
pub mod codex;
pub mod kimi;
pub mod openclaw;
pub mod opencode;
pub mod workbuddy;

use crate::session::{
    jump_supported_for, status_sort_priority, AgentType, ProcessForm, Session, SessionStatus,
    SessionsResponse,
};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// 读取 CLI grace period（秒），默认 5
fn get_cli_grace_secs() -> i64 {
    crate::database::get_setting("cli_grace_secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
}

/// 读取 APP grace period（秒），默认 30
fn get_app_grace_secs() -> i64 {
    crate::database::get_setting("app_grace_secs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30)
}

/// 记录每个 PID 最近一次 Stop 事件的 (时间戳, grace_duration_secs)，用于 grace period 判定
/// grace_duration 按进程形态区分：App 形态更长（30s），CLI 形态更短（5s）
static STOP_GRACE: Lazy<Mutex<HashMap<u32, (i64, i64)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// 通用进程信息
#[derive(Debug, Clone)]
pub struct AgentProcess {
    pub pid: u32,
    pub cpu_usage: f32,
    pub cwd: Option<std::path::PathBuf>,
    /// 可执行路径（宿主判定 / classify_form 的关键输入；review F2 起 App 宿主判定依赖）
    pub exe: Option<std::path::PathBuf>,
    pub form: ProcessForm,
}

/// Hook 事件名大小写格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventCase {
    PascalCase,
    CamelCase,
    None,
}

/// MCP 配置格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFormat {
    Json,
    Toml,
    Jsonc,
}

/// Agent 适配器 trait — 每个工具实现此接口
pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn agent_type(&self) -> AgentType;
    fn process_names(&self) -> &'static [&'static str];
    fn find_processes(&self, system: &System) -> Vec<AgentProcess>;
    fn base_dir(&self) -> std::path::PathBuf;

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        let _ = processes;
        Vec::new()
    }

    fn hook_supported(&self) -> bool {
        false
    }
    fn hook_event_case(&self) -> HookEventCase {
        HookEventCase::None
    }
    fn hook_events(&self) -> Vec<&'static str> {
        Vec::new()
    }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        None
    }

    fn mcp_format(&self) -> McpFormat {
        McpFormat::Json
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        None
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    fn plugin_dirs(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn plugin_config_paths(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}

/// 已注册工具 id 列表（登记顺序即扫描/展示顺序）
pub const TOOL_IDS: &[&str] = &[
    "claude",
    "codex",
    "opencode",
    "openclaw",
    "kimi",
    "workbuddy",
];

/// 工具 id → adapter 的唯一登记处。新增工具只需在此加一行（+ 其 adapter 文件），
/// 服务层（mcp/skill/plugin/preset/resource/detector）统一经此分发，无需各自加 arm
pub fn adapter_by_id(tool_id: &str) -> Option<Box<dyn AgentAdapter>> {
    match tool_id {
        "claude" => Some(Box::new(claude::ClaudeAdapter)),
        "codex" => Some(Box::new(codex::CodexAdapter)),
        "opencode" => Some(Box::new(opencode::OpenCodeAdapter)),
        "openclaw" => Some(Box::new(openclaw::OpenClawAdapter)),
        "kimi" => Some(Box::new(kimi::KimiAdapter)),
        "workbuddy" => Some(Box::new(workbuddy::WorkBuddyAdapter)),
        _ => None,
    }
}

/// 全部已注册 adapter（会话扫描、工具检测的调度入口）
pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> {
    TOOL_IDS
        .iter()
        .filter_map(|&id| adapter_by_id(id))
        .collect()
}

/// 仅已启用（设置-工具管理中勾选）工具的 adapter。
/// W5：会话扫描等用户可见入口使用；管理类入口（工具设置/资源扫描）仍走
/// all_adapters()/all_adapters_with_ids()，保证未勾选工具可重新开启。
/// 行缺失视为启用（get_tool_enabled 的防御语义），老用户升级零感知
pub fn enabled_adapters() -> Vec<Box<dyn AgentAdapter>> {
    TOOL_IDS
        .iter()
        .filter(|id| crate::database::dao::agent_tool::get_tool_enabled(id))
        .filter_map(|id| adapter_by_id(id))
        .collect()
}

/// 全部已注册 (工具 id, adapter)（资源扫描等需要 id 的场景）
pub fn all_adapters_with_ids() -> Vec<(&'static str, Box<dyn AgentAdapter>)> {
    TOOL_IDS
        .iter()
        .filter_map(|&id| adapter_by_id(id).map(|a| (id, a)))
        .collect()
}

/// 共享 System 实例 — 每轮询周期刷新一次，所有 adapter 共用
static SHARED_SYSTEM: Mutex<Option<System>> = Mutex::new(None);

/// 会话级去重：同一 (工具, session id) 只保留首张卡。
/// 全局防线：任何解析器的"文件级/进程级复制"型 bug（如 opencode 多进程同会话、
/// codex 每轮新 rollout）在这里统一兜住，一处修复覆盖全部工具
pub fn dedup_sessions(sessions: &mut Vec<Session>) {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    sessions.retain(|s| seen.insert((format!("{:?}", s.agent_type), s.id.clone())));
}

/// 获取所有注册 adapter 的会话
pub fn get_all_sessions() -> SessionsResponse {
    // W5：未勾选工具不参与会话扫描（看板卡/通知随之静默）
    let adapters: Vec<Box<dyn AgentAdapter>> = enabled_adapters();

    // Phase 1: 刷新共享 System 快照，发现所有进程
    let all_processes: Vec<Vec<AgentProcess>> = {
        let mut guard = SHARED_SYSTEM.lock().unwrap();
        let system = guard.get_or_insert_with(|| {
            log::debug!("Initializing shared System instance");
            System::new_with_specifics(
                RefreshKind::new().with_processes(
                    ProcessRefreshKind::new()
                        .with_cmd(sysinfo::UpdateKind::Always)
                        .with_cwd(sysinfo::UpdateKind::Always)
                        // exe 路径是 Windows MSIX 形态判定（classify_form）的关键输入：
                        // 缺失时 ChatGPT 内嵌 codex.exe 会被误判为 CLI（提权进程 cmd 也读不到）
                        .with_exe(sysinfo::UpdateKind::Always)
                        .with_cpu(),
                ),
            )
        });
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_cwd(sysinfo::UpdateKind::Always)
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cpu(),
        );

        adapters.iter().map(|a| a.find_processes(system)).collect()
    }; // 释放 System 锁 — 下方文件 I/O 无需持锁

    // Phase 2: 解析会话（文件 I/O）
    let mut all_sessions: Vec<Session> = Vec::new();
    for (adapter, processes) in adapters.iter().zip(all_processes.iter()) {
        let sessions = adapter.find_sessions(processes);
        log::info!(
            "{}: {} processes, {} sessions",
            adapter.name(),
            processes.len(),
            sessions.len()
        );
        all_sessions.extend(sessions);
    }

    // W4：WorkBuddy 心跳消失补偿（转绿未被观测的会话补插未读，随本轮合并为绿卡）
    crate::monitor::workbuddy_parser::compensate_vanished_heartbeats();

    // 会话级去重（见 dedup_sessions 注释）
    dedup_sessions(&mut all_sessions);

    // review F2：宿主 APP 已死的孤儿会话进程不得产出活跃卡。
    // 复用 SHARED_SYSTEM 快照（Phase 1 已带 exe 刷新），不另起全量扫描
    {
        let guard = SHARED_SYSTEM.lock().unwrap();
        filter_host_dead_cards(&mut all_sessions, &|tool_id| {
            guard
                .as_ref()
                .map(|system| crate::monitor::host::tool_host_alive_in(system, tool_id))
                .unwrap_or(true) // 快照不可用时防御性放行（与旧行为一致）
        });
    }

    // W4：APP 类未读卡合并 + 未读池维护（宿主存活检查 / 变黄删除 / 过期清理）
    sync_unread_sessions(&mut all_sessions);

    // Hook 事件集成：用新鲜事件（<30s）更新会话状态
    let hook_events = crate::monitor::hooks::read_hook_events();
    let now_ts = chrono::Utc::now().timestamp();
    let mut grace = STOP_GRACE.lock().unwrap();
    for session in &mut all_sessions {
        if let Some(event) = hook_events.get(&session.pid) {
            match event.event.as_str() {
                "Stop" | "stop" => {
                    // 按形态计算 grace 时长：APP 形态更长（subagent 调度场景，单步间隔长），CLI 较短
                    let grace_secs = if matches!(session.form, ProcessForm::App) {
                        get_app_grace_secs()
                    } else {
                        get_cli_grace_secs()
                    };
                    // 记录 grace 时间戳和时长，不直接改 status — 由 grace 判定综合决定
                    grace.insert(session.pid, (event.ts, grace_secs));
                    if now_ts - event.ts < grace_secs {
                        // grace 期内：保持黄灯（覆盖 JSONL 推导的 Waiting/Idle）
                        if !matches!(
                            session.status,
                            SessionStatus::Processing
                                | SessionStatus::Thinking
                                | SessionStatus::Compacting
                        ) {
                            log::debug!(
                                "Stop grace 期内（{}s）保持黄灯: pid={}, form={:?}",
                                grace_secs,
                                session.pid,
                                session.form
                            );
                            session.status = SessionStatus::Processing;
                        }
                    } else {
                        // 过期：Agent 已停止活动超过 grace 期，进入等待用户态
                        session.status = SessionStatus::Waiting;
                    }
                }
                _ => {
                    // 其他事件：清 grace，正常映射
                    grace.remove(&session.pid);
                    let new_status = match event.event.as_str() {
                        "PreToolUse" | "preToolUse" => Some(SessionStatus::Processing),
                        "UserPromptSubmit" | "userPromptSubmit" => Some(SessionStatus::Thinking),
                        "SessionStart" | "sessionStart" => Some(SessionStatus::Idle),
                        "SessionEnd" | "sessionEnd" => Some(SessionStatus::Finished),
                        _ => None,
                    };
                    if let Some(status) = new_status {
                        log::debug!(
                            "Hook event {} → {:?} for pid={}",
                            event.event,
                            status,
                            session.pid
                        );
                        session.status = status;
                    }
                }
            }
        } else if let Some(&(stop_ts, grace_secs)) = grace.get(&session.pid) {
            // 没有新事件但有过 Stop 记录 — 使用存储的 grace duration 判断过期
            if now_ts - stop_ts >= grace_secs {
                // grace 已过期：Agent 已停止活动，进入 Idle 状态
                session.status = SessionStatus::Idle;
                grace.remove(&session.pid);
            }
        }
    }

    // T2：用户 X 掉的 App 形态卡按 (tool, session, status) 过滤——放在 Hook 状态更新
    // 之后（status 已是最终值），排序之前。状态变化后 key 不匹配自然重现
    {
        let dismissals = crate::monitor::SESSION_DISMISALS.lock().unwrap();
        crate::monitor::filter_dismissed_cards(&mut all_sessions, &|tool, sid, status| {
            dismissals.contains(&(tool.to_string(), sid.to_string(), status.to_string()))
        });
    }

    // 按状态优先级排序（比较器见 session_sort_cmp）
    all_sessions.sort_by(session_sort_cmp);

    let waiting_count = all_sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Waiting))
        .count();

    // 更新会话状态缓存（通知去重用）
    for session in &all_sessions {
        let _ = crate::database::update_session_status(
            &session.id,
            &format!("{:?}", session.agent_type),
            &format!("{:?}", session.status),
        );
    }
    // 清理不再活跃的会话缓存
    let active_ids: HashSet<String> = all_sessions.iter().map(|s| s.id.clone()).collect();
    crate::database::cleanup_stale_sessions(&active_ids);

    SessionsResponse {
        total_count: all_sessions.len(),
        waiting_count,
        sessions: all_sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_sessions() {
        let response = get_all_sessions();
        eprintln!("=== SESSION SCAN ===");
        eprintln!(
            "Total: {}, Waiting: {}",
            response.total_count, response.waiting_count
        );
        for session in &response.sessions {
            eprintln!(
                "  [{:?}] {} {:?} pid={} form={:?} jump={} status={:?} msg={}",
                session.agent_type,
                session.project_name,
                session.status,
                session.pid,
                session.form,
                session.jump_supported,
                session.status,
                session.last_message.as_deref().unwrap_or("(none)")
            );
        }
        eprintln!("=== END ===");
    }
}
/// 看板排序比较器：状态优先级 → 同状态组内未读卡排后（spec §5 前端「未读卡排后」）
/// → 最近活动倒序。Rust sort_by 稳定，键全等时保持原相对次序
fn session_sort_cmp(a: &Session, b: &Session) -> std::cmp::Ordering {
    let pa = status_sort_priority(&a.status);
    let pb = status_sort_priority(&b.status);
    pa.cmp(&pb)
        .then_with(|| a.unread.cmp(&b.unread)) // false(活跃) 在前、true(未读) 排后
        .then_with(|| b.last_activity_at.cmp(&a.last_activity_at))
}

/// review F2：宿主 APP 已死 → App 形态活跃卡全部清除（孤儿 codebuddy 心跳未过期
/// 也不得出卡）；CLI 卡不依赖宿主；未读卡（unread=true）归池/宿主退出清池管线治理，
/// 本过滤器不碰。host_alive 经参数注入，复用 SHARED_SYSTEM 快照避免重复全量扫描
fn filter_host_dead_cards(sessions: &mut Vec<Session>, host_alive: &dyn Fn(&str) -> bool) {
    sessions.retain(|s| {
        !matches!(s.form, ProcessForm::App)
            || s.unread
            || host_alive(&format!("{:?}", s.agent_type).to_lowercase())
    });
}

/// 未读池动作（review F1：迁移触发语义，替代电平 upsert——
/// 电平 upsert 会让「跳转已读」删掉的行在下一轮复活，已读永远不生效）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnreadPoolAction {
    /// 上一轮非绿 → 本轮绿（状态迁移）：插入/覆盖池行
    Insert,
    /// 持续绿色：仅刷新在场行展示字段（不重插已删行，转绿时间不滑动）
    RefreshDisplay,
    /// 非绿：池无动作（行删除由调用方无条件执行）
    None,
}

/// 判定未读池动作。prev_status 为状态缓存中的上一轮状态
/// （Debug 格式，与 get_all_sessions 末尾的统一更新一致）
fn unread_pool_action(prev_status: Option<&str>, idle: bool) -> UnreadPoolAction {
    if !idle {
        return UnreadPoolAction::None;
    }
    let was_idle = matches!(prev_status, Some("Idle") | Some("Finished"));
    if was_idle {
        UnreadPoolAction::RefreshDisplay
    } else {
        UnreadPoolAction::Insert
    }
}

/// W4 未读机制核心：把 DB 中的未读会话合并为 Session 卡，并维护未读池
/// - 会话当前非空闲（黄/红）→ 删未读行（活跃卡可见，防同会话双卡）
/// - 转绿（Idle/Finished）→ upsert 未读行（未在池中时）
/// - 宿主 APP 进程全部退出 → 清空该工具未读行与在板未读卡
/// - 过期（24h）→ 清理
fn sync_unread_sessions(active: &mut Vec<Session>) {
    let now_ms = chrono::Utc::now().timestamp_millis();

    // 1) 活跃会话驱动未读池变更
    for s in active.iter() {
        if !matches!(s.form, ProcessForm::App) {
            continue; // 仅 APP 类参与（spec W4 范围）
        }
        let tool = format!("{:?}", s.agent_type).to_lowercase();
        let idle = matches!(s.status, SessionStatus::Idle | SessionStatus::Finished);
        if idle {
            // review F1：迁移触发——状态缓存此刻存的是上一轮状态（回合末尾才统一更新），
            // 仅「上一轮非绿 → 本轮绿」才插入；持续绿色只刷新在场行展示字段，
            // 「跳转已读」删掉的行不再被电平 upsert 复活
            let prev = crate::database::find_status(&s.id);
            let record = crate::database::dao::unread::UnreadSessionRecord {
                tool_id: tool,
                session_id: s.id.clone(),
                project_name: s.project_name.clone(),
                title: s.title.clone(),
                last_message: s.last_message.clone(),
                turned_green_at_ms: now_ms,
                expires_at_ms: now_ms + 24 * 3600 * 1000,
            };
            match unread_pool_action(prev.as_deref(), true) {
                UnreadPoolAction::Insert => crate::database::dao::unread::upsert(&record),
                UnreadPoolAction::RefreshDisplay => {
                    crate::database::dao::unread::refresh_display(&record)
                }
                UnreadPoolAction::None => {}
            }
        } else {
            // 变黄/红：删未读（状态迁移，非重置机制）
            crate::database::dao::unread::delete(&tool, &s.id);
        }
    }

    // 2) 宿主进程退出 → 清该工具全部未读（运行中被关 + 重启残留检查统一规则）
    let unread_now = crate::database::dao::unread::list(now_ms);
    let mut dead_tools: Vec<String> = Vec::new();
    for r in &unread_now {
        if !dead_tools.contains(&r.tool_id) && !crate::monitor::host::tool_host_alive(&r.tool_id) {
            dead_tools.push(r.tool_id.clone());
        }
    }
    for t in &dead_tools {
        crate::database::dao::unread::clear_tool(t);
    }

    // 3) 过期清理
    {
        let conn = crate::database::connection::DB.lock().unwrap();
        crate::database::dao::unread::cleanup_expired_unread(&conn, now_ms);
    }

    // 4) 未读池合并为卡（纯映射见 build_unread_cards；追加在末尾，最终顺序由 session_sort_cmp 决定）
    let final_unread = crate::database::dao::unread::list(now_ms);
    let cards = build_unread_cards(&final_unread, active, &|id| {
        crate::database::dao::agent_tool::get_tool_enabled(id)
    });
    active.extend(cards);
}

/// 未读池 → 未读卡（纯函数，spec §8 可测试；启用判定经参数注入，测试不触库）：
/// - 跳过当前已在板的活跃会话（活跃卡由进程监控渲染，防同会话双卡）
/// - 未注册工具 id 的行丢弃（防御）
/// - 已停用工具的行丢弃（W5 纵深防御：即使其他路径让行残留池中，也不得复活为卡/通知）
/// - 卡字段：status=Idle / unread=true / pid=0（pid 失效，跳转走按工具兜底）/ form=App
fn build_unread_cards(
    pool: &[crate::database::dao::unread::UnreadSessionRecord],
    active: &[Session],
    tool_enabled: &dyn Fn(&str) -> bool,
) -> Vec<Session> {
    let active_keys: HashSet<(String, String)> = active
        .iter()
        .map(|s| (format!("{:?}", s.agent_type).to_lowercase(), s.id.clone()))
        .collect();
    let mut cards = Vec::new();
    for r in pool {
        if active_keys.contains(&(r.tool_id.clone(), r.session_id.clone())) {
            continue;
        }
        // W5 门禁：停用工具（彻底隐藏/通知静音）的池行不得合并为未读卡
        if !tool_enabled(&r.tool_id) {
            continue;
        }
        let Ok(agent_type) = serde_json::from_value::<AgentType>(serde_json::json!(r.tool_id))
        else {
            continue;
        };
        cards.push(Session {
            id: r.session_id.clone(),
            agent_type,
            project_name: r.project_name.clone(),
            project_path: String::new(),
            title: r.title.clone(),
            git_branch: None,
            github_url: None,
            status: SessionStatus::Idle,
            last_message: r.last_message.clone(),
            last_message_role: None,
            last_activity_at: chrono::DateTime::from_timestamp_millis(r.turned_green_at_ms)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default(),
            pid: 0, // pid 失效场景：跳转走 activate_agent_app 的按工具兜底
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: jump_supported_for(ProcessForm::App),
            unread: true,
        });
    }
    cards
}

#[cfg(test)]
mod unread_pool_action_tests {
    use super::*;

    /// review F1：未读池动作必须是「迁移触发」而非「电平触发」——
    /// 仅上一轮非绿 → 本轮绿才插入；持续绿色只刷新展示字段（不重插已读删掉的行）
    #[test]
    fn unread_pool_action_is_edge_triggered() {
        use UnreadPoolAction::{Insert, RefreshDisplay};
        // 首次观测（无缓存）：转绿 → 插入
        assert_eq!(unread_pool_action(None, true), Insert);
        // 上一轮非绿（黄/红/未知）→ 本轮绿：状态迁移，插入
        assert_eq!(unread_pool_action(Some("Thinking"), true), Insert);
        assert_eq!(unread_pool_action(Some("Processing"), true), Insert);
        assert_eq!(unread_pool_action(Some("Waiting"), true), Insert);
        // 上一轮已绿 → 本轮仍绿：仅刷新展示字段
        assert_eq!(unread_pool_action(Some("Idle"), true), RefreshDisplay);
        assert_eq!(unread_pool_action(Some("Finished"), true), RefreshDisplay);
        // 非绿：池无动作（行删除由调用方无条件执行）
        assert_eq!(
            unread_pool_action(Some("Idle"), false),
            UnreadPoolAction::None
        );
        assert_eq!(unread_pool_action(None, false), UnreadPoolAction::None);
    }
}

#[cfg(test)]
mod host_liveness_filter_tests {
    use super::*;

    fn fake(id: &str, form: ProcessForm, unread: bool) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::WorkBuddy,
            project_name: "P".into(),
            project_path: String::new(),
            title: None,
            git_branch: None,
            github_url: None,
            status: SessionStatus::Idle,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid: 7,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form,
            jump_supported: true,
            unread,
        }
    }

    /// review F2：宿主 APP 已死（孤儿 codebuddy 心跳仍新鲜）时，App 形态活跃卡
    /// 必须消失；CLI 卡不依赖宿主；未读卡由池管线（宿主退出清池）单独治理
    #[test]
    fn host_dead_removes_active_app_cards_only() {
        let mut sessions = vec![
            fake("wb-active", ProcessForm::App, false),
            fake("cli", ProcessForm::Cli, false),
            fake("unread-card", ProcessForm::App, true),
        ];
        filter_host_dead_cards(&mut sessions, &|_| false);
        assert!(
            !sessions.iter().any(|s| s.id == "wb-active"),
            "宿主死的活跃卡必须清掉"
        );
        assert!(
            sessions.iter().any(|s| s.id == "cli"),
            "CLI 卡不依赖宿主存活"
        );
        assert!(
            sessions.iter().any(|s| s.id == "unread-card"),
            "未读卡归池管线治理，此过滤器不碰"
        );
    }

    #[test]
    fn host_alive_keeps_everything() {
        let mut sessions = vec![
            fake("wb-active", ProcessForm::App, false),
            fake("cli", ProcessForm::Cli, false),
        ];
        filter_host_dead_cards(&mut sessions, &|_| true);
        assert_eq!(sessions.len(), 2);
    }
}

/// 统一维护每个工具的原生 skill 根目录，避免扫描、清理和启用各自硬编码路径
pub fn skill_dir_for_tool(tool_id: &str, home_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    match tool_id {
        "claude" => Some(home_dir.join(".claude").join("skills")),
        // Codex CLI 实际读取 ~/.agents/skills（项目约定 + 当前会话技能来源）
        "codex" => Some(home_dir.join(".agents").join("skills")),
        "opencode" => Some(home_dir.join(".config").join("opencode").join("skills")),
        "openclaw" => Some(home_dir.join(".openclaw").join("skills")),
        // Kimi Code 读取 $KIMI_CODE_HOME/skills（默认 <home_dir>/.kimi-code/skills），
        // 经 kimi_home_with 保持 KIMI_CODE_HOME 重定向与 adapter 同源，同时尊重注入的 home_dir
        "kimi" => Some(crate::monitor::kimi_parser::kimi_home_with(home_dir).join("skills")),
        // WorkBuddy 读取 ~/.workbuddy/skills（数据根目录 ~/.workbuddy）
        "workbuddy" => Some(home_dir.join(".workbuddy").join("skills")),
        _ => None,
    }
}

/// 获取当前用户环境下工具的主 skill 目录
pub fn primary_skill_dir(tool_id: &str) -> Option<std::path::PathBuf> {
    skill_dir_for_tool(tool_id, &dirs::home_dir().unwrap_or_default())
}

#[cfg(test)]
mod skill_dir_tests {
    use super::*;

    #[test]
    fn codex_skill_dir_uses_real_cli_directory() {
        let dir = skill_dir_for_tool("codex", std::path::Path::new("/home/test"))
            .expect("codex skill dir must be registered");
        assert_eq!(dir, std::path::Path::new("/home/test/.agents/skills"));
    }

    #[test]
    fn unknown_tool_has_no_skill_dir() {
        assert_eq!(
            skill_dir_for_tool("unknown", std::path::Path::new("/home/test")),
            None
        );
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::dedup_sessions;
    use crate::session::model::{AgentType, ProcessForm, Session, SessionStatus};

    fn fake(id: &str, pid: u32) -> Session {
        Session {
            id: id.to_string(),
            agent_type: AgentType::OpenCode,
            project_name: "p".into(),
            project_path: "p".into(),
            title: None,
            git_branch: None,
            github_url: None,
            status: SessionStatus::Idle,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::Cli,
            jump_supported: true,
            unread: false,
        }
    }

    #[test]
    fn dedup_keeps_first_per_agent_and_session_id() {
        // 同 (工具, session id) 双 pid（opencode 多进程同会话实测形态）+ 一条不同 id
        let mut sessions = vec![fake("ses_A", 111), fake("ses_A", 222), fake("ses_B", 333)];
        dedup_sessions(&mut sessions);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].pid, 111); // 保留首张
        assert_eq!(sessions[1].id, "ses_B");
    }
}

#[cfg(test)]
mod sort_tests {
    use super::session_sort_cmp;
    use crate::session::model::{AgentType, ProcessForm, Session, SessionStatus};

    fn card(id: &str, unread: bool, activity: &str) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::WorkBuddy,
            project_name: "p".into(),
            project_path: "p".into(),
            title: None,
            git_branch: None,
            github_url: None,
            status: SessionStatus::Idle,
            last_message: None,
            last_message_role: None,
            last_activity_at: activity.into(),
            pid: 0,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: true,
            unread,
        }
    }

    #[test]
    fn unread_cards_sort_after_active_within_same_status() {
        // spec §5 前端「未读卡排后」：同状态组内活跃卡（unread=false）在前，
        // 未读卡排后（组内未读之间仍按最近活动倒序）
        let mut sessions = [
            card("old-active", false, "2026-09-04T09:00:00Z"),
            card("unread-new", true, "2026-09-04T10:00:00Z"),
            card("unread-old", true, "2026-09-04T08:00:00Z"),
        ];
        sessions.sort_by(session_sort_cmp);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["old-active", "unread-new", "unread-old"]);
    }

    #[test]
    fn status_priority_still_dominates_unread_flag() {
        // 未读标记只在同状态组内生效，不得跨状态提前（黄/红仍在绿前）
        let mut sessions = [
            card("unread-idle", true, "2026-09-04T10:00:00Z"),
            card("active-thinking", false, "2026-09-04T09:00:00Z"),
        ];
        sessions[1].status = SessionStatus::Thinking;
        sessions.sort_by(session_sort_cmp);
        assert_eq!(sessions[0].id, "active-thinking");
    }
}

#[cfg(test)]
mod unread_card_tests {
    use super::build_unread_cards;
    use crate::database::dao::unread::UnreadSessionRecord;
    use crate::session::model::{AgentType, ProcessForm, Session, SessionStatus};

    fn record(tool: &str, sid: &str) -> UnreadSessionRecord {
        UnreadSessionRecord {
            tool_id: tool.into(),
            session_id: sid.into(),
            project_name: "proj".into(),
            title: Some("标题".into()),
            last_message: Some("消息".into()),
            turned_green_at_ms: 1000,
            expires_at_ms: 1000 + 24 * 3600 * 1000,
        }
    }

    fn active_card(agent: AgentType, sid: &str) -> Session {
        Session {
            id: sid.into(),
            agent_type: agent,
            project_name: "proj".into(),
            project_path: "p".into(),
            title: None,
            git_branch: None,
            github_url: None,
            status: SessionStatus::Thinking,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid: 42,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: true,
            unread: false,
        }
    }

    #[test]
    fn merges_pool_skipping_active_and_mapping_fields() {
        // 在板活跃会话（WorkBuddy/live）跳过（活跃卡已由进程监控渲染，防双卡）；
        // 其余映射为未读卡：tool_id → AgentType、unread=true、pid=0（跳转走按工具兜底）、
        // form=App、status=Idle
        let active = vec![active_card(AgentType::WorkBuddy, "live")];
        let pool = vec![record("workbuddy", "live"), record("codex", "done")];
        let cards = build_unread_cards(&pool, &active, &|_| true);
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.id, "done");
        assert_eq!(c.agent_type, AgentType::Codex);
        assert!(c.unread);
        assert_eq!(c.pid, 0);
        assert_eq!(c.form, ProcessForm::App);
        assert_eq!(c.status, SessionStatus::Idle);
    }

    #[test]
    fn unknown_tool_row_is_dropped() {
        // 未注册工具 id（防御）→ 丢弃该行，不产出卡
        let cards = build_unread_cards(&[record("ghost", "s1")], &[], &|_| true);
        assert!(cards.is_empty());
    }

    #[test]
    fn disabled_tool_row_is_dropped() {
        // W5 纵深防御：已停用工具的池行不得合并为未读卡（停用 = 彻底隐藏/通知静音，
        // 防止补偿等残留路径把未读卡「复活」并触发完成通知）
        let pool = [record("workbuddy", "s1"), record("codex", "s2")];
        let cards = build_unread_cards(&pool, &[], &|id| id != "workbuddy");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].id, "s2");
        assert_eq!(cards[0].agent_type, AgentType::Codex);
        assert!(cards[0].unread);
    }
}
