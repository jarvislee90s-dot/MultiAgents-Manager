// 会话内容读取层（M3 Task 7，C2 后端）：八工具统一出口（P9）
//
// 每个工具从其原生存储读取消息流尾部，映射为统一的 `SessionMessage`（camelCase，
// 对齐移动端契约）。映射关系逐工具对齐既有 parser（monitor/*_parser.rs）的实测格式：
// - zcode    → ~/.zcode/cli/db/db.sqlite（message/part 表，见 zcode_parser.rs 数据事实）
// - dsh      → ~/.dsh/sessions/<project>/<dir>/session.vN.jsonl.zstd（代际日志，dsh/log.rs）
// - claude   → ~/.claude/projects/*/<session-uuid>.jsonl（role + content[] 块协议）
// - codex    → ~/.codex/sessions/**/rollout-*.jsonl（type+payload 协议）优先，
//              未命中回退 thread_history_*.sqlite 的 thread_items（APP 线路，codex_thread_parser）
// - kimi     → session_index.jsonl 定位 + agents/main/wire.jsonl 事件流
// - workbuddy → ~/.workbuddy/projects/*/<sessionId>.jsonl（OpenAI 风格 type/role/content）
// - opencode → ~/.local/share/opencode/opencode.db（message/part 表）
// - openclaw → ~/.openclaw/state/openclaw.sqlite 的 acp_replay_events（ACP 协议 replay 日志，
//              2026-09-15 实机探测确认）；无该存储 → Err 优雅降级（Task 7 裁决，不阻塞其余七工具）
//
// 预算说明：本层是**按需单会话读取**（移动端点开详情才调用），不是 3 秒轮询路径，
// 不接 monitor::session_scan 的 L2/L3 缓存；但有界防御——文件读取一律走尾部窗口
// （read_recent_lines，512KB 上限），SQLite 查询一律 LIMIT + 参数化（session_id 走 ?1 占位，
// LIMIT 为 usize 格式化无注入面）。
//
// 测试约束（宪法级）：一律 tempdir / tmp sqlite / 黄金夹具，绝不触真实 ~/.zcode ~/.dsh 等；
// 生产薄壳只做 dirs::home_dir() → `*_with(home)` 注入核的转发（zcode_home_with 先例）。

use serde::Serialize;
use std::path::Path;

/// 统一消息条目（camelCase 序列化 = 移动端契约，勿改字段名）。
/// `seq` 是返回数组内的稳定顺序号（0 起文件序递增，前端按 seq 排序稳定）；
/// `collapsed`：thinking 与 tool-call 默认折叠。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub seq: i64,
    /// user / assistant（thinking、tool-call、tool-result 属 agent 侧工作产物，归 assistant）
    pub role: String,
    /// user / assistant / thinking / tool-call / tool-result
    pub kind: String,
    /// 文本内容或工具摘要
    pub content: String,
    /// epoch 毫秒（原生存储无时间戳的条目为 None）
    pub ts: Option<i64>,
    /// kind = tool-call 时的工具名
    pub tool_name: Option<String>,
    /// kind = tool-call 时的参数 JSON 字符串（原生存储无参数则 None）
    pub tool_args: Option<String>,
    pub collapsed: bool,
}

impl SessionMessage {
    /// 文本类条目（user / assistant / thinking / tool-result）。
    /// collapsed 由 kind 推导：thinking 恒折叠
    fn text(kind: &str, content: impl Into<String>, ts: Option<i64>) -> Self {
        let kind = kind.to_string();
        let role = if kind == "user" { "user" } else { "assistant" };
        SessionMessage {
            seq: 0,
            role: role.to_string(),
            collapsed: kind == "thinking",
            kind,
            content: content.into(),
            ts,
            tool_name: None,
            tool_args: None,
        }
    }

    /// 工具调用条目（collapsed = true）
    fn tool_call(
        content: impl Into<String>,
        ts: Option<i64>,
        name: Option<String>,
        args: Option<String>,
    ) -> Self {
        SessionMessage {
            seq: 0,
            role: "assistant".to_string(),
            kind: "tool-call".to_string(),
            content: content.into(),
            ts,
            tool_name: name,
            tool_args: args,
            collapsed: true,
        }
    }
}

/// 会话内容读取结果：消息页 + 头部截断标记（Bug 1，M3 验收）。
/// truncated = 文件头部被字节窗截断（存在更早但未在本页的内容）——移动端据此
/// 显示「加载更早消息」；SQLite 系（zcode/opencode/openclaw/codex thread）与 dsh
/// （自有 zstd 代际读取）恒 false
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesPage {
    pub messages: Vec<SessionMessage>,
    pub truncated: bool,
}

/// 会话内容源函数形态（RemoteState 注入缝的类型别名，生产 = read_session_messages）
pub type MessageSourceFn = dyn Fn(&str, &str, usize) -> Result<MessagesPage, String> + Send + Sync;

/// env 双参读取（DSH_HOME / KIMI_CODE_HOME）——**生产薄壳专用单一归口**（终审
/// Important 1）：/session-messages 与 /session-files 两条生产薄壳都从这里取值，
/// 禁止在别处复制 std::env::var 逻辑。值一律以参数传入注入核（impl），测试路径
/// 零真实 env 接触（与并行 env 测试互斥锁无交集）
pub(crate) fn read_env_homes() -> (Option<String>, Option<String>) {
    (
        std::env::var("DSH_HOME").ok(),
        std::env::var("KIMI_CODE_HOME").ok(),
    )
}

/// 统一出口（生产薄壳）：真实 home + 环境重定向（DSH_HOME / KIMI_CODE_HOME，
/// 与各工具看板扫描同源——dsh 走 scan_sessions(&dsh_home()) 的 M0 F14 优先级，
/// kimi 走 resolve_data_root 的 env 优先级）→ 注入核。
/// agent_type 与 `AgentType::tool_id()` 的小写形态一致（zcode/dsh/claude/codex/kimi/
/// workbuddy/opencode/openclaw）。
pub fn read_session_messages(
    agent_type: &str,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法确定用户主目录".to_string())?;
    // env 只在生产薄壳读取（fix round 1 Important 1；读取归口 read_env_homes，
    // 终审 Important 1 起 files.rs 生产薄壳同源复用）
    let (dsh_env, kimi_env) = read_env_homes();
    read_session_messages_impl(
        &home,
        dsh_env.as_deref(),
        kimi_env.as_deref(),
        agent_type,
        session_id,
        limit,
    )
}

/// 注入核（测试直调 tempdir home，零真实数据目录接触；env 重定向恒 None）
pub fn read_session_messages_with(
    home: &Path,
    agent_type: &str,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    read_session_messages_impl(home, None, None, agent_type, session_id, limit)
}

/// 派发核（env 双参注入，dsh/kimi 消费；kimi 的 resolve_data_root 双参模式同款）。
/// `pub(crate)`（终审 Important 1）：files.rs 的 env 注入核（extract_file_paths_with_env）
/// 同源复用——文件面板提取与 /session-messages 走同一派发路径
pub(crate) fn read_session_messages_impl(
    home: &Path,
    dsh_env_home: Option<&str>,
    kimi_env_home: Option<&str>,
    agent_type: &str,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    // 防御：空 session_id 直接拒绝（各工具文件名/SQL 均以 id 拼 key，空串会形成
    // ".jsonl" 这类危险形态）；路径分隔符与 ".." 拒绝——claude/workbuddy 的文件
    // 定位是 `目录.join(format!("{sid}.jsonl"))`，穿越字符会逃出项目根（端点在
    // 设备门禁之后，但内容层自身 fail-closed）；limit 夹取 [1, 1000]
    if session_id.trim().is_empty() {
        return Err("session_id 为空".to_string());
    }
    if session_id.contains(['/', '\\', '\0']) || session_id.contains("..") {
        return Err(format!("session_id 含非法字符: {session_id}"));
    }
    let limit = limit.clamp(1, 1000);
    match agent_type {
        "zcode" => read_zcode_messages_with(home, session_id, limit),
        "dsh" => read_dsh_messages_with(home, dsh_env_home, session_id, limit),
        "claude" => read_claude_messages_with(home, session_id, limit),
        "codex" => read_codex_messages_with(home, session_id, limit),
        "kimi" => read_kimi_messages_with(home, kimi_env_home, session_id, limit),
        "workbuddy" => read_workbuddy_messages_with(home, session_id, limit),
        "opencode" => read_opencode_messages_with(home, session_id, limit),
        "openclaw" => read_openclaw_messages_with(home, session_id, limit),
        _ => Err(format!("未知工具: {agent_type}")),
    }
}

// ============================================================
// 公共小件
// ============================================================

/// 尾部截取 + seq 重排：取**文件序尾部**至多 limit 条（更早的内容由前端以更大
/// limit 重拉——Task 7 裁决：M3 不做 before 游标），seq 用返回数组内全局递增号
fn finalize(mut msgs: Vec<SessionMessage>, limit: usize) -> Vec<SessionMessage> {
    if msgs.len() > limit {
        msgs = msgs.split_off(msgs.len() - limit);
    }
    for (i, m) in msgs.iter_mut().enumerate() {
        m.seq = i as i64;
    }
    msgs
}

/// 组页小件：finalize + 截断标记打包（各工具读取器统一出口）
fn page(msgs: Vec<SessionMessage>, limit: usize, truncated: bool) -> MessagesPage {
    MessagesPage {
        messages: finalize(msgs, limit),
        truncated,
    }
}

/// JSONL 行读取预算：映射后条数上限 limit 对应的行窗口（assistant 消息会展开为
/// 多条目，按 4 倍预留；下限 500 对齐各 parser 的 RECENT_LINES 口径）
fn line_budget(limit: usize) -> usize {
    (limit.saturating_mul(4)).clamp(500, 4096)
}

/// JSONL 字节窗预算（Bug 1，M3 验收）：512KB 基准按 limit 放大（每 200 条一档），
/// 封顶 4MB——「加载更早消息」以更大 limit 重拉时字节窗必须同放大，否则按钮
/// 拉不到更早内容（死功能根因之一）。limit 在派发核入口已夹 [1,1000]，此处
/// saturating 防御即可；封顶属纵深防御（limit=1000 时 2.5MB 未触顶）
fn byte_budget(limit: usize) -> u64 {
    const BASE: u64 = 512 * 1024;
    const CAP: u64 = 4 * 1024 * 1024;
    BASE.saturating_mul(limit.div_ceil(200) as u64).min(CAP)
}

/// ISO 8601 时间戳 → epoch 毫秒（claude/codex 行内 timestamp 形态）；解析失败 None
fn iso_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// 数组内 text 块拼接（content[]: [{type:"text",text:...}, ...] → 单串，空格连接）。
/// 元素不是对象或无 text 字段时跳过该元素
fn join_text_parts(v: &serde_json::Value) -> String {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// content[] 里第一个非空字符串字段值（Claude tool_result 的 block.content 双形态：
/// 字符串直取；数组取 text 拼接）
fn content_block_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(_) => {
            let t = join_text_parts(v);
            (!t.is_empty()).then_some(t)
        }
        _ => None,
    }
}

/// TEXT/BLOB 双形态防御读取（ZCode/OpenCode 私有库类型不保证；zcode_parser 同款）
fn sqlite_text(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<String> {
    use rusqlite::types::ValueRef;
    match row.get_ref(idx)? {
        ValueRef::Text(t) => Ok(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Ok(String::from_utf8_lossy(b).into_owned()),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(rusqlite::Error::InvalidColumnType(
                idx,
                "text/blob".to_string(),
                other.data_type(),
            )),
        )),
    }
}

