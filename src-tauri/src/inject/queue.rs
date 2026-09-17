//! 状态门控队列（W2，裁决 12 / 可输入态口径）：黄入队；会话回到可输入态
//! （红·等待 / 绿·完成空闲——agent 把光标交回输入框的任何时刻）逐条 flush；
//! 一次一条，等下一可输入态 = 单会话串行。红·中断（快照中会话消失）不 flush，挂起明示。
//!
//! ## 锁纪律（M4 死锁教训的两侧镜像，勿退化）
//! - `DB.lock()` 临界区内**只做 SQL**（取队首 / 落账两个短临界区）；
//! - 快照复核与注入**零 DB 锁**：`(st.session_source)()` 的生产实现（get_all_sessions）
//!   内部会锁同一把全局 DB（unread / agent_tool 等 DAO）——锁内调用即自锁死锁。
//!
//! 测试策略（零接触真实 ~/.mam）：`flush_one` 的 DB 依赖走 dao 全局包装（生产口径，
//! MAM_HOME 重定向经 pin.rs / hooks.rs 实证不可行），故单测驱动拆分内核
//! [`try_flush`]（快照复核 + 注入，零 DB）+ [`settle`]（落账 + 审计，conn 显式注入
//! 内存库）；两者的组合即 `flush_one` 全部行为，组合本身仅 10 行取件/落账薄壳。

use crate::database::connection::DB;
use crate::database::dao::inject_queue::{self, QueueRow};
use crate::database::dao::write_audit;
use crate::session::SessionStatus;

/// 运行中三态（黄灯）：flush 常规路径不投递（等 agent 交回输入框）
pub fn is_running(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Processing | SessionStatus::Thinking | SessionStatus::Compacting
    )
}

/// 可输入三态（红·等待 / 绿·完成空闲）：flush 逐条消费队首
pub fn is_input_ready(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Waiting | SessionStatus::Idle | SessionStatus::Finished
    )
}

/// wire 状态名 → SessionStatus（serde 单源反序列化，与 watcher::status_wire 互逆——
/// 不做「变体名小写 == wire 串」的隐式约定推导，先例警告见 session/model.rs tool_id 注释）
fn status_from_wire(s: &str) -> Option<SessionStatus> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// 跃迁事件的 to（wire 字符串形态）是否可输入态；未知串保守判否（不触发 flush）
fn is_input_ready_str(wire: &str) -> bool {
    status_from_wire(wire).is_some_and(|s| is_input_ready(&s))
}

/// 单次投递结论（[`try_flush`] 的产出，[`settle`] 按此落账）
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FlushOutcome {
    /// 注入成功：mark_sent + 审计（action=flush|jump, result=ok）
    Sent,
    /// 注入失败：mark_failed + 审计（action=fail, result=failed:e）
    Failed(String),
    /// 快照中无此会话（红·中断挂起，W2）：不消费不落账
    Suspended,
    /// 仍在运行且非插队：不消费不落账（等下个可输入态事件）
    Deferred,
}

/// 快照复核 + 注入（flush 的判定与投递半边，**零 DB 接触**；落账交 [`settle`]）。
/// 调用方保证运行于 spawn_blocking（flush 循环与 Task 6 端点 handler 同先例），
/// 本体不自行 spawn_blocking——session_source 是同步阻塞调用。
/// - 会话查找直调注入源（数据同源铁律，与 files.rs 既有 `find` 形态一致）；
///   找不到 → [`FlushOutcome::Suspended`]（jump 也不发——红·中断无从定位 pid）；
/// - 找到但 is_running 且 !jump → [`FlushOutcome::Deferred`]；jump=true 跳过 is_running
///   复核（裁决 12 插队语义：运行中 TUI 把消息放进自身输入缓冲，用户显式要求即刻送达）；
/// - content 已在入队时 compose 完毕（Task 6），flush 直发
pub(crate) fn try_flush(
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
) -> FlushOutcome {
    let Some(session) = (st.session_source)()
        .sessions
        .into_iter()
        .find(|s| s.id == item.session_id)
    else {
        return FlushOutcome::Suspended;
    };
    if is_running(&session.status) && !jump {
        return FlushOutcome::Deferred;
    }
    match st.injector.locate_and_inject(session.pid, &item.content) {
        Ok(()) => FlushOutcome::Sent,
        Err(e) => FlushOutcome::Failed(e),
    }
}

