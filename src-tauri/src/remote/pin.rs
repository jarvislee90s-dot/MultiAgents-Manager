// 访问密码（PIN）内核（M5 A2）：KV 存取 + 校验/生成纯函数 + per-IP 限速状态机
// （阈值 / 锁定时长 / **失败记账的滑动窗口** 三个旋钮，见 FAILURE_WINDOW_MS）。
// 接线现状（A3 起）：POST /pair/pin 端点消费本模块（api::pair_pin）；
// 限速状态机为内存态（重启即清），绝不放 DB。

use crate::remote::KEY_ACCESS_PIN;
use std::collections::{HashMap, VecDeque};

/// 连续失败上限（用户裁决）：连续输错 5 次锁 10 分钟（按来源 IP）
pub const MAX_FAILURES: u32 = 5;

/// 锁定时长：10 分钟（毫秒；时间轴全模块统一毫秒，与 pairing 时钟口径一致）
pub const LOCK_MS: i64 = 10 * 60 * 1000;

/// 失败记账的**滑动窗口**长度：= [`LOCK_MS`]（用户裁决「给桶底开个洞」，评审 A-I1）。
/// 计数语义是「**最近 10 分钟内的失败次数**」而非「进程启动至今累计」——窗口外的旧
/// 失败在 check / record_failure 读到时惰性丢弃（无后台清理线程，勿过度设计）。
/// 取锁定长度：①「窗口内失败密度」正好是"该不该锁"的那个量；② 锁定到期时，锁定前
/// 记的失败恰好滑出窗口 —— 与既有「到期后从零重新累计」语义自然合一（无需另写清零）。
pub const FAILURE_WINDOW_MS: i64 = LOCK_MS;

/// 全局桶的键（固定值，含 NUL 前缀——与任何真实来源键不可能冲突）。
/// **注意它不是真实来源地址**：这是限速的内部哨兵（来源不可证时 fail-closed 回落
/// 到此桶），任何情况下都不得写进设备指纹 / 花名册 / 审计的来源列
/// （展示来源另走 `gate::display_origin`，评审 A-M3）。
pub const GLOBAL_BUCKET: &str = "\u{0}global";

/// 全局上限（分布式爆破只能靠它挡：单个来源的限速挡不住多来源齐爆）。
/// **必须宽到单个来源填不满**——这样捣乱者只会填满自己的桶，伤不到别人。
/// 为什么单来源填不满：全局桶同样按 [`FAILURE_WINDOW_MS`] 滑动窗口记账，而单个来源
/// 在其分来源桶（阈值 [`MAX_FAILURES`]）锁定期内不再记账 → **每窗口最多贡献
/// MAX_FAILURES 次**，本阈值是它的 10 倍。
/// 取值 50：一个窗口内需 ≥10 个独立来源齐爆才触顶（分布式爆破防护不损）。
/// 历史教训（评审 A-I1）：本阈值曾配「进程生命周期单调计数」——单来源 10 个周期
/// （≈100 分钟）即可填满并锁死所有人（含局域网/本机合法新设备配对）。**勿退回单调累计**。
pub const GLOBAL_MAX_FAILURES: u32 = 50;

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
    rand::random_range(1000..=9999).to_string()
}

/// 过闸判定结果：Allowed 放行；Locked 直接拒绝并带剩余秒数（供 429 retryAfter 文案）
#[derive(Debug, PartialEq, Eq)]
pub enum RateDecision {
    Allowed,
    Locked { retry_after_secs: i64 },
}

/// 单桶状态（私有）
#[derive(Default)]
struct Entry {
    /// **窗口内**的失败时刻（毫秒，升序）。存时间戳列表而非裸计数：滑动窗口必须知道
    /// "每次失败何时发生"才能逐个滑出。内存有界——到 max_failures 即置锁并清空，
    /// 故每桶至多 max_failures 条；桶数 = 访问过的客户端数 + 1 个全局桶（桌面远程极小）。
    failures: VecDeque<i64>,
    /// Some = 锁定截止时刻（毫秒）；None = 未锁定
    locked_until: Option<i64>,
}

impl Entry {
    /// 惰性过期：丢弃窗口外的旧失败记录（**半开区间**——只保留 `now - ts < window_ms`，
    /// 语义 = "最近 window_ms 毫秒内的失败"）。恰满窗口（now - ts == window_ms）即过期，
    /// 与锁定边界 `now < until` 同一口径。时间戳升序入队，故只从头弹。
    fn prune(&mut self, now: i64, window_ms: i64) {
        while let Some(&ts) = self.failures.front() {
            if now - ts >= window_ms {
                self.failures.pop_front();
            } else {
                break;
            }
        }
    }