/// 工具摘要文案：有名给「调用 <名>」，无名兜底「工具调用」
fn tool_summary(name: Option<&str>) -> String {
    match name {
        Some(n) => format!("调用 {n}"),
        None => "工具调用".to_string(),
    }
}

// ============================================================
// ZCode：~/.zcode/cli/db/db.sqlite（message + part 表）
// ============================================================

/// 在 <tool_dir>/projects/*/ 下按 session_id 定位 .jsonl 文件（claude / workbuddy 共用）
fn find_jsonl_in_projects(
    home: &std::path::Path,
    tool_dir: &str,
    session_id: &str,
) -> Result<std::path::PathBuf, String> {
    let projects = home.join(tool_dir).join("projects");
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Err(format!("{tool_dir} projects 目录不存在"));
    };
    for dir in entries.flatten() {
        let candidate = dir.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("会话文件不存在: {session_id}"))
}

fn read_zcode_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    use crate::monitor::zcode_parser::ZcodeRoots;
    let roots = ZcodeRoots::from_home(home);
    let conn = crate::monitor::sqlite::open_readonly_with_timeout(&roots.cli_db)
        .ok_or_else(|| "ZCode cli db 不可读".to_string())?;

    // 顺序列探测（load_tail_messages 同款）：sequence 存在按流顺序（懒落库时间戳
    // 不可靠、顺序可靠），否则降级 time_created；两列皆无（升级改表）→ Err
    let order_col = if conn.prepare("SELECT sequence FROM message LIMIT 0").is_ok() {
        "sequence"
    } else if conn
        .prepare("SELECT time_created FROM message LIMIT 0")
        .is_ok()
    {
        "time_created"
    } else {
        return Err("ZCode message 表缺顺序列".to_string());
    };
    // LIMIT 为 usize 格式化（无注入面），session_id 走 ?1 参数化
    let sql = format!(
        "SELECT id, data, time_created FROM message WHERE session_id = ?1 \
         ORDER BY {order_col} DESC LIMIT {limit}"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("ZCode 查询失败: {e}"))?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((
                sqlite_text(row, 0)?,
                sqlite_text(row, 1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(|e| format!("ZCode 查询失败: {e}"))?;
    // 查询取「最新在前」，反转为文件序（旧 → 新）
    let mut messages: Vec<(String, String, Option<i64>)> = rows.filter_map(|r| r.ok()).collect();
    messages.reverse();
    // 会话不存在（0 行）→ Err（404 语义），与「存在但无消息条目」区分
    if messages.is_empty() {
        return Err(format!("ZCode 会话不存在: {session_id}"));
    }

    // parts 按 message_id 分组（IN 子句单查询，保序同 load_parts_for_messages）
    let parts = zcode_parts_by_message(&conn, &messages)?;

    let mut out = Vec::new();
    for (id, data, ts) in &messages {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue; // data 损坏 → 跳过该消息（防御）
        };
        let role = v.get("role").and_then(|r| r.as_str()).unwrap_or_default();
        let semantic = v
            .pointer("/semantics/kind")
            .and_then(|k| k.as_str())
            .unwrap_or_default();
        if matches!(
            (role, semantic),
            ("user", "todo_reminder") | ("user", "timeline_event")
        ) {
            continue; // 记账消息整条跳过（zcode_parser flatten_entries 同口径）
        }
        let msg_parts = parts.get(id);
        match role {
            "user" => {
                // 真实用户消息：text parts 拼接（实测 user 消息带 type=text part）
                let text = msg_parts
                    .map(|ps| zcode_text_of(ps, "text"))
                    .unwrap_or_default();
                if !text.is_empty() {
                    out.push(SessionMessage::text("user", text, *ts));
                }
            }
            "assistant" => {
                let Some(ps) = msg_parts else { continue };
                for p in ps {
                    let ptype = p.get("type").and_then(|t| t.as_str()).unwrap_or_default();
                    match ptype {
                        "text" => {
                            let t = p
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default()
                                .trim();
                            if !t.is_empty() {
                                out.push(SessionMessage::text("assistant", t, *ts));
                            }
                        }
                        "reasoning" => {
                            let t = p
                                .get("text")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default()
                                .trim();
                            if !t.is_empty() {
                                out.push(SessionMessage::text("thinking", t, *ts));
                            }
                        }
                        "tool" => {
                            // 实测形态 {"type":"tool","tool":"Skill","state":{"input":{...}}}
                            let name = p
                                .get("tool")
                                .and_then(|t| t.as_str())
                                .or_else(|| p.get("name").and_then(|t| t.as_str()));
                            let args = p
                                .pointer("/state/input")
                                .filter(|v| !v.is_null())
                                .and_then(|v| serde_json::to_string(v).ok());
                            out.push(SessionMessage::tool_call(
                                tool_summary(name),
                                *ts,
                                name.map(String::from),
                                args,
                            ));
                        }
                        // step-start / step-finish / timeline / file / compaction 是
                        // 步进边界与记账标记，无内容载荷，不产出条目（接口 kind 集无
                        // turn-start/turn-end 形态——Task 7 修订，见报告）
                        _ => {}
                    }
                }
            }
            _ => {} // 未知角色 → 跳过
        }
    }
    // SQLite 查询自带 LIMIT：无「头部截断」语义（truncated 恒 false，Bug 1 契约）
    Ok(page(out, limit, false))
}

/// ZCode：一组消息的全部 parts（IN 子句，sequence 列存在则保流顺序，否则 rowid）
fn zcode_parts_by_message(
    conn: &rusqlite::Connection,
    messages: &[(String, String, Option<i64>)],
) -> Result<std::collections::HashMap<String, Vec<serde_json::Value>>, String> {
    let mut map: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    if messages.is_empty() {
        return Ok(map);
    }
    let order_col = if conn.prepare("SELECT sequence FROM part LIMIT 0").is_ok() {
        "sequence"
    } else {
        "rowid"
    };
    let placeholders: Vec<String> = (0..messages.len()).map(|i| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT message_id, data FROM part WHERE message_id IN ({}) ORDER BY {order_col}",
        placeholders.join(",")
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("ZCode parts 查询失败: {e}"))?;
    let ids: Vec<&str> = messages.iter().map(|(id, _, _)| id.as_str()).collect();
    let params: Vec<&dyn rusqlite::types::ToSql> = ids
        .iter()
        .map(|id| id as &dyn rusqlite::types::ToSql)
        .collect();
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((sqlite_text(row, 0)?, sqlite_text(row, 1)?))
        })
        .map_err(|e| format!("ZCode parts 查询失败: {e}"))?;
    for row in rows.filter_map(|r| r.ok()) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&row.1) {
            map.entry(row.0).or_default().push(v);
        }
    }
    Ok(map)
}

/// ZCode：parts 里指定 type 的 text 字段拼接（user 消息正文）
fn zcode_text_of(parts: &[serde_json::Value], ptype: &str) -> String {
    parts
        .iter()
        .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some(ptype))
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
}

// ============================================================
// dsh：~/.dsh/sessions/<project>/<session-dir>/session.vN.jsonl.zstd
// ============================================================

fn read_dsh_messages_with(
    home: &Path,
    env_home: Option<&str>,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    use crate::monitor::dsh::log;
    // 数据根（fix round 1 Important 1）：$DSH_HOME 覆盖优先（M0 F14：env 非空 >
    // ~/.dsh），与看板生产扫描 scan_sessions(&dsh_home()) **同源**——否则设了
    // DSH_HOME 的机器上会话卡在板、详情 404。env 值由生产薄壳作参数传入
    // （dsh_home_with 的双参注入同款），测试传 None 走回落分支，零真实 env 接触
    let dsh_root = match env_home {
        Some(v) if !v.trim().is_empty() => std::path::PathBuf::from(v),
        _ => home.join(".dsh"),
    };
    let sessions_root = dsh_root.join("sessions");
    let Ok(entries) = std::fs::read_dir(&sessions_root) else {
        return Err("dsh sessions 目录不存在".to_string());
    };
    // 目录名是转义形态不可反解 id（M0 评审漏项 #3）——必须逐会话读 header 匹配
    for project_dir in entries.flatten() {
        let ppath = project_dir.path();
        if !ppath.is_dir() {
            continue;
        }
        let Ok(session_dirs) = std::fs::read_dir(&ppath) else {
            continue;
        };
        for sdir in session_dirs.flatten() {
            let spath = sdir.path();
            if !spath.is_dir() {
                continue;
            }
            // 代际选择：取版本最大者（scan_sessions 同款）；读不到（0 字节/损坏）跳过。
            // 先探测代际存在（与 read_best_generation 重复枚举，但让「无代际文件」与
            // 「解码失败」同走 continue，分支语义清晰）
            if log::generation_logs(&spath).is_empty() {
                continue;
            }
            let Some(read) = log::read_best_generation(&spath) else {
                continue;
            };
            let Some(header) = log::parse_header(&read.text) else {
                continue;
            };
            if header.id != session_id {
                continue;
            }
            // 命中：事件流映射（纯函数，独立可测）
            let events = log::parse_events(&read.text);
            // dsh 走自有 zstd 代际整读，无字节尾窗（truncated 恒 false，Bug 1 契约）
            return Ok(page(map_dsh_events(&events), limit, false));
        }
    }
    Err(format!("dsh 会话不存在或日志不可读: {session_id}"))
}

