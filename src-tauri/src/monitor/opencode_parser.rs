// OpenCode 会话解析器 — 基于 SQLite 数据库（opencode.db）
// OpenCode 1.17+ 使用 SQLite 替代分散 JSON 文件，此模块查询数据库获取会话状态

use super::cwd::{cwd_equivalent, normalize_cwd_for_match};
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, Session, SessionStatus};
use log::{debug, info};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// message.data JSON 结构
#[derive(Deserialize)]
struct MessageData {
    role: Option<String>,
    error: Option<ErrorData>,
}

/// message.data.error JSON 结构（取证样本：`{"name":"APIError","data":{…}}`，
/// 本机 10 条：9×APIError 401 + 1×MessageAbortedError）
#[derive(Deserialize)]
struct ErrorData {
    name: Option<String>,
    data: Option<ErrorDataInner>,
}

/// error.data JSON 结构（API 错误详情等）
#[derive(Deserialize)]
struct ErrorDataInner {
    message: Option<String>,
}

/// 末条消息失败错误的摘要（§4.3 前置规则：状态判定 + 卡片消息行展示）
struct LastError {
    name: String,
    message: Option<String>,
}

/// part.data JSON 结构
#[derive(Deserialize)]
struct PartData {
    #[serde(rename = "type")]
    part_type: Option<String>,
    text: Option<String>,
}

/// 获取 OpenCode 会话（生产入口：DB 固定在 ~/.local/share/opencode/opencode.db）。
/// 扫描预算豁免说明（monitor::session_scan）：SQLite 查询即按 cwd 过滤、零进程已
/// 空判早退（L1），无"全量历史重扫"问题，故不接 L2/L3
pub fn get_opencode_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let Some(h) = dirs::home_dir() else {
        return Vec::new();
    };
    get_opencode_sessions_with_db(
        &h.join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db"),
        processes,
    )
}

