// OpenCode 会话解析器 — 基于 SQLite 数据库（opencode.db）
// OpenCode 1.17+ 使用 SQLite 替代分散 JSON 文件，此模块查询数据库获取会话状态

use super::parser::{cwd_equivalent, normalize_cwd_for_match};
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, Session, SessionStatus};
use log::{debug, info};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// message.data JSON 结构
#[derive(Deserialize)]
struct MessageData {
    role: Option<String>,
}

/// part.data JSON 结构
#[derive(Deserialize)]
struct PartData {
    #[serde(rename = "type")]
    part_type: Option<String>,
    text: Option<String>,
}

/// 获取 OpenCode 会话
pub fn get_opencode_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    if processes.is_empty() {
        return Vec::new();
    }

    let db_path = match dirs::home_dir() {
        Some(h) => h
            .join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db"),
        None => return Vec::new(),
    };

    if !db_path.exists() {
        debug!("OpenCode database not found: {:?}", db_path);
        return Vec::new();
    }

    // 只读连接，避免锁冲突
    let conn = match Connection::open_with_flags(
        &db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(e) => {
            debug!("Failed to open OpenCode database: {}", e);
            return Vec::new();
        }
    };
    let _ = conn.busy_timeout(Duration::from_millis(1000));

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

    // ---- 主匹配：session.directory（会话启动目录）与进程 cwd 归一化相等 ----
    // （含 global 会话；取代原按 directory 精确 SQL 的 global 回退——SQL 精确匹配无法
    //   处理分隔符/大小写差异，归一化比较必须在 Rust 侧做）
    for process in processes {
        let Some(cwd) = &process.cwd else { continue };
        let cwd_str = cwd.to_string_lossy();
        if let Some((session_id, directory, title, time_updated)) = recent
            .iter()
            .find(|(_, dir, _, _)| cwd_equivalent(dir, &cwd_str))
        {
            matched_pids.insert(process.pid);
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
    let (last_role, last_message) = get_last_message_info(conn, session_id);
    let last_msg_time = get_last_message_time(conn, session_id);

    let status = determine_opencode_status(
        process.cpu_usage,
        last_role.as_deref(),
        last_msg_time,
        time_updated,
    );
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
    let display_message = last_message.or_else(|| {
        if !title.is_empty() {
            Some(title.clone())
        } else {
            None
        }
    });

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
        title: Some(title),
    })
}

/// 获取最后一条消息的时间戳（毫秒）
fn get_last_message_time(conn: &Connection, session_id: &str) -> i64 {
    conn.query_row(
        "SELECT time_created FROM message WHERE session_id = ? ORDER BY time_created DESC LIMIT 1",
        [session_id],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
}

/// 获取会话最后一条消息的角色和文本
fn get_last_message_info(conn: &Connection, session_id: &str) -> (Option<String>, Option<String>) {
    // 查最后一条消息
    let mut stmt = match conn.prepare(
        "SELECT id, data FROM message WHERE session_id = ? ORDER BY time_created DESC LIMIT 1",
    ) {
        Ok(s) => s,
        Err(_) => return (None, None),
    };

    let result = stmt.query_row([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    });

    let (message_id, data_json) = match result {
        Ok(r) => r,
        Err(_) => return (None, None),
    };

    // 解析 message.data JSON 获取 role
    let role = serde_json::from_str::<MessageData>(&data_json)
        .ok()
        .and_then(|d| d.role);

    // 查该消息的 text 类型 parts
    let text = get_message_text(conn, &message_id);

    (role, text)
}

/// 获取消息的文本内容（优先 text 类型，其次 reasoning）
fn get_message_text(conn: &Connection, message_id: &str) -> Option<String> {
    let mut stmt = conn
        .prepare("SELECT data FROM part WHERE message_id = ? ORDER BY time_created ASC")
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

/// OpenCode 状态判断：CPU > 5% → Processing，assistant 且近期活跃 → Waiting，否则 Idle
fn determine_opencode_status(
    cpu: f32,
    last_role: Option<&str>,
    last_msg_time: i64,
    session_updated: i64,
) -> SessionStatus {
    if cpu > 5.0 {
        SessionStatus::Processing
    } else {
        // 检查是否近期活跃（最后消息时间或会话更新时间在 60s 内）
        let now = chrono::Utc::now().timestamp_millis();
        let last_active = last_msg_time.max(session_updated);
        let is_recent = now - last_active < 60_000; // 60 秒内
        match last_role {
            Some("assistant") if is_recent => SessionStatus::Waiting,
            Some("user") if is_recent => SessionStatus::Processing,
            _ => SessionStatus::Idle, // 超过 60s 无活动 → Idle（防止空闲误报 Waiting）
        }
    }
}

/// 毫秒时间戳 → ISO 8601 字符串
fn ms_to_iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp(ms / 1000, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}
