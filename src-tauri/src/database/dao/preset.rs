use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

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
