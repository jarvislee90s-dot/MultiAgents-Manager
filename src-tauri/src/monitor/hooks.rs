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
# 从 stdin 读取 JSON，写入 ~/.mam/events/<session_id>.json
EVENTS_DIR="$HOME/.mam/events"
mkdir -p "$EVENTS_DIR"
INPUT=$(cat)
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
SESSION_ID=$(echo "$INPUT" | grep -o '"session_id"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
CWD=$(echo "$INPUT" | grep -o '"cwd"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
TS=$(date +%s)
LAST_EVENT_AT=$(date -u +%Y-%m-%dT%H:%M:%SZ)
# 事件以 session_id 为键（$PPID 在 claude 脱管 hook 进程里恒为 1，多会话互覆——
# 2026-09-12 第三轮探测 C2 实证；session_id 来自 stdin）。字符白名单外的值
# 直接丢弃（防路径注入；合法 UUID 形态永不触发）。同会话覆盖=保留最新状态
if printf '%s' "$SESSION_ID" | grep -qE '^[A-Za-z0-9-]+$'; then
  echo "{\"event\":\"$EVENT\",\"session_id\":\"$SESSION_ID\",\"cwd\":\"$CWD\",\"ts\":$TS,\"last_event_at\":\"$LAST_EVENT_AT\"}" > "$EVENTS_DIR/$SESSION_ID.json"
fi
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
    // marker helper 安装（issue #43）：把与主程序同目录的 mam-marker 拷到 ~/.mam/bin/
    // 供 hook 脚本调用。helper 未构建/未随包分发是合法状态——hook 检测不到即跳过，
    // 跳转链完整回落既有消歧层（零回归）。无条件覆盖保证升级后新版 helper 生效
    install_marker_helper();
    script_path
}

/// 拷贝 mam-marker helper 到 ~/.mam/bin/（存在才拷；返回目标路径）
fn install_marker_helper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let bin_dir = dirs::home_dir()?.join(".mam").join("bin");
    let _ = fs::create_dir_all(&bin_dir);
    // Windows 发行名带 .exe；macOS/Linux 开发态为裸名（helper 实际仅 Windows 生效，
    // 非 Windows 拷贝只为保持路径逻辑一致、无害）
    for name in ["mam-marker.exe", "mam-marker"] {
        let src = dir.join(name);
        if src.is_file() {
            let dst = bin_dir.join(name);
            if fs::copy(&src, &dst).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = fs::metadata(&dst) {
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(&dst, perms);
                    }
                }
                return Some(dst);
            }
        }
    }
    None
}

/// Windows hook 命令：路径含空格才加引号。claude 在 Windows 经
/// `powershell -Command "<command>"` 包装执行钩子，内层双引号会破坏外层配对
/// （SessionStart 报错根因）；无空格路径去引号即绕开
fn quote_bash_command(path_str: &str) -> String {
    if path_str.contains(' ') {
        format!("bash \"{path_str}\"")
    } else {
        format!("bash {path_str}")
    }
}

