// 直通配对状态机（M2 最简版）——dsh-remote-web-ui pairing.ts 七不变量的 Rust 移植
// （研究库源码摘要 §不变量；审批制/花名册 UI 为 M4，不在本文件）

use crate::remote::gate::COOKIE_NAME as COOKIE;
use rand::RngCore;
use rusqlite::OptionalExtension;

/// 设备 cookie 有效期：180 天（dsh 设备会话 TTL，cookie Max-Age 同源）
pub const DEVICE_TTL_MS: i64 = 180 * 24 * 3600 * 1000;

/// 注入式时钟与随机源（可单测；`Send + Sync` 供 axum 跨线程共享）
pub struct PairingClock {
    pub now: Box<dyn Fn() -> i64 + Send + Sync>,
    pub token: Box<dyn Fn() -> String + Send + Sync>,
}

/// accept() 结果：拒绝原因显式区分（Expired 仅在调用方持有真实 secret 时可达）
#[derive(Debug, PartialEq)]
pub enum AcceptResult {
    Ok { device_id: String },
    Invalid,
    Expired,
    Used,
}

/// 活跃 token 记录（私有：模块外不可见）。
/// `secret` 为发行时注入的密文——accept() 必须与之比对
/// （dsh 以密文为 map 键查表，查不到即 invalid；比对是"不给有效性预言机"的前提）
struct TokenRecord {
    secret: String,
    expires_at: i64,
    used: bool,
}

/// 配对服务：同一时刻至多一个活跃 token（单槽即单活跃不变量）
pub struct PairingService {
    ttl_ms: i64,
    clock: PairingClock,
    token: Option<TokenRecord>,
    stopped: bool,
}

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

impl PairingService {
    pub fn new(ttl_ms: i64, clock: PairingClock) -> Self {
        Self {
            ttl_ms,
            clock,
            token: None,
            stopped: false,
        }
    }

    /// 发行新 token：单活跃——发行即作废旧 token（刷新二维码语义）；
    /// stop() 后经此重新武装（dsh：stopped 服务由刷新动作复活）
    pub fn issue(&mut self) -> String {
        let secret = (self.clock.token)();
        let now = (self.clock.now)();
        self.token = Some(TokenRecord {
            secret: secret.clone(),
            expires_at: now + self.ttl_ms,
            used: false,
        });
        self.stopped = false;
        secret
    }

    pub fn accept(&mut self, secret: &str) -> AcceptResult {
        let now = (self.clock.now)();
        let Some(rec) = self.token.as_ref() else {
            return AcceptResult::Invalid;
        };
        // 密文比对失败 = 未知 token，与"无 token"同拒（不泄露任何活跃性信息）
        if rec.secret != secret {
            return AcceptResult::Invalid;
        }
        if self.stopped {
            return AcceptResult::Invalid;
        }
        // 过期仅在持有真实密文时可见（对照 dsh：过期与未知同拒，此处按简报区分码）
        if now > rec.expires_at {
            return AcceptResult::Expired;
        }
        if rec.used {
            return AcceptResult::Used;
        }
        // 一次性：先标记再发设备 id（防重入）
        self.token.as_mut().unwrap().used = true;
        let mut b = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut b);
        AcceptResult::Ok {
            device_id: b.iter().map(|x| format!("{x:02x}")).collect(),
        }
    }

    /// 停止配对服务：**仅清内存 token**（并置 `stopped`）。
    /// 已配对设备的吊销需调用方另行 `revoke_all()`，两者共同构成不变量 5 的「全吊销」——
    /// 单独调用 `stop()` 不足以断开已配对设备（Task 4 的 `stop_server()` 两条都会调）。
    pub fn stop(&mut self) {
        self.token = None;
        self.stopped = true;
    }

    pub fn active_token_expires_at(&self) -> Option<i64> {
        self.token.as_ref().map(|t| t.expires_at)
    }
}

