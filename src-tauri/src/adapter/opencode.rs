// OpenCode adapter — 进程发现 + SQLite 会话解析（opencode.db）

use super::*;
use crate::monitor;

pub struct OpenCodeAdapter;

impl AgentAdapter for OpenCodeAdapter {
    fn name(&self) -> &'static str { "OpenCode" }
    fn agent_type(&self) -> AgentType { AgentType::OpenCode }
    fn process_names(&self) -> &'static [&'static str] { &["opencode"] }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_opencode_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::opencode_parser::get_opencode_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir().unwrap_or_default().join(".config").join("opencode")
    }

    fn hook_supported(&self) -> bool { false }

    fn mcp_format(&self) -> McpFormat { McpFormat::Jsonc }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("opencode.json"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("skills")]
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("agents"))
    }
}
