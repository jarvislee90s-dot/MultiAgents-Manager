// Kimi Code adapter — Moonshot 终端 Agent（主命令 kimi）
// 会话解析见 monitor::kimi_parser（session_index.jsonl 定位 + wire.jsonl 状态判定）
// 配置布局（官方文档）：config.toml（TOML）/ mcp.json（JSON，mcpServers 段）/
// skills/ / plugins/managed/，数据根目录可用 KIMI_CODE_HOME 重定向

use super::*;
use crate::monitor;

pub struct KimiAdapter;

impl AgentAdapter for KimiAdapter {
    fn name(&self) -> &'static str {
        "Kimi Code"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::Kimi
    }
    fn process_names(&self) -> &'static [&'static str] {
        // 主进程 kimi；Web Worker 子进程 "kimi-code-worker" basename 不同名不会误匹配，
        // 且父链过滤会剔除同工具子进程
        &["kimi"]
    }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::process::find_kimi_processes(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::kimi_parser::get_kimi_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        monitor::kimi_parser::kimi_home()
    }

    fn hook_supported(&self) -> bool {
        // Kimi 支持 [[hooks]]（config.toml，PascalCase 事件，stdin JSON 含
        // hook_event_name/session_id/cwd），但现有注册器只写 Claude 风格 JSON 配置，
        // TOML [[hooks]] 注册器作为后续扩展（见 IMPLEMENTATION_NOTES）；
        // 状态判定由 wire.jsonl 尾部解析承担，与 opencode/openclaw 同档
        false
    }

    fn mcp_format(&self) -> McpFormat {
        // 官方文档：MCP 服务器声明在 $KIMI_CODE_HOME/mcp.json（JSON，mcpServers 段），
        // 与 Claude Code 同构——注意 config.toml 本身是 TOML 但不放 MCP 配置
        McpFormat::Json
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(monitor::kimi_parser::kimi_home().join("mcp.json"))
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        vec![monitor::kimi_parser::kimi_home().join("skills")]
    }

    fn subagent_dir(&self) -> Option<std::path::PathBuf> {
        // Kimi 子 agent 为会话内 swarm（wire 中的 swarm_mode 事件），无独立子 agent 目录
        None
    }

    fn plugin_dirs(&self) -> Vec<std::path::PathBuf> {
        // 官方文档：本地插件安装到 $KIMI_CODE_HOME/plugins/managed/<id>/（kimi.plugin.json 清单）
        vec![
            monitor::kimi_parser::kimi_home()
                .join("plugins")
                .join("managed"),
        ]
    }
    fn plugin_config_paths(&self) -> Vec<std::path::PathBuf> {
        // 插件为 manifest 目录型，非配置段型，不支持 config 型插件写入
        vec![]
    }
}
