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
/// 冲突时仅刷新展示快照（project_name/title/last_message）；
/// turned_green_at/expires_at 保留首次转绿值——spec §5 口径：turned_green_at＝转绿时间、
/// expires_at＝首绿 +24h 兜底，若随每轮轮询刷新会让 24h 窗口无限滑动、永不过期
pub fn upsert_unread(conn: &Connection, r: &UnreadSessionRecord) {
    let _ = conn.execute(
        "INSERT INTO unread_sessions
            (tool_id, session_id, project_name, title, last_message, turned_green_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(tool_id, session_id) DO UPDATE SET
            project_name = excluded.project_name,
            title = excluded.title,
            last_message = excluded.last_message",
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
/// review F1：仅刷新在场行的展示字段（UPDATE 无插入）——「跳转已读」删掉的行
/// 不得复活，转绿时间不得滑动。「持续绿色」路径专用；插入路径走 upsert_unread
pub fn refresh_display_unread(conn: &Connection, r: &UnreadSessionRecord) {
    let _ = conn.execute(
        "UPDATE unread_sessions
         SET project_name = ?3, title = ?4, last_message = ?5
         WHERE tool_id = ?1 AND session_id = ?2",
        params![
            r.tool_id,
            r.session_id,
            r.project_name,
            r.title,
            r.last_message
        ],
    );
}

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

pub fn refresh_display(r: &UnreadSessionRecord) {
    let conn = crate::database::connection::DB.lock().unwrap();
    refresh_display_unread(&conn, r);
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
        // 二次 upsert 仅刷新展示字段；turned_green_at/expires_at 保留首绿值
        // （spec §5：turned_green_at＝转绿时间，24h 兜底过期自首绿起算，禁止逐轮滑动）
        let mut updated = rec("workbuddy", "s1", 2000);
        updated.last_message = Some("新消息".into());
        updated.title = Some("新标题".into());
        upsert_unread(&conn, &updated);
        assert_eq!(list_unread(&conn, 3000).len(), 1);
        let row = &list_unread(&conn, 3000)[0];
        assert_eq!(row.turned_green_at_ms, 1000); // 首绿时间不被每轮轮询刷新
        assert_eq!(row.expires_at_ms, 1000 + 24 * 3600 * 1000); // 兜底过期随之固定
        assert_eq!(row.last_message.as_deref(), Some("新消息")); // 展示快照仍更新
        assert_eq!(row.title.as_deref(), Some("新标题"));
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

    /// review F1 回归锁：refresh_display 仅更新在场行的展示字段，
    /// 绝不复活「跳转已读」删掉的行（电平 upsert 会导致已读冲销）
    #[test]
    fn refresh_display_never_reinserts_deleted_row() {
        let conn = mem();
        let mut r = rec("workbuddy", "s1", 1000);
        upsert_unread(&conn, &r);
        delete_unread(&conn, "workbuddy", "s1"); // 跳转已读 → 删行
        r.title = Some("刷新后的标题".into());
        refresh_display_unread(&conn, &r);
        assert!(list_unread(&conn, 2000).is_empty(), "已删行不得复活");
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM unread_sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "已删行不得复活（物理行数）");
        // 在场行：仅刷新展示字段，转绿时间不得滑动
        upsert_unread(&conn, &rec("workbuddy", "s2", 5000));
        refresh_display_unread(&conn, &rec("workbuddy", "s2", 9999));
        let rows = list_unread(&conn, 10000);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].turned_green_at_ms, 5000,
            "刷新展示字段不得滑动转绿时间"
        );
    }
}