/// dsh 事件流 → 统一条目（纯函数）。跳过注入与记账事件（M0 F8：user/message 仅
/// source.kind=="user" 是真人；system/message 是插件注入）。
/// 产出映射：assistant/message.content[] 的 text→assistant、reasoning→thinking、
/// tool-call→tool-call；tool/call→tool-call；tool/result→tool-result。
/// 同一调用在事件流里两路呈现（assistant 内嵌块 `id` + 独立 tool/call `callId`，
/// 黄金样本实证同 id）→ 按调用 id 去重保留先出现者；id 缺失保守保留（无键判等，
/// 去重有误杀真实并行调用的风险）（fix round 1 Important 2）
fn map_dsh_events(events: &[crate::monitor::dsh::log::DshEvent]) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    // 已出条的调用 id 集（内嵌块 id 与 tool/call callId 同一 id 空间）
    let mut seen_calls: std::collections::HashSet<String> = std::collections::HashSet::new();
    for e in events {
        let ts = e.time;
        match e.kind.as_str() {
            "user/message" => {
                if e.data.pointer("/source/kind").and_then(|v| v.as_str()) != Some("user") {
                    continue; // agent-instructions / plugin / skill-catalog 注入，过滤（M0 F8）
                }
                let text =
                    join_text_parts(e.data.get("content").unwrap_or(&serde_json::Value::Null));
                if !text.trim().is_empty() {
                    out.push(SessionMessage::text("user", text, ts));
                }
            }
            "assistant/message" => {
                let content = e
                    .data
                    .pointer("/message/content")
                    .unwrap_or(&serde_json::Value::Null);
                let Some(arr) = content.as_array() else {
                    continue;
                };
                for c in arr {
                    match c.get("type").and_then(|t| t.as_str()).unwrap_or_default() {
                        "text" => {
                            let t = c.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(SessionMessage::text("assistant", t, ts));
                            }
                        }
                        "reasoning" => {
                            let t = c.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(SessionMessage::text("thinking", t, ts));
                            }
                        }
                        "tool-call" => {
                            let name = c.get("name").and_then(|t| t.as_str());
                            // arguments 已是 JSON 字符串；防御非串形态（序列化兜底）
                            let args = match c.get("arguments") {
                                Some(serde_json::Value::String(s)) => Some(s.clone()),
                                Some(v) if !v.is_null() => serde_json::to_string(v).ok(),
                                _ => None,
                            };
                            // 同 id 去重（保留先出现者）；id 缺失保守保留
                            match c.get("id").and_then(|i| i.as_str()) {
                                Some(id) if !seen_calls.insert(id.to_string()) => continue,
                                _ => {}
                            }
                            out.push(SessionMessage::tool_call(
                                tool_summary(name),
                                ts,
                                name.map(String::from),
                                args,
                            ));
                        }
                        _ => {}
                    }
                }
            }
            "tool/call" => {
                // 独立工具调用事件：data.name + data.arguments（JSON 字符串）
                let name = e.data.get("name").and_then(|t| t.as_str());
                let args = match e.data.get("arguments") {
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(v) if !v.is_null() => serde_json::to_string(v).ok(),
                    _ => None,
                };
                // 同 id 去重（保留先出现者）；id 缺失保守保留
                match e.data.get("callId").and_then(|i| i.as_str()) {
                    Some(id) if !seen_calls.insert(id.to_string()) => continue,
                    _ => {}
                }
                out.push(SessionMessage::tool_call(
                    tool_summary(name),
                    ts,
                    name.map(String::from),
                    args,
                ));
            }
            "tool/result" => {
                // 实测形态：data.message.content[] = [{type:"tool-result", content:[{type:"text",...}]}]
                let blocks = e
                    .data
                    .pointer("/message/content")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let mut texts = Vec::new();
                if let Some(arr) = blocks.as_array() {
                    for b in arr {
                        if let Some(t) = b.get("content").and_then(content_block_text) {
                            texts.push(t);
                        }
                    }
                }
                let text = texts.join(" ");
                out.push(SessionMessage::text(
                    "tool-result",
                    if text.is_empty() {
                        "工具结果".to_string()
                    } else {
                        text
                    },
                    ts,
                ));
            }
            // session / turn|step 边界 / approval / request / session/title /
            // system/message（插件注入）/ agent/inbox / permission / sandbox → 跳过
            _ => {}
        }
    }
    out
}

// ============================================================
// Claude：~/.claude/projects/<mangled-cwd>/<session-uuid>.jsonl
// ============================================================

fn read_claude_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let path = find_jsonl_in_projects(home, ".claude", session_id)?;
    // Bug 1：字节窗随 limit 放大（byte_budget），头部截断经 truncated 上报
    let (lines, truncated) = crate::monitor::jsonl::read_recent_lines_with_budget(
        &path,
        line_budget(limit),
        byte_budget(limit),
    );
    Ok(page(map_claude_lines(&lines), limit, truncated))
}

/// Claude JSONL 行 → 统一条目（纯函数）。行协议：type user/assistant +
/// message.content（字符串或块数组：text/thinking/tool_use/tool_result）。
/// isMeta 行是系统注入，跳过；summary/system/file-history-snapshot 等记账行跳过
fn map_claude_lines(lines: &[String]) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("isMeta").and_then(|m| m.as_bool()) == Some(true) {
            continue; // 系统注入（Meta 行）
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(iso_to_ms);
        let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        let Some(content) = v.pointer("/message/content") else {
            continue;
        };
        match msg_type {
            "user" => match content {
                serde_json::Value::String(s) => {
                    if !s.trim().is_empty() {
                        out.push(SessionMessage::text("user", s.trim().to_string(), ts));
                    }
                }
                serde_json::Value::Array(blocks) => {
                    for b in blocks {
                        match b.get("type").and_then(|t| t.as_str()).unwrap_or_default() {
                            "text" => {
                                let t = b.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                                if !t.trim().is_empty() {
                                    out.push(SessionMessage::text("user", t, ts));
                                }
                            }
                            "tool_result" => {
                                // content 双形态（字符串 / text 块数组）
                                let text = b
                                    .get("content")
                                    .and_then(content_block_text)
                                    .unwrap_or_else(|| "工具结果".to_string());
                                out.push(SessionMessage::text("tool-result", text, ts));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            "assistant" => {
                let Some(blocks) = content.as_array() else {
                    continue;
                };
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()).unwrap_or_default() {
                        "text" => {
                            let t = b.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(SessionMessage::text("assistant", t, ts));
                            }
                        }
                        "thinking" => {
                            let t = b
                                .get("thinking")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default();
                            if !t.trim().is_empty() {
                                out.push(SessionMessage::text("thinking", t, ts));
                            }
                        }
                        "tool_use" => {
                            let name = b.get("name").and_then(|t| t.as_str());
                            let args = b
                                .get("input")
                                .filter(|v| !v.is_null())
                                .and_then(|v| serde_json::to_string(v).ok());
                            out.push(SessionMessage::tool_call(
                                tool_summary(name),
                                ts,
                                name.map(String::from),
                                args,
                            ));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

// ============================================================
// Codex：rollout JSONL（CLI 线路）→ 未命中回退 thread_history SQLite（APP 线路）
// ============================================================

fn read_codex_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    // 路线 1：rollout JSONL（CLI 前端线路；两条线路共享同一会话 id 空间——
    // codex_thread_parser 以 rollout session id 排除 DB 侧重复出卡，即此结论）
    if let Ok(msgs) = read_codex_rollout_with(home, session_id, limit) {
        return Ok(msgs);
    }
    // 路线 2：thread_history_*.sqlite 内容投影（APP 线路；remote_control 活跃对话
    // 投影滞后属上游限制，此处如实返回已投影内容）
    if let Ok(msgs) = read_codex_thread_with(home, session_id, limit) {
        return Ok(msgs);
    }
    Err(format!(
        "codex 会话不存在（rollout 与 thread history 均未命中）: {session_id}"
    ))
}

/// rollout 递归收集（sessions/<y>/<m>/<d>/rollout-*.jsonl；codex_parser
/// is_rollout_file 判据的本地同款——原函数私有，判据三行不值得跨模块提权）
fn collect_rollout_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rollout_files(&p, out);
        } else if p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout") && n.ends_with(".jsonl"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// rollout 首行 session_meta → 会话 id（payload.id；缺失/损坏 None）
fn rollout_session_id(path: &Path) -> Option<String> {
    let first = crate::monitor::jsonl::read_first_lines(path, 1)
        .into_iter()
        .next()?;
    let v: serde_json::Value = serde_json::from_str(&first).ok()?;
    v.pointer("/payload/id")
        .and_then(|i| i.as_str())
        .map(String::from)
}

fn read_codex_rollout_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let sessions_dir = home.join(".codex").join("sessions");
    let mut files = Vec::new();
    collect_rollout_files(&sessions_dir, &mut files);
    if files.is_empty() {
        return Err("codex rollout 目录为空".to_string());
    }
    // 快路径：rollout 文件名内嵌 session id（rollout-<ts>-<uuid>.jsonl，实测命名）；
    // 命中后仍读首行校验 payload.id，防同名巧合。快路径未中 → 全量首行匹配兜底
    // （历史会话文件名可能不含 id 的旧形态；单次请求路径，非轮询，可接受）
    let mut hit: Option<std::path::PathBuf> = None;
    if let Some(f) = files.iter().find(|f| {
        f.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.contains(session_id))
            .unwrap_or(false)
    }) {
        if rollout_session_id(f).as_deref() == Some(session_id) {
            hit = Some(f.clone());
        }
    }
    if hit.is_none() {
        for f in &files {
            if rollout_session_id(f).as_deref() == Some(session_id) {
                hit = Some(f.clone());
                break;
            }
        }
    }
    let Some(f) = hit else {
        return Err(format!("codex rollout 未命中: {session_id}"));
    };
    // Bug 1：字节窗随 limit 放大，头部截断经 truncated 上报
    let (lines, truncated) = crate::monitor::jsonl::read_recent_lines_with_budget(
        &f,
        line_budget(limit),
        byte_budget(limit),
    );
    Ok(page(map_codex_lines(&lines), limit, truncated))
}

/// Codex rollout 行 → 统一条目（纯函数；codex_entry_kind 的内容版映射）。
/// response_item 外壳：message(user/assistant)、function_call、function_call_output、
/// reasoning(summary)；session_meta / event_msg / turn_context 记账跳过
fn map_codex_lines(lines: &[String]) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(iso_to_ms);
        let Some(payload) = v.get("payload") else {
            continue;
        };
        match payload
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
        {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or_default();
                let text =
                    join_text_parts(payload.get("content").unwrap_or(&serde_json::Value::Null));
                // user / assistant 正文（developer 等系统角色跳过）
                if matches!(role, "user" | "assistant") && !text.trim().is_empty() {
                    out.push(SessionMessage::text(role, text, ts));
                }
            }
            "function_call" => {
                let name = payload.get("name").and_then(|t| t.as_str());
                let args = payload
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .map(String::from);
                out.push(SessionMessage::tool_call(
                    tool_summary(name),
                    ts,
                    name.map(String::from),
                    args,
                ));
            }
            "function_call_output" => {
                // output 双形态（字符串 / 对象）
                let text = match payload.get("output") {
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(v) if !v.is_null() => serde_json::to_string(v).ok(),
                    _ => None,
                }
                .unwrap_or_else(|| "工具结果".to_string());
                out.push(SessionMessage::text("tool-result", text, ts));
            }
            "reasoning" => {
                // summary 元素双形态（实测纯字符串；亦容忍 {text} 对象）
                let text = payload
                    .get("summary")
                    .and_then(|s| s.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|i| {
                                i.as_str().map(String::from).or_else(|| {
                                    i.get("text").and_then(|t| t.as_str()).map(String::from)
                                })
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    out.push(SessionMessage::text("thinking", text, ts));
                }
            }
            _ => {}
        }
    }
    out
}

/// Codex APP 线路：thread_history_*.sqlite 的 thread_items（codex_thread_parser
/// 的表/列实测布局；item_type ∈ userMessage/agentMessage/commandExecution/
/// reasoning/fileChange/imageView/plan）
fn read_codex_thread_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    use crate::monitor::codex_thread_parser::CodexThreadRoots;
    let roots = CodexThreadRoots::from_home(home);
    let conn = crate::monitor::sqlite::open_readonly_with_timeout(&roots.history_db)
        .ok_or_else(|| "codex thread history db 不可读".to_string())?;
    let sql = format!(
        "SELECT item_type, item_json, created_at_ms FROM thread_items \
         WHERE thread_id = ?1 ORDER BY rollout_ordinal DESC LIMIT {}",
        line_budget(limit)
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Err("codex thread_items 表不可查".to_string());
    };
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return Err(format!("codex thread 未命中: {session_id}"));
    };
    let mut items: Vec<(String, String, Option<i64>)> = rows.filter_map(|r| r.ok()).collect();
    if items.is_empty() {
        return Err(format!("codex thread 未命中: {session_id}"));
    }
    items.reverse(); // 「最新在前」→ 文件序
    let mut out = Vec::new();
    for (item_type, item_json, ts) in &items {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(item_json) else {
            continue;
        };
        match item_type.as_str() {
            "userMessage" => {
                let text = join_text_parts(v.get("content").unwrap_or(&serde_json::Value::Null));
                if !text.trim().is_empty() {
                    out.push(SessionMessage::text("user", text, *ts));
                }
            }
            "agentMessage" => {
                let t = v.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                if !t.trim().is_empty() {
                    out.push(SessionMessage::text("assistant", t, *ts));
                }
            }
            "commandExecution" => {
                let cmd = v
                    .get("command")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();
                let args = serde_json::json!({ "command": cmd }).to_string();
                out.push(SessionMessage::tool_call(
                    if cmd.is_empty() {
                        "工具调用".to_string()
                    } else {
                        cmd.to_string()
                    },
                    *ts,
                    Some("exec".to_string()),
                    Some(args),
                ));
            }
            "reasoning" => {
                let text = v
                    .get("summary")
                    .and_then(|s| s.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|i| {
                                i.as_str().map(String::from).or_else(|| {
                                    i.get("text").and_then(|t| t.as_str()).map(String::from)
                                })
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    out.push(SessionMessage::text("thinking", text, *ts));
                }
            }
            "fileChange" => {
                let paths: Vec<String> = v
                    .get("changes")
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| c.get("path").and_then(|p| p.as_str()))
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                if !paths.is_empty() {
                    let args =
                        serde_json::to_string(&v.get("changes").cloned().unwrap_or_default()).ok();
                    out.push(SessionMessage::tool_call(
                        format!("修改 {}", paths.join(", ")),
                        *ts,
                        Some("file".to_string()),
                        args,
                    ));
                }
            }
            // imageView / plan / 未知类型 → 跳过（codex_thread_parser 记账同口径）
            _ => {}
        }
    }
    // SQLite 查询自带 LIMIT：无「头部截断」语义（truncated 恒 false，Bug 1 契约）
    Ok(page(out, limit, false))
}

