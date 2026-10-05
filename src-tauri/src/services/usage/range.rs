//! 时间范围与桶（说明书 §P6 / §P8 / §10）。
//! 五档：近 5 小时（最近 5 个**完整整点桶**，不含当前小时，整点锚定不做秒级滚动）/
//! 当日（0 时至当前，小时桶）/ 近 7 天 / 近 30 天（含今日滚动窗口，日桶）/
//! 自定义（日桶，跨度 ≤31 天含首尾，超限截断为最近 31 天并常驻提示）。
//! 跨源日界：优先各源自带时区（codex `turn_context.timezone`），缺失回退宿主本地。
use chrono::{Datelike, Local, TimeZone, Timelike};

use super::error::UsageError;
use super::model::{UsageRange, UsageRangePreset};

/// 自定义区间最大跨度（天，含首尾）
pub const MAX_CUSTOM_SPAN_DAYS: i64 = 31;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;

/// 记录级时区：源自带 IANA 名（能解析则用）或宿主本地
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTz {
    HostLocal,
    Named(chrono_tz::Tz),
}

impl SourceTz {
    pub fn from_name(name: Option<&str>) -> Self {
        match name.map(str::trim).filter(|s| !s.is_empty()) {
            Some(n) => n
                .parse::<chrono_tz::Tz>()
                .map(SourceTz::Named)
                .unwrap_or(SourceTz::HostLocal),
            None => SourceTz::HostLocal,
        }
    }
}

pub fn hour_key_of(ts_ms: i64, tz: &SourceTz) -> String {
    match tz {
        SourceTz::HostLocal => Local
            .timestamp_millis_opt(ts_ms)
            .single()
            .map(|t| {
                format!(
                    "{:04}-{:02}-{:02}T{:02}",
                    t.year(),
                    t.month(),
                    t.day(),
                    t.hour()
                )
            })
            .unwrap_or_default(),
        SourceTz::Named(z) => z
            .timestamp_millis_opt(ts_ms)
            .single()
            .map(|t| {
                format!(
                    "{:04}-{:02}-{:02}T{:02}",
                    t.year(),
                    t.month(),
                    t.day(),
                    t.hour()
                )
            })
            .unwrap_or_default(),
    }
}

pub fn day_key_of(ts_ms: i64, tz: &SourceTz) -> String {
    let h = hour_key_of(ts_ms, tz);
    h.chars().take(10).collect()
}

pub fn hour_key_of_host(ts_ms: i64) -> String {
    hour_key_of(ts_ms, &SourceTz::HostLocal)
}

