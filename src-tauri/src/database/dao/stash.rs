// 暂存日志（spec §3.4/§5.3）：暂存区唯一账本，崩溃恢复依据
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntryRecord {
    pub id: i64,
    pub tool_id: String,
    pub skill_name: String,
    pub stashed_path: String,
    pub original_path: String,
    pub created_at: String,
    pub restored_at: Option<String>,
}

pub fn record_stash(
    tool_id: &str,
    skill_name: &str,
    stashed_path: &str,
    original_path: &str,
) -> Result<i64, String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO stash_journal (tool_id, skill_name, stashed_path, original_path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![tool_id, skill_name, stashed_path, original_path, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

pub fn mark_stash_restored(id: i64) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE stash_journal SET restored_at = ?2 WHERE id = ?1",
        params![id, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 未恢复条目；tool_id = None 时跨全部工具（启动孤儿扫描）
pub fn unrestored_stash(tool_id: Option<&str>) -> Vec<StashEntryRecord> {
    let conn = DB.lock().unwrap();
    let sql = match tool_id {
        Some(_) => "SELECT id, tool_id, skill_name, stashed_path, original_path, created_at, restored_at FROM stash_journal WHERE restored_at IS NULL AND tool_id = ?1",
        None => "SELECT id, tool_id, skill_name, stashed_path, original_path, created_at, restored_at FROM stash_journal WHERE restored_at IS NULL",
    };
    conn.prepare(sql)
        .ok()
        .and_then(|mut stmt| {
            let map_row = |row: &rusqlite::Row| {
                Ok(StashEntryRecord {
                    id: row.get(0)?,
                    tool_id: row.get(1)?,
                    skill_name: row.get(2)?,
                    stashed_path: row.get(3)?,
                    original_path: row.get(4)?,
                    created_at: row.get(5)?,
                    restored_at: row.get(6)?,
                })
            };
            let rows = match tool_id {
                Some(t) => stmt.query_map([t], map_row),
                None => stmt.query_map([], map_row),
            };
            rows.ok().map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}
