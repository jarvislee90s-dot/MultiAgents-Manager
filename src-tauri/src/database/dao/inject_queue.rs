// 注入队列表 DAO（M7）：每会话 FIFO 待发消息账本
// jumped 列为 Task 6 插队标记（UPDATE 专用），入队默认 0，不进 QueueRow 模型
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

/// SELECT 列清单（顺序与 row_to_queue 对齐）
const COLS: &str = "id, session_id, agent_type, device_id, device_name, content, enqueued_at, sent_at, failed_reason";

/// 行映射（列序见 COLS）
fn row_to_queue(row: &rusqlite::Row) -> rusqlite::Result<QueueRow> {
    Ok(QueueRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        agent_type: row.get(2)?,
        device_id: row.get(3)?,
        device_name: row.get(4)?,
        content: row.get(5)?,
        enqueued_at: row.get(6)?,
        sent_at: row.get(7)?,
        failed_reason: row.get(8)?,
    })
}

/// 队列行：sent_at / failed_reason 均为空 = 待发（pending）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueRow {
    pub id: i64,
    pub session_id: String,
    pub agent_type: String,
    pub device_id: String,
    pub device_name: String,
    pub content: String,
    pub enqueued_at: i64,
    pub sent_at: Option<i64>,
    pub failed_reason: Option<String>,
}

/// 入队（全局 DB）
pub fn enqueue(
    session_id: &str,
    agent_type: &str,
    device_id: &str,
    device_name: &str,
    content: &str,
    enqueued_at: i64,
) -> i64 {
    let conn = DB.lock().unwrap();
    enqueue_conn(
        &conn,
        session_id,
        agent_type,
        device_id,
        device_name,
        content,
        enqueued_at,
    )
}

/// 某会话全部待发消息，FIFO（全局 DB）
pub fn pending_for_session(session_id: &str) -> Vec<QueueRow> {
    let conn = DB.lock().unwrap();
    pending_for_session_conn(&conn, session_id)
}

/// 队首（全局 DB）
pub fn next_pending(session_id: &str) -> Option<QueueRow> {
    let conn = DB.lock().unwrap();
    next_pending_conn(&conn, session_id)
}

/// 标记已发送（全局 DB）
pub fn mark_sent(id: i64, now: i64) {
    let conn = DB.lock().unwrap();
    mark_sent_conn(&conn, id, now);
}

/// 标记失败（全局 DB）
pub fn mark_failed(id: i64, reason: &str) {
    let conn = DB.lock().unwrap();
    mark_failed_conn(&conn, id, reason);
}

/// 撤回（仅当该行属于 sid 才删）（全局 DB）
pub fn retract(session_id: &str, id: i64) -> bool {
    let conn = DB.lock().unwrap();
    retract_conn(&conn, session_id, id)
}

/// 按 id 查单行（全局 DB；mark_sent / mark_failed 后观察落库状态用）
pub fn get(id: i64) -> Option<QueueRow> {
    let conn = DB.lock().unwrap();
    get_conn(&conn, id)
}

/// 入队：插入一行 pending（jumped 走列默认 0），返回新 id
pub fn enqueue_conn(
    conn: &Connection,
    session_id: &str,
    agent_type: &str,
    device_id: &str,
    device_name: &str,
    content: &str,
    enqueued_at: i64,
) -> i64 {
    conn.execute(
        "INSERT INTO inject_queue (session_id, agent_type, device_id, device_name, content, enqueued_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![session_id, agent_type, device_id, device_name, content, enqueued_at],
    )
    .expect("inject_queue 入队失败");
    conn.last_insert_rowid()
}

/// 某会话全部待发消息（sent_at IS NULL AND failed_reason IS NULL），FIFO（按 id 升序）
pub fn pending_for_session_conn(conn: &Connection, session_id: &str) -> Vec<QueueRow> {
    let sql = format!(
        "SELECT {COLS} FROM inject_queue WHERE session_id = ?1 AND sent_at IS NULL AND failed_reason IS NULL ORDER BY id ASC"
    );
    let mut stmt = conn.prepare(&sql).expect("inject_queue 查询 pending 失败");
    let rows = stmt
        .query_map([session_id], row_to_queue)
        .expect("inject_queue 遍历 pending 失败");
    rows.filter_map(|r| r.ok()).collect()
}

/// 队首：该会话最小 id 的待发行
pub fn next_pending_conn(conn: &Connection, session_id: &str) -> Option<QueueRow> {
    let sql = format!(
        "SELECT {COLS} FROM inject_queue WHERE session_id = ?1 AND sent_at IS NULL AND failed_reason IS NULL ORDER BY id ASC LIMIT 1"
    );
    conn.query_row(&sql, [session_id], row_to_queue)
        .optional()
        .ok()
        .flatten()
}

