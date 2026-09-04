// WorkBuddy 会话解析：心跳文件（~/.workbuddy/sessions/<PID>.json）关联进程与会话，
// 会话历史在 ~/.workbuddy/projects/<路径编码>/<sessionId>.jsonl（OpenAI 风格 type/role/content）
// 所有文件均为未文档化私有格式：解析失败一律跳过/降级，禁止 panic（spec W3 防御性要求）

use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, ProcessForm, Session, SessionStatus};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 心跳新鲜阈值：取 MAM 轮询周期（约 30s）的 3 倍，防止轮询间隙卡片闪烁
pub const HEARTBEAT_FRESH_MS: u64 = 90_000;

/// 每轮观测到的 pid → sessionId（Task 10 心跳消失补偿的依据）
pub static LAST_SEEN_SESSIONS: Lazy<Mutex<HashMap<u32, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct Heartbeat {
    pub pid: u32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(rename = "lastHeartbeat")]
    pub last_heartbeat_ms: u64,
}

pub fn parse_heartbeat(json: &str) -> Option<Heartbeat> {
    serde_json::from_str(json).ok()
}

/// sessionId 为 UUID（非 interactive-*）才是真实任务会话；--serve 常驻服务排除
pub fn heartbeat_session_id_is_uuid(hb: &Heartbeat) -> bool {
    hb.session_id.len() == 36 && hb.session_id.bytes().filter(|c| *c == b'-').count() == 4
}

pub fn heartbeat_is_alive(hb: &Heartbeat, now_ms: u64) -> bool {
    now_ms.saturating_sub(hb.last_heartbeat_ms) < HEARTBEAT_FRESH_MS
}

/// 项目路径编码：去首分隔符后 / 与 \ 统一替换为 -
pub fn mangle_project_path(cwd: &str) -> String {
    let trimmed = cwd.trim_start_matches('/');
    trimmed.replace(['/', '\\'], "-")
}

pub fn session_jsonl_path(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    home.join(".workbuddy")
        .join("projects")
        .join(mangle_project_path(cwd))
        .join(format!("{}.jsonl", session_id))
}

/// JSONL 尾部状态推导：最后一条有效条目决定状态（spec W3 映射）
pub fn derive_status_from_tail(lines: &[String]) -> SessionStatus {
    let mut last: Option<&String> = None;
    for line in lines {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) if v.get("type").is_some() => last = Some(line),
            _ => continue,
        }
    }
    let Some(line) = last else {
        return SessionStatus::Waiting;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return SessionStatus::Waiting;
    };
    match v["type"].as_str().unwrap_or_default() {
        "message" => match v["role"].as_str().unwrap_or_default() {
            "user" => SessionStatus::Thinking,
            _ => SessionStatus::Idle, // assistant 完成
        },
        "function_call" | "function_call_result" => SessionStatus::Processing,
        // reasoning/file-history-snapshot 等中间条目按运行中处理
        _ => SessionStatus::Processing,
    }
}

/// 会话标题：只读打开 workbuddy.db 读 sessions.title；失败降级 None（调用方再降级首条 user 消息）
pub fn title_from_db(home: &Path, session_id: &str) -> Option<String> {
    use rusqlite::OpenFlags;
    let db = home.join(".workbuddy").join("workbuddy.db");
    let conn = rusqlite::Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT title FROM sessions WHERE id = ?1 AND deleted_at IS NULL",
        [session_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .filter(|t| !t.trim().is_empty())
}

fn heartbeat_path(home: &Path, pid: u32) -> PathBuf {
    home.join(".workbuddy")
        .join("sessions")
        .join(format!("{}.json", pid))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 主入口：活跃心跳的 WorkBuddy 进程 → 每会话一张卡
pub fn get_workbuddy_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return sessions;
    };
    let now = now_ms();

    for process in processes {
        // 防御：心跳文件缺失/损坏 → 跳过该进程（含独立 CLI、空闲 prewarm）
        let Some(hb) = std::fs::read_to_string(heartbeat_path(&home, process.pid))
            .ok()
            .and_then(|s| parse_heartbeat(&s))
        else {
            continue;
        };
        if !heartbeat_session_id_is_uuid(&hb) || !heartbeat_is_alive(&hb, now) {
            continue;
        }

        let jsonl = session_jsonl_path(&home, &hb.cwd, &hb.session_id);
        if !jsonl.exists() {
            continue; // 会话文件未落盘（防御）
        }

        // 尾部解析（复用通用 JSONL 尾读设施；行数与 codex 一致 500）
        let lines = read_recent_lines(&jsonl, 500);
        let status = derive_status_from_tail(&lines);
        let last_message = lines
            .iter()
            .rev()
            .find_map(|l| extract_message_text(l))
            .unwrap_or_default();

        let title = title_from_db(&home, &hb.session_id)
            .or_else(|| first_user_text(&lines))
            .map(|t| t.chars().take(60).collect::<String>());

        sessions.push(Session {
            id: hb.session_id.clone(),
            agent_type: AgentType::WorkBuddy,
            project_name: project_name_from_path(&hb.cwd),
            project_path: hb.cwd.clone(),
            title,
            git_branch: None,
            github_url: get_github_url(&hb.cwd),
            status,
            last_message: if last_message.is_empty() {
                None
            } else {
                Some(last_message)
            },
            last_message_role: None,
            last_activity_at: jsonl
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
                .unwrap_or_default(),
            pid: process.pid,
            cpu_usage: process.cpu_usage,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: jump_supported_for(ProcessForm::App),
            unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        });

        // 记录本轮 pid→session（心跳消失补偿依据）
        LAST_SEEN_SESSIONS
            .lock()
            .unwrap()
            .insert(process.pid, hb.session_id.clone());
    }
    sessions
}

