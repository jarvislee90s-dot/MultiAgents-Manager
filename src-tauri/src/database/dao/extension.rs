use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

// ===== 扩展资源 =====

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

pub fn disable_subagent_assignment(ext_id: &str, tool_id: &str, sub_agent_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let id = format!("{}-{}-{}", ext_id, tool_id, sub_agent_id);
    conn.execute(
        "UPDATE extension_assignments SET enabled = 0, link_status = 'missing' WHERE id = ?1",
        params![id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

// ===== 原生扩展资源 =====

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
