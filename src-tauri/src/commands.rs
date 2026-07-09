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
    let preset_count = crate::database::list_presets().len();
    let last_count = crate::database::get_setting("last_preset_count").and_then(|s| s.parse::<usize>().ok()).unwrap_or(usize::MAX);
    if preset_count != last_count {
        let _ = crate::plugins::system_tray::update_tray_with_presets(&app);
        crate::database::set_setting("last_preset_count", &preset_count.to_string());
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
    crate::database::get_setting(&key)
}

/// 写入设置
#[tauri::command]
pub fn set_setting(key: String, value: String) {
    crate::database::set_setting(&key, &value);
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
    pub tags: Option<String>,
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
    let extensions = crate::database::list_extensions();
    let assignments = crate::database::list_all_assignments();

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
            tags: ext.tags.clone(),
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
    crate::database::create_preset(&name, &items)
}

/// 删除预设组
#[tauri::command]
pub fn delete_preset(preset_id: String) -> Result<(), String> {
    crate::database::delete_preset(&preset_id)
}

/// 应用预设组到工具
use crate::database::PresetRecord;

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
    crate::database::list_presets()
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

use crate::database::SubAgentRecord;

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
    crate::database::list_sub_agents(&tool_id)
}

/// 读取工具的 MCP 服务器列表
/// 返回统一格式: { "servers": { name: { command, args, env }, ... } }
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

    // 统一提取 servers 对象，支持多种键名
    let servers = raw.get("mcpServers")
        .or_else(|| raw.get("mcp_servers"))
        .or_else(|| raw.get("mcp"))
        .or_else(|| raw.get("servers"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(serde_json::json!({ "servers": servers }))
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

/// 扫描指定工具的原生资源（尚未导入全局仓库）
#[tauri::command]
pub fn scan_native_resources(tool_id: String) -> Vec<crate::database::NativeExtensionRecord> {
    let mut results = Vec::new();

    // 扫描工具的 skill 目录
    let skill_dir = match tool_id.as_str() {
        "claude" => Some(dirs::home_dir().unwrap_or_default().join(".claude").join("skills")),
        "codex" => Some(dirs::home_dir().unwrap_or_default().join(".codex").join("skills")),
        "opencode" => Some(dirs::home_dir().unwrap_or_default().join(".config").join("opencode").join("skills")),
        "openclaw" => Some(dirs::home_dir().unwrap_or_default().join(".openclaw").join("skills")),
        _ => None,
    };

    if let Some(dir) = skill_dir {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let existing = crate::database::list_extensions();
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        // 检查是否已在全局仓库中
                        let ext_id = format!("skill-{}", name);
                        let exists = existing.iter().any(|e| e.id == ext_id);
                        if !exists {
                            results.push(crate::database::NativeExtensionRecord {
                                id: ext_id,
                                kind: "skill".to_string(),
                                name: name.clone(),
                                description: None,
                                source_path: path.to_string_lossy().to_string(),
                                source_tool: tool_id.clone(),
                                detected_at: chrono::Utc::now().to_rfc3339(),
                                imported: false,
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

/// 将原生资源导入全局仓库
#[tauri::command]
pub fn import_native_resources(items: Vec<(String, String)>) -> crate::manager::ImportStats {
    let mut imported = 0;
    let mut skipped = 0;

    for (source_path, name) in items {
        let path = std::path::Path::new(&source_path);
        if !path.exists() {
            skipped += 1;
            continue;
        }

        // 复制到全局仓库
        if let Err(e) = crate::linker::install_to_repo(path, &name) {
            log::warn!("导入 {} 失败: {}", name, e);
            skipped += 1;
            continue;
        }

        // 记录到数据库
        let ext = crate::database::ExtensionRecord {
            id: format!("skill-{}", name),
            kind: "skill".to_string(),
            name: name.clone(),
            description: None,
            source_path: source_path.clone(),
            source_url: None,
            version: None,
            tags: None,
            suite: None,
            source_tool: None,
            is_native: true,
        };
        let _ = crate::database::insert_extension(&ext);
        imported += 1;
    }

    crate::manager::ImportStats {
        imported,
        newly_added: imported,
        skipped_dup: skipped,
        source_counts: vec![],
    }
}

/// 获取工具的所有资源（全局 + 原生）
#[tauri::command]
pub fn list_tool_resources(tool_id: String) -> serde_json::Value {
    let global = crate::database::list_extensions();
    let native = scan_native_resources(tool_id.clone());

    // 过滤出分配给该工具且启用的全局资源
    let assignments = crate::database::list_assignments(&tool_id);
    let global_filtered: Vec<_> = global.iter()
        .filter(|e| {
            assignments.iter().any(|a| a.extension_id == e.id && a.enabled)
        })
        .collect();

    serde_json::json!({
        "global": global_filtered,
        "native": native,
    })
}

/// 检查预设组与工具的兼容性
#[tauri::command]
pub fn check_preset_compatibility(preset_id: String, tool_id: String) -> crate::manager::preset::CompatibilityReport {
    crate::manager::preset::check_compatibility(&preset_id, &tool_id)
}

// ===== 截图功能 =====

/// 截图结果
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotResult {
    pub success: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

/// 捕获应用窗口截图（macOS）
#[tauri::command]
pub fn capture_window_screenshot(app: tauri::AppHandle) -> ScreenshotResult {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        // 生成截图文件路径
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let screenshot_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".mam")
            .join("screenshots");

        if let Err(e) = std::fs::create_dir_all(&screenshot_dir) {
            return ScreenshotResult {
                success: false,
                path: None,
                error: Some(format!("创建截图目录失败: {}", e)),
            };
        }

        let screenshot_path = screenshot_dir.join(format!("screenshot_{}.png", timestamp));
        let path_str = screenshot_path.to_string_lossy().to_string();

        // 使用 screencapture 命令截图
        // -x: 不播放截图声音
        // -o: 不包含窗口阴影
        // -W: 截取窗口（需要用户选择）
        // 或者使用 -l <window_id> 指定窗口

        // 先尝试通过窗口标题找到应用窗口
        let window_id_output = Command::new("sh")
            .args([
                "-c",
                "osascript -e 'tell application \"System Events\" to get id of first process whose name contains \"multi-agents-manager\"' 2>/dev/null || echo ''"
            ])
            .output();

        let capture_result = if let Ok(output) = window_id_output {
            let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !window_id.is_empty() && window_id.parse::<u32>().is_ok() {
                // 使用窗口 ID 截图
                Command::new("screencapture")
                    .args(["-x", "-o", "-l", &window_id, &path_str])
                    .output()
            } else {
                // 回退到全屏截图
                Command::new("screencapture")
                    .args(["-x", "-o", &path_str])
                    .output()
            }
        } else {
            // 回退到全屏截图
            Command::new("screencapture")
                .args(["-x", "-o", &path_str])
                .output()
        };

        match capture_result {
            Ok(output) if output.status.success() => {
                log::info!("截图已保存到: {}", path_str);
                ScreenshotResult {
                    success: true,
                    path: Some(path_str),
                    error: None,
                }
            }
            Ok(output) => {
                let error_msg = String::from_utf8_lossy(&output.stderr).to_string();
                log::error!("截图失败: {}", error_msg);
                ScreenshotResult {
                    success: false,
                    path: None,
                    error: Some(format!("截图命令失败: {}", error_msg)),
                }
            }
            Err(e) => {
                log::error!("截图命令执行失败: {}", e);
                ScreenshotResult {
                    success: false,
                    path: None,
                    error: Some(format!("截图命令执行失败: {}", e)),
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        ScreenshotResult {
            success: false,
            path: None,
            error: Some("截图功能目前仅支持 macOS".to_string()),
        }
    }
}

/// 获取最近的截图文件列表
#[tauri::command]
pub fn list_screenshots() -> Vec<String> {
    let screenshot_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("screenshots");

    if !screenshot_dir.exists() {
        return Vec::new();
    }

    let mut paths: Vec<String> = std::fs::read_dir(&screenshot_dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        == Some("png")
                })
                .map(|e| e.path().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();

    // 按修改时间排序（最新的在前）
    paths.sort_by(|a, b| {
        let meta_a = std::fs::metadata(a).ok();
        let meta_b = std::fs::metadata(b).ok();
        match (meta_a, meta_b) {
            (Some(a), Some(b)) => b.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH).cmp(
                &a.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            ),
            _ => std::cmp::Ordering::Equal,
        }
    });

    paths
}
