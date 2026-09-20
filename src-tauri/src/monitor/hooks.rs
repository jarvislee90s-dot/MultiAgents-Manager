// Hook 系统 — 事件注册 + 共享脚本 + 事件文件读取
// Claude Code: settings.json (PascalCase) / Codex CLI: hooks.json (camelCase)

use log::{debug, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::monitor::hook_listener;

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
    // 事件目录与 helper 写侧同源（hook_listener::default_events_dir，MAM_HOME
    // debug 重定向同 connection.rs 先例）。bash 版脚本内部仍硬编码 $HOME——
    // Windows/正式链路已由原生 helper 承载（批次甲 T1），unix dev 重定向场景属
    // 已知边界（脚本目录 hooks/ 保持真实家目录，避免 unix 存量注册路径漂移）
    let events_dir = hook_listener::default_events_dir();
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
    // hook 事件 helper 安装（批次甲 T1）：同一分发管道落盘 mam-hook-listener，
    // 供 hook 配置直启（零 shell 依赖，替代 bash 脚本）。缺失同样是合法状态——
    // 注册侧检测不到即回落 bash 形态命令（零回归）
    install_hook_listener_helper();
    script_path
}

/// helper 安装目标目录解析（纯函数）：home → `~/.mam/bin`
fn helper_install_dir(home: &std::path::Path) -> PathBuf {
    home.join(".mam").join("bin")
}

/// 拷贝 mam-marker helper 到 ~/.mam/bin/（存在才拷；返回目标路径）
fn install_marker_helper() -> Option<PathBuf> {
    install_helper_bin(&["mam-marker.exe", "mam-marker"])
}

/// 拷贝 mam-hook-listener helper 到 ~/.mam/bin/（批次甲 T1；返回目标路径）。
/// 未构建/未随包分发 → None（注册回落 bash 形态，零回归）
pub(crate) fn install_hook_listener_helper() -> Option<PathBuf> {
    install_helper_bin(&["mam-hook-listener.exe", "mam-hook-listener"])
}

/// helper 分发入口（marker / hook-listener 共用核心）：从当前 exe 同目录拷候选名
/// 到 `~/.mam/bin/`（无条件覆盖，升级后新版 helper 生效）
fn install_helper_bin(names: &[&str]) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let src_dir = exe.parent()?.to_path_buf();
    let bin_dir = helper_install_dir(&dirs::home_dir()?);
    install_helper_from_to(&src_dir, &bin_dir, names)
}