/// DB 路径注入版（单测用）：与生产入口同逻辑
fn get_opencode_sessions_with_db(db_path: &Path, processes: &[AgentProcess]) -> Vec<Session> {
    if processes.is_empty() {
        return Vec::new();
    }

    if !db_path.exists() {
        debug!("OpenCode database not found: {:?}", db_path);
        return Vec::new();
    }

    // 只读连接 + busy_timeout（共享 helper，P1-4 消根：opencode/workbuddy 同规）
    let conn = match super::sqlite::open_readonly_with_timeout(db_path) {
        Some(c) => c,
        None => {
            debug!("Failed to open OpenCode database: {:?}", db_path);
            return Vec::new();
        }
    };

    // 归一化 cwd -> process 映射（统一分隔符、去尾部、Windows 下小写）
    let mut cwd_to_process: HashMap<String, &AgentProcess> = HashMap::new();
    for process in processes {
        if let Some(cwd) = &process.cwd {
            cwd_to_process.insert(normalize_cwd_for_match(&cwd.to_string_lossy()), process);
        }
    }

    // 最近会话行（主匹配数据源，归一化比较在 Rust 侧做）
    let recent: Vec<(String, String, Option<String>, i64)> = conn
        .prepare("SELECT id, directory, title, time_updated FROM session ORDER BY time_updated DESC LIMIT 200")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default();

    let mut sessions = Vec::new();
    let mut matched_pids: HashSet<u32> = HashSet::new();
    // 已被某进程认领的 session 行下标（同目录双开防遮蔽：recent 按 time_updated DESC，
    // 最新行配第一个进程、次新行配第二个——借鉴 kimi_parser Phase 1 的 matched 集合）
    let mut matched_rows: HashSet<usize> = HashSet::new();

    // ---- 主匹配：session.directory（会话启动目录）与进程 cwd 归一化相等 ----
    // （含 global 会话；取代原按 directory 精确 SQL 的 global 回退——SQL 精确匹配无法
    //   处理分隔符/大小写差异，归一化比较必须在 Rust 侧做）
    for process in processes {
        let Some(cwd) = &process.cwd else { continue };
        let cwd_str = cwd.to_string_lossy();
        if let Some((row_idx, row)) = recent
            .iter()
            .enumerate()
            .find(|(i, (_, dir, _, _))| !matched_rows.contains(i) && cwd_equivalent(dir, &cwd_str))
        {
            let (session_id, directory, title, time_updated) = row;
            if let Some(session) = build_session_from_row(
                &conn,
                session_id,
                directory,
                title.as_deref(),
                None,
                *time_updated,
                process,
            ) {
                sessions.push(session);
                // 构造成功才认领行与 pid（与 kimi Phase 1 同时序）：若将来构造可能
                // 过滤返回 None，失败时不烧掉行/pid，该进程仍可走回退匹配
                matched_rows.insert(row_idx);
                matched_pids.insert(process.pid);
            }
        }
    }

    // ---- 回退匹配：project worktree 前缀（进程 cwd 等于或在 worktree 之下）----
    let projects: Vec<(String, String, Option<String>)> = conn
        .prepare("SELECT id, worktree, name FROM project WHERE id != 'global'")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default();

    for (project_id, worktree, name) in &projects {
        let wt = normalize_cwd_for_match(worktree);
        let matching_process = cwd_to_process
            .iter()
            .find(|(cwd, proc)| {
                !matched_pids.contains(&proc.pid)
                    && (*cwd == wt.as_str() || cwd.starts_with(&format!("{}/", wt)))
            })
            .map(|(_, p)| *p);

        if let Some(process) = matching_process {
            debug!(
                "OpenCode project {} matched to pid={}",
                worktree, process.pid
            );
            matched_pids.insert(process.pid);
            if let Some(session) =
                get_latest_session_for_project(&conn, project_id, name.as_deref(), process)
            {
                sessions.push(session);
            }
        }
    }

    info!(
        "OpenCode: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// 获取项目的最新会话
fn get_latest_session_for_project(
    conn: &Connection,
    project_id: &str,
    project_name: Option<&str>,
    process: &AgentProcess,
) -> Option<Session> {
    let (session_id, directory, title, time_updated) = conn
        .prepare("SELECT id, directory, title, time_updated FROM session WHERE project_id = ? ORDER BY time_updated DESC LIMIT 1")
        .ok()?
        .query_row([project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .ok()?;

    build_session_from_row(
        conn,
        &session_id,
        &directory,
        // title 为 String（行内值），转 Option<&str> 传给共用构造
        Some(title.as_str()),
        project_name,
        time_updated,
        process,
    )
}

/// 由会话行构造 Session（主匹配与项目匹配共用）
fn build_session_from_row(
    conn: &Connection,
    session_id: &str,
    directory: &str,
    title: Option<&str>,
    project_name_override: Option<&str>,
    time_updated: i64,
    process: &AgentProcess,
) -> Option<Session> {
    let (last_role, last_message, last_error) = get_last_message_info(conn, session_id);
    let last_msg_time = get_last_message_time(conn, session_id);
    // 会话尾部部件信号（§4.3）：末条 part + 所属 role；仅 text/patch 尾按需查 step 部件。
    // user 消息的 part 也是 text 类型，但 tail_part_signal 对 role=user 首行即短路，
    // 先排除 user 尾再查 has_step，省掉最高频状态（输入刚提交）的每轮查询
    let tail = get_session_tail_part(conn, session_id)
        .map(|t| {
            let ptype = t
                .part
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or_default();
            let has_step = t.message_role.as_deref() != Some("user")
                && (ptype == "text" || ptype == "patch")
                && message_has_step_part(conn, &t.message_id);
            tail_part_signal(&t.part, t.message_role.as_deref(), has_step)
        })
        .unwrap_or(TailSignal::Fallback);

    // §4.3 前置规则优先：末条消息失败/中止判定高于尾部部件信号
    let status = failed_request_status(last_error.as_ref().map(|e| e.name.as_str()))
        .unwrap_or_else(|| {
            determine_opencode_status(
                process.cpu_usage,
                last_role.as_deref(),
                last_msg_time,
                time_updated,
                tail,
            )
        });
    let last_activity_at = ms_to_iso(time_updated);

    let title = title.unwrap_or("").to_string();
    let project_name = project_name_override
        .map(String::from)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            directory
                .rsplit(['/', '\\'])
                .find(|s| !s.is_empty())
                .unwrap_or("Unknown")
                .to_string()
        });
    let display_message = match &last_error {
        // 失败请求：错误摘要进消息行（与完成绿/中止一眼可辨，spec §4.3）；
        // 主动中止不标注（不提示裁决，维持原展示）
        Some(e) if e.name != "MessageAbortedError" => Some(format!(
            "❌ {}: {}",
            if e.name.is_empty() { "Error" } else { &e.name },
            e.message.as_deref().unwrap_or("")
        )),
        _ => last_message.or_else(|| {
            if !title.is_empty() {
                Some(title.clone())
            } else {
                None
            }
        }),
    };

    Some(Session {
        id: session_id.to_string(),
        agent_type: AgentType::OpenCode,
        project_name,
        project_path: directory.to_string(),
        git_branch: None,
        github_url: None,
        status,
        last_message: display_message,
        last_message_role: last_role,
        last_activity_at,
        pid: process.pid,
        cpu_usage: process.cpu_usage,
        active_subagent_count: 0,
        form: process.form,
        jump_supported: jump_supported_for(process.form),
        unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        title: Some(title),
    })
}

/// 获取最后一条消息的时间戳（毫秒）
fn get_last_message_time(conn: &Connection, session_id: &str) -> i64 {
    conn.query_row(
        "SELECT time_created FROM message WHERE session_id = ? ORDER BY time_created DESC, id DESC LIMIT 1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
}

/// 获取会话最后一条消息的角色、文本与失败错误摘要（错误供 §4.3 前置规则）
fn get_last_message_info(
    conn: &Connection,
    session_id: &str,
) -> (Option<String>, Option<String>, Option<LastError>) {
    // 查最后一条消息（id 为 ULID，作同毫秒平局破缺键）
    let mut stmt = match conn.prepare(
        "SELECT id, data FROM message WHERE session_id = ? ORDER BY time_created DESC, id DESC LIMIT 1",
    ) {
        Ok(s) => s,
        Err(_) => return (None, None, None),
    };

    let result = stmt.query_row([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });

    let (message_id, data_json) = match result {
        Ok(r) => r,
        Err(_) => return (None, None, None),
    };

    // 解析 message.data JSON 获取 role 与 error 摘要
    let data = serde_json::from_str::<MessageData>(&data_json).ok();
    let role = data.as_ref().and_then(|d| d.role.clone());
    let error = data.and_then(|d| d.error).map(|e| LastError {
        name: e.name.unwrap_or_default(),
        message: e.data.and_then(|inner| inner.message),
    });

    // 查该消息的 text 类型 parts
    let text = get_message_text(conn, &message_id);

    (role, text, error)
}

/// 获取消息的文本内容（优先 text 类型，其次 reasoning）
fn get_message_text(conn: &Connection, message_id: &str) -> Option<String> {
    let mut stmt = conn
        .prepare("SELECT data FROM part WHERE message_id = ? ORDER BY time_created ASC, id ASC")
        .ok()?;

    let mut text_content: Option<String> = None;
    let mut reasoning_content: Option<String> = None;

    let rows = stmt.query_map([message_id], |row| row.get::<_, String>(0));
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            if let Ok(part) = serde_json::from_str::<PartData>(&row) {
                match part.part_type.as_deref() {
                    Some("text") if text_content.is_none() => {
                        text_content = part.text;
                    }
                    Some("reasoning") if reasoning_content.is_none() => {
                        reasoning_content = part.text;
                    }
                    _ => {}
                }
            }
        }
    }

    let content = text_content.or(reasoning_content)?;

    // 跳过系统提示（XML 格式）
    let trimmed = content.trim();
    if trimmed.starts_with('<') && (trimmed.contains("ultrawork") || trimmed.contains("mode>")) {
        return None;
    }

    // 截断过长的消息
    if content.chars().count() > 100 {
        Some(format!(
            "{}...",
            content.chars().take(100).collect::<String>()
        ))
    } else {
        Some(content)
    }
}

/// 会话末条部件（跨消息，按落盘时间倒序取 1）+ 所属消息 role（spec §4.3）。
/// part 表自带 session_id 列，无需绕消息表过滤；role 经 LEFT JOIN message 取
struct SessionTailPart {
    message_id: String,
    message_role: Option<String>,
    part: serde_json::Value,
}

fn get_session_tail_part(conn: &Connection, session_id: &str) -> Option<SessionTailPart> {
    let row = conn
        .query_row(
            "SELECT p.message_id, p.data, m.data FROM part p \
             LEFT JOIN message m ON m.id = p.message_id \
             WHERE p.session_id = ?1 ORDER BY p.time_created DESC, p.id DESC LIMIT 1",
            [session_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .ok()?;
    let (message_id, part_data, message_data) = row;
    let part = serde_json::from_str(&part_data).ok()?;
    let message_role = message_data.and_then(|d| {
        serde_json::from_str::<MessageData>(&d)
            .ok()
            .and_then(|m| m.role)
    });
    Some(SessionTailPart {
        message_id,
        message_role,
        part,
    })
}

/// 消息是否含 step 部件（新格式标识；仅尾部为 text/patch 时按需调用）。
/// 沿模块既有模式取原始 data 在 Rust 侧解析（不依赖 SQLite JSON1 扩展）
fn message_has_step_part(conn: &Connection, message_id: &str) -> bool {
    let Ok(mut stmt) = conn.prepare("SELECT data FROM part WHERE message_id = ?1") else {
        return false;
    };
    let Ok(rows) = stmt.query_map([message_id], |r| r.get::<_, String>(0)) else {
        return false;
    };
    for row in rows.flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&row) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("step-start") | Some("step-finish") => return true,
            _ => {}
        }
    }
    false
}

