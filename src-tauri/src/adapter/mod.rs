// AgentAdapter trait + 枚举 + 会话发现调度器
// 移植自 agent-sessions agent/mod.rs，扩展支持 Codex CLI/APP 和 OpenCode

pub mod claude;
pub mod codex;
pub mod kimi;
pub mod openclaw;
pub mod opencode;

use crate::session::{
    status_sort_priority, AgentType, ProcessForm, Session, SessionStatus, SessionsResponse,
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
pub const TOOL_IDS: &[&str] = &["claude", "codex", "opencode", "openclaw", "kimi"];

/// 工具 id → adapter 的唯一登记处。新增工具只需在此加一行（+ 其 adapter 文件），
/// 服务层（mcp/skill/plugin/preset/resource/detector）统一经此分发，无需各自加 arm
pub fn adapter_by_id(tool_id: &str) -> Option<Box<dyn AgentAdapter>> {
    match tool_id {
        "claude" => Some(Box::new(claude::ClaudeAdapter)),
        "codex" => Some(Box::new(codex::CodexAdapter)),
        "opencode" => Some(Box::new(opencode::OpenCodeAdapter)),
        "openclaw" => Some(Box::new(openclaw::OpenClawAdapter)),
        "kimi" => Some(Box::new(kimi::KimiAdapter)),
        _ => None,
    }
}

/// 全部已注册 adapter（会话扫描、工具检测的调度入口）
pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> {
    TOOL_IDS.iter().filter_map(|&id| adapter_by_id(id)).collect()
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
    let adapters: Vec<Box<dyn AgentAdapter>> = all_adapters();

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

    // 会话级去重（见 dedup_sessions 注释）
    dedup_sessions(&mut all_sessions);

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

    // 按状态优先级排序
    all_sessions.sort_by(|a, b| {
        let pa = status_sort_priority(&a.status);
        let pb = status_sort_priority(&b.status);
        if pa != pb {
            pa.cmp(&pb)
        } else {
            b.last_activity_at.cmp(&a.last_activity_at)
        }
    });

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
/// 统一维护每个工具的原生 skill 根目录，避免扫描、清理和启用各自硬编码路径
pub fn skill_dir_for_tool(tool_id: &str, home_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    match tool_id {
        "claude" => Some(home_dir.join(".claude").join("skills")),
        // Codex CLI 实际读取 ~/.agents/skills（项目约定 + 当前会话技能来源）
        "codex" => Some(home_dir.join(".agents").join("skills")),
        "opencode" => Some(home_dir.join(".config").join("opencode").join("skills")),
        "openclaw" => Some(home_dir.join(".openclaw").join("skills")),
        // Kimi Code 读取 $KIMI_CODE_HOME/skills（默认 ~/.kimi-code/skills），
        // 经 kimi_home() 保持 KIMI_CODE_HOME 重定向与 adapter 同源
        "kimi" => Some(crate::monitor::kimi_parser::kimi_home().join("skills")),
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