/// 标记已发送（落 sent_at，行保留作审计痕迹）
pub fn mark_sent_conn(conn: &Connection, id: i64, now: i64) {
    conn.execute(
        "UPDATE inject_queue SET sent_at = ?2 WHERE id = ?1",
        params![id, now],
    )
    .expect("inject_queue 标记已发失败");
}

/// 标记失败（落 failed_reason，行保留作审计痕迹）
pub fn mark_failed_conn(conn: &Connection, id: i64, reason: &str) {
    conn.execute(
        "UPDATE inject_queue SET failed_reason = ?2 WHERE id = ?1",
        params![id, reason],
    )
    .expect("inject_queue 标记失败失败");
}

/// 撤回：仅当 id 属于 session_id 才删，返回是否删到
pub fn retract_conn(conn: &Connection, session_id: &str, id: i64) -> bool {
    matches!(
        conn.execute(
            "DELETE FROM inject_queue WHERE session_id = ?1 AND id = ?2",
            params![session_id, id],
        ),
        Ok(n) if n > 0
    )
}

/// 按 id 查单行（测试观察 sent_at / failed_reason 落库）
pub fn get_conn(conn: &Connection, id: i64) -> Option<QueueRow> {
    let sql = format!("SELECT {COLS} FROM inject_queue WHERE id = ?1");
    conn.query_row(&sql, [id], row_to_queue)
        .optional()
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        conn
    }

    /// 入队返回自增 id，pending FIFO，队首 = 最小 id；跨会话互不可见
    #[test]
    fn enqueue_returns_id_and_pending_lists_in_order() {
        let c = mem();
        let a = enqueue_conn(&c, "s1", "claude", "d1", "iPhone", "msg-a", 1000);
        let b = enqueue_conn(&c, "s1", "claude", "d1", "iPhone", "msg-b", 1001);
        assert!(b > a, "自增 id 必须递增");
        let pending = pending_for_session_conn(&c, "s1");
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].content, "msg-a"); // FIFO
        assert_eq!(pending[1].content, "msg-b");
        assert_eq!(next_pending_conn(&c, "s1").unwrap().id, a);
        // 跨会话隔离：s2 看不到 s1 的队列
        assert!(pending_for_session_conn(&c, "s2").is_empty());
        assert_eq!(next_pending_conn(&c, "s2"), None);
    }

    /// mark_sent 后退出 pending，行保留且 sent_at 落库可观察
    #[test]
    fn mark_sent_removes_from_pending() {
        let c = mem();
        let id = enqueue_conn(&c, "s1", "claude", "d1", "iPhone", "msg-a", 1000);
        mark_sent_conn(&c, id, 2000);
        assert!(
            pending_for_session_conn(&c, "s1").is_empty(),
            "已发 行不得再出现在 pending"
        );
        assert_eq!(next_pending_conn(&c, "s1"), None);
        let row = get_conn(&c, id).expect("mark_sent 后行应保留（审计痕迹）");
        assert_eq!(row.sent_at, Some(2000));
        assert_eq!(row.failed_reason, None);
    }

    /// mark_failed 后退出 pending，failed_reason 落库可观察
    #[test]
    fn mark_failed_records_reason() {
        let c = mem();
        let id = enqueue_conn(&c, "s1", "claude", "d1", "iPhone", "msg-a", 1000);
        mark_failed_conn(&c, id, "device offline");
        assert!(
            pending_for_session_conn(&c, "s1").is_empty(),
            "已败 行不得再出现在 pending"
        );
        let row = get_conn(&c, id).expect("mark_failed 后行应保留（审计痕迹）");
        assert_eq!(row.failed_reason.as_deref(), Some("device offline"));
        assert_eq!(row.sent_at, None, "失败行不得带 sent_at");
    }

    /// 撤回只认属主会话：跨会话 id 删不到返回 false，同会话删到返回 true
    #[test]
    fn retract_only_own_session() {
        let c = mem();
        let id1 = enqueue_conn(&c, "s1", "claude", "d1", "iPhone", "msg-a", 1000);
        let id2 = enqueue_conn(&c, "s2", "codex", "d1", "iPhone", "msg-b", 1001);
        // 跨会话撤回：拒绝且不误删
        assert!(!retract_conn(&c, "s2", id1), "s2 不得撤到 s1 的行");
        assert!(get_conn(&c, id1).is_some(), "跨会话撤回不得误删他人队列行");
        // 同会话撤回：删到
        assert!(retract_conn(&c, "s1", id1));
        assert!(get_conn(&c, id1).is_none());
        assert!(get_conn(&c, id2).is_some(), "他人行不受影响");
        // 重复撤回：已不存在，返回 false
        assert!(!retract_conn(&c, "s1", id1));
    }
}