/// 会话尾部部件信号（spec 假绿治理 §4.3，2026-09-16）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TailSignal {
    /// step-finish(reason=stop)：回合结束 → 走既有 last_role+60s 启发式
    ///（60s 窗内 Waiting 红 → Idle 绿；用户验证该收尾转换为正常语义，保留）
    TurnDone,
    /// 步骤进行中 / 用户输入刚提交 / step-finish(reason≠stop) → Processing 黄
    Running,
    /// **用户输入类工具待决**（丁T1，2026-09-21）→ Waiting 红：终端正显示问答 UI
    /// 等用户作答。判据见 [`pending_question_part`]（状态 pending/running 且无
    /// answers）——**语义红**（有明确证据），与「启发式假红」完全不同轴
    WaitingForUser,
    /// 无部件或老格式无 step 信号（team-mode text/patch）→ 回退既有启发式
    Fallback,
}

/// opencode 的用户输入类工具名（丁T1）：调用产物就是用户输入。
/// 来源 = 2026-09-21 本机 `~/.local/share/opencode/opencode.db` part 表全表扫描：
/// `tool` 取值里 `question` 是唯一的问答工具（其余 bash/read/edit/write/glob/grep/
/// skill/task/todowrite 均无 questions 入参形态）。**新增名字必须带实测证据**
const USER_INPUT_TOOL_NAME: &str = "question";

