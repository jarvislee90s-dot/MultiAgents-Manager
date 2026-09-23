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
//!    读 stdin → serde 解析 → **同会话旧事件文件一次读**（T1 承接窗，见
//!    [`with_carried_question_fields`]）→ 同目录临时文件+rename 原子写 → 退出，
//!    毫秒级；无网络、无重试、无等待、无 fsync（TTL 30s 的临时数据不值得付落盘
//!    屏障延迟）。承接读的最坏耗时 = 同目录 ≤（64KB tool_input + 4KB message，
//!    约 68KB）文件的一次 page-cached 读 + parse；失败即降级为不承接（不是错误
//!    路径，是保守回退），仍是毫秒级。
//! 3. `async:true` 钩子被 codex 跳过——本内核天然同步快速；注册侧不得带 async
//!    （T2 落实，见 hooks.rs）。
//! 4. payload 差异（claude/codex/kimi 的 tool_input / message 等）是 T2/T3 的事——
//!    本内核只做「stdin JSON → session_id / hook_event_name → 事件文件」薄管道。
//!    **T8 例外（AskUserQuestion 问答通道）**：`tool_name` 全事件照收（小字符串）；
//!    `tool_input` **仅当** PreToolUse ∧ tool_name=="AskUserQuestion" 时附加（64KB
//!    上限，超限整字段丢弃）——这是问答卡片的实时识别通道（通道 A）。实机取证
//!    （2026-09-21 探测档案 research/refs/phase2-消息注入/
//!    2026-09-21-claude-askuserquestion-按键语义探测.md）：问题 UI 弹出期间 pending
//!    的 tool_use **不落盘**（会话 JSONL 停在 user 消息），纯文件路径实时不可达，
//!    故问答识别的主通道是 claude 投递给 helper 的 PreToolUse hook payload
//!    `{hook_event_name:"PreToolUse", tool_name:"AskUserQuestion",
//!    tool_input:{questions:[…]}}`。
//!    **批次丙 T1 例外（Notification 语义 + 未决问答承接）**：`message` **仅当**
//!    hook_event_name=="Notification" 时附加（4KB 上限、**前缀截断**保留；语义
//!    判据的锚点在 message 前部，整字段丢弃会让消费侧失据，故与 tool_input 的
//!    整字段丢弃策略有意不同）。**但实机取证推翻了「用 message 判问答」的假设**：
//!    claude 的 permission_prompt 对真实审批与 AUQ 待答发出的 message **逐字相同**
//!    （均为 `Claude needs your permission`，两场景各实测一次）——message 只作
//!    诊断留痕；真正的判据锚点是 **tool_name**（见下）。
//!    **未决问答承接窗**（[`with_carried_question_fields`]）：claude 对 AUQ 待答的
//!    事件序是 `PreToolUse(AUQ)` → `PermissionRequest(AUQ)` → `Notification`，
//!    事件文件同会话覆盖写 → 读取侧常只见到最后那条不带工具名的 Notification。
//!    故写盘前把同会话**新鲜的** AUQ 进入信号（PreToolUse/PermissionRequest，
//!    30s 窗）的 tool_name/tool_input 承接到本次 Notification 上——消费侧因此仍能
//!    按 tool_name 判出问答。取证见 research/refs/phase2-消息注入/
//!    2026-09-21-claude-notification-message-取证.md。
//!
//! # 事件文件格式（以读取侧 `monitor::hooks::read_hook_events_from` 为准，
//! 自 HOOK_SCRIPT bash 版逐字段移植）
//!
//! 路径 `<events_dir>/<session_id>.json`（session_id 白名单 `[A-Za-z0-9_-]` 防路径
//! 注入，合法 UUID / kimi `session_<uuid>` 形态永不触发；helper 侧另加 128 字符
//! 防御上限，真身 ≤ 44），
//! 内容单行 JSON：`{"event","session_id","cwd","ts"(unix 秒),"last_event_at"(UTC
//! ISO8601)}`——读取侧 `HookEvent` 消费 event/ts/last_event_at，30s TTL。同会话
//! 覆盖写 = 保留最新状态（bash 版语义）。T8 起两个**向后兼容的可选字段**：payload
//! 带 `tool_name`（非空）时追加 `"tool_name"`；PreToolUse ∧ AskUserQuestion 时追加
//! `"tool_input"`（tool_input 的原文 JSON 串）。T1 起第三个可选字段：Notification
//! 事件且 payload **带 `message` 键**（含空串）时追加 `"message"`（4KB 前缀截断）
//! ——空串照落键是刻意的：「观测到 message 但为空」本身有诊断价值，且读取侧
//! `Option<String>` 下空串与缺席同义，不影响消费。
//! 其余事件（无 tool_name / 无 message）正文与 T8 前**逐字节一致**（读取侧 serde
//! default 兼容 bash 兜底脚本形态——bash 解析嵌套 tool_input 不可靠，问题通道不
//! 承载，见 hooks.rs HOOK_SCRIPT 注释）。

use serde_json::json;

/// AskUserQuestion 工具名（T8 问答通道唯一识别名，claude 官方工具名原文）。
/// 写侧（本模块问答分支）与读取侧（adapter 状态链 / 端点通道 B）共用同一常量，
/// 单一事实源。本模块是 helper 独立编译单元，常量必须落在无 crate 依赖的这里
pub const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";

/// tool_input 附加字段字节上限（T8 任务书：64KB，超限截断丢弃该字段）。questions
/// 真实形态是数百字节的选项表，上限只防御病态 payload（巨型 descriptions 等）；
/// 丢弃策略=整字段 None（截半截 JSON 会产出不可解析的孤儿字段，不如不给）
pub const TOOL_INPUT_MAX_BYTES: usize = 64 * 1024;

