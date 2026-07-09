// Hook 系统 — 事件注册 + 共享脚本 + 事件文件读取
// Claude Code: settings.json (PascalCase) / Codex CLI: hooks.json (camelCase)

use log::{debug, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Hook 脚本内容（从 stdin 读 JSON，写入事件文件）
const HOOK_SCRIPT: &str = r#"#!/bin/bash
# MultiAgents Manager 状态 Hook 脚本
# 从 stdin 读取 JSON，写入 ~/.mam/events/<ppid>.json
EVENTS_DIR="$HOME/.mam/events"
mkdir -p "$EVENTS_DIR"
INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
SESSION_ID=$(echo "$INPUT" | grep -o '"session_id"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
CWD=$(echo "$INPUT" | grep -o '"cwd"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
TS=$(date +%s)
LAST_EVENT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo "{\"event\":\"$EVENT\",\"session_id\":\"$SESSION_ID\",\"cwd\":\"$CWD\",\"ts\":$TS,\"last_event_at\":\"$LAST_EVENT_AT\"}" > "$EVENTS_DIR/$PPID.json"
"#;

/// 确保 Hook 脚本和事件目录存在
pub fn ensure_hook_script() -> PathBuf {
    let mam_dir = dirs::home_dir().unwrap_or_default().join(".mam");
    let hooks_dir = mam_dir.join("hooks");
    let events_dir = mam_dir.join("events");
    let _ = fs::create_dir_all(&hooks_dir);
    let _ = fs::create_dir_all(&events_dir);

    let script_path = hooks_dir.join("status-hook.sh");
    if !script_path.exists() {
        let _ = fs::write(&script_path, HOOK_SCRIPT);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&script_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&script_path, perms);
            }
        }
        info!("Hook 脚本已创建: {:?}", script_path);
    }
    script_path
}

/// 为指定工具注册 Hook
/// adapter_name: 工具名称, config_path: 配置文件路径, events: 事件列表, event_case: 大小写格式
pub fn register_hooks_for_tool(
    config_path: &PathBuf,
    events: &[&str],
    is_pascal_case: bool,
) -> Result<(), String> {
    let script_path = ensure_hook_script();
    let script_path_str = script_path.to_string_lossy().to_string();

    // 读取现有配置（不存在则创建空对象）
    let existing = fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
    let mut config: serde_json::Value = serde_json::from_str(&existing)
        .map_err(|e| format!("解析配置文件失败: {}", e))?;

    // 确保 hooks 对象存在
    // 确保 hooks 对象存在
    if config.get("hooks").is_none() {
        config["hooks"] = serde_json::json!({});
    }
    let hooks = config.get_mut("hooks")
        .ok_or("hooks 字段不存在")?;
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or("hooks 字段不是对象")?;

    let mut added = 0;
    for &event in events {
        let event_name = if is_pascal_case {
            event.to_string()
        } else {
            // PascalCase → camelCase: 首字母小写
            let mut chars = event.chars();
            match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect::<String>(),
                None => continue,
            }
        };

        // 检查是否已注册（避免重复）
        if let Some(existing_arr) = hooks_obj.get(&event_name) {
            if let Some(arr) = existing_arr.as_array() {
                let already = arr.iter().any(|entry| {
                    entry.get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|hooks| hooks.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(|c| c.contains("status-hook.sh"))
                                .unwrap_or(false)
                        }))
                        .unwrap_or(false)
                });
                if already {
                    debug!("Hook 已注册: {}", event_name);
                    continue;
                }
            }
        }

        // 添加 Hook 条目
        let hook_entry = serde_json::json!([{
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": &script_path_str
            }]
        }]);
        hooks_obj.insert(event_name, hook_entry);
        added += 1;
    }

    if added > 0 {
        // 创建备份（防止写入失败导致配置丢失）
        if config_path.exists() {
            let backup = config_path.with_extension("json.bak");
            let _ = fs::copy(config_path, &backup);
        }
        let pretty = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("序列化配置失败: {}", e))?;
        crate::linker::write_config_locked(config_path, &pretty)
            .map_err(|e| format!("写入配置文件失败: {}", e))?;
        info!("已注册 {} 个 Hook 到 {:?}", added, config_path);
    }

    Ok(())
}

/// 读取所有 Hook 事件文件，返回 PPID → 事件数据的映射
pub fn read_hook_events() -> HashMap<u32, HookEvent> {
    let mut events = HashMap::new();
    let events_dir = dirs::home_dir().unwrap_or_default().join(".mam").join("events");

    if !events_dir.exists() {
        return events;
    }

    if let Ok(entries) = fs::read_dir(&events_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if let Ok(ppid) = filename.trim_end_matches(".json").parse::<u32>() {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(event) = serde_json::from_str::<HookEvent>(&content) {
                            // 过滤过期事件（>30s）
                            let now = chrono::Utc::now().timestamp();
                            if now - event.ts < 30 {
                                events.insert(ppid, event);
                            }
                        }
                    }
                }
            }
        }
    }

    events
}

/// Hook 事件数据
#[derive(Debug, Deserialize)]
pub struct HookEvent {
    pub event: String,
    pub ts: i64,
    pub last_event_at: String,
}

/// 为所有支持 Hook 的工具注册 Hook（在应用启动时调用）
pub fn register_all_hooks() {
    // 检查是否已注册过（避免每次启动都读写用户配置）
    if let Some(val) = crate::database::get_setting("hooks_registered") {
        if val == "true" {
            debug!("Hooks already registered, skipping");
            // 仍确保脚本存在
            ensure_hook_script();
            return;
        }
    }
    use crate::adapter::{AgentAdapter, HookEventCase};
    use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter};

    let adapters: Vec<Box<dyn AgentAdapter>> = vec![
        Box::new(ClaudeAdapter),
        Box::new(CodexAdapter),
    ];

    for adapter in &adapters {
        if !adapter.hook_supported() {
            continue;
        }
        if let Some(config_path) = adapter.hook_config_path() {
            let events = adapter.hook_events();
            let is_pascal = matches!(adapter.hook_event_case(), HookEventCase::PascalCase);
            match register_hooks_for_tool(&config_path, &events, is_pascal) {
                Ok(()) => {
            info!("Hook 注册成功: {} → {:?}", adapter.name(), config_path);
            crate::database::set_setting("hooks_registered", "true");
        }
                Err(e) => warn!("Hook 注册失败 {} → {:?}: {}", adapter.name(), config_path, e),
            }
        }
    }
}
