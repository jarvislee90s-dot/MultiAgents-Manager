// 设备持久化与 cookie 拼装（M5 A3 起只剩 DAO 侧）——原直通 token 状态机
// （PairingService/PairingClock）随密码制下线，配对入口见 api::pair_pin

use crate::remote::gate::COOKIE_NAME as COOKIE;
use rusqlite::OptionalExtension;

/// 设备 cookie 有效期：180 天（dsh 设备会话 TTL，cookie Max-Age 同源）
pub const DEVICE_TTL_MS: i64 = 180 * 24 * 3600 * 1000;

/// 待持久化的新设备（配对成功产物）
pub struct NewDevice {
    pub id: String,
    pub name: String,
    pub ua: String,
    pub origin_ip: String,
    /// 接入通道：ASCII 枚举 `"local" | "lan" | "quick" | "named"`（花名册徽标数据源，
    /// 前端 Task A6 再映射中文）。运行时判定（回环 + Host 快照）属 Task A3——
    /// 在其接入前调用方暂传空串占位（与列 DEFAULT 一致，表示"尚未判定"）
    pub via: String,
    pub paired_at: i64,
}

/// 设备指纹（纯函数）：sha256(`ua` + "|" + `origin_ip`) 的 hex 小写。
/// 收口在 DAO 内部调用——调用方只传 ua/origin_ip，拼接格式由本函数唯一锁定
/// （固定向量测试 `fingerprint_for_is_deterministic_and_concatenation_locked` 防漂移）
pub fn fingerprint_for(ua: &str, origin_ip: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut buf = String::with_capacity(ua.len() + 1 + origin_ip.len());
    buf.push_str(ua);
    buf.push('|');
    buf.push_str(origin_ip);
    Sha256::digest(buf.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 持久化设备（M5 A1：upsert-by-fingerprint——同浏览器同一网络重绑**覆盖不新增**）。
/// - 按指纹查**未吊销**行 → 命中：覆盖 ua / origin_ip / last_seen_at / via
///   （via 随重连刷新——M5 实锤定通道 2026-09-18 裁决：via = 最近一次接入的通道，
///   设备换通道接入时徽标自动跟上；**保留原 id 与原 name**（含用户重命名——命中
///   路径完全不更新 name）、保留 first_paired_at，revoked 保持 0）；
///   返回原行 id（调用方下发 cookie 必须用它，否则命中路径的 cookie 指向不存在的行）。
/// - 未命中，或命中行已吊销 → 新建行（**吊销行不复活**），name 取设备自报，返回新 id。
pub fn persist_device(conn: &rusqlite::Connection, d: &NewDevice) -> Result<String, String> {
    let fp = fingerprint_for(&d.ua, &d.origin_ip);
    // 只匹配未吊销行；LIMIT 1 + 按最近活跃排序保 determinism（不变量上每指纹至多一行未吊销）
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM remote_devices
             WHERE fingerprint = ?1 AND revoked = 0
             ORDER BY last_seen_at DESC LIMIT 1",
            [&fp],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("persist_device 查指纹失败: {e}"))?;
    if let Some(id) = existing {
        // 命中路径：ua/origin_ip 实际与指纹输入同源（防御式重写），last_seen 必刷新；
        // via 刷新为本次接入通道；name / first_paired_at / revoked 不动
        conn.execute(
            "UPDATE remote_devices
             SET ua = ?2, origin_ip = ?3, last_seen_at = ?4, via = ?5
             WHERE id = ?1",
            rusqlite::params![id, d.ua, d.origin_ip, d.paired_at, d.via],
        )
        .map_err(|e| format!("persist_device 更新失败: {e}"))?;
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO remote_devices
             (id, name, ua, origin_ip, fingerprint, via, first_paired_at, last_seen_at, revoked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 0)",
        rusqlite::params![d.id, d.name, d.ua, d.origin_ip, fp, d.via, d.paired_at],
    )
    .map_err(|e| format!("persist_device 插入失败: {e}"))?;
    Ok(d.id.clone())
}

/// 按设备 id 重命名（M5 A1 DAO；供 Task A4 的 `remote_rename_device` 命令调用）。
/// 返回是否命中行——false = 上层 404 语义（不区分"不存在"与"已吊销"，按 id 直改）。
///
/// 名字长度在 DAO 内收敛：trim 后截前 40 个字符——与 /pair/pin 设备自报名
/// （api::pair_pin 的 `name.trim().chars().take(40)`）同一口径。
/// 选"DAO 自守"而非"注释交上层收敛"：花名册 name 是纯展示字段，DAO 截断
/// 即可保证任何调用方（含 Task A4 命令）都不会把无界名写进库
pub fn rename_device(
    conn: &rusqlite::Connection,
    device_id: &str,
    name: &str,
) -> Result<bool, String> {
    let name: String = name.trim().chars().take(40).collect();
    let n = conn
        .execute(
            "UPDATE remote_devices SET name = ?2 WHERE id = ?1",
            rusqlite::params![device_id, name],
        )
        .map_err(|e| format!("rename_device: {e}"))?;
    Ok(n > 0)
}

pub fn device_valid(conn: &rusqlite::Connection, device_id: &str, now: i64) -> bool {
    conn.query_row(
        "SELECT revoked, last_seen_at FROM remote_devices WHERE id = ?1",
        [device_id],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .map(|(revoked, seen)| revoked == 0 && now - seen <= DEVICE_TTL_MS)
    .unwrap_or(false)
}

pub fn touch_device(conn: &rusqlite::Connection, device_id: &str, now: i64) {
    let _ = conn.execute(
        "UPDATE remote_devices SET last_seen_at = ?2 WHERE id = ?1",
        rusqlite::params![device_id, now],
    );
}

/// 按设备 id 查花名（M7 Task 6：注入来源标记 `[mobile <名>]` 与审计设备名列的数据源）。
/// 形态对齐 device_valid/touch_device：锁内只 SQL 的轻量查询。
/// 查无该行（不存在 / id 非法）→ 回落 "unknown"（端点侧不因设备行缺失而中断注入流程）。
pub fn device_name(conn: &rusqlite::Connection, device_id: &str) -> String {
    conn.query_row(
        "SELECT name FROM remote_devices WHERE id = ?1",
        [device_id],
        |r| r.get(0),
    )
    .unwrap_or_else(|_| "unknown".to_string())
}

pub fn revoke_all(conn: &rusqlite::Connection) -> Result<usize, String> {
    conn.execute("UPDATE remote_devices SET revoked = 1", [])
        .map_err(|e| format!("revoke_all: {e}"))
}

/// 有效设备数（上限门计数口径：revoked=0）
pub fn device_count(conn: &rusqlite::Connection) -> usize {
    conn.query_row(
        "SELECT COUNT(*) FROM remote_devices WHERE revoked = 0",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as usize)
    .unwrap_or(0)
}

/// 单设备吊销（返回影响行数：0 = 本就无效/不存在）
pub fn revoke_device(conn: &rusqlite::Connection, device_id: &str) -> Result<usize, String> {
    conn.execute(
        "UPDATE remote_devices SET revoked = 1 WHERE id = ?1 AND revoked = 0",
        [device_id],
    )
    .map_err(|e| format!("吊销设备失败: {e}"))
}

/// 设备 cookie 统一拼装（api::pair / pair-poll / pair-confirm 三处共用，防漂移）
pub fn device_cookie(device_id: &str) -> String {
    format!(
        "{COOKIE}={device_id}; Path=/m; HttpOnly; SameSite=Lax; Max-Age={}",
        DEVICE_TTL_MS / 1000
    )
}

/// 设备存储注入缝：生产走全局 DB，测试注入内存库（绝不写真实 ~/.mam）
pub enum DeviceStore {
    /// 生产：全局 `~/.mam/mam.db`（`crate::database::connection::DB`）
    Global,
    /// 测试：注入自建库（内存库，绝不落盘真实数据目录）
    Owned(std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>),
}

impl DeviceStore {
    /// 生产构造：锁全局 DB
    pub fn global() -> Self {
        Self::Global
    }

    /// 测试构造：自建内存库并建表。
    /// 调用序必须为 `schema::init`（建全部表）→ `migration::migrate`（增量迁移）——
    /// migrate 首条语句是 `ALTER TABLE extensions`，空库直跑报 "no such table: extensions"
    /// （真机调用序见 `src/database/mod.rs` 的 `init()`）
    pub fn memory() -> Self {
        let conn = rusqlite::Connection::open_in_memory().expect("打开内存库失败");
        crate::database::schema::init(&conn);
        crate::database::migration::migrate(&conn).expect("内存库建表失败");
        Self::Owned(std::sync::Arc::new(std::sync::Mutex::new(conn)))
    }

    /// 在锁定连接上执行闭包。
    /// 注意：连接锁不能作为引用逃逸出闭包（借用会失效），故返回值由闭包自己决定。
    ///
    /// **不可重入**：闭包内再调 `with()` 会在同一把 `Mutex` 上二次加锁——同线程嵌套调用
    /// 必自锁死（std Mutex 非重入）。需要"先判断、后写入"的两段逻辑必须合并进**同一个**
    /// 闭包（如 gate 的 device_valid + touch_device），不得嵌套调用。
    pub fn with<R>(&self, f: impl FnOnce(&rusqlite::Connection) -> R) -> R {
        match self {
            DeviceStore::Global => {
                // 锁自愈取锁（P3 统一）：Global 是生产全局咽喉，毒锁不连坐全部 DB 访问方
                let c = crate::database::connection::DB
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                f(&c)
            }
            DeviceStore::Owned(arc) => {
                let c = arc.lock().unwrap();
                f(&c)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ttl_180d() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 同上：schema::init → migration::migrate（控制者裁决）
        crate::database::schema::init(&conn);
        crate::database::migration::migrate(&conn).unwrap();
        persist_device(
            &conn,
            &NewDevice {
                id: "d1".into(),
                name: String::new(),
                ua: String::new(),
                origin_ip: String::new(),
                via: String::new(),
                paired_at: 0,
            },
        )
        .unwrap();
        touch_device(&conn, "d1", 1000);
        assert!(device_valid(&conn, "d1", 1000 + DEVICE_TTL_MS - 1));
        assert!(!device_valid(&conn, "d1", 1000 + DEVICE_TTL_MS + 1));
    }

    /// DeviceStore 注入缝契约：memory() 建表可用（persist/valid/touch 全链路），
    /// 且 with() 的 `FnOnce(&Connection) -> R` 形态支持取值/布尔/写操作三种用法
    #[test]
    fn device_store_memory_is_wired_and_never_touches_real_db() {
        let store = DeviceStore::memory();
        let now = 1_700_000_000_000i64;
        let dev = NewDevice {
            id: "mem-1".into(),
            name: "测试设备".into(),
            ua: "ua".into(),
            origin_ip: "127.0.0.1".into(),
            via: String::new(),
            paired_at: now,
        };
        // 写（返回 Result，忽略）——若 memory() 未建表此处会 Err 且后续断言失败
        store.with(|c| persist_device(c, &dev).unwrap());
        // 布尔取值
        assert!(store.with(|c| device_valid(c, "mem-1", now)));
        // 写操作（touch）
        store.with(|c| touch_device(c, "mem-1", now + 10));
        let seen: i64 = store.with(|c| {
            c.query_row(
                "SELECT last_seen_at FROM remote_devices WHERE id = 'mem-1'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(seen, now + 10);
        // 未知设备与已吊销设备都不通过
        assert!(!store.with(|c| device_valid(c, "no-such", now)));
        store.with(|c| revoke_all(c).unwrap());
        assert!(!store.with(|c| device_valid(c, "mem-1", now + 10)));
    }

    /// 反向自审：`global()` 只返回标记（不触碰 DB），构造本身不产生副作用。
    /// 若误把 Global 写成急切实例化，本测试仍会通过——故此处仅锁定"构造不写库"，
    /// 真实 DB 访问路径由 gate 的集成测试覆盖（测试一律走 memory()）
    #[test]
    fn device_store_global_construction_has_no_side_effect() {
        let store = DeviceStore::global();
        assert!(matches!(store, DeviceStore::Global));
    }

    // ==== M5 A1：设备指纹 upsert 与 via 字段 ====

    /// 内存库构造（真机调用序：schema::init → migration::migrate，见 DeviceStore::memory 注释）
    fn memory_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        crate::database::migration::migrate(&conn).unwrap();
        conn
    }

    /// 合成测试设备：ua/ip 显式给值（upsert 按指纹去重——空 ua/ip 会互相撞键）
    fn dev(id: &str, ua: &str, ip: &str, paired_at: i64) -> NewDevice {
        NewDevice {
            id: id.into(),
            name: format!("自报名-{id}"),
            ua: ua.into(),
            origin_ip: ip.into(),
            via: "lan".into(),
            paired_at,
        }
    }

    /// 读整行（name, ua, origin_ip, via, first_paired_at, last_seen_at, revoked）
    fn row(
        conn: &rusqlite::Connection,
        id: &str,
    ) -> (String, String, String, String, i64, i64, i64) {
        conn.query_row(
            "SELECT name, ua, origin_ip, via, first_paired_at, last_seen_at, revoked
             FROM remote_devices WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .unwrap()
    }

    /// 总行数（含吊销行——验证"覆盖不新增"/"吊销行不复活"用）
    fn total_rows(conn: &rusqlite::Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM remote_devices", [], |r| r.get(0))
            .unwrap()
    }

    /// 指纹纯函数：确定性 + 拼接格式锁定（sha256("UA|1.2.3.4") 预计算向量）+
    /// UA/IP 任一变化即变（防拼接歧义），64 位小写 hex
    #[test]
    fn fingerprint_for_is_deterministic_and_concatenation_locked() {
        let a = fingerprint_for("UA", "1.2.3.4");
        assert_eq!(a, fingerprint_for("UA", "1.2.3.4"));
        // 固定向量锁定拼接格式为 `{ua}|{origin_ip}`（hex 小写）
        assert_eq!(
            a,
            "b1ed8bf818a01d1d8794475f526dcb5d72323a98f5a2e251661a7bf97d1a980a"
        );
        // 任一分量变化即变
        assert_ne!(
            fingerprint_for("UA", "1.2.3.4"),
            fingerprint_for("UA", "5.6.7.8")
        );
        assert_ne!(
            fingerprint_for("UA", "1.2.3.4"),
            fingerprint_for("UA-B", "1.2.3.4")
        );
        let h = fingerprint_for("x", "y");
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    /// upsert 命中：同指纹二次配对 → 覆盖不新增、返回原 id、
    /// name 保留（含"先重命名后重连"场景）、first_paired 不变、last_seen 刷新、
    /// **via 刷新为本次接入通道**（2026-09-18 实锤定通道裁决：via = 最近一次接入
    /// 的通道，设备换通道接入时徽标自动跟上；id/name/first_paired/revoked 不动）
    #[test]
    fn upsert_hit_keeps_id_and_name_and_first_paired() {
        let conn = memory_conn();
        assert_eq!(
            persist_device(&conn, &dev("d1", "UA", "1.1.1.1", 1000)).unwrap(),
            "d1"
        );
        // 插入路径：via 落库为设备自报值
        assert_eq!(row(&conn, "d1").3, "lan");
        // 桌面端重命名（Task A4 命令语义），随后同一浏览器重连（新 id、同 UA/IP、
        // 时间前进、通道字段不同——验证命中路径 via 刷新为新通道）
        assert!(rename_device(&conn, "d1", "我的手机").unwrap());
        let mut reconnect = dev("d2", "UA", "1.1.1.1", 2000);
        reconnect.via = "quick".into();
        assert_eq!(
            persist_device(&conn, &reconnect).unwrap(),
            "d1",
            "同指纹命中必须返回原行 id（cookie 下发依赖它）"
        );
        let (name, ua, ip, via, first_paired, last_seen, revoked) = row(&conn, "d1");
        assert_eq!(name, "我的手机", "命中路径完全不更新 name——重命名不丢");
        assert_eq!(ua, "UA");
        assert_eq!(ip, "1.1.1.1");
        assert_eq!(via, "quick", "命中路径 via 刷新为本次接入通道");
        assert_eq!(first_paired, 1000, "first_paired 保留");
        assert_eq!(last_seen, 2000, "last_seen 刷新");
        assert_eq!(revoked, 0);
        assert_eq!(total_rows(&conn), 1, "覆盖而非新增");
    }

    /// upsert 命中已吊销行：不复活——新建行（新 id），旧行保持 revoked=1
    #[test]
    fn upsert_hit_on_revoked_row_creates_new_row() {
        let conn = memory_conn();
        persist_device(&conn, &dev("old", "UA", "1.1.1.1", 1000)).unwrap();
        revoke_all(&conn).unwrap();
        let id = persist_device(&conn, &dev("new", "UA", "1.1.1.1", 2000)).unwrap();
        assert_eq!(id, "new", "吊销行不复活，返回新 id");
        assert!(!device_valid(&conn, "old", 2000), "旧行保持吊销");
        assert!(device_valid(&conn, "new", 2000));
        assert_eq!(total_rows(&conn), 2);
        // 吊销行 name 也不受影响（未被覆盖）
        assert_eq!(row(&conn, "old").0, "自报名-old");
    }

    /// 不同指纹（UA 或 IP 不同）→ 新行、返回新 id
    #[test]
    fn different_fingerprint_creates_new_row() {
        let conn = memory_conn();
        persist_device(&conn, &dev("a", "UA", "1.1.1.1", 1000)).unwrap();
        assert_eq!(
            persist_device(&conn, &dev("b", "UA", "2.2.2.2", 2000)).unwrap(),
            "b"
        );
        assert_eq!(
            persist_device(&conn, &dev("c", "UA-B", "1.1.1.1", 3000)).unwrap(),
            "c"
        );
        assert_eq!(total_rows(&conn), 3);
        assert_eq!(device_count(&conn), 3);
    }

    /// rename DAO：命中改名成功返回 true；未命中返回 false（上层 404 语义）
    #[test]
    fn rename_device_hits_and_misses() {
        let conn = memory_conn();
        persist_device(&conn, &dev("d1", "UA", "1.1.1.1", 1000)).unwrap();
        assert!(rename_device(&conn, "d1", "客厅平板").unwrap());
        assert_eq!(row(&conn, "d1").0, "客厅平板");
        assert!(
            !rename_device(&conn, "no-such", "x").unwrap(),
            "未命中 = false"
        );
        // 未命中不写任何行
        assert_eq!(total_rows(&conn), 1);
    }

    /// rename 长度收敛（Minor 5）：trim 后截前 40 字符——与审批自报名同一口径，
    /// DAO 自守防止无界名入库
    #[test]
    fn rename_device_trims_and_caps_name_at_40_chars() {
        let conn = memory_conn();
        persist_device(&conn, &dev("d1", "UA", "1.1.1.1", 1000)).unwrap();
        // 50 个汉字 → 前 40 个
        assert!(rename_device(&conn, "d1", &"甲".repeat(50)).unwrap());
        assert_eq!(row(&conn, "d1").0, "甲".repeat(40));
        // 前后空白 trim
        assert!(rename_device(&conn, "d1", "  平板  ").unwrap());
        assert_eq!(row(&conn, "d1").0, "平板");
    }

    /// device_name（M7 Task 6）：命中行返回花名；查无该行回落 "unknown"
    /// （注入前缀 [mobile <名>] 的数据源，缺失行不得中断端点流程）
    #[test]
    fn device_name_hits_row_and_falls_back_to_unknown() {
        let conn = memory_conn();
        persist_device(&conn, &dev("d1", "UA", "1.1.1.1", 1000)).unwrap();
        assert_eq!(device_name(&conn, "d1"), "自报名-d1");
        assert_eq!(device_name(&conn, "no-such"), "unknown");
    }
}