/// 待决 question part 判定（丁T1，纯函数，可测）：
/// 尾部 part 是 question 工具的调用且**尚未作答** → true。
///
/// 判据三层（全部来自 2026-09-21 本机 part 表实证，13 条 question part 全表）：
/// 1. **是不是 question**：`type=="tool"` 且（`tool=="question"` ∨ `state.input`
///    含非空 `questions[]` 形态）——名字是权威、形态是加强面（工具改名也接住），
///    与 codex 侧同款口径；
/// 2. **在不在等**：`state.status` ∈ {`pending`, `running`}——事件日志实证
///    pending 13 次 / running 13 次（同一调用的两次事件）；`completed` 与 `error`
///    分别是已答与被拒/中断，**不触发**；
/// 3. **有没有答过**：`state.metadata` 无 `answers` 键——completed 的 metadata
///    实证形态 `{"answers":[["out"]],"truncated":false}`，pending/running 无 metadata。
///    第三层与第二层冗余是有意的：未来若某版本在 pending 期间预写 metadata，
///    有 answers 即视为已答（宁漏不误报）。
fn pending_question_part(part: &serde_json::Value) -> bool {
    if part.get("type").and_then(|t| t.as_str()) != Some("tool") {
        return false;
    }
    let state = part.get("state").unwrap_or(&serde_json::Value::Null);
    let by_name = part.get("tool").and_then(|t| t.as_str()) == Some(USER_INPUT_TOOL_NAME);
    let by_shape = state
        .pointer("/input/questions")
        .and_then(|q| q.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if !(by_name || by_shape) {
        return false;
    }
    if !matches!(
        state.get("status").and_then(|s| s.as_str()),
        Some("pending") | Some("running")
    ) {
        return false;
    }
    // 已答（metadata.answers 在场）→ 不触发；metadata 缺失/null 视为未答
    let answered = state
        .get("metadata")
        .and_then(|m| m.get("answers"))
        .is_some_and(|a| !a.is_null());
    !answered
}

/// 尾部部件 → 信号（纯函数）。词汇表活体取证 2026-09-16（opencode v1.18.22）：
/// step-start / reasoning / tool / step-finish(reason=tool-calls|stop)。
/// 注意 reason=length 归 Running（宁黄不假绿），与 ZCode part_entry_kind 的
/// length→TurnEnd 语义相反，故不共享其映射（spec §8 决策 7）。
///
/// 丁T1 插入点：待决 question part → [`TailSignal::WaitingForUser`]（红灯）；
/// **其余 tool 部件语义零变化**（仍 Running 黄）——顺序放在下方 `"step-start" |
/// "reasoning" | "tool"` 臂**之前**，因为 question 也走 `"tool"` 臂
fn tail_part_signal(
    part: &serde_json::Value,
    message_role: Option<&str>,
    message_has_step: bool,
) -> TailSignal {
    // 用户消息的部件（任意类型）= 输入刚提交——含 assistant 占位行空窗
    //（opencode 按回车后 ~130ms 即写空 assistant 行，末条 part 仍属 user 消息）
    if message_role == Some("user") {
        return TailSignal::Running;
    }
    // 丁T1：待决的用户输入类工具调用（question UI 开着，等用户作答）→ 语义红。
    // 已答 / 被拒（error）的 question part 落回下方既有臂（tool → Running 黄），
    // 与「红=失败/批准专用」的既有裁决不冲突：那条针对**启发式**假红（正常完成
    // 不走红），本条是**证据红**（用户确实被问住了）
    if pending_question_part(part) {
        return TailSignal::WaitingForUser;
    }
    let ptype = part
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or_default();
    match ptype {
        "step-finish" => {
            if part.get("reason").and_then(|r| r.as_str()) == Some("stop") {
                TailSignal::TurnDone
            } else {
                TailSignal::Running // tool-calls / length 等：后续还有动作
            }
        }
        "step-start" | "reasoning" | "tool" => TailSignal::Running,
        // assistant 的 text/patch：消息含 step 部件 → 新格式流式窗口进行中；
        // 无 step 部件 → team-mode 老格式无信号，回退启发式（老会话零回归）
        "text" | "patch" => {
            if message_has_step {
                TailSignal::Running
            } else {
                TailSignal::Fallback
            }
        }
        _ => TailSignal::Fallback,
    }
}

/// 末条消息失败判定（spec §4.3 前置规则，2026-09-17 用户裁决）：末条消息
/// `data.error` 非空 → Some(终态)，优先于尾部部件信号——失败请求的 error 落在
/// 0 part 的空 assistant 占位行上、尾部 part 停留在 user 文本，走部件规则会
/// 误判「输入刚提交」永久黄。`MessageAbortedError`（用户主动 Esc）→ Idle 绿
/// （主动终止是用户已知事实，不提示）；其余 error（含 name 缺失）→ Waiting 红
/// （需要介入，绿→红边沿为正确报警）
fn failed_request_status(error_name: Option<&str>) -> Option<SessionStatus> {
    match error_name {
        None => None,
        Some("MessageAbortedError") => Some(SessionStatus::Idle),
        Some(_) => Some(SessionStatus::Waiting),
    }
}

/// OpenCode 状态判断：tail=Running（会话尾部部件强信号——步骤进行中/用户输入/
/// step-finish(reason≠stop)）→ 直接 Processing；tail=WaitingForUser（丁T1：待决
/// question 部件）→ 直接 Waiting；否则——非 assistant 回复完且 CPU > 15% →
/// Processing，user 尾近期活跃 → Processing，其余（含 assistant 回复完）→ Idle
/// 完成即绿（2026-09-17 用户裁决：红=失败/批准专用，失败红由
/// failed_request_status 前置规则给出，本函数不再产出**启发式** Waiting）。
///
/// **丁T1 与该裁决的关系（写清防误读）**：那条裁决否定的是「正常完成/停更猜等待」
/// 这类**启发式**假红（旧实现 60s 窗内判 Waiting，用户验证是噪声）；本次新增的
/// Waiting 是**语义红**——尾部存在待决的 question 部件（用户输入类工具调用没被
/// 回答，问答 UI 就开在终端上），有明确证据，与「运行中不误红灯」不矛盾：
/// 已答（metadata.answers）/ 被拒（state.status=error）的 question 都在
/// [`pending_question_part`] 处被排除，落回既有语义
fn determine_opencode_status(
    cpu: f32,
    last_role: Option<&str>,
    last_msg_time: i64,
    session_updated: i64,
    tail: TailSignal,
) -> SessionStatus {
    // 尾部部件强信号（spec 假绿治理 §4.3）：步骤进行中/用户输入 → 黄灯，短路既有
    // 启发式（修「输入瞬间绿→红假语音」「运行全程红」「单步>60s 假绿」三症状）
    if tail == TailSignal::Running {
        return SessionStatus::Processing;
    }
    // 丁T1：待决问答 = 语义红（终端在等用户作答）——短路既有启发式（否则 60s 后
    // 落 Idle 绿、问答卡挂不上去）
    if tail == TailSignal::WaitingForUser {
        return SessionStatus::Waiting;
    }
    // CPU 为瞬时采样噪声大：仅当会话不是"assistant 已回复完"且 CPU 明显高（阈值提高至 15%）
    // 才升级为 Processing，避免任务结束后后台活动（GC/索引）导致绿黄横跳
    if cpu > 15.0 && last_role != Some("assistant") {
        SessionStatus::Processing
    } else {
        // 检查是否近期活跃（最后消息时间或会话更新时间在 60s 内）
        let now = chrono::Utc::now().timestamp_millis();
        let last_active = last_msg_time.max(session_updated);
        let is_recent = now - last_active < 60_000; // 60 秒内
                                                    // user 尾 + 近期活跃 = 输入刚提交 → 黄；其余（含 assistant 回复完）→ Idle
                                                    // 完成即绿（2026-09-17 用户裁决）：不再有 60s Waiting 红窗
        if last_role == Some("user") && is_recent {
            SessionStatus::Processing
        } else {
            SessionStatus::Idle
        }
    }
}

/// 毫秒时间戳 → ISO 8601 字符串
fn ms_to_iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp(ms / 1000, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn cpu_spike_after_assistant_reply_is_not_processing() {
        // assistant 已回复完：CPU 抖动（后台 GC/索引）不得把状态拉回 Processing；
        // 完成即绿（2026-09-17 裁决）：新鲜窗口内高 CPU 也不回黄/红
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            determine_opencode_status(50.0, Some("assistant"), now, now, TailSignal::Fallback),
            SessionStatus::Idle
        );
        let old = now - 61_000;
        assert_eq!(
            determine_opencode_status(50.0, Some("assistant"), old, old, TailSignal::Fallback),
            SessionStatus::Idle
        );
    }

    #[test]
    fn cpu_still_marks_processing_before_reply() {
        // 尚未回复完（最后消息是 user）：高 CPU 正常判 Processing
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            determine_opencode_status(50.0, Some("user"), now, now, TailSignal::Fallback),
            SessionStatus::Processing
        );
    }
}

