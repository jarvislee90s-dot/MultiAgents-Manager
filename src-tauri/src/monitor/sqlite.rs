// 工具私有 SQLite（opencode.db / workbuddy.db）只读访问公共设施（P1-4 消根）：
// 解析器只读查询工具自身数据库时统一走此 helper，避免各解析器各自复刻
// busy_timeout 模式（此前 opencode 有、workbuddy 无，锁竞争路径不可复现）。

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::Duration;

/// 以只读 + busy_timeout(1000) 打开工具数据库。
/// 只读避免与应用写入锁冲突；busy_timeout 防应用写入高峰期查询失败（与 opencode
/// 既有模式对齐）。打开失败 → None（调用方防御性降级，不 panic）
pub fn open_readonly_with_timeout(path: &Path) -> Option<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(Duration::from_millis(1000));
    Some(conn)
}
