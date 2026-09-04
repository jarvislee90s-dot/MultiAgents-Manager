// WorkBuddy（腾讯全场景 AI 办公工作台，Electron APP）适配器
// 会话运行时 = APP 内嵌 cli/bin/codebuddy；状态提取见 monitor/workbuddy_parser.rs

use super::*;
use crate::monitor;

pub struct WorkBuddyAdapter;

impl AgentAdapter for WorkBuddyAdapter {
    fn name(&self) -> &'static str {
        "WorkBuddy"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::WorkBuddy
    }
    fn process_names(&self) -> &'static [&'static str] {
        // APP 内嵌运行时进程；独立安装的腾讯 CodeBuddy CLI 同名进程
        // 无 ~/.workbuddy 心跳，由解析器天然排除
        &["codebuddy"]
    }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_workbuddy_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::workbuddy_parser::get_workbuddy_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".workbuddy")
    }

    fn mcp_format(&self) -> McpFormat {
        // MCP 服务器声明在 ~/.workbuddy/mcp.json（JSON，mcpServers 段）
        McpFormat::Json
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("mcp.json"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        super::primary_skill_dir("workbuddy")
            .map(|dir| vec![dir])
            .unwrap_or_else(|| vec![self.base_dir().join("skills")])
    }

    // WorkBuddy 插件为市场化版本化管理，不纳入 MAM（spec W3）；无 hook 机制、
    // 无独立子 agent 目录 —— 均沿用 trait 默认实现（空 / false / None）
}