/// Notification.message 附加字段字节上限（批次丙 T1：4KB）。message 是纯文本而非
/// JSON——claude 的真实模板为 `Claude needs your permission to use <summary>`（数十
/// 字节），4KB 只防御病态 payload。**截断策略=前缀截断保留**（与 tool_input 的整
/// 字段丢弃不同）：消费侧判据的锚点在 message 前部（工具名/通知语义短语），整字段
/// 丢弃会让判据失据、退回「审批默认」= 幽灵审批标记回归。截断按**字符边界**安全
/// 切分（多字节 UTF-8 不得切半）
pub const NOTIFICATION_MESSAGE_MAX_BYTES: usize = 4 * 1024;

/// 按字符边界安全截断到 ≤ max_bytes（不做字节硬切——UTF-8 切半会产出不可解码的
/// 字符串）。超限则保留前缀；未超限原样返回
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// stdin 解析产物（薄管道三字段 + T8 问答通道两可选字段 + T1 通知语义一可选字段）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHook {
    /// 会话 id（事件文件名键；白名单已校验）
    pub session_id: String,
    /// hook_event_name 原文（PascalCase，三家一致：claude/codex 官方 payload 均为
    /// PascalCase，kimi 同构）
    pub event_name: String,
    /// 工作目录（payload `cwd`，缺失记空串——bash 版 grep 不到时的同款语义）
    pub cwd: String,
    /// payload `tool_name`（T8 全事件照收；缺失记空串。claude PreToolUse 携带
    /// 被调工具名，codex PermissionRequest 也带——照收不分支，消费侧按需判定）
    pub tool_name: String,
    /// `tool_input` 原文 JSON 串（T8 问答通道：**仅当** PreToolUse ∧
    /// tool_name==AskUserQuestion 时携带，64KB 上限；其余事件恒 None——事件文件
    /// 体积与 bash 兜底形态兼容）
    pub tool_input: Option<String>,
    /// `message` 通知正文（批次丙 T1：**仅当** hook_event_name=="Notification" 时
    /// 携带，4KB 前缀截断上限；其余事件恒 None）。claude 的 Notification payload 带
    /// `message`/`title`/`notification_type` 三字段，permission_prompt 类型对
    /// AskUserQuestion 待答也照发。**实机取证结论：该文本不具判别力**——真实审批与
    /// AUQ 待答的 message 逐字相同（均为 `Claude needs your permission`），故消费侧
    /// 只作诊断留痕，判据锚点是 `tool_name`（见 adapter::is_question_entry_event）。
    /// **空串照携带**（payload 带该键即为观测事实），读取侧与缺席同义
    pub notification_message: Option<String>,
}