// ============================================================
// Kimi：session_index.jsonl 定位 + agents/main/wire.jsonl 事件流
// ============================================================

fn read_kimi_messages_with(
    home: &Path,
    env_home: Option<&str>,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    use crate::monitor::kimi_parser::{
        parse_session_index, resolve_data_root, resolve_session_dir,
    };
    // 数据根（fix round 1 Minor①）：KIMI_CODE_HOME env 值以**参数**传入
    // resolve_data_root（双参注入，kimi_parser 自用同款），生产薄壳读 env、
    // 测试传 None——内容层自身不读真实进程 env，与并行 env 测试无互斥交集
    let Some(root) = resolve_data_root(env_home, home) else {
        return Err("kimi 数据根不存在".to_string());
    };
    let Some(entry) = parse_session_index(&root)
        .into_iter()
        .find(|e| e.session_id == session_id)
    else {
        return Err(format!("kimi 会话不在索引中: {session_id}"));
    };
    let session_dir = resolve_session_dir(&root, &entry.session_dir);
    // 信任边界（stat_index_entry 同款）：sessionDir 必须落在 sessions 根之下
    if !session_dir.starts_with(&root.sessions) {
        return Err(format!("kimi sessionDir 越界: {session_id}"));
    }
    let wire = session_dir.join("agents").join("main").join("wire.jsonl");
    // Bug 1：字节窗随 limit 放大，头部截断经 truncated 上报（kimi 实测 854KB 胖
    // 会话头部 39.5% 被旧 512KB 恒定窗切掉）
    let (lines, truncated) = crate::monitor::jsonl::read_recent_lines_with_budget(
        &wire,
        line_budget(limit),
        byte_budget(limit),
    );
    if lines.is_empty() {
        return Err(format!("kimi wire.jsonl 不可读: {session_id}"));
    }
    let msgs = map_kimi_lines(&lines);
    Ok(page(msgs, limit, truncated))
}

/// Kimi wire.jsonl 行 → 统一条目（纯函数；entry_text/entry_status 的内容版映射）。
/// 流式 content.part 事件会碎片化 → 连续同 kind 文本行合并为一条（tool 行为边界）；
/// turn.prompt 与 append_message(user) 是同一输入的两次落笔（实机取证）→ 连续全等去重
fn map_kimi_lines(lines: &[String]) -> Vec<SessionMessage> {
    let mut out: Vec<SessionMessage> = Vec::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let etype = v.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        let ts = v.get("time").and_then(|t| t.as_i64());
        match etype {
            // 用户输入（turn 级事件，input[]: [{type:"text",text}]
            "turn.prompt" | "turn.steer" => {
                let text = v
                    .get("input")
                    .and_then(|i| i.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    push_kimi(&mut out, SessionMessage::text("user", text, ts));
                }
            }
            "context.append_message" => {
                let msg = v.get("message");
                let role = msg
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .unwrap_or_default();
                let text = msg
                    .map(|m| join_text_parts(m.get("content").unwrap_or(&serde_json::Value::Null)))
                    .unwrap_or_default();
                if !text.trim().is_empty() {
                    let kind = if role == "user" { "user" } else { "assistant" };
                    push_kimi(&mut out, SessionMessage::text(kind, text, ts));
                }
            }
            "context.append_loop_event" => {
                let Some(event) = v.get("event") else {
                    continue;
                };
                match event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                {
                    "content.part" => {
                        let Some(part) = event.get("part") else {
                            continue;
                        };
                        match part
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or_default()
                        {
                            "text" => {
                                let t = part
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .unwrap_or_default();
                                if !t.trim().is_empty() {
                                    push_kimi(&mut out, SessionMessage::text("assistant", t, ts));
                                }
                            }
                            "think" => {
                                // 实测 think 块的字段名是 think（非 text）
                                let t = part
                                    .get("think")
                                    .and_then(|t| t.as_str())
                                    .unwrap_or_default();
                                if !t.trim().is_empty() {
                                    push_kimi(&mut out, SessionMessage::text("thinking", t, ts));
                                }
                            }
                            _ => {}
                        }
                    }
                    "tool.call" => {
                        let name = event.get("name").and_then(|t| t.as_str());
                        // 实测 args 是对象（非 JSON 字符串）→ 序列化
                        let args = match event.get("args") {
                            Some(v) if !v.is_null() => serde_json::to_string(v).ok(),
                            _ => None,
                        };
                        out.push(SessionMessage::tool_call(
                            tool_summary(name),
                            ts,
                            name.map(String::from),
                            args,
                        ));
                    }
                    "tool.result" => {
                        // 实测 result 是对象：{"output": "..."}
                        let text = event
                            .pointer("/result/output")
                            .and_then(|o| o.as_str())
                            .map(String::from)
                            .or_else(|| match event.get("result") {
                                Some(s @ serde_json::Value::String(_)) => {
                                    s.as_str().map(String::from)
                                }
                                Some(v) if !v.is_null() => serde_json::to_string(v).ok(),
                                _ => None,
                            })
                            .unwrap_or_else(|| "工具结果".to_string());
                        out.push(SessionMessage::text("tool-result", text, ts));
                    }
                    _ => {} // step.begin/end 等边界 → 跳过
                }
            }
            // metadata / config.update / usage.record / turn.ended / interaction.* 等跳过
            _ => {}
        }
    }
    out
}

/// Kimi 追加条目（连续合并 + 全等去重）：
/// - 与上一条同为文本类且同 kind → content 以换行拼接（流式碎片合并）；
/// - 与上一条全等（role+kind+content）→ 丢弃（turn.prompt 与 append_message 双写）。
/// - tool-call / tool-result 恒独立成条（天然是边界）
///
/// 合并只作用于 assistant / thinking（流式碎片）；两条不同内容的 user 消息是两次
/// 独立输入，保持独立条目
fn push_kimi(out: &mut Vec<SessionMessage>, msg: SessionMessage) {
    if let Some(last) = out.last_mut() {
        if last.kind == msg.kind && msg.tool_name.is_none() && last.tool_name.is_none() {
            if last.content == msg.content {
                return; // 全等去重（双写）
            }
            // 流式碎片合并仅限 agent 侧连续正文/思考；user 不合并（两次输入是两条）
            if matches!(msg.kind.as_str(), "assistant" | "thinking") {
                last.content.push('\n');
                last.content.push_str(&msg.content);
                return;
            }
        }
    }
    out.push(msg);
}

// ============================================================
// WorkBuddy：~/.workbuddy/projects/*/<sessionId>.jsonl（OpenAI 风格）
// ============================================================

fn read_workbuddy_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let path = find_jsonl_in_projects(home, ".workbuddy", session_id)?;
    // Bug 1：字节窗随 limit 放大，头部截断经 truncated 上报
    let (lines, truncated) = crate::monitor::jsonl::read_recent_lines_with_budget(
        &path,
        line_budget(limit),
        byte_budget(limit),
    );
    Ok(page(map_workbuddy_lines(&lines), limit, truncated))
}

/// WorkBuddy JSONL 行 → 统一条目（纯函数；workbuddy_entry_kind 的内容版映射）。
/// message(user/assistant) 正文；function_call → tool-call；function_call_result →
/// tool-result；reasoning / file-history-snapshot 等记账行跳过
fn map_workbuddy_lines(lines: &[String]) -> Vec<SessionMessage> {
    let mut out = Vec::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let vtype = v.get("type").and_then(|t| t.as_str()).unwrap_or_default();
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(iso_to_ms);
        match vtype {
            "message" => {
                let role = v.get("role").and_then(|r| r.as_str()).unwrap_or_default();
                // content 双形态（数组 text 块为主，实测；字符串防御兼容）
                let text = match v.get("content") {
                    Some(c @ serde_json::Value::Array(_)) => join_text_parts(c),
                    Some(serde_json::Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                if !text.trim().is_empty() {
                    let kind = if role == "user" { "user" } else { "assistant" };
                    out.push(SessionMessage::text(kind, text, ts));
                }
            }
            "function_call" => {
                let name = v.get("name").and_then(|t| t.as_str());
                let args = v
                    .get("arguments")
                    .and_then(|a| a.as_str())
                    .map(String::from);
                out.push(SessionMessage::tool_call(
                    tool_summary(name),
                    ts,
                    name.map(String::from),
                    args,
                ));
            }
            "function_call_result" => {
                let text = match v.get("output") {
                    Some(serde_json::Value::String(s)) => Some(s.clone()),
                    Some(c @ serde_json::Value::Array(_)) => Some(join_text_parts(c)),
                    Some(vv) if !vv.is_null() => serde_json::to_string(vv).ok(),
                    _ => None,
                }
                .unwrap_or_else(|| "工具结果".to_string());
                out.push(SessionMessage::text("tool-result", text, ts));
            }
            _ => {}
        }
    }
    out
}

// ============================================================
// OpenCode：~/.local/share/opencode/opencode.db（message + part 表）
// ============================================================

fn read_opencode_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let db = home
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db");
    let conn = crate::monitor::sqlite::open_readonly_with_timeout(&db)
        .ok_or_else(|| "opencode.db 不可读".to_string())?;
    let sql = format!(
        "SELECT id, data, time_created FROM message WHERE session_id = ?1 \
         ORDER BY time_created DESC LIMIT {limit}"
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Err("opencode message 表不可查".to_string());
    };
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            sqlite_text(row, 0)?,
            sqlite_text(row, 1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    });
    let Ok(rows) = rows else {
        return Err(format!("opencode 查询失败: {session_id}"));
    };
    let mut messages: Vec<(String, String, Option<i64>)> = rows.filter_map(|r| r.ok()).collect();
    if messages.is_empty() {
        return Err(format!("opencode 会话不存在: {session_id}"));
    }
    messages.reverse(); // 文件序
    let mut out = Vec::new();
    for (id, data, ts) in &messages {
        let Ok(mv) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        let role = mv.get("role").and_then(|r| r.as_str()).unwrap_or_default();
        if role != "user" && role != "assistant" {
            continue;
        }
        // parts 按消息读取（opencode_parser get_message_text 同款：time_created ASC）
        let Ok(mut pstmt) =
            conn.prepare("SELECT data FROM part WHERE message_id = ?1 ORDER BY time_created ASC")
        else {
            continue;
        };
        let prows = pstmt.query_map([id], |row| sqlite_text(row, 0));
        let Ok(prows) = prows else { continue };
        for pdata in prows.filter_map(|r| r.ok()) {
            let Ok(pv) = serde_json::from_str::<serde_json::Value>(&pdata) else {
                continue;
            };
            match pv.get("type").and_then(|t| t.as_str()).unwrap_or_default() {
                "text" => {
                    let t = pv.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                    if !t.trim().is_empty() {
                        out.push(SessionMessage::text(
                            if role == "user" { "user" } else { "assistant" },
                            t,
                            *ts,
                        ));
                    }
                }
                "reasoning" => {
                    let t = pv.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                    if !t.trim().is_empty() {
                        out.push(SessionMessage::text("thinking", t, *ts));
                    }
                }
                "tool" => {
                    // 实测形态未定（本机库无 tool part 样本）——多字段名防御取值
                    let name = pv
                        .get("tool")
                        .and_then(|t| t.as_str())
                        .or_else(|| pv.get("name").and_then(|t| t.as_str()));
                    let args = pv
                        .pointer("/state/input")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::to_string(v).ok());
                    out.push(SessionMessage::tool_call(
                        tool_summary(name),
                        *ts,
                        name.map(String::from),
                        args,
                    ));
                }
                // step-start / patch / file / 未知 → 跳过
                _ => {}
            }
        }
    }
    // SQLite 查询自带 LIMIT：无「头部截断」语义（truncated 恒 false，Bug 1 契约）
    Ok(page(out, limit, false))
}