    /// 窗口内失败数（非变异读：remaining_attempts 用它，不必为一次查询改动状态）
    fn count_in_window(&self, now: i64, window_ms: i64) -> u32 {
        self.failures
            .iter()
            .filter(|&&ts| now - ts < window_ms)
            .count() as u32
    }
}

/// per-IP 限速状态机（内存态，重启即清——不放 DB）。
///
/// 时钟注入：内核**零真实时钟读取**，`now` 由调用方显式传入（生产 A3 侧传
/// chrono 毫秒；测试传合成时间）——时间显式传参形态：测试完全确定、零 sleep
/// （与旧 PairingClock 注入时钟同一目的，形态更简，连闭包都不需要）。
///
/// 衰减（§G5 / 评审 A-I1，用户裁决「给桶底开个洞」）：计数不是"只进不出"，而是
/// **滑动窗口**（`window_ms`，生产 = [`FAILURE_WINDOW_MS`] = 锁定时长）——判断前先
/// 丢掉窗口外的旧失败。成功（record_success）仍整条清桶；**没有**"成功清零全局桶"
/// 语义（成功只清自己那一份来源桶）。
///
/// 键的三态（真实推导在 `gate::rate_bucket_key`；分桶键只进限速与审计、不进鉴权）：
/// ① 来源地址（非回环直连，`SocketAddr::ip().to_string()` 形态）；② 隧道场景下
/// 经**已声明通道**核验的权威来源头（回环 + Host 命中该通道已登记域名 + 该通道静态
/// 声明的权威头在场才采信）；③ 全局固定键（`GLOBAL_BUCKET`，来源不可证即 fail-closed 回落）。
/// 无界增长说明：桶数量级 = 访问过的客户端数 + 1 个全局桶，且每桶失败时刻至多
/// max_failures 条（到阈值即置锁并清空）；惰性过期，不做后台清理线程/扫描（勿过度设计）。
pub struct PinRateLimiter {
    entries: HashMap<String, Entry>,
    max_failures: u32,
    lock_ms: i64,
    /// 失败记账窗口长度（毫秒）；窗口外的失败自动过期（见模块顶 `FAILURE_WINDOW_MS`）
    window_ms: i64,
}

impl PinRateLimiter {
    /// 分来源桶：沿用既有语义（5 次 / 10 分钟；窗口 = 锁定时长）
    pub fn new() -> Self {
        Self::with_limits(MAX_FAILURES, LOCK_MS, FAILURE_WINDOW_MS)
    }

    /// 全局桶：更宽的阈值（见 `GLOBAL_MAX_FAILURES`）；窗口同为 FAILURE_WINDOW_MS
    pub fn global() -> Self {
        Self::with_limits(GLOBAL_MAX_FAILURES, LOCK_MS, FAILURE_WINDOW_MS)
    }

    /// 三个旋钮：阈值 / 锁定时长 / **失败记账的滑动窗口**（互相独立，测试可各自缩放）
    pub fn with_limits(max_failures: u32, lock_ms: i64, window_ms: i64) -> Self {
        Self {
            entries: HashMap::new(),
            max_failures,
            lock_ms,
            window_ms,
        }
    }

    /// 过闸判定：锁定期内 → Locked（带剩余秒数，向上取整——剩 1ms 也报 1 秒，
    /// 避免 0 秒误导客户端立刻重试）；锁已到期 → 解锁并放行（锁定前记的失败已随窗口
    /// 滑出，等价于"到期从零重新累计"）。
    /// **判定与记账分离**：本方法不改失败计数（成败与否由调用方在 PIN 比对后
    /// 另行 record_failure / record_success）
    pub fn check(&mut self, key: &str, now: i64) -> RateDecision {
        let window_ms = self.window_ms;
        let e = self.entries.entry(key.to_string()).or_default();
        e.prune(now, window_ms);
        if let Some(until) = e.locked_until {
            if now < until {
                return RateDecision::Locked {
                    retry_after_secs: (until - now + 999) / 1000,
                };
            }
            e.locked_until = None;
        }
        RateDecision::Allowed
    }

