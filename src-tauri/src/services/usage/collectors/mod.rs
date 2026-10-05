//! 7 个采集器的登记处（Task 10 占位 → Task 11 起逐个登记：claude → codex → kimi → opencode
//! → workbuddy → zcode → dsh）。
//!
//! **纪律（登记表是 7 个采集器任务共同的交汇点）**：每个采集器任务在写完自己的 `collect()`
//! 之后，**必须回到这里把 `Box::new(XxxCollector)` 加进 `all()`**（每个任务都给了改后的
//! 完整函数体）。漏登记的后果**不是编译错，而是静默不采**：`run_collection` 只遍历 `all()`，
//! 于是 Task 23B 的 `sources.len() == 7` 与 Task 24 的用户可见验收面各露一次馅
//! （独立验证复核 §3.2.2 阻塞 4）。每个采集器文件的测试模块里都有一条
//! `collector_is_registered_in_all()` 就地锁住自己那一行。
use super::collect::UsageCollector;

/// 采集器登记表（登记顺序 = 采集顺序 = 契约 `UsageSourceStatus` 顺序）
/// 本任务返回空表：`run_collection` 会得到 `sources: []`，不影响本任务断言。
pub fn all() -> Vec<Box<dyn UsageCollector>> {
    Vec::new()
}