// ============================================================
// OpenClaw：~/.openclaw/state/openclaw.sqlite 的 acp_replay_events（ACP 协议
// replay 日志，2026-09-15 实机探测：session_id/seq/at/update_json，
// update_json 为 ACP sessionUpdate 结构）。无该存储 → Err 优雅降级（Task 7 裁决）
// ============================================================

const OPENCLAW_NO_HISTORY: &str = "OpenClaw 暂无本地消息历史可读";

fn read_openclaw_messages_with(
    home: &Path,
    session_id: &str,
    limit: usize,
) -> Result<MessagesPage, String> {
    let db = home.join(".openclaw").join("state").join("openclaw.sqlite");
    if !db.exists() {
        return Err(OPENCLAW_NO_HISTORY.to_string());
    }
    let Some(conn) = crate::monitor::sqlite::open_readonly_with_timeout(&db) else {
        return Err(OPENCLAW_NO_HISTORY.to_string());
    };
    let sql = format!(
        "SELECT seq, at, update_json FROM acp_replay_events \
         WHERE session_id = ?1 ORDER BY seq ASC LIMIT {}",
        line_budget(limit)
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return Err(OPENCLAW_NO_HISTORY.to_string()); // 表缺失（旧版存储布局）
    };
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1)?,
            sqlite_text(row, 2)?,
        ))
    });
    let Ok(rows) = rows else {
        return Err(OPENCLAW_NO_HISTORY.to_string());
    };
    let events: Vec<(i64, Option<i64>, String)> = rows.filter_map(|r| r.ok()).collect();
    if events.is_empty() {
        return Err(OPENCLAW_NO_HISTORY.to_string());
    }
    let mut out = Vec::new();
    for (_seq, at, update_json) in &events {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(update_json) else {
            continue;
        };
        let update = v
            .get("sessionUpdate")
            .and_then(|u| u.as_str())
            .unwrap_or_default();
        // ACP ContentBlock 单对象形态 {type:"text",text}；防御数组变体
        let block_text = |v: &serde_json::Value| -> String {
            match v {
                serde_json::Value::Array(_) => join_text_parts(v),
                other => other
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default()
                    .to_string(),
            }
        };
        match update {
            "user_message_chunk" => {
                let t = v.get("content").map(block_text).unwrap_or_default();
                if !t.trim().is_empty() {
                    out.push(SessionMessage::text("user", t, *at));
                }
            }
            "agent_message_chunk" => {
                let t = v.get("content").map(block_text).unwrap_or_default();
                if !t.trim().is_empty() {
                    out.push(SessionMessage::text("assistant", t, *at));
                }
            }
            "agent_thought_chunk" => {
                let t = v.get("content").map(block_text).unwrap_or_default();
                if !t.trim().is_empty() {
                    out.push(SessionMessage::text("thinking", t, *at));
                }
            }
            "tool_call" | "tool_call_update" => {
                let title = v
                    .get("title")
                    .and_then(|t| t.as_str())
                    .or_else(|| v.get("toolCallId").and_then(|t| t.as_str()));
                let args = v
                    .get("rawInput")
                    .filter(|x| !x.is_null())
                    .and_then(|x| serde_json::to_string(x).ok());
                out.push(SessionMessage::tool_call(
                    tool_summary(title),
                    *at,
                    title.map(String::from),
                    args,
                ));
            }
            // session_info_update / available_commands_update / plan 等会话级
            // 元事件 → 跳过
            _ => {}
        }
    }
    // SQLite 查询自带 LIMIT：无「头部截断」语义（truncated 恒 false，Bug 1 契约）
    Ok(page(out, limit, false))
}