#[cfg(test)]
mod tail_signal_tests {
    use super::*;

    fn part(ptype: &str, reason: Option<&str>) -> serde_json::Value {
        let mut v = serde_json::json!({ "type": ptype });
        if let Some(r) = reason {
            v["reason"] = serde_json::json!(r);
        }
        v
    }

    /// §4.3 判定表（词汇表活体取证 2026-09-16，opencode v1.18.22）
    #[test]
    fn tail_part_signal_rules() {
        // step-finish(stop) → 回合结束信号（TurnDone → 完成即绿，2026-09-17 裁决）
        assert_eq!(
            tail_part_signal(&part("step-finish", Some("stop")), Some("assistant"), false),
            TailSignal::TurnDone
        );
        // step-finish(tool-calls / length) → 后续还有动作（length 宁黄不假绿，spec §8 决策 7）
        assert_eq!(
            tail_part_signal(
                &part("step-finish", Some("tool-calls")),
                Some("assistant"),
                false
            ),
            TailSignal::Running
        );
        assert_eq!(
            tail_part_signal(
                &part("step-finish", Some("length")),
                Some("assistant"),
                false
            ),
            TailSignal::Running
        );
        // 步骤进行中部件
        assert_eq!(
            tail_part_signal(&part("step-start", None), Some("assistant"), false),
            TailSignal::Running
        );
        assert_eq!(
            tail_part_signal(&part("reasoning", None), Some("assistant"), false),
            TailSignal::Running
        );
        assert_eq!(
            tail_part_signal(&part("tool", None), Some("assistant"), false),
            TailSignal::Running
        );
        // 用户消息的部件（任意类型）= 输入刚提交（含 assistant 占位行空窗）→ Running（修输入瞬间假红）
        assert_eq!(
            tail_part_signal(&part("text", None), Some("user"), false),
            TailSignal::Running
        );
        // assistant text/patch：新格式（消息含 step 部件）流式窗口 → Running；
        // 老格式 team-mode（无 step 部件）→ Fallback 回退启发式（老会话零回归）
        assert_eq!(
            tail_part_signal(&part("text", None), Some("assistant"), true),
            TailSignal::Running
        );
        assert_eq!(
            tail_part_signal(&part("text", None), Some("assistant"), false),
            TailSignal::Fallback
        );
        assert_eq!(
            tail_part_signal(&part("patch", None), Some("assistant"), false),
            TailSignal::Fallback
        );
        // 未知类型 / 无 role 信息 → Fallback
        assert_eq!(
            tail_part_signal(&part("file", None), Some("assistant"), false),
            TailSignal::Fallback
        );
        assert_eq!(
            tail_part_signal(&part("text", None), None, false),
            TailSignal::Fallback
        );
    }

