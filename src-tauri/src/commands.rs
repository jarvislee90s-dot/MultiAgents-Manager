// Tauri IPC 命令 — 前端通过 invoke("...") 调用

use crate::adapter;
use crate::session::SessionsResponse;

/// 获取所有活跃会话（看板轮询调用）+ 更新托盘聚合状态
#[tauri::command]
pub fn get_all_sessions(app: tauri::AppHandle) -> SessionsResponse {
    let response = adapter::get_all_sessions();
    let has_processing = response.sessions.iter().any(|s| {
        matches!(s.status, crate::session::SessionStatus::Processing
            | crate::session::SessionStatus::Thinking
            | crate::session::SessionStatus::Compacting)
    });
    crate::plugins::system_tray::update_tray_status(
        &app, response.waiting_count, response.total_count, has_processing,
    );
    // 仅在预设组数量变化时重建托盘菜单（避免每 1.5s 重建）
    let preset_count = crate::store::list_presets().len();
    let last_count = crate::store::get_setting("last_preset_count").and_then(|s| s.parse::<usize>().ok()).unwrap_or(usize::MAX);
    if preset_count != last_count {
        let _ = crate::plugins::system_tray::update_tray_with_presets(&app);
        crate::store::set_setting("last_preset_count", &preset_count.to_string());
    }
    response
}

/// 跳转到指定会话的终端窗口（US3）
#[tauri::command]
pub fn focus_session(pid: u32) -> Result<(), String> {
    crate::terminal::focus_terminal_for_pid(pid)
}

/// 读取设置
#[tauri::command]
pub fn get_setting(key: String) -> Option<String> {
    crate::store::get_setting(&key)
}

/// 写入设置
#[tauri::command]
pub fn set_setting(key: String, value: String) {
    crate::store::set_setting(&key, &value);
}


