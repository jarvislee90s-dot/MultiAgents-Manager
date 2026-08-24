use log::debug;
use once_cell::sync::Lazy;
use rusqlite::Connection;
use std::sync::Mutex;

/// 应用数据主目录：优先取 MAM_HOME 环境变量（测试重定向用），否则用 dirs::home_dir()
/// Windows 下 dirs::home_dir 指向真实用户目录且无法用 HOME 重定向，故提供专用覆盖变量
fn app_data_home() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("MAM_HOME") {
        if !home.is_empty() {
            return std::path::PathBuf::from(home);
        }
    }
    dirs::home_dir().unwrap_or_default()
}

/// 全局数据库连接（从 store.rs 搬移，保持原有模式）
pub static DB: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let db_dir = app_data_home().join(".mam");
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
    let db_path = app_data_home()
        .join(".mam")
        .join("mam.db");
    Connection::open(&db_path).map_err(|e| format!("打开数据库失败: {}", e))
}