    /// Running 强信号短路既有启发式：即便 last_role=assistant 且新鲜（旧逻辑判 Waiting 红），
    /// 也返回 Processing（修「输入瞬间绿→红假语音」与「运行全程红」）；
    /// 时间戳超窗（单步 >60s 无新消息，旧逻辑落 Idle 假绿）同样短路（症状③回归锁，spec §4.3）
    #[test]
    fn running_signal_short_circuits_to_processing() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            determine_opencode_status(0.0, Some("assistant"), now, now, TailSignal::Running),
            crate::session::SessionStatus::Processing
        );
        let old = now - 61_000;
        assert_eq!(
            determine_opencode_status(0.0, Some("assistant"), old, old, TailSignal::Running),
            crate::session::SessionStatus::Processing
        );
    }

    /// 末条消息失败判定（§4.3 前置规则，2026-09-17 用户裁决）：
    /// APIError → 红（需介入）；主动中止（Esc）→ 绿（用户已知，不提示）；
    /// 无 error → None 走尾部部件规则；未知 error 一律红（宁红不漏报）
    #[test]
    fn failed_request_status_rules() {
        assert_eq!(
            failed_request_status(Some("APIError")),
            Some(crate::session::SessionStatus::Waiting)
        );
        assert_eq!(
            failed_request_status(Some("MessageAbortedError")),
            Some(crate::session::SessionStatus::Idle)
        );
        assert_eq!(
            failed_request_status(Some("WhateverError")),
            Some(crate::session::SessionStatus::Waiting)
        );
        assert_eq!(failed_request_status(None), None);
    }

    /// 尾部件查询平局破缺：同毫秒两条 part 按 id 倒序取——id 为 ULID（字典序=时间序，
    /// opencode 自身索引同以 id 为顺序键）；无破缺时平局胜者依查询计划而定，
    /// text 与 step-finish(stop) 信号互异（Running vs TurnDone）会翻转状态
    #[test]
    fn tail_part_tie_breaks_by_id_desc() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m1','s1',100,'{\"role\":\"assistant\"}')",
            [],
        )
        .unwrap();
        // 同毫秒两条，插入序与 id 序相悖（先插 id 大者）：应稳定取 id 大的 step-finish(stop)
        conn.execute(
            "INSERT INTO part VALUES ('prt_0AA2','m1','s1',300,'{\"type\":\"step-finish\",\"reason\":\"stop\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('prt_0AA1','m1','s1',300,'{\"type\":\"text\",\"text\":\"hi\"}')",
            [],
        )
        .unwrap();
        let tail = get_session_tail_part(&conn, "s1").unwrap();
        assert_eq!(
            tail.part.get("type").and_then(|t| t.as_str()),
            Some("step-finish")
        );
    }

    /// TurnDone / Fallback：完成即绿（2026-09-17 用户裁决——正常完成不走红，
    /// 红=失败/批准专用；新鲜窗口内也不再 Waiting），超窗 Idle 不变
    #[test]
    fn turn_done_and_fallback_complete_green_immediately() {
        let now = chrono::Utc::now().timestamp_millis();
        for tail in [TailSignal::TurnDone, TailSignal::Fallback] {
            assert_eq!(
                determine_opencode_status(0.0, Some("assistant"), now, now, tail),
                crate::session::SessionStatus::Idle,
                "完成即绿：新鲜窗口内也不走红"
            );
            let old = now - 61_000;
            assert_eq!(
                determine_opencode_status(0.0, Some("assistant"), old, old, tail),
                crate::session::SessionStatus::Idle
            );
        }
    }

    // ==== 丁T1 · 待决 question 部件 → 语义红 ====

    /// 真实夹具（2026-09-21 本机 part 表实测形态）：question 工具调用，state.status
    /// ∈ {pending, running}（事件日志实证各 13 次），无 metadata。input.questions
    /// 为结构化题目数组
    fn question_part(status: &str, metadata: Option<serde_json::Value>) -> serde_json::Value {
        let mut state = serde_json::json!({
            "status": status,
            "input": {"questions": [{
                "question": "Which folder should hold build output?",
                "header": "Build output folder",
                "options": [
                    {"label": "dist", "description": "Place build output in the dist folder"},
                    {"label": "out", "description": "Place build output in the out folder"}
                ]
            }]}
        });
        if let Some(m) = metadata {
            state["metadata"] = m;
        }
        serde_json::json!({
            "type": "tool",
            "callID": "call_00_wYIBa5tx0EDcP9ttqb1W0316",
            "tool": "question",
            "state": state
        })
    }

    /// 待决主判据（丁T1）：pending / running 的 question part → WaitingForUser；
    /// 已答（completed + metadata.answers）与被拒（error）→ 不触发（落回既有语义）
    #[test]
    fn pending_question_part_detection() {
        let answered = serde_json::json!({"answers": [["out"]], "truncated": false});
        for status in ["pending", "running"] {
            assert!(
                pending_question_part(&question_part(status, None)),
                "{status} 的 question part 必须判待决"
            );
        }
        // 已答：completed + metadata.answers（真实形态，本机 9 次）
        assert!(
            !pending_question_part(&question_part("completed", Some(answered))),
            "有 answers → 不触发"
        );
        // 被拒 / 中断：error（本机 4 次，error 字段形如 "The user dismissed this question"）
        assert!(
            !pending_question_part(&question_part("error", None)),
            "error 状态落回既有语义（不抢 failed_request_status 的判定）"
        );
        // completed 但 metadata 缺失（防御）：仍视为已答（状态层排除）
        assert!(!pending_question_part(&question_part("completed", None)));
    }

    /// 非 question 的 tool 部件 → 不触发（零回归）：既有 tool 部件仍走 Running 黄
    #[test]
    fn non_question_tool_part_is_not_pending() {
        let bash_tool = serde_json::json!({
            "type": "tool",
            "tool": "bash",
            "state": {"status": "running", "input": {"command": "ls"}}
        });
        assert!(!pending_question_part(&bash_tool));
        assert_eq!(
            tail_part_signal(&bash_tool, Some("assistant"), false),
            TailSignal::Running,
            "普通工具调用尾部仍是运行中（零回归）"
        );
        // 非 tool 类型的 part 也不触发
        assert!(!pending_question_part(&serde_json::json!({"type": "text"})));
    }

    /// 形态加强面：工具改名但 `state.input.questions[]` 形态在场 → 同样接住
    ///（与 codex 侧同款口径；风险边界见 codex_parser::arguments_have_questions_shape）
    #[test]
    fn question_shape_with_other_tool_name_also_pending() {
        let renamed = question_part("running", None);
        let mut v = renamed.clone();
        v["tool"] = serde_json::json!("question_v2");
        assert!(
            pending_question_part(&v),
            "问答形态加强面：工具改名不影响接住"
        );
        // 形态不存在（无 questions / 空数组）→ 不触发
        let mut empty = renamed.clone();
        empty["state"]["input"] = serde_json::json!({"questions": []});
        empty["tool"] = serde_json::json!("other");
        assert!(!pending_question_part(&empty));
        let mut none = renamed;
        none["state"]["input"] = serde_json::json!({"command": "ls"});
        none["tool"] = serde_json::json!("other");
        assert!(!pending_question_part(&none));
    }

    /// 待决部件 → 尾部信号 WaitingForUser；已答/被拒 → 落回原有 Running 黄
    #[test]
    fn tail_part_signal_routes_pending_question_to_waiting() {
        assert_eq!(
            tail_part_signal(&question_part("running", None), Some("assistant"), false),
            TailSignal::WaitingForUser
        );
        let answered = serde_json::json!({"answers": [["out"]], "truncated": false});
        assert_eq!(
            tail_part_signal(
                &question_part("completed", Some(answered)),
                Some("assistant"),
                false
            ),
            TailSignal::Running,
            "已答的 question 是普通工具调用（黄，零回归）"
        );
        assert_eq!(
            tail_part_signal(&question_part("error", None), Some("assistant"), false),
            TailSignal::Running,
            "被拒/中断的 question 落回既有 tool 语义"
        );
    }

    /// WaitingForUser 短路启发式 → Waiting 红（与「完成即绿」裁决不冲突：
    /// 那条针对启发式假红，本条是证据红）
    #[test]
    fn waiting_for_user_signal_short_circuits_to_waiting() {
        let now = chrono::Utc::now().timestamp_millis();
        assert_eq!(
            determine_opencode_status(0.0, Some("assistant"), now, now, TailSignal::WaitingForUser),
            crate::session::SessionStatus::Waiting
        );
        // 超窗（1h 无新消息）同样红灯——问答 UI 开着与时间无关
        let old = now - 3_600_000;
        assert_eq!(
            determine_opencode_status(0.0, Some("assistant"), old, old, TailSignal::WaitingForUser),
            crate::session::SessionStatus::Waiting
        );
    }

    /// 既有短路序不打架（丁T1）：末条消息 `data.error`（failed_request_status）
    /// 优先于尾部部件信号——question 的 error 落在 **part 的 state**（非
    /// message.data.error），二者互不遮蔽。本测试锁住两条路径各自的结果
    #[test]
    fn failed_request_precedence_does_not_conflict_with_question_state_error() {
        // message.data.error 在场 → 前置规则胜（question 的 state.error 不在 message 上）
        assert_eq!(
            failed_request_status(Some("APIError")),
            Some(crate::session::SessionStatus::Waiting)
        );
        // question 的 state.error（被拒）不产 message error → 前置规则返回 None，
        // 状态由 tail signal 决定（被拒的 question 落 Running 黄）
        assert_eq!(failed_request_status(None), None);
        assert_eq!(
            tail_part_signal(&question_part("error", None), Some("assistant"), false),
            TailSignal::Running
        );
    }

    /// 尾部件查询：跨消息取会话末条 part + 所属 role（占位行空窗场景——末条 part 属 user 消息）
    #[test]
    fn session_tail_part_crosses_messages() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m1','s1',100,'{\"role\":\"user\"}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('m2','s1',200,'{\"role\":\"assistant\"}')",
            [],
        )
        .unwrap();
        // 占位行 m2 无部件；末条 part 属 m1（user）
        conn.execute(
            "INSERT INTO part VALUES ('prt_001','m1','s1',101,'{\"type\":\"text\",\"text\":\"列一下目录\"}')",
            [],
        )
        .unwrap();
        let tail = get_session_tail_part(&conn, "s1").unwrap();
        assert_eq!(tail.message_role.as_deref(), Some("user"));
        assert_eq!(tail.part.get("type").and_then(|t| t.as_str()), Some("text"));
        // m2 无 step 部件
        assert!(!message_has_step_part(&conn, "m2"));
        // step 部件探测
        conn.execute(
            "INSERT INTO part VALUES ('prt_002','m2','s1',300,'{\"type\":\"step-finish\",\"reason\":\"stop\"}')",
            [],
        )
        .unwrap();
        assert!(message_has_step_part(&conn, "m2"));
        // 末条 part 现属 m2 → role assistant
        let tail2 = get_session_tail_part(&conn, "s1").unwrap();
        assert_eq!(tail2.message_role.as_deref(), Some("assistant"));
    }
}

