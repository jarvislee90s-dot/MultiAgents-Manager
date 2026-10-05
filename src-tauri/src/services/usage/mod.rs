// 用量采集与存储底座（计划①）：契约 §2 值对象 + 口径层 + 7 采集器 + 查询 + 账本落库。
// 纪律：本域的一切文件读取只经 stream.rs::read_incremental；一切口径计算只在
// semantics / dedup / project / provider / range 五个纯函数模块里（采集器只上报原始字段）。
// 模块声明随任务增量补齐（每个任务在 Files 里写明自己要 append 的那一行），
// 避免出现「声明了但文件还没建」的中间态编译失败。
pub mod delta;
pub mod error;
pub mod ledger;
pub mod model;
pub mod semantics;

pub use error::UsageError;
pub use model::*;

/// 供应商不可得时的行标签 = **i18n 键**（zh/en 同父同键，前端 `t(label)` 渲染）。
/// **不得**在 Rust 侧硬编码任何中文文案（W5）；CSV 导出不落 i18n 键（该列留空，见 Task 19）。
pub const UNKNOWN_PROVIDER_LABEL_KEY: &str = "usage.label.unknownProvider";

/// 当前毫秒（全域唯一时钟入口，便于测试注入）
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
