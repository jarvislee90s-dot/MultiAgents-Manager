// dsh 三色状态判定（设计 P2 定死映射）：
// 优先级 error > approval > running > done（foxbell 生产语义复刻，M0 §2）
// interrupted 判定 = "写入者是否存活"（lock 交叉判定，评审升级 #2），与任务时长无关

use crate::monitor::dsh::log::DshEvent;
use crate::session::SessionStatus;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LockState {
    /// 某个 dsh 进程持有该会话写租约（真运行）
    Held,
    /// 锁文件存在但无人持有（写入者已死 → 中断）
    Free,
    /// 无锁文件（v0 旧目录 / 探测失败）→ mtime 静默兜底
    Unknown,
}

/// v0 兜底：日志静默超此阈值 → 判中断（备忘 §A4）
pub const V0_SILENCE_INTERRUPT_MS: i64 = 30 * 60 * 1000;

pub struct StatusInput<'a> {
    pub events: &'a [DshEvent],
    pub lock: LockState,
    /// 日志 mtime 距今毫秒（仅 lock=Unknown 时消费）
    pub silence_ms: Option<i64>,
}

pub struct StatusOutcome {
    pub status: SessionStatus,
    /// 最近 turn/end 的 reason.kind（诊断/审计用）
    pub end_kind: Option<String>,
    /// 最近 turn/end 的 seq（水位/未读判定用）
    pub last_end_seq: Option<i64>,
}