/// helper 安装路径解析 + 拷贝核心（tempdir 可测缝）：`names` 依序探测 src_dir 下
/// 候选（Windows 发行 .exe 在前、非 Windows 开发态裸名在后），首个存在者拷到
/// dst_dir 同名（无条件覆盖；unix 补 0755）
fn install_helper_from_to(
    src_dir: &std::path::Path,
    dst_dir: &std::path::Path,
    names: &[&str],
) -> Option<PathBuf> {
    let _ = fs::create_dir_all(dst_dir);
    for name in names {
        let src = src_dir.join(name);
        if src.is_file() {
            let dst = dst_dir.join(name);
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

/// helper 直启命令（批次甲 T1 零 shell 依赖）：正斜杠归一 + 含空格才加引号——
/// 与 quote_bash_command 同一套两层转义实证结论（claude 的 powershell -Command
/// 包装 / codex 直接 spawn 两层皆安全），去掉 bash 前缀。含空格路径的引号残留
/// 边界同 bash 形态（spec §3.4 已知残留，非本批处理面）
fn helper_command_for(helper_path: &std::path::Path) -> String {
    let normalized = helper_path.to_string_lossy().replace('\\', "/");
    if normalized.contains(' ') {
        format!("\"{normalized}\"")
    } else {
        normalized
    }
}

/// Hook 命令规格（T1 单一事实源：注册 / 核验 / 迁移三处同源）
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HookCommandSpec {
    /// 注册命令主体：claude `command` 字段；codex 的非 Windows 落地形态
    command: String,
    /// codex 官方 `commandWindows` 字段（Windows 直启覆盖，codex hook_config.rs
    /// L167 实证；直接 spawn 零 shell——issue #74 根因 2 的根治面）。claude 无此
    /// 字段 → None
    command_windows: Option<String>,
    /// helper 落盘绝对路径原文（我方条目判据 / 迁移标记用；bash 兜底形态 → None）
    helper_path: Option<String>,
}

/// 按工具构造命令规格（生产入口：平台语义取 cfg!(windows)；跨平台可测纯函数见
/// [`hook_command_spec_for_impl`])
fn hook_command_spec_for(
    tool_id: &str,
    script_path: &std::path::Path,
    helper_path: Option<&std::path::Path>,
) -> HookCommandSpec {
    hook_command_spec_for_impl(tool_id, script_path, helper_path, cfg!(windows))
}

/// 规格构造纯决策（`windows_semantics` 由生产入口传 `cfg!(windows)`，测试双平台
/// 语义均可显式驱动）：
/// - **claude + Windows + helper**：`command` 直启 helper（powershell 包装下正斜杠
///   安全，helper_command_for）；helper 缺席回落 bash 形态（零回归）
/// - **codex + Windows + helper**：`command` 恒为 bash 形态（非 Windows 落地用，
///   unix 行为不变）；Windows 由官方 `commandWindows` 直启 helper
/// - **兜底**（非 Windows / helper 未随包分发 / 其余工具）：bash 形态、无覆盖字段
///   ——与 T1 前行为完全一致
fn hook_command_spec_for_impl(
    tool_id: &str,
    script_path: &std::path::Path,
    helper_path: Option<&std::path::Path>,
    windows_semantics: bool,
) -> HookCommandSpec {
    let bash_form = hook_command_for(script_path);
    let helper_str = helper_path.map(|p| p.to_string_lossy().to_string());
    match (tool_id, windows_semantics, helper_path) {
        ("claude", true, Some(h)) => HookCommandSpec {
            command: helper_command_for(h),
            command_windows: None,
            helper_path: helper_str,
        },
        ("codex", true, Some(h)) => HookCommandSpec {
            command: bash_form,
            command_windows: Some(helper_command_for(h)),
            helper_path: helper_str,
        },
        _ => HookCommandSpec {
            command: bash_form,
            command_windows: None,
            helper_path: None,
        },
    }
}

/// 我方条目判据标记集：脚本绝对路径 + helper 落盘路径，各配正反斜杠双形态（F3
/// 双形态判据的 helper 扩展——历史条目可能以任一路径、任一斜杠形态在册）
fn ours_markers(script_path_str: &str, spec: &HookCommandSpec) -> Vec<String> {
    let mut markers = vec![
        script_path_str.to_string(),
        script_path_str.replace('\\', "/"),
    ];
    if let Some(hp) = &spec.helper_path {
        markers.push(hp.clone());
        markers.push(hp.replace('\\', "/"));
    }
    markers
}

/// 条目命令是否 MAM 注册（含任一标记即算——`contains` 语义与 F3 迁移判据同源）
fn command_is_ours(command: &str, markers: &[String]) -> bool {
    markers.iter().any(|m| command.contains(m.as_str()))
}

/// F3 旧键迁移纯函数（PascalCase 注册形态专用；跨平台可测）：把 event（如 "Stop"）
/// 的首字母小写旧键（"stop"）从 hooks 配置移除——**仅当旧键全部条目都是 MAM 注册**
/// （每条 command 命中我方标记集 [`ours_markers`]：脚本路径/helper 路径 × 正反斜杠
/// 双形态）；混有用户条目 → 保守不动（codex 对未知键不触发，残留无害）。旧键不
/// 存在 → false。返回 true 表示发生了移除（计入 migrated 保证纯迁移场景也持久化）。
fn remove_legacy_camel_key(
    hooks_obj: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    markers: &[String],
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
                                        .map(|s| command_is_ours(s, markers))
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

/// 启动核验判据（纯函数，跨平台可测）：当前命令在场 **且**（codex）commandWindows
/// 期望命令在场 **且** 全部期望事件键按该工具的键形态在场（PascalCase 工具查
/// PascalCase 键）。键形态核验是 F3 存量迁移的可达性前提；commandWindows 核验是
/// T1 存量迁移的可达性前提——只查 command 不够：旧 bash 条目的 command 与 codex
/// 规格的 command 完全相同（差异只在 commandWindows 有无），会把存量文件误判已
/// 核验、commandWindows 迁移永不触达。
fn hooks_file_verified(
    content: &str,
    spec: &HookCommandSpec,
    events: &[&str],
    is_pascal_case: bool,
) -> bool {
    if !content.contains(&spec.command) {
        return false;
    }
    if let Some(want) = &spec.command_windows {
        if !content.contains(want.as_str()) {
            return false;
        }
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

/// 为指定工具注册 Hook（生产入口：脚本落盘 + helper 安装 + 命令规格注入核心）。
/// `spec` 由调用方按工具构造（[`hook_command_spec_for`]）——claude/codex 的命令
/// 形态不同（command 直启 vs commandWindows 覆盖），规格单一事实源避免注册/核验/
/// 迁移三处口径漂移
pub(crate) fn register_hooks_for_tool(
    config_path: &std::path::Path,
    events: &[&str],
    is_pascal_case: bool,
    spec: &HookCommandSpec,
) -> Result<(), String> {
    let script_path = ensure_hook_script();
    let script_path_str = script_path.to_string_lossy().to_string();
    register_hooks_in_file(config_path, events, is_pascal_case, &script_path_str, spec).map(|_| ())
}

/// 注册核心（tempfile 可测缝：脚本路径/命令规格显式注入，零接触真实 ~/.mam）。
/// 返回 (新增条目数, 迁移条目数)。
fn register_hooks_in_file(
    config_path: &std::path::Path,
    events: &[&str],
    is_pascal_case: bool,
    script_path_str: &str,
    spec: &HookCommandSpec,
) -> Result<(usize, usize), String> {
    // 读取现有配置（不存在则创建空对象）
    let existing = fs::read_to_string(config_path).unwrap_or_else(|_| "{}".to_string());
    let mut config: serde_json::Value =
        serde_json::from_str(&existing).map_err(|e| format!("解析配置文件失败: {}", e))?;

    // 确保 hooks 对象存在
    if config.get("hooks").is_none() {
        config["hooks"] = serde_json::json!({});
    }
    let hooks = config.get_mut("hooks").ok_or("hooks 字段不存在")?;
    let hooks_obj = hooks.as_object_mut().ok_or("hooks 字段不是对象")?;

    let markers = ours_markers(script_path_str, spec);
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

        // 已注册检测 + 旧形态迁移：条目 command 命中我方标记集（脚本路径/helper
        // 路径 × 正反斜杠，[`command_is_ours`]）即视为我们注册的。与当前命令一致
        // 且 Windows 覆盖字段齐备 → 跳过；形态旧（如 bash 脚本命令、缺
        // commandWindows）→ 原地改写为当前规格（追加会造成双写事件且旧条目继续
        // 触发报错）
        let mut already = false;
        let mut migrated_this_event = 0usize;

        // F3 旧键迁移（仅 PascalCase 注册形态；codex hook_event_case CamelCase→
        // PascalCase 存量修正，2026-09-20）：旧注册把 MAM 条目写在首字母小写键下
        // （如 "stop"），codex 0.155.x 只认 PascalCase 键——旧键永不触发但残留
        // 文件。移除判据见 [`remove_legacy_camel_key`]。
        // 跨键移除只计入迁移日志（migrated），**不参与下方 skip 守卫**：旧键条目
        // 已删除、新键可能尚不存在（纯存量 camelCase 文件），必须走下方追加建键
        // ——共用计数会把纯存量文件迁成空 {"hooks":{}}（复评 P1-1，2026-09-20）
        let legacy_removed = is_pascal_case && remove_legacy_camel_key(hooks_obj, event, &markers);
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
                    // 用户自己的 hook 条目，不动
                    if !command_is_ours(&c, &markers) {
                        continue;
                    }
                    let cmd_ok = c == spec.command;
                    // commandWindows 期望形态比对：spec 无覆盖 → 不强求（unix 上
                    // 该字段惰性，历史残留无害不折腾）；spec 有覆盖 → 必须在场且
                    // 一致（缺失/旧值都算未迁移——T1 bash→helper 迁移的主形态）
                    let win_ok = match (
                        &spec.command_windows,
                        h.get("commandWindows").and_then(|w| w.as_str()),
                    ) {
                        (Some(want), Some(cur)) => cur == want,
                        (Some(_), None) => false,
                        (None, _) => true,
                    };
                    if cmd_ok && win_ok {
                        already = true;
                        continue;
                    }
                    if !cmd_ok {
                        h["command"] = serde_json::json!(spec.command);
                        migrated_this_event += 1;
                    }
                    if !win_ok {
                        if let Some(want) = &spec.command_windows {
                            h["commandWindows"] = serde_json::json!(want);
                        }
                        migrated_this_event += 1;
                    }
                }
            }
        }
        migrated += migrated_this_event + usize::from(legacy_removed);
        // 已注册或本轮完成原地迁移：条目已等于当前规格，再追加会产生同命令重复
        // 条目（同事件双触发、双写事件），直接进入下一事件；持久化由 migrated 计数保证
        if already || migrated_this_event > 0 {
            debug!("Hook 已注册/已迁移: {}", event_name);
            continue;
        }

        // 合并式追加：用户已有同事件 hooks 时保留其条目，仅追加我们的（不整组替换）
        let mut handler = serde_json::json!({ "type": "command", "command": &spec.command });
        if let Some(want) = &spec.command_windows {
            handler["commandWindows"] = serde_json::json!(want);
        }
        let our_entry = serde_json::json!({
            "matcher": "",
            "hooks": [handler]
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
/// 文件名承载；旧 PPID 形态文件 30s TTL 内短暂并存、键永不匹配任何会话，无害）。
/// 目录定位与 helper 写侧同源（hook_listener::default_events_dir，MAM_HOME debug
/// 重定向口径一致——写读两侧经同一函数出路径，任何配置下互不脱靶）
pub fn read_hook_events() -> HashMap<String, HookEvent> {
    let events_dir = hook_listener::default_events_dir();
    read_hook_events_from(&events_dir)
}

/// 核心逻辑（tempdir 可测）：文件名即 session_id，白名单校验 + 30s TTL 过滤
fn read_hook_events_from(events_dir: &std::path::Path) -> HashMap<String, HookEvent> {
    let mut events = HashMap::new();
    if !events_dir.exists() {
        return events;
    }
    // 白名单谓词与 helper 写侧同一函数（hook_listener::session_id_allowed——
    // 含 T1 新增的 128 字符防御上限，两侧同口径）
    let valid_sid = hook_listener::session_id_allowed;
    if let Ok(entries) = fs::read_dir(events_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                let Some(sid) = filename.strip_suffix(".json") else {
                    continue; // helper 原子写的 *.tmp 中间文件在此天然跳过
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
    // T1 原生 helper：随启动分发管道落盘（mam-hook-listener）；未构建/未随包分发
    // 是合法状态 → None → 各工具规格回落 bash 形态命令（零回归）
    let helper_path = install_hook_listener_helper();

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
        // T1 命令规格（按工具单一事实源）：claude → command 直启 helper；codex →
        // commandWindows 直启 helper（command 保持 bash 形态供非 Windows 落地）。
        // 核验/注册/迁移三处共用同一规格
        let spec = hook_command_spec_for(
            adapter.agent_type().tool_id(),
            &script_path,
            helper_path.as_deref(),
        );
        // 启动核验：配置文件实际引用**当前命令规格**（含 codex 的 commandWindows）、
        // 脚本存在、且**事件键形态在场**（F3 键形态核验，2026-09-20）才跳过。只查
        // command 不够——旧 camelCase 注册的 command 与当前完全相同（只有事件键大
        // 小写不同），会把存量文件误判已核验、迁移永不触达（复评 P1-2）；同理旧
        // bash 条目与 codex 规格的 command 相同（只差 commandWindows），T1 迁移
        // 也要求核验覆盖 Windows 覆盖字段
        let verified = fs::read_to_string(&config_path)
            .map(|c| hooks_file_verified(&c, &spec, &events, is_pascal))
            .unwrap_or(false)
            && script_path.exists();
        if verified {
            crate::database::set_setting(&tool_key, "true");
            debug!("{} Hook 已确认: {:?}", adapter.name(), config_path);
            continue;
        }

        match register_hooks_for_tool(&config_path, &events, is_pascal, &spec) {
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

    /// 用独立 tempdir 作为 events 目录跑 read_hook_events（测试以 tempdir 直注核心
    /// 逻辑，零接触真实 ~/.mam；T1 起 read_hook_events 目录定位与 helper 写侧同源
    /// ——hook_listener::default_events_dir，MAM_HOME debug 重定向两侧口径一致）
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
    use super::{ours_markers, remove_legacy_camel_key, HookCommandSpec};

    fn our_entry(cmd: &str) -> serde_json::Value {
        serde_json::json!({ "matcher": "", "hooks": [{ "type": "command", "command": cmd }] })
    }

    /// bash 兜底规格（T1 前形态：无 helper、无 Windows 覆盖）——既有 F3 用例语义不变
    fn bash_spec(script_path_str: &str, command_str: &str) -> (Vec<String>, HookCommandSpec) {
        (
            ours_markers(
                script_path_str,
                &HookCommandSpec {
                    command: command_str.to_string(),
                    command_windows: None,
                    helper_path: None,
                },
            ),
            HookCommandSpec {
                command: command_str.to_string(),
                command_windows: None,
                helper_path: None,
            },
        )
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
        let (markers, _) = bash_spec(r"C:\Users\u\.mam\hooks\status-hook.sh", "x");
        assert!(remove_legacy_camel_key(&mut obj, "Stop", &markers));
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
        let (markers, _) = bash_spec(r"C:\u\.mam\hooks\status-hook.sh", "x");
        assert!(!remove_legacy_camel_key(&mut obj, "Stop", &markers));
        assert!(obj.get("stop").is_some(), "混用户条目不得移除");
    }

    #[test]
    fn absent_or_wrong_case_legacy_key_is_noop() {
        let mut obj = serde_json::Map::new();
        let (markers, _) = bash_spec("/x/status-hook.sh", "x");
        assert!(!remove_legacy_camel_key(&mut obj, "Stop", &markers));
        // PascalCase 键名与 legacy 相同（防御：本就全小写事件名无 twins）
        let mut obj2 = serde_json::Map::new();
        obj2.insert("stop".into(), serde_json::json!([our_entry("x")]));
        let (markers2, _) = bash_spec("/x/status-hook.sh", "x");
        assert!(!remove_legacy_camel_key(&mut obj2, "stop", &markers2));
        assert!(obj2.get("stop").is_some());
    }

    /// T1 扩展：helper 形态条目（command 为 helper 直启命令）同样命中我方判据
    /// ——旧键迁移对 bash→helper 迁移后的文件依然可达
    #[test]
    fn helper_form_entries_are_recognized_as_ours() {
        let mut obj = serde_json::Map::new();
        obj.insert(
            "stop".into(),
            serde_json::json!([our_entry("C:/Users/u/.mam/bin/mam-hook-listener.exe")]),
        );
        let spec = HookCommandSpec {
            command: "C:/Users/u/.mam/bin/mam-hook-listener.exe".to_string(),
            command_windows: None,
            helper_path: Some(r"C:\Users\u\.mam\bin\mam-hook-listener.exe".to_string()),
        };
        assert!(remove_legacy_camel_key(
            &mut obj,
            "Stop",
            &ours_markers("/x/s", &spec)
        ));
        assert!(obj.get("stop").is_none(), "helper 形态旧键也必须移除");
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

        let (_, spec) = bash_spec(marker, &cmd);
        register_hooks_in_file(&cfg, &["Stop", "PreToolUse"], true, marker, &spec).unwrap();

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
        let (_, spec) = bash_spec("/x/status-hook.sh", "bash /x/status-hook.sh");
        let legacy = r#"{"hooks":{"stop":[{"hooks":[{"command":"bash /x/status-hook.sh"}]}]}}"#;
        let modern = r#"{"hooks":{"Stop":[{"hooks":[{"command":"bash /x/status-hook.sh"}]}]}}"#;
        // PascalCase 工具 + 旧 camel 键：command 在场也不得核验（迁移入口保持可达）
        assert!(!hooks_file_verified(legacy, &spec, &["Stop"], true));
        // 全期望键在场：核验
        assert!(hooks_file_verified(modern, &spec, &["Stop"], true));
        // 多事件任缺一键：不核验
        assert!(!hooks_file_verified(
            modern,
            &spec,
            &["Stop", "PreToolUse"],
            true
        ));
        // camelCase 形态工具按 camel 键核验（形态匹配即核验）
        assert!(hooks_file_verified(legacy, &spec, &["stop"], false));
        // command 缺席：不核验
        let (_, other) = bash_spec("/x/status-hook.sh", "bash /other.sh");
        assert!(!hooks_file_verified(modern, &other, &["Stop"], true));
    }

    /// T1 存量迁移可达性：codex 规格下旧 bash 条目（command 与规格 command 完全
    /// 相同、仅缺 commandWindows）不得被判已核验——否则 commandWindows 迁移永不
    /// 触达（与 P1-2 键形态核验同一逻辑的 Windows 覆盖字段版）
    #[test]
    fn hooks_file_verified_requires_command_windows_when_spec_has_one() {
        use super::hooks_file_verified;
        let legacy = r#"{"hooks":{"Stop":[{"hooks":[{"command":"bash /x/status-hook.sh"}]}]}}"#;
        let migrated = r#"{"hooks":{"Stop":[{"hooks":[{"command":"bash /x/status-hook.sh","commandWindows":"C:/u/.mam/bin/mam-hook-listener.exe"}]}]}}"#;
        let spec = HookCommandSpec {
            command: "bash /x/status-hook.sh".to_string(),
            command_windows: Some("C:/u/.mam/bin/mam-hook-listener.exe".to_string()),
            helper_path: Some(r"C:\u\.mam\bin\mam-hook-listener.exe".to_string()),
        };
        assert!(
            !hooks_file_verified(legacy, &spec, &["Stop"], true),
            "缺 commandWindows 的旧条目不得核验通过"
        );
        assert!(
            hooks_file_verified(migrated, &spec, &["Stop"], true),
            "commandWindows 齐备才核验通过"
        );
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

    let script_path = ensure_hook_script();
    let helper = install_hook_listener_helper();
    let spec = hook_command_spec_for("codex", &script_path, helper.as_deref());
    register_hooks_for_tool(&path, &events, is_pascal, &spec).expect("codex hooks 注册失败");

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

/// T1 命令规格纯决策（windows_semantics 显式驱动，双平台语义任意平台可测）
#[cfg(test)]
mod helper_command_spec_tests {
    use super::{helper_command_for, hook_command_spec_for_impl};

    const SCRIPT: &str = r"C:\Users\u\.mam\hooks\status-hook.sh";
    const HELPER: &str = r"C:\Users\u\.mam\bin\mam-hook-listener.exe";

    #[test]
    fn helper_command_normalizes_slashes_and_quotes_spaces() {
        // 与 quote_bash_command 同一套实证结论（正斜杠两层安全 + 含空格保引号），
        // 去掉 bash 前缀——直启形态
        assert_eq!(
            helper_command_for(std::path::Path::new(HELPER)),
            "C:/Users/u/.mam/bin/mam-hook-listener.exe"
        );
        assert_eq!(
            helper_command_for(std::path::Path::new(
                r"C:\Users\John Doe\.mam\bin\mam-hook-listener.exe"
            )),
            "\"C:/Users/John Doe/.mam/bin/mam-hook-listener.exe\""
        );
    }

    #[test]
    fn claude_windows_semantics_spawns_helper_directly() {
        let spec = hook_command_spec_for_impl(
            "claude",
            std::path::Path::new(SCRIPT),
            Some(std::path::Path::new(HELPER)),
            true,
        );
        assert_eq!(
            spec.command, "C:/Users/u/.mam/bin/mam-hook-listener.exe",
            "claude 的 command 必须直启 helper（零 shell 依赖）"
        );
        assert!(
            spec.command_windows.is_none(),
            "claude 无 commandWindows 字段"
        );
        assert_eq!(spec.helper_path.as_deref(), Some(HELPER));
    }

    #[test]
    fn codex_windows_semantics_keeps_bash_command_and_sets_command_windows() {
        let spec = hook_command_spec_for_impl(
            "codex",
            std::path::Path::new(SCRIPT),
            Some(std::path::Path::new(HELPER)),
            true,
        );
        assert_eq!(
            spec.command, "bash C:/Users/u/.mam/hooks/status-hook.sh",
            "codex 的 command 恒为 bash 形态（非 Windows 落地用，行为不变）"
        );
        assert_eq!(
            spec.command_windows.as_deref(),
            Some("C:/Users/u/.mam/bin/mam-hook-listener.exe"),
            "Windows 走官方 commandWindows 直启 helper（cmd 包装下裸 bash 不可解析的根治）"
        );
        assert_eq!(spec.helper_path.as_deref(), Some(HELPER));
    }

    #[test]
    fn unix_semantics_keep_bash_form_without_windows_field() {
        // windows_semantics=false：两工具 command 都回到 hook_command_for 产物
        // （平台原生的 bash 形态——本断言以同一函数为期望，不拼平台路径），
        // 且不得携带 commandWindows / helper 标记（unix 行为与 T1 前完全一致）
        for tool in ["claude", "codex"] {
            let spec = hook_command_spec_for_impl(
                tool,
                std::path::Path::new(SCRIPT),
                Some(std::path::Path::new(HELPER)),
                false,
            );
            assert_eq!(
                spec.command,
                super::hook_command_for(std::path::Path::new(SCRIPT)),
                "unix 语义 = bash 兜底形态"
            );
            assert_eq!(spec.command_windows, None);
            assert_eq!(spec.helper_path, None);
        }
    }

    #[test]
    fn helper_absent_falls_back_to_bash_on_both_platforms() {
        // helper 未随包分发是合法状态：Windows 语义也必须回落 bash 形态（零回归）
        for (tool, win) in [
            ("claude", true),
            ("codex", true),
            ("claude", false),
            ("codex", false),
        ] {
            let spec = hook_command_spec_for_impl(tool, std::path::Path::new(SCRIPT), None, win);
            assert_eq!(spec.command, "bash C:/Users/u/.mam/hooks/status-hook.sh");
            assert_eq!(spec.command_windows, None);
            assert_eq!(spec.helper_path, None);
        }
    }

    #[test]
    fn other_tools_get_bash_fallback_even_with_helper() {
        // kimi 等 hook 未接入工具：规格兜底 bash 形态（注册器 T2 再议）
        let spec = hook_command_spec_for_impl(
            "kimi",
            std::path::Path::new(SCRIPT),
            Some(std::path::Path::new(HELPER)),
            true,
        );
        assert_eq!(spec.command, "bash C:/Users/u/.mam/hooks/status-hook.sh");
        assert_eq!(spec.command_windows, None);
        assert_eq!(spec.helper_path, None);
    }
}

/// T1 存量迁移（register_hooks_in_file 判据扩展：脚本路径 + helper 路径双标记）
#[cfg(test)]
mod helper_migration_tests {
    use super::{register_hooks_in_file, HookCommandSpec};

    const SCRIPT: &str = r"C:\Users\u\.mam\hooks\status-hook.sh";
    const HELPER: &str = r"C:\Users\u\.mam\bin\mam-hook-listener.exe";
    const HELPER_CMD: &str = "C:/Users/u/.mam/bin/mam-hook-listener.exe";

    fn our_entry(cmd: &str) -> serde_json::Value {
        serde_json::json!({ "matcher": "", "hooks": [{ "type": "command", "command": cmd }] })
    }

    fn claude_spec() -> HookCommandSpec {
        HookCommandSpec {
            command: HELPER_CMD.to_string(),
            command_windows: None,
            helper_path: Some(HELPER.to_string()),
        }
    }

    fn codex_spec() -> HookCommandSpec {
        HookCommandSpec {
            command: "bash C:/Users/u/.mam/hooks/status-hook.sh".to_string(),
            command_windows: Some(HELPER_CMD.to_string()),
            helper_path: Some(HELPER.to_string()),
        }
    }

    /// claude 形态迁移：settings.json 旧 bash 条目（正斜杠历史形态）→ command
    /// 原地改写为 helper 直启命令；用户条目不受影响
    #[test]
    fn claude_bash_entry_migrates_to_helper_command() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("settings.json");
        std::fs::write(
            &cfg,
            serde_json::json!({"hooks": {"Stop": [
                our_entry("bash C:/Users/u/.mam/hooks/status-hook.sh"),
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "user-own" }] }
            ]}})
            .to_string(),
        )
        .unwrap();

        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &claude_spec()).unwrap();
        assert_eq!((added, migrated), (0, 1), "应为纯迁移零新增");
        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let entry = &out["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(
            entry["command"], HELPER_CMD,
            "command 必须改写为 helper 直启"
        );
        assert!(
            entry.get("commandWindows").is_none(),
            "claude 条目不得带 commandWindows 字段"
        );
        assert_eq!(
            out["hooks"]["Stop"][1]["hooks"][0]["command"], "user-own",
            "用户条目必须原样保留"
        );
    }

    /// codex 形态迁移：hooks.json 旧 bash 条目 → command 保持 bash（非 Windows
    /// 落地用）+ 注入官方 commandWindows 字段指向 helper
    #[test]
    fn codex_bash_entry_gains_command_windows() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("hooks.json");
        std::fs::write(
            &cfg,
            serde_json::json!({"hooks": {"Stop": [our_entry("bash C:/Users/u/.mam/hooks/status-hook.sh")]}})
                .to_string(),
        )
        .unwrap();

        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &codex_spec()).unwrap();
        assert_eq!((added, migrated), (0, 1), "commandWindows 注入计入迁移");
        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let entry = &out["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(
            entry["command"], "bash C:/Users/u/.mam/hooks/status-hook.sh",
            "codex 的 command 保持 bash 形态（unix 行为不变）"
        );
        assert_eq!(entry["commandWindows"], HELPER_CMD);
    }

    /// codex 完全迁移后的稳定态：command + commandWindows 均与规格一致 → already
    /// 跳过、零改动（文件不重写，字节不变）
    #[test]
    fn codex_fully_migrated_entry_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("hooks.json");
        let before = serde_json::json!({"hooks": {"Stop": [serde_json::json!({
            "matcher": "",
            "hooks": [{ "type": "command", "command": "bash C:/Users/u/.mam/hooks/status-hook.sh",
                        "commandWindows": HELPER_CMD }]
        })]}})
        .to_string();
        std::fs::write(&cfg, &before).unwrap();

        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &codex_spec()).unwrap();
        assert_eq!((added, migrated), (0, 0), "稳定态零新增零迁移");
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            before,
            "稳定态不得重写文件"
        );
    }

    /// helper 路径变更（升级换位置）等场景：旧 commandWindows 值原地刷新
    #[test]
    fn stale_command_windows_value_is_refreshed() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("hooks.json");
        std::fs::write(
            &cfg,
            serde_json::json!({"hooks": {"Stop": [our_entry("bash C:/Users/u/.mam/hooks/status-hook.sh")]}})
                .to_string(),
        )
        .unwrap();
        // 第一轮：注入 commandWindows
        register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &codex_spec()).unwrap();
        // 第二轮（规格换新 helper 路径）：旧值必须被刷新，不产生双条目
        let new_spec = HookCommandSpec {
            command: "bash C:/Users/u/.mam/hooks/status-hook.sh".to_string(),
            command_windows: Some("C:/new/bin/mam-hook-listener.exe".to_string()),
            helper_path: Some(r"C:\new\bin\mam-hook-listener.exe".to_string()),
        };
        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &new_spec).unwrap();
        assert_eq!((added, migrated), (0, 1));
        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let entries = out["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "不得追加双条目");
        assert_eq!(
            entries[0]["hooks"][0]["commandWindows"],
            "C:/new/bin/mam-hook-listener.exe"
        );
    }

    /// 非我方条目不动（helper 形态含脚本路径误写用户命令的边界也不误伤——判据
    /// 是「含标记」而非「等于标记」与 F3 语义一致，但用户命令不含任何标记）
    #[test]
    fn non_mam_entries_are_untouched_and_still_get_ours_appended() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("settings.json");
        std::fs::write(
            &cfg,
            serde_json::json!({"hooks": {"Stop": [
                { "matcher": "", "hooks": [{ "type": "command", "command": "my-own-listener" }] }
            ]}})
            .to_string(),
        )
        .unwrap();

        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &claude_spec()).unwrap();
        assert_eq!((added, migrated), (1, 0), "仅追加我方条目");
        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let entries = out["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0]["hooks"][0]["command"], "my-own-listener",
            "用户条目不动"
        );
        assert_eq!(entries[1]["hooks"][0]["command"], HELPER_CMD);
    }

    /// 空配置冷注册：claude 规格直接落 helper 命令条目
    #[test]
    fn fresh_claude_registration_writes_helper_command() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("settings.json");
        let (added, migrated) =
            register_hooks_in_file(&cfg, &["Stop"], true, SCRIPT, &claude_spec()).unwrap();
        assert_eq!((added, migrated), (1, 0));
        let out: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(out["hooks"]["Stop"][0]["hooks"][0]["command"], HELPER_CMD);
    }
}