/// 提取 message 条目 content 数组中首个非空 text 片段
fn extract_message_text(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str()? != "message" {
        return None;
    }
    v["content"]
        .as_array()?
        .iter()
        .find_map(|c| {
            c.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.trim().is_empty())
}

/// 降级标题：首条 user 消息文本（DB 查询失败时使用）
fn first_user_text(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            if v["type"].as_str() == Some("message") && v["role"].as_str() == Some("user") {
                extract_message_text(l)
            } else {
                None
            }
        })
        .next()
}

/// 转绿竞态补偿（spec W4）：任务完成 → prewarm 回池（心跳删除）可能只隔几秒，
/// 若两轮扫描间完成，「转绿」从未被观测 → 未读漏插。此处对上一轮见过、本轮心跳
/// 消失的 pid 读其 JSONL 终态：完成 → 补插未读；运行中被杀 → 不插
pub fn compensate_vanished_heartbeats() {
    let Some(home) = dirs::home_dir() else { return };
    let now = now_ms();

    // 锁内只做纯内存快照（锁 hygiene：绝不持锁跨文件 I/O），判定消失在锁外进行
    let candidates: Vec<(u32, String)> = {
        let last_seen = LAST_SEEN_SESSIONS.lock().unwrap();
        last_seen
            .iter()
            .map(|(pid, sid)| (*pid, sid.clone()))
            .collect()
    };
    let vanished: Vec<(u32, String)> = candidates
        .into_iter()
        .filter(|(pid, _)| {
            // 心跳文件没了 = 回池/退出；过期同样视为消失
            match std::fs::read_to_string(heartbeat_path(&home, *pid))
                .ok()
                .and_then(|s| parse_heartbeat(&s))
            {
                Some(hb) => !heartbeat_is_alive(&hb, now),
                None => true,
            }
        })
        .collect();

    for (pid, session_id) in vanished {
        // 逐个短暂重锁移除（不做额外清理，未消失条目保留供下轮参考）
        LAST_SEEN_SESSIONS.lock().unwrap().remove(&pid);
        // 找该会话的 JSONL：遍历 projects 下所有 <sessionId>.jsonl（会话可能换过项目目录）
        let projects_dir = home.join(".workbuddy").join("projects");
        let Ok(entries) = std::fs::read_dir(&projects_dir) else {
            continue; // 防御：目录读取失败只跳过该 pid，不中断其余补偿
        };
        let target = entries.filter_map(|e| e.ok()).find_map(|dir| {
            let p = dir.path().join(format!("{}.jsonl", session_id));
            p.exists().then_some(p)
        });
        let Some(jsonl) = target else { continue };
        let lines = read_recent_lines(&jsonl, 500);
        if derive_status_from_tail(&lines) != SessionStatus::Idle {
            continue; // 非完成态（运行中被杀等）→ 不补
        }
        // cwd 直接用 WorkBuddy DB 的 cwd 字段反查
        let cwd = workbuddy_cwd_from_db(&home, &session_id).unwrap_or_default();
        let last_message = lines.iter().rev().find_map(|l| extract_message_text(l));
        crate::database::dao::unread::upsert(&crate::database::dao::unread::UnreadSessionRecord {
            tool_id: "workbuddy".into(),
            session_id: session_id.clone(),
            project_name: if cwd.is_empty() {
                "WorkBuddy".into()
            } else {
                project_name_from_path(&cwd)
            },
            title: title_from_db(&home, &session_id),
            last_message,
            turned_green_at_ms: now as i64,
            expires_at_ms: now as i64 + 24 * 3600 * 1000,
        });
    }
}