/// 持久化设备（M5 A1：upsert-by-fingerprint——同浏览器同一网络重绑**覆盖不新增**）。
/// - 按指纹查**未吊销**行 → 命中：覆盖 ua / origin_ip / last_seen_at，
///   **保留原 id 与原 name**（含用户重命名——命中路径完全不更新 name）、
///   保留 first_paired_at，revoked 保持 0，via 沿用建行值（运行时刷新属 Task A3）；
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
        // name / first_paired_at / via / revoked 一律不动
        conn.execute(
            "UPDATE remote_devices
             SET ua = ?2, origin_ip = ?3, last_seen_at = ?4
             WHERE id = ?1",
            rusqlite::params![id, d.ua, d.origin_ip, d.paired_at],
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
/// 名字长度在 DAO 内收敛：trim 后截前 40 个字符——与审批自报名
/// （`ApprovalService::create` 的 `name.trim().chars().take(40)`）同一口径。
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
                let c = crate::database::connection::DB.lock().unwrap();
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
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    /// 测试时钟：可预测 token 源 + 可推进的 now。
    ///
    /// 偏离简报原稿一处（编译硬阻断）：原稿用 `Cell<i64>` 捕获进 `now` 闭包，但
    /// `PairingClock::now` 是 `Box<dyn Fn() -> i64 + Send + Sync>` 且 `Cell<i64>` 非 Sync，
    /// 装箱报 E0277（同步性是 Task 3 `RemoteState` 跨 axum 线程共享所需）。
    /// 改用 `Arc<AtomicI64>`，测试语义（读/写当前时刻）完全等价。
    fn svc(ttl_ms: i64) -> (PairingService, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1000i64));
        let now = t.clone();
        let clock = PairingClock {
            now: Box::new(move || now.load(Ordering::SeqCst)),
            token: Box::new(|| "tok-fixed".to_string()),
        };
        (PairingService::new(ttl_ms, clock), t)
    }

    #[test]
    fn single_active_token_issue_invalidates_previous() {
        // 不变量 1+2：issue() 清旧 token——刷新二维码即作废旧链接
        let (mut s, _) = svc(600_000);
        let t1 = s.issue();
        // 注入随机源可预测
        assert_eq!(t1, "tok-fixed");
        // 简报原稿此处写 `assert_eq!(.., AcceptResult::Ok { device_id: _ })`，有两处不可行：
        // `_` 在表达式位置非法（`_ not allowed here`），且 device_id 是随机值不可比对。
        // `matches!` 是等价写法，断言强度不变（只断言变体，不比随机值）
        assert!(matches!(s.accept(&t1), AcceptResult::Ok { .. }));
    }

    /// 追加测试（简报五个之外，理由见报告 §④）：只覆盖 `stop()` 之后的**重新武装**
    /// （dsh：stopped 经 issue 复活，原文 `pairing.ts:219` `this.stopped = false`）。
    /// 注意：本测试证明不了不变量 1——`stop()` 自己已把 token 置 `None`，
    /// 旧密文无论 `issue()` 是否清旧 token 都必然 `Invalid`；不变量 1 的证伪测试见
    /// `issue_twice_without_stop_invalidates_first`。
    #[test]
    fn issue_invalidates_previous_and_rearms_after_stop() {
        let counter = Arc::new(AtomicI64::new(0));
        let c = counter.clone();
        let t = Arc::new(AtomicI64::new(1000));
        let now = t.clone();
        let clock = PairingClock {
            now: Box::new(move || now.load(Ordering::SeqCst)),
            token: Box::new(move || format!("tok-{}", c.fetch_add(1, Ordering::SeqCst))),
        };
        let mut s = PairingService::new(600_000, clock);
        let t1 = s.issue(); // tok-0
        s.stop(); // 全吊销（内存 token 部分）
        let t2 = s.issue(); // tok-1：重新武装
        assert_ne!(t1, t2);
        assert_eq!(s.accept(&t1), AcceptResult::Invalid); // 旧 token 已作废
        assert!(matches!(s.accept(&t2), AcceptResult::Ok { .. })); // 新 token 生效
    }

    /// 不变量 1 的**证伪测试**（评审 Important 1）：不经过 `stop()`，用计数器 token 源
    /// 连续 `issue()` 两次。若实现只在 `stop()`/清空后才作废旧 token（即"唯一缺陷 = issue()
    /// 不清旧 token"的变异体），本测试的 `accept(&a) == Invalid` 断言会失败——
    /// 单点变异已实测：真实实现通过、变异体失败。
    #[test]
    fn issue_twice_without_stop_invalidates_first() {
        let counter = Arc::new(AtomicI64::new(0));
        let c = counter.clone();
        let t = Arc::new(AtomicI64::new(1000));
        let now = t.clone();
        let clock = PairingClock {
            now: Box::new(move || now.load(Ordering::SeqCst)),
            token: Box::new(move || format!("tok-{}", c.fetch_add(1, Ordering::SeqCst))),
        };
        let mut s = PairingService::new(600_000, clock);
        let a = s.issue(); // tok-0
        let b = s.issue(); // tok-1（发行即作废旧 token，未经过 stop）
        assert_ne!(a, b);
        assert_eq!(s.accept(&a), AcceptResult::Invalid); // 旧 token 已作废
        assert!(matches!(s.accept(&b), AcceptResult::Ok { .. })); // 新 token 生效
    }

    #[test]
    fn token_is_one_shot() {
        // 不变量 3：首用即耗，重放 Used
        let (mut s, _) = svc(600_000);
        let t = s.issue();
        assert!(matches!(s.accept(&t), AcceptResult::Ok { .. }));
        assert_eq!(s.accept(&t), AcceptResult::Used);
    }

    #[test]
    fn token_expires() {
        // 不变量 4：过期等同未知（不给有效性预言机）
        let (mut s, t) = svc(600_000);
        let tok = s.issue();
        t.store(1000 + 600_001, Ordering::SeqCst);
        assert_eq!(s.accept(&tok), AcceptResult::Expired);
        assert_eq!(s.accept("never-issued"), AcceptResult::Invalid);
    }

    #[test]
    fn stop_revokes_everything() {
        // 不变量 5：stop 后已配对设备的 DB 校验失败 + token 清空
        let (mut s, _) = svc(600_000);
        let tok = s.issue();
        let AcceptResult::Ok { device_id } = s.accept(&tok) else {
            panic!()
        };
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 控制者裁决（2026-09-14）：真机调用序 = schema::init（建全部表）→ migration::migrate
        // （migrate 首条语句是 ALTER TABLE extensions，空库直跑报 "no such table: extensions"）；
        // remote_devices 表由 migration 建、schema.rs 不建它——断言语义不变
        crate::database::schema::init(&conn);
        crate::database::migration::migrate(&conn).unwrap();
        persist_device(
            &conn,
            &NewDevice {
                id: device_id.clone(),
                name: "p".into(),
                ua: "ua".into(),
                origin_ip: "127.0.0.1".into(),
                via: String::new(),
                paired_at: 1000,
            },
        )
        .unwrap();
        assert!(device_valid(&conn, &device_id, 1000));
        s.stop();
        revoke_all(&conn).unwrap();
        assert!(!device_valid(&conn, &device_id, 2000));
        assert_eq!(s.active_token_expires_at(), None);
    }

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
    /// via 保留建行值（契约覆盖清单仅 ua/origin_ip/last_seen_at）
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
        // 时间前进、通道字段不同——验证命中路径不覆盖 via）
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
        assert_eq!(via, "lan", "命中路径 via 保留建行原值");
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
}
