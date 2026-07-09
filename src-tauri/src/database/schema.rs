use rusqlite::Connection;

/// 初始化数据库 schema（所有 CREATE TABLE 语句）
pub fn init(conn: &Connection) {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS session_status_cache (
            session_id    TEXT PRIMARY KEY,
            agent_type    TEXT NOT NULL,
            status        TEXT NOT NULL,
            last_seen     TEXT NOT NULL,
            previous_status TEXT
        );
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS extensions (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,
            name        TEXT NOT NULL,
            description TEXT,
            source_path TEXT NOT NULL,
            source_url  TEXT,
            version     TEXT,
            tags        TEXT,
            suite       TEXT,
            source_tool TEXT,
            is_native   INTEGER NOT NULL DEFAULT 0,
            installed_at TEXT NOT NULL,
            updated_at  TEXT NOT NULL,
            manifest_path TEXT,
            permissions TEXT,
            min_runtime TEXT
        );
        CREATE TABLE IF NOT EXISTS extension_assignments (
            id            TEXT PRIMARY KEY,
            extension_id  TEXT NOT NULL,
            agent_tool_id TEXT NOT NULL,
            sub_agent_id  TEXT,
            enabled       INTEGER NOT NULL DEFAULT 1,
            link_status   TEXT NOT NULL DEFAULT 'missing',
            assigned_at   TEXT NOT NULL,
            UNIQUE(extension_id, agent_tool_id, sub_agent_id)
        );
        CREATE TABLE IF NOT EXISTS agent_tools (
            id                TEXT PRIMARY KEY,
            name              TEXT NOT NULL,
            process_name      TEXT NOT NULL,
            base_dir          TEXT NOT NULL,
            hook_supported    INTEGER NOT NULL DEFAULT 0,
            hook_event_case   TEXT NOT NULL DEFAULT 'none',
            mcp_format        TEXT NOT NULL DEFAULT 'json',
            detected          INTEGER NOT NULL DEFAULT 0,
            enabled           INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS sub_agents (
            id            TEXT PRIMARY KEY,
            name          TEXT NOT NULL,
            agent_tool_id TEXT NOT NULL,
            config_path   TEXT NOT NULL,
            format        TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS presets (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS preset_items (
            id           TEXT PRIMARY KEY,
            preset_id    TEXT NOT NULL,
            extension_id TEXT NOT NULL,
            kind         TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS preset_applications (
            id            TEXT PRIMARY KEY,
            preset_id     TEXT NOT NULL,
            agent_tool_id TEXT NOT NULL,
            sub_agent_id  TEXT,
            applied_at    TEXT NOT NULL,
            active        INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS native_extensions (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,
            name        TEXT NOT NULL,
            description TEXT,
            source_path TEXT NOT NULL,
            source_tool TEXT NOT NULL,
            detected_at TEXT NOT NULL,
            imported    INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .expect("Failed to initialize database schema");
}
