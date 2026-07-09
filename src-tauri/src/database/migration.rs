use rusqlite::Connection;

pub fn migrate(conn: &Connection) -> Result<(), String> {
    // 检查 extension 表是否有 manifest_path 列
    let has_manifest_path = conn.prepare("SELECT manifest_path FROM extensions LIMIT 0").is_ok();
    if !has_manifest_path {
        conn.execute_batch(
            "ALTER TABLE extensions ADD COLUMN manifest_path TEXT;
             ALTER TABLE extensions ADD COLUMN permissions TEXT;
             ALTER TABLE extensions ADD COLUMN min_runtime TEXT;"
        ).map_err(|e| format!("迁移失败: {}", e))?;
    }
    Ok(())
}
