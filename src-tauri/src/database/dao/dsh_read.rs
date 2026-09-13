// dsh 会话已读水位（MAM 自有库——绝不写 ~/.dsh）
// 未读判定：最近关键事件时间 > last_read_at ⇒ 未读（覆盖"刚完成"与"错误保持未读"两语义）

use rusqlite::Connection;

/// 查询会话已读水位；无记录（含查询失败）返回 None，视为从未读过
pub fn last_read_at(conn: &Connection, session_id: &str) -> Option<i64> {
    conn.query_row(
        "SELECT last_read_at FROM dsh_session_read WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )
    .ok()
}

/// 标记会话已读（upsert：无记录插入、有记录覆盖刷新）
pub fn mark_read(conn: &Connection, session_id: &str, now_ms: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO dsh_session_read (session_id, last_read_at) VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET last_read_at = ?2",
        rusqlite::params![session_id, now_ms],
    )
    .map_err(|e| format!("dsh mark_read 失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE IF NOT EXISTS dsh_session_read (
                 session_id TEXT PRIMARY KEY,
                 last_read_at INTEGER NOT NULL DEFAULT 0
             );",
        )
        .unwrap();
        c
    }

    #[test]
    fn mark_and_read_roundtrip() {
        let c = conn();
        assert_eq!(last_read_at(&c, "session-1"), None);
        mark_read(&c, "session-1", 1000).unwrap();
        assert_eq!(last_read_at(&c, "session-1"), Some(1000));
        mark_read(&c, "session-1", 2000).unwrap(); // 幂等覆盖
        assert_eq!(last_read_at(&c, "session-1"), Some(2000));
    }
}