/// 扩展 + 分配组合
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionWithAssignments {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub source_path: String,
    pub suite: Option<String>,
    pub source_tool: Option<String>,
    pub assignments: Vec<AssignmentSummary>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentSummary {
    pub agent_tool_id: String,
    pub enabled: bool,
    pub link_status: String,
}

/// 列出所有扩展及其分配状态
#[tauri::command]
pub fn list_extensions_with_assignments() -> Vec<ExtensionWithAssignments> {
    let extensions = crate::store::list_extensions();
    let assignments = crate::store::list_all_assignments();

    extensions.iter().map(|ext| {
        let ext_assignments: Vec<AssignmentSummary> = assignments.iter()
            .filter(|a| a.extension_id == ext.id)
            .map(|a| AssignmentSummary {
                agent_tool_id: a.agent_tool_id.clone(),
                enabled: a.enabled,
                link_status: a.link_status.clone(),
            })
            .collect();
        ExtensionWithAssignments {
            id: ext.id.clone(),
            kind: ext.kind.clone(),
            name: ext.name.clone(),
            description: ext.description.clone(),
            source_path: ext.source_path.clone(),
            suite: ext.suite.clone(),
            source_tool: ext.source_tool.clone(),
            assignments: ext_assignments,
        }
    }).collect()
}

/// 列出全局仓库中的 skill 名称
#[tauri::command]
pub fn list_repo_skills() -> Vec<String> {
    crate::linker::list_repo_skills()
}

/// 安装 skill 到全局仓库
#[tauri::command]
pub fn install_skill(source_path: String, name: String) -> Result<(), String> {
    crate::manager::install_skill(&source_path, &name)
}

/// 为工具启用/禁用 MCP 服务器
#[tauri::command]
pub fn toggle_mcp_for_tool(mcp_name: String, tool_id: String, enabled: bool) -> Result<(), String> {
    crate::manager::toggle_mcp(&mcp_name, &tool_id, enabled)
}

/// 为工具启用/禁用 Plugin
#[tauri::command]
pub fn toggle_plugin_for_tool(plugin_name: String, tool_id: String, enabled: bool, kind: String) -> Result<(), String> {
    crate::manager::plugin::toggle_plugin(&plugin_name, &tool_id, enabled, &kind)
}

/// 应用预设组到子 Agent
#[tauri::command]
pub fn apply_preset_to_subagent(preset_id: String, tool_id: String, sub_agent_id: String) -> PresetApplyResult {
    let result = crate::manager::preset::apply_preset_to_subagent(&preset_id, &tool_id, &sub_agent_id);
    PresetApplyResult { success_count: result.success, failures: result.failures, conflicts: result.conflicts }
}

/// 取消激活子 Agent 级预设组
#[tauri::command]
pub fn deactivate_preset_from_subagent(preset_id: String, tool_id: String, sub_agent_id: String) -> Result<(), String> {
    crate::manager::preset::deactivate_preset_from_subagent(&preset_id, &tool_id, &sub_agent_id)
}




/// 创建预设组
#[tauri::command]
pub fn create_preset(name: String, items: Vec<(String, String)>) -> Result<String, String> {
    crate::store::create_preset(&name, &items)
}

/// 删除预设组
#[tauri::command]
pub fn delete_preset(preset_id: String) -> Result<(), String> {
    crate::store::delete_preset(&preset_id)
}

/// 应用预设组到工具
use crate::store::PresetRecord;

/// 预设组应用结果
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetApplyResult {
    pub success_count: usize,
    pub failures: Vec<String>,
    pub conflicts: Vec<String>,
}

/// 列出所有预设组
#[tauri::command]
pub fn list_presets() -> Vec<PresetRecord> {
    crate::store::list_presets()
}

/// 应用预设组到工具
#[tauri::command]
pub fn apply_preset(preset_id: String, tool_id: String) -> PresetApplyResult {
    let result = crate::manager::preset::apply_preset(&preset_id, &tool_id);
    PresetApplyResult { success_count: result.success, failures: result.failures, conflicts: result.conflicts }
}

/// 取消激活预设组
#[tauri::command]
pub fn deactivate_preset(preset_id: String, tool_id: String) -> Result<(), String> {
    crate::manager::preset::deactivate_preset(&preset_id, &tool_id)
}

/// 检测工具的子 Agent 列表
#[tauri::command]
pub fn detect_subagents(tool_id: String) -> Vec<String> {
    crate::manager::detect_subagents(&tool_id)
}

use crate::store::SubAgentRecord;

/// 终止会话进程
#[tauri::command]
pub fn kill_session(pid: u32) -> Result<(), String> {
    use sysinfo::{Pid, Signal};
    if let Some(process) = sysinfo::System::new_all().process(Pid::from_u32(pid)) {
        process.kill_with(Signal::Term);
        Ok(())
    } else {
        Err(format!("进程 {} 不存在", pid))
    }
}

/// 列出工具的子 Agent
#[tauri::command]
pub fn list_sub_agents(tool_id: String) -> Vec<SubAgentRecord> {
    crate::store::list_sub_agents(&tool_id)
}

/// 读取工具的 MCP 服务器列表
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
    match adapter.mcp_format() {
        crate::adapter::McpFormat::Json | crate::adapter::McpFormat::Jsonc => {
            serde_json::from_str(&content).map_err(|e| e.to_string())
        }
        crate::adapter::McpFormat::Toml => {
            let toml_val: toml::Value = content.parse().map_err(|e: toml::de::Error| e.to_string())?;
            let json_str = serde_json::to_string(&toml_val).map_err(|e| e.to_string())?;
            serde_json::from_str(&json_str).map_err(|e| e.to_string())
        }
    }
}

/// 写入单个 MCP 服务器配置到工具
#[tauri::command]
pub fn write_mcp_server(tool_id: String, mcp_name: String, command: String, args: Vec<String>, env: std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let config = crate::manager::mcp::McpConfig { command, args, env };
    crate::manager::mcp::write_mcp(&tool_id, &mcp_name, &config)
}

/// 移除单个 MCP 服务器配置
#[tauri::command]
pub fn remove_mcp_server(tool_id: String, mcp_name: String) -> Result<(), String> {
    crate::manager::mcp::remove_mcp(&tool_id, &mcp_name)
}

/// 检测已安装的工具
#[tauri::command]
pub fn detect_tools() -> Vec<crate::linker::detector::ToolDetection> {
    crate::linker::detector::detect_all_tools()
}

/// 为子 Agent 分配 skill
#[tauri::command]
pub fn assign_skill_to_subagent(skill_name: String, tool_id: String, sub_agent_id: String) -> Result<(), String> {
    crate::manager::assign_skill_to_subagent(&skill_name, &tool_id, &sub_agent_id)
}

/// 重新扫描各工具的 skill 目录，导入新增的 skill（前端"重新扫描"按钮调用）
#[tauri::command]
pub fn rescan_skills() -> crate::manager::ImportStats {
    crate::manager::auto_import_extensions(true)
}
