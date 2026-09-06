// APP 类会话状态推导共享核（issue #6）：
// WorkBuddy 与 Codex APP 两个工具的 JSONL 状态判定统一收敛于此——
// 各工具只写一个「格式翻译」适配器（平铺 JSONL / response_item 外壳 → AppEntryKind），
// 判定规则（尾部倒扫取第一条有语义条目 + 300s mtime 叠加）为唯一事实源。
// 不依赖任何工具协议细节；CLI 工具的消息模型判定（monitor::status::determine_status）不在此统一

use crate::session::SessionStatus;

/// 归一化 APP 会话条目（两个工具的 JSONL 格式翻译后的公共语义）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEntryKind {
    /// 用户消息（提问/指令）→ Thinking
    UserMessage,
    /// 助手纯文本消息（回合完成信号）→ Idle
    AssistantMessage,
    /// 工具调用（function_call / function_call_output / function_call_result）→ Processing
    ToolCall,
    /// 回合开始（Codex event_msg task_started）→ Processing
    TurnStart,
    /// 回合结束（Codex event_msg task_complete）→ Idle
    TurnEnd,
    /// 记账/系统条目（reasoning、token_count、item_completed、world_state、turn_context、
    /// thread_settings_applied、session_meta、developer 消息等）→ 不参与判定
    Other,
}

/// App 形态状态叠加阈值（spec §4「叠加 mtime 阈值（App 形态 300s，与 Codex APP 一致）」）：
/// JSONL mtime 停更超过该时长时，函数调用类尾部（Processing）降级为 Waiting
pub const APP_STATUS_STALE_MS: u64 = 300_000;

/// mtime 阈值叠加（spec §4）：函数调用类尾部停更 >= 300s 降级 Waiting；
/// assistant 文本（Idle）等其余状态不受影响。mtime 年龄不可知时按未过期处理（防御）
pub fn overlay_mtime_stale(status: SessionStatus, mtime_age_ms: u64) -> SessionStatus {
    match status {
        SessionStatus::Processing if mtime_age_ms >= APP_STATUS_STALE_MS => SessionStatus::Waiting,
        other => other,
    }
}

/// 判定核（纯函数）：从尾部倒扫，跳过 Other，取第一条有语义条目定状态——
/// 「最后一条说了什么就是什么」的推广（WorkBuddy 语义不变；Codex 修复点 =
/// 工具调用条目不再被无视，第二轮运行期不再恒判 Idle）。
/// entries 须按文件顺序（旧 → 新）传入，尾部 = 切片末端。
/// 无任何语义条目时返回 None，兜底留给各工具（Codex：文件新鲜 → Processing，
/// 停更 → Waiting；WorkBuddy：→ Waiting）——两边边界行为都不变
pub fn derive_app_status(entries: &[AppEntryKind]) -> Option<SessionStatus> {
    entries.iter().rev().find_map(|k| match k {
        AppEntryKind::UserMessage => Some(SessionStatus::Thinking),
        AppEntryKind::AssistantMessage => Some(SessionStatus::Idle),
        AppEntryKind::ToolCall => Some(SessionStatus::Processing),
        AppEntryKind::TurnStart => Some(SessionStatus::Processing),
        AppEntryKind::TurnEnd => Some(SessionStatus::Idle),
        AppEntryKind::Other => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 判定核：尾部倒扫取第一条有语义条目 ----

    #[test]
    fn user_message_tail_is_thinking() {
        assert_eq!(
            derive_app_status(&[AppEntryKind::UserMessage]),
            Some(SessionStatus::Thinking)
        );
    }

    #[test]
    fn assistant_message_tail_is_idle() {
        assert_eq!(
            derive_app_status(&[AppEntryKind::AssistantMessage]),
            Some(SessionStatus::Idle)
        );
    }

    #[test]
    fn tool_call_tail_is_processing() {
        assert_eq!(
            derive_app_status(&[AppEntryKind::ToolCall]),
            Some(SessionStatus::Processing)
        );
    }

    #[test]
    fn turn_start_tail_is_processing() {
        assert_eq!(
            derive_app_status(&[AppEntryKind::TurnStart]),
            Some(SessionStatus::Processing)
        );
    }

    #[test]
    fn turn_end_tail_is_idle() {
        assert_eq!(
            derive_app_status(&[AppEntryKind::TurnEnd]),
            Some(SessionStatus::Idle)
        );
    }

    #[test]
    fn last_semantic_entry_wins() {
        // 工具调用 → 助手文本：最后一条有语义条目（assistant）定状态
        let entries = [
            AppEntryKind::UserMessage,
            AppEntryKind::ToolCall,
            AppEntryKind::AssistantMessage,
        ];
        assert_eq!(derive_app_status(&entries), Some(SessionStatus::Idle));
        // 助手文本 → 工具调用：运行中
        let entries = [
            AppEntryKind::UserMessage,
            AppEntryKind::AssistantMessage,
            AppEntryKind::ToolCall,
        ];
        assert_eq!(derive_app_status(&entries), Some(SessionStatus::Processing));
    }

    #[test]
    fn bookkeeping_entries_are_skipped() {
        // 记账条目（Other）在尾不改变判定：其前一条有语义条目定状态
        let entries = [
            AppEntryKind::UserMessage,
            AppEntryKind::ToolCall,
            AppEntryKind::Other,
            AppEntryKind::Other,
        ];
        assert_eq!(derive_app_status(&entries), Some(SessionStatus::Processing));
        // 回合结束 + 尾部记账条目 → 仍为 Idle（顺带改善：旧实现误显运行中）
        let entries = [
            AppEntryKind::UserMessage,
            AppEntryKind::AssistantMessage,
            AppEntryKind::Other,
        ];
        assert_eq!(derive_app_status(&entries), Some(SessionStatus::Idle));
    }

    #[test]
    fn no_semantic_entry_returns_none() {
        assert_eq!(derive_app_status(&[]), None);
        assert_eq!(
            derive_app_status(&[AppEntryKind::Other, AppEntryKind::Other]),
            None
        );
    }

    // ---- 300s mtime 叠加（自 workbuddy_parser 迁入，语义不变）----

    #[test]
    fn processing_stale_downgrades_to_waiting() {
        // 函数调用类尾部 + JSONL 停更 >= 300s → Processing 降级 Waiting
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS),
            SessionStatus::Waiting
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS + 1),
            SessionStatus::Waiting
        );
    }

    #[test]
    fn processing_fresh_stays_processing() {
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS - 1),
            SessionStatus::Processing
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, 0),
            SessionStatus::Processing
        );
    }

    #[test]
    fn idle_stays_idle_regardless_of_mtime() {
        // assistant 纯文本尾部是明确完成信号：文件过旧也不拉回 Waiting（与 determine_status 语义一致）
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Idle, APP_STATUS_STALE_MS * 10),
            SessionStatus::Idle
        );
    }

    #[test]
    fn waiting_passes_through_unaffected() {
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Waiting, APP_STATUS_STALE_MS * 10),
            SessionStatus::Waiting
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Waiting, 0),
            SessionStatus::Waiting
        );
    }
}