#[cfg(test)]
mod matching_tests {
    use super::*;
    use crate::session::ProcessForm;

    fn fake_process(pid: u32, cwd: &str) -> AgentProcess {
        AgentProcess {
            pid,
            cpu_usage: 0.0,
            cwd: Some(std::path::PathBuf::from(cwd)),
            form: ProcessForm::Cli,
            exe: None,
        }
    }

    /// 最小 schema 夹具 DB：仅建解析器 SQL 引用的表/列；message/part 允许为空。
    /// project 表留空（session 行硬编码 project_id='p1' 但不插 project 行）= project 回退段不触发
    fn fixture_db(path: &std::path::Path, sessions: &[(&str, &str, &str, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, title TEXT, time_updated INTEGER);
             CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT, name TEXT);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (id TEXT, message_id TEXT, session_id TEXT, data TEXT, time_created INTEGER);",
        )
        .unwrap();
        for (id, dir, title, ts) in sessions {
            conn.execute(
                "INSERT INTO session (id, project_id, directory, title, time_updated) VALUES (?1,'p1',?2,?3,?4)",
                rusqlite::params![id, dir, title, ts],
            )
            .unwrap();
        }
    }

    /// §4.3 前置规则端到端（取证形态 ses_f5574c39：末条消息 = 0 part 空 assistant
    /// 占位行且带 data.error，尾部 part 停留在 user 文本 → 部件规则判 Running 黄；
    /// error 判定须优先）：APIError → Waiting 红；MessageAbortedError（主动 Esc）→ Idle 绿
    #[test]
    fn failed_request_red_and_abort_green_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(
            &db,
            &[
                ("ses_fail", "C:/Users/x/F", "失败会话", 2000),
                ("ses_abort", "C:/Users/x/G", "中止会话", 2000),
            ],
        );
        let conn = Connection::open(&db).unwrap();
        let err_api = r#"{"name":"APIError","data":{"message":"Invalid API key."}}"#;
        let err_abort = r#"{"name":"MessageAbortedError","data":{"message":"Aborted"}}"#;
        for (ses, err) in [("ses_fail", err_api), ("ses_abort", err_abort)] {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, 100, '{\"role\":\"user\"}')",
                rusqlite::params![format!("mu_{ses}"), ses],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO part (id, message_id, session_id, data, time_created) VALUES (?1, ?2, ?3, '{\"type\":\"text\",\"text\":\"跑一下\"}', 101)",
                rusqlite::params![format!("pu_{ses}"), format!("mu_{ses}"), ses],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, 200, ?3)",
                rusqlite::params![
                    format!("ma_{ses}"),
                    ses,
                    format!(r#"{{"role":"assistant","error":{err}}}"#)
                ],
            )
            .unwrap();
        }
        drop(conn);
        let procs = vec![
            fake_process(11, "C:\\Users\\x\\F"),
            fake_process(22, "C:\\Users\\x\\G"),
        ];
        let sessions = get_opencode_sessions_with_db(&db, &procs);
        assert_eq!(sessions.len(), 2, "两会话各一张卡");
        for s in &sessions {
            match s.id.as_str() {
                "ses_fail" => {
                    assert_eq!(
                        s.status,
                        crate::session::SessionStatus::Waiting,
                        "失败请求挂红（需要用户介入）"
                    );
                    let msg = s.last_message.as_deref().unwrap_or("");
                    assert!(
                        msg.contains("APIError") && msg.contains("Invalid API key"),
                        "失败卡消息行显示错误摘要，实际：{msg}"
                    );
                }
                "ses_abort" => assert_eq!(
                    s.status,
                    crate::session::SessionStatus::Idle,
                    "主动中止不提示（绿）"
                ),
                other => panic!("意外会话 {other}"),
            }
        }
    }

    /// 2026-09-09 事故回归锁：同目录双开两会话，两进程必须各得一张卡（id 各异）。
    /// 修复前红：两进程 find() 命中同一条最新行，另一会话被遮蔽
    #[test]
    fn same_dir_dual_sessions_both_surfaced() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(
            &db,
            &[
                ("ses_new", "C:/Users/x/Desktop/苏州", "公司股东查询", 2000),
                (
                    "ses_old",
                    "C:/Users/x/Desktop/苏州",
                    "查看公司2025年末总资产",
                    1000,
                ),
            ],
        );
        let procs = vec![
            fake_process(11, "C:\\Users\\x\\Desktop\\苏州"),
            fake_process(22, "C:\\Users\\x\\Desktop\\苏州"),
        ];
        let sessions = get_opencode_sessions_with_db(&db, &procs);
        assert_eq!(sessions.len(), 2, "两个终端两张卡");
        let ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            ["ses_new", "ses_old"].into_iter().collect(),
            "两会话都出现，不再互相遮蔽"
        );
    }

    /// 回归：不同目录双开各配各的（既有行为不回退）
    #[test]
    fn distinct_dirs_each_matched() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(
            &db,
            &[
                ("ses_a", "C:/Users/x/A", "会话A", 2000),
                ("ses_b", "C:/Users/x/B", "会话B", 1000),
            ],
        );
        let procs = vec![
            fake_process(11, "C:\\Users\\x\\A"),
            fake_process(22, "C:\\Users\\x\\B"),
        ];
        let sessions = get_opencode_sessions_with_db(&db, &procs);
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|s| s.id == "ses_a"));
        assert!(sessions.iter().any(|s| s.id == "ses_b"));
    }

    /// 回归：单进程 + 同目录多会话 → 取最新（配对顺序语义）
    #[test]
    fn single_process_gets_newest_same_dir_session() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(
            &db,
            &[
                ("ses_new", "C:/Users/x/Desktop/苏州", "公司股东查询", 2000),
                (
                    "ses_old",
                    "C:/Users/x/Desktop/苏州",
                    "查看公司2025年末总资产",
                    1000,
                ),
            ],
        );
        let sessions =
            get_opencode_sessions_with_db(&db, &[fake_process(11, "C:\\Users\\x\\Desktop\\苏州")]);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ses_new");
    }

    // ==== 丁T1 · 待决 question 端到端（DB fixture）====

    /// 端到端主判据（丁T1）：`part` 表尾部是待决 question part → 会话卡 Waiting 红。
    /// 夹具用 2026-09-21 本机 part 表实测形态（tool=question，state.status=pending/
    /// running，input.questions 结构化数组，无 metadata）
    #[test]
    fn pending_question_part_end_to_end_is_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(&db, &[("ses_q", "C:/Users/x/Q", "问答会话", 2000)]);
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES ('m1','ses_q',100,'{\"role\":\"assistant\"}')",
            [],
        )
        .unwrap();
        let part = serde_json::json!({
            "type": "tool",
            "callID": "call_00_pending",
            "tool": "question",
            "state": {
                "status": "running",
                "input": {"questions": [{"header": "Build output folder", "question": "Which folder?", "options": [{"label": "dist", "description": "d"}, {"label": "out", "description": "o"}]}]},
                "time": {"start": 1783326720870i64}
            }
        });
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, data, time_created) VALUES ('prt_q1','m1','ses_q',?1,1783326720870)",
            rusqlite::params![part.to_string()],
        )
        .unwrap();
        drop(conn);

        let sessions = get_opencode_sessions_with_db(&db, &[fake_process(11, "C:\\Users\\x\\Q")]);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].status,
            crate::session::SessionStatus::Waiting,
            "待决 question 尾部必须红灯（终端在等用户作答）"
        );
    }

    /// 端到端回归（丁T1「运行中不误红灯」）：已答 question（completed +
    /// metadata.answers）与普通工具调用尾部都**不得**红灯。
    /// 已答尾部回落到 Running 黄（question 是 tool 部件）
    #[test]
    fn answered_question_end_to_end_is_not_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        fixture_db(
            &db,
            &[
                ("ses_done", "C:/Users/x/D", "已答会话", 2000),
                ("ses_bash", "C:/Users/x/B", "普通工具会话", 2000),
            ],
        );
        let conn = Connection::open(&db).unwrap();
        for (mid, ses, data, ts) in [
            (
                "md1",
                "ses_done",
                serde_json::json!({
                    "type": "tool",
                    "tool": "question",
                    "state": {
                        "status": "completed",
                        "input": {"questions": [{"question": "Which folder?", "options": [{"label": "out"}]}]},
                        "metadata": {"answers": [["out"]], "truncated": false},
                        "time": {"start": 1783326720870i64, "end": 1783326749518i64}
                    }
                })
                .to_string(),
                1783326749518i64,
            ),
            (
                "md2",
                "ses_bash",
                serde_json::json!({
                    "type": "tool",
                    "tool": "bash",
                    "state": {"status": "running", "input": {"command": "ls"}}
                })
                .to_string(),
                1783326749518i64,
            ),
        ] {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, 100, '{\"role\":\"assistant\"}')",
                rusqlite::params![mid, ses],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO part (id, message_id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![format!("prt_{mid}"), mid, ses, data, ts],
            )
            .unwrap();
        }
        drop(conn);

        let sessions = get_opencode_sessions_with_db(
            &db,
            &[
                fake_process(11, "C:\\Users\\x\\D"),
                fake_process(22, "C:\\Users\\x\\B"),
            ],
        );
        assert_eq!(sessions.len(), 2);
        for s in &sessions {
            assert_ne!(
                s.status,
                crate::session::SessionStatus::Waiting,
                "{}：已答/普通工具尾部不得红灯（运行中不误报）",
                s.id
            );
        }
    }
}
