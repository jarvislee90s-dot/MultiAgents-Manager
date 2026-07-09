use rusqlite::Connection;

/// 数据库迁移逻辑（预留，当前 schema 使用 IF NOT EXISTS 自动兼容）
pub fn migrate(_conn: &Connection) -> Result<(), String> {
    Ok(())
}
