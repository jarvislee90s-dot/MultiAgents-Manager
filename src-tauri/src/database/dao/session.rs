use rusqlite::params;
use std::collections::HashSet;

use crate::database::connection::DB;

/// 更新会话状态，返回是否状态发生了变化（用于通知去重）
pub fn update_session_status(
    session_id: &str,
    agent_type: &str,
    status: &str,
) -> Option<String> {
    let conn = DB.lock().unwrap();
    let previous: Option<String> = conn
        .query_row(
            "SELECT status FROM session_status_cache WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )
        .ok();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO session_status_cache
         (session_id, agent_type, status, last_seen, previous_status)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![session_id, agent_type, status, now, previous.as_deref()],
    )
    .ok();
    if previous.as_deref() == Some(status) {
        None
    } else {
        previous
    }
}

/// 清理不再活跃的会话缓存
pub fn cleanup_stale_sessions(active_ids: &HashSet<String>) {
    let conn = DB.lock().unwrap();
    let all: Vec<String> = conn
        .prepare("SELECT session_id FROM session_status_cache")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    for id in &all {
        if !active_ids.contains(id) {
            let _ = conn.execute(
                "DELETE FROM session_status_cache WHERE session_id = ?",
                [id],
            );
        }
    }
}

/// Session 数据访问标准接口
pub trait SessionDao {
    fn find_all_statuses(&self) -> Vec<(String, String, String)>;
    fn find_status(&self, session_id: &str) -> Option<String>;
    fn upsert_status(&self, session_id: &str, agent_type: &str, status: &str) -> Option<String>;
    fn delete(&self, session_id: &str);
}

pub struct SessionDaoImpl;
impl SessionDao for SessionDaoImpl {
    fn find_all_statuses(&self) -> Vec<(String, String, String)> {
        let conn = DB.lock().unwrap();
        conn.prepare("SELECT session_id, agent_type, status FROM session_status_cache")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    }
    fn find_status(&self, session_id: &str) -> Option<String> {
        let conn = DB.lock().unwrap();
        conn.query_row(
            "SELECT status FROM session_status_cache WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )
        .ok()
    }
    fn upsert_status(&self, session_id: &str, agent_type: &str, status: &str) -> Option<String> {
        update_session_status(session_id, agent_type, status)
    }
    fn delete(&self, session_id: &str) {
        let conn = DB.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM session_status_cache WHERE session_id = ?",
            [session_id],
        );
    }
}
