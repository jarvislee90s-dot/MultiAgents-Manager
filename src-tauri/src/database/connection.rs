use log::debug;
use once_cell::sync::Lazy;
use rusqlite::Connection;
use std::sync::Mutex;

/// 全局数据库连接（从 store.rs 搬移，保持原有模式）
pub static DB: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let db_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam");
    let _ = std::fs::create_dir_all(&db_dir);
    let db_path = db_dir.join("mam.db");
    let conn = Connection::open(&db_path).expect("Failed to open mam database");
    crate::database::schema::init(&conn);
    Mutex::new(conn)
});

/// 初始化数据库（在应用启动时调用）
pub fn init() {
    Lazy::force(&DB);
    debug!("Database initialized at ~/.mam/mam.db");
}

/// 打开新连接（少数场景使用）
pub fn open() -> Result<Connection, String> {
    let db_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("mam.db");
    Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))
}
