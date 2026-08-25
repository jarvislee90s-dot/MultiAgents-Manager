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
# 注入窗口标题 marker（MAM:<session_id 前 8 位>）。/dev/tty 在 hook（原生进程 spawn 的 bash）
# 上下文不可达，改写 Windows 控制台设备 CONOUT$（hook 子进程继承宿主控制台）
MID=$(printf '%s' "$SESSION_ID" | cut -c1-8)
powershell -NoProfile -Command "[IO.File]::WriteAllText('CONOUT$',[char]27+\"]0;MAM:$MID\"+[char]7)" >/dev/null 2>&1 || true
"#;

/// 确保 Hook 脚本和事件目录存在
pub fn ensure_hook_script() -> PathBuf {
    let mam_dir = dirs::home_dir().unwrap_or_default().join(".mam");
    let hooks_dir = mam_dir.join("hooks");
    let events_dir = mam_dir.join("events");
    let _ = fs::create_dir_all(&hooks_dir);
    let _ = fs::create_dir_all(&events_dir);

    let script_path = hooks_dir.join("status-hook.sh");
    // 无条件重写：脚本由应用托管，幂等重写保证升级后新 marker 生效
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

    // Windows 无法直接执行 .sh，hook 命令经 bash 调用（Git Bash 随开发/使用环境存在）
    let command_str = if cfg!(windows) {
        format!("bash \"{}\"", script_path_str)
    } else {
        script_path_str.clone()
    };

    // 读取现有配置（不存在则创建空对象）
    let existing = fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
    let mut config: serde_json::Value =
        serde_json::from_str(&existing).map_err(|e| format!("解析配置文件失败: {}", e))?;

    // 确保 hooks 对象存在
    // 确保 hooks 对象存在
    if config.get("hooks").is_none() {
        config["hooks"] = serde_json::json!({});
    }
    let hooks = config.get_mut("hooks").ok_or("hooks 字段不存在")?;
    let hooks_obj = hooks.as_object_mut().ok_or("hooks 字段不是对象")?;

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
                    entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|hooks| {
                            hooks.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(|c| c.contains("status-hook.sh"))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                });
                if already {
                    debug!("Hook 已注册: {}", event_name);
                    continue;
                }
            }
        }

        // 合并式追加：用户已有同事件 hooks 时保留其条目，仅追加我们的（不整组替换）
        let our_entry = serde_json::json!({
            "matcher": "",
            "hooks": [{
                "type": "command",
                "command": &command_str
            }]
        });
        match hooks_obj.get_mut(&event_name) {
            Some(arr) if arr.is_array() => {
                arr.as_array_mut().unwrap().push(our_entry);
            }
            _ => {
                hooks_obj.insert(event_name, serde_json::json!([our_entry]));
            }
        }
        added += 1;
    }

    if added > 0 {
        // 创建备份（防止写入失败导致配置丢失）
        if config_path.exists() {
            let backup = config_path.with_extension("json.bak");
            let _ = fs::copy(config_path, &backup);
        }
        let pretty =
            serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {}", e))?;
        crate::linker::write_config_locked(config_path, &pretty)
            .map_err(|e| format!("写入配置文件失败: {}", e))?;
        info!("已注册 {} 个 Hook 到 {:?}", added, config_path);
    }

    Ok(())
}

/// 读取所有 Hook 事件文件，返回 PPID → 事件数据的映射
pub fn read_hook_events() -> HashMap<u32, HookEvent> {
    let mut events = HashMap::new();
    let events_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("events");

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
/// 核验实际配置状态而非信任 DB 标志：修复"全局单标志 + 永不核验"导致的假阳性
/// （此前 claude 注册失败后因 codex 成功置位而永不重试）
pub fn register_all_hooks() {
    use crate::adapter::{claude::ClaudeAdapter, codex::CodexAdapter};
    use crate::adapter::{AgentAdapter, HookEventCase};

    let adapters: Vec<Box<dyn AgentAdapter>> =
        vec![Box::new(ClaudeAdapter), Box::new(CodexAdapter)];
    let script_path = ensure_hook_script();

    for adapter in &adapters {
        if !adapter.hook_supported() {
            continue;
        }
        let Some(config_path) = adapter.hook_config_path() else {
            continue;
        };
        let tool_key = format!(
            "hooks_registered_{}",
            format!("{:?}", adapter.agent_type()).to_lowercase()
        );

        // 启动核验：配置文件实际包含 status-hook 引用且脚本存在才跳过
        let verified = fs::read_to_string(&config_path)
            .map(|c| c.contains("status-hook.sh"))
            .unwrap_or(false)
            && script_path.exists();
        if verified {
            crate::database::set_setting(&tool_key, "true");
            debug!("{} Hook 已确认: {:?}", adapter.name(), config_path);
            continue;
        }

        let events = adapter.hook_events();
        let is_pascal = matches!(adapter.hook_event_case(), HookEventCase::PascalCase);
        match register_hooks_for_tool(&config_path, &events, is_pascal) {
            Ok(()) => {
                info!("Hook 注册成功: {} → {:?}", adapter.name(), config_path);
                crate::database::set_setting(&tool_key, "true");
            }
            Err(e) => warn!(
                "Hook 注册失败 {} → {:?}: {}",
                adapter.name(),
                config_path,
                e
            ),
        }
    }
}
