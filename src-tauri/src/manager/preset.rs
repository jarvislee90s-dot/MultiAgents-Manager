// 预设组应用逻辑 — 增量应用 + 精确移除 + 部分成功处理

use crate::manager;
use crate::store;
use log::info;

/// 应用预设组到工具
/// 返回 (成功数, 失败消息列表) — 部分成功处理
pub struct ApplyResult {
    pub success: usize,
    pub failures: Vec<String>,
    pub conflicts: Vec<String>,
}

pub fn apply_preset(preset_id: &str, tool_id: &str) -> ApplyResult {
    let items = store::get_preset_items(preset_id);
    let mut success = 0;
    let mut failures = Vec::new();
    let mut conflicts = Vec::new();

    for (ext_id, kind) in &items {
        // 冲突检查：MCP 已存在则跳过，skill symlink 已 valid 则跳过
        let conflict_check = check_conflict(ext_id, kind, tool_id);
        if let Some(msg) = conflict_check {
            conflicts.push(msg);
            continue;
        }

        let result = match kind.as_str() {
            "skill" => {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                manager::enable_skill_for_tool(name, tool_id)
            }
            "mcp" => {
                let name = ext_id.strip_prefix("mcp-").unwrap_or(ext_id);
                manager::toggle_mcp(name, tool_id, true)
            }
            "plugin" => {
                let name = ext_id.strip_prefix("plugin-").unwrap_or(ext_id);
                // 从 extensions 表读取 plugin 的 tags 字段（存储了 "file" 或 "config" 子类型）
                let plugin_kind = crate::store::list_extensions()
                    .iter()
                    .find(|e| e.id == *ext_id)
                    .and_then(|e| e.tags.clone())
                    .unwrap_or_else(|| "file".to_string());
                crate::manager::plugin::toggle_plugin(name, tool_id, true, &plugin_kind)
            }
            _ => Err(format!("未知类型: {}", kind)),
        };
        match result {
            Ok(()) => success += 1,
            Err(e) => failures.push(format!("{}: {}", ext_id, e)),
        }
    }

    let _ = store::record_preset_application(preset_id, tool_id, true);
    info!("预设组 {} → {} — 成功 {} 失败 {} 冲突 {}", preset_id, tool_id, success, failures.len(), conflicts.len());
    ApplyResult { success, failures, conflicts }
}

