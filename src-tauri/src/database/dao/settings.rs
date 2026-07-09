use rusqlite::params;

use crate::database::connection::DB;

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