    /// 记一次失败：**窗口内**累计到阈值（分来源桶 5 次 = MAX_FAILURES / 全局桶 50 次 =
    /// GLOBAL_MAX_FAILURES，由构造时的 max_failures 决定）即锁定 lock_ms（分来源桶
    /// 10 分钟/全局桶同 LOCK_MS；阈值那一次起 check 即 Locked）。
    /// 锁定期内的失败**不计数也不延期**（简单口径——锁定本身就是足额惩罚，延期会把
    /// 锁定窗口推着攻击者走；正常流程到不了此处——check 先拒，此分支是防御）。
    /// 已过期的锁在本方法内**先镜像 check 的到期分支**（解锁 + 窗口过期）再累计：
    /// 否则「到期后未经 check 直接 record_failure」攒下的失败（最长 4 次）会被之后
    /// check 的到期处理一并丢掉——静默蒸发，攻击者白得一轮全新 5 次预算
    /// （评审 Minor 1，测试 `failures_after_expiry_survive_late_check` 锁定）
    pub fn record_failure(&mut self, key: &str, now: i64) {
        let window_ms = self.window_ms;
        let max_failures = self.max_failures;
        let e = self.entries.entry(key.to_string()).or_default();
        e.prune(now, window_ms);
        if let Some(until) = e.locked_until {
            if now < until {
                return; // 锁定期内：不计数、不延期
            }
            // 已过期：解锁。计数不另行清零——**窗口衰减是唯一的记账规则**（窗口 =
            // 锁定时长时，锁定前的失败在解锁时已全部滑出窗口，效果与"清零"一致）
            e.locked_until = None;
        }
        e.failures.push_back(now);
        if e.count_in_window(now, window_ms) >= max_failures {
            e.locked_until = Some(now + self.lock_ms);
            e.failures.clear(); // 锁定起清空：解锁后从零重新累计
        }
    }

    /// 记一次成功（PIN 比对通过）：整条清除该桶状态（计数清零）。
    /// 锁定期内不会走到此处（check 先拒、PIN 比对根本不发生），故无「成功解锁」语义
    pub fn record_success(&mut self, key: &str) {
        self.entries.remove(key);
    }

