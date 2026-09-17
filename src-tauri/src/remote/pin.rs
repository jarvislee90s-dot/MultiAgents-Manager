// 访问密码（PIN）内核（M5 A2）：KV 存取 + 校验/生成纯函数 + per-IP 限速状态机
// 范围红线：本模块是纯内核——不接线 HTTP 端点（POST /pair/pin 属 Task A3）、
// 不动 gate/server/api；限速状态机为内存态（重启即清），绝不放 DB。

use crate::remote::KEY_ACCESS_PIN;
use std::collections::HashMap;

/// 连续失败上限（用户裁决）：连续输错 5 次锁 10 分钟（按来源 IP）
pub const MAX_FAILURES: u32 = 5;

/// 锁定时长：10 分钟（毫秒；时间轴全模块统一毫秒，与 pairing 时钟口径一致）
pub const LOCK_MS: i64 = 10 * 60 * 1000;

/// 读访问密码（KV `remote.access_pin`；None = 未设置——是否自动生成由上层裁决）
pub fn get_pin() -> Option<String> {
    crate::database::dao::settings::get_setting(KEY_ACCESS_PIN)
}

/// 写访问密码（覆盖写；是否 4 位数字由调用方经 validate_pin 先行校验——本层只管存取）
pub fn set_pin(pin: &str) {
    crate::database::dao::settings::set_setting(KEY_ACCESS_PIN, pin);
}

/// 校验纯函数（无正则，手写字节判断即可）：trim 后必须恰好 4 个 ASCII 数字。
/// `0000` 合法——自填口径以「4 位数字」为准（线稿占位符即 0000）；
/// 只有**随机生成**（generate_pin）才避头零。全角数字（０１２３）非 ASCII 字节，拒绝。
pub fn validate_pin(raw: &str) -> bool {
    let s = raw.trim();
    // 字节长度 == 4 且每字节都在 '0'..='9'：任何多字节字符（全角数字等）必然
    // 撑爆字节长度或含非 ASCII 字节，两条任一即拒
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())
}

/// 随机生成：1000..=9999 均匀随机（4 位不头零——与线稿 randPin 口径一致）。
/// 注意与自填口径的差异：validate_pin 允许 0000，随机生成永不头零
pub fn generate_pin() -> String {
    rand::Rng::gen_range(&mut rand::thread_rng(), 1000..=9999).to_string()
}

/// 过闸判定结果：Allowed 放行；Locked 直接拒绝并带剩余秒数（供 429 retryAfter 文案）
#[derive(Debug, PartialEq, Eq)]
pub enum RateDecision {
    Allowed,
    Locked { retry_after_secs: i64 },
}

/// 单 IP 状态（私有）
#[derive(Default)]
struct Entry {
    /// 连续失败计数（进入锁定时即清零——到期后从零重新累计，不被旧账秒锁）
    failures: u32,
    /// Some = 锁定截止时刻（毫秒）；None = 未锁定
    locked_until: Option<i64>,
}

/// per-IP 限速状态机（内存态，重启即清——不放 DB）。
///
/// 时钟注入：内核**零真实时钟读取**，`now` 由调用方显式传入（生产 A3 侧传
/// chrono 毫秒；测试传合成时间）——与 pairing.rs 的 PairingClock + fake clock
/// 同一目的，形态更简：时间显式传参，连闭包都不需要，测试完全确定、零 sleep。
///
/// 无界增长说明：键为来源 IP（`SocketAddr::ip().to_string()` 形态），数量级 =
/// 访问过的客户端数，桌面远程场景极小；不做后台清理线程/惰性扫描（勿过度设计）。
pub struct PinRateLimiter {
    entries: HashMap<String, Entry>,
}

impl PinRateLimiter {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// 过闸判定：锁定期内 → Locked（带剩余秒数，向上取整——剩 1ms 也报 1 秒，
    /// 避免 0 秒误导客户端立刻重试）；锁已到期 → 计数清零重新开始并放行。
    /// **判定与记账分离**：本方法不改失败计数（成败与否由调用方在 PIN 比对后
    /// 另行 record_failure / record_success）
    pub fn check(&mut self, ip: &str, now: i64) -> RateDecision {
        let e = self.entries.entry(ip.to_string()).or_default();
        if let Some(until) = e.locked_until {
            if now < until {
                return RateDecision::Locked {
                    retry_after_secs: (until - now + 999) / 1000,
                };
            }
            // 到期解锁：清锁定与计数，重新开始
            e.locked_until = None;
            e.failures = 0;
        }
        RateDecision::Allowed
    }

