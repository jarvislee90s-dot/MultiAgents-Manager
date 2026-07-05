// AgentAdapter trait + 枚举 + 会话发现调度器
// 移植自 agent-sessions agent/mod.rs，扩展支持 Codex CLI/APP 和 OpenCode

pub mod claude;
pub mod codex;
pub mod opencode;

use crate::session::{status_sort_priority, AgentType, ProcessForm, Session, SessionStatus, SessionsResponse};
use std::collections::HashSet;
use std::sync::Mutex;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

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

    fn hook_supported(&self) -> bool { false }
    fn hook_event_case(&self) -> HookEventCase { HookEventCase::None }
    fn hook_events(&self) -> Vec<&'static str> { Vec::new() }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> { None }

    fn mcp_format(&self) -> McpFormat { McpFormat::Json }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> { None }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> { Vec::new() }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> { None }

}

/// 共享 System 实例 — 每轮询周期刷新一次，所有 adapter 共用
static SHARED_SYSTEM: Mutex<Option<System>> = Mutex::new(None);

/// 获取所有注册 adapter 的会话
pub fn get_all_sessions() -> SessionsResponse {
    let adapters: Vec<Box<dyn AgentAdapter>> = vec![
        Box::new(claude::ClaudeAdapter),
        Box::new(codex::CodexAdapter),
        Box::new(opencode::OpenCodeAdapter),
    ];

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
                        .with_cpu()
                )
            )
        });
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_cwd(sysinfo::UpdateKind::Always)
                .with_cpu()
        );

        adapters.iter().map(|a| a.find_processes(system)).collect()
    }; // 释放 System 锁 — 下方文件 I/O 无需持锁

    // Phase 2: 解析会话（文件 I/O）
    let mut all_sessions: Vec<Session> = Vec::new();
    for (adapter, processes) in adapters.iter().zip(all_processes.iter()) {
        let sessions = adapter.find_sessions(processes);
        log::info!("{}: {} processes, {} sessions",
            adapter.name(), processes.len(), sessions.len());
        all_sessions.extend(sessions);
    }

    // Hook 事件集成：用新鲜事件（<30s）更新会话状态
    let hook_events = crate::monitor::hooks::read_hook_events();
    for session in &mut all_sessions {
        if let Some(event) = hook_events.get(&session.pid) {
            // 根据 Hook 事件类型更新状态
            let new_status = match event.event.as_str() {
                "Stop" | "stop" => Some(SessionStatus::Waiting),
                "PreToolUse" | "preToolUse" => Some(SessionStatus::Processing),
                "UserPromptSubmit" | "userPromptSubmit" => Some(SessionStatus::Thinking),
                "SessionStart" | "sessionStart" => Some(SessionStatus::Idle),
                "SessionEnd" | "sessionEnd" => Some(SessionStatus::Finished),
                _ => None,
            };
            if let Some(status) = new_status {
                log::debug!("Hook event {} → {:?} for pid={}", event.event, status, session.pid);
                session.status = status;
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

    let waiting_count = all_sessions.iter()
        .filter(|s| matches!(s.status, SessionStatus::Waiting))
        .count();

    // 更新会话状态缓存（通知去重用）
    for session in &all_sessions {
        let _ = crate::store::update_session_status(
            &session.id,
            &format!("{:?}", session.agent_type),
            &format!("{:?}", session.status),
        );
    }
    // 清理不再活跃的会话缓存
    let active_ids: HashSet<String> = all_sessions.iter().map(|s| s.id.clone()).collect();
    crate::store::cleanup_stale_sessions(&active_ids);

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
        eprintln!("Total: {}, Waiting: {}", response.total_count, response.waiting_count);
        for session in &response.sessions {
            eprintln!("  [{:?}] {} {:?} pid={} form={:?} jump={} status={:?} msg={}",
                session.agent_type, session.project_name, session.status,
                session.pid, session.form, session.jump_supported, session.status,
                session.last_message.as_deref().unwrap_or("(none)"));
        }
        eprintln!("=== END ===");
    }
}
