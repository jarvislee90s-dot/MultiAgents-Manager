// MCP 配置格式转换器 — JSON (Claude) / TOML (Codex) / JSONC (OpenCode)

use crate::adapter::{McpFormat, claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter, openclaw::OpenClawAdapter, AgentAdapter};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// MCP 内部统一格式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// 写入 MCP 配置到工具
pub fn write_mcp(tool_id: &str, mcp_name: &str, config: &McpConfig) -> Result<(), String> {
    let (format, config_path) = get_tool_mcp_info(tool_id)?;
    match format {
        McpFormat::Json => write_mcp_json(&config_path, mcp_name, config),
        McpFormat::Toml => write_mcp_toml(&config_path, mcp_name, config),
        McpFormat::Jsonc => write_mcp_jsonc(&config_path, mcp_name, config),
    }
}

/// 移除 MCP 配置
pub fn remove_mcp(tool_id: &str, mcp_name: &str) -> Result<(), String> {
    let (format, config_path) = get_tool_mcp_info(tool_id)?;
    match format {
        McpFormat::Json => remove_mcp_json(&config_path, mcp_name),
        McpFormat::Toml => remove_mcp_toml(&config_path, mcp_name),
        McpFormat::Jsonc => remove_mcp_jsonc(&config_path, mcp_name),
    }
}

fn get_tool_mcp_info(tool_id: &str) -> Result<(McpFormat, std::path::PathBuf), String> {
    let adapter: Box<dyn AgentAdapter> = match tool_id {
        "claude" => Box::new(ClaudeAdapter),
        "codex" => Box::new(CodexAdapter),
        "opencode" => Box::new(OpenCodeAdapter),
        "openclaw" => Box::new(OpenClawAdapter),
        _ => return Err(format!("未知工具: {}", tool_id)),
    };
    let path = adapter.mcp_config_path().ok_or("工具不支持 MCP 配置")?;
    Ok((adapter.mcp_format(), path))
}

// ===== JSON (Claude Code: ~/.claude.json mcpServers) =====

fn write_mcp_json(path: &std::path::Path, name: &str, config: &McpConfig) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSON 配置失败: {}", e))?;
    if root.get("mcpServers").is_none() {
        root["mcpServers"] = serde_json::json!({});
    }
    root["mcpServers"][name] = serde_json::json!({
        "command": config.command,
        "args": config.args,
        "env": config.env,
    });
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    crate::linker::write_config_locked(path, &pretty)
}

fn remove_mcp_json(path: &std::path::Path, name: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSON 配置失败: {}", e))?;
    if let Some(servers) = root.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
        servers.remove(name);
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    crate::linker::write_config_locked(path, &pretty)
}

// ===== TOML (Codex CLI: config.toml mcp_servers) =====

fn write_mcp_toml(path: &std::path::Path, name: &str, config: &McpConfig) -> Result<(), String> {
    // 使用 toml_edit 保留原文件注释和格式
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = content.parse()
        .map_err(|e| format!("解析 TOML 失败: {}", e))?;

    // 确保 mcp_servers 段存在
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // 写入服务器配置（覆盖同名）
    {
        let server = &mut doc["mcp_servers"][name];
        server["command"] = toml_edit::value(&config.command);
        let args_array: toml_edit::Array = config.args.iter()
            .map(|a| toml_edit::Value::String(toml_edit::Formatted::new(a.clone())))
            .collect();
        server["args"] = toml_edit::Item::Value(toml_edit::Value::Array(args_array));
        if !config.env.is_empty() {
            let mut env_table = toml_edit::Table::new();
            for (k, v) in &config.env {
                env_table.insert(k, toml_edit::value(v));
            }
            server["env"] = toml_edit::Item::Table(env_table);
        }
    }

    let toml_str = doc.to_string();
    crate::linker::write_config_locked(path, &toml_str)
}

fn remove_mcp_toml(path: &std::path::Path, name: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = content.parse()
        .map_err(|e| format!("解析 TOML 失败: {}", e))?;
    if let Some(servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_mut()) {
        servers.remove(name);
    }
    let toml_str = doc.to_string();
    crate::linker::write_config_locked(path, &toml_str)
}

// ===== JSONC (OpenCode: opencode.json mcp) =====

fn write_mcp_jsonc(path: &std::path::Path, name: &str, config: &McpConfig) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSONC 配置失败: {}", e))?;
    if root.get("mcp").is_none() {
        root["mcp"] = serde_json::json!({});
    }
    // OpenCode 格式：command 是数组，env 是 environment
    let mut cmd_array = vec![config.command.clone()];
    cmd_array.extend(config.args.iter().cloned());
    root["mcp"][name] = serde_json::json!({
        "type": "local",
        "command": cmd_array,
        "environment": config.env,
    });
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    crate::linker::write_config_locked(path, &pretty)
}

fn remove_mcp_jsonc(path: &std::path::Path, name: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 JSONC 配置失败: {}", e))?;
    if let Some(servers) = root.get_mut("mcp").and_then(|s| s.as_object_mut()) {
        servers.remove(name);
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    crate::linker::write_config_locked(path, &pretty)
}
