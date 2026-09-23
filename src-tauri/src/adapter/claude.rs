// Claude Code adapter

use super::*;
use crate::monitor;

pub struct ClaudeAdapter;

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "Claude Code"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::Claude
    }
    fn process_names(&self) -> &'static [&'static str] {
        &["claude"]
    }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_claude_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::claude_parser::get_claude_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir().unwrap_or_default().join(".claude")
    }

    fn hook_supported(&self) -> bool {
        true
    }
    fn hook_event_case(&self) -> HookEventCase {
        HookEventCase::PascalCase
    }
    // T2 审批事件注册扩展（issue #74 / 调研 §2，2026-09-20）：
    // - `PermissionRequest`：即时审批信号（弹窗前触发）；exit 2 对该事件**不生效**
    //   （官方明文 "Exit code 2 isn't honored"），helper exit 0 + 空输出 = decline
    //   to decide，审批流原样继续；
    // - `Notification`：审批等待第二信号（permission_prompt 类型，比弹窗晚约 6 秒，
    //   先被应答则不触发；message 字段携带提示文案——判定②文本源）。注册带
    //   matcher="permission_prompt"（hook_event_matcher，官方形态逐字段同构）；
    // - `PostToolUseFailure`：退出清除链补齐（PostToolUse / PostToolUseFailure →
    //   Stop（兜底）→ UserPromptSubmit，T3 持久等待标记的清除信号集）。
    // 其余四事件（Stop/UserPromptSubmit/SessionStart/SessionEnd）+ PreToolUse/
    // PostToolUse 为既有六事件，形态不动。
    fn hook_events(&self) -> Vec<&'static str> {
        vec![
            "Stop",
            "UserPromptSubmit",
            "SessionStart",
            "SessionEnd",
            "PreToolUse",
            "PostToolUse",
            "PostToolUseFailure",
            "PermissionRequest",
            "Notification",
        ]
    }
    fn hook_event_matcher(&self, event: &str) -> Option<&'static str> {
        match event {
            // 仅按通知类型过滤审批等待通知（permission_prompt）；空/`*` matcher
            // 会把 idle_prompt 等无关通知也写进事件文件（高频噪音），不采用
            "Notification" => Some("permission_prompt"),
            _ => None,
        }
    }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("settings.json"))
    }

    fn mcp_format(&self) -> McpFormat {
        McpFormat::Json
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(dirs::home_dir().unwrap_or_default().join(".claude.json"))
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
        vec![self.base_dir().join("settings.json")]
    }
}