/// 落账（flush 的记账半边，conn 显式注入：生产 = `DB.lock()` 短临界区，测试 = 内存库；
/// 锁内只做 SQL）。返回是否实际发出（Sent = true；其余 false）。
/// 挂起 / 等待分支不消费队首、不写审计（行保持 pending，等下一跃迁）。
pub(crate) fn settle(
    conn: &rusqlite::Connection,
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
    outcome: FlushOutcome,
) -> bool {
    match outcome {
        FlushOutcome::Suspended | FlushOutcome::Deferred => false,
        FlushOutcome::Sent => {
            inject_queue::mark_sent_conn(conn, item.id, chrono::Utc::now().timestamp_millis());
            audit_write(conn, st, item, if jump { "jump" } else { "flush" }, "ok");
            true
        }
        FlushOutcome::Failed(e) => {
            inject_queue::mark_failed_conn(conn, item.id, &e);
            audit_write(conn, st, item, "fail", &format!("failed:{e}"));
            false
        }
    }
}

/// 审计写口（DB + events::audit 日志并行，M4 T2e 惯例）：channel = 注入器名，
/// summary 只存摘要（W5 防审计库膨胀）。conn 由调用方短临界区传入（锁内只 SQL）。
/// Task 6 的 session-send / queue 端点审计提升复用本函数（届时如需跨模块可见再放宽可见性）。
fn audit_write(
    conn: &rusqlite::Connection,
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    action: &str,
    result: &str,
) {
    let channel = st.injector.name();
    let summary = super::normalize::summarize(&item.content, super::normalize::AUDIT_SUMMARY_CHARS);
    write_audit::record_conn(
        conn,
        chrono::Utc::now().timestamp_millis(),
        &item.device_id,
        &item.device_name,
        &item.agent_type,
        &item.session_id,
        channel,
        action,
        &summary,
        result,
    );
    // 日志留痕并行（remote_audit target 可 grep 追溯）
    crate::remote::events::audit(
        action,
        &format!(
            "sid={} channel={channel} result={result} item={}",
            item.session_id, item.id
        ),
    );
}

/// 单次投递内核（循环与端点共用）：jump=true 越过「仍在运行」复核（裁决 12 插队语义），
/// 但仍要求快照中会话存在（红·中断挂起，W2）。成功/失败都写审计（DB + events::audit
/// 日志并行）。返回是否实际发出。
pub fn flush_one(st: &crate::remote::server::RemoteState, session_id: &str, jump: bool) -> bool {
    // 1) 取队首：DB.lock 短临界区，锁内只做 SQL（M4 死锁教训）
    let pending = {
        let conn = DB.lock().unwrap();
        inject_queue::next_pending_conn(&conn, session_id)
    };
    // 2) 无待发
    let Some(item) = pending else {
        return false;
    };
    // 3+4) 快照复核 + 注入：零 DB 锁（session_source 生产实现内部会锁同一把全局 DB，
    //      锁内调用即自锁死锁——见模块头锁纪律）
    let outcome = try_flush(st, &item, jump);
    // 5) 落账：再次短临界区（mark + 审计双通道，锁内只 SQL）
    let conn = DB.lock().unwrap();
    settle(&conn, st, &item, jump, outcome)
}

