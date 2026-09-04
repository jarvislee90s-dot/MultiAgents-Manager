// 未读会话（APP 类绿色已完成、用户未查看）持久层 — 单轨物理删除（spec W4）
use rusqlite::{params, Connection};

use crate::database::connection::DB;

/// 未读会话记录（联合唯一键 tool_id + session_id；expires_at = turned_green_at + 24h）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadSessionRecord {
    pub tool_id: String,
    pub session_id: String,
    pub project_name: String,
    pub title: Option<String>,
    pub last_message: Option<String>,
    pub turned_green_at_ms: i64,
    pub expires_at_ms: i64,
}

/// 插入或覆盖（同 tool_id + session_id 直接更新，无软删标记）
pub fn upsert_unread(conn: &Connection, r: &UnreadSessionRecord) {
    let _ = conn.execute(
        "INSERT INTO unread_sessions
            (tool_id, session_id, project_name, title, last_message, turned_green_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(tool_id, session_id) DO UPDATE SET
            project_name = excluded.project_name,
            title = excluded.title,
            last_message = excluded.last_message,
            turned_green_at = excluded.turned_green_at,
            expires_at = excluded.expires_at",
        params![
            r.tool_id,
            r.session_id,
            r.project_name,
            r.title,
            r.last_message,
            r.turned_green_at_ms,
            r.expires_at_ms
        ],
    );
}

/// 单条物理删除（已读 / 手动关闭等终态直接删行）
pub fn delete_unread(conn: &Connection, tool_id: &str, session_id: &str) {
    let _ = conn.execute(
        "DELETE FROM unread_sessions WHERE tool_id = ?1 AND session_id = ?2",
        params![tool_id, session_id],
    );
}

/// 未过期未读列表（按转绿时间倒序）
pub fn list_unread(conn: &Connection, now_ms: i64) -> Vec<UnreadSessionRecord> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT tool_id, session_id, project_name, title, last_message, turned_green_at, expires_at
         FROM unread_sessions WHERE expires_at > ?1
         ORDER BY turned_green_at DESC",
    ) else {
        return Vec::new();
    };
    stmt.query_map(params![now_ms], |row| {
        Ok(UnreadSessionRecord {
            tool_id: row.get(0)?,
            session_id: row.get(1)?,
            project_name: row.get(2)?,
            title: row.get(3)?,
            last_message: row.get(4)?,
            turned_green_at_ms: row.get(5)?,
            expires_at_ms: row.get(6)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// 清空指定工具的全部未读（工具取消勾选等场景）
pub fn clear_unread_for_tool(conn: &Connection, tool_id: &str) {
    let _ = conn.execute(
        "DELETE FROM unread_sessions WHERE tool_id = ?1",
        params![tool_id],
    );
}

/// 物理清理已过期行（expires_at <= now_ms）
pub fn cleanup_expired_unread(conn: &Connection, now_ms: i64) {
    let _ = conn.execute(
        "DELETE FROM unread_sessions WHERE expires_at <= ?1",
        params![now_ms],
    );
}

// ---- 全局连接包装（业务侧零锁代码） ----

pub fn upsert(r: &UnreadSessionRecord) {
    let conn = DB.lock().unwrap();
    upsert_unread(&conn, r);
}

pub fn delete(tool_id: &str, session_id: &str) {
    let conn = DB.lock().unwrap();
    delete_unread(&conn, tool_id, session_id);
}

pub fn list(now_ms: i64) -> Vec<UnreadSessionRecord> {
    let conn = DB.lock().unwrap();
    list_unread(&conn, now_ms)
}

pub fn clear_tool(tool_id: &str) {
    let conn = DB.lock().unwrap();
    clear_unread_for_tool(&conn, tool_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        conn
    }

    fn rec(tool: &str, sid: &str, at: i64) -> UnreadSessionRecord {
        UnreadSessionRecord {
            tool_id: tool.into(),
            session_id: sid.into(),
            project_name: "proj".into(),
            title: Some("标题".into()),
            last_message: Some("消息".into()),
            turned_green_at_ms: at,
            expires_at_ms: at + 24 * 3600 * 1000,
        }
    }

    #[test]
    fn upsert_then_list_and_dedupe() {
        let conn = mem();
        upsert_unread(&conn, &rec("workbuddy", "s1", 1000));
        upsert_unread(&conn, &rec("workbuddy", "s1", 2000)); // 同键覆盖
        assert_eq!(list_unread(&conn, 3000).len(), 1);
        assert_eq!(list_unread(&conn, 3000)[0].turned_green_at_ms, 2000);
    }

    #[test]
    fn delete_single_and_clear_tool() {
        let conn = mem();
        upsert_unread(&conn, &rec("workbuddy", "s1", 1000));
        upsert_unread(&conn, &rec("workbuddy", "s2", 1000));
        upsert_unread(&conn, &rec("codex", "s3", 1000));
        delete_unread(&conn, "workbuddy", "s1");
        clear_unread_for_tool(&conn, "codex");
        let left = list_unread(&conn, 2000);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].session_id, "s2");
    }

    #[test]
    fn expired_not_listed_and_cleanup_removes() {
        let conn = mem();
        upsert_unread(&conn, &rec("workbuddy", "old", 1000));
        let far = 1000 + 24 * 3600 * 1000 + 1;
        assert!(list_unread(&conn, far).is_empty());
        cleanup_expired_unread(&conn, far);
        assert!(list_unread(&conn, far).is_empty());
        // 物理删除后行数归零
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM unread_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