/// 补偿用：只读打开 workbuddy.db 反查会话 cwd；列缺失/打开失败一律 None（防御）
fn workbuddy_cwd_from_db(home: &Path, session_id: &str) -> Option<String> {
    use rusqlite::OpenFlags;
    let db = home.join(".workbuddy").join("workbuddy.db");
    let conn = rusqlite::Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT cwd FROM sessions WHERE id = ?1 AND deleted_at IS NULL",
        [session_id],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEARTBEAT_ACTIVE: &str = r#"{
      "pid": 11952,
      "lastHeartbeat": 1788444900119,
      "sessionId": "7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c",
      "cwd": "/Users/jarvis/Documents/MultiAgents-Manager",
      "startedAt": 1788444900112,
      "kind": "interactive",
      "updatedAt": 1788444900347
    }"#;

    const HEARTBEAT_SERVE: &str = r#"{
      "pid": 8979,
      "lastHeartbeat": 1788445813951,
      "sessionId": "interactive-8979",
      "cwd": "/private/var/folders/xx/T/workbuddy-host-cli/xxx",
      "kind": "interactive",
      "url": "http://127.0.0.1:50027"
    }"#;

    #[test]
    fn mangle_strips_leading_slash_and_replaces_separators() {
        assert_eq!(
            mangle_project_path("/Users/jarvis/Documents/MultiAgents-Manager"),
            "Users-jarvis-Documents-MultiAgents-Manager"
        );
        // Windows 形态容错：反斜杠路径同样编码
        assert_eq!(
            mangle_project_path("C:\\Users\\jarvis\\proj"),
            "C:-Users-jarvis-proj"
        );
    }

    #[test]
    fn heartbeat_uuid_session_is_real_task() {
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert_eq!(hb.pid, 11952);
        assert!(heartbeat_session_id_is_uuid(&hb));
        let serve = parse_heartbeat(HEARTBEAT_SERVE).unwrap();
        assert!(!heartbeat_session_id_is_uuid(&serve)); // --serve 排除
    }

    #[test]
    fn heartbeat_parse_rejects_garbage() {
        assert!(parse_heartbeat("not json").is_none());
        assert!(parse_heartbeat("{}").is_none()); // 缺 sessionId
    }

    #[test]
    fn heartbeat_freshness() {
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert!(heartbeat_is_alive(&hb, hb.last_heartbeat_ms + 1));
        assert!(!heartbeat_is_alive(
            &hb,
            hb.last_heartbeat_ms + HEARTBEAT_FRESH_MS + 1
        ));
    }

    #[test]
    fn session_jsonl_path_layout() {
        let p = session_jsonl_path(
            std::path::Path::new("/home/u"),
            "/Users/jarvis/Documents/MultiAgents-Manager",
            "7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c",
        );
        assert_eq!(
            p,
            std::path::PathBuf::from(
                "/home/u/.workbuddy/projects/Users-jarvis-Documents-MultiAgents-Manager/7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c.jsonl"
            )
        );
    }

    #[test]
    fn tail_user_message_is_thinking() {
        let lines = vec![
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"跑测试"}]}"#
                .into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Thinking);
    }

    #[test]
    fn tail_function_call_is_processing() {
        let lines = vec![r#"{"type":"function_call","name":"shell"}"#.into()];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Processing);
    }

    #[test]
    fn tail_assistant_text_is_idle() {
        let lines = vec![
            r#"{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"完成"}]}"#.into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Idle);
    }

    #[test]
    fn tail_last_entry_wins() {
        let lines = vec![
            r#"{"type":"function_call","name":"shell"}"#.into(),
            r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"好"}]}"#.into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Idle);
    }

    #[test]
    fn tail_empty_is_waiting() {
        assert_eq!(derive_status_from_tail(&[]), SessionStatus::Waiting);
    }
}
