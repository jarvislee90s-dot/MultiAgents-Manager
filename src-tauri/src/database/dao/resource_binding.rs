// 专属绑定（spec §3.4/§4）：资源 → 允许工具列表（空 = 通用可迁移），手动为真值源
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBindingRecord {
    pub extension_id: String,
    pub exclusive_tools: String,
    pub reason: Option<String>,
    pub updated_at: String,
}

pub fn upsert_resource_binding(
    extension_id: &str,
    exclusive_tools: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR REPLACE INTO resource_bindings (extension_id, exclusive_tools, reason, updated_at) VALUES (?1, ?2, ?3, ?4)",
        params![extension_id, exclusive_tools, reason, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_resource_binding(extension_id: &str) -> Option<ResourceBindingRecord> {
    let conn = DB.lock().unwrap();
    conn.query_row(
        "SELECT extension_id, exclusive_tools, reason, updated_at FROM resource_bindings WHERE extension_id = ?1",
        [extension_id],
        |row| {
            Ok(ResourceBindingRecord {
                extension_id: row.get(0)?,
                exclusive_tools: row.get(1)?,
                reason: row.get(2)?,
                updated_at: row.get(3)?,
            })
        },
    )
    .ok()
}

pub fn delete_resource_binding(extension_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    conn.execute("DELETE FROM resource_bindings WHERE extension_id = ?1", [extension_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_resource_bindings() -> Vec<ResourceBindingRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT extension_id, exclusive_tools, reason, updated_at FROM resource_bindings")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(ResourceBindingRecord {
                    extension_id: row.get(0)?,
                    exclusive_tools: row.get(1)?,
                    reason: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}

/// 工具是否可用该资源：无绑定或绑定列表为空 → 通用放行
pub fn tool_allowed(extension_id: &str, tool_id: &str) -> bool {
    match get_resource_binding(extension_id) {
        None => true,
        Some(b) => {
            let list = b.exclusive_tools.trim();
            list.is_empty() || list.split(',').any(|t| t.trim() == tool_id)
        }
    }
}