/// 检查冲突：MCP 已存在或 skill symlink 已 valid 则跳过
fn check_conflict(ext_id: &str, kind: &str, tool_id: &str) -> Option<String> {
    match kind {
        "skill" => {
            let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
            let adapter: Box<dyn crate::adapter::AgentAdapter> = match tool_id {
                "claude" => Box::new(crate::adapter::claude::ClaudeAdapter),
                "codex" => Box::new(crate::adapter::codex::CodexAdapter),
                "opencode" => Box::new(crate::adapter::opencode::OpenCodeAdapter),
                _ => return None,
            };
            if let Some(dir) = adapter.skill_dirs().into_iter().next() {
                let target = dir.join(name);
                if target.exists() || target.is_symlink() {
                    return Some(format!("Skill {} 已存在，跳过", name));
                }
            }
            None
        }
        "mcp" => {
            let name = ext_id.strip_prefix("mcp-").unwrap_or(ext_id);
            let adapter: Box<dyn crate::adapter::AgentAdapter> = match tool_id {
                "claude" => Box::new(crate::adapter::claude::ClaudeAdapter),
                "codex" => Box::new(crate::adapter::codex::CodexAdapter),
                "opencode" => Box::new(crate::adapter::opencode::OpenCodeAdapter),
                _ => return None,
            };
            if let Some(config_path) = adapter.mcp_config_path() {
                if config_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        // Check if the MCP name already exists in the config
                        let has_conflict = match adapter.mcp_format() {
                            crate::adapter::McpFormat::Json | crate::adapter::McpFormat::Jsonc => {
                                if let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) {
                                    let servers = root.get("mcpServers").or_else(|| root.get("mcp"));
                                    servers.map(|s| s.get(name).is_some()).unwrap_or(false)
                                } else {
                                    false
                                }
                            }
                            crate::adapter::McpFormat::Toml => {
                                if let Ok(doc) = content.parse::<toml_edit::DocumentMut>() {
                                    doc.get("mcp_servers").and_then(|s| s.get(name)).is_some()
                                } else {
                                    false
                                }
                            }
                        };
                        if has_conflict {
                            return Some(format!("MCP {} 已在 {} 配置中存在", name, tool_id));
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// 取消激活预设组
pub fn deactivate_preset(preset_id: &str, tool_id: &str) -> Result<(), String> {
    let items = store::get_preset_items(preset_id);
    let mut errors = Vec::new();
    for (ext_id, kind) in &items {
        let result = match kind.as_str() {
            "skill" => {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                manager::disable_skill_for_tool(name, tool_id)
            }
            "mcp" => {
                let name = ext_id.strip_prefix("mcp-").unwrap_or(ext_id);
                manager::toggle_mcp(name, tool_id, false)
            }
            "plugin" => {
                let name = ext_id.strip_prefix("plugin-").unwrap_or(ext_id);
                let plugin_kind = crate::store::list_extensions()
                    .iter()
                    .find(|e| e.id == *ext_id)
                    .and_then(|e| e.tags.clone())
                    .unwrap_or_else(|| "file".to_string());
                crate::manager::plugin::toggle_plugin(name, tool_id, false, &plugin_kind)
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            errors.push(format!("{}: {}", ext_id, e));
        }
    }
    store::record_preset_application(preset_id, tool_id, false)?;
    if !errors.is_empty() {
        log::warn!("deactivate_preset 部分失败: {:?}", errors);
    }
    info!("预设组 {} 从 {} 取消激活", preset_id, tool_id);
    Ok(())
}

/// 应用预设组到子 Agent
pub fn apply_preset_to_subagent(preset_id: &str, tool_id: &str, sub_agent_id: &str) -> ApplyResult {
    let items = store::get_preset_items(preset_id);
    let mut success = 0;
    let mut failures = Vec::new();
    let mut conflicts = Vec::new();

    for (ext_id, kind) in &items {
        // 子 Agent 级只支持 skill（MCP 和 Plugin 是工具级配置）
        if kind != "skill" {
            conflicts.push(format!("{} 类型 {} 不支持子 Agent 级分配", ext_id, kind));
            continue;
        }

        // 检查是否在工具级范围内
        let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
        if !crate::manager::is_skill_in_tool_range(name, tool_id) {
            failures.push(format!("{}: 该 skill 未在工具级启用", ext_id));
            continue;
        }

        let result = crate::manager::assign_skill_to_subagent(name, tool_id, sub_agent_id);
        match result {
            Ok(()) => success += 1,
            Err(e) => failures.push(format!("{}: {}", ext_id, e)),
        }
    }

    let _ = store::record_preset_application_subagent(preset_id, tool_id, sub_agent_id, true);
    info!("预设组 {} -> {}:{} -- 成功 {} 失败 {} 冲突 {}",
        preset_id, tool_id, sub_agent_id, success, failures.len(), conflicts.len());
    ApplyResult { success, failures, conflicts }
}

/// 取消激活子 Agent 级预设组
pub fn deactivate_preset_from_subagent(preset_id: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    let items = store::get_preset_items(preset_id);
    let mut errors = Vec::new();
    for (ext_id, kind) in &items {
        if kind != "skill" { continue; }
        let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
        let result = crate::linker::layer3::unlink_skill_from_layer3(name, tool_id, sub_agent_id);
        if let Err(e) = result {
            errors.push(format!("{}: {}", ext_id, e));
        }
        // 更新数据库记录
        let _ = crate::store::disable_subagent_assignment(ext_id, tool_id, sub_agent_id);
    }
    store::record_preset_application_subagent(preset_id, tool_id, sub_agent_id, false)?;
    if !errors.is_empty() {
        log::warn!("deactivate_preset_from_subagent 部分失败: {:?}", errors);
    }
    info!("预设组 {} 从 {}:{} 取消激活", preset_id, tool_id, sub_agent_id);
    Ok(())
}