/// flush 循环：订阅跃迁事件 → to 为可输入态的会话 → 单次投递内核（常规路径，jump=false）。
/// spawn 用 tauri::async_runtime（与 watcher 同裁决：任意线程可用）；DB/注入走
/// spawn_blocking。错误只记日志不 panic（下一跃迁自会重试队首）。
/// **接线注**：本任务只交付函数体；serve() 内挂载随 Task 6 合入编译。
pub fn spawn_flush_loop(state: std::sync::Arc<crate::remote::server::RemoteState>) {
    let mut rx = state.watcher_tx.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if !is_input_ready_str(&ev.to) {
                continue;
            }
            let st = state.clone();
            let _ =
                tokio::task::spawn_blocking(move || flush_one(&st, &ev.session_id, false)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inject::engine::Injector;
    use crate::session::{AgentType, ProcessForm, Session};

    // ==== 纯核：状态分类（Step 1 契约测试） ====

    #[test]
    fn status_classes() {
        use crate::session::model::SessionStatus::*;
        for s in [Processing, Thinking, Compacting] {
            assert!(is_running(&s));
            assert!(!is_input_ready(&s));
        }
        for s in [Waiting, Idle, Finished] {
            assert!(!is_running(&s));
            assert!(is_input_ready(&s));
        }
    }

    /// wire 状态名解析（is_input_ready_str）：与快照 status 同一 serde 单源形态
    /// （watcher::status_wire 产出的就是这套小写串）；未知串保守判否
    #[test]
    fn wire_status_gates_input_ready() {
        for s in ["waiting", "idle", "finished"] {
            assert!(is_input_ready_str(s), "{s} 应判可输入");
        }
        for s in ["processing", "thinking", "compacting"] {
            assert!(!is_input_ready_str(s), "{s} 运行中不应触发 flush");
        }
        assert_eq!(status_from_wire("bogus"), None, "未知 wire 串解析为 None");
        assert!(!is_input_ready_str("bogus"), "未知串保守不触发 flush");
    }

    // ==== 拆分内核驱动（Fake session_source + FakeInjector + 内存 DB，零接触真实 ~/.mam） ====

    /// 注入器假体：记录 locate_and_inject 调用（pid, text）；fail=Some 时恒 Err
    struct FakeInjector {
        calls: std::sync::Mutex<Vec<(u32, String)>>,
        fail: Option<&'static str>,
    }

    impl FakeInjector {
        fn ok() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                fail: None,
            })
        }
        fn failing(reason: &'static str) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                fail: Some(reason),
            })
        }
        fn recorded(&self) -> Vec<(u32, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Injector for FakeInjector {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push((pid, text.to_string()));
            match self.fail {
                Some(e) => Err(e.to_string()),
                None => Ok(()),
            }
        }
        fn locate_and_send_key(&self, _pid: u32, _key: &str) -> Result<(), String> {
            Ok(())
        }
    }

    /// 会话夹具（字段形状对齐 server.rs 既有测试构造）
    fn sess(id: &str, status: SessionStatus, pid: u32) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::Claude,
            project_name: "proj".into(),
            project_path: "/tmp/proj".into(),
            title: None,
            git_branch: None,
            github_url: None,
            status,
            last_message: None,
            last_message_role: None,
            last_activity_at: "2026-09-18T00:00:00Z".into(),
            pid,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        }
    }

    /// 测试态：session_source 注入给定快照，injector 注入假体（其余缝全空载，
    /// 形状对齐 server.rs test_state 先例——零 DB 零真实目录）
    fn state_with(
        sessions: Vec<Session>,
        injector: std::sync::Arc<dyn Injector>,
    ) -> crate::remote::server::RemoteState {
        crate::remote::server::RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: sessions.clone(),
                total_count: sessions.len(),
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: std::sync::Arc::new(crate::remote::server::SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| None),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            injector,
        }
    }

    fn mem() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        conn
    }

    fn enq(conn: &rusqlite::Connection, sid: &str, content: &str) -> i64 {
        inject_queue::enqueue_conn(conn, sid, "claude", "dev-1", "测试设备", content, 1000)
    }

    /// 黄态（Processing）常规路径不消费：不投递、队首仍在、不写审计
    #[test]
    fn running_session_regular_path_holds_queue_head() {
        let c = mem();
        enq(&c, "s-run", "hello");
        let fake = FakeInjector::ok();
        let st = state_with(
            vec![sess("s-run", SessionStatus::Processing, 7)],
            fake.clone(),
        );
        let item = inject_queue::next_pending_conn(&c, "s-run").unwrap();

        assert_eq!(try_flush(&st, &item, false), FlushOutcome::Deferred);
        assert!(
            fake.recorded().is_empty(),
            "黄态常规路径不得投递（等 agent 交回输入框）"
        );
        assert!(
            !settle(&c, &st, &item, false, FlushOutcome::Deferred),
            "挂起返回 false（未发出）"
        );
        assert!(
            inject_queue::next_pending_conn(&c, "s-run").is_some(),
            "队首必须仍在（等下个事件）"
        );
        assert!(
            write_audit::recent_conn(&c, 10).is_empty(),
            "未消费不写审计（审计只记成功/失败）"
        );
    }

    /// 插队语义回归锁（裁决 12）：jump=true 越过 is_running 复核照发，
    /// FakeInjector 收到 content，行被 mark_sent，审计 action=jump channel=fake
    #[test]
    fn jump_delivers_even_when_running() {
        let c = mem();
        enq(&c, "s-run", "插队消息");
        let fake = FakeInjector::ok();
        let st = state_with(
            vec![sess("s-run", SessionStatus::Processing, 7)],
            fake.clone(),
        );
        let item = inject_queue::next_pending_conn(&c, "s-run").unwrap();

        assert_eq!(try_flush(&st, &item, true), FlushOutcome::Sent);
        assert_eq!(
            fake.recorded(),
            vec![(7, "插队消息".to_string())],
            "插队必须照发（运行中 TUI 把消息放进自身输入缓冲）"
        );
        assert!(settle(&c, &st, &item, true, FlushOutcome::Sent));
        let row = inject_queue::get_conn(&c, item.id).unwrap();
        assert!(row.sent_at.is_some(), "mark_sent 落库");
        assert_eq!(row.failed_reason, None);
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1, "成功恰一条审计");
        assert_eq!(audits[0].action, "jump", "插队审计 action=jump");
        assert_eq!(audits[0].channel, "fake", "channel 取注入器名");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].session_id, "s-run");
        assert!(inject_queue::next_pending_conn(&c, "s-run").is_none());
    }

    /// Waiting（红·等待）会话常规路径消费队首：投递 + mark_sent + 审计 action=flush
    #[test]
    fn waiting_session_flushes_queue_head() {
        let c = mem();
        enq(&c, "s-wait", "常规消息");
        let fake = FakeInjector::ok();
        let st = state_with(
            vec![sess("s-wait", SessionStatus::Waiting, 8)],
            fake.clone(),
        );
        let item = inject_queue::next_pending_conn(&c, "s-wait").unwrap();

        assert_eq!(try_flush(&st, &item, false), FlushOutcome::Sent);
        assert_eq!(fake.recorded(), vec![(8, "常规消息".to_string())]);
        assert!(settle(&c, &st, &item, false, FlushOutcome::Sent));
        let row = inject_queue::get_conn(&c, item.id).unwrap();
        assert!(row.sent_at.is_some());
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "flush", "常规路径审计 action=flush");
        assert_eq!(audits[0].result, "ok");
        assert!(inject_queue::next_pending_conn(&c, "s-wait").is_none());
    }

    /// 快照无该会话（红·中断）不消费：jump=true 也不发（pid 无从定位），行挂起
    #[test]
    fn missing_session_suspends_even_on_jump() {
        let c = mem();
        enq(&c, "s-gone", "hi");
        let fake = FakeInjector::ok();
        let st = state_with(vec![], fake.clone()); // 空快照 = 会话消失
        let item = inject_queue::next_pending_conn(&c, "s-gone").unwrap();

        assert_eq!(
            try_flush(&st, &item, true),
            FlushOutcome::Suspended,
            "jump 也要求快照中会话存在（红·中断挂起，W2）"
        );
        assert!(fake.recorded().is_empty(), "挂起不得投递");
        assert!(!settle(&c, &st, &item, true, FlushOutcome::Suspended));
        assert!(
            inject_queue::next_pending_conn(&c, "s-gone").is_some(),
            "挂起行保持 pending（不消费不失败）"
        );
        assert!(write_audit::recent_conn(&c, 10).is_empty());
    }

    /// 注入失败：mark_failed 落 failed_reason + 审计 action=fail result=failed:e；
    /// 失败行退出 pending（行保留作审计痕迹，与 dao 既有语义一致）
    #[test]
    fn inject_failure_marks_failed_and_audits() {
        let c = mem();
        enq(&c, "s-wait", "hi");
        let fake = FakeInjector::failing("定位终端失败：pid 不存在");
        let st = state_with(
            vec![sess("s-wait", SessionStatus::Waiting, 9)],
            fake.clone(),
        );
        let item = inject_queue::next_pending_conn(&c, "s-wait").unwrap();

        assert_eq!(
            try_flush(&st, &item, false),
            FlushOutcome::Failed("定位终端失败：pid 不存在".to_string())
        );
        assert!(
            !settle(
                &c,
                &st,
                &item,
                false,
                FlushOutcome::Failed("定位终端失败：pid 不存在".to_string())
            ),
            "注入失败返回 false（未发出）"
        );
        let row = inject_queue::get_conn(&c, item.id).unwrap();
        assert_eq!(
            row.failed_reason.as_deref(),
            Some("定位终端失败：pid 不存在"),
            "mark_failed 落 failed_reason"
        );
        assert_eq!(row.sent_at, None, "失败行不得带 sent_at");
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "fail");
        assert_eq!(
            audits[0].result, "failed:定位终端失败：pid 不存在",
            "失败审计携带原因（failed:e）"
        );
        assert!(
            inject_queue::next_pending_conn(&c, "s-wait").is_none(),
            "失败行退出 pending（下一跃迁不再重试同一行）"
        );
    }

    /// 审计摘要走 W5 截断口径（只存摘要防审计库膨胀）
    #[test]
    fn audit_summary_is_truncated() {
        let c = mem();
        let long = "长".repeat(super::super::normalize::AUDIT_SUMMARY_CHARS + 10);
        enq(&c, "s-wait", &long);
        let fake = FakeInjector::ok();
        let st = state_with(vec![sess("s-wait", SessionStatus::Waiting, 10)], fake);
        let item = inject_queue::next_pending_conn(&c, "s-wait").unwrap();
        let outcome = try_flush(&st, &item, false);
        assert!(settle(&c, &st, &item, false, outcome));
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1);
        let want =
            super::super::normalize::summarize(&long, super::super::normalize::AUDIT_SUMMARY_CHARS);
        assert_eq!(audits[0].summary, want, "summary 必须经 summarize 截断");
        assert!(
            audits[0].summary.chars().count() <= super::super::normalize::AUDIT_SUMMARY_CHARS + 1
        );
    }
}