    /// 记一次失败：累计到第 5 次（MAX_FAILURES）即锁定 10 分钟（第 5 次起 check 即 Locked）。
    /// 锁定期内的失败**不计数也不延期**（简单口径——锁定本身就是足额惩罚，延期会把
    /// 锁定窗口推着攻击者走；正常流程到不了此处——check 先拒，此分支是防御）。
    /// 已过期的锁在本方法内**先镜像 check 的到期分支**（清锁 + 清计数）再累计：
    /// 否则「到期后未经 check 直接 record_failure」攒下的失败（最长 4 次）会被之后
    /// check 的到期分支一笔清零——静默蒸发，攻击者白得一轮全新 5 次预算
    /// （评审 Minor 1，测试 `failures_after_expiry_survive_late_check` 锁定）
    pub fn record_failure(&mut self, ip: &str, now: i64) {
        let e = self.entries.entry(ip.to_string()).or_default();
        if let Some(until) = e.locked_until {
            if now < until {
                return; // 锁定期内：不计数、不延期
            }
            // 已过期：与 check 到期分支同口径——清锁 + 清计数，过期后的失败从零起算
            e.locked_until = None;
            e.failures = 0;
        }
        e.failures += 1;
        if e.failures >= MAX_FAILURES {
            e.locked_until = Some(now + LOCK_MS);
            e.failures = 0; // 锁定起计数即清：到期解锁后从零重新累计
        }
    }

    /// 记一次成功（PIN 比对通过）：整条清除该 IP 状态（计数清零）。
    /// 锁定期内不会走到此处（check 先拒、PIN 比对根本不发生），故无「成功解锁」语义
    pub fn record_success(&mut self, ip: &str) {
        self.entries.remove(ip);
    }
}

