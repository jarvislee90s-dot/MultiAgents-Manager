//! OpenCode 子 agent source（spec §4.2：活跃度判据，数据模型最优）。
//! `session_v2.parent_id` 非空即子 agent 会话；运行中 = time_updated 距 now < 90s
//! （「停止」即活跃度自然过期，无显式终止事件；time_idle 不用于判据——已完成会话
//! 亦为 NULL，spec §2.2）。SQLite 查询即最新：**无缓存**（豁免 L2/L3 预算纪律）。
//! 桶位映射：tokens_input→input、cache_read→cacheRead、cache_write→cacheCreation、
//! output→output；tokens_reasoning 不计入展示合计（§4.2）。

use super::{ms_to_iso, sort_views, SubagentView, TokenUsage, ACTIVE_SECS};
use std::path::Path;

/// 生产入口
pub fn collect(session_id: &str) -> Vec<SubagentView> {
    let Some(db) = dirs::home_dir().map(|h| {
        h.join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db")
    }) else {
        return Vec::new();
    };
    collect_with(&db, session_id, chrono::Utc::now().timestamp_millis())
}

/// 可注入核心（db 路径 / now 毫秒）
pub(crate) fn collect_with(db_path: &Path, session_id: &str, now_ms: i64) -> Vec<SubagentView> {
    let Some(conn) = crate::monitor::sqlite::open_readonly_with_timeout(db_path) else {
        return Vec::new();
    };
    // 2.x 判据单点（opencode_parser::schema_is_v2，禁止第二实现）
    if !crate::monitor::opencode_parser::schema_is_v2(&conn) {
        return Vec::new();
    }
    let sql = "SELECT id, agent, tokens_input, tokens_cache_read, tokens_cache_write, \
               tokens_output, time_created, time_updated FROM session_v2 \
               WHERE parent_id IS NOT NULL AND parent_id = ?1 ORDER BY time_created ASC";
    let Ok(mut stmt) = conn.prepare(sql) else {
        return Vec::new();
    };
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
        ))
    });
    let mut views = Vec::new();
    if let Ok(rows) = rows {
        for r in rows.flatten() {
            let (id, agent, tin, tcr, tcw, tout, created, updated) = r;
            // 活跃过滤（毫秒域严格小于；§4.2）
            if now_ms.saturating_sub(updated) >= (ACTIVE_SECS as i64) * 1000 {
                continue;
            }
            views.push(SubagentView {
                name: agent.unwrap_or_else(|| id.chars().take(8).collect()),
                id,
                description: None,
                spawn_ts: ms_to_iso(created),
                tokens: TokenUsage {
                    input: tin.max(0) as u64,
                    cache_read: tcr.max(0) as u64,
                    cache_creation: tcw.max(0) as u64,
                    output: tout.max(0) as u64,
                },
            });
        }
    }
    sort_views(&mut views);
    views
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// 建 v2 库（列齐全版——含 token 五桶与 parent_id；先例：opencode_parser tests）
    fn make_db(p: &Path) -> Connection {
        let conn = Connection::open(p).unwrap();
        conn.execute_batch(
            "CREATE TABLE session_v2 (id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, title TEXT,
                 parent_id TEXT, agent TEXT, tokens_input INTEGER DEFAULT 0, tokens_output INTEGER DEFAULT 0,
                 tokens_reasoning INTEGER DEFAULT 0, tokens_cache_read INTEGER DEFAULT 0,
                 tokens_cache_write INTEGER DEFAULT 0, time_created INTEGER, time_updated INTEGER,
                 time_idle INTEGER, idle_outcome TEXT, version TEXT);",
        )
        .unwrap();
        conn
    }
    #[allow(clippy::too_many_arguments)] // 测试夹具：九列直插，拆 struct 反而费读
    fn ins(
        conn: &Connection,
        id: &str,
        parent: Option<&str>,
        agent: Option<&str>,
        in_t: i64,
        out_t: i64,
        cr: i64,
        cw: i64,
        created: i64,
        updated: i64,
    ) {
        conn.execute(
            "INSERT INTO session_v2 (id, parent_id, agent, tokens_input, tokens_output,
                 tokens_cache_read, tokens_cache_write, time_created, time_updated)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![id, parent, agent, in_t, out_t, cr, cw, created, updated],
        )
        .unwrap();
    }

    /// 活跃子会话出板：parent_id 过滤 + 桶位映射（§4.2）+ agent 名 + spawnTs
    #[test]
    fn active_child_maps_and_filters() {
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("opencode.db");
        let conn = make_db(&db);
        let now = 2_000_000_000_000i64;
        ins(
            &conn,
            "main-1",
            None,
            None,
            0,
            0,
            0,
            0,
            now - 9_000_000,
            now,
        );
        ins(
            &conn,
            "child-1",
            Some("main-1"),
            Some("explore"),
            5124,
            9312,
            56448,
            7,
            now - 60_000,
            now - 30_000,
        );
        ins(
            &conn,
            "child-2",
            Some("main-1"),
            Some("general"),
            1,
            2,
            3,
            4,
            now - 30_000,
            now - 30_000,
        );
        ins(
            &conn,
            "orphan",
            None,
            Some("explore"),
            1,
            1,
            1,
            1,
            now - 1000,
            now,
        ); // parent_id NULL → 非子
        drop(conn);
        let v = collect_with(&db, "main-1", now);
        assert_eq!(v.len(), 2, "parent_id NULL 不算子 agent");
        assert_eq!(v[0].id, "child-1"); // time_created 升序
        assert_eq!(v[0].name, "explore");
        assert_eq!(
            v[0].tokens,
            TokenUsage {
                input: 5124,
                cache_read: 56448,
                cache_creation: 7,
                output: 9312
            }
        );
        assert_eq!(v[0].spawn_ts, ms_to_iso(now - 60_000));
        // 换会话查 → 空（parent 过滤）
        assert!(collect_with(&db, "main-2", now).is_empty());
    }

    /// 活跃度边界（spec §4.2 严格小于）：恰好 90s → 过期；89.999s → 活跃。
    /// 「停止」即活跃度自然过期，无需显式终止事件。
    #[test]
    fn active_boundary_expires_at_90s() {
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("opencode.db");
        let conn = make_db(&db);
        let now = 2_000_000_000_000i64;
        ins(
            &conn,
            "c90",
            Some("m"),
            Some("a"),
            0,
            0,
            0,
            0,
            now - 100_000,
            now - 90_000,
        );
        ins(
            &conn,
            "c89",
            Some("m"),
            Some("a"),
            0,
            0,
            0,
            0,
            now - 100_000,
            now - 89_999,
        );
        drop(conn);
        let v = collect_with(&db, "m", now);
        assert_eq!(v.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["c89"]);
    }

    /// 长静默子 agent 暂时隐藏（spec §8.3 如实口径）+ v1 库（无 session_v2）空态 + 库缺失空态
    #[test]
    fn stale_child_hidden_v1_and_missing_db_empty() {
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("opencode.db");
        let conn = make_db(&db);
        let now = 2_000_000_000_000i64;
        ins(
            &conn,
            "stale",
            Some("m"),
            Some("a"),
            0,
            0,
            0,
            0,
            now - 400_000,
            now - 200_000,
        );
        drop(conn);
        assert!(collect_with(&db, "m", now).is_empty(), "超窗隐藏");
        // v1：只有 session 表（无 session_v2）
        let v1db = td.path().join("v1.db");
        let c1 = Connection::open(&v1db).unwrap();
        c1.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT, title TEXT, time_updated INTEGER);",
        )
        .unwrap();
        drop(c1);
        assert!(
            collect_with(&v1db, "m", now).is_empty(),
            "1.x 库无 session_v2 → 空态（判据单点 schema_is_v2）"
        );
        assert!(collect_with(&td.path().join("missing.db"), "m", now).is_empty());
    }

    /// agent 名缺失 → 回落 id 前 8 位；tokens_reasoning 不入四桶（§4.2 申报）
    #[test]
    fn name_fallback_and_reasoning_excluded() {
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("opencode.db");
        let conn = make_db(&db);
        let now = 2_000_000_000_000i64;
        conn.execute(
            "INSERT INTO session_v2 (id, parent_id, agent, tokens_input, tokens_output, tokens_reasoning,
                 tokens_cache_read, tokens_cache_write, time_created, time_updated)
             VALUES ('abcdefgh1234','m',NULL,100,200,9999,50,10,?,?)",
            rusqlite::params![now - 1000, now],
        )
        .unwrap();
        drop(conn);
        let v = collect_with(&db, "m", now);
        assert_eq!(v[0].name, "abcdefgh");
        assert_eq!(
            v[0].tokens.total(),
            360,
            "reasoning 9999 不计入（100+50+10+200）"
        );
    }
}
