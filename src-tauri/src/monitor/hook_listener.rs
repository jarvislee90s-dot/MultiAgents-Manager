//! hook 事件监听内核（批次甲 T1 · 原生 helper 替换 bash —— issue #74 根因 2）
//!
//! 本模块是 mam-hook-listener helper 与主程序（`monitor::hooks` 读取侧）共享的
//! 纯逻辑层：bin 侧经 `#[path]` 引入（独立编译单元，不链接整个 lib——mam-marker
//! 独立二进制先例，保证 helper 体积小、启动毫秒级），lib 侧作为常规子模块。因此
//! 本模块 **禁止引用 `crate::`**，仅依赖 std + serde_json + dirs（零新依赖）。
//!
//! # 实现红线（任务书 §1，违反即返工；文末单测同步断言）
//!
//! 1. **监听模式统一 exit 0 + 空 stdout**——codex 侧 exit 2+stderr = Deny（劫持
//!    审批）、JSON stdout = 劫持审批框（与 claude 语义相反，claude 的 exit 2 不
//!    生效）。任何输入（非法 JSON / 空 stdin / 写盘失败 / panic 前的一切错误路径）
//!    都不得产生 stdout 字节与非零退出码；stderr 保守起见同样全静默（helper 无
//!    log 初始化，诊断依赖落盘结果本身）。
//! 2. **瞬时完成**——codex 命令钩子默认 600s 超时、Interrupt/SessionEnd 仅 1s：
//!    读 stdin → serde 解析 → 同目录临时文件+rename 原子写 → 退出，毫秒级；
//!    无网络、无重试、无等待、无 fsync（TTL 30s 的临时数据不值得付落盘屏障延迟）。
//! 3. `async:true` 钩子被 codex 跳过——本内核天然同步快速；注册侧不得带 async
//!    （T2 落实，见 hooks.rs）。
//! 4. payload 差异（claude/codex/kimi 的 tool_input / message 等）是 T2/T3 的事——
//!    本内核只做「stdin JSON → session_id / hook_event_name → 事件文件」薄管道。
//!
//! # 事件文件格式（以读取侧 `monitor::hooks::read_hook_events_from` 为准，
//! 自 HOOK_SCRIPT bash 版逐字段移植）
//!
//! 路径 `<events_dir>/<session_id>.json`（session_id 白名单 `[A-Za-z0-9-]` 防路径
//! 注入，合法 UUID 形态永不触发；helper 侧另加 128 字符防御上限，真身 UUID ≤ 36），
//! 内容单行 JSON：`{"event","session_id","cwd","ts"(unix 秒),"last_event_at"(UTC
//! ISO8601)}`——读取侧 `HookEvent` 消费 event/ts/last_event_at，30s TTL。同会话
//! 覆盖写 = 保留最新状态（bash 版语义）。

use serde_json::json;

/// stdin 解析产物（薄管道三字段：事件文件格式所需的最小集）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHook {
    /// 会话 id（事件文件名键；白名单已校验）
    pub session_id: String,
    /// hook_event_name 原文（PascalCase，三家一致：claude/codex 官方 payload 均为
    /// PascalCase，kimi 同构）
    pub event_name: String,
    /// 工作目录（payload `cwd`，缺失记空串——bash 版 grep 不到时的同款语义）
    pub cwd: String,
}

/// session_id 白名单（HOOK_SCRIPT `^[A-Za-z0-9-]+$` 同源移植 + 128 字符防御上限）：
/// 非空 + 仅 ASCII 字母数字与连字符。文件名 = `<session_id>.json`，白名单外的值
/// 直接丢弃（防路径注入；合法 UUID 形态永不触发）。与读取侧
/// `monitor::hooks::read_hook_events_from` 的 valid_sid 同一谓词（读取侧已改为
/// 复用本函数，单一事实源）
pub fn session_id_allowed(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 解析 hook stdin JSON（claude/codex/kimi 三家同形态：snake_case 键 + 公共字段
/// session_id / hook_event_name / cwd）。缺失/类型非法/白名单不过 → None（bin 层
/// 静默跳过写盘，红线 1）。**注意是 from_str 非 from_slice**：非法 UTF-8 在
/// bin 层 read_to_string 就已失败，到不了这里
pub fn parse_hook_stdin(raw: &str) -> Option<ParsedHook> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let session_id = value.get("session_id")?.as_str()?.to_string();
    let event_name = value.get("hook_event_name")?.as_str()?.to_string();
    if !session_id_allowed(&session_id) || event_name.is_empty() {
        return None;
    }
    let cwd = value
        .get("cwd")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    Some(ParsedHook {
        session_id,
        event_name,
        cwd,
    })
}

