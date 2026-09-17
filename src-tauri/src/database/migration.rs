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

    // M2（远程接入）：已配对设备表（cookie deviceId 跨重启持久）
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS remote_devices (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL DEFAULT '',
             ua TEXT NOT NULL DEFAULT '',
             origin_ip TEXT NOT NULL DEFAULT '',
             fingerprint TEXT NOT NULL DEFAULT '',
             via TEXT NOT NULL DEFAULT '',
             first_paired_at INTEGER NOT NULL DEFAULT 0,
             last_seen_at INTEGER NOT NULL DEFAULT 0,
             revoked INTEGER NOT NULL DEFAULT 0
         );",
    )
    .map_err(|e| format!("建 remote_devices 失败: {}", e))?;

    // M5 A1：设备指纹（sha256(UA|"|"|origin_ip) hex 小写，同浏览器重绑 upsert 的去重键）
    // 与接入通道（ASCII 枚举 "local"|"lan"|"quick"|"named"）两列——
    // 存量库走 ALTER（新库已由上方建表语句直建，column_exists 短路）。
    // SQLite 加 NOT NULL 列必须带 DEFAULT。存量行取 DEFAULT 空串：指纹算法在 Rust 侧
    // 无法在 SQL 内回填，空串也永不与真实指纹撞键（只影响首次重连多一行，可接受）
    if !column_exists(conn, "remote_devices", "fingerprint") {
        conn.execute(
            "ALTER TABLE remote_devices ADD COLUMN fingerprint TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|e| format!("迁移设备指纹列失败: {}", e))?;
    }
    if !column_exists(conn, "remote_devices", "via") {
        conn.execute(
            "ALTER TABLE remote_devices ADD COLUMN via TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|e| format!("迁移设备 via 列失败: {}", e))?;
    }

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

    /// M5 A1：旧结构 remote_devices（M4，无 fingerprint/via）经迁移后两列存在，
    /// 存量行取 DEFAULT 空串（无法回填指纹——算法在 Rust 侧，且空串永不与真实指纹撞键）
    #[test]
    fn migrate_adds_remote_device_fingerprint_and_via_columns() {
        let conn = Connection::open_in_memory().unwrap();
        // migrate 首段是 ALTER TABLE extensions——沿用本文件既有测试做法先建 extensions
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
        // 旧结构表：M4 原始形状（故意不含两新列），并预置一行存量数据
        conn.execute_batch(
            "CREATE TABLE remote_devices (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                ua TEXT NOT NULL DEFAULT '',
                origin_ip TEXT NOT NULL DEFAULT '',
                first_paired_at INTEGER NOT NULL DEFAULT 0,
                last_seen_at INTEGER NOT NULL DEFAULT 0,
                revoked INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO remote_devices (id, name) VALUES ('legacy', '旧设备');",
        )
        .unwrap();
        migrate(&conn).unwrap();
        assert!(conn
            .prepare("SELECT fingerprint FROM remote_devices LIMIT 0")
            .is_ok());
        assert!(conn
            .prepare("SELECT via FROM remote_devices LIMIT 0")
            .is_ok());
        // 幂等：迁移两遍不报错（column_exists 短路 ALTER）
        migrate(&conn).unwrap();
        // 存量行新列为 DEFAULT 空串
        let (fp, via): (String, String) = conn
            .query_row(
                "SELECT fingerprint, via FROM remote_devices WHERE id = 'legacy'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((fp.as_str(), via.as_str()), ("", ""));
    }

    /// M5 A1：全新库（schema::init → migrate）建出的 remote_devices 直含两列，
    /// 无需走 ALTER 路径
    #[test]
    fn fresh_remote_devices_table_includes_fingerprint_and_via() {
        let conn = Connection::open_in_memory().unwrap();
        // 真机调用序：schema::init 建全部表 → migration::migrate 增量迁移
        crate::database::schema::init(&conn);
        migrate(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(remote_devices)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            cols.contains(&"fingerprint".to_string()),
            "缺 fingerprint 列: {cols:?}"
        );
        assert!(cols.contains(&"via".to_string()), "缺 via 列: {cols:?}");
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