impl Default for PinRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==== KV 薄封装 ====

    /// KV 薄封装冒烟：键名 = 用户裁决的 `remote.access_pin`（外部契约，防漂移）。
    /// 直调 get_pin/set_pin 会锁全局 DB 连接（真实 ~/.mam/mam.db）——零污染红线禁止，
    /// 与 hooks.rs 测试「MAM_HOME 重定向不可行」同一结论；读写往返语义由 settings DAO
    /// 自身测试（settings_kv_roundtrip_and_overwrite）覆盖，本层只锁定键名契约
    /// （先例：remote/mod.rs 的 setting_keys_are_stable）
    #[test]
    fn access_pin_kv_key_is_stable() {
        assert_eq!(KEY_ACCESS_PIN, "remote.access_pin");
    }

    // ==== validate_pin：校验纯函数 ====

    /// 4 位 ASCII 数字即真，`0000` 合法（自填口径以「4 位数字」为准，线稿占位符即 0000）
    #[test]
    fn validate_pin_accepts_four_ascii_digits_including_0000() {
        assert!(validate_pin("1234"));
        assert!(validate_pin("0000"), "0000 合法——只有随机生成才避头零");
        assert!(validate_pin("9999"));
        assert!(validate_pin("0420"), "中间带 0 也合法");
    }

    /// trim 掉首尾空白后恰好 4 数字 → 真
    #[test]
    fn validate_pin_accepts_after_trimming_ends() {
        assert!(validate_pin(" 1234 "), "首尾空格 trim 后 4 数字");
        assert!(validate_pin("5678\t"), "尾制表符");
        assert!(validate_pin("\n6789"), "头换行");
    }

    /// 反例矩阵：位数不对 / 混入非 ASCII 数字字符 / 全角数字 / 纯空白
    #[test]
    fn validate_pin_rejects_bad_shapes() {
        assert!(!validate_pin("123"), "3 位");
        assert!(!validate_pin("12345"), "5 位");
        assert!(!validate_pin("12a4"), "含字母");
        assert!(!validate_pin("12 4"), "含中间空格（trim 只去首尾）");
        assert!(!validate_pin(""), "空串");
        assert!(!validate_pin("   "), "纯空白（trim 后空串）");
        assert!(!validate_pin("１２３４"), "全角数字非 ASCII");
        assert!(!validate_pin("１２34"), "全角混半角");
        assert!(!validate_pin("  12"), "trim 后只剩 2 位");
        assert!(!validate_pin("12.4"), "含标点");
        assert!(!validate_pin("+123"), "含符号");
        assert!(!validate_pin("12３4"), "末位全角");
    }

    // ==== generate_pin：随机生成 ====

    /// 多次采样恒落 1000..=9999、恒 4 位 ASCII 数字、永不头零（与线稿 randPin 口径一致）
    #[test]
    fn generate_pin_always_in_1000_9999_and_never_leading_zero() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..500 {
            let p = generate_pin();
            assert_eq!(p.len(), 4, "恒 4 位: {p}");
            assert!(p.bytes().all(|b| b.is_ascii_digit()), "恒 ASCII 数字: {p}");
            assert_ne!(p.as_bytes()[0], b'0', "随机生成永不头零: {p}");
            let n: i32 = p.parse().unwrap();
            assert!((1000..=9999).contains(&n), "范围 1000..=9999: {p}");
            seen.insert(p);
        }
        assert!(
            seen.len() > 1,
            "500 次采样应出现多个不同值（恒输出常数即坏）"
        );
    }

    // ==== per-IP 限速状态机（注入 fake clock）====

    /// 合成时钟（fake clock，对齐 pairing.rs 测试的注入时钟模式——时间显式传参形态下
    /// 一个可推进的变量即注入时钟；全程零真实时钟读取、零 sleep，测试完全确定）
    struct FakeClock(i64);

    impl FakeClock {
        fn now(&self) -> i64 {
            self.0
        }
        fn advance_ms(&mut self, ms: i64) {
            self.0 += ms;
        }
    }

    /// 错 4 次 → 仍 Allowed；第 5 次失败 → 进入锁定（第 5 次起 check 即 Locked，
    /// 剩余 600 秒 = LOCK_MS 整额）
    #[test]
    fn four_failures_allowed_fifth_locks() {
        let mut rl = PinRateLimiter::new();
        let t = FakeClock(1_000_000);
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Allowed,
            "错 4 次仍放行"
        );
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "第 5 次失败即锁 10 分钟（600 秒）"
        );
    }

    /// 锁定期内 retry_after 随时间推进递减；到 10 分钟整（now == 截止）解锁放行，
    /// 且计数已清——解锁后需再错满 5 次才会再锁
    #[test]
    fn retry_after_decrements_then_unlocks_with_fresh_counter() {
        let mut rl = PinRateLimiter::new();
        let mut t = FakeClock(0);
        for _ in 0..5 {
            rl.record_failure("1.1.1.1", t.now());
        }
        t.advance_ms(300_000); // +5 分钟
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 300
            },
            "剩余 5 分钟 = 300 秒"
        );
        t.advance_ms(299_000); // 距到期还剩 1 秒
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 1
            },
            "递减到 1 秒"
        );
        t.advance_ms(1_000); // 恰好 10 分整（now == locked_until）
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Allowed,
            "到期（>=10 分）解锁"
        );
        // 解锁后计数已清：再错 4 次仍放行，第 5 次才再锁
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(rl.check("1.1.1.1", t.now()), RateDecision::Allowed);
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "解锁后重新累计，第 5 次再锁"
        );
    }

    /// 锁定期内的失败既不计数也不延期（简单口径，注释写明）：锁内刷失败后
    /// 剩余秒数不变；到期后从零重新累计——锁内那批失败一笔勾销
    #[test]
    fn failures_during_lock_neither_count_nor_extend() {
        let mut rl = PinRateLimiter::new();
        let mut t = FakeClock(0);
        for _ in 0..5 {
            rl.record_failure("1.1.1.1", t.now());
        }
        t.advance_ms(120_000); // +2 分钟
        for _ in 0..10 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 480
            },
            "锁内刷失败不延期——剩余仍是 8 分钟"
        );
        t.advance_ms(480_000); // 恰到期
        assert_eq!(rl.check("1.1.1.1", t.now()), RateDecision::Allowed);
        // 锁内失败未计数：到期后错 4 次仍放行（若被计数则这里已是第 15 次、必锁）
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Allowed,
            "锁内失败不计数——到期后从零重新累计"
        );
    }

    /// 评审 Minor 1（A2 fixup）：到期后未经 check 直接 record_failure 的失败必须存活——
    /// 若 record_failure 不先清过期锁，之后 check 的到期分支会把过期后攒下的失败
    /// （本测试恰 4 次）一笔清零（静默蒸发，攻击者白得全新 5 次预算）。
    /// 变异锚点：还原为"仅锁内 return、不清过期锁"时，末断言必由 Locked 退化 Allowed
    #[test]
    fn failures_after_expiry_survive_late_check() {
        let mut rl = PinRateLimiter::new();
        let mut t = FakeClock(0);
        for _ in 0..5 {
            rl.record_failure("1.1.1.1", t.now());
        }
        t.advance_ms(600_000); // 恰到期（now == 截止），**不经过 check**
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now()); // 过期后累计，须存活
        }
        // 此刻才首次观察到过期——4 次失败不得被清
        assert_eq!(rl.check("1.1.1.1", t.now()), RateDecision::Allowed);
        // 过期后第 5 次失败即再锁（若被蒸发则此处仍 Allowed）
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "过期后未经 check 攒下的失败必须存活，第 5 次即再锁"
        );
    }

    /// record_success 清零该 IP 计数：错 4 次 → 成功 → 再错 4 次仍放行（从零起算）
    #[test]
    fn record_success_resets_counter() {
        let mut rl = PinRateLimiter::new();
        let t = FakeClock(100);
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now());
        }
        rl.record_success("1.1.1.1");
        for _ in 0..4 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Allowed,
            "成功清零后 4 次失败不足以锁定"
        );
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "清零后重新数满 5 次"
        );
    }

    /// 评审补缺 a：retry_after 向上取整的**非整秒半边**——剩 1500ms 报 2 秒
    /// （与"剩 1ms 报 1 秒"同一 ceil 口径，避免 0/小额秒数误导客户端立即重试）
    #[test]
    fn retry_after_rounds_up_partial_seconds() {
        let mut rl = PinRateLimiter::new();
        let mut t = FakeClock(0);
        for _ in 0..5 {
            rl.record_failure("1.1.1.1", t.now());
        }
        t.advance_ms(598_500); // 剩 1500ms
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 2
            },
            "1500ms 向上取整为 2 秒"
        );
    }

    /// 评审补缺 c：record_success 对未见过的 IP 是无害空操作（remove absent key）——
    /// 不 panic、不留状态、不产生隐性计数（后续 4 次失败仍不足锁定）
    #[test]
    fn record_success_on_unseen_ip_is_harmless_noop() {
        let mut rl = PinRateLimiter::new();
        let t = FakeClock(7);
        rl.record_success("9.9.9.9");
        assert_eq!(rl.check("9.9.9.9", t.now()), RateDecision::Allowed);
        for _ in 0..4 {
            rl.record_failure("9.9.9.9", t.now());
        }
        assert_eq!(
            rl.check("9.9.9.9", t.now()),
            RateDecision::Allowed,
            "未见 IP 的成功不留任何隐性计数"
        );
    }

    /// 不同 IP 互不影响：A 锁定不影响 B 计数与放行，C 未出现过的 IP 直接放行
    #[test]
    fn ips_are_independent() {
        let mut rl = PinRateLimiter::new();
        let t = FakeClock(50);
        for _ in 0..5 {
            rl.record_failure("1.1.1.1", t.now());
        }
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            }
        );
        for _ in 0..4 {
            rl.record_failure("2.2.2.2", t.now());
        }
        assert_eq!(
            rl.check("2.2.2.2", t.now()),
            RateDecision::Allowed,
            "B 错 4 次不受 A 锁定影响"
        );
        assert_eq!(
            rl.check("3.3.3.3", t.now()),
            RateDecision::Allowed,
            "未出现过的 IP 直接放行"
        );
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "A 的锁定状态独立存续"
        );
    }
}