/// 当前 unix 时间（秒）。时钟倒退/系统无时钟等极端态返回 0（事件文件 30s TTL
/// 下 0 值只会让事件即刻过期，无害且不阻塞审批流——红线 1 优先于数据保真）
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// unix 秒 → `YYYY-MM-DDTHH:MM:SSZ`（UTC ISO8601，bash 版 `date -u` 同款格式；
/// 读取侧仅作字符串透传，TTL 判定只用 ts）。纯 std 实现（helper 不链接 chrono，
/// 体积/启动最紧），日期换算用 Howard Hinnant civil_from_days 公有域算法
pub fn format_event_time(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    let secs = ts.rem_euclid(86_400);
    let (hh, mm, ss) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// 天数（自 1970-01-01）→ (年, 月, 日)。Howard Hinnant 公有域算法（chrono 内部
/// 同款），负数天（1970 前）亦正确
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 事件文件正文（与 HOOK_SCRIPT bash 版逐字段同构；serde_json 序列化天然转义，
/// 根除 bash 版 grep+模板拼接对特殊字符不设防的注入面——调研 §2.4 遗留项）。
/// 键序 event/session_id/cwd/ts/last_event_at 与 bash 版一致（preserve_order）
pub fn event_body(parsed: &ParsedHook, ts: i64) -> String {
    json!({
        "event": parsed.event_name,
        "session_id": parsed.session_id,
        "cwd": parsed.cwd,
        "ts": ts,
        "last_event_at": format_event_time(ts),
    })
    .to_string()
}

/// 写事件文件（瞬时 + 原子）：同目录临时文件 + rename 覆盖（Windows 上
/// std::fs::rename = MoveFileExW(MOVEFILE_REPLACE_EXISTING)，替换语义成立）——
/// 读取侧永不读到半截。临时名不带 .json 后缀 → 读取侧 strip_suffix(".json")
/// 天然跳过。失败返回 Err（bin 层吞掉，红线 1：错误路径零输出非零退出）
pub fn write_event_file(
    events_dir: &std::path::Path,
    parsed: &ParsedHook,
    ts: i64,
) -> std::io::Result<()> {
    std::fs::create_dir_all(events_dir)?;
    let body = event_body(parsed, ts);
    // sid+pid 双唯一：同会话并发 helper（不同进程）互不踩临时文件
    let tmp = events_dir.join(format!("{}.{}.tmp", parsed.session_id, std::process::id()));
    let dst = events_dir.join(format!("{}.json", parsed.session_id));
    std::fs::write(&tmp, body.as_bytes())?;
    match std::fs::rename(&tmp, &dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            // 覆盖失败不留半截临时文件（目录被杀毒锁住等瞬态；下一个事件自然重试）
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// 应用数据主目录（与 `database/connection.rs::app_data_home` 同口径——helper 独立
/// 编译单元无法复用 lib，两处同步维护）：MAM_HOME 仅 debug/test 构建生效（测试
/// 重定向数据目录；Windows 下 dirs::home_dir 无法用 HOME 重定向），release 生产
/// 恒为真实用户目录，防环境变量误设导致读写割裂
pub fn app_data_home() -> std::path::PathBuf {
    if cfg!(debug_assertions) {
        if let Some(home) = std::env::var_os("MAM_HOME") {
            if !home.is_empty() {
                return std::path::PathBuf::from(home);
            }
        }
    }
    dirs::home_dir().unwrap_or_default()
}

/// 事件目录（helper 写侧与读取侧 read_hook_events 的同源定位点：写读两侧经同一
/// 函数出路径，任何配置下都不会互看不到对方）
pub fn default_events_dir() -> std::path::PathBuf {
    app_data_home().join(".mam").join("events")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 红线 1 的内核面：一切非法输入 → None（bin 层据此零落盘零输出） ----------

    /// 合法 payload（claude 形态样本：官方公共字段 + Notification 专属字段照收不炸）
    #[test]
    fn parse_claude_shape_payload() {
        let raw = r#"{
            "session_id":"01a08083-5ca0-4948-8276-9a0b8c7d6e5f",
            "transcript_path":"/home/u/.claude/projects/x/01a08083.jsonl",
            "cwd":"E:\\proj\\demo","hook_event_name":"Stop",
            "permission_mode":"default","effort":"high"
        }"#;
        let parsed = parse_hook_stdin(raw).expect("合法 claude payload 必须解析成功");
        assert_eq!(parsed.session_id, "01a08083-5ca0-4948-8276-9a0b8c7d6e5f");
        assert_eq!(parsed.event_name, "Stop");
        assert_eq!(parsed.cwd, "E:\\proj\\demo");
    }

    /// codex 形态（PascalCase 事件 + tool_name/tool_input 附加字段——薄管道照收，
    /// payload 差异是 T2/T3 的事）
    #[test]
    fn parse_codex_permission_request_payload() {
        let raw = r#"{
            "session_id":"0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f",
            "turn_id":"t1","agent_id":"a1","cwd":"/home/u/proj",
            "hook_event_name":"PermissionRequest","model":"gpt-5",
            "permission_mode":"on-request","tool_name":"shell",
            "tool_input":{"command":"rm -rf /","description":"清理构建产物"}
        }"#;
        let parsed = parse_hook_stdin(raw).expect("合法 codex payload 必须解析成功");
        assert_eq!(parsed.event_name, "PermissionRequest");
        assert_eq!(parsed.session_id, "0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f");
    }

    /// kimi 形态（调研 §2.4/§3.2：stdin JSON 三家同构）——同一薄管道零分支兼容
    #[test]
    fn parse_kimi_shape_payload_is_same_pipe() {
        let raw =
            r#"{"session_id":"kimi-abc123","hook_event_name":"PermissionRequest","cwd":"/w"}"#;
        let parsed = parse_hook_stdin(raw).expect("kimi 同构 payload 必须解析成功");
        assert_eq!(parsed.event_name, "PermissionRequest");
        assert_eq!(parsed.session_id, "kimi-abc123");
    }

    #[test]
    fn parse_rejects_missing_session_id() {
        assert!(parse_hook_stdin(r#"{"hook_event_name":"Stop","cwd":"/w"}"#).is_none());
    }

    #[test]
    fn parse_rejects_missing_event_name() {
        assert!(parse_hook_stdin(r#"{"session_id":"sid-1","cwd":"/w"}"#).is_none());
    }

    #[test]
    fn parse_rejects_empty_event_name() {
        assert!(parse_hook_stdin(r#"{"session_id":"sid-1","hook_event_name":""}"#).is_none());
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_hook_stdin("not json at all").is_none());
        assert!(parse_hook_stdin("{\"session_id\":").is_none());
    }

    #[test]
    fn parse_rejects_empty_input() {
        assert!(parse_hook_stdin("").is_none());
        assert!(parse_hook_stdin("   \n\t").is_none());
    }

    #[test]
    fn parse_rejects_non_string_fields() {
        // 类型非法（数字/对象形态的 session_id / hook_event_name）按缺失处置
        assert!(parse_hook_stdin(r#"{"session_id":123,"hook_event_name":"Stop"}"#).is_none());
        assert!(
            parse_hook_stdin(r#"{"session_id":"sid","hook_event_name":{"e":"Stop"}}"#).is_none()
        );
    }

    #[test]
    fn parse_rejects_illegal_session_id() {
        // 白名单外：路径注入形态 / 点号 / 空串 / 非 ASCII
        for bad_sid in ["../evil", "a/b", "..", "a.b", "", "会话一", "sid space"] {
            let raw = format!(r#"{{"session_id":"{bad_sid}","hook_event_name":"Stop"}}"#);
            assert!(
                parse_hook_stdin(&raw).is_none(),
                "非法 session_id {bad_sid:?} 必须拒绝"
            );
        }
    }

    #[test]
    fn parse_rejects_overlong_session_id() {
        let ok = "a".repeat(128);
        let too_long = "a".repeat(129);
        assert!(session_id_allowed(&ok));
        assert!(!session_id_allowed(&too_long));
        let raw = format!(r#"{{"session_id":"{too_long}","hook_event_name":"Stop"}}"#);
        assert!(parse_hook_stdin(&raw).is_none(), "超长 sid 防御上限生效");
    }

    // ---------- 时间戳（纯 std，已知时刻锚定） ----------

    #[test]
    fn format_event_time_known_values() {
        assert_eq!(format_event_time(0), "1970-01-01T00:00:00Z");
        // 2026-09-12T00:00:00Z（bash 版脚本注释同日锚）
        assert_eq!(format_event_time(1_789_171_200), "2026-09-12T00:00:00Z");
        assert_eq!(format_event_time(1_789_171_261), "2026-09-12T00:01:01Z");
        // 1970 前（负 ts，div_euclid/rem_euclid 正确处理）
        assert_eq!(format_event_time(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn now_unix_is_epoch_seconds() {
        let now = now_unix();
        assert!(now > 1_700_000_000, "now_unix 应为 2023+ 的 unix 秒: {now}");
    }

    // ---------- 事件文件构造与原子写 ----------

    #[test]
    fn event_body_has_reader_shape() {
        let parsed = ParsedHook {
            session_id: "sid-x".into(),
            event_name: "Stop".into(),
            cwd: "/tmp".into(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&event_body(&parsed, 1_789_171_200)).unwrap();
        assert_eq!(v["event"], "Stop");
        assert_eq!(v["session_id"], "sid-x");
        assert_eq!(v["cwd"], "/tmp");
        assert_eq!(v["ts"], 1_789_171_200);
        assert_eq!(v["last_event_at"], "2026-09-12T00:00:00Z");
    }

    #[test]
    fn write_event_file_roundtrip_overwrite_and_no_tmp_residue() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested").join("events"); // 目录不存在 → 自动创建
        let parsed = ParsedHook {
            session_id: "01a08083-5ca0".into(),
            event_name: "Stop".into(),
            cwd: "/w".into(),
        };
        write_event_file(&dir, &parsed, 1_789_171_200).unwrap();
        let dst = dir.join("01a08083-5ca0.json");
        let first = std::fs::read_to_string(&dst).unwrap();
        assert_eq!(first, event_body(&parsed, 1_789_171_200));

        // 同会话覆盖写 = 保留最新状态（bash 版语义；rename 的替换语义承载）
        let second_ts = 1_789_171_200 + 5;
        write_event_file(&dir, &parsed, second_ts).unwrap();
        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            event_body(&parsed, second_ts),
            "覆盖写必须以最新事件为准"
        );

        // 原子写的临时文件零残留（读取侧视角只有 .json）
        let residue: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            residue,
            vec!["01a08083-5ca0.json".to_string()],
            "{residue:?}"
        );
    }

    #[test]
    fn write_event_file_failure_is_err_not_panic() {
        // events_dir 路径穿过一个普通文件 → create_dir_all 必败：错误以 Err 返回
        //（bin 层吞掉），绝不 panic——红线 1「panic 前的一切错误路径静默」的内核面
        let tmp = tempfile::tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let parsed = ParsedHook {
            session_id: "sid".into(),
            event_name: "Stop".into(),
            cwd: String::new(),
        };
        assert!(write_event_file(&blocker.join("events"), &parsed, 0).is_err());
    }

    #[test]
    fn cwd_defaults_to_empty_when_missing() {
        let parsed = parse_hook_stdin(r#"{"session_id":"sid","hook_event_name":"Stop"}"#).unwrap();
        assert_eq!(
            parsed.cwd, "",
            "cwd 缺失记空串（bash 版 grep 不到同款语义）"
        );
    }
}
