// Codex CLI + 桌面 APP adapter

use super::*;
use crate::monitor;

pub struct CodexAdapter;

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "Codex CLI"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::Codex
    }
    fn process_names(&self) -> &'static [&'static str] {
        &["codex", "Codex"]
    }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_codex_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::codex_parser::get_codex_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir().unwrap_or_default().join(".codex")
    }

    fn hook_supported(&self) -> bool {
        true
    }
    // F3（存量 bug 修复，2026-09-20）：codex 0.155.x 的 hooks 配置键为 **PascalCase**
    // （codex-rs/config/src/hook_config.rs serde rename 逐字段核对；payload
    // hook_event_name 亦 PascalCase，与 claude 一致）——旧 CamelCase 注册写入
    // "stop"/"userPromptSubmit" 等键，codex 不识别 → **hooks 链路整体不生效**
    // （疑为 hooks 失效真根因）。证据：research/refs/phase2-消息注入/
    // 2026-09-19-审批事件钩子通道调研.md §3.2/§3.3。读侧 mod.rs 消费 "Stop"|"stop"
    // 双口径，事件名归 PascalCase 后与 claude 同形态，读侧零改动。
    // 存量 hooks.json 重写语义：register_all_hooks 每次启动重跑（lib.rs），旧
    // camelCase 键由 register_hooks_for_tool 的 F3 迁移步移除（仅 MAM 自写条目）。
    fn hook_event_case(&self) -> HookEventCase {
        HookEventCase::PascalCase
    }
    fn hook_events(&self) -> Vec<&'static str> {
        vec![
            "Stop",
            "UserPromptSubmit",
            "SessionStart",
            "SessionEnd",
            "PreToolUse",
            "PostToolUse",
        ]
    }
    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("hooks.json"))
    }

    fn mcp_format(&self) -> McpFormat {
        McpFormat::Toml
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("config.toml"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        // MAM 激活目标为 codex 私有目录 ~/.codex/skills（spec 2026-09-09 §4.1；
        // ~/.agents/skills 已降级为只读共享导入源，MAM 不再写入）
        super::primary_skill_dir("codex")
            .map(|dir| vec![dir])
            .unwrap_or_else(|| vec![self.base_dir().join("skills")])
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
