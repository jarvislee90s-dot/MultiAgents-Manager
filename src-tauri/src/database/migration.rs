use rusqlite::Connection;

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("SELECT {} FROM {} LIMIT 0", column, table))
        .is_ok()
}

pub fn migrate(conn: &Connection) -> Result<(), String> {
    // 检查 extension 表是否有 manifest_path 列
    if !column_exists(conn, "extensions", "manifest_path") {
        conn.execute("ALTER TABLE extensions ADD COLUMN manifest_path TEXT", [])
            .map_err(|e| format!("迁移失败: {}", e))?;
    }
    if !column_exists(conn, "extensions", "permissions") {
        conn.execute("ALTER TABLE extensions ADD COLUMN permissions TEXT", [])
            .map_err(|e| format!("迁移失败: {}", e))?;
    }
    if !column_exists(conn, "extensions", "min_runtime") {
        conn.execute("ALTER TABLE extensions ADD COLUMN min_runtime TEXT", [])
            .map_err(|e| format!("迁移失败: {}", e))?;
    }

    // 旧版 extensions 表没有 suite/source_tool/is_native，会导致资源列表和补链读不到历史数据
    if !column_exists(conn, "extensions", "suite") {
        conn.execute("ALTER TABLE extensions ADD COLUMN suite TEXT", [])
            .map_err(|e| format!("迁移扩展资源字段失败: {}", e))?;
    }
    if !column_exists(conn, "extensions", "source_tool") {
        conn.execute("ALTER TABLE extensions ADD COLUMN source_tool TEXT", [])
            .map_err(|e| format!("迁移扩展资源字段失败: {}", e))?;
    }
    if !column_exists(conn, "extensions", "is_native") {
        conn.execute(
            "ALTER TABLE extensions ADD COLUMN is_native INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("迁移扩展资源字段失败: {}", e))?;
    }

    // 根据旧表记录回填来源工具
    conn.execute_batch(
        "UPDATE extensions SET source_tool = 'claude'
            WHERE kind = 'skill' AND source_tool IS NULL
              AND (source_path LIKE '%/.claude/skills/%' OR tags = 'claude');
         UPDATE extensions SET source_tool = 'codex'
            WHERE kind = 'skill' AND source_tool IS NULL
              AND (source_path LIKE '%/.codex/skills/%'
                   OR source_path LIKE '%/.agents/skills/%'
                   OR tags = 'codex');
         UPDATE extensions SET source_tool = 'opencode'
            WHERE kind = 'skill' AND source_tool IS NULL
              AND (source_path LIKE '%/.config/opencode/skills/%' OR tags = 'opencode');
         UPDATE extensions SET source_tool = 'openclaw'
            WHERE kind = 'skill' AND source_tool IS NULL
              AND (source_path LIKE '%/.openclaw/skills/%' OR tags = 'openclaw');",
    )
    .map_err(|e| format!("回填来源工具失败: {}", e))?;

    // 015：native_extensions 表从未被业务写入，移除（历史库中 DROP）
    conn.execute_batch("DROP TABLE IF EXISTS native_extensions;")
        .map_err(|e| format!("移除 native_extensions 失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_adds_skill_source_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE extensions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                source_path TEXT NOT NULL,
                source_url TEXT,
                version TEXT,
                tags TEXT,
                installed_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert!(conn
            .prepare("SELECT source_tool FROM extensions LIMIT 0")
            .is_ok());
        assert!(conn
            .prepare("SELECT is_native FROM extensions LIMIT 0")
            .is_ok());
    }

    #[test]
    fn migrate_backfills_source_tool_from_path() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE extensions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                source_path TEXT NOT NULL,
                source_url TEXT,
                version TEXT,
                tags TEXT,
                installed_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO extensions (id, kind, name, source_path, tags, installed_at, updated_at)
            VALUES ('skill-old', 'skill', 'old', '/Users/test/.agents/skills/old', 'codex', 'now', 'now');"
        ).unwrap();

        migrate(&conn).unwrap();

        let source_tool: Option<String> = conn
            .query_row(
                "SELECT source_tool FROM extensions WHERE id = 'skill-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_tool.as_deref(), Some("codex"));
    }

    #[test]
    fn migrate_drops_native_extensions() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE extensions (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                source_path TEXT NOT NULL,
                source_url TEXT,
                version TEXT,
                tags TEXT,
                installed_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE native_extensions (id TEXT PRIMARY KEY);",
        )
        .unwrap();
        migrate(&conn).unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='native_extensions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!exists);
    }
}
