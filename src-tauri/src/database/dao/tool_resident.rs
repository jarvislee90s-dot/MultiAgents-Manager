// 常驻名单（spec §3.4）：工具 × 资源豁免独占清扫；原生 MCP 段/自装插件定义上即常驻，不进本表
use rusqlite::params;

use crate::database::connection::DB;

pub fn set_tool_resident(tool_id: &str, extension_id: &str, resident: bool) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    if resident {
        conn.execute(
            "INSERT OR IGNORE INTO tool_residents (tool_id, extension_id) VALUES (?1, ?2)",
            params![tool_id, extension_id],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "DELETE FROM tool_residents WHERE tool_id = ?1 AND extension_id = ?2",
            params![tool_id, extension_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn is_tool_resident(tool_id: &str, extension_id: &str) -> bool {
    let conn = DB.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM tool_residents WHERE tool_id = ?1 AND extension_id = ?2",
        params![tool_id, extension_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

pub fn list_tool_residents(tool_id: &str) -> Vec<String> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT extension_id FROM tool_residents WHERE tool_id = ?1")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([tool_id], |row| row.get(0))
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default()
}

/// 删除某资源的全部常驻行（extension 修剪时防孤儿连带清理）
pub fn delete_tool_residents_for(extension_id: &str) -> Result<(), String> {
    let conn = DB.lock().unwrap();
    conn.execute(
        "DELETE FROM tool_residents WHERE extension_id = ?1",
        [extension_id],
    )
    .map_err(|e| e.to_string())
    .map(|_| ())
}