pub fn derive(input: &StatusInput) -> StatusOutcome {
    let mut last_start: Option<i64> = None;
    let mut last_end: Option<(i64, String)> = None; // (seq, kind)
    let mut open_approvals: std::collections::HashSet<String> = std::collections::HashSet::new();

    for e in input.events {
        match e.kind.as_str() {
            "turn/start" => last_start = e.seq.or(last_start),
            "turn/end" => {
                if let Some(seq) = e.seq {
                    let kind = e
                        .data
                        .pointer("/reason/kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    // 只保留最新（seq 单调）
                    if last_end.as_ref().map(|(s, _)| seq >= *s).unwrap_or(true) {
                        last_end = Some((seq, kind));
                    }
                }
            }
            "approval/asked" => {
                if let Some(id) = e.data.get("id").and_then(|v| v.as_str()) {
                    open_approvals.insert(id.to_string());
                }
            }
            "approval/decided" => {
                if let Some(id) = e.data.get("id").and_then(|v| v.as_str()) {
                    open_approvals.remove(id);
                }
            }
            _ => {}
        }
    }

    let open_turn = match (last_start, &last_end) {
        (Some(s), Some((e, _))) => s > *e,
        (Some(_), None) => true,
        _ => false,
    };

    let (status, end_kind) = if open_turn {
        if !open_approvals.is_empty() {
            (SessionStatus::Waiting, None) // 红 · 等待批准
        } else {
            match input.lock {
                LockState::Held => (SessionStatus::Processing, None),
                LockState::Free => (SessionStatus::Waiting, Some("interrupted".into())), // 写入者已死
                LockState::Unknown => {
                    let silence = input.silence_ms.unwrap_or(0);
                    if silence > V0_SILENCE_INTERRUPT_MS {
                        (SessionStatus::Waiting, Some("interrupted".into()))
                    } else {
                        (SessionStatus::Processing, None)
                    }
                }
            }
        }
    } else {
        let kind = last_end.as_ref().map(|(_, k)| k.clone());
        let st = match kind.as_deref() {
            Some("error") | Some("interrupted") => SessionStatus::Waiting, // 红
            Some("blocked") => SessionStatus::Processing,                  // 黄 · 等待
            Some("max-tokens") | Some("completed") => SessionStatus::Finished, // 绿
            Some("aborted") => SessionStatus::Idle,                        // 用户取消不点红
            _ => SessionStatus::Idle,
        };
        (st, kind)
    };

    StatusOutcome {
        status,
        end_kind,
        last_end_seq: last_end.map(|(s, _)| s),
    }
}

// 测试：黄金夹具判定 + P2 映射表 + lock 交叉判定分支
#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::dsh::log::DshEvent;
    use crate::session::SessionStatus;
    use serde_json::json;

    fn fixture_events(name: &str) -> Vec<DshEvent> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/dsh")
            .join(name);
        crate::monitor::dsh::log::parse_events(&std::fs::read_to_string(&p).unwrap())
    }

    fn ev(kind: &str, seq: i64, data: serde_json::Value) -> DshEvent {
        DshEvent {
            kind: kind.into(),
            seq: Some(seq),
            time: None,
            data,
        }
    }

    fn input(events: &[DshEvent], lock: LockState) -> StatusInput<'_> {
        StatusInput {
            events,
            lock,
            silence_ms: None,
        }
    }

    #[test]
    fn golden_completed_is_finished() {
        let e = fixture_events("sample1-completed.sanitized.jsonl");
        let out = derive(&input(&e, LockState::Held));
        assert_eq!(out.status, SessionStatus::Finished);
        assert_eq!(out.end_kind.as_deref(), Some("completed"));
    }

    #[test]
    fn golden_tool_error_session_completes_finished() {
        let e = fixture_events("sample2-tool-error.sanitized.jsonl");
        // 探测报告 line 55：sample2 = 工具错误后 turn 正常 completed；工具错误不改 turn
        // 结束 kind，不亮红（P2：红·错误仅指 reason.kind=="error"）。本测试守护：夹具含
        // tool-result 错误文本不得误翻状态
        assert_eq!(
            derive(&input(&e, LockState::Held)).status,
            SessionStatus::Finished
        );
        assert_eq!(
            derive(&input(&e, LockState::Held)).end_kind.as_deref(),
            Some("completed")
        );
    }

    #[test]
    fn golden_approval_pending_is_waiting() {
        let e = fixture_events("sample5-approval-pending.sanitized.jsonl");
        // turn 打开 + 审批未决 → 红（优先于"运行中"）
        assert_eq!(
            derive(&input(&e, LockState::Held)).status,
            SessionStatus::Waiting
        );
    }

    #[test]
    fn golden_approval_decided_recovers_finished() {
        // T5-1 黄金夹具（sample3，此前零消费）：asked(seq19)→decided(seq20) 同 id，
        // 决议必须清除开放审批集；样本随后 turn/end(seq26) 以 completed 收尾——
        // 最终绿（Finished），不得因"曾有审批"滞留红（Waiting）误判
        let e = fixture_events("sample3-approval-decided.sanitized.jsonl");
        let out = derive(&input(&e, LockState::Held));
        assert_eq!(out.status, SessionStatus::Finished);
        assert_eq!(out.end_kind.as_deref(), Some("completed"));
    }

    #[test]
    fn golden_running_is_processing() {
        let e = fixture_events("sample4-running.sanitized.jsonl");
        assert_eq!(
            derive(&input(&e, LockState::Held)).status,
            SessionStatus::Processing
        );
    }

    #[test]
    fn open_turn_with_free_lock_is_interrupted_waiting() {
        // 评审升级 #2：turn 打开 + 写入者已死（锁无人持有）→ 红·中断
        let e = fixture_events("sample4-running.sanitized.jsonl");
        assert_eq!(
            derive(&input(&e, LockState::Free)).status,
            SessionStatus::Waiting
        );
    }

    #[test]
    fn open_turn_unknown_lock_uses_silence() {
        let e = fixture_events("sample4-running.sanitized.jsonl");
        // 静默 10 分钟 → 仍黄（长任务不误判）
        let short = StatusInput {
            events: &e,
            lock: LockState::Unknown,
            silence_ms: Some(10 * 60 * 1000),
        };
        assert_eq!(derive(&short).status, SessionStatus::Processing);
        // 静默 31 分钟 → 红·中断（v0 兜底阈值 30min）
        let long = StatusInput {
            events: &e,
            lock: LockState::Unknown,
            silence_ms: Some(31 * 60 * 1000),
        };
        assert_eq!(derive(&long).status, SessionStatus::Waiting);
    }

    #[test]
    fn end_kind_mapping_table() {
        let base = |kind: &str| {
            vec![
                ev("turn/start", 4, json!({})),
                ev("turn/end", 6, json!({ "reason": { "kind": kind } })),
            ]
        };
        // 设计 P2 定死映射：aborted→空闲（不点红）；blocked→黄；max-tokens→绿；interrupted→红
        assert_eq!(
            derive(&input(&base("aborted"), LockState::Held)).status,
            SessionStatus::Idle
        );
        assert_eq!(
            derive(&input(&base("blocked"), LockState::Held)).status,
            SessionStatus::Processing
        );
        assert_eq!(
            derive(&input(&base("max-tokens"), LockState::Held)).status,
            SessionStatus::Finished
        );
        assert_eq!(
            derive(&input(&base("interrupted"), LockState::Held)).status,
            SessionStatus::Waiting
        );
        // P2"运行失败→红"：reason.kind=="error" 是红·错误的唯一来源（控制者裁决补充行）
        assert_eq!(
            derive(&input(&base("error"), LockState::Held)).status,
            SessionStatus::Waiting
        );
    }

    #[test]
    fn approval_open_without_decided_is_waiting() {
        // approval/asked 无同 id 的 decided → 红（turn 未闭合场景）
        let e = vec![
            ev("turn/start", 4, json!({})),
            ev("approval/asked", 5, json!({ "id": "appr-1" })),
            ev("approval/decided", 6, json!({ "id": "appr-x" })), // 决的是别的审批
        ];
        assert_eq!(
            derive(&input(&e, LockState::Held)).status,
            SessionStatus::Waiting
        );
    }
}
