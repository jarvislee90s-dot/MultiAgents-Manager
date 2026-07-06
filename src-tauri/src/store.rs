// SQLite 数据层 — 会话状态缓存（通知去重）+ 设置读写
// 数据库路径：~/.mam/mam.db

use log::debug;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::sync::Mutex;

static DB: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let db_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam");
    let _ = std::fs::create_dir_all(&db_dir);
    let db_path = db_dir.join("mam.db");
    let conn = Connection::open(&db_path).expect("Failed to open mam database");
    init_schema(&conn);
    Mutex::new(conn)
});

fn init_schema(conn: &Connection) {
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
            updated_at  TEXT NOT NULL
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

/// 初始化数据库（在应用启动时调用）
pub fn init() {
    Lazy::force(&DB);
    debug!("Database initialized at ~/.mam/mam.db");
}

/// 更新会话状态，返回是否状态发生了变化（用于通知去重）
/// 返回 Some(previous_status) 表示状态从 previous 变为当前值
/// 返回 None 表示首次记录或状态未变
pub fn update_session_status(
    session_id: &str,
    agent_type: &str,
    status: &str,
) -> Option<String> {
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
        None // 状态未变
    } else {
        previous
    }
}

/// 清理不再活跃的会话缓存
pub fn cleanup_stale_sessions(active_ids: &HashSet<String>) {
    let conn = DB.lock().unwrap();
    let all: Vec<String> = conn
        .prepare("SELECT session_id FROM session_status_cache")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    for id in &all {
        if !active_ids.contains(id) {
            let _ = conn.execute(
                "DELETE FROM session_status_cache WHERE session_id = ?",
                [id],
            );
        }
    }
}

/// 读取设置
pub fn get_setting(key: &str) -> Option<String> {
    let conn = DB.lock().unwrap();
    conn.query_row("SELECT value FROM settings WHERE key = ?", [key], |row| {
        row.get(0)
    })
    .ok()
}

/// 写入设置
pub fn set_setting(key: &str, value: &str) {
    let conn = DB.lock().unwrap();
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    );
}

// ===== 扩展资源 CRUD =====

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub source_path: String,
    pub source_url: Option<String>,
    pub version: Option<String>,
    pub tags: Option<String>,
    pub suite: Option<String>,
    pub source_tool: Option<String>,
    pub is_native: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentRecord {
    pub id: String,
    pub extension_id: String,
    pub agent_tool_id: String,
    pub sub_agent_id: Option<String>,
    pub enabled: bool,
    pub link_status: String,
}

pub fn insert_extension(ext: &ExtensionRecord) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO extensions (id, kind, name, description, source_path, source_url, version, tags, suite, source_tool, is_native, installed_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![ext.id, ext.kind, ext.name, ext.description, ext.source_path, ext.source_url, ext.version, ext.tags, ext.suite, ext.source_tool, ext.is_native as i64, &now, &now],
    ).map_err(|e| e.to_string()).map(|_| ())
}

pub fn list_extensions() -> Vec<ExtensionRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT id, kind, name, description, source_path, source_url, version, tags, suite, source_tool, is_native FROM extensions")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(ExtensionRecord {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    description: row.get(3)?,
                    source_path: row.get(4)?,
                    source_url: row.get(5)?,
                    version: row.get(6)?,
                    tags: row.get(7)?,
                    suite: row.get(8)?,
                    source_tool: row.get(9)?,
                    is_native: row.get::<_, i64>(10)? != 0,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

pub fn list_assignments(tool_id: &str) -> Vec<AssignmentRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT id, extension_id, agent_tool_id, sub_agent_id, enabled, link_status FROM extension_assignments WHERE agent_tool_id = ?1")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([tool_id], |row| {
                Ok(AssignmentRecord {
                    id: row.get(0)?,
                    extension_id: row.get(1)?,
                    agent_tool_id: row.get(2)?,
                    sub_agent_id: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    link_status: row.get(5)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

pub fn upsert_assignment_with_subagent(ext_id: &str, tool_id: &str, sub_agent_id: &str, enabled: bool, link_status: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}-{}-{}", ext_id, tool_id, sub_agent_id);
    conn.execute(
        "INSERT OR REPLACE INTO extension_assignments (id, extension_id, agent_tool_id, sub_agent_id, enabled, link_status, assigned_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, ext_id, tool_id, sub_agent_id, enabled as i64, link_status, now],
    ).map_err(|e| e.to_string()).map(|_| ())
}

pub fn upsert_assignment(ext_id: &str, tool_id: &str, enabled: bool, link_status: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}-{}", ext_id, tool_id);
    conn.execute(
        "INSERT OR REPLACE INTO extension_assignments (id, extension_id, agent_tool_id, enabled, link_status, assigned_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, ext_id, tool_id, enabled as i64, link_status, now],
    ).map_err(|e| e.to_string()).map(|_| ())
}
pub fn list_all_assignments() -> Vec<AssignmentRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT id, extension_id, agent_tool_id, sub_agent_id, enabled, link_status FROM extension_assignments")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(AssignmentRecord {
                    id: row.get(0)?,
                    extension_id: row.get(1)?,
                    agent_tool_id: row.get(2)?,
                    sub_agent_id: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    link_status: row.get(5)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

// ===== 预设组 CRUD =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetRecord {
    pub id: String,
    pub name: String,
    pub items: Vec<PresetItemRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetItemRecord {
    pub extension_id: String,
    pub kind: String,
    pub extension_name: String,
}

