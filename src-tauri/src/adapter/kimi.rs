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

    // T2 hooks 通道接入（issue #74 / 官方 hooks 文档，2026-09-20）：config.toml
    // `[[hooks]]`（字段仅 event/matcher/command/timeout 四个，无 type/commandWindows
    // /async）；事件名 PascalCase。注册 `PermissionRequest`（审批等待前触发——判定①
    // Waiting 信号）+ `PermissionResult`（审批完成——T3 持久等待标记的清除信号）。
    // kimi 此前无 hook 通道（hook_supported 恒 false）→ **无存量迁移面**；注册由
    // monitor::hooks::register_all_hooks 的 TOML 分支承载（register_kimi_hooks_in_file）。
    // 官方文档锚点：kimi.com/code/docs .../customization/hooks.html（stdin JSON 含
    // hook_event_name/session_id/cwd，helper 薄管道零改动兼容；exit 0 = allow 且
    // stdout 会进上下文——helper 空输出不污染上下文，exit 2 = block 但 helper 恒 0）
    fn hook_supported(&self) -> bool {
        true
    }

    fn hook_event_case(&self) -> HookEventCase {
        HookEventCase::PascalCase
    }

    fn hook_events(&self) -> Vec<&'static str> {
        vec!["PermissionRequest", "PermissionResult"]
    }

    fn hook_event_matcher(&self, _event: &str) -> Option<&'static str> {
        // 官方 [[hooks]] matcher 为可选正则（按目标过滤）；审批事件无需过滤 → 不带
        None
    }

    fn hook_config_path(&self) -> Option<std::path::PathBuf> {
        // 官方文档：hooks 以 `[[hooks]]` 数组表落在 $KIMI_CODE_HOME/config.toml
        // （与 MCP 的 mcp.json 分离——config.toml 本身是 TOML 但不放 MCP 配置）
        Some(monitor::kimi_parser::kimi_home().join("config.toml"))
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
        vec![monitor::kimi_parser::kimi_home()
            .join("plugins")
            .join("managed")]
    }
    fn plugin_config_paths(&self) -> Vec<std::path::PathBuf> {
        // 插件为 manifest 目录型，非配置段型，不支持 config 型插件写入
        vec![]
    }
}
