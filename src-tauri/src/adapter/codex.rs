// Codex CLI + 桌面 APP adapter

use super::*;
use crate::monitor;

pub struct CodexAdapter;

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &'static str { "Codex CLI" }
    fn agent_type(&self) -> AgentType { AgentType::Codex }
    fn process_names(&self) -> &'static [&'static str] { &["codex", "Codex"] }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_codex_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::parser::get_codex_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir().unwrap_or_default().join(".codex")
    }

    fn hook_supported(&self) -> bool { true }
    fn hook_event_case(&self) -> HookEventCase { HookEventCase::CamelCase }
    fn hook_events(&self) -> Vec<&'static str> {
        vec!["stop", "userPromptSubmit", "sessionStart", "sessionEnd", "preToolUse", "postToolUse"]
    }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("hooks.json"))
    }

    fn mcp_format(&self) -> McpFormat { McpFormat::Toml }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("config.toml"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        // Codex CLI 使用 ~/.agents/skills/（与 AGENTS.md 约定一致）
        vec![dirs::home_dir().unwrap_or_default().join(".agents").join("skills")]
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("agents"))
    }

    fn plugin_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("plugins")]
    }
    fn plugin_config_paths(&self) -> Vec<std::path::PathBuf> {
        vec![self.base_dir().join("config.toml")]
    }
}