// ============================================================
// 测试（Task 7 Step 3）：黄金夹具 / 合成数据，零真实 ~/.zcode ~/.dsh 接触。
// 命名延续各 parser 测试惯例；ZCode/OpenCode/OpenClaw 用 tmp sqlite，
// dsh 用 tests/fixtures/dsh 黄金样本 + tmp home zstd 代际文件，
// JSONL 族（claude/codex/kimi/workbuddy）用合成行直测映射纯函数。
// ============================================================
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    // ==== 公共小件 ====

    const SID: &str = "sess_0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f";
    const UUID: &str = "0f1e2d3c-4b5a-4948-8276-9a0b8c7d6e5f";

    #[test]
    fn dispatch_rejects_unknown_tool_and_empty_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let r = read_session_messages_with(tmp.path(), "not-a-tool", SID, 200);
        assert!(r.unwrap_err().contains("未知工具"));
        let r = read_session_messages_with(tmp.path(), "zcode", "  ", 200);
        assert!(r.unwrap_err().contains("session_id 为空"));
        // 路径穿越防御：claude/workbuddy 文件定位按 id 拼 `.jsonl` 文件名，
        // 分隔符 / .. / NUL 必须在内容层入口 fail-closed
        for evil in ["../../etc/passwd", "a/b", "a\\b", "..", "x\\0y"] {
            let r = read_session_messages_with(tmp.path(), "claude", evil, 200);
            assert!(
                r.unwrap_err().contains("非法字符"),
                "穿越形态 {evil:?} 必须被拒绝"
            );
        }
    }

    /// camelCase 序列化契约（移动端字段名，勿漂移）：toolName/toolArgs/collapsed
    #[test]
    fn session_message_serializes_camel_case() {
        let m =
            SessionMessage::tool_call("调用 Bash", Some(1), Some("Bash".into()), Some("{}".into()));
        let v = serde_json::to_value(&m).unwrap();
        assert!(v.get("toolName").is_some(), "toolName 字段（camelCase）");
        assert!(v.get("toolArgs").is_some(), "toolArgs 字段（camelCase）");
        assert!(v.get("collapsed").is_some());
        assert!(v.get("seq").is_some() && v.get("role").is_some() && v.get("kind").is_some());
        assert_eq!(
            v["collapsed"],
            serde_json::json!(true),
            "tool-call 默认折叠"
        );
        // thinking 折叠、user/assistant/tool-result 不折叠
        assert!(SessionMessage::text("thinking", "x", None).collapsed);
        assert!(!SessionMessage::text("user", "x", None).collapsed);
        assert!(!SessionMessage::text("assistant", "x", None).collapsed);
        assert!(!SessionMessage::text("tool-result", "x", None).collapsed);
    }

    // ==== ZCode（tmp sqlite：message + part 表）====

    fn zcode_db(path: &Path) -> Connection {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY, session_id TEXT, sequence INTEGER,
                time_created INTEGER, data TEXT
             );
             CREATE TABLE part (
                id TEXT PRIMARY KEY, message_id TEXT, sequence INTEGER, data TEXT
             );",
        )
        .unwrap();
        conn
    }

    fn zcode_msg(conn: &Connection, id: &str, seq: i64, ts: i64, role: &str, kind: &str) {
        conn.execute(
            "INSERT INTO message (id, session_id, sequence, time_created, data) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![
                id,
                SID,
                seq,
                ts,
                format!(r#"{{"role":"{role}","semantics":{{"kind":"{kind}"}}}}"#)
            ],
        )
        .unwrap();
    }

    fn zcode_part(conn: &Connection, id: &str, mid: &str, seq: i64, data: &str) {
        conn.execute(
            "INSERT INTO part (id, message_id, sequence, data) VALUES (?1,?2,?3,?4)",
            rusqlite::params![id, mid, seq, data],
        )
        .unwrap();
    }

    /// 全 kind 映射 + 记账消息跳过（todo_reminder / timeline_event 整条丢弃）；
    /// step-start / step-finish 是步进边界，不产出条目
    #[test]
    fn zcode_maps_all_kinds_and_skips_bookkeeping() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = zcode_db(&tmp.path().join(".zcode/cli/db/db.sqlite"));
        // m1: 真实用户消息（user_prompt，text part）
        zcode_msg(&conn, "m1", 1, 1000, "user", "user_prompt");
        zcode_part(&conn, "p1", "m1", 1, r#"{"type":"text","text":"你好"}"#);
        // m2/m3: 记账消息（user 角色注入）——整条跳过，text part 不得泄入
        zcode_msg(&conn, "m2", 2, 1100, "user", "todo_reminder");
        zcode_part(
            &conn,
            "p2",
            "m2",
            1,
            r#"{"type":"text","text":"<todo>…</todo>"}"#,
        );
        zcode_msg(&conn, "m3", 3, 1200, "user", "timeline_event");
        zcode_part(&conn, "p3", "m3", 1, r#"{"type":"text","text":"timeline"}"#);
        // m4: assistant——reasoning → thinking、tool → tool-call（含 name/args）、
        //     text → assistant、step-start/step-finish → 无条目
        zcode_msg(&conn, "m4", 4, 1300, "assistant", "assistant");
        zcode_part(&conn, "q1", "m4", 1, r#"{"type":"step-start"}"#);
        zcode_part(
            &conn,
            "q2",
            "m4",
            2,
            r#"{"type":"reasoning","text":"先想一想"}"#,
        );
        zcode_part(
            &conn,
            "q3",
            "m4",
            3,
            r#"{"type":"tool","tool":"Skill","state":{"status":"completed","input":{"skill":"demo"}}}"#,
        );
        zcode_part(&conn, "q4", "m4", 4, r#"{"type":"text","text":"做完了"}"#);
        zcode_part(
            &conn,
            "q5",
            "m4",
            5,
            r#"{"type":"step-finish","reason":"end_turn"}"#,
        );

        let msgs = read_session_messages_with(tmp.path(), "zcode", SID, 200)
            .unwrap()
            .messages;
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["user", "thinking", "tool-call", "assistant"],
            "记账消息与步进边界不得产出条目，实得 {kinds:?}"
        );
        assert_eq!(msgs[0].content, "你好");
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].ts, Some(1000));
        assert_eq!(msgs[1].content, "先想一想");
        assert!(msgs[1].collapsed, "thinking 默认折叠");
        assert_eq!(msgs[2].tool_name.as_deref(), Some("Skill"));
        assert!(
            msgs[2]
                .tool_args
                .as_deref()
                .unwrap_or_default()
                .contains("\"skill\":\"demo\""),
            "tool_args 应为 state.input 的 JSON 串"
        );
        assert!(msgs[2].collapsed);
        assert_eq!(msgs[3].content, "做完了");
        // seq 是返回数组内的稳定递增号
        assert_eq!(msgs[3].seq, 3);
    }

    /// limit = 尾部窗口：更早的条目被截去，保留文件序末尾 N 条且 seq 重排 0..n
    #[test]
    fn zcode_limit_takes_tail_in_file_order() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = zcode_db(&tmp.path().join(".zcode/cli/db/db.sqlite"));
        for i in 0..5 {
            zcode_msg(&conn, &format!("m{i}"), i, 1000 + i, "user", "user_prompt");
            zcode_part(
                &conn,
                &format!("p{i}"),
                &format!("m{i}"),
                1,
                &format!(r#"{{"type":"text","text":"msg{i}"}}"#),
            );
        }
        let msgs = read_session_messages_with(tmp.path(), "zcode", SID, 3)
            .unwrap()
            .messages;
        let contents: Vec<&str> = msgs.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(contents, vec!["msg2", "msg3", "msg4"], "取尾部且保持文件序");
        assert_eq!(msgs[0].seq, 0, "seq 重排为 0..n");
        assert_eq!(msgs[2].seq, 2);
    }

    /// fix round 1 Minor②：limit 夹取边界——0（未传/坏参路径的 0 值）按 1 生效、
    /// 超大值截回上限 1000（读端在 impl 入口 clamp，此处锁定两端行为）。
    /// 两套独立 tmp home（ZCodeRoots 固定读 .zcode/cli/db/db.sqlite，不能同目录换库）
    #[test]
    fn limit_is_clamped_to_1_1000() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = zcode_db(&tmp.path().join(".zcode/cli/db/db.sqlite"));
        for i in 0..5 {
            zcode_msg(&conn, &format!("m{i}"), i, 1000 + i, "user", "user_prompt");
            zcode_part(
                &conn,
                &format!("p{i}"),
                &format!("m{i}"),
                1,
                &format!(r#"{{"type":"text","text":"msg{i}"}}"#),
            );
        }
        // 下界：0 → 按 1 生效（只出文件序最后 1 条）
        let msgs = read_session_messages_with(tmp.path(), "zcode", SID, 0)
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 1, "limit=0 必须夹为 1");
        assert_eq!(msgs[0].content, "msg4", "夹取后仍取尾部");
        // 上界：999999 → 截回 1000（1001 条消息夹去 1 条）
        let big = tempfile::tempdir().unwrap();
        let conn = zcode_db(&big.path().join(".zcode/cli/db/db.sqlite"));
        for i in 0..1001 {
            zcode_msg(&conn, &format!("m{i}"), i, 1000 + i, "user", "user_prompt");
            zcode_part(
                &conn,
                &format!("p{i}"),
                &format!("m{i}"),
                1,
                &format!(r#"{{"type":"text","text":"big{i}"}}"#),
            );
        }
        let r = read_session_messages_impl(big.path(), None, None, "zcode", SID, 999_999)
            .unwrap()
            .messages;
        assert_eq!(r.len(), 1000, "limit 上限截回 1000");
        assert_eq!(r[0].content, "big1", "截去最旧的 1 条（文件序头部）");
    }

    #[test]
    fn zcode_missing_session_or_db_errs() {
        let tmp = tempfile::tempdir().unwrap();
        // 库不存在
        assert!(read_session_messages_with(tmp.path(), "zcode", SID, 200).is_err());
        // 库在但会话不在
        let conn = zcode_db(&tmp.path().join(".zcode/cli/db/db.sqlite"));
        zcode_msg(&conn, "m1", 1, 1000, "user", "user_prompt");
        zcode_part(&conn, "p1", "m1", 1, r#"{"type":"text","text":"hi"}"#);
        let r = read_session_messages_with(
            tmp.path(),
            "zcode",
            "sess_ffffffff-ffff-ffff-ffff-ffffffffffff",
            200,
        );
        assert!(r.is_err());
    }

    // ==== dsh（黄金夹具 + tmp home zstd 代际文件）====

    fn dsh_fixture(name: &str) -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dsh")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取夹具失败 {p:?}: {e}"))
    }

    /// 黄金样本 sample5 映射：真人输入保留、注入（agent-instructions / plugin /
    /// skill-catalog）过滤；同一调用（内嵌块 id 与 tool/call callId 同 id）去重后
    /// 恰好一条 tool-call（fix round 1 Important 2）
    #[test]
    fn dsh_sanitized_fixture_filters_injection_and_maps_tools() {
        let text = dsh_fixture("sample5-approval-pending.sanitized.jsonl");
        let events = crate::monitor::dsh::log::parse_events(&text);
        let msgs = map_dsh_events(&events);
        // 真人输入在板（M0 F8：source.kind=="user" 才是真人）
        assert!(
            msgs.iter()
                .any(|m| m.kind == "user" && m.content.contains("run a command")),
            "真人输入必须出条目"
        );
        // 注入过滤：agent-instructions / skill-catalog / plugin 的大段注入不得出条目
        let injected = msgs.iter().any(|m| {
            m.kind == "user" && (m.content.contains("chars") || m.content.contains("AGENTS"))
        });
        assert!(!injected, "注入 user/message 必须被过滤");
        // 同一调用两路呈现（assistant content 内嵌块 id == tool/call callId，均 bash）：
        // 按调用 id 去重后恰好 1 条，不得在板上看成两次调用
        let tool_calls: Vec<&SessionMessage> =
            msgs.iter().filter(|m| m.kind == "tool-call").collect();
        assert_eq!(
            tool_calls.len(),
            1,
            "同 id 调用必须去重为一条，实得 {tool_calls:?}"
        );
        assert_eq!(tool_calls[0].tool_name.as_deref(), Some("bash"));
        assert!(
            tool_calls[0]
                .tool_args
                .as_deref()
                .is_some_and(|a| !a.is_empty()),
            "arguments（JSON 字符串）应透传为 toolArgs"
        );
        // 记账事件（session / turn / step / approval / session/title / system/message）不出条目
        assert!(!msgs
            .iter()
            .any(|m| m.kind == "user" && m.content.contains("permission")));
    }

    /// 去重防御：调用 id 缺失（内嵌块无 id / tool/call 无 callId）时保守保留两条——
    /// 无键可判等，去重有误杀真实并行调用的风险
    #[test]
    fn dsh_tool_calls_without_ids_are_kept_separately() {
        let events = vec![
            crate::monitor::dsh::log::DshEvent {
                kind: "assistant/message".into(),
                seq: Some(1),
                time: Some(10),
                data: serde_json::json!({
                    "message": { "content": [
                        { "type": "tool-call", "name": "bash" }
                    ]}
                }),
            },
            crate::monitor::dsh::log::DshEvent {
                kind: "tool/call".into(),
                seq: Some(2),
                time: Some(11),
                data: serde_json::json!({ "name": "bash", "arguments": "{}" }),
            },
        ];
        let msgs = map_dsh_events(&events);
        let tool_calls: Vec<&SessionMessage> =
            msgs.iter().filter(|m| m.kind == "tool-call").collect();
        assert_eq!(
            tool_calls.len(),
            2,
            "id 缺失时不得去重（无键判等，保守保留）"
        );
    }

    /// tool/result 事件 → tool-result 条目（sample2 含 isError 工具失败样本）
    #[test]
    fn dsh_tool_result_maps_from_fixture() {
        let text = dsh_fixture("sample2-tool-error.sanitized.jsonl");
        let events = crate::monitor::dsh::log::parse_events(&text);
        let msgs = map_dsh_events(&events);
        let results: Vec<&SessionMessage> =
            msgs.iter().filter(|m| m.kind == "tool-result").collect();
        assert!(!results.is_empty(), "tool/result 必须映射 tool-result");
        assert!(
            results.iter().any(|m| m.content.contains("Error")),
            "工具结果文本应进入 content"
        );
        assert!(!results[0].collapsed, "tool-result 不折叠");
    }

    /// reasoning 块 → thinking（黄金样本无 reasoning 形态，合成事件补位）
    #[test]
    fn dsh_reasoning_block_maps_to_thinking() {
        let events = vec![crate::monitor::dsh::log::DshEvent {
            kind: "assistant/message".into(),
            seq: Some(1),
            time: Some(42),
            data: serde_json::json!({
                "message": { "content": [
                    { "type": "reasoning", "text": "推理过程" },
                    { "type": "text", "text": "结论" }
                ]}
            }),
        }];
        let msgs = map_dsh_events(&events);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].kind, "thinking");
        assert_eq!(msgs[0].content, "推理过程");
        assert!(msgs[0].collapsed);
        assert_eq!(msgs[1].kind, "assistant");
    }

    /// 在指定 dsh 根下写一个 zstd 代际会话（tmp 夹具共享助手）
    fn write_dsh_generation(dsh_root: &Path, session_id: &str) {
        let header = format!(
            r#"{{"type":"session","version":3,"id":"{session_id}","cwd":"/tmp/proj","createdAt":1000,"isSeeded":false}}"#
        );
        let events = concat!(
            r#"{"type":"turn/start","seq":4,"data":{}}"#,
            "\n",
            r#"{"type":"user/message","seq":8,"time":1111,"data":{"content":[{"type":"text","text":"hi dsh"}],"source":{"kind":"user"}}}"#,
            "\n",
            r#"{"type":"assistant/message","seq":9,"time":1112,"data":{"message":{"content":[{"type":"text","text":"done"}]}}}"#,
            "\n",
        );
        let frame = zstd::stream::encode_all(format!("{header}\n{events}").as_bytes(), 3).unwrap();
        let sess = dsh_root
            .join("sessions")
            .join("--proj--")
            .join("escaped~dir");
        std::fs::create_dir_all(&sess).unwrap();
        std::fs::write(sess.join("session.v3.jsonl.zstd"), &frame).unwrap();
    }

    /// tmp home 上的代际文件读取：header.id 匹配路由到会话；未知 id → Err。
    /// 默认根 = home/.dsh（home 为 dsh 数据根，与 dsh_home_with 的回落分支同形）
    #[test]
    fn dsh_reads_zstd_generation_by_header_id() {
        let tmp = tempfile::tempdir().unwrap();
        write_dsh_generation(&tmp.path().join(".dsh"), "session-abc");

        let msgs = read_session_messages_with(tmp.path(), "dsh", "session-abc", 200)
            .unwrap()
            .messages;
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "assistant"], "turn/start 不出条目");
        assert_eq!(msgs[0].content, "hi dsh");
        assert_eq!(msgs[0].ts, Some(1111));
        // 未知会话 → Err（404 语义）
        assert!(read_session_messages_with(tmp.path(), "dsh", "session-other", 200).is_err());
    }

    /// fix round 1 Important 1：$DSH_HOME 重定向必须与看板同源（M0 F14：env >
    /// ~/.dsh，生产扫描是 scan_sessions(&dsh_home())）。env 值以**参数**注入
    /// （测试绝不读真实进程 env，杜绝与并行 env 测试互踩）：
    /// Some(env 根) → 命中 env 根下会话；None（env 未设形态）→ 回落 home/.dsh
    #[test]
    fn dsh_env_home_redirects_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let env_dir = tempfile::tempdir().unwrap();
        // 会话数据在 env 根下；home 下无任何 dsh 数据（旧的「恒 ~/.dsh」实现在此必红）
        write_dsh_generation(env_dir.path(), "session-abc");
        let env = env_dir.path().to_str().unwrap().to_string();

        let msgs = read_dsh_messages_with(tmp.path(), Some(env.as_str()), "session-abc", 200)
            .unwrap()
            .messages;
        assert_eq!(msgs[0].content, "hi dsh", "env 根指向必须生效");
        // env 未设（None）→ 回落 home/.dsh → 找不到（数据只在 env 根下）
        assert!(read_dsh_messages_with(tmp.path(), None, "session-abc", 200).is_err());
    }

    // ==== Claude（合成 JSONL 行）====

    #[test]
    fn claude_maps_blocks_and_skips_meta() {
        let lines = vec![
            // 记账行：summary / file-history-snapshot → 跳过
            r#"{"type":"summary","summary":"标题","leafUuid":"x"}"#.to_string(),
            r#"{"type":"file-history-snapshot","messageId":"m"}"#.to_string(),
            // isMeta 注入行 → 跳过
            r#"{"type":"user","isMeta":true,"timestamp":"2026-09-15T00:00:00.000Z","message":{"role":"user","content":"<系统注入>"}}"#.to_string(),
            // 真实用户消息（content 字符串形态）
            r#"{"type":"user","timestamp":"2026-09-15T00:00:01.000Z","message":{"role":"user","content":"查一下状态"}}"#.to_string(),
            // assistant：thinking + tool_use + text
            r#"{"type":"assistant","timestamp":"2026-09-15T00:00:02.000Z","message":{"role":"assistant","content":[{"type":"thinking","thinking":"想一想"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}},{"type":"text","text":"好了"}]}}"#.to_string(),
            // user 侧 tool_result（块数组形态）
            r#"{"type":"user","timestamp":"2026-09-15T00:00:03.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"total 0"}]}}"#.to_string(),
        ];
        let msgs = map_claude_lines(&lines);
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["user", "thinking", "tool-call", "assistant", "tool-result"]
        );
        assert_eq!(msgs[0].content, "查一下状态");
        // timestamp 是 ISO 8601 → epoch 毫秒（期望值就地解析，不写魔法数字）
        assert_eq!(
            msgs[0].ts,
            chrono::DateTime::parse_from_rfc3339("2026-09-15T00:00:01.000Z")
                .ok()
                .map(|dt| dt.timestamp_millis())
        );
        assert_eq!(msgs[2].tool_name.as_deref(), Some("Bash"));
        assert!(msgs[2]
            .tool_args
            .as_deref()
            .unwrap_or_default()
            .contains("\"command\":\"ls\""));
        assert_eq!(msgs[4].content, "total 0");
    }

    /// Bug 1 修复（M3 验收）：胖 JSONL（>512KB）头部被字节尾窗切掉——内容层必须
    /// 报告 truncated，且放大 limit 时字节窗同放大（512KB×⌈limit/200⌉）、头部行回归。
    /// 这是「加载更早消息」死功能的根因之一：按钮出现后重拉拿不到更早内容
    #[test]
    fn claude_fat_file_reports_truncation_and_grows_window_with_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".claude/projects/-Users-x-demo");
        std::fs::create_dir_all(&dir).unwrap();
        // 头部小行（首条用户指令）+ 600KB 大行 + 尾行：512KB 窗必切进大行
        let big_text = "x".repeat(600 * 1024);
        let head = r#"{"type":"user","timestamp":"2026-09-15T00:00:01.000Z","message":{"role":"user","content":"首条指令"}}"#;
        let fat = format!(
            r#"{{"type":"assistant","timestamp":"2026-09-15T00:00:02.000Z","message":{{"role":"assistant","content":[{{"type":"text","text":"{big_text}"}}]}}}}"#
        );
        let tail = r#"{"type":"assistant","timestamp":"2026-09-15T00:00:03.000Z","message":{"role":"assistant","content":[{"type":"text","text":"尾行"}]}}"#;
        std::fs::write(
            dir.join(format!("{UUID}.jsonl")),
            format!("{head}\n{fat}\n{tail}\n"),
        )
        .unwrap();

        // limit=200：字节窗 512KB → truncated=true，头部行缺席（旧实现的静默截断）
        let page = read_session_messages_impl(tmp.path(), None, None, "claude", UUID, 200).unwrap();
        assert!(page.truncated, "超窗文件必须报告头部截断");
        assert!(
            !page.messages.iter().any(|m| m.content == "首条指令"),
            "窗外的头部行本页不可见"
        );
        // limit=400：字节窗放大到 1MB → 全文件进窗，头部行回归
        let page = read_session_messages_impl(tmp.path(), None, None, "claude", UUID, 400).unwrap();
        assert!(!page.truncated, "放大窗后不得再报告截断");
        assert_eq!(page.messages[0].content, "首条指令", "头部行必须可见");
    }

    #[test]
    fn claude_reads_file_by_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".claude/projects/-Users-x-demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{UUID}.jsonl")),
            r#"{"type":"user","timestamp":"2026-09-15T00:00:01.000Z","message":{"role":"user","content":"hi claude"}}"#,
        )
        .unwrap();
        let msgs = read_session_messages_with(tmp.path(), "claude", UUID, 200)
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi claude");
        assert!(read_session_messages_with(
            tmp.path(),
            "claude",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            200
        )
        .is_err());
    }

    // ==== Codex（rollout 合成行 + thread_history tmp sqlite）====

    #[test]
    fn codex_maps_rollout_lines() {
        let lines = vec![
            // session_meta / event_msg 记账 → 跳过
            r#"{"timestamp":"2026-09-06T05:40:24.228Z","type":"session_meta","payload":{"id":"x","cwd":"/w"}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:24.300Z","type":"event_msg","payload":{"type":"task_started","turn_id":"t1"}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:30.320Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"检查仓库"}]}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:34.579Z","type":"response_item","payload":{"type":"reasoning","summary":["先看 git 状态"]}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:34.952Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"我看一下"}]}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:34.953Z","type":"response_item","payload":{"type":"function_call","id":"fc_1","name":"exec_command","arguments":"{\"cmd\":\"git status\"}"}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:35.831Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":"ok"}}"#.to_string(),
            r#"{"timestamp":"2026-09-06T05:40:35.887Z","type":"event_msg","payload":{"type":"token_count","info":{}}}"#.to_string(),
        ];
        let msgs = map_codex_lines(&lines);
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["user", "thinking", "assistant", "tool-call", "tool-result"]
        );
        assert_eq!(msgs[0].content, "检查仓库");
        assert_eq!(msgs[1].content, "先看 git 状态");
        assert_eq!(msgs[3].tool_name.as_deref(), Some("exec_command"));
        assert_eq!(
            msgs[3].tool_args.as_deref(),
            Some(r#"{"cmd":"git status"}"#)
        );
        assert_eq!(msgs[4].content, "ok");
    }

    #[test]
    fn codex_reads_rollout_by_id_and_falls_back_to_thread_db() {
        let tmp = tempfile::tempdir().unwrap();
        // (a) rollout 快路径：文件名内嵌 session id
        let dir = tmp.path().join(".codex/sessions/2026/09/06");
        std::fs::create_dir_all(&dir).unwrap();
        let rollout = dir.join(format!("rollout-2026-09-06T13-40-16-{UUID}.jsonl"));
        std::fs::write(
            &rollout,
            format!(
                r#"{{"timestamp":"2026-09-06T05:40:24.228Z","type":"session_meta","payload":{{"id":"{UUID}","cwd":"/w"}}}}"#,
            ),
        )
        .unwrap();
        // rollout 只有 meta（无内容条目）→ 应继续落空但不炸
        let r = read_session_messages_with(tmp.path(), "codex", UUID, 200)
            .unwrap()
            .messages;
        assert!(r.is_empty(), "仅 meta 的 rollout 映射为空");

        // (b) APP 线路回退：thread_history sqlite（无 rollout 文件时）
        std::fs::remove_file(&rollout).unwrap();
        let hist = tmp.path().join(".codex/thread_history_1.sqlite");
        std::fs::create_dir_all(hist.parent().unwrap()).unwrap();
        let conn = Connection::open(&hist).unwrap();
        conn.execute_batch(
            "CREATE TABLE thread_items (
                thread_id TEXT, turn_id TEXT, item_id TEXT,
                rollout_ordinal INTEGER, created_at_ms INTEGER,
                item_json TEXT NOT NULL, item_type TEXT NOT NULL DEFAULT ''
             );",
        )
        .unwrap();
        let ins = |ord: i64, ms: i64, ty: &str, json: &str| {
            conn.execute(
                "INSERT INTO thread_items (thread_id, turn_id, item_id, rollout_ordinal, created_at_ms, item_json, item_type) VALUES (?1,'t',?2,?3,?4,?5,?6)",
                rusqlite::params![UUID, format!("i{ord}"), ord, ms, json, ty],
            )
            .unwrap();
        };
        ins(
            1,
            100,
            "userMessage",
            r#"{"type":"userMessage","content":[{"type":"text","text":"查状态"}]}"#,
        );
        ins(
            2,
            200,
            "reasoning",
            r#"{"type":"reasoning","summary":["想一想","再查"]}"#,
        );
        ins(
            3,
            300,
            "agentMessage",
            r#"{"type":"agentMessage","text":"一切正常"}"#,
        );
        ins(
            4,
            400,
            "commandExecution",
            r#"{"type":"commandExecution","command":"git status","aggregatedOutput":"clean"}"#,
        );
        ins(
            5,
            500,
            "fileChange",
            r#"{"type":"fileChange","changes":[{"path":"/w/a.rs","kind":{"type":"update"}}]}"#,
        );
        let msgs = read_session_messages_with(tmp.path(), "codex", UUID, 200)
            .unwrap()
            .messages;
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["user", "thinking", "assistant", "tool-call", "tool-call"]
        );
        assert_eq!(msgs[0].content, "查状态");
        assert_eq!(
            msgs[1].content, "想一想 再查",
            "reasoning.summary 字符串数组拼接"
        );
        assert_eq!(msgs[3].tool_name.as_deref(), Some("exec"));
        assert!(msgs[3]
            .tool_args
            .as_deref()
            .unwrap_or_default()
            .contains("git status"));
        assert!(msgs[4].content.contains("a.rs"), "fileChange 出路径摘要");
        // 未知会话 → Err
        assert!(read_session_messages_with(
            tmp.path(),
            "codex",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            200
        )
        .is_err());
    }

    // ==== Kimi（合成 wire 行 + tmp home session_index）====

    #[test]
    fn kimi_maps_wire_lines_with_merge_and_dedup() {
        let lines = vec![
            // 记账行 → 跳过
            r#"{"type":"metadata","protocol_version":"1.4"}"#.to_string(),
            r#"{"type":"config.update","config":{}}"#.to_string(),
            // 用户输入：turn.prompt 与 append_message(user) 双写 → 全等去重只留一条
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"帮我写个函数"}],"time":1000}"#.to_string(),
            r#"{"type":"context.append_message","message":{"role":"user","content":[{"type":"text","text":"帮我写个函数"}]},"time":1001}"#.to_string(),
            // 流式正文碎片：连续 text part 合并为一条（\n 连接）
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"text","text":"第一段"}},"time":1100}"#.to_string(),
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"text","text":"第二段"}},"time":1101}"#.to_string(),
            // think 块字段名是 think（非 text）
            r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"think","think":"推理中"}},"time":1200}"#.to_string(),
            // 工具调用（args 是对象）与结果（result.output）
            r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"c1","name":"Bash","args":{"command":"ls"}},"time":1300}"#.to_string(),
            r#"{"type":"context.append_loop_event","event":{"type":"tool.result","toolCallId":"c1","result":{"output":"a.rs"}},"time":1400}"#.to_string(),
            // 第二次用户输入：两条不同内容的 user 不合并（独立输入独立条目）
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"再改一下"}],"time":1600}"#.to_string(),
            // 轮尾记账 → 跳过
            r#"{"type":"turn.ended","reason":"completed","time":1700}"#.to_string(),
            r#"{"type":"usage.record","usage":{},"time":1701}"#.to_string(),
        ];
        let msgs = map_kimi_lines(&lines);
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "user",
                "assistant",
                "thinking",
                "tool-call",
                "tool-result",
                "user"
            ]
        );
        assert_eq!(msgs[0].content, "帮我写个函数", "双写去重只留一条");
        assert_eq!(msgs[1].content, "第一段\n第二段", "连续 text part 合并");
        assert_eq!(msgs[2].content, "推理中");
        assert_eq!(msgs[3].tool_name.as_deref(), Some("Bash"));
        assert!(msgs[3]
            .tool_args
            .as_deref()
            .unwrap_or_default()
            .contains("\"command\":\"ls\""));
        assert_eq!(msgs[4].content, "a.rs");
        assert_eq!(msgs[4].ts, Some(1400));
        assert_eq!(msgs[5].content, "再改一下", "不同内容的两次用户输入不合并");
    }

    #[test]
    fn kimi_reads_via_session_index() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(".kimi-code");
        let session_dir = home
            .join("sessions/wd_demo_0123456789ab")
            .join(format!("session_{UUID}"));
        std::fs::create_dir_all(session_dir.join("agents/main")).unwrap();
        std::fs::write(
            session_dir.join("agents/main/wire.jsonl"),
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi kimi"}],"time":1782300900000}"#,
        )
        .unwrap();
        let index_line = serde_json::json!({
            "sessionId": UUID,
            "sessionDir": session_dir.to_string_lossy(),
            "workDir": "/work/demo",
        })
        .to_string();
        std::fs::write(home.join("session_index.jsonl"), format!("{index_line}\n")).unwrap();

        let msgs = read_session_messages_with(tmp.path(), "kimi", UUID, 200)
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi kimi");
        // 索引外会话 → Err
        assert!(read_session_messages_with(
            tmp.path(),
            "kimi",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            200
        )
        .is_err());
    }

    /// fix round 1 Minor①：KIMI_CODE_HOME 重定向以**参数**注入（env 值作参，
    /// 测试不读真实进程 env——与 kimi_parser 的 run_with_home 互斥锁不再有交集）。
    /// Some(env 根) → 命中 env 根下的索引与会话；None → 回落 home/.kimi-code
    #[test]
    fn kimi_env_home_redirects_data_root() {
        let tmp = tempfile::tempdir().unwrap();
        let env_dir = tempfile::tempdir().unwrap();
        let session_dir = env_dir
            .path()
            .join("sessions/wd_demo_0123456789ab")
            .join(format!("session_{UUID}"));
        std::fs::create_dir_all(session_dir.join("agents/main")).unwrap();
        std::fs::write(
            session_dir.join("agents/main/wire.jsonl"),
            r#"{"type":"turn.prompt","input":[{"type":"text","text":"kimi in env"}],"time":1}"#,
        )
        .unwrap();
        let index_line = serde_json::json!({
            "sessionId": UUID,
            "sessionDir": session_dir.to_string_lossy(),
            "workDir": "/work/demo",
        })
        .to_string();
        std::fs::write(
            env_dir.path().join("session_index.jsonl"),
            format!("{index_line}\n"),
        )
        .unwrap();
        let env = env_dir.path().to_str().unwrap().to_string();

        let msgs = read_kimi_messages_with(tmp.path(), Some(env.as_str()), UUID, 200)
            .unwrap()
            .messages;
        assert_eq!(msgs[0].content, "kimi in env", "env 根指向必须生效");
        // env 未设（None）→ 回落 home/.kimi-code → 找不到
        assert!(read_kimi_messages_with(tmp.path(), None, UUID, 200).is_err());
    }

    // ==== WorkBuddy（合成 JSONL 行 + tmp home projects 扫描）====

    #[test]
    fn workbuddy_maps_lines() {
        let lines = vec![
            r#"{"type":"file-history-snapshot"}"#.to_string(),
            r#"{"type":"reasoning","providerData":{"messageId":"m1"}}"#.to_string(),
            r#"{"type":"message","role":"user","content":[{"type":"text","text":"跑个测试"}]}"#.to_string(),
            r#"{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"pnpm test\"}"}"#.to_string(),
            r#"{"type":"function_call_result","output":"all green"}"#.to_string(),
            r#"{"type":"message","role":"assistant","content":[{"type":"text","text":"测试通过"}]}"#.to_string(),
        ];
        let msgs = map_workbuddy_lines(&lines);
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "tool-call", "tool-result", "assistant"]);
        assert_eq!(msgs[0].content, "跑个测试");
        assert_eq!(msgs[1].tool_name.as_deref(), Some("shell"));
        assert_eq!(msgs[2].content, "all green");
        assert_eq!(msgs[3].content, "测试通过");
    }

    #[test]
    fn workbuddy_reads_by_projects_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".workbuddy/projects/Users-jarvis-demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{UUID}.jsonl")),
            r#"{"type":"message","role":"user","content":[{"type":"text","text":"hi wb"}]}"#,
        )
        .unwrap();
        let msgs = read_session_messages_with(tmp.path(), "workbuddy", UUID, 200)
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hi wb");
        assert!(read_session_messages_with(
            tmp.path(),
            "workbuddy",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            200
        )
        .is_err());
    }

    // ==== OpenCode（tmp sqlite）====

    #[test]
    fn opencode_maps_message_parts() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (message_id TEXT, data TEXT, time_created INTEGER);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m1', ?1, 100, '{\"role\":\"user\"}')",
            [SID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m1', '{\"type\":\"text\",\"text\":\"看下这个\"}', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m2', ?1, 200, '{\"role\":\"assistant\"}')",
            [SID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m2', '{\"type\":\"reasoning\",\"text\":\"推演\"}', 2)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m2', '{\"type\":\"tool\",\"tool\":\"bash\",\"state\":{\"input\":{\"cmd\":\"ls\"}}}', 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('m2', '{\"type\":\"text\",\"text\":\"结论\"}', 4)",
            [],
        )
        .unwrap();

        let msgs = read_session_messages_with(tmp.path(), "opencode", SID, 200)
            .unwrap()
            .messages;
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, vec!["user", "thinking", "tool-call", "assistant"]);
        assert_eq!(msgs[0].content, "看下这个");
        assert_eq!(msgs[2].tool_name.as_deref(), Some("bash"));
        assert!(msgs[2]
            .tool_args
            .as_deref()
            .unwrap_or_default()
            .contains("\"cmd\":\"ls\""));
        // 未知会话 → Err
        assert!(read_session_messages_with(tmp.path(), "opencode", "ses_other", 200).is_err());
    }

    // ==== OpenClaw（acp_replay tmp sqlite + 优雅降级）====

    fn openclaw_db(path: &Path) -> Connection {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        Connection::open(path).unwrap()
    }

    fn openclaw_event(conn: &Connection, session: &str, seq: i64, at: i64, update: &str) {
        conn.execute(
            "INSERT INTO acp_replay_events (session_id, seq, at, session_key, update_json) VALUES (?1,?2,?3,'k',?4)",
            rusqlite::params![session, seq, at, update],
        )
        .unwrap();
    }

    /// 优雅降级裁决的三态：无 ~/.openclaw、库在但表不在、表在但会话不在 →
    /// 一律 Err("OpenClaw 暂无本地消息历史可读")，不阻塞其余七工具
    #[test]
    fn openclaw_degrades_gracefully_without_history() {
        let tmp = tempfile::tempdir().unwrap();
        // (1) 目录不存在
        let r = read_session_messages_with(tmp.path(), "openclaw", "sess-x", 200);
        assert!(r.unwrap_err().contains("OpenClaw 暂无本地消息历史可读"));
        // (2) 库在但 acp_replay_events 表缺失（旧版存储布局）
        openclaw_db(&tmp.path().join(".openclaw/state/openclaw.sqlite"));
        let r = read_session_messages_with(tmp.path(), "openclaw", "sess-x", 200);
        assert!(r.unwrap_err().contains("OpenClaw 暂无本地消息历史可读"));
        // (3) 表在但会话无事件
        let conn = openclaw_db(&tmp.path().join(".openclaw/state/openclaw.sqlite"));
        conn.execute_batch(
            "CREATE TABLE acp_replay_events (
                session_id TEXT NOT NULL, seq INTEGER NOT NULL, at INTEGER NOT NULL,
                session_key TEXT NOT NULL, run_id TEXT, update_json TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
             );",
        )
        .unwrap();
        openclaw_event(
            &conn,
            "other-session",
            1,
            100,
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}"#,
        );
        let r = read_session_messages_with(tmp.path(), "openclaw", "sess-x", 200);
        assert!(r.unwrap_err().contains("OpenClaw 暂无本地消息历史可读"));
    }

    /// ACP sessionUpdate → 统一条目映射（2026-09-15 实机探测的 update_json 结构）
    #[test]
    fn openclaw_maps_acp_replay_events() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = openclaw_db(&tmp.path().join(".openclaw/state/openclaw.sqlite"));
        conn.execute_batch(
            "CREATE TABLE acp_replay_events (
                session_id TEXT NOT NULL, seq INTEGER NOT NULL, at INTEGER NOT NULL,
                session_key TEXT NOT NULL, run_id TEXT, update_json TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
             );",
        )
        .unwrap();
        openclaw_event(
            &conn,
            "s1",
            1,
            100,
            r#"{"sessionUpdate":"session_info_update","title":"t"}"#,
        );
        openclaw_event(
            &conn,
            "s1",
            2,
            200,
            r#"{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"帮我看看"}}"#,
        );
        openclaw_event(
            &conn,
            "s1",
            3,
            300,
            r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"思考中"}}"#,
        );
        openclaw_event(
            &conn,
            "s1",
            4,
            400,
            r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"Bash","rawInput":{"command":"ls"}}"#,
        );
        openclaw_event(
            &conn,
            "s1",
            5,
            500,
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"完成"}}"#,
        );

        let msgs = read_session_messages_with(tmp.path(), "openclaw", "s1", 200)
            .unwrap()
            .messages;
        let kinds: Vec<&str> = msgs.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["user", "thinking", "tool-call", "assistant"],
            "会话级元事件不出条目"
        );
        assert_eq!(msgs[0].content, "帮我看看");
        assert_eq!(msgs[0].ts, Some(200));
        assert_eq!(msgs[2].tool_name.as_deref(), Some("Bash"));
        assert!(msgs[2]
            .tool_args
            .as_deref()
            .unwrap_or_default()
            .contains("\"command\":\"ls\""));
        assert_eq!(msgs[3].content, "完成");
    }
}
