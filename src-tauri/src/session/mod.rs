pub mod model;

pub use model::{AgentType, Session, SessionStatus, SessionsResponse, ProcessForm};

/// 状态排序优先级（数字越小越靠前）
pub fn status_sort_priority(status: &SessionStatus) -> u8 {
    match status {
        SessionStatus::Thinking => 0,
        SessionStatus::Processing => 0,
        SessionStatus::Compacting => 0,
        SessionStatus::Waiting => 1,
        SessionStatus::Idle => 2,
        SessionStatus::Finished => 3,
    }
}
