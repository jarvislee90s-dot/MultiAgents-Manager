// MCP 管理命令

use tauri::{Builder, Runtime};

#[tauri::command]
pub fn toggle_mcp_for_tool(mcp_name: String, tool_id: String, enabled: bool) -> Result<(), String> {
    crate::services::toggle_mcp(&mcp_name, &tool_id, enabled)
}

#[tauri::command]
pub fn read_mcp_servers(tool_id: String) -> Result<serde_json::Value, String> {
    use crate::adapter::{AgentAdapter, claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter};
    let adapter: Box<dyn AgentAdapter> = match tool_id.as_str() {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };
    let path = adapter.mcp_config_path().ok_or("工具不支持 MCP")?;
    let content = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let raw: serde_json::Value = match adapter.mcp_format() {
        crate::adapter::McpFormat::Json | crate::adapter::McpFormat::Jsonc => {
            serde_json::from_str(&content).map_err(|e| e.to_string())?
        }
        crate::adapter::McpFormat::Toml => {
            let toml_val: toml::Value = content.parse().map_err(|e: toml::de::Error| e.to_string())?;
            let json_str = serde_json::to_string(&toml_val).map_err(|e| e.to_string())?;
            serde_json::from_str(&json_str).map_err(|e| e.to_string())?
        }
    };
    let servers = raw.get("mcpServers")
        .or_else(|| raw.get("mcp_servers"))
        .or_else(|| raw.get("mcp"))
        .or_else(|| raw.get("servers"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(serde_json::json!({ "servers": servers }))
}

#[tauri::command]
pub fn write_mcp_server(tool_id: String, mcp_name: String, command: String, args: Vec<String>, env: std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let config = crate::services::mcp::McpConfig { command, args, env };
    crate::services::mcp::write_mcp(&tool_id, &mcp_name, &config)
}

#[tauri::command]
pub fn remove_mcp_server(tool_id: String, mcp_name: String) -> Result<(), String> {
    crate::services::mcp::remove_mcp(&tool_id, &mcp_name)
}

pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        toggle_mcp_for_tool, read_mcp_servers, write_mcp_server, remove_mcp_server
    ])
}