pub fn create_preset(name: &str, items: &[(String, String)]) -> Result<String, String> {
    let conn = DB.lock().unwrap();
    let id = format!("preset-{}", chrono::Utc::now().timestamp_millis());
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO presets (id, name, created_at) VALUES (?1, ?2, ?3)",
        params![id, name, now],
    ).map_err(|e| e.to_string())?;
    for (ext_id, kind) in items {
        let item_id = format!("{}-{}", id, ext_id);
        conn.execute(
            "INSERT INTO preset_items (id, preset_id, extension_id, kind) VALUES (?1, ?2, ?3, ?4)",
            params![item_id, id, ext_id, kind],
        ).map_err(|e| e.to_string())?;
    }
    Ok(id)
}

pub fn list_presets() -> Vec<PresetRecord> {
    let conn = DB.lock().unwrap();
    let preset_ids: Vec<(String, String)> = conn
        .prepare("SELECT id, name FROM presets ORDER BY created_at DESC")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    preset_ids.iter().map(|(id, name)| {
        let items: Vec<PresetItemRecord> = conn
            .prepare("SELECT pi.extension_id, pi.kind, e.name FROM preset_items pi LEFT JOIN extensions e ON pi.extension_id = e.id WHERE pi.preset_id = ?1")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([id], |row| {
                    Ok(PresetItemRecord {
                        extension_id: row.get(0)?,
                        kind: row.get(1)?,
                        extension_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
            })
            .unwrap_or_default();
        PresetRecord { id: id.clone(), name: name.clone(), items }
    }).collect()
}

pub fn delete_preset(preset_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    conn.execute("DELETE FROM preset_items WHERE preset_id = ?1", [preset_id]).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM preset_applications WHERE preset_id = ?1", [preset_id]).map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM presets WHERE id = ?1", [preset_id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_preset_items(preset_id: &str) -> Vec<(String, String)> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT extension_id, kind FROM preset_items WHERE preset_id = ?1")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([preset_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

pub fn record_preset_application(preset_id: &str, tool_id: &str, active: bool) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}-{}", preset_id, tool_id);
    conn.execute(
        "INSERT OR REPLACE INTO preset_applications (id, preset_id, agent_tool_id, applied_at, active) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, preset_id, tool_id, now, active as i64],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_preset_application_subagent(preset_id: &str, tool_id: &str, sub_agent_id: &str, active: bool) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}-{}-{}", preset_id, tool_id, sub_agent_id);
    conn.execute(
        "INSERT OR REPLACE INTO preset_applications (id, preset_id, agent_tool_id, sub_agent_id, applied_at, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, preset_id, tool_id, sub_agent_id, now, active as i64],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// 禁用子 Agent 的指定 extension 分配记录
pub fn disable_subagent_assignment(ext_id: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let id = format!("{}-{}-{}", ext_id, tool_id, sub_agent_id);
    conn.execute(
        "UPDATE extension_assignments SET enabled = 0, link_status = 'missing' WHERE id = ?1",
        params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

// ===== 子 Agent CRUD =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRecord {
    pub id: String,
    pub name: String,
    pub agent_tool_id: String,
    pub config_path: String,
    pub format: String,
}

pub fn list_sub_agents(tool_id: &str) -> Vec<SubAgentRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT id, name, agent_tool_id, config_path, format FROM sub_agents WHERE agent_tool_id = ?1")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([tool_id], |row| {
                Ok(SubAgentRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    agent_tool_id: row.get(2)?,
                    config_path: row.get(3)?,
                    format: row.get(4)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}

// ===== 原生扩展资源 CRUD =====

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeExtensionRecord {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: Option<String>,
    pub source_path: String,
    pub source_tool: String,
    pub detected_at: String,
    pub imported: bool,
}

pub fn insert_native_extension(ext: &NativeExtensionRecord) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO native_extensions (id, kind, name, description, source_path, source_tool, detected_at, imported) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![ext.id, ext.kind, ext.name, ext.description, ext.source_path, ext.source_tool, ext.detected_at, ext.imported as i64],
    ).map_err(|e| e.to_string()).map(|_| ())
}

pub fn list_native_extensions(tool_id: Option<&str>) -> Vec<NativeExtensionRecord> {
    let conn = DB.lock().unwrap();
    let result = if let Some(tool) = tool_id {
        conn.prepare("SELECT id, kind, name, description, source_path, source_tool, detected_at, imported FROM native_extensions WHERE source_tool = ?1 AND imported = 0 ORDER BY detected_at DESC")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([tool], |row| {
                    Ok(NativeExtensionRecord {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        description: row.get(3)?,
                        source_path: row.get(4)?,
                        source_tool: row.get(5)?,
                        detected_at: row.get(6)?,
                        imported: row.get::<_, i64>(7)? != 0,
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
            })
            .unwrap_or_default()
    } else {
        conn.prepare("SELECT id, kind, name, description, source_path, source_tool, detected_at, imported FROM native_extensions WHERE imported = 0 ORDER BY detected_at DESC")
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok(NativeExtensionRecord {
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        name: row.get(2)?,
                        description: row.get(3)?,
                        source_path: row.get(4)?,
                        source_tool: row.get(5)?,
                        detected_at: row.get(6)?,
                        imported: row.get::<_, i64>(7)? != 0,
                    })
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default()
            })
            .unwrap_or_default()
    };
    result
}

pub fn mark_native_imported(ids: &[String]) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    for id in ids {
        conn.execute(
            "UPDATE native_extensions SET imported = 1 WHERE id = ?1",
            params![id],
        ).map_err(|e| e.to_string())?;
    }
    Ok(())
}
