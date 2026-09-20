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
    // 无条件重写：脚本由应用托管，幂等重写保证升级后新脚本内容生效
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
    // 供 MAM 主进程跳转按需注入调用（window/win32.rs::inject_marker_on_demand）。
    // helper 未构建/未随包分发是合法状态——主进程检测不到即跳过注入，
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

/// Windows hook 命令：**正斜杠**路径，含空格才加引号。两层转义约束（第四轮实机
/// 验收实证）：① 引号在 claude 的 `powershell -Command "<command>"` 包装下破坏
/// 外层配对（SessionStart 报错根因）；② 裸反斜杠路径在 bash 端被当转义序列吃掉
/// （`C:\Users` → `C:Users`，exit=127 通道全断）。正斜杠两层皆安全（bash 原生
/// 接受、powershell 不转义），第四轮离线探针三层全通。含空格路径必须保引号
/// （已知残留：该形态下 SessionStart 报错可能复现，spec §3.4 边界）
fn quote_bash_command(path_str: &str) -> String {
    let normalized = path_str.replace('\\', "/");
    if normalized.contains(' ') {
        format!("bash \"{normalized}\"")
    } else {
        format!("bash {normalized}")
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

/// F3 旧键迁移纯函数（PascalCase 注册形态专用；跨平台可测）：把 event（如 "Stop"）
/// 的首字母小写旧键（"stop"）从 hooks 配置移除——**仅当旧键全部条目都是 MAM 注册**
/// （每条 command 含本脚本路径，正反斜杠双形态判据对齐条目迁移逻辑）；混有用户
/// 条目 → 保守不动（codex 对未知键不触发，残留无害）。旧键不存在 → false。
/// 返回 true 表示发生了移除（计入 migrated 保证纯迁移场景也持久化）。
fn remove_legacy_camel_key(
    hooks_obj: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    script_path_str: &str,
) -> bool {
    let legacy: String = {
        let mut chars = event.chars();
        match chars.next() {
            Some(first) => first.to_lowercase().chain(chars).collect::<String>(),
            None => return false,
        }
    };
    if legacy == event {
        return false; // 本就全小写的事件名无 twins（防御）
    }
    let fwd = script_path_str.replace('\\', "/");
    let all_ours = hooks_obj
        .get(&legacy)
        .and_then(|v| v.as_array())
        .map(|arr| {
            !arr.is_empty()
                && arr.iter().all(|entry| {
                    entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|cmds| {
                            !cmds.is_empty()
                                && cmds.iter().all(|h| {
                                    h.get("command")
                                        .and_then(|c| c.as_str())
                                        .map(|s| s.contains(script_path_str) || s.contains(&fwd))
                                        .unwrap_or(false)
                                })
                        })
                        .unwrap_or(false)
                })
        });
    if all_ours == Some(true) {
        hooks_obj.remove(&legacy).is_some()
    } else {
        false
    }
}

/// 启动核验判据（纯函数，跨平台可测）：command 在场 **且** 全部期望事件键按
/// 该工具的键形态在场（PascalCase 工具查 PascalCase 键）。键形态核验是 F3
/// 存量迁移的可达性前提——command 在旧/新注册间完全相同，只有键大小写不同。
fn hooks_file_verified(
    content: &str,
    expected_cmd: &str,
    events: &[&str],
    is_pascal_case: bool,
) -> bool {
    if !content.contains(expected_cmd) {
        return false;
    }
    events.iter().all(|e| {
        let key = if is_pascal_case {
            (*e).to_string()
        } else {
            let mut chars = e.chars();
            match chars.next() {
                Some(first) => first.to_lowercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        };
        content.contains(&format!("\"{key}\""))
    })
}

/// 为指定工具注册 Hook（生产入口：脚本落盘 + 命令构造 + 核心）
pub fn register_hooks_for_tool(
    config_path: &std::path::Path,
    events: &[&str],
    is_pascal_case: bool,
) -> Result<(), String> {
    let script_path = ensure_hook_script();
    let script_path_str = script_path.to_string_lossy().to_string();

    // Windows 无法直接执行 .sh，hook 命令经 bash 调用（Git Bash 随开发/使用环境存在）
    let command_str = hook_command_for(&script_path);
    register_hooks_in_file(
        config_path,
        events,
        is_pascal_case,
        &script_path_str,
        &command_str,
    )
    .map(|_| ())
}

/// 注册核心（tempfile 可测缝：脚本路径/命令显式注入，零接触真实 ~/.mam）。
/// 返回 (新增条目数, 迁移条目数)。
fn register_hooks_in_file(
    config_path: &std::path::Path,
    events: &[&str],
    is_pascal_case: bool,
    script_path_str: &str,
    command_str: &str,
) -> Result<(usize, usize), String> {
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
    // 原地迁移的旧条目计数（处）：仅用于成功日志区分「新注册」与「迁移」，
    // 不改变控制流语义（0 ⇔ 原来的 migrated_any=false）
    let mut migrated = 0usize;
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
        let mut migrated_this_event = 0usize;

        // F3 旧键迁移（仅 PascalCase 注册形态；codex hook_event_case CamelCase→
        // PascalCase 存量修正，2026-09-20）：旧注册把 MAM 条目写在首字母小写键下
        // （如 "stop"），codex 0.155.x 只认 PascalCase 键——旧键永不触发但残留
        // 文件。移除判据见 [`remove_legacy_camel_key`]。
        // 跨键移除只计入迁移日志（migrated），**不参与下方 skip 守卫**：旧键条目
        // 已删除、新键可能尚不存在（纯存量 camelCase 文件），必须走下方追加建键
        // ——共用计数会把纯存量文件迁成空 {"hooks":{}}（复评 P1-1，2026-09-20）
        let legacy_removed =
            is_pascal_case && remove_legacy_camel_key(hooks_obj, event, script_path_str);
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
                    // 双形态判据：脚本绝对路径正反斜杠各查一次。只查原路径会漏掉
                    // 正斜杠形态的历史条目（如第四轮 1f8fcf4 产出的无引号形态），
                    // 它们会因识别不出而被当作用户条目跳过 → 坏条目残留 + 新条目追加
                    let fwd_path = script_path_str.replace('\\', "/");
                    if !c.contains(script_path_str) && !c.contains(&fwd_path) {
                        continue; // 用户自己的 hook 条目，不动
                    }
                    if c == command_str {
                        already = true;
                    } else {
                        h["command"] = serde_json::json!(command_str);
                        migrated_this_event += 1;
                    }
                }
            }
        }
        migrated += migrated_this_event + usize::from(legacy_removed);
        // 已注册或本轮完成原地迁移：条目已等于当前命令，再追加会产生同命令重复
        // 条目（同事件双触发、双写事件），直接进入下一事件；持久化由 migrated 计数保证
        if already || migrated_this_event > 0 {
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

    if added > 0 || migrated > 0 {
        // 创建备份（防止写入失败导致配置丢失）
        if config_path.exists() {
            let backup = config_path.with_extension("json.bak");
            let _ = fs::copy(config_path, &backup);
        }
        let pretty =
            serde_json::to_string_pretty(&config).map_err(|e| format!("序列化配置失败: {}", e))?;
        crate::linker::write_config_locked(config_path, &pretty)
            .map_err(|e| format!("写入配置文件失败: {}", e))?;
        // 仅迁移（added=0）时「已注册 0 个」有误导：日志区分迁移条目数
        if migrated > 0 {
            info!(
                "已注册 {} 个 Hook（迁移旧条目 {} 处）到 {:?}",
                added, migrated, config_path
            );
        } else {
            info!("已注册 {} 个 Hook 到 {:?}", added, config_path);
        }
    }

    Ok((added, migrated))
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

        let events = adapter.hook_events();
        let is_pascal = matches!(adapter.hook_event_case(), HookEventCase::PascalCase);
        // 启动核验：配置文件实际引用**当前脚本绝对路径**、脚本存在、且**事件键
        // 形态在场**（F3 键形态核验，2026-09-20）才跳过。只查 command 不够——
        // 旧 camelCase 注册的 command 与当前完全相同（只有事件键大小写不同），
        // 会把存量文件误判已核验、迁移永不触达（复评 P1-2）；此处保证 MAM 侧
        // 配置口径（含键形态）始终正确
        let expected_cmd = hook_command_for(&script_path);
        let verified = fs::read_to_string(&config_path)
            .map(|c| hooks_file_verified(&c, &expected_cmd, &events, is_pascal))
            .unwrap_or(false)
            && script_path.exists();
        if verified {
            crate::database::set_setting(&tool_key, "true");
            debug!("{} Hook 已确认: {:?}", adapter.name(), config_path);
            continue;
        }

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
    fn no_space_path_becomes_forward_slash_unquoted() {
        // 第四轮实测：裸反斜杠路径被 bash 当转义序列吃掉（C:\Users → C:Users，
        // exit=127 通道全断）；正斜杠 + 无引号在 powershell 包装 / bash 两层皆安全
        assert_eq!(
            quote_bash_command(r"C:\Users\bunny\.mam\hooks\status-hook.sh"),
            r"bash C:/Users/bunny/.mam/hooks/status-hook.sh"
        );
    }

    #[test]
    fn spaced_path_keeps_quotes_forward_slash() {
        // 含空格路径必须保引号（已知残留：该形态 SessionStart 报错可能复现，spec 3.4）；
        // 分隔符仍归一为正斜杠（bash 端语义一致）
        assert_eq!(
            quote_bash_command(r"C:\Users\John Doe\.mam\hooks\status-hook.sh"),
            r#"bash "C:/Users/John Doe/.mam/hooks/status-hook.sh""#
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

#[cfg(test)]
mod legacy_camel_key_tests {
    use super::remove_legacy_camel_key;

    fn our_entry(cmd: &str) -> serde_json::Value {
        serde_json::json!({ "matcher": "", "hooks": [{ "type": "command", "command": cmd }] })
    }

    #[test]
    fn removes_legacy_key_when_all_entries_ours() {
        // F3 存量形态：codex 旧注册把条目写在 "stop"（camelCase）下；
        // 条目命令与传入脚本路径同源（正斜杠形态经 fwd 判据命中）
        let mut obj = serde_json::Map::new();
        obj.insert(
            "stop".into(),
            serde_json::json!([our_entry("bash C:/Users/u/.mam/hooks/status-hook.sh")]),
        );
        assert!(remove_legacy_camel_key(
            &mut obj,
            "Stop",
            r"C:\Users\u\.mam\hooks\status-hook.sh"
        ));
        assert!(obj.get("stop").is_none(), "全我们条目的旧键必须移除");
    }

    #[test]
    fn keeps_legacy_key_with_user_entries() {
        // 混有用户条目（command 不含脚本路径）→ 保守不动
        let mut obj = serde_json::Map::new();
        obj.insert(
            "stop".into(),
            serde_json::json!([
                our_entry("bash /home/u/.mam/hooks/status-hook.sh"),
                { "matcher": "", "hooks": [{ "type": "command", "command": "user-own-script" }] }
            ]),
        );
        assert!(!remove_legacy_camel_key(
            &mut obj,
            "Stop",
            r"C:\u\.mam\hooks\status-hook.sh"
        ));
        assert!(obj.get("stop").is_some(), "混用户条目不得移除");
    }

    #[test]
    fn absent_or_wrong_case_legacy_key_is_noop() {
        let mut obj = serde_json::Map::new();
        assert!(!remove_legacy_camel_key(
            &mut obj,
            "Stop",
            "/x/status-hook.sh"
        ));
        // PascalCase 键名与 legacy 相同（防御：本就全小写事件名无 twins）
        let mut obj2 = serde_json::Map::new();
        obj2.insert("stop".into(), serde_json::json!([our_entry("x")]));
        assert!(!remove_legacy_camel_key(
            &mut obj2,
            "stop",
            "/x/status-hook.sh"
        ));
        assert!(obj2.get("stop").is_some());
    }

    /// 复评 P1-1 回归锁（2026-09-20）：纯存量 camelCase 文件（条目 command 与
    /// 当前完全一致、只有键是旧形态）必须被完整迁移为 PascalCase 键——旧实现
    /// 跨键移除计数误触 skip 守卫，六事件走完后落盘 {"hooks":{}}（迁空）。
    #[test]
    fn legacy_camel_full_migration_rebuilds_pascal_keys() {
        use super::register_hooks_in_file;
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("hooks.json");
        let marker = "/fake-mam/hooks/status-hook.sh";
        let cmd = format!("bash {marker}");
        let legacy = serde_json::json!({"hooks": {
            "stop": [our_entry(&cmd)],
            "preToolUse": [our_entry(&cmd)],
        }});
        std::fs::write(&cfg, legacy.to_string()).unwrap();

        register_hooks_in_file(&cfg, &["Stop", "PreToolUse"], true, marker, &cmd).unwrap();

        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let hooks = out.get("hooks").unwrap();
        assert!(
            hooks.get("Stop").is_some(),
            "PascalCase Stop 必须重建: {out}"
        );
        assert!(
            hooks.get("PreToolUse").is_some(),
            "PascalCase PreToolUse 必须重建: {out}"
        );
        assert!(
            hooks.get("stop").is_none() && hooks.get("preToolUse").is_none(),
            "旧 camelCase 键应移除: {out}"
        );
        assert!(
            std::fs::read_to_string(&cfg).unwrap().matches(&cmd).count() >= 2,
            "重建条目须携带当前命令"
        );
    }

    /// 复评 P1-2 回归锁（2026-09-20）：command 在场但事件键是旧 camelCase 形态
    /// → 未核验（须走注册迁移）；全期望键按形态在场 → 才核验跳过。
    #[test]
    fn hooks_file_verified_requires_event_key_form() {
        use super::hooks_file_verified;
        let cmd = "bash /x/status-hook.sh";
        let legacy = r#"{"hooks":{"stop":[{"hooks":[{"command":"bash /x/status-hook.sh"}]}]}}"#;
        let modern = r#"{"hooks":{"Stop":[{"hooks":[{"command":"bash /x/status-hook.sh"}]}]}}"#;
        // PascalCase 工具 + 旧 camel 键：command 在场也不得核验（迁移入口保持可达）
        assert!(!hooks_file_verified(legacy, cmd, &["Stop"], true));
        // 全期望键在场：核验
        assert!(hooks_file_verified(modern, cmd, &["Stop"], true));
        // 多事件任缺一键：不核验
        assert!(!hooks_file_verified(
            modern,
            cmd,
            &["Stop", "PreToolUse"],
            true
        ));
        // camelCase 形态工具按 camel 键核验（形态匹配即核验）
        assert!(hooks_file_verified(legacy, cmd, &["stop"], false));
        // command 缺席：不核验
        assert!(!hooks_file_verified(
            modern,
            "bash /other.sh",
            &["Stop"],
            true
        ));
    }
}

/// F3 实机验证（#[ignore]：显式实机跑，M9R ffi_hop 先例；**M1A 前置**）——
/// 注册后跑一次真实 codex 会话确认钩子真触发：
/// ① 本测试（`cargo test --lib monitor::hooks::codex_pascal -- --ignored`）：
///    按生产装配对真实 `~/.codex/hooks.json` 注册 codex 六事件（PascalCase），
///    断言文件落盘 PascalCase 键 + 旧 camelCase 键被迁移清除；
/// ② 人工步骤（Mac 回传清单 C-16）：跑一次真实 codex 交互会话，确认
///    `~/.mam/events/<session_id>.json` 出现（hook 真触发）。
/// 本机无 codex 时测试失败（前置自检 `codex --version`）。
#[test]
#[ignore = "实机验证：改写真实 ~/.codex/hooks.json（M1A 前置，显式 --ignored 跑）"]
fn codex_pascal_case_registration_real_machine() {
    use crate::adapter::AgentAdapter;

    // 前置自检：codex 在场
    let ver = std::process::Command::new("codex")
        .arg("--version")
        .output()
        .expect("codex 命令不可用——本测试需要实机安装 codex");
    assert!(ver.status.success(), "codex --version 失败");

    let adapter = crate::adapter::codex::CodexAdapter;
    let path = adapter
        .hook_config_path()
        .expect("codex 必须有 hooks 配置路径");
    let events = adapter.hook_events();
    let is_pascal = matches!(
        adapter.hook_event_case(),
        crate::adapter::HookEventCase::PascalCase
    );
    assert!(is_pascal, "F3 修复后 codex 必须是 PascalCase 注册形态");

    register_hooks_for_tool(&path, &events, is_pascal).expect("codex hooks 注册失败");

    let raw = std::fs::read_to_string(&path).expect("hooks.json 应存在");
    let cfg: serde_json::Value = serde_json::from_str(&raw).expect("hooks.json 合法 JSON");
    let hooks = cfg
        .get("hooks")
        .and_then(|h| h.as_object())
        .expect("hooks 对象");
    for ev in events {
        assert!(
            hooks.get(ev).is_some(),
            "PascalCase 键 {ev} 必须在注册后出现：{}",
            hooks.keys().cloned().collect::<Vec<_>>().join(",")
        );
        let legacy: String = {
            let mut chars = ev.chars();
            let first = chars.next().unwrap().to_lowercase();
            first.chain(chars).collect()
        };
        assert!(
            hooks.get(&legacy).is_none(),
            "旧 camelCase 键 {legacy} 必须被 F3 迁移清除"
        );
    }
}
