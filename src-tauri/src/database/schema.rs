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
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            scope       TEXT NOT NULL DEFAULT 'universal',
            bound_tool  TEXT,
            created_at  TEXT NOT NULL
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
        CREATE TABLE IF NOT EXISTS resource_bindings (
            extension_id    TEXT PRIMARY KEY,
            exclusive_tools TEXT NOT NULL,
            reason          TEXT,
            updated_at      TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_residents (
            tool_id       TEXT NOT NULL,
            extension_id  TEXT NOT NULL,
            PRIMARY KEY (tool_id, extension_id)
        );
        CREATE TABLE IF NOT EXISTS tool_base_snapshots (
            tool_id          TEXT PRIMARY KEY,
            active_preset_id TEXT,
            created_at       TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_base_snapshot_items (
            tool_id       TEXT NOT NULL,
            extension_id  TEXT NOT NULL,
            kind          TEXT NOT NULL,
            origin        TEXT NOT NULL,
            PRIMARY KEY (tool_id, extension_id)
        );
        CREATE TABLE IF NOT EXISTS stash_journal (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            tool_id       TEXT NOT NULL,
            skill_name    TEXT NOT NULL,
            stashed_path  TEXT NOT NULL,
            original_path TEXT NOT NULL,
            created_at    TEXT NOT NULL,
            restored_at   TEXT
        );
        CREATE TABLE IF NOT EXISTS unread_sessions (
            tool_id          TEXT NOT NULL,
            session_id       TEXT NOT NULL,
            project_name     TEXT NOT NULL DEFAULT '',
            title            TEXT,
            last_message     TEXT,
            turned_green_at  INTEGER NOT NULL,
            expires_at       INTEGER NOT NULL,
            PRIMARY KEY (tool_id, session_id)
        );
        CREATE TABLE IF NOT EXISTS unread_read_tombstones (
            tool_id     TEXT NOT NULL,
            session_id  TEXT NOT NULL,
            read_at     INTEGER NOT NULL,
            PRIMARY KEY (tool_id, session_id)
        );
        CREATE TABLE IF NOT EXISTS heartbeat_observations (
            pid           INTEGER PRIMARY KEY,
            tool_id       TEXT NOT NULL,
            session_id    TEXT NOT NULL,
            last_seen_at  INTEGER NOT NULL
        );
        -- T3（手工验收修复批）：审批等待持久标记——hook 审批进入事件写入、清除事件/会话
        -- 消失删除；状态链接入（adapter/mod.rs）据此强制 Waiting。不用 30s TTL 事件文件
        -- 承载等待态（事件文件只是触发器，等待是一等持久信号，issue #74 根因①）
        CREATE TABLE IF NOT EXISTS approval_wait_marks (
            tool_id       TEXT NOT NULL,
            session_id    TEXT NOT NULL,
            ts            INTEGER NOT NULL,
            summary       TEXT,
            PRIMARY KEY (tool_id, session_id)
        );
        CREATE TABLE IF NOT EXISTS inject_queue (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id    TEXT NOT NULL,
            agent_type    TEXT NOT NULL,
            device_id     TEXT NOT NULL,
            device_name   TEXT NOT NULL,
            content       TEXT NOT NULL,
            enqueued_at   INTEGER NOT NULL,
            sent_at       INTEGER,
            failed_reason TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_inject_queue_session ON inject_queue(session_id, id);
        CREATE TABLE IF NOT EXISTS write_audit (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            ts          INTEGER NOT NULL,
            device_id   TEXT NOT NULL,
            device_name TEXT NOT NULL,
            agent_type  TEXT NOT NULL,
            session_id  TEXT NOT NULL,
            channel     TEXT NOT NULL,
            action      TEXT NOT NULL,
            summary     TEXT NOT NULL,
            result      TEXT NOT NULL
        );
        "#,
    )
    .expect("Failed to initialize database schema");
}
