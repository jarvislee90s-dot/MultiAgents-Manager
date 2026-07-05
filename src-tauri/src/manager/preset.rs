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
        "mcp" => None, // MCP 是配置型写入，重复存在不视为错误；write_mcp_toml/json 会用同名 key 覆盖
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
