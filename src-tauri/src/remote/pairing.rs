// 直通配对状态机（M2 最简版）——dsh-remote-web-ui pairing.ts 七不变量的 Rust 移植
// （研究库源码摘要 §不变量；审批制/花名册 UI 为 M4，不在本文件）

use rand::RngCore;

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
    pub paired_at: i64,
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

    pub fn stop(&mut self) {
        self.token = None;
        self.stopped = true;
    }

    pub fn active_token_expires_at(&self) -> Option<i64> {
        self.token.as_ref().map(|t| t.expires_at)
    }
}

pub fn persist_device(conn: &rusqlite::Connection, d: &NewDevice) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO remote_devices
             (id, name, ua, origin_ip, first_paired_at, last_seen_at, revoked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 0)",
        rusqlite::params![d.id, d.name, d.ua, d.origin_ip, d.paired_at],
    )
    .map_err(|e| format!("persist_device: {e}"))?;
    Ok(())
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

    /// 追加测试（简报五个之外，理由见报告 §④）：简报固定 token 源下每次 issue 都是同一字符串，
    /// "发行即作废旧 token" 无法被证伪（单槽实现天然满足）。此处换用计数器 token 源，
    /// 正向验证不变量 1（旧链接失效）与 issue() 对 stop() 的重新武装（dsh：stopped 经 issue 复活）。
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
        s.stop(); // 全吊销
        let t2 = s.issue(); // tok-1：重新武装
        assert_ne!(t1, t2);
        assert_eq!(s.accept(&t1), AcceptResult::Invalid); // 旧 token 已作废
        assert!(matches!(s.accept(&t2), AcceptResult::Ok { .. })); // 新 token 生效
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
                paired_at: 0,
            },
        )
        .unwrap();
        touch_device(&conn, "d1", 1000);
        assert!(device_valid(&conn, "d1", 1000 + DEVICE_TTL_MS - 1));
        assert!(!device_valid(&conn, "d1", 1000 + DEVICE_TTL_MS + 1));
    }
}