pub fn day_key_of_host(ts_ms: i64) -> String {
    day_key_of(ts_ms, &SourceTz::HostLocal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeGranularity {
    Hour,
    Day,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeBuckets {
    pub granularity: RangeGranularity,
    /// 升序桶键：小时档 `"YYYY-MM-DDTHH"`、日档 `"YYYY-MM-DD"`（字典序 = 时间序）
    pub keys: Vec<String>,
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrevPeriod {
    pub granularity: RangeGranularity,
    pub keys: Vec<String>,
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRange {
    pub preset: UsageRangePreset,
    pub cur: TimeBuckets,
    pub prev: Option<PrevPeriod>,
    /// 自定义超 31 天被截断（UI 常驻提示「已截断为最近 31 天」）
    pub truncated: bool,
}

fn local_date(y: i32, m: u32, d: u32, h: u32) -> Option<i64> {
    Local
        .with_ymd_and_hms(y, m, d, h, 0, 0)
        .single()
        .map(|t| t.timestamp_millis())
}

fn start_of_hour(now_ms: i64) -> i64 {
    match Local.timestamp_millis_opt(now_ms).single() {
        Some(t) => t
            .with_minute(0)
            .and_then(|x| x.with_second(0))
            .and_then(|x| x.with_nanosecond(0))
            .map(|x| x.timestamp_millis())
            .unwrap_or(now_ms),
        None => now_ms,
    }
}

fn start_of_day(now_ms: i64) -> i64 {
    match Local.timestamp_millis_opt(now_ms).single() {
        Some(t) => t
            .with_hour(0)
            .and_then(|x| x.with_minute(0))
            .and_then(|x| x.with_second(0))
            .and_then(|x| x.with_nanosecond(0))
            .map(|x| x.timestamp_millis())
            .unwrap_or(now_ms),
        None => now_ms,
    }
}

fn shift_days(day_key: &str, delta: i64) -> String {
    let base = chrono::NaiveDate::parse_from_str(day_key, "%Y-%m-%d").ok();
    match base.and_then(|d| d.checked_add_signed(chrono::Duration::days(delta))) {
        Some(d) => d.format("%Y-%m-%d").to_string(),
        None => day_key.to_string(),
    }
}

fn hour_keys_between(from_ms: i64, count: i64, inclusive_last: bool) -> Vec<String> {
    // from_ms 必须是整点起点；count 为桶数
    let n = if inclusive_last { count + 1 } else { count };
    (0..n)
        .map(|i| hour_key_of_host(from_ms + i * HOUR_MS))
        .collect()
}

fn day_keys_between(from_day: &str, count: i64) -> Vec<String> {
    (0..count).map(|i| shift_days(from_day, i)).collect()
}

pub fn resolve_range(range: &UsageRange, now_ms: i64) -> Result<ResolvedRange, UsageError> {
    let hour0 = start_of_hour(now_ms);
    let day0_ms = start_of_day(now_ms);
    let today = day_key_of_host(now_ms);

    let (cur, prev, truncated) = match range.preset {
        UsageRangePreset::Last5h => {
            let from = hour0 - 5 * HOUR_MS;
            let cur = TimeBuckets {
                granularity: RangeGranularity::Hour,
                keys: hour_keys_between(from, 5, false),
                from_ms: from,
                to_ms: hour0,
            };
            let prev_from = from - 5 * HOUR_MS;
            let prev = PrevPeriod {
                granularity: RangeGranularity::Hour,
                keys: hour_keys_between(prev_from, 5, false),
                from_ms: prev_from,
                to_ms: from,
            };
            (cur, Some(prev), false)
        }
        UsageRangePreset::Today => {
            let cur = TimeBuckets {
                granularity: RangeGranularity::Hour,
                keys: hour_keys_between(day0_ms, (hour0 - day0_ms) / HOUR_MS, true),
                from_ms: day0_ms,
                to_ms: now_ms,
            };
            // 上一周期 = 昨日同一时段（0 时到昨日同一时刻）
            let prev_from = day0_ms - DAY_MS;
            let prev_to = prev_from + (now_ms - day0_ms);
            let prev = PrevPeriod {
                granularity: RangeGranularity::Hour,
                keys: hour_keys_between(prev_from, (prev_to - prev_from) / HOUR_MS, true),
                from_ms: prev_from,
                to_ms: prev_to,
            };
            (cur, Some(prev), false)
        }
        UsageRangePreset::Last7d | UsageRangePreset::Last30d => {
            let n = if range.preset == UsageRangePreset::Last7d {
                7
            } else {
                30
            };
            let first = shift_days(&today, -(n - 1));
            let cur = TimeBuckets {
                granularity: RangeGranularity::Day,
                keys: day_keys_between(&first, n),
                from_ms: day0_ms - (n - 1) * DAY_MS,
                to_ms: now_ms,
            };
            let prev_first = shift_days(&first, -n);
            let prev = PrevPeriod {
                granularity: RangeGranularity::Day,
                keys: day_keys_between(&prev_first, n),
                from_ms: cur.from_ms - n * DAY_MS,
                to_ms: cur.from_ms,
            };
            (cur, Some(prev), false)
        }
        UsageRangePreset::Custom => {
            let from = range
                .from
                .as_deref()
                .and_then(|s| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
                .ok_or_else(|| {
                    UsageError::new("usage-range-invalid", "自定义区间缺少合法的 from")
                })?;
            let to = range
                .to
                .as_deref()
                .and_then(|s| chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
                .ok_or_else(|| UsageError::new("usage-range-invalid", "自定义区间缺少合法的 to"))?;
            if to < from {
                return Err(UsageError::new("usage-range-invalid", "from 晚于 to"));
            }
            let span = (to - from).num_days() + 1;
            let truncated = span > MAX_CUSTOM_SPAN_DAYS;
            let (first, last) = if truncated {
                (
                    shift_days(
                        &to.format("%Y-%m-%d").to_string(),
                        -(MAX_CUSTOM_SPAN_DAYS - 1),
                    ),
                    to.format("%Y-%m-%d").to_string(),
                )
            } else {
                (
                    from.format("%Y-%m-%d").to_string(),
                    to.format("%Y-%m-%d").to_string(),
                )
            };
            let n = if truncated {
                MAX_CUSTOM_SPAN_DAYS
            } else {
                span
            };
            let cur = TimeBuckets {
                granularity: RangeGranularity::Day,
                keys: day_keys_between(&first, n),
                from_ms: local_date_of_day(&first),
                to_ms: local_date_of_day(&shift_days(&last, 1)),
            };
            let prev_first = shift_days(&first, -n);
            let prev = PrevPeriod {
                granularity: RangeGranularity::Day,
                keys: day_keys_between(&prev_first, n),
                from_ms: local_date_of_day(&prev_first),
                to_ms: cur.from_ms,
            };
            (cur, Some(prev), truncated)
        }
    };

    Ok(ResolvedRange {
        preset: range.preset,
        cur,
        prev,
        truncated,
    })
}

/// 日键 → 当日 0 点毫秒（宿主本地；解析失败回退 0）
fn local_date_of_day(day_key: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(day_key, "%Y-%m-%d")
        .ok()
        .and_then(|d| local_date(d.year(), d.month(), d.day(), 0))
        .unwrap_or(0)
}

pub fn hour_label(key: &str) -> String {
    key.split_once('T')
        .map(|(_, h)| format!("{}:00", h))
        .unwrap_or_else(|| key.to_string())
}

pub fn day_label(key: &str) -> String {
    let mut it = key.split('-');
    match (it.next(), it.next(), it.next()) {
        (Some(_y), Some(m), Some(d)) => format!(
            "{}/{}",
            m.trim_start_matches('0'),
            d.trim_start_matches('0')
        ),
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::usage::model::UsageRangePreset;

    fn local_ms(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        chrono::Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .single()
            .expect("测试时刻必须存在（避开 DST 跳变）")
            .timestamp_millis()
    }
    fn range(preset: UsageRangePreset) -> UsageRange {
        UsageRange {
            preset,
            from: None,
            to: None,
        }
    }

    /// 说明书 §P6 第 1 条：窗口 = **最近 5 个完整整点桶，不含进行中的当前小时**。
    /// 现在 14:37 → 窗口 = 09:00–14:00（半开），桶 09/10/11/12/13。
    #[test]
    fn last5h_is_five_complete_hour_buckets() {
        let now = local_ms(2026, 10, 3, 14, 37);
        let r = resolve_range(&range(UsageRangePreset::Last5h), now).unwrap();
        assert_eq!(r.cur.granularity, RangeGranularity::Hour);
        assert_eq!(
            r.cur.keys,
            vec![
                "2026-10-03T09",
                "2026-10-03T10",
                "2026-10-03T11",
                "2026-10-03T12",
                "2026-10-03T13"
            ]
        );
        assert_eq!(
            r.cur.to_ms,
            local_ms(2026, 10, 3, 14, 0),
            "窗口右端 = 当前整点"
        );
        assert_eq!(r.cur.from_ms, local_ms(2026, 10, 3, 9, 0));
    }

    /// 说明书 §P6 第 2 条：**整点锚定、不做秒级滚动**——同一小时内任何时刻得到同一窗口。
    #[test]
    fn last5h_does_not_roll_by_seconds() {
        let a = resolve_range(
            &range(UsageRangePreset::Last5h),
            local_ms(2026, 10, 3, 14, 0),
        )
        .unwrap();
        let b = resolve_range(
            &range(UsageRangePreset::Last5h),
            local_ms(2026, 10, 3, 14, 37),
        )
        .unwrap();
        let c = resolve_range(
            &range(UsageRangePreset::Last5h),
            local_ms(2026, 10, 3, 14, 59),
        )
        .unwrap();
        assert_eq!(a.cur.keys, b.cur.keys);
        assert_eq!(b.cur.keys, c.cur.keys);
        // 仅比键不足以抓住「秒级滚动」：键是**格式化**出来的，分钟被格式化掉后仍然相同
        // （变异测试实证：把 start_of_hour 的分钟截断去掉，上面两条断言**全绿**）。
        // 因此必须把**窗口边界本身**也钉死——否则「不做秒级滚动」这条判据是假绿。
        assert_eq!(a.cur.to_ms, b.cur.to_ms, "同一小时内窗口右端不得随秒推进");
        assert_eq!(b.cur.to_ms, c.cur.to_ms, "同一小时内窗口右端不得随秒推进");
        assert_eq!(
            a.cur.from_ms, b.cur.from_ms,
            "同一小时内窗口左端不得随秒推进"
        );
        assert_eq!(
            b.cur.from_ms, c.cur.from_ms,
            "同一小时内窗口左端不得随秒推进"
        );
    }

    /// 跨整点：窗口整体前移一格（14:59 → 仍 09–13；15:00 → 10–14）
    #[test]
    fn last5h_shifts_on_hour_boundary() {
        let before = resolve_range(
            &range(UsageRangePreset::Last5h),
            local_ms(2026, 10, 3, 14, 59),
        )
        .unwrap();
        let after = resolve_range(
            &range(UsageRangePreset::Last5h),
            local_ms(2026, 10, 3, 15, 0),
        )
        .unwrap();
        assert_eq!(before.cur.keys.last().unwrap(), "2026-10-03T13");
        assert_eq!(after.cur.keys.first().unwrap(), "2026-10-03T10");
        assert_eq!(after.cur.keys.last().unwrap(), "2026-10-03T14");
        // 跨整点必须是**整体平移一格**（而不是只换键、边界仍粘在 14:59/14:00）
        assert_eq!(
            after.cur.to_ms - before.cur.to_ms,
            HOUR_MS,
            "右端整体前移一个整点"
        );
        assert_eq!(
            after.cur.from_ms - before.cur.from_ms,
            HOUR_MS,
            "左端整体前移一个整点"
        );
    }

    /// 凌晨跨日回绕（说明书 §10 明文要求）：02:10 → 前一日 21/22/23 + 当日 00/01
    #[test]
    fn last5h_wraps_across_midnight() {
        let now = local_ms(2026, 1, 15, 2, 10);
        let r = resolve_range(&range(UsageRangePreset::Last5h), now).unwrap();
        assert_eq!(
            r.cur.keys,
            vec![
                "2026-01-14T21",
                "2026-01-14T22",
                "2026-01-14T23",
                "2026-01-15T00",
                "2026-01-15T01"
            ]
        );
        // 上一周期 = 再往前 5 个整点桶（继续回绕到 1-14 的 16–20 点）
        let prev = r.prev.unwrap();
        assert_eq!(prev.keys.first().unwrap(), "2026-01-14T16");
        assert_eq!(prev.keys.last().unwrap(), "2026-01-14T20");
    }

    /// 「当日」= 自然日 0 时至当前（小时桶，含进行中的当前小时）
    #[test]
    fn today_covers_midnight_to_current_hour() {
        let r =
            resolve_range(&range(UsageRangePreset::Today), local_ms(2026, 10, 3, 2, 5)).unwrap();
        assert_eq!(
            r.cur.keys,
            vec!["2026-10-03T00", "2026-10-03T01", "2026-10-03T02"]
        );
        // 上一周期 = 昨日 0 时至昨日同一时刻
        let prev = r.prev.unwrap();
        assert_eq!(
            prev.keys,
            vec!["2026-10-02T00", "2026-10-02T01", "2026-10-02T02"]
        );
    }

    /// 近 7 天 / 近 30 天 = **含今日在内的滚动窗口**，日粒度
    #[test]
    fn rolling_day_windows_include_today() {
        let now = local_ms(2026, 10, 3, 12, 0);
        let w = resolve_range(&range(UsageRangePreset::Last7d), now).unwrap();
        assert_eq!(w.cur.granularity, RangeGranularity::Day);
        assert_eq!(w.cur.keys.len(), 7);
        assert_eq!(w.cur.keys.first().unwrap(), "2026-09-27");
        assert_eq!(w.cur.keys.last().unwrap(), "2026-10-03");
        let m = resolve_range(&range(UsageRangePreset::Last30d), now).unwrap();
        assert_eq!(m.cur.keys.len(), 30);
        assert_eq!(m.cur.keys.first().unwrap(), "2026-09-04");
        // 上一周期：7d → 再往前 7 天；30d → 再往前 30 天
        assert_eq!(w.prev.unwrap().keys.first().unwrap(), "2026-09-20");
        assert_eq!(m.prev.unwrap().keys.first().unwrap(), "2026-08-05");
    }

    /// 自定义：跨度 ≤31 天（含首尾）；超限截断为最近 31 天并置 truncated
    #[test]
    fn custom_range_truncates_to_31_days() {
        let now = local_ms(2026, 10, 3, 12, 0);
        let ok = resolve_range(
            &UsageRange {
                preset: UsageRangePreset::Custom,
                from: Some("2026-09-20".into()),
                to: Some("2026-10-03".into()),
            },
            now,
        )
        .unwrap();
        assert!(!ok.truncated);
        assert_eq!(ok.cur.keys.len(), 14);

        let over = resolve_range(
            &UsageRange {
                preset: UsageRangePreset::Custom,
                from: Some("2026-01-01".into()),
                to: Some("2026-10-03".into()),
            },
            now,
        )
        .unwrap();
        assert!(over.truncated, ">31 天必须截断并置位");
        assert_eq!(over.cur.keys.len(), 31);
        assert_eq!(
            over.cur.keys.last().unwrap(),
            "2026-10-03",
            "保留最近 31 天"
        );
        assert_eq!(over.cur.keys.first().unwrap(), "2026-09-03");
        // 自定义的上一周期 = 当前区间等长的前一段
        let prev = over.prev.unwrap();
        assert_eq!(prev.keys.len(), 31);
        assert_eq!(prev.keys.last().unwrap(), "2026-09-02");
    }

    /// 非法自定义：缺 from/to、格式坏、from > to → usage-range-invalid（不得静默回落当日）
    #[test]
    fn custom_range_validates_input() {
        let now = local_ms(2026, 10, 3, 12, 0);
        for bad in [
            UsageRange {
                preset: UsageRangePreset::Custom,
                from: None,
                to: Some("2026-10-03".into()),
            },
            UsageRange {
                preset: UsageRangePreset::Custom,
                from: Some("2026-13-99".into()),
                to: Some("2026-10-03".into()),
            },
            UsageRange {
                preset: UsageRangePreset::Custom,
                from: Some("2026-10-03".into()),
                to: Some("2026-09-01".into()),
            },
        ] {
            let e = resolve_range(&bad, now).unwrap_err();
            assert_eq!(e.code, "usage-range-invalid");
        }
    }

    /// 跨源日界（§P6）：优先用各源自带时区（codex `turn_context.timezone`），
    /// 缺失才回退宿主本地时区。测试用两个**显式**时区名，与宿主时区无关。
    #[test]
    fn day_key_follows_source_timezone() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-03T16:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            day_key_of(ts, &SourceTz::from_name(Some("UTC"))),
            "2026-10-03"
        );
        assert_eq!(
            day_key_of(ts, &SourceTz::from_name(Some("Asia/Shanghai"))),
            "2026-10-04",
            "+8 时区下已跨日"
        );
        assert_eq!(
            hour_key_of(ts, &SourceTz::from_name(Some("Asia/Shanghai"))),
            "2026-10-04T00"
        );
        // 非法/缺失时区名 → 回退宿主本地（不 panic、不报错）
        assert!(matches!(
            SourceTz::from_name(Some("Not/AZone")),
            SourceTz::HostLocal
        ));
        assert!(matches!(SourceTz::from_name(None), SourceTz::HostLocal));
    }

    /// 展示标签：小时档 "HH:00"、日档 "M/D"（契约 §2 TrendPoint.label）
    #[test]
    fn trend_labels_match_contract() {
        assert_eq!(hour_label("2026-10-03T09"), "09:00");
        assert_eq!(day_label("2026-10-03"), "10/3");
    }

    // ------------------------------------------------------------------
    // 以下 5 条为 Task 8 实现者在任务书 10 条之外**增补**的回归锁（见报告「偏差申报」）。
    // 理由：任务书点名的判据里，桶粒度（today/custom 两档）、环比等长（五档全覆盖、
    // 含 ms 边界）、自定义 31 天**边界值**（30/31/32 天）、时区回退/越界降级、
    // 宿主键包装函数，原先都没有能变红的断言（Task 5/6/7 的「命门一半没锁」同族风险）。
    // ------------------------------------------------------------------

    /// 环比（D14）= **紧邻当前区间之前的等长区间**：五档逐一断言
    /// ① 粒度一致 ② 桶数等长 ③ 小时档/滚动日档/自定义档首尾相接
    /// ④ 小时档/自定义档/当日档 ms 跨度严格等长（当日档 = 昨日同一时段，与今日此刻对齐）。
    #[test]
    fn prev_period_is_equal_length_and_adjacent_for_all_presets() {
        let now = local_ms(2026, 10, 3, 14, 37);
        let cases = [
            range(UsageRangePreset::Last5h),
            range(UsageRangePreset::Today),
            range(UsageRangePreset::Last7d),
            range(UsageRangePreset::Last30d),
            UsageRange {
                preset: UsageRangePreset::Custom,
                // 刻意选一段**全时区都无夏令时切换**的日期（2026-05-10..05-23），
                // 使下面那条 ms 等长断言与宿主时区无关（确定性）
                from: Some("2026-05-10".into()),
                to: Some("2026-05-23".into()),
            },
        ];
        for r in cases {
            let res = resolve_range(&r, now).unwrap();
            assert_eq!(
                res.preset, r.preset,
                "ResolvedRange.preset 必须回显入参档位"
            );
            assert!(
                !res.truncated,
                "{:?} 不是超限自定义区间，truncated 必须为 false",
                r.preset
            );
            let prev = res.prev.expect("D14：五档时间范围都要能算环比");
            assert_eq!(
                prev.granularity, res.cur.granularity,
                "{:?} 环比桶粒度必须与当前档一致",
                r.preset
            );
            assert_eq!(
                prev.keys.len(),
                res.cur.keys.len(),
                "{:?} 上一周期必须与当前周期等长（桶数）",
                r.preset
            );

            // 「紧邻」：小时档 / 滚动日档 / 自定义档的上一周期右端 = 当前周期左端
            if matches!(
                r.preset,
                UsageRangePreset::Last5h
                    | UsageRangePreset::Last7d
                    | UsageRangePreset::Last30d
                    | UsageRangePreset::Custom
            ) {
                assert_eq!(
                    prev.to_ms, res.cur.from_ms,
                    "{:?} 上一周期必须紧邻当前区间（不留缝、不重叠）",
                    r.preset
                );
            }
            // 「等长」：滚动日档 = 当前桶数个**整日**（ms 纯算术，与夏令时无关）
            if matches!(
                r.preset,
                UsageRangePreset::Last7d | UsageRangePreset::Last30d
            ) {
                assert_eq!(
                    prev.to_ms - prev.from_ms,
                    res.cur.keys.len() as i64 * DAY_MS,
                    "{:?} 上一周期必须是当前桶数个整日（等长）",
                    r.preset
                );
            }
            // 「等长」：小时档（5×1h）、自定义档（同桶数的整日跨度）、当日档（0 时→昨日同一时刻）
            if matches!(
                r.preset,
                UsageRangePreset::Last5h | UsageRangePreset::Custom | UsageRangePreset::Today
            ) {
                assert_eq!(
                    prev.to_ms - prev.from_ms,
                    res.cur.to_ms - res.cur.from_ms,
                    "{:?} 环比区间必须与当前区间严格等长（ms）",
                    r.preset
                );
            }
        }
    }

    /// 自定义跨度上限 31 天（含首尾）的**边界值**：30/31 天不截断、32 天截断、单日 = 1 桶。
    #[test]
    fn custom_span_boundary_is_31_days_inclusive() {
        assert_eq!(
            MAX_CUSTOM_SPAN_DAYS, 31,
            "说明书 §P6：自定义跨度 ≤31 天（含首尾）"
        );
        let now = local_ms(2026, 10, 3, 12, 0);
        let custom = |from: &str, to: &str| {
            resolve_range(
                &UsageRange {
                    preset: UsageRangePreset::Custom,
                    from: Some(from.into()),
                    to: Some(to.into()),
                },
                now,
            )
            .unwrap()
        };

        // 单日（含首尾 = 1 桶）；上一周期 = 前一日
        let one = custom("2026-10-03", "2026-10-03");
        assert!(!one.truncated);
        assert_eq!(one.cur.keys, vec!["2026-10-03"]);
        assert_eq!(one.prev.unwrap().keys, vec!["2026-10-02"]);

        // 30 天（含首尾）不截断
        let d30 = custom("2026-09-04", "2026-10-03");
        assert!(!d30.truncated);
        assert_eq!(d30.cur.keys.len(), 30);
        assert_eq!(d30.cur.keys.first().unwrap(), "2026-09-04");

        // 31 天（含首尾）= 上限，正好不截断
        let d31 = custom("2026-09-03", "2026-10-03");
        assert!(!d31.truncated, "31 天（含首尾）是允许的上限，不得截断");
        assert_eq!(d31.cur.keys.len(), 31);
        assert_eq!(d31.cur.keys.first().unwrap(), "2026-09-03");

        // 32 天 → 截断为最近 31 天，保留 to 端；上一周期仍等长（31 桶）
        let d32 = custom("2026-09-02", "2026-10-03");
        assert!(d32.truncated, "32 天必须截断");
        assert_eq!(d32.cur.keys.len(), 31);
        assert_eq!(d32.cur.keys.first().unwrap(), "2026-09-03");
        assert_eq!(d32.cur.keys.last().unwrap(), "2026-10-03");
        assert_eq!(d32.prev.unwrap().keys.len(), 31);
    }

    /// 宿主键包装函数（`ledger.rs` 的 `cutoff_day` 回改后直接依赖这两个入口）：
    /// ① 必须等于显式 `HostLocal` 分支 ② 日键 = 小时键的日期前缀（两档不得分叉）
    /// ③ 越界时间戳给空串（与 ledger 原内联实现同款降级，保证回改**行为不变**）。
    #[test]
    fn host_key_helpers_delegate_to_host_local_and_degrade_out_of_range() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-03T16:30:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(hour_key_of_host(ts), hour_key_of(ts, &SourceTz::HostLocal));
        assert_eq!(day_key_of_host(ts), day_key_of(ts, &SourceTz::HostLocal));

        for tz in [
            SourceTz::HostLocal,
            SourceTz::from_name(Some("Asia/Shanghai")),
            SourceTz::from_name(Some("UTC")),
            SourceTz::from_name(Some("America/Los_Angeles")),
        ] {
            let h = hour_key_of(ts, &tz);
            assert_eq!(h.len(), 13, "小时键形态 = YYYY-MM-DDTHH：{h}");
            assert_eq!(h.as_bytes()[10], b'T', "小时键第 11 位必须是 T：{h}");
            assert_eq!(
                day_key_of(ts, &tz),
                h[..10].to_string(),
                "日键必须是小时键的日期前缀"
            );
        }
        // 越界时间戳：Local/Tz 换算均不可表示 → 空串（不得 panic、不得回落 0 或当日）
        assert_eq!(hour_key_of_host(i64::MAX), "");
        assert_eq!(day_key_of_host(i64::MAX), "");
        assert_eq!(day_key_of(i64::MIN, &SourceTz::from_name(Some("UTC"))), "");
        assert_eq!(day_key_of_host(i64::MIN), "");
    }

    /// 跨源日界必须是**各源 IANA 时区**，不得退化成宿主时区：同一瞬间在日界两侧的源
    /// 必须落不同日键；非法/空/纯空白名一律回退 HostLocal（不 panic）。
    #[test]
    fn source_timezone_boundaries_are_iana_not_host() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-10-03T16:30:00Z")
            .unwrap()
            .timestamp_millis();
        let utc = SourceTz::from_name(Some("UTC"));
        let shanghai = SourceTz::from_name(Some("Asia/Shanghai")); // UTC+8
        let la = SourceTz::from_name(Some("America/Los_Angeles")); // UTC-7（2026-10-03 仍在 PDT）
        let kiritimati = SourceTz::from_name(Some("Pacific/Kiritimati")); // UTC+14

        assert_eq!(day_key_of(ts, &utc), "2026-10-03");
        assert_eq!(day_key_of(ts, &la), "2026-10-03");
        assert_eq!(hour_key_of(ts, &la), "2026-10-03T09");
        assert_eq!(day_key_of(ts, &shanghai), "2026-10-04");
        assert_eq!(hour_key_of(ts, &kiritimati), "2026-10-04T06");
        assert_ne!(
            day_key_of(ts, &la),
            day_key_of(ts, &kiritimati),
            "跨日界两侧的源必须落不同日键（退化用宿主时区就会相等）"
        );

        // IANA 名解析：合法名 = Named；首尾空白裁剪后仍可解析
        assert_eq!(
            SourceTz::from_name(Some("UTC")),
            SourceTz::Named(chrono_tz::Tz::UTC)
        );
        assert_eq!(SourceTz::from_name(Some("  Asia/Shanghai  ")), shanghai);
        assert_ne!(SourceTz::Named(chrono_tz::Tz::UTC), SourceTz::HostLocal);
        // 非法 / 空串 / 纯空白 / 缺失 → HostLocal（回退，不是 panic、不是错误）
        assert!(matches!(
            SourceTz::from_name(Some("Not/AZone")),
            SourceTz::HostLocal
        ));
        assert!(matches!(SourceTz::from_name(Some("")), SourceTz::HostLocal));
        assert!(matches!(
            SourceTz::from_name(Some("   ")),
            SourceTz::HostLocal
        ));
        assert!(matches!(SourceTz::from_name(None), SourceTz::HostLocal));
    }

    /// 桶粒度逐档对齐契约 §C：`last5h` / `today` → 小时桶 `YYYY-MM-DDTHH`；
    /// `last7d` / `last30d` / `custom` → 日桶 `YYYY-MM-DD`；桶键升序且不重复
    /// （字典序 = 时间序，是趋势区按 key 排序的前提）。
    #[test]
    fn bucket_granularity_matches_contract_for_every_preset() {
        let now = local_ms(2026, 10, 3, 14, 37);
        let cases = [
            (range(UsageRangePreset::Last5h), RangeGranularity::Hour),
            (range(UsageRangePreset::Today), RangeGranularity::Hour),
            (range(UsageRangePreset::Last7d), RangeGranularity::Day),
            (range(UsageRangePreset::Last30d), RangeGranularity::Day),
            (
                UsageRange {
                    preset: UsageRangePreset::Custom,
                    from: Some("2026-09-20".into()),
                    to: Some("2026-10-03".into()),
                },
                RangeGranularity::Day,
            ),
        ];
        for (r, want) in cases {
            let res = resolve_range(&r, now).unwrap();
            assert_eq!(
                res.cur.granularity, want,
                "{:?} 的桶粒度与契约 §C 不符",
                r.preset
            );
            assert!(!res.cur.keys.is_empty(), "{:?} 至少要有一个桶", r.preset);
            for k in &res.cur.keys {
                match want {
                    RangeGranularity::Hour => {
                        assert_eq!(k.len(), 13, "小时桶键 = YYYY-MM-DDTHH：{k}");
                        assert_eq!(k.as_bytes()[10], b'T', "小时桶键第 11 位必须是 T：{k}");
                    }
                    RangeGranularity::Day => {
                        assert_eq!(k.len(), 10, "日桶键 = YYYY-MM-DD：{k}");
                        assert!(!k.contains('T'), "日桶键不得含 T：{k}");
                    }
                }
            }
            let mut sorted = res.cur.keys.clone();
            sorted.sort();
            assert_eq!(
                sorted, res.cur.keys,
                "{:?} 桶键必须升序（字典序 = 时间序）",
                r.preset
            );
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                res.cur.keys.len(),
                "{:?} 桶键不得重复",
                r.preset
            );
        }
    }
}