/// T1 helper 安装管道（install_marker_helper 先例扩展的共用核心）
#[cfg(test)]
mod helper_install_tests {
    use super::{helper_install_dir, install_helper_from_to};

    #[test]
    fn install_dir_is_home_mam_bin() {
        let home = std::path::Path::new("/home/u");
        assert_eq!(
            helper_install_dir(home),
            std::path::Path::new("/home/u").join(".mam").join("bin")
        );
    }

    #[test]
    fn copies_first_existing_candidate_with_dotted_names() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        // 两个候选都在：Windows 发行 .exe 序在前（与 mam-marker 候选序一致）
        std::fs::write(src.path().join("mam-hook-listener.exe"), b"exe").unwrap();
        std::fs::write(src.path().join("mam-hook-listener"), b"bare").unwrap();
        let names = ["mam-hook-listener.exe", "mam-hook-listener"];
        let installed =
            install_helper_from_to(src.path(), dst.path(), &names).expect("候选在场必须安装成功");
        assert_eq!(
            installed,
            dst.path().join("mam-hook-listener.exe"),
            "按候选序取首个存在者"
        );
        assert_eq!(
            std::fs::read(dst.path().join("mam-hook-listener.exe")).unwrap(),
            b"exe"
        );
    }

    #[test]
    fn falls_back_to_bare_name_when_exe_absent() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("mam-hook-listener"), b"bare").unwrap();
        let names = ["mam-hook-listener.exe", "mam-hook-listener"];
        let installed = install_helper_from_to(src.path(), dst.path(), &names)
            .expect("裸名候选在场必须安装成功");
        assert_eq!(installed, dst.path().join("mam-hook-listener"));
    }

    #[test]
    fn missing_candidates_yield_none_and_dst_dir_created() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let target = dst.path().join("bin");
        assert!(
            install_helper_from_to(src.path(), &target, &["mam-hook-listener.exe"]).is_none(),
            "候选全缺 → None（注册回落 bash 形态的合法状态）"
        );
        assert!(target.is_dir(), "目标目录仍应创建（幂等分发语义）");
    }

    #[test]
    fn overwrite_keeps_installed_helper_fresh() {
        // 升级覆盖语义：无条件重拷保证新版 helper 生效（mam-marker 先例）
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let names = ["mam-hook-listener.exe"];
        std::fs::write(src.path().join("mam-hook-listener.exe"), b"v1").unwrap();
        install_helper_from_to(src.path(), dst.path(), &names).unwrap();
        std::fs::write(src.path().join("mam-hook-listener.exe"), b"v2").unwrap();
        install_helper_from_to(src.path(), dst.path(), &names).unwrap();
        assert_eq!(
            std::fs::read(dst.path().join("mam-hook-listener.exe")).unwrap(),
            b"v2",
            "重装必须覆盖旧版"
        );
    }
}

