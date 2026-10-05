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

/// `?immutable=1` 只读打开：缺 `-wal/-shm` 的库 `mode=ro` 会 SQLITE_CANTOPEN(14)，
/// 必须走 immutable（zcode 762 MB 库即此形态）。**代价**：目标程序正在写时可能读到撕裂
/// 数据（§9.4）——用量采集对此可容忍（下一轮会补正），但**优先**仍走 mode=ro。
pub fn open_readonly_immutable(path: &Path) -> Option<Connection> {
    let uri = format!("file:{}?immutable=1", path.to_string_lossy());
    let conn = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(Duration::from_millis(1000));
    Some(conn)
}

/// 用量采集统一入口：先 `mode=ro`（安全），失败回退 `immutable=1`（能打开但可能撕裂）。
/// 两者都失败 → None（调用方按源失败处理，不影响其他源）。
///
/// **必须验「真能读」，不能只验「句柄开出来了」**（Task 14 实测的计划缺陷，见报告偏差申报）：
/// `sqlite3_open_v2` 是**惰性的**——缺 `-wal/-shm` 的 WAL 库在 `mode=ro` 下同样返回 `Ok`，
/// 真正的 `SQLITE_CANTOPEN(14)`（§9.4 的坑）要等到**第一句 SQL** 才抛出来
/// （本机 rusqlite 0.32 / 内置 sqlite 3.46 实测）。只 `or_else` 句柄的话这条回退**永不触发**，
/// 采集器拿到一把「查什么都报错」的连接，而它的每个查询点都是软失败
/// （`unwrap_or(false)` / `let ... else`）→ 该源**静默少算**（正是 GC 7 / W-06 那一族）。
pub fn open_usage_db(path: &Path) -> Option<Connection> {
    open_readonly_with_timeout(path)
        .filter(probe_readable)
        .or_else(|| open_readonly_immutable(path))
}

/// 廉价可读性探针：摸一下 `sqlite_master` 就能逼出惰性打开期藏起来的致命错
/// （`CANTOPEN` / `NOTADB` / 权限）。只读、零写入、不建表、不改既有函数行为。
///
/// **已知取舍（fix round 1 / 评审 Minor #1 后半，登记不改行为）**：探针把 `SQLITE_BUSY`
/// 也当成「打不开」→ 目标程序**正好持写锁**时（`busy_timeout(1000)` 用尽）本入口会
/// **静默降级到 `immutable=1`**，而 immutable 会**绕过 WAL** 读到 checkpoint 之前的旧值
/// （表现为「本轮数字回退一点」，下一轮 WAL 可读后补正）。取舍理由：① 用量采集对单轮滞后可容忍；
/// ② 反过来（BUSY 时不回退）会让该源**整轮失败**（`ok=false`），代价更大；
/// ③ 要区分二者就得把 `ErrorCode` 从探针里传出来，属 Task 15/16 需要时的加固项，本轮不做。
fn probe_readable(conn: &Connection) -> bool {
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .is_ok()
}
