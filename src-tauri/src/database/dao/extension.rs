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

pub fn upsert_assignment_with_subagent(
    ext_id: &str,
    tool_id: &str,
    sub_agent_id: &str,
    enabled: bool,
    link_status: &str,
) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let id = format!("{}-{}-{}", ext_id, tool_id, sub_agent_id);
    conn.execute(
        "INSERT OR REPLACE INTO extension_assignments (id, extension_id, agent_tool_id, sub_agent_id, enabled, link_status, assigned_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, ext_id, tool_id, sub_agent_id, enabled as i64, link_status, now],
    ).map_err(|e| e.to_string()).map(|_| ())
}

pub fn upsert_assignment(
    ext_id: &str,
    tool_id: &str,
    enabled: bool,
    link_status: &str,
) -> Result<(), String> {
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

pub fn disable_subagent_assignment(
    ext_id: &str,
    tool_id: &str,
    sub_agent_id: &str,
) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let id = format!("{}-{}-{}", ext_id, tool_id, sub_agent_id);
    conn.execute(
        "UPDATE extension_assignments SET enabled = 0, link_status = 'missing' WHERE id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn delete_extension(ext_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    delete_extension_on(&conn, ext_id)
}

pub fn delete_extension_on(conn: &rusqlite::Connection, ext_id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM extensions WHERE id = ?1", params![ext_id])
        .map_err(|e| e.to_string())
        .map(|_| ())
}

/// 删除某资源的全部 assignment（含子 Agent 维度）
pub fn delete_assignments_for(ext_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    delete_assignments_for_on(&conn, ext_id)
}

pub fn delete_assignments_for_on(conn: &rusqlite::Connection, ext_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM extension_assignments WHERE extension_id = ?1",
        params![ext_id],
    )
    .map_err(|e| e.to_string())
    .map(|_| ())
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use crate::database::schema;

    fn mem_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        schema::init(&conn);
        conn
    }

    #[test]
    fn delete_extension_removes_row() {
        let conn = mem_conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO extensions (id, kind, name, source_path, installed_at, updated_at) \
             VALUES ('skill-x','skill','x','/tmp/x',?1,?1)",
            [&now],
        )
        .unwrap();
        delete_extension_on(&conn, "skill-x").unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM extensions WHERE id='skill-x'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn delete_assignments_covers_subagent_rows() {
        let conn = mem_conn();
        let now = chrono::Utc::now().to_rfc3339();
        conn
            .execute(
                "INSERT INTO extension_assignments \
                 (id, extension_id, agent_tool_id, sub_agent_id, enabled, link_status, assigned_at) VALUES \
                 ('a','skill-x','claude',NULL,1,'valid',?1),\
                 ('b','skill-x','claude','sub1',1,'valid',?1)",
                [&now],
            )
            .unwrap();
        delete_assignments_for_on(&conn, "skill-x").unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM extension_assignments WHERE extension_id='skill-x'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }
}