    /// 剩余可尝试次数（401 `invalid_pin.remaining` 文案数据源——移动端要展示
    /// 「还可尝试 N 次」）：锁定期恒 0；否则 max_failures - **窗口内**失败数（窗口外的
    /// 旧失败不算账，故本查询非变异读：逐条按 now 过滤，不改状态）。未见过桶满额。
    /// 第 5 次失败 record_failure 已置锁并清计数——本方法先看锁，恰返 0（不泄露
    /// "计数已清零"的内部态），与「5 次错后第 6 次请求必 429」的外部语义一致
    pub fn remaining_attempts(&self, key: &str, now: i64) -> u32 {
        match self.entries.get(key) {
            Some(e) if e.locked_until.is_some() => 0,
            Some(e) => self
                .max_failures
                .saturating_sub(e.count_in_window(now, self.window_ms)),
            None => self.max_failures,
        }
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
    /// 直调 get_pin/set_pin 会锁全局 DB 连接（真实 ~/.tuvis/tuvis.db）——零污染红线禁止，
    /// 与 hooks.rs 测试「TUVIS_HOME 重定向不可行」同一结论；读写往返语义由 settings DAO
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

    // ==== 全局桶的滑动窗口衰减（§G5 / 评审 A-I1）====

    /// 评审 A-I1 复现（红锚点）：全局桶若是**进程生命周期内的单调计数**，单个来源
    /// 每 10 分钟周期最多贡献 MAX_FAILURES 次（其分来源桶 5 次即锁、锁内不记账），
    /// 10 个周期（≈100 分钟）即可被**单个**来源填满 → 全局闸把所有人（含局域网/
    /// 本机合法新设备配对）挡 10 分钟。修法（用户裁决「给桶底开个洞」）：滑动窗口
    /// 衰减，窗口外的旧失败自动过期——单来源每窗口贡献 ≤ MAX_FAILURES，永远填不满。
    /// 变异锚点：去掉窗口/改回单调累计，本测试在第 10 个周期起必红（Locked）。
    #[test]
    fn global_bucket_is_never_filled_by_a_single_source_across_windows() {
        let mut global = PinRateLimiter::global();
        let mut source = PinRateLimiter::new();
        let mut t = 0i64;
        for cycle in 0..12 {
            // 该来源一个窗口内能记进全局的次数上限 = 它自己的阈值
            // （第 5 次 record_failure 即置锁，锁定期内的失败不计数）
            for _ in 0..MAX_FAILURES {
                if matches!(source.check("198.51.100.7", t), RateDecision::Allowed) {
                    source.record_failure("198.51.100.7", t);
                    global.record_failure(GLOBAL_BUCKET, t);
                }
            }
            assert_eq!(
                global.check(GLOBAL_BUCKET, t),
                RateDecision::Allowed,
                "第 {cycle} 个窗口：单来源每窗口贡献上限 {MAX_FAILURES} 次，绝填不满全局桶"
            );
            t += LOCK_MS;
            let _ = source.check("198.51.100.7", t); // 分来源桶到期解锁
        }
    }

    /// 窗口边界（评审 A-I1 ①）：恰满窗口的旧失败自动过期（注入时钟推进，零 sleep）。
    /// 窗口是半开区间 `now - ts < window_ms`，与锁定边界 `now < until` 同一口径。
    /// 变异锚点：删掉 prune（回到单调累计）或把边界写成 `>`，本测试必红（前者 remaining
    /// 停在 1、后者提前回满额）。
    #[test]
    fn failures_outside_window_expire_at_exact_window_boundary() {
        let mut rl = PinRateLimiter::with_limits(3, LOCK_MS, FAILURE_WINDOW_MS);
        let mut t = FakeClock(0);
        rl.record_failure("1.1.1.1", t.now());
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.remaining_attempts("1.1.1.1", t.now()),
            1,
            "窗口内计 2 次"
        );
        t.advance_ms(FAILURE_WINDOW_MS - 1);
        assert_eq!(
            rl.remaining_attempts("1.1.1.1", t.now()),
            1,
            "窗口内（差 1ms 未满）仍记账"
        );
        t.advance_ms(1); // now - ts == window_ms：恰满窗口即过期
        assert_eq!(
            rl.remaining_attempts("1.1.1.1", t.now()),
            3,
            "窗口外的旧失败自动过期（回满额）"
        );
        // 过期后的失败从零起算：连错 2 次仍放行，第 3 次才锁（旧账不参与）
        rl.record_failure("1.1.1.1", t.now());
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(rl.check("1.1.1.1", t.now()), RateDecision::Allowed);
        rl.record_failure("1.1.1.1", t.now());
        assert_eq!(
            rl.check("1.1.1.1", t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "窗口内重新数满 3 次即锁"
        );
    }

    /// 窗口内密集失败照样触发全局锁（评审 A-I1 ②）：分布式爆破防护不因衰减而损——
    /// 同一瞬间灌满阈值即锁，retry_after 为整额。
    #[test]
    fn dense_failures_within_window_still_lock_global_bucket() {
        let mut g = PinRateLimiter::global();
        let t = FakeClock(1_700_000_000_000);
        for _ in 0..GLOBAL_MAX_FAILURES {
            g.record_failure(GLOBAL_BUCKET, t.now());
        }
        assert_eq!(
            g.check(GLOBAL_BUCKET, t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "窗口内 {GLOBAL_MAX_FAILURES} 次齐爆必锁（爆破防护不损）"
        );
    }

    /// 全局阈值**宽度**本身（评审 A-I1 ③ + A-M5）：把「宽到单个来源填不满」钉死在
    /// 单元层——单来源每窗口贡献上限 MAX_FAILURES，全局阈值必须远宽于它（本仓库口径：
    /// 10 倍，即需 **≥10 个独立来源**在同一窗口齐爆才触顶）。
    /// 变异锚点：`50 → 6` 这类把全局阈值压到与单来源同量级的改动，本测试的 45 次档必红
    /// （2 个来源即锁）；`50 → 49/51` 由下方恰满阈值档抓。
    #[test]
    fn global_threshold_is_far_wider_than_one_source_can_contribute() {
        let t = FakeClock(0);
        // 行为级宽度：9 个独立来源各贡献满额（9 × MAX_FAILURES = 45 次）仍不得触顶
        let mut g = PinRateLimiter::global();
        for _ in 0..9 {
            for _ in 0..MAX_FAILURES {
                g.record_failure(GLOBAL_BUCKET, t.now());
            }
        }
        assert_eq!(
            g.check(GLOBAL_BUCKET, t.now()),
            RateDecision::Allowed,
            "9 个来源齐爆（45 次）不得锁全局桶——阈值须宽到需 ≥10 个来源"
        );
        // 行为级：恰满阈值才锁（49 次放行 / 第 50 次锁）——阈值精确性锚点
        let mut g = PinRateLimiter::global();
        for _ in 0..GLOBAL_MAX_FAILURES - 1 {
            g.record_failure(GLOBAL_BUCKET, t.now());
        }
        assert_eq!(g.check(GLOBAL_BUCKET, t.now()), RateDecision::Allowed);
        g.record_failure(GLOBAL_BUCKET, t.now());
        assert_eq!(
            g.check(GLOBAL_BUCKET, t.now()),
            RateDecision::Locked {
                retry_after_secs: 600
            },
            "第 {GLOBAL_MAX_FAILURES} 次失败即锁"
        );
    }

    /// A-M5：`with_limits` / `global()` 的直接单测锚点（此前只有 `new()` 被覆盖，
    /// 「更宽阈值 + 可缩放参数」这一语义没有单元级证据）。
    #[test]
    fn with_limits_and_global_honor_their_parameters() {
        // 阈值 2 / 锁 1 秒 / 窗口 1 秒：第 2 次失败即锁，retry_after 向上取整 1 秒
        let mut tight = PinRateLimiter::with_limits(2, 1_000, 1_000);
        let mut t = FakeClock(0);
        tight.record_failure("k", t.now());
        assert_eq!(tight.check("k", t.now()), RateDecision::Allowed);
        tight.record_failure("k", t.now());
        assert_eq!(
            tight.check("k", t.now()),
            RateDecision::Locked {
                retry_after_secs: 1
            },
            "锁定时长取自 lock_ms 参数（1000ms → 1 秒）"
        );
        t.advance_ms(1_000); // 窗口与锁同时到期
        assert_eq!(
            tight.check("k", t.now()),
            RateDecision::Allowed,
            "参数化锁定到期即放行"
        );

        // 更宽阈值（10）：单来源阈值 5 次在这条桶上仍放行——「更宽」语义可证
        let mut wide = PinRateLimiter::with_limits(10, LOCK_MS, FAILURE_WINDOW_MS);
        for _ in 0..MAX_FAILURES {
            wide.record_failure("k", t.now());
        }
        assert_eq!(
            wide.check("k", t.now()),
            RateDecision::Allowed,
            "with_limits(10) 下 5 次失败不得锁定"
        );
        assert_eq!(
            wide.remaining_attempts("k", t.now()),
            MAX_FAILURES,
            "剩余 = 10 - 5（阈值参数被 remaining 消费）"
        );

        // global() 三个旋钮：阈值 = GLOBAL_MAX_FAILURES、窗口 = FAILURE_WINDOW_MS = LOCK_MS
        assert_eq!(
            FAILURE_WINDOW_MS, LOCK_MS,
            "生产窗口 = 锁定时长（解锁时锁定前的失败恰好滑出 → 解锁即全新预算）"
        );
        let mut g = PinRateLimiter::global();
        g.record_failure(GLOBAL_BUCKET, t.now());
        assert_eq!(
            g.remaining_attempts(GLOBAL_BUCKET, t.now()),
            GLOBAL_MAX_FAILURES - 1,
            "global() 阈值 = GLOBAL_MAX_FAILURES"
        );
        t.advance_ms(LOCK_MS); // 恰满窗口
        assert_eq!(
            g.remaining_attempts(GLOBAL_BUCKET, t.now()),
            GLOBAL_MAX_FAILURES,
            "global() 窗口 = FAILURE_WINDOW_MS（窗口外旧失败过期）"
        );
        assert_eq!(
            PinRateLimiter::new().remaining_attempts("k", t.now()),
            MAX_FAILURES,
            "new() = 分来源桶（阈值 MAX_FAILURES）"
        );
    }

    // ==== remaining_attempts（A3 端点 401 remaining 文案数据源）====

    /// 递减语义：未见过 → 5；错 1..4 次 → 4..1；第 5 次（置锁）→ 0；
    /// record_success 清零 → 满 5 重来
    #[test]
    fn remaining_attempts_decrements_and_locks_at_zero() {
        let mut rl = PinRateLimiter::new();
        let t = FakeClock(0);
        assert_eq!(
            rl.remaining_attempts("1.1.1.1", t.now()),
            5,
            "未见过 IP 满额"
        );
        for want in [4, 3, 2, 1] {
            rl.record_failure("1.1.1.1", t.now());
            assert_eq!(rl.remaining_attempts("1.1.1.1", t.now()), want);
        }
        rl.record_failure("1.1.1.1", t.now()); // 第 5 次：置锁
        assert_eq!(rl.remaining_attempts("1.1.1.1", t.now()), 0, "锁定期恒 0");
        rl.record_success("1.1.1.1");
        assert_eq!(
            rl.remaining_attempts("1.1.1.1", t.now()),
            5,
            "成功清零后满额重来"
        );
    }
}
