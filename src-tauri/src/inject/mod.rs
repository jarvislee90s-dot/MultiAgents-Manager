pub mod engine;
pub mod normalize;
pub mod queue;
pub mod routing;

/// 审计写口（DB + events::audit 日志并行，M4 T2e 惯例）：channel = 注入器名，
/// summary 只存摘要（W5 防审计库膨胀）。conn 由调用方短临界区传入（锁内只 SQL）。
/// 原 Task 5 的 queue.rs 私有版提升至此（Task 6）：flush/jump/fail 落账（queue::settle）
/// 与 session-send / queue / retract 端点审计两处共用，单一出口防词表漂移。
/// 字段化入参而非 QueueRow：端点侧审计（send/queue/retract）没有整行可传。
#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_write(
    conn: &rusqlite::Connection,
    st: &crate::remote::server::RemoteState,
    device_id: &str,
    device_name: &str,
    agent_type: &str,
    session_id: &str,
    content: &str,
    action: &str,
    result: &str,
) {
    let channel = st.injector.name();
    let summary = normalize::summarize(content, normalize::AUDIT_SUMMARY_CHARS);
    crate::database::dao::write_audit::record_conn(
        conn,
        chrono::Utc::now().timestamp_millis(),
        device_id,
        device_name,
        agent_type,
        session_id,
        channel,
        action,
        &summary,
        result,
    );
    // 日志留痕并行（remote_audit target 可 grep 追溯）
    crate::remote::events::audit(
        action,
        &format!("sid={session_id} channel={channel} result={result}"),
    );
}