/// T1 事件格式 round-trip：helper 产出的事件文件必须被读取侧原样认得
#[cfg(test)]
mod helper_event_roundtrip_tests {
    use super::read_hook_events_from;
    use crate::monitor::hook_listener::{event_body, now_unix, parse_hook_stdin, write_event_file};

    /// 写侧（helper 内核）→ 读侧（read_hook_events_from）全链：文件名键 / event /
    /// ts / last_event_at 逐字段一致；同会话覆盖写后读到最新
    #[test]
    fn helper_event_file_roundtrips_through_reader() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_payload = r#"{"session_id":"01a08083-5ca0-4948-8276-9a0b8c7d6e5f",
            "hook_event_name":"Stop","cwd":"E:\\proj","transcript_path":"/x.jsonl"}"#;
        let parsed = parse_hook_stdin(claude_payload).expect("claude 样本必须可解析");

        let ts1 = now_unix();
        write_event_file(tmp.path(), &parsed, ts1).unwrap();
        let m = read_hook_events_from(tmp.path());
        assert_eq!(m.len(), 1, "单会话单卡");
        let ev = m
            .get("01a08083-5ca0-4948-8276-9a0b8c7d6e5f")
            .expect("文件名即键");
        assert_eq!(ev.event, "Stop");
        assert_eq!(ev.ts, ts1);
        assert_eq!(
            ev.last_event_at,
            event_body(&parsed, ts1)
                .split_once("\"last_event_at\":\"")
                .and_then(|(_, rest)| rest.split('"').next())
                .unwrap_or_default(),
            "last_event_at 与写侧同源"
        );

        // 覆盖写（同会话新事件）→ 读取侧拿到最新 ts（保留最新状态语义）
        let ts2 = ts1 + 2;
        write_event_file(tmp.path(), &parsed, ts2).unwrap();
        let m = read_hook_events_from(tmp.path());
        assert_eq!(m["01a08083-5ca0-4948-8276-9a0b8c7d6e5f"].ts, ts2);
    }

    /// 白名单/临时文件边界：helper 不会为非法 sid 落盘；原子写中间态（*.tmp）
    /// 永不入读取侧键集
    #[test]
    fn reader_skips_helper_tmp_files_and_illegal_names() {
        let tmp = tempfile::tempdir().unwrap();
        let ts = now_unix();
        let body = format!(
            r#"{{"event":"Stop","session_id":"sid-ok","cwd":"","ts":{ts},"last_event_at":"2026-09-20T00:00:00Z"}}"#
        );
        std::fs::write(tmp.path().join("sid-ok.json"), &body).unwrap();
        // helper 原子写中间态形态（<sid>.<pid>.tmp）
        std::fs::write(tmp.path().join("sid-ok.424242.tmp"), &body).unwrap();
        // 非法 sid 文件（路径注入形态）
        std::fs::write(tmp.path().join("bad.name.json"), &body).unwrap();

        let m = read_hook_events_from(tmp.path());
        assert_eq!(m.len(), 1, "只认白名单 sid 的 .json: {:?}", m.keys());
        assert!(m.contains_key("sid-ok"));
    }
}
