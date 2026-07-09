// Services 统一入口 - 业务逻辑按功能域拆分到子目录

pub mod skill;
pub mod resource;
pub mod preset;
pub mod mcp;
pub mod plugin;
pub mod manifest;

use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, openclaw::OpenClawAdapter, AgentAdapter};
use log::info;

// 重新导出 skill 函数（保持 crate::services::install_skill 向后兼容）
pub use skill::{install_skill, enable_skill_for_tool, disable_skill_for_tool, is_skill_in_tool_range, assign_skill_to_subagent};
// 重新导出 resource 函数
pub use resource::{auto_import_extensions, ImportStats};

/// 为工具启用/禁用 Plugin（委托到 plugin 子模块）
pub fn toggle_plugin(plugin_name: &str, tool_id: &str, enabled: bool, kind: &str) -> Result<(), String> {
    plugin::toggle_plugin(plugin_name, tool_id, enabled, kind)
}

/// 为工具启用/禁用 MCP（委托到 mcp 子模块）
pub fn toggle_mcp(mcp_name: &str, tool_id: &str, enabled: bool) -> Result<(), String> {
    if enabled {
        let repo = dirs::home_dir().unwrap_or_default().join(".mam").join("mcp");
        let config_path = repo.join(format!("{}.json", mcp_name));
        if !config_path.exists() {
            return Err(format!("MCP 配置不在全局仓库中: {}", mcp_name));
        }
        let content = std::fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
        let config: mcp::McpConfig = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        mcp::write_mcp(tool_id, mcp_name, &config)?;
    } else {
        mcp::remove_mcp(tool_id, mcp_name)?;
    }
    let ext_id = format!("mcp-{}", mcp_name);
    crate::database::upsert_assignment(&ext_id, tool_id, enabled, if enabled { "valid" } else { "missing" })?;
    info!("MCP {} 已为 {} {}", mcp_name, tool_id, if enabled { "启用" } else { "禁用" });
    Ok(())
}

/// 检测工具的子 Agent 列表
pub fn detect_subagents(tool_id: &str) -> Vec<String> {
    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        "openclaw" => Box::new(OpenClawAdapter),
        _ => return Vec::new(),
    };
    if let Some(dir) = adapter.subagent_dir() {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                return entries.flatten()
                    .filter_map(|e| {
                        e.path().file_stem()
                            .and_then(|s| s.to_str())
                            .map(String::from)
                    })
                    .collect();
            }
        }
    }
    Vec::new()
}
