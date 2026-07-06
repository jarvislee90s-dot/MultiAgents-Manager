// OpenClaw adapter — process discovery + openclaw.json parsing

use super::*;
use crate::monitor;

pub struct OpenClawAdapter;

impl AgentAdapter for OpenClawAdapter {
    fn name(&self) -> &'static str { "OpenClaw" }
    fn agent_type(&self) -> AgentType { AgentType::OpenClaw }
    fn process_names(&self) -> &'static [&'static str] { &["openclaw"] }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_openclaw_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::openclaw_parser::get_openclaw_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(".openclaw")
    }

    fn hook_supported(&self) -> bool { false }

    fn mcp_format(&self) -> McpFormat { McpFormat::Json }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("openclaw.json"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("skills")]
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("agents"))
    }

    fn plugin_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("plugins")]
    }
    fn plugin_config_paths(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("openclaw.json")]
    }
}
