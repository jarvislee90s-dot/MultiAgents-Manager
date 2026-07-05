// Claude Code adapter

use super::*;
use crate::monitor;

pub struct ClaudeAdapter;

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &'static str { "Claude Code" }
    fn agent_type(&self) -> AgentType { AgentType::Claude }
    fn process_names(&self) -> &'static [&'static str] { &["claude"] }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_claude_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::parser::get_claude_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir().unwrap_or_default().join(".claude")
    }

    fn hook_supported(&self) -> bool { true }
    fn hook_event_case(&self) -> HookEventCase { HookEventCase::PascalCase }
    fn hook_events(&self) -> Vec<&'static str> {
        vec!["Stop", "UserPromptSubmit", "SessionStart", "SessionEnd", "PreToolUse", "PostToolUse"]
    }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("settings.json"))
    }

    fn mcp_format(&self) -> McpFormat { McpFormat::Json }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(dirs::home_dir().unwrap_or_default().join(".claude.json"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("skills")]
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("agents"))
    }
}