/// 当前平台注册的 hook 命令（注册 / 核验 / 去重三处同源，单一事实源）
fn hook_command_for(script_path: &std::path::Path) -> String {
    let s = script_path.to_string_lossy().to_string();
    if cfg!(windows) {
        quote_bash_command(&s)
    } else {
        s
    }
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
    let command_str = hook_command_for(&script_path);

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
    let mut migrated_any = false;
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

        // 已注册检测 + 旧形态迁移：条目 command 含脚本路径即视为我们注册的。
        // 与当前命令一致 → 跳过；含路径但形态旧（如带引号旧格式）→ 原地改写
        // 为当前命令（追加会造成双写事件且旧条目继续触发 SessionStart 报错）
        let mut already = false;
        let mut migrated_this_event = false;
        if let Some(arr) = hooks_obj
            .get_mut(&event_name)
            .and_then(|v| v.as_array_mut())
        {
            for entry in arr.iter_mut() {
                let Some(cmds) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) else {
                    continue;
                };
                for h in cmds.iter_mut() {
                    let Some(c) = h
                        .get("command")
                        .and_then(|c| c.as_str())
                        .map(|s| s.to_string())
                    else {
                        continue;
                    };
                    if !c.contains(&script_path_str) {
                        continue; // 用户自己的 hook 条目，不动
                    }
                    if c == command_str {
                        already = true;
                    } else {
                        h["command"] = serde_json::json!(command_str);
                        migrated_this_event = true;
                    }
                }
            }
        }
        migrated_any |= migrated_this_event;
        // 已注册或本轮完成原地迁移：条目已等于当前命令，再追加会产生同命令重复
        // 条目（同事件双触发、双写事件），直接进入下一事件；持久化由 migrated_any 保证
        if already || migrated_this_event {
            debug!("Hook 已注册/已迁移: {}", event_name);
            continue;
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

    if added > 0 || migrated_any {
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

/// 读取所有 Hook 事件文件，返回 session_id → 事件数据的映射（键由脚本侧
/// 文件名承载；旧 PPID 形态文件 30s TTL 内短暂并存、键永不匹配任何会话，无害）
pub fn read_hook_events() -> HashMap<String, HookEvent> {
    let events_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("events");
    read_hook_events_from(&events_dir)
}

/// 核心逻辑（tempdir 可测）：文件名即 session_id，白名单校验 + 30s TTL 过滤
fn read_hook_events_from(events_dir: &std::path::Path) -> HashMap<String, HookEvent> {
    let mut events = HashMap::new();
    if !events_dir.exists() {
        return events;
    }
    let valid_sid =
        |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if let Ok(entries) = fs::read_dir(events_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                let Some(sid) = filename.strip_suffix(".json") else {
                    continue;
                };
                if !valid_sid(sid) {
                    continue;
                }
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(event) = serde_json::from_str::<HookEvent>(&content) {
                        let now = chrono::Utc::now().timestamp();
                        if now - event.ts < 30 {
                            events.insert(sid.to_string(), event);
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
        let tool_key = format!("hooks_registered_{}", adapter.agent_type().tool_id());

        // 启动核验：配置文件实际引用**当前脚本绝对路径**且脚本存在才跳过。
        // 只查 "status-hook.sh" 文件名子串会把旧位置的历史注册误判为已核验、
        // 永不重写（2026-09-11 实机验收发现 2 弱点；codex 0.149.1 hook 链路
        // 不执行是 codex 侧问题，此处保证 MAM 侧配置口径始终正确）
        let expected_cmd = hook_command_for(&script_path);
        let verified = fs::read_to_string(&config_path)
            .map(|c| c.contains(&expected_cmd))
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

#[cfg(test)]
mod command_quote_tests {
    use super::quote_bash_command;

    #[test]
    fn no_space_path_is_unquoted() {
        // 无空格路径不加引号：消除 powershell -Command 包装层的引号嵌套
        // （claude Windows 侧 SessionStart 报错根因，2026-09-12 第三轮探测 C1）
        assert_eq!(
            quote_bash_command(r"C:\Users\bunny\.mam\hooks\status-hook.sh"),
            r"bash C:\Users\bunny\.mam\hooks\status-hook.sh"
        );
    }

    #[test]
    fn spaced_path_keeps_quotes() {
        // 含空格路径必须保引号（已知残留：该形态下 SessionStart 报错可能复现，spec 3.4）
        assert_eq!(
            quote_bash_command(r"C:\Users\John Doe\.mam\hooks\status-hook.sh"),
            r#"bash "C:\Users\John Doe\.mam\hooks\status-hook.sh""#
        );
    }
}

#[cfg(test)]
mod event_channel_tests {
    use super::*;
    use std::io::Write;

    fn write_event(dir: &std::path::Path, name: &str, event: &str, age_secs: i64) {
        let ts = chrono::Utc::now().timestamp() - age_secs;
        let body = format!(
            r#"{{"event":"{event}","session_id":"sid-x","cwd":"/tmp","ts":{ts},"last_event_at":"2026-09-12T00:00:00Z"}}"#
        );
        let mut f = std::fs::File::create(dir.join(format!("{name}.json"))).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    /// 用独立 tempdir 作为 events 目录跑 read_hook_events（经 MAM_HOME 重定向不可行，
    /// 该函数直接拼 home 路径——测试以子进程隔离或直接抽取核心逻辑。此处选择抽取：
    /// 见 Step 3 的 read_hook_events_from，read_hook_events 成为其薄包装）
    #[test]
    fn events_are_keyed_by_session_id_with_ttl() {
        let tmp = tempfile::tempdir().unwrap();
        write_event(tmp.path(), "01a08083-5ca0", "Stop", 0);
        write_event(tmp.path(), "0f1e2d3c-4b5a", "Stop", 120); // 过期
        write_event(tmp.path(), "1", "Stop", 0); // 旧 PPID 形态孤儿（合法字符，30s 后自然消失）
        let m = read_hook_events_from(tmp.path());
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("01a08083-5ca0"));
        assert!(m.contains_key("1"));
        assert!(!m.contains_key("0f1e2d3c-4b5a"));
    }

    #[test]
    fn script_uses_session_id_key_and_has_no_marker_block() {
        // spec 改动三：hook 周期注入退役；事件键 session_id 化（脚本内容回归锁）
        assert!(HOOK_SCRIPT.contains("$SESSION_ID.json"));
        assert!(!HOOK_SCRIPT.contains("MAM_MARKER"));
        assert!(!HOOK_SCRIPT.contains("mam-marker"));
        assert!(HOOK_SCRIPT.contains("^[A-Za-z0-9-]+$")); // 白名单守卫在场
    }
}
