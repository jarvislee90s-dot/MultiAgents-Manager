use rusqlite::params;
use std::collections::HashSet;

use crate::database::connection::DB;

/// 更新会话状态，返回是否状态发生了变化（用于通知去重）
pub fn update_session_status(session_id: &str, agent_type: &str, status: &str) -> Option<String> {
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
/// 读取缓存的上一轮状态（回合一末尾才统一更新缓存，轮中读取即「上一轮」值；
/// review F1 未读迁移语义据此判定状态迁移边沿）
pub fn find_status(session_id: &str) -> Option<String> {
    let conn = DB.lock().unwrap();
    conn.query_row(
        "SELECT status FROM session_status_cache WHERE session_id = ?",
        [session_id],
        |row| row.get(0),
    )
    .ok()
}

/// 清理不再活跃的会话缓存。
/// issue #35-1：离板即删会让上一轮状态「失忆」——心跳间隙（系统睡眠唤醒 / sidecar
/// 挂起）后回板的会话读到 prev=None，Insert 边沿把已读删掉的未读行复活、Codex 绿卡
/// 复现。改为 TTL 清理：离板行保留 24h（与未读池窗口一致）后随下一轮清理移除，
/// 活跃行照常每轮刷新不受影响
pub fn cleanup_stale_sessions(active_ids: &HashSet<String>) {
    let conn = DB.lock().unwrap();
    cleanup_stale_sessions_conn(&conn, active_ids);
}

/// 可测核心（连接注入）：TTL = 24h，last_seen 为 RFC3339 文本（统一 UTC 生成，
/// 同格式字典序即时间序）
pub fn cleanup_stale_sessions_conn(conn: &rusqlite::Connection, active_ids: &HashSet<String>) {
    const CACHE_TTL_MS: i64 = 24 * 3600 * 1000;
    let cutoff = (chrono::Utc::now() - chrono::Duration::milliseconds(CACHE_TTL_MS)).to_rfc3339();
    let all: Vec<(String, String)> = conn
        .prepare("SELECT session_id, last_seen FROM session_status_cache")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default();
    for (id, last_seen) in &all {
        if !active_ids.contains(id) && last_seen.as_str() < cutoff.as_str() {
            let _ = conn.execute(
                "DELETE FROM session_status_cache WHERE session_id = ?",
                [id],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        conn
    }

    fn insert_status(conn: &Connection, sid: &str, last_seen: &str) {
        conn.execute(
            "INSERT INTO session_status_cache (session_id, agent_type, status, last_seen)
             VALUES (?1, 'WorkBuddy', 'Idle', ?2)",
            [sid, last_seen],
        )
        .unwrap();
    }

    fn exists(conn: &Connection, sid: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM session_status_cache WHERE session_id = ?",
            [sid],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// issue #35-1 回归锁：离板行在 TTL 内保留（心跳间隙后回板仍读得到上一轮状态，
    /// Insert 边沿不失忆）；超龄行清除；活跃行永不清理
    #[test]
    fn stale_cache_cleanup_is_ttl_based() {
        let conn = mem();
        let now = chrono::Utc::now().to_rfc3339();
        insert_status(&conn, "active", &now);
        insert_status(&conn, "fresh-gone", &now);
        insert_status(&conn, "aged-gone", "2020-01-01T00:00:00+00:00");
        let active: HashSet<String> = ["active".to_string()].into();
        cleanup_stale_sessions_conn(&conn, &active);
        assert!(exists(&conn, "active"), "活跃行不得清理");
        assert!(exists(&conn, "fresh-gone"), "TTL 内的离板行不得清理");
        assert!(!exists(&conn, "aged-gone"), "超龄离板行应被清理");
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