/// session_id 白名单（HOOK_SCRIPT `^[A-Za-z0-9-]+$` 同源移植 + 128 字符防御上限，
/// **F8 实测扩下划线**）：非空 + 仅 ASCII 字母数字、连字符与下划线。文件名 =
/// `<session_id>.json`，白名单外的值直接丢弃（防路径注入；`.`/`/`/`\` 仍拒绝，
/// 故无穿越面）。**下划线是 kimi 的实际形态**（`session_<uuid>`，2026-09-21 实机
/// 取证：kimi 2.0.2 stdin `session_id` 与 session_index.jsonl 的 sessionId 均为
/// 该前缀形态）——旧白名单 `[A-Za-z0-9-]` 会把 kimi 全部事件静默丢弃（helper
/// exit 0 无输出、事件文件不落，表现为「钩子注册了但状态链永远不动」）。
/// 与读取侧 `monitor::hooks::read_hook_events_from` 的 valid_sid 同一谓词
/// （读取侧复用本函数，单一事实源）
pub fn session_id_allowed(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
    // T8 问答通道（通道 A 写侧）：tool_name 全事件照收（缺失空串）；tool_input
    // 仅在「PreToolUse ∧ AskUserQuestion」分支序列化原文（64KB 上限整字段丢弃）
    let tool_name = value
        .get("tool_name")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    // T8 问答通道（通道 A 写侧）载荷捕获：AUQ 的 tool_input 在 **PreToolUse 与
    // PermissionRequest 两个事件上都到达**（T1 实机取证：PermissionRequest 携带
    // tool_name + 完整 tool_input，是问题卡的第二个载荷源——见
    // 2026-09-21-claude-notification-message-取证.md）
    let tool_input = if matches!(event_name.as_str(), "PreToolUse" | "PermissionRequest")
        && tool_name == ASK_USER_QUESTION_TOOL
    {
        value.get("tool_input").and_then(|ti| {
            let s = ti.to_string();
            if s.len() <= TOOL_INPUT_MAX_BYTES {
                Some(s)
            } else {
                None // 超限：整字段丢弃（截半截 JSON 不可解析，不如不给）
            }
        })
    } else {
        None
    };
    // T1 通知语义通道：message 仅在 Notification 事件附加（其余事件的 message
    // 字段语义未被取证，不无差别照收——事件文件体积与兼容面最小化）；超 4KB 前缀
    // 截断保留（判据锚点在前部，见 NOTIFICATION_MESSAGE_MAX_BYTES 注释）。
    // **实测语义边界**：claude 的 permission_prompt 对「真实审批」与「AskUserQuestion
    // 待答」发出的 message **逐字相同**（均为 `Claude needs your permission`），
    // 单凭该文本无法判别两者——故本字段在消费侧只作次级判据 + 诊断留痕，主判据是
    // PermissionRequest 的 tool_name（见 adapter::apply_hook_event_to_session）
    let notification_message = if event_name == "Notification" {
        value
            .get("message")
            .and_then(|m| m.as_str())
            .map(|m| truncate_on_char_boundary(m, NOTIFICATION_MESSAGE_MAX_BYTES))
    } else {
        None
    };
    Some(ParsedHook {
        session_id,
        event_name,
        cwd,
        tool_name,
        tool_input,
        notification_message,
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
/// 键序 event/session_id/cwd/ts/last_event_at 与 bash 版一致（preserve_order）。
/// T8 可选字段：`tool_name`（payload 带非空 tool_name 时追加）与 `tool_input`
/// （PreToolUse ∧ AskUserQuestion 时追加）——两者缺席时正文与 T8 前逐字节一致
/// （非问答事件零变化回归锁见 `event_body_without_optional_fields_is_legacy_shape`）。
/// T1 可选字段：`message`（Notification 事件携带 payload `message` 时追加）——
/// 同样缺席时零变化（同一回归锁覆盖）
pub fn event_body(parsed: &ParsedHook, ts: i64) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("event".into(), json!(parsed.event_name));
    obj.insert("session_id".into(), json!(parsed.session_id));
    obj.insert("cwd".into(), json!(parsed.cwd));
    obj.insert("ts".into(), json!(ts));
    obj.insert("last_event_at".into(), json!(format_event_time(ts)));
    if !parsed.tool_name.is_empty() {
        obj.insert("tool_name".into(), json!(parsed.tool_name));
    }
    if let Some(ti) = &parsed.tool_input {
        obj.insert("tool_input".into(), json!(ti));
    }
    if let Some(msg) = &parsed.notification_message {
        obj.insert("message".into(), json!(msg));
    }
    serde_json::Value::Object(obj).to_string()
}

/// Notification 事件的「未决问答字段承接窗」（秒，T1）：与读取侧 30s TTL 同口径
/// ——承接来的信息永远不会活过读取侧本就会采信的时间窗，不放大失真。依据（实机
/// 取证）：claude 对 AskUserQuestion 待答的事件序是
/// `PreToolUse(AUQ)` → `PermissionRequest(AUQ)` → `Notification(permission_prompt)`
/// （本机实测间隔 ≈1s / ≈7s），事件文件同会话覆盖写——若 MAM 那一轮轮询落在
/// Notification 落盘之后，读取侧只会看到不带 tool_name 的 Notification，问答识别
/// 就丢了。承接把「未决的 AUQ 进入信号」带过 Notification 这一跳
pub const NOTIFICATION_CARRY_FORWARD_SECS: i64 = 30;

/// 承接上一条「未决问答进入信号」的可选字段（T1）：仅当本次事件是 Notification
/// 且同会话事件文件里躺着**新鲜的** PreToolUse/PermissionRequest ∧ AUQ 时，
/// 把 `tool_name`/`tool_input` 带到本次事件上——消费侧因此仍能按 tool_name 判出
/// 问答（判据见 adapter::apply_hook_event_to_session）。
///
/// **收窄到 AUQ 进入信号**（不是无条件承接）：答完的 `PostToolUse(AUQ)` 事件同样
/// 带 tool_name=AskUserQuestion，若对它也承接，则「答完 AUQ 后 30s 内的真实审批
/// Notification」会被误判成问答、红卡丢失——故 predecessor 只认
/// `PreToolUse`/`PermissionRequest` 两个进入事件。
///
/// 任何读取/解析失败 → 不承接（返回原 parsed 的克隆；helper 红线 1：错误路径静默）
fn with_carried_question_fields(
    events_dir: &std::path::Path,
    parsed: &ParsedHook,
    ts: i64,
) -> ParsedHook {
    let mut out = parsed.clone();
    if parsed.event_name != "Notification" || !parsed.tool_name.is_empty() {
        return out; // 只补 Notification 的空缺；自带 tool_name 的事件不覆盖
    }
    let path = events_dir.join(format!("{}.json", parsed.session_id));
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return out;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return out;
    };
    let prev_event = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
    if !matches!(prev_event, "PreToolUse" | "PermissionRequest") {
        return out;
    }
    if v.get("tool_name").and_then(|t| t.as_str()) != Some(ASK_USER_QUESTION_TOOL) {
        return out;
    }
    let prev_ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
    if ts.saturating_sub(prev_ts) > NOTIFICATION_CARRY_FORWARD_SECS {
        return out;
    }
    out.tool_name = ASK_USER_QUESTION_TOOL.to_string();
    if let Some(ti) = v.get("tool_input").and_then(|t| t.as_str()) {
        if ti.len() <= TOOL_INPUT_MAX_BYTES {
            out.tool_input = Some(ti.to_string());
        }
    }
    out
}

/// 写事件文件（瞬时 + 原子）：同目录临时文件 + rename 覆盖（Windows 上
/// std::fs::rename = MoveFileExW(MOVEFILE_REPLACE_EXISTING)，替换语义成立）——
/// 读取侧永不读到半截。临时名不带 .json 后缀 → 读取侧 strip_suffix(".json")
/// 天然跳过。失败返回 Err（bin 层吞掉，红线 1：错误路径零输出非零退出）。
/// T1 起写盘前先做「未决问答字段承接」（[`with_carried_question_fields`]）——
/// 目录里无同会话旧文件时行为与 T8 前完全一致（legacy 逐字节回归锁覆盖）
pub fn write_event_file(
    events_dir: &std::path::Path,
    parsed: &ParsedHook,
    ts: i64,
) -> std::io::Result<()> {
    std::fs::create_dir_all(events_dir)?;
    let effective = with_carried_question_fields(events_dir, parsed, ts);
    let body = event_body(&effective, ts);
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
        // 白名单外：路径注入形态 / 点号 / 空串 / 非 ASCII（下划线自 F8 起合法，
        // 已从本清单移出——见 session_id_allowed 的 kimi 形态说明）
        for bad_sid in [
            "../evil",
            "a/b",
            "..",
            "a.b",
            "",
            "会话一",
            "sid space",
            "a\\b",
            "sid\ttab",
        ] {
            let raw = format!(r#"{{"session_id":"{bad_sid}","hook_event_name":"Stop"}}"#);
            assert!(
                parse_hook_stdin(&raw).is_none(),
                "非法 session_id {bad_sid:?} 必须拒绝"
            );
        }
    }

    /// F8 实机取证回归锁：kimi 的 session_id 是 `session_<uuid>`（下划线前缀）——
    /// 旧白名单 `[A-Za-z0-9-]` 会静默丢弃 kimi 全部事件（实机实证：同 payload
    /// 换成裸 UUID 才落盘）。三家形态必须全部通过：claude/codex 裸 UUID、
    /// kimi 下划线前缀。
    #[test]
    fn session_id_whitelist_accepts_all_three_tool_shapes() {
        // claude / codex：裸 UUID（连字符）
        assert!(session_id_allowed("ec770a70-519f-43c9-81ca-9c74038ead8d"));
        // kimi：session_ 前缀 + UUID（下划线 + 连字符）——F8 修复点
        assert!(session_id_allowed(
            "session_ec770a70-519f-43c9-81ca-9c74038ead8d"
        ));
        assert!(session_id_allowed(
            "session_44554114-366e-4c61-9a57-733a3f3b79d0"
        ));
        // 下划线放行不引入穿越面：路径分隔符与点号仍拒绝
        for bad in [
            "session_../x",
            "session_/x",
            "session_\\x",
            "session_.",
            ".._",
        ] {
            assert!(!session_id_allowed(bad), "{bad:?} 必须拒绝（防注入）");
        }
        // 端到端一致性：kimi 形态经 parse_hook_stdin 可达（不是只过了谓词）
        let raw = r#"{"session_id":"session_ec770a70-519f-43c9-81ca-9c74038ead8d","hook_event_name":"PermissionRequest","cwd":"C:/x"}"#;
        let parsed = parse_hook_stdin(raw).expect("kimi 形态必须解析成功");
        assert_eq!(parsed.event_name, "PermissionRequest");
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
            tool_name: String::new(),
            tool_input: None,
            notification_message: None,
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
            tool_name: String::new(),
            tool_input: None,
            notification_message: None,
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
            tool_name: String::new(),
            tool_input: None,
            notification_message: None,
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

    // ---------- T8 问答通道（通道 A 写侧）：PreToolUse ∧ AskUserQuestion ----------

    /// T8①：claude 投递给 helper 的真实形态（探测档案 §1/§2 取证——AskUserQuestion
    /// 触发时 claude 向 helper stdin 投递 PreToolUse payload，tool_input.questions
    /// 为原样选项表；夹具取探测档案 §3 单选真实 JSON 缩录）→ ParsedHook 携带
    /// tool_name + tool_input 原文（questions 原样保留）
    #[test]
    fn parse_pretooluse_askuserquestion_carries_tool_input() {
        // 探测档案 §3 单选夹具（tool_input.questions 原样缩录；cwd 取档案 §1 实测形态）
        let raw = concat!(
            r#"{"session_id":"1b0ba2d5-eecd-48b6-b1e2-c1a784e0db1f","#,
            r#""transcript_path":"/home/u/.claude/projects/x/1b0ba2d5.jsonl","#,
            r#""cwd":"C:\\Users\\bunny\\AppData\\Local\\Temp\\mam-probe-askq-proj","#,
            r#""hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","#,
            r#""permission_mode":"default","tool_input":{"questions":["#,
            r#"{"header":"Next step","multiSelect":false,"options":["#,
            r#"{"description":"Explain how AskUserQuestion works and when it's used.","label":"Tool demo"},"#,
            r#"{"description":"Start a coding or file task in this directory.","label":"Start a task"},"#,
            r#"{"description":"You have no further request for now.","label":"Nothing yet"}],"#,
            r#""question":"This is a demo question — what would you like to do next?"}]}}"#
        );
        let parsed = parse_hook_stdin(raw).expect("claude AUQ payload 必须解析成功");
        assert_eq!(parsed.event_name, "PreToolUse");
        assert_eq!(parsed.tool_name, "AskUserQuestion");
        let ti = parsed
            .tool_input
            .as_deref()
            .expect("AUQ 必须携带 tool_input");
        let v: serde_json::Value = serde_json::from_str(ti).expect("tool_input 是原文 JSON");
        let questions = v["questions"].as_array().expect("questions 数组在场");
        assert_eq!(questions.len(), 1, "探测档案单选夹具 = 单问题");
        assert_eq!(questions[0]["header"], "Next step");
        assert_eq!(questions[0]["multiSelect"], false);
        assert_eq!(questions[0]["options"].as_array().unwrap().len(), 3);
        assert_eq!(questions[0]["options"][1]["label"], "Start a task");
        assert_eq!(
            questions[0]["options"][0]["description"],
            "Explain how AskUserQuestion works and when it's used."
        );
    }

    /// T8① 反例：非 AskUserQuestion 的 PreToolUse（普通 Bash 调用等）→ tool_name
    /// 照收但 **不带 tool_input**（问答通道不承载普通工具调用）
    #[test]
    fn pretooluse_other_tool_has_no_tool_input() {
        let raw = r#"{"session_id":"sid-auq-2","hook_event_name":"PreToolUse","cwd":"/w","tool_name":"Bash","tool_input":{"command":"ls -la"}}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        assert_eq!(parsed.tool_name, "Bash");
        assert!(
            parsed.tool_input.is_none(),
            "非 AUQ 的 PreToolUse 不得携带 tool_input"
        );
    }

    /// T8① 反例：AskUserQuestion 但事件不是 PreToolUse（理论形态，防御性收窄）→
    /// 不带 tool_input
    #[test]
    fn askuserquestion_on_other_event_has_no_tool_input() {
        let raw = r#"{"session_id":"sid-auq-3","hook_event_name":"PostToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[]}}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        assert!(parsed.tool_input.is_none(), "非 PreToolUse 恒不携带");
    }

    /// T8①：tool_input 序列化超 64KB → **整字段丢弃**（不截半截 JSON——孤儿字段
    /// 不可解析；截断策略注释见 TOOL_INPUT_MAX_BYTES）
    #[test]
    fn oversized_tool_input_field_is_dropped() {
        // 构造 >64KB 的 description（单字段即可把整体序列化推过上限）
        let big = "x".repeat(TOOL_INPUT_MAX_BYTES + 1);
        let raw = format!(
            r#"{{"session_id":"sid-auq-4","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{{"questions":[{{"header":"h","question":"q","multiSelect":false,"options":[{{"label":"L","description":"{big}"}}]}}]}}}}"#
        );
        assert!(raw.len() > TOOL_INPUT_MAX_BYTES);
        let parsed = parse_hook_stdin(&raw).expect("payload 本身合法，必须解析成功");
        assert!(
            parsed.tool_input.is_none(),
            "超限 tool_input 必须整字段丢弃"
        );
        // 边界内（恰好 ≤ 上限）照常携带
        let fit = "x".repeat(TOOL_INPUT_MAX_BYTES - 256);
        let raw_fit = format!(
            r#"{{"session_id":"sid-auq-4","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{{"questions":[{{"header":"h","question":"q","options":[{{"label":"L","description":"{fit}"}}]}}]}}}}"#
        );
        let parsed_fit = parse_hook_stdin(&raw_fit).unwrap();
        assert!(parsed_fit.tool_input.is_some(), "上限内照常携带");
    }

    /// T8 回归锁：无 tool_name 的普通事件（Stop 等）事件正文与 T8 前**逐字节一致**
    /// （可选字段缺席不落键；读取侧 serde default 兼容 bash 兜底形态）
    #[test]
    fn event_body_without_optional_fields_is_legacy_shape() {
        let parsed = ParsedHook {
            session_id: "sid-x".into(),
            event_name: "Stop".into(),
            cwd: "/tmp".into(),
            tool_name: String::new(),
            tool_input: None,
            notification_message: None,
        };
        let body = event_body(&parsed, 1_789_171_200);
        assert_eq!(
            body,
            r#"{"event":"Stop","session_id":"sid-x","cwd":"/tmp","ts":1789171200,"last_event_at":"2026-09-12T00:00:00Z"}"#,
            "无 tool_name 事件正文必须与 bash 版逐字段同构（T8 零变化）"
        );
    }

    /// T8：问答事件正文携带 tool_name + tool_input 两可选字段；非问答但带 tool_name
    /// 的事件（codex PermissionRequest 形态）只带 tool_name
    #[test]
    fn event_body_includes_optional_fields_conditionally() {
        let auq = ParsedHook {
            session_id: "sid-q".into(),
            event_name: "PreToolUse".into(),
            cwd: String::new(),
            tool_name: ASK_USER_QUESTION_TOOL.into(),
            tool_input: Some(r#"{"questions":[]}"#.into()),
            notification_message: None,
        };
        let v: serde_json::Value = serde_json::from_str(&event_body(&auq, 1)).unwrap();
        assert_eq!(v["tool_name"], "AskUserQuestion");
        assert_eq!(v["tool_input"], r#"{"questions":[]}"#);
        // tool_input 是**原文 JSON 串**（string 字段），非嵌套对象——读取侧按
        // Option<String> 透传，解析归端点/状态链消费侧
        assert!(v["tool_input"].is_string());

        let codex_like = ParsedHook {
            session_id: "sid-c".into(),
            event_name: "PermissionRequest".into(),
            cwd: String::new(),
            tool_name: "shell".into(),
            tool_input: None,
            notification_message: None,
        };
        let v: serde_json::Value = serde_json::from_str(&event_body(&codex_like, 1)).unwrap();
        assert_eq!(v["tool_name"], "shell");
        assert!(v.get("tool_input").is_none());
    }

    /// T8 端到端（bin run 缝同款）：AUQ payload → 事件文件含 tool_name+tool_input
    /// （questions 原样）；覆盖写语义不受新字段影响
    #[test]
    fn write_event_file_roundtrips_question_channel() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = r#"{"session_id":"sid-qe2e","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[{"header":"Favorite fruits","multiSelect":true,"options":[{"description":"A sweet, crisp fruit available in many varieties.","label":"Apple"},{"description":"A soft, tropical fruit rich in potassium.","label":"Banana"}],"question":"Which fruits are your favorites? (Select all that apply)"}]}}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        write_event_file(tmp.path(), &parsed, 1_789_171_200).unwrap();
        let body = std::fs::read_to_string(tmp.path().join("sid-qe2e.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["event"], "PreToolUse");
        assert_eq!(v["tool_name"], "AskUserQuestion");
        let ti: serde_json::Value =
            serde_json::from_str(v["tool_input"].as_str().unwrap()).unwrap();
        assert_eq!(
            ti["questions"][0]["options"][0]["label"], "Apple",
            "questions 原样落盘（探测档案多选夹具）"
        );
        assert_eq!(ti["questions"][0]["multiSelect"], true);
    }

    // ---------- 批次丙 T1 通知语义通道：Notification ∧ message ----------

    /// T1①：claude 真实 Notification payload（含 message/title/notification_type 三
    /// 专属字段）→ ParsedHook 携带 notification_message；message 判据锚点的形态由
    /// 实机取证档案钉死（research/refs/phase2-消息注入/
    /// 2026-09-21-claude-notification-message-取证.md）
    #[test]
    fn parse_notification_carries_message() {
        let raw = concat!(
            r#"{"session_id":"0c41365d-1111-2222-3333-444455556666","#,
            r#""transcript_path":"/home/u/.claude/projects/x/0c41365d.jsonl","#,
            r#""cwd":"E:\\proj\\demo","hook_event_name":"Notification","#,
            r#""message":"Claude needs your permission","#,
            r#""title":"Claude Code","notification_type":"permission_prompt"}"#
        );
        let parsed = parse_hook_stdin(raw).expect("claude Notification payload 必须解析成功");
        assert_eq!(parsed.event_name, "Notification");
        assert_eq!(
            parsed.notification_message.as_deref(),
            Some("Claude needs your permission"),
            "Notification 的 message 必须原样透传（实机取证原文逐字）"
        );
    }

    /// T1① 反例：Notification 无 message（旧 claude 版本 / 其他工具形态）→
    /// 字段 None，正文回到 legacy 形态（零变化）
    #[test]
    fn parse_notification_without_message_yields_none() {
        let raw = r#"{"session_id":"sid-n1","hook_event_name":"Notification","cwd":"/w","notification_type":"permission_prompt"}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        assert!(parsed.notification_message.is_none());
        // message 类型非法（对象/数字）按缺失处置
        let raw2 =
            r#"{"session_id":"sid-n2","hook_event_name":"Notification","message":{"text":"x"}}"#;
        assert!(parse_hook_stdin(raw2)
            .unwrap()
            .notification_message
            .is_none());
        let raw3 = r#"{"session_id":"sid-n3","hook_event_name":"Notification","message":42}"#;
        assert!(parse_hook_stdin(raw3)
            .unwrap()
            .notification_message
            .is_none());
    }

    /// T1① 反例：非 Notification 事件带 message（语义未被取证）→ **不附加**
    ///（最小兼容面：只在我们有证据的事件上落新字段）
    #[test]
    fn message_field_is_not_attached_on_other_events() {
        let raw =
            r#"{"session_id":"sid-n4","hook_event_name":"Stop","cwd":"/w","message":"whatever"}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        assert!(
            parsed.notification_message.is_none(),
            "非 Notification 事件的 message 不得照收"
        );
    }

    /// T1：message 超 4KB → **前缀截断保留**（与 tool_input 的整字段丢弃有意不同：
    /// 判据锚点在前部，整字段丢弃会让判据失据 → 幽灵审批标记回归）；截断按字符
    /// 边界（多字节 UTF-8 不得切半）
    #[test]
    fn oversized_notification_message_is_prefix_truncated() {
        // ASCII：恰好截到上限
        let big = "x".repeat(NOTIFICATION_MESSAGE_MAX_BYTES + 100);
        let raw = format!(
            r#"{{"session_id":"sid-n5","hook_event_name":"Notification","message":"{big}"}}"#
        );
        let parsed = parse_hook_stdin(&raw).unwrap();
        let msg = parsed
            .notification_message
            .expect("超限 message 仍须保留前缀");
        assert_eq!(msg.len(), NOTIFICATION_MESSAGE_MAX_BYTES);
        assert!(msg.starts_with("xxx"));

        // 多字节：上限处落在字符中间 → 回退到字符边界（结果 ≤ 上限且可解码）
        let cjk = "中".repeat(NOTIFICATION_MESSAGE_MAX_BYTES); // 3 字节/字
        let raw = format!(
            r#"{{"session_id":"sid-n6","hook_event_name":"Notification","message":"{cjk}"}}"#
        );
        let parsed = parse_hook_stdin(&raw).unwrap();
        let msg = parsed.notification_message.unwrap();
        assert!(msg.len() <= NOTIFICATION_MESSAGE_MAX_BYTES);
        assert_eq!(msg.len() % 3, 0, "截断必须落在字符边界（UTF-8 不切半）");
        assert!(msg.chars().all(|c| c == '中'));

        // 上限内原样保留
        let ok = "Claude needs your permission to use Bash";
        let raw = format!(
            r#"{{"session_id":"sid-n7","hook_event_name":"Notification","message":"{ok}"}}"#
        );
        assert_eq!(
            parse_hook_stdin(&raw)
                .unwrap()
                .notification_message
                .as_deref(),
            Some(ok)
        );
    }

    /// T1 回归锁（与 T8 同一条，扩到第三个可选字段）：无 tool_name / 无 message 的
    /// 普通事件正文与 T8 前**逐字节一致**。含「payload 带 message 键但事件不是
    /// Notification」的形态（message 只对 Notification 附加）
    #[test]
    fn event_body_with_message_absent_is_byte_identical() {
        let parsed = parse_hook_stdin(
            r#"{"session_id":"sid-x","hook_event_name":"Stop","cwd":"/tmp","message":"ignored"}"#,
        )
        .unwrap();
        assert_eq!(
            event_body(&parsed, 1_789_171_200),
            r#"{"event":"Stop","session_id":"sid-x","cwd":"/tmp","ts":1789171200,"last_event_at":"2026-09-12T00:00:00Z"}"#,
            "非 Notification 事件正文必须与 legacy 逐字节一致（T1 零变化）"
        );
    }

    /// T1：Notification 正文追加 `message` 键（键序在既有可选字段之后；其余键位
    /// 不变）；payload 带 message 键时**含空串也落键**（「观测到但为空」有诊断价值，
    /// 读取侧空串与缺席同义——见 ParsedHook::notification_message 注释）
    #[test]
    fn event_body_includes_message_for_notification_only() {
        let raw = r#"{"session_id":"sid-m1","hook_event_name":"Notification","cwd":"/w","message":"Claude needs your permission to use Bash"}"#;
        let parsed = parse_hook_stdin(raw).unwrap();
        let body = event_body(&parsed, 1_789_171_200);
        assert_eq!(
            body,
            r#"{"event":"Notification","session_id":"sid-m1","cwd":"/w","ts":1789171200,"last_event_at":"2026-09-12T00:00:00Z","message":"Claude needs your permission to use Bash"}"#
        );
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(v["message"].is_string(), "message 是 string 字段");

        // 空串 message：仍落键（正文相对「无 message 键」多一个键，故不属于
        // legacy 形态——legacy 锁只覆盖「无 message 键」与非 Notification 两类）
        let empty = parse_hook_stdin(
            r#"{"session_id":"sid-m2","hook_event_name":"Notification","cwd":"/w","message":""}"#,
        )
        .unwrap();
        assert_eq!(empty.notification_message.as_deref(), Some(""));
        assert!(
            event_body(&empty, 1).contains("\"message\":\"\""),
            "空串照落键（与注释/读取侧语义一致）"
        );
        // 对照：完全无 message 键的 Notification → 正文不含 message 键（legacy 形态）
        let absent = parse_hook_stdin(
            r#"{"session_id":"sid-m3","hook_event_name":"Notification","cwd":"/w"}"#,
        )
        .unwrap();
        assert_eq!(absent.notification_message, None);
        assert!(!event_body(&absent, 1).contains("\"message\""));
    }

    /// T1 端到端（bin run 缝同款）：Notification payload → 事件文件含 message 原文
    #[test]
    fn write_event_file_roundtrips_notification_message() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = concat!(
            r#"{"session_id":"sid-ne2e","hook_event_name":"Notification","cwd":"/w","#,
            r#""title":"Claude Code","notification_type":"permission_prompt","#,
            r#""message":"Claude needs your permission"}"#
        );
        let parsed = parse_hook_stdin(raw).unwrap();
        write_event_file(tmp.path(), &parsed, 1_789_171_200).unwrap();
        let body = std::fs::read_to_string(tmp.path().join("sid-ne2e.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["event"], "Notification");
        assert_eq!(v["message"], "Claude needs your permission");
    }

    // ---------- 批次丙 T1 未决问答字段承接（Notification 那一跳） ----------

    /// T1 承接①：AUQ 的 PermissionRequest 落盘 → 随后不带 tool_name 的
    /// Notification → 事件文件**承接** tool_name + tool_input（实机事件序：
    /// PreToolUse(AUQ) → PermissionRequest(AUQ) → Notification(permission_prompt)，
    /// 读取侧只见到最后那条 → 不承接就丢问答识别）
    #[test]
    fn notification_carries_forward_pending_question_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = r#"{"questions":[{"question":"Which color do you prefer?","header":"Color","options":[{"label":"Blue","description":"d"}],"multiSelect":false}]}"#;
        // ① AUQ 进入事件先落盘（PermissionRequest 形态，实机取证）
        let auq = format!(
            r#"{{"session_id":"sid-cf1","hook_event_name":"PermissionRequest","tool_name":"AskUserQuestion","tool_input":{payload}}}"#
        );
        write_event_file(tmp.path(), &parse_hook_stdin(&auq).unwrap(), 1_000).unwrap();
        // ② 同会话 Notification 到达（不带 tool_name）→ 承接
        let notif = parse_hook_stdin(
            r#"{"session_id":"sid-cf1","hook_event_name":"Notification","message":"Claude needs your permission","notification_type":"permission_prompt"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &notif, 1_001).unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("sid-cf1.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(v["event"], "Notification", "事件名不得被承接篡改");
        assert_eq!(v["tool_name"], ASK_USER_QUESTION_TOOL, "承接 AUQ 工具名");
        assert_eq!(v["tool_input"], payload, "承接原始 questions 载荷");
        assert_eq!(
            v["message"], "Claude needs your permission",
            "本事件自身的 message 照常落盘"
        );
    }

    /// T1 承接②收窄：答完信号 `PostToolUse(AUQ)` **不得**触发承接——否则
    /// 「答完 AUQ 后 30s 内的真实审批 Notification」会被误判成问答、红卡丢失
    #[test]
    fn carry_forward_ignores_answer_signal_and_other_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let done = parse_hook_stdin(
            r#"{"session_id":"sid-cf2","hook_event_name":"PostToolUse","tool_name":"AskUserQuestion"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &done, 1_000).unwrap();
        let notif = parse_hook_stdin(
            r#"{"session_id":"sid-cf2","hook_event_name":"Notification","message":"Claude needs your permission"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &notif, 1_001).unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("sid-cf2.json")).unwrap(),
        )
        .unwrap();
        assert!(
            v.get("tool_name").is_none(),
            "答完信号不得触发承接（真实审批 Notification 的红卡保住）: {v}"
        );
        // 非 AUQ 的进入事件同样不承接（真实审批 PermissionRequest(Write)）
        let write_ev = parse_hook_stdin(
            r#"{"session_id":"sid-cf3","hook_event_name":"PermissionRequest","tool_name":"Write","tool_input":{"file_path":"C:\\x.txt"}}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &write_ev, 1_000).unwrap();
        let notif3 = parse_hook_stdin(
            r#"{"session_id":"sid-cf3","hook_event_name":"Notification","message":"Claude needs your permission"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &notif3, 1_001).unwrap();
        let v3: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("sid-cf3.json")).unwrap(),
        )
        .unwrap();
        assert!(
            v3.get("tool_name").is_none(),
            "非 AUQ 进入事件不得触发承接（真实审批的红卡保住）: {v3}"
        );
    }

    /// T1 承接③窗口：超出 30s 承接窗（与读取侧 TTL 同口径）→ 不承接；时钟倒退
    /// （now < prev，saturating_sub=0）→ 视为新鲜、照常承接（保守侧=保问答卡）
    #[test]
    fn carry_forward_respects_freshness_window() {
        let tmp = tempfile::tempdir().unwrap();
        let auq = parse_hook_stdin(
            r#"{"session_id":"sid-cf4","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[]}}"#,
        )
        .unwrap();
        let notif = |ts: i64| {
            write_event_file(
                tmp.path(),
                &parse_hook_stdin(
                    r#"{"session_id":"sid-cf4","hook_event_name":"Notification","message":"m"}"#,
                )
                .unwrap(),
                ts,
            )
            .unwrap();
            let v: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(tmp.path().join("sid-cf4.json")).unwrap(),
            )
            .unwrap();
            v
        };
        // 窗口内（=30s）承接
        write_event_file(tmp.path(), &auq, 1_000).unwrap();
        assert_eq!(notif(1_030)["tool_name"], ASK_USER_QUESTION_TOOL);
        // 超窗（31s）不承接
        write_event_file(tmp.path(), &auq, 1_000).unwrap();
        assert!(notif(1_031).get("tool_name").is_none(), "超窗不承接");
        // 时钟倒退不视为过期（承接=保问答卡，宁可多承接一轮 TTL）
        write_event_file(tmp.path(), &auq, 2_000).unwrap();
        assert_eq!(notif(1_000)["tool_name"], ASK_USER_QUESTION_TOOL);
    }

    /// T1 承接④：目录里无同会话旧文件（首事件 / 跨会话）→ 行为与 T8 前完全一致
    /// （legacy 形态零变化：这是向后兼容的硬约束回归锁）
    #[test]
    fn carry_forward_is_noop_without_predecessor() {
        let tmp = tempfile::tempdir().unwrap();
        let notif = parse_hook_stdin(
            r#"{"session_id":"sid-cf5","hook_event_name":"Notification","message":"Claude needs your permission"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &notif, 1_000).unwrap();
        let body = std::fs::read_to_string(tmp.path().join("sid-cf5.json")).unwrap();
        assert_eq!(
            body,
            r#"{"event":"Notification","session_id":"sid-cf5","cwd":"","ts":1000,"last_event_at":"1970-01-01T00:16:40Z","message":"Claude needs your permission"}"#,
            "无前驱时正文与 T8 版逐字节一致（承接零副作用）"
        );
        // 损坏的旧文件 → 不承接、不 panic（红线 1：错误路径静默）
        std::fs::write(tmp.path().join("sid-cf6.json"), b"{not json").unwrap();
        let n2 = parse_hook_stdin(
            r#"{"session_id":"sid-cf6","hook_event_name":"Notification","message":"m"}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &n2, 2_000).unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("sid-cf6.json")).unwrap(),
        )
        .unwrap();
        assert!(v.get("tool_name").is_none(), "损坏前驱不承接且不炸");
    }

    /// T1 承接⑤：清除族事件（Stop/UserPromptSubmit 等）覆盖事件文件后，承接链
    /// 自然打断——后续真实审批 Notification 读到的前驱已不是 AUQ 进入事件
    #[test]
    fn carry_forward_stops_after_clear_family_event() {
        let tmp = tempfile::tempdir().unwrap();
        let auq = parse_hook_stdin(
            r#"{"session_id":"sid-cf7","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[]}}"#,
        )
        .unwrap();
        write_event_file(tmp.path(), &auq, 1_000).unwrap();
        // 答完 → Stop（清除族）覆盖事件文件
        write_event_file(
            tmp.path(),
            &parse_hook_stdin(r#"{"session_id":"sid-cf7","hook_event_name":"Stop"}"#).unwrap(),
            1_001,
        )
        .unwrap();
        // 随后的真实审批 Notification：前驱是 Stop → 不承接（红卡保住）
        write_event_file(
            tmp.path(),
            &parse_hook_stdin(
                r#"{"session_id":"sid-cf7","hook_event_name":"Notification","message":"Claude needs your permission"}"#,
            )
            .unwrap(),
            1_002,
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(tmp.path().join("sid-cf7.json")).unwrap(),
        )
        .unwrap();
        assert!(v.get("tool_name").is_none(), "清除族打断承接链: {v}");
    }
}
