//! **注入能力自适应开关**（2026-10-03 用户设计）：注入形态在 TUI 里的消费行为是
//! 「(工具版本 × 注入方式)」的函数——同一个数字键，键盘敲入可能被直选、MAM 注入
//! 可能被忽略（反之亦然），且随工具版本漂移（低版本只认走位+空格、高版本支持数字
//! 直选——两个方向都有可能）。
//!
//! # 设计（用户裁决）
//!
//! - **内存开关表**，键 = `(tool_id, cli_version)`：探测一次终身（应用生命周期）
//!   复用；版本变化 → key 变化 → 自动重新探测（天然适配版本升级/降级）
//! - **不持久化**：MAM 重启重新探测，成本仅一次（首次 toggle 多花一节数字+核验）
//! - 探测纪律：首次先试数字 → 屏读核验翻转：
//!   - 翻转 → [`DigitToggle::Supported`]（后续数字直发）
//!   - 屏面**完全无变化** → [`DigitToggle::Unsupported`]（回退走位+空格，本次内完成）
//!   - **屏面有变化但未翻转**（数字被半消费：焦点动了/勾了别的行）= **异常状态**
//!     （用户拍板）→ 中止引导人工，不记开关不回退（回退走位会在未知状态上继续发键）
//!
//! # 表的所有权
//!
//! 表挂在 `RemoteState` 上（per-state，[`Table`]/[`new_table`]）——端点测试各自
//! 独立表避免并行探测写回互相污染；生产装配共享同一 Arc（全局一份）。
//!
//! 与 kimi 的数字禁令（'2'+Enter 误批准实证）不冲突：那是 kimi 批准框场景；本表
//! 仅 claude 多选 toggle，且每次数字都带翻转核验。

use std::collections::HashMap;
use std::sync::Mutex;

/// 数字直选能力（多选 toggle 选项勾选的注入路径选择）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitToggle {
    /// 未探测：首次 toggle 先试数字
    Unknown,
    /// 数字直选可用：选项勾选 = 数字直发 + 翻转核验（不走位不空格）
    Supported,
    /// 数字无效：走位 + 空格（既有编排）
    Unsupported,
}

/// 能力开关表句柄：`RemoteState` 持有（per-state；生产装配共享同一 Arc）。
pub type Table = std::sync::Arc<Mutex<HashMap<(String, String), DigitToggle>>>;

/// 新建空表。
pub fn new_table() -> Table {
    std::sync::Arc::new(Mutex::new(HashMap::new()))
}

/// 取当前 (tool, version) 的能力值（无记录 → [`DigitToggle::Unknown`]）。
pub fn digit_toggle_in(table: &Table, tool: &str, version: &str) -> DigitToggle {
    let guard = table.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get(&(tool.to_string(), version.to_string()))
        .copied()
        .unwrap_or(DigitToggle::Unknown)
}

/// 写回探测/降级结果。
pub fn set_digit_toggle_in(table: &Table, tool: &str, version: &str, cap: DigitToggle) {
    let mut guard = table.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert((tool.to_string(), version.to_string()), cap);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_default_and_roundtrip() {
        let table = new_table();
        assert_eq!(
            digit_toggle_in(&table, "claude-test-rt", "9.9.9"),
            DigitToggle::Unknown
        );
        set_digit_toggle_in(&table, "claude-test-rt", "9.9.9", DigitToggle::Supported);
        assert_eq!(
            digit_toggle_in(&table, "claude-test-rt", "9.9.9"),
            DigitToggle::Supported
        );
        set_digit_toggle_in(&table, "claude-test-rt", "9.9.9", DigitToggle::Unknown);
        assert_eq!(
            digit_toggle_in(&table, "claude-test-rt", "9.9.9"),
            DigitToggle::Unknown
        );
    }

    /// 版本进 key：不同版本各自探测各自记（评审设计核心——版本漂移自适应）。
    #[test]
    fn version_keys_are_isolated() {
        let table = new_table();
        set_digit_toggle_in(&table, "claude-test-iso", "1.0", DigitToggle::Supported);
        set_digit_toggle_in(&table, "claude-test-iso", "2.0", DigitToggle::Unsupported);
        assert_eq!(
            digit_toggle_in(&table, "claude-test-iso", "1.0"),
            DigitToggle::Supported
        );
        assert_eq!(
            digit_toggle_in(&table, "claude-test-iso", "2.0"),
            DigitToggle::Unsupported
        );
    }
}
