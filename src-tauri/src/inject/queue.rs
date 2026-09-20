//! 状态门控队列（W2，裁决 12 / 可输入态口径）：黄入队；会话回到可输入态
//! （红·等待 / 绿·完成空闲——agent 把光标交回输入框的任何时刻）逐条 flush；
//! 一次一条，等下一可输入态 = 单会话串行。红·中断（快照中会话消失）不 flush，挂起明示。
//!
//! ## A1 分层写入确认（M9R Task 5）
//! 注入成功 ≠ 已送达：[`try_flush`] 在注入 Ok 后按直发/插队分派确认
//! （实现全在 `super::confirm`，本模块只接线）——直发以会话文件戳命中定
//! 「已送达」（超时走屏读回查补按回车）；插队以占用排空确认。确认轮询/屏读
//! 全在 DB 锁外（`flush_one` 取件/落账两短临界区结构保证），调用经
//! `RemoteState.confirm_probe` / `injector` 缝——测试零接触真实文件。
//!
//! ## 锁纪律（M4 死锁教训的两侧镜像，勿退化）
//! - `DB.lock()` 临界区内**只做 SQL**（取队首 / 落账两个短临界区）；
//! - 快照复核与注入**零 DB 锁**：`(st.session_source)()` 的生产实现（get_all_sessions）
//!   内部会锁同一把全局 DB（unread / agent_tool 等 DAO）——锁内调用即自锁死锁。
//!
//! 测试策略（零接触真实 ~/.mam）：`flush_one` 的 DB 依赖经 `RemoteState.store`
//! （生产 = `DeviceStore::Global` 即全局 DB 同锁同连接；测试 = 内存库，端点测试
//! 不触真实目录），单测另可拆分内核 [`try_flush`]（快照复核 + 注入 + A1 确认，
//! 零 DB）+ [`settle`]（落账 + 审计，conn 显式注入内存库）直接驱动；两者的组合即
//! `flush_one` 全部行为，组合本身仅 10 行取件/落账薄壳。

use crate::database::dao::inject_queue::{self, QueueRow};
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

/// 单次投递结论（[`try_flush`] 的产出，[`settle`] 按此落账；P1-4 起同时是投递内核
/// 的对外回执——端点按态精确映射 delivered/submitted/queued/failed）。
/// 可见性说明：`pub fn flush_one` 的返回类型必须同级可见（private_interfaces 门禁），
/// 故自 Task 5 的 `pub(crate)` 收宽为 `pub`——裸枚举无泄露面（变体载荷只有 String）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushOutcome {
    /// 注入成功且 A1 确认通过（直发=会话文件戳命中；插队=占用排空/best-effort）：
    /// mark_sent + 审计（action=flush|jump, result=ok）
    Sent,
    /// 注入成功 + 戳未中 + 屏读**无滞留草稿**（D7/T3 确认判据收紧）：消息已被 TUI
    /// 收进内部队列 = **已投递未确认**（中性非失败）——mark_sent 消费（行退出
    /// pending：消息已在 TUI 手里，flush 循环重投即双发）+ 审计
    /// （action=flush, result=unconfirmed，与确认送达 ok / 确认失败 failed:e 三分，
    /// 不冒充成功也不冒充失败）。**不提供重试**（TUI 那份无法撤回，重试 = 双发）
    Submitted,
    /// 注入失败或 A1 确认失败（滞留 + 补回车失败 / 补回车后 3s 仍未见戳——真失败，
    /// D7/T3 后「非滞留」不再归本态）：mark_failed + 审计（action=fail,
    /// result=failed:e）——防重警示文案保留，重试由用户判断
    Failed(String),
    /// 快照中无此会话（红·中断挂起，W2）：不消费不落账
    Suspended,
    /// 仍在运行且非插队：不消费不落账（等下个可输入态事件）。[`flush_one`] 无
    /// pending 时亦归本态（编排裁决：无可投递亦非失败，与黄态同形——端点按 queued
    /// 回执，不谎报 delivered 也不误报失败）
    Deferred,
}

/// 快照复核 + 注入 + A1 写入确认（flush 的判定与投递半边，**零 DB 接触**；落账
/// 交 [`settle`]）。调用方保证运行于 spawn_blocking（flush 循环与 Task 6 端点
/// handler 同先例），本体不自行 spawn_blocking——session_source 是同步阻塞调用，
/// 确认轮询（[`confirm::await_direct_receipt`] / [`confirm::await_jump_receipt`]）
/// 亦为阻塞语义且全在本函数内完成（DB 锁外——`flush_one` 的取件/落账两短临界区
/// 结构保证，勿破坏）。
/// - 会话查找直调注入源（数据同源铁律，与 files.rs 既有 `find` 形态一致）；
///   找不到 → [`FlushOutcome::Suspended`]（jump 也不发——红·中断无从定位 pid）；
/// - 找到但 is_running 且 !jump → [`FlushOutcome::Deferred`]；jump=true 跳过 is_running
///   复核（裁决 12 插队语义：运行中 TUI 把消息放进自身输入缓冲，用户显式要求即刻送达）；
/// - 注入按族规格走 `locate_and_inject_spec`（spec = `families::family_for`，无族
///   回退 [`families::FALLBACK_SPEC`]——Task 3 trait 扩展正是为这里）；
/// - **A1 分层写入确认（M9R Task 5，注入成功 ≠ 已送达）+ D7/T3 三态分诊**：注入 Ok
///   后按 jump 分派——直发以会话文件戳命中定「已送达」（超时走屏读回查：滞留判定
///   → 补按回车 → 复查，并按屏读结果分诊——非滞留 = Submitted 已投递未确认中性；
///   滞留补回车失败 / 复查未中 = Failed）；插队以占用排空确认（屏读 best-effort）。
///   真失败才 [`FlushOutcome::Failed`]；
/// - content 已在入队时 compose 完毕（Task 6），flush 直发
pub(crate) fn try_flush(
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
) -> FlushOutcome {
    try_flush_with(st, item, jump, None)
}

/// 变体：直发确认轮询窗可覆盖（质量评审 Important 1——测试小超时入口，保持套件
/// 无 5s 级慢测；`None` = 按族规格，生产路径）。仅影响直发确认的轮询窗，
/// 注入行为与插队路径不受影响。
pub(crate) fn try_flush_with(
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
    confirm_timeout_override: Option<u64>,
) -> FlushOutcome {
    // 会话 id 只在工具内唯一（watcher/dedup 同口径）：必须按 (tool, id) 复合匹配，
    // 跨工具撞 id 时裸 id 匹配会向错误会话的 pid 注入
    let Some(session) = (st.session_source)()
        .sessions
        .into_iter()
        .find(|s| s.id == item.session_id && s.agent_type.tool_id() == item.agent_type)
    else {
        return FlushOutcome::Suspended;
    };
    if is_running(&session.status) && !jump {
        return FlushOutcome::Deferred;
    }
    let spec = crate::inject::families::family_for(&item.agent_type)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    let confirm_timeout = confirm_timeout_override.unwrap_or(spec.confirm_timeout_ms);
    let receipt = match st
        .injector
        .locate_and_inject_spec(session.pid, &item.content, &spec)
    {
        // 注入失败短路确认（时序锁语义）：确认只在注入成功后起跑
        Err(e) => super::confirm::DirectReceipt::Failed(e),
        Ok(()) => {
            if jump {
                match super::confirm::await_jump_receipt(st, &session, &item.content) {
                    Ok(()) => super::confirm::DirectReceipt::Confirmed,
                    Err(e) => super::confirm::DirectReceipt::Failed(e),
                }
            } else {
                // timeout 同源下发（spec）；确认结论为 D7/T3 三态分诊（非滞留 =
                // Submitted 中性；滞留补回车后命中 = Confirmed；其余 = Failed），
                // 失败文案按「工具 × 平台」感知（F2：families::macos_enter_swallowed
                // 投影表，mac-reverify §四-B）
                super::confirm::await_direct_receipt(st, &session, &item.content, confirm_timeout)
            }
        }
    };
    match receipt {
        super::confirm::DirectReceipt::Confirmed => FlushOutcome::Sent,
        super::confirm::DirectReceipt::Submitted => FlushOutcome::Submitted,
        super::confirm::DirectReceipt::Failed(e) => FlushOutcome::Failed(e),
    }
}

/// 落账（flush 的记账半边，conn 显式注入：生产 = `DB.lock()` 短临界区，测试 = 内存库；
/// 锁内只做 SQL）。返回投递结论（Task 6 演进：bool → Result——端点直发/插队需要
/// 失败原因作回执）：
/// - `Ok(())` = 已发出（Sent / D7/T3 Submitted 已投递未确认）；或挂起/等待（行保持
///   pending 等下个跃迁，**非失败**）；
/// - `Err(e)` = 注入失败原因（行已 mark_failed 退出 pending）。
///
/// 挂起 / 等待分支不消费队首、不写审计（行保持 pending，等下一跃迁）。
pub(crate) fn settle(
    conn: &rusqlite::Connection,
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
    outcome: FlushOutcome,
) -> Result<(), String> {
    match outcome {
        FlushOutcome::Suspended | FlushOutcome::Deferred => Ok(()),
        FlushOutcome::Sent => {
            inject_queue::mark_sent_conn(conn, item.id, chrono::Utc::now().timestamp_millis());
            super::audit_write(
                conn,
                st,
                &item.device_id,
                &item.device_name,
                &item.agent_type,
                &item.session_id,
                &item.content,
                if jump { "jump" } else { "flush" },
                "ok",
            );
            Ok(())
        }
        FlushOutcome::Submitted => {
            // D7/T3：注入 Ok + 戳未中 + 屏读无滞留草稿 = 消息已被 TUI 收进内部
            // 队列（已投递未确认）。行必须消费退出 pending——消息已在 TUI 手里，
            // flush 循环再投即双发（TUI 那份无法撤回），故 mark_sent；但不得冒充
            // 确认成功（Sent 的 ok）也不得误报失败（failed:e 会诱导重试 = 双发），
            // 审计 result=unconfirmed 单列，与「确认送达 ok」「确认失败 failed:e」
            // 三分。Submitted 仅直发分诊产出（jump 以占用排空定论），action 沿
            // 路径标注仅为防呆对称
            inject_queue::mark_sent_conn(conn, item.id, chrono::Utc::now().timestamp_millis());
            super::audit_write(
                conn,
                st,
                &item.device_id,
                &item.device_name,
                &item.agent_type,
                &item.session_id,
                &item.content,
                if jump { "jump" } else { "flush" },
                "unconfirmed",
            );
            Ok(())
        }
        FlushOutcome::Failed(e) => {
            inject_queue::mark_failed_conn(conn, item.id, &e);
            super::audit_write(
                conn,
                st,
                &item.device_id,
                &item.device_name,
                &item.agent_type,
                &item.session_id,
                &item.content,
                "fail",
                &format!("failed:{e}"),
            );
            Err(e)
        }
    }
}

/// 单次投递内核（循环与端点共用）：jump=true 越过「仍在运行」复核（裁决 12 插队语义），
/// 但仍要求快照中会话存在（红·中断挂起，W2）。成功/失败都写审计（DB + events::audit
/// 日志并行）。
/// 返回值（Task 6 P1-4 终态：Result → [`FlushOutcome`] 五态上抛，端点按态精确映射
/// delivered/submitted/queued/failed）：
/// - `Sent` = 已发出（行已 mark_sent）；
/// - `Submitted` = 已投递未确认（D7/T3：消息已被 TUI 收进内部队列，行已 mark_sent
///   消费退出 pending——防 flush 循环重投 = 双发；中性非失败，端点按 submitted 回执）；
/// - `Failed(e)` = 注入失败原因（行已 mark_failed 退出 pending——失败不留残留，重试安全，W1）；
/// - `Suspended` = 快照中无此会话（行保持 pending 等会话回来）；
/// - `Deferred` = 仍在运行非插队 / 无 pending（行保持 pending 等下个跃迁，**非失败**——
///   端点按 queued 回执，挂起项由 flush 循环/对账接力）。
///
/// DB 依赖经 `st.store`：生产 `DeviceStore::Global`（即全局 DB 同锁同连接），
/// 测试注入内存库（端点测试零接触真实 ~/.mam）。
pub fn flush_one(
    st: &crate::remote::server::RemoteState,
    session_id: &str,
    jump: bool,
) -> FlushOutcome {
    flush_one_with(st, session_id, jump, None)
}

/// 变体：直发确认轮询窗可覆盖（透传 [`try_flush_with`]，测试小超时入口）。
pub(crate) fn flush_one_with(
    st: &crate::remote::server::RemoteState,
    session_id: &str,
    jump: bool,
    confirm_timeout_override: Option<u64>,
) -> FlushOutcome {
    // 1) 取队首：短临界区，临界区内只做 SQL（M4 死锁教训；DeviceStore::with 即锁语义）
    let pending = st
        .store
        .with(|conn| inject_queue::next_pending_conn(conn, session_id));
    // 2) 无待发：无可投递亦非失败（编排裁决：与黄态同形 Deferred，端点按 queued 回执）
    let Some(item) = pending else {
        return FlushOutcome::Deferred;
    };
    // 3+4) 快照复核 + 注入 + 确认：零 DB 锁（session_source 生产实现内部会锁同一把
    //      全局 DB，锁内调用即自锁死锁——见模块头锁纪律）
    let outcome = try_flush_with(st, &item, jump, confirm_timeout_override);
    // 5) 落账：再次短临界区（mark + 审计双通道，锁内只 SQL）。settle 的 Err 与
    //    Failed(e) 同源同因（只在 Failed 分支产生），随 outcome 原样上抛不丢
    let _ = st
        .store
        .with(|conn| settle(conn, st, &item, jump, outcome.clone()));
    outcome
}

/// 指定条目投递（session-queue/jump 端点用）：插队语义允许点名 pending 中的任意条目
/// （裁决 12：用户显式要求即刻送达，不限于队首），故不走 flush_one 的「取队首」——
/// 调用方（端点）已按 (session_id, item_id) 前查该行 pending 归属，且在本函数执行的
/// 全程持有 in-flight 守卫（防与 flush 循环对同一会话双投；F1 后守卫取在端点的
/// spawn_blocking 闭包内）。**守卫下必须复查该行仍 pending**（前查与守卫之间的间隙
/// 他方可能已消费该条目，不复查即对已 sent 行再注入）——该复查义务已抽为
/// [`flush_given_if_pending`]（收尾批 P2 可测内核），端点经它调用本函数。快照复核 +
/// 注入 + 落账与 [`flush_one`] 同一内核（审计 action=jump 由 [`settle`] 写入），
/// 返回语义同 [`flush_one`]。
pub(crate) fn flush_given(
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
) -> FlushOutcome {
    let outcome = try_flush(st, item, jump);
    // settle 的 Err 与 Failed(e) 同源同因，随 outcome 上抛（见 flush_one_with 同注）
    let _ = st
        .store
        .with(|conn| settle(conn, st, item, jump, outcome.clone()));
    outcome
}

/// jump 守卫下的 pending 再校验 + 投递（收尾批 P2 抽函数：session-queue/jump 端点
/// spawn_blocking 闭包内的「守卫下再校验 pending 归属」可测内核）。调用方（端点）
/// 必须**先取到该会话 in-flight 守卫再调本函数**——守卫是闭包第一条语句的义务留在
/// 端点，与 [`flush_one`]/flush 循环事件臂同形。行为：
/// - 复查条目仍 pending（端点前查与守卫之间有间隙，flush 循环可能已投递本条并释放
///   守卫——不复查会对已 sent 条目再注入，双投）：`store.with` 短临界区，锁内只 SQL；
/// - 仍 pending → [`flush_given`]（注入与确认零 DB 锁，锁纪律不破坏）；
/// - 已被消费 → [`FlushOutcome::Deferred`]（端点按 queued/position=0 回执，语义
///   「已不在队列，由投递循环接力」，前端须容忍 0）。
pub(crate) fn flush_given_if_pending(
    st: &crate::remote::server::RemoteState,
    item: &QueueRow,
    jump: bool,
) -> FlushOutcome {
    let still_pending = st.store.with(|c| {
        inject_queue::pending_for_session_conn(c, &item.session_id)
            .iter()
            .any(|i| i.id == item.id)
    });
    if !still_pending {
        return FlushOutcome::Deferred;
    }
    flush_given(st, item, jump)
}

/// 启动对账补投内核（P2-5，[`spawn_flush_loop`] 启动时与周期兜底共用）：取 distinct
/// pending 会话列表（短临界区只 SQL，口径同源 [`inject_queue::pending_session_ids_conn`]）
/// → 逐会话取 in-flight 守卫（与 flush 循环事件臂/端点直发/插队互斥，防双投）→
/// [`flush_one`] 常规路径补投。调用方保证运行于 spawn_blocking（flush_one 内的
/// session_source 是同步阻塞调用）。结果处置与 flush 循环事件臂同口径：
/// Sent 静默 / Failed(e) log::warn / 其余（Submitted 已投递未确认已落账、
/// Deferred/Suspended 不消费不落账）静默。
///
/// 守卫生命周期核对（Critical 1 同审）：守卫在本函数体内、与 [`flush_one`] 同栈同
/// 生命周期——本函数整体跑在调用方的 spawn_blocking 阻塞段里（abort 只取消调用方
/// future 不打阻塞段），不存在循环事件臂「future 先 drop 释放守卫、阻塞段仍在跑」
/// 的窗口，无需再入更内层闭包。
pub(crate) fn reconcile_once(state: &std::sync::Arc<crate::remote::server::RemoteState>) {
    let sessions = state
        .store
        .with(crate::database::dao::inject_queue::pending_session_ids_conn);
    for sid in sessions {
        // 该会话已有投递进行中 → 跳过（进行中的那次已覆盖；残留由下轮兜底再收）
        let Some(_guard) = try_acquire_inflight(&sid) else {
            continue;
        };
        match flush_one(state, &sid, false) {
            FlushOutcome::Sent => {}
            FlushOutcome::Failed(e) => log::warn!("对账补投失败（会话 {sid}）: {e}"),
            // Submitted（D7/T3）：已投递未确认，settle 已按 unconfirmed 落账——
            // 非失败静默；Deferred/Suspended：不消费不落账——静默
            FlushOutcome::Submitted | FlushOutcome::Deferred | FlushOutcome::Suspended => {}
        }
    }
}

/// 周期兜底门控（P2-5 + 灰2）：先 `store.with` 纯 SQL 计数（零文件零解析零注入），
/// 0 直接跳过——守宪法会话扫描预算契约（周期扫描不得无门控展开）；有 pending
/// 会话才进入 [`reconcile_once`] 逐会话补投（覆盖跃迁事件丢失/事件臂 Lagged 的残留）。
pub(crate) fn sweep_if_pending(state: &std::sync::Arc<crate::remote::server::RemoteState>) {
    let pending_sessions = state
        .store
        .with(crate::database::dao::inject_queue::count_pending_sessions_conn);
    if pending_sessions == 0 {
        return;
    }
    reconcile_once(state);
}

/// per-session in-flight 守卫（Task 6 评审追记 3）：同一会话并发 flush_one 会双投
/// （Task 5 评审 TOCTOU——快照复核与注入不持 DB 锁，两个并发调用可同时通过复核，
/// 对同一队列头各注入一次）。进程级单例，flush 循环与端点直发/插队共用；
/// 不引新依赖，用 `Mutex<HashSet>`。try 语义：已 in-flight → `None`（调用方跳过本次，
/// 进行中的那次投递已覆盖该会话）。守卫 Drop 自释放：flush_one 中途 panic 也不永久占位。
/// 锁自愈取锁（P3 统一）：临界区 panic 毒化不扩散，后续取锁者照常工作
static INFLIGHT: once_cell::sync::Lazy<std::sync::Mutex<std::collections::HashSet<String>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

/// in-flight 占位句柄（RAII）：Drop 时释放会话占位
pub(crate) struct InflightGuard(String);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        INFLIGHT
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// 尝试占用会话的 in-flight 名额：空闲 → `Some(守卫)`；已有投递进行中 → `None`
pub(crate) fn try_acquire_inflight(session_id: &str) -> Option<InflightGuard> {
    let mut set = INFLIGHT.lock().unwrap_or_else(|e| e.into_inner());
    if set.contains(session_id) {
        return None;
    }
    set.insert(session_id.to_string());
    Some(InflightGuard(session_id.to_string()))
}

/// 幂等护栏（对齐 watcher::LOOP_HANDLE 模式）：serve() 可重入（开关切换/热重启），
/// 循环存活期间重复调用不重复 spawn（否则每次重启净增一个循环任务，且远程关闭后
/// 残留循环仍在投递）；旧句柄已结束（异常退出）取走重建，自愈。
static FLUSH_LOOP_HANDLE: once_cell::sync::Lazy<
    std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// 循环句柄存活判定（纯函数，对齐 remote/watcher 同名先例）
fn flush_loop_handle_is_live(h: &Option<tauri::async_runtime::JoinHandle<()>>) -> bool {
    h.as_ref()
        .map(|jh| !jh.inner().is_finished())
        .unwrap_or(false)
}

/// 测试专用串行锁（仅 cfg(test)；评审 Important 4 裁决：独占资源方案，替代重试启发式）：
/// `stop_freezes_flush_loop` 与 remote::mod 两个真实调 [`abort_flush_loop`] 的
/// stop_server_core 内核测试**全程持锁**——FLUSH_LOOP_HANDLE 是进程级单例槽，三测并行
/// 时互相清槽/验槽会假红，持本锁强制串行（确定性）。锁序：TEST_LOCK 先取，锁内才可能
/// 触 FLUSH_LOOP_HANDLE（无反向获取者，无锁序环）。
#[cfg(test)]
pub(crate) static LOOP_HANDLE_TEST_LOCK: once_cell::sync::Lazy<std::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

/// 冻结队列（裁决 19：停止远程 = 投递循环随服务同停）：取 FLUSH_LOOP_HANDLE 锁 →
/// take → Some(h) 则 `h.abort()`。槽清空即幂等——重复调用 take 得 None，天然无害；
/// 自取自己的锁，与 SERVER_HANDLE 不嵌套（两把锁不嵌套纪律保持）。
///
/// 停服真实语义（评审 Important 2 归因纠正）：①abort 只取消循环 future，**不再发起新
/// 投递**（冻结）——在途投递是 spawn_blocking 阻塞段，不受 abort 影响，detached 跑完
/// 并正常 settle 落账（账面自洽，不存在停服硬停造成的「已注入未落账」）；②守卫在投递
/// 闭包内（Critical 1）随投递全程占位，停服→热重启后的新循环/对账经 INFLIGHT 互斥让位，
/// 无双投；③启动对账（[`reconcile_once`]，P2-5）真正兜底的窗口是「进程崩溃/强杀发生在
/// 注入成功后、落账前」（重启后 at-least-once 重投）与 stop→start 间隙丢失的跃迁事件。
pub(crate) fn abort_flush_loop() {
    let mut slot = FLUSH_LOOP_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = slot.take() {
        h.abort();
    }
}

/// flush 循环（`tokio::select!` 双臂，M9R Task 6 队列生命周期）：
/// - **事件臂**：订阅跃迁事件 → to 为可输入态的会话 → 单次投递内核（常规路径，jump=false）；
/// - **周期臂**（灰2）：60s 一跳 `sweep_if_pending`（纯 SQL 计数门控，0 直接跳过——
///   守宪法扫描预算），兜底跃迁事件丢失 / 事件臂 Lagged 丢最旧后的残留 pending；
/// - **启动对账**（P2-5）：订阅建立后、进入 loop 前立即 `reconcile_once` 一次——
///   主场景是进程重启后的遗留 pending 补投（兜底窗口 = 进程崩溃/强杀发生在「注入成功
///   后、落账前」→ 重启后 at-least-once 重投；裁决 19 停服冻结期堆积与 stop→start
///   间隙丢失的跃迁事件同由本扫 + 周期臂兜底）；subscribe 在 spawn 前已完成，期间
///   发布的跃迁事件由通道缓冲，无丢失。
///
/// spawn 用 tauri::async_runtime（与 watcher 同裁决：任意线程可用）；DB/注入走
/// spawn_blocking。错误只记日志不 panic（下一跃迁/下轮兜底自会重试队首）。
///
/// Task 6 评审追记落地（事件臂，全数保留）：
/// 1. **Lagged 自愈**：Lagged（消费落后超通道容量，丢最旧事件）只 warn 并继续，
///    Closed（无发布者，进程收尾）才退出；
/// 2. **burst 抑制**：recv 后 `try_recv` 排空积压，按 session_id 去重，每会话每批至多
///    一次 flush_one（避免逐事件排队放大投递；快照在 flush_one 内现取，去重后单次即最新态）；
/// 3. **并发双投防护**：投递前取 in-flight 守卫（守卫在阻塞闭包内、与投递同生命周期
///    ——Critical 1 修订，见闭包内注释），同会话并发触发只投一次；
/// 4. JoinError 至少 log::warn（原 `let _ =` 把任务 panic/取消吞得不可见）。
/// 5. **幂等**：FLUSH_LOOP_HANDLE 护栏（见上），serve() 重入不重复 spawn；停服经
///    [`abort_flush_loop`] 清槽（裁决 19 冻结队列，重开续跑）。
pub fn spawn_flush_loop(state: std::sync::Arc<crate::remote::server::RemoteState>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut handle_slot = FLUSH_LOOP_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    if flush_loop_handle_is_live(&handle_slot) {
        return; // 存活期间幂等：不重复 spawn
    }
    let mut rx = state.watcher_tx.subscribe();
    let spawned = tauri::async_runtime::spawn(async move {
        // P2-5 启动对账：进 loop 前补投一次遗留 pending（此时通道已在缓冲订阅后事件，
        // 与事件臂共守 in-flight 守卫，互斥不双投）
        {
            let st = state.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || reconcile_once(&st)).await {
                log::warn!("flush 循环启动对账任务异常: {e}");
            }
        }
        // 灰2 周期兜底：interval 首跳即完成（tokio 语义）＝启动后即刻多一次与启动对账
        // 同口径的空转计数门（纯 SQL，零成本幂等）；此后每 60s 一跳
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! {
                ev = rx.recv() => {
                    match ev {
                        Err(RecvError::Closed) => break,
                        Err(RecvError::Lagged(n)) => {
                            // 追记 1：Lagged 只丢最旧事件（宁缺不堵生产端），warn 后继续消费
                            log::warn!("flush 循环落后，丢弃 {n} 条跃迁事件（继续消费）");
                            continue;
                        }
                        Ok(ev) => {
                            if !is_input_ready_str(&ev.to) {
                                continue;
                            }
                            // 追记 2：把通道里已就绪的积压一次排空，按会话去重后逐会话一次投递
                            let mut batch = vec![ev.session_id];
                            while let Ok(more) = rx.try_recv() {
                                if is_input_ready_str(&more.to) && !batch.contains(&more.session_id) {
                                    batch.push(more.session_id);
                                }
                            }
                            for sid in batch {
                                let st = state.clone();
                                let sid_blocking = sid.clone();
                                match tokio::task::spawn_blocking(move || {
                                    // 追记 3（Critical 1 修订：守卫移入阻塞闭包）——守卫必须与
                                    // 投递同生命周期：abort 循环只 drop future（在 .await 点），
                                    // 守卫若持在 future 里会先于 detached 阻塞段释放，热重启后的
                                    // 新循环/对账即可取到名额，对同一仍 pending 的队首再注入 = 双投。
                                    // 守卫在闭包内时，detached 旧投递全程占位，新循环取不到名额
                                    // 即让位（Deferred 静默）——停服→热重启无双投
                                    let Some(_guard) = try_acquire_inflight(&sid_blocking) else {
                                        // 该会话已有投递进行中（含停服后 detached 的旧投递）→
                                        // 让位：进行中的那次已覆盖本会话，队首若仍有残留，下一
                                        // 跃迁事件/周期兜底自会再触发
                                        return FlushOutcome::Deferred;
                                    };
                                    flush_one(&st, &sid_blocking, false)
                                })
                                .await
                                {
                                    // P1-4 五态上抛：Sent 高频成功路径只记 debug；
                                    // Submitted（D7/T3 已投递未确认）非失败，debug 留痕；
                                    // Failed 保留既有 warn；Deferred/Suspended 静默（行保持
                                    // pending 等下个跃迁/兜底，非失败——既有 Ok(()) 静默语义保持）
                                    Ok(FlushOutcome::Sent) => {
                                        log::debug!("flush 已投递（会话 {sid}）")
                                    }
                                    Ok(FlushOutcome::Submitted) => {
                                        log::debug!(
                                            "flush 已投递未确认（会话 {sid}，消息在 TUI 内部队列，\
                                             agent 空闲后处理）"
                                        )
                                    }
                                    Ok(FlushOutcome::Failed(e)) => {
                                        log::warn!("flush 投递失败（会话 {sid}）: {e}")
                                    }
                                    Ok(FlushOutcome::Deferred | FlushOutcome::Suspended) => {}
                                    // 追记 4：JoinError（任务 panic/取消）不再静默吞掉
                                    Err(e) => log::warn!("flush 任务异常（会话 {sid}）: {e}"),
                                }
                            }
                        }
                    }
                }
                // 灰2：周期兜底臂——计数门在 sweep_if_pending 内（纯 SQL，0 跳过）。
                // 披露（评审 Minor）：本串行结构自身是 Lag 生产源——长投递（含对账）期间
                // rx 不被轮询，通道容量 64 可溢出丢最旧，sweep 即为其兜底；interval 默认
                // Burst 行为，停机/阻塞期间的错失 tick 补拍亦经计数门，无害
                _ = ticker.tick() => {
                    let st = state.clone();
                    if let Err(e) =
                        tokio::task::spawn_blocking(move || sweep_if_pending(&st)).await
                    {
                        log::warn!("flush 周期兜底任务异常: {e}");
                    }
                }
            }
        }
    });
    // 句柄落槽（存活期间后续调用幂等返回）
    *handle_slot = Some(spawned);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::dao::write_audit;
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

    /// kimi 会话夹具（F2 工具感知测试用——kimi = macOS 回车吞没投影表成员）
    fn sess_kimi(id: &str, status: SessionStatus, pid: u32) -> Session {
        let mut s = sess(id, status, pid);
        s.agent_type = AgentType::Kimi;
        s
    }

    /// 测试态：session_source 注入给定快照，injector 注入假体（其余缝全空载，
    /// 形状对齐 server.rs test_state 先例——零 DB 零真实目录）；confirm_probe
    /// 恒命中（首轮即中，零延迟零等待）
    fn state_with(
        sessions: Vec<Session>,
        injector: std::sync::Arc<dyn Injector>,
    ) -> crate::remote::server::RemoteState {
        state_with_probe(sessions, injector, std::sync::Arc::new(|_, _, _| true))
    }

    /// 变体：confirm_probe 可注入（A1 确认失败用例就地覆盖恒 false）
    fn state_with_probe(
        sessions: Vec<Session>,
        injector: std::sync::Arc<dyn Injector>,
        confirm_probe: std::sync::Arc<crate::remote::server::ConfirmProbeFn>,
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
            home_source: Box::new(|| None),
            injector,
            confirm_probe,
            // R5 一键 resume spawn 缝（Task 11）：flush 路径不消费，注 no-op 桩（零真开窗）
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
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
            settle(&c, &st, &item, false, FlushOutcome::Deferred).is_ok(),
            "挂起不视为失败（行保持 pending 等下个跃迁）"
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
        assert!(settle(&c, &st, &item, true, FlushOutcome::Sent).is_ok());
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
        assert!(settle(&c, &st, &item, false, FlushOutcome::Sent).is_ok());
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
        assert!(
            settle(&c, &st, &item, true, FlushOutcome::Suspended).is_ok(),
            "挂起不视为失败（行保持 pending）"
        );
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
            settle(
                &c,
                &st,
                &item,
                false,
                FlushOutcome::Failed("定位终端失败：pid 不存在".to_string())
            )
            .is_err(),
            "注入失败返回 Err(原因)（端点失败回执数据源）"
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
        assert!(settle(&c, &st, &item, false, outcome).is_ok());
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1);
        let want =
            super::super::normalize::summarize(&long, super::super::normalize::AUDIT_SUMMARY_CHARS);
        assert_eq!(audits[0].summary, want, "summary 必须经 summarize 截断");
        assert!(
            audits[0].summary.chars().count() <= super::super::normalize::AUDIT_SUMMARY_CHARS + 1
        );
    }

    // ==== A1 写入确认（M9R Task 5）：直呼确认函数 + 小超时（避免 5s 慢测） ====

    /// 直发确认分诊行为锁（D7/T3，原 `direct_confirm_failure_returns_err` 按新
    /// 语义更新）：confirm_probe 恒 false + 假 pid（屏读必败 → 无滞留草稿证据）→
    /// 小超时轮询未中 + 屏读回查——**Windows**：分诊走 NotStuck → Submitted 中性
    /// 「已投递未确认」（旧断言「恒 Err」正是验收问题 #5 假失败的根因：屏读无
    /// 滞留 = 字已被 TUI 收进内部队列，非滞留≠失败）；**非 Windows**：分诊不可达
    /// → Failed 原文案路径保持（macOS 行为不变的任务书约束）
    #[test]
    fn direct_confirm_without_stamp_triages_by_screen_read() {
        let st = state_with_probe(
            vec![sess("s-cf", SessionStatus::Waiting, 21)],
            FakeInjector::ok(),
            std::sync::Arc::new(|_, _, _| false),
        );
        let s = sess("s-cf", SessionStatus::Waiting, 21);
        let outcome = super::super::confirm::await_direct_receipt(&st, &s, "直发确认消息", 60);
        #[cfg(windows)]
        assert!(
            matches!(outcome, super::super::confirm::DirectReceipt::Submitted),
            "Windows 假 pid 屏读必败 → 无滞留草稿 → 中性 Submitted：{outcome:?}"
        );
        #[cfg(not(windows))]
        match outcome {
            super::super::confirm::DirectReceipt::Failed(e) => assert!(
                e.contains("已注入未确认"),
                "非 Windows 分诊不可达，保持失败回执原文案：{e}"
            ),
            other => panic!("非 Windows 应保持 Failed，实际 {other:?}"),
        }
    }

    /// 工具感知行为断言（M3B 接线锁 + F2 更正，mac-reverify §四-B；T3 按 D7 分诊
    /// 语义更新）：kimi 会话直发确认（probe 恒 false）——**macOS** 上分诊不可达 →
    /// Failed，回执与纯函数选择器在本机 OS 下的产出逐字一致（证明 tool 参数真实
    /// 参与选文案）；**Windows** 上假 pid 无滞留证据 → Submitted 中性（分诊态与
    /// 工具无关；kimi 文案只在 Failed 态显现，工具 × 平台感知由 confirm.rs
    /// 表驱动测试 `direct_confirm_fail_copy_is_tool_platform_aware` 与分诊纯核
    /// `direct_triage_screen_recovery_table` 的 kimi × macos 格覆盖）
    #[test]
    fn direct_confirm_failure_receipt_is_tool_consistent() {
        let st = state_with_probe(
            vec![sess_kimi("s-cf2", SessionStatus::Waiting, 27)],
            FakeInjector::ok(),
            std::sync::Arc::new(|_, _, _| false),
        );
        let s = sess_kimi("s-cf2", SessionStatus::Waiting, 27);
        let outcome = super::super::confirm::await_direct_receipt(&st, &s, "工具感知确认消息", 60);
        #[cfg(windows)]
        assert!(
            matches!(outcome, super::super::confirm::DirectReceipt::Submitted),
            "Windows 假 pid 无滞留证据 → 中性 Submitted：{outcome:?}"
        );
        #[cfg(not(windows))]
        match outcome {
            super::super::confirm::DirectReceipt::Failed(err) => assert_eq!(
                err,
                super::super::confirm::direct_confirm_fail_copy("kimi", std::env::consts::OS,),
                "回执必须与纯函数选文案一致（工具 × 本机 OS）：{err}"
            ),
            other => panic!("macOS 分诊不可达应保持 Failed，实际 {other:?}"),
        }
    }

    /// 插队 best-effort：假 pid 下排空查询基础设施失败（wait_input_drained Err）→
    /// 以「写入成功」为准返回 Ok（诊断通道不可用不得误报投递超时）；macOS 无
    /// drain 可等 → 直接 Ok（同断言跨平台恒真）
    #[test]
    fn jump_receipt_best_effort_when_drain_unavailable() {
        let st = state_with(
            vec![sess("s-cj", SessionStatus::Processing, 22)],
            FakeInjector::ok(),
        );
        let s = sess("s-cj", SessionStatus::Processing, 22);
        assert_eq!(
            super::super::confirm::await_jump_receipt(&st, &s, "插队确认消息"),
            Ok(()),
            "排空查询基础设施失败走 best-effort（以写入成功为准）"
        );
    }

    /// 契约/测试面 API 回归（session_stamp_hit 唯一消费者=测试模块——生产确认经
    /// confirm_probe 缝直调 content::read_session_messages，不经本函数）：本测覆盖
    /// 其复用 message_source 读路径的失败语义；读失败 = 未命中（诚实口径——确认
    /// 不足不伪装成功）
    #[test]
    fn session_stamp_hit_misses_when_read_fails() {
        // state_with 的 message_source 为恒 Err 桩：read_session_messages_core
        // 经缝读失败 → false
        let st = state_with(
            vec![sess("s-sh", SessionStatus::Waiting, 23)],
            FakeInjector::ok(),
        );
        assert!(
            !super::super::confirm::session_stamp_hit(&st, "claude", "s-sh", "任意戳"),
            "读失败必须判未命中"
        );
    }

    /// flush_one 端到端确认分诊（A1 主回执闭环 + D7/T3 判据收紧，原
    /// `flush_one_end_to_end_confirm_failure` 按新语义更新）：注入 Ok + probe 恒
    /// false（小超时覆盖，无 5s 慢测）——**Windows**：屏读（假 pid）无滞留草稿 →
    /// `Submitted` 中性：行 mark_sent 消费退出 pending（消息已在 TUI 手里，重投
    /// 即双发）、审计 action=flush result=unconfirmed 与确认送达/确认失败三分、
    /// 不落 failed_reason；**非 Windows**：分诊不可达 → `Failed` 既有口径不变
    /// （mark_failed + failed:{e} + 防重警示文案全句）。注入确实发生的前提自证
    /// 两臂共守（确认分诊而非注入失败）。入队/断言全走 st.store 同一内存库
    #[test]
    fn flush_one_end_to_end_unconfirmed_triage() {
        let fake = FakeInjector::ok();
        let st = state_with_probe(
            vec![sess("s-e2f", SessionStatus::Waiting, 24)],
            fake.clone(),
            std::sync::Arc::new(|_, _, _| false),
        );
        let item_id = st.store.with(|c| enq(c, "s-e2f", "端到端确认消息"));

        let outcome = flush_one_with(&st, "s-e2f", false, Some(60));
        assert_eq!(
            fake.recorded(),
            vec![(24, "端到端确认消息".to_string())],
            "前提自证：注入确实发生（分诊发生在注入成功之后）"
        );
        #[cfg(windows)]
        {
            assert_eq!(
                outcome,
                FlushOutcome::Submitted,
                "Windows 无滞留草稿证据 → 已投递未确认（中性非失败）"
            );
            let (failed_reason, sent_at) = st.store.with(|c| {
                let row = inject_queue::get_conn(c, item_id).unwrap();
                (row.failed_reason, row.sent_at)
            });
            assert_eq!(
                failed_reason, None,
                "Submitted 非失败：不落 failed_reason（不诱导重试）"
            );
            assert!(
                sent_at.is_some(),
                "Submitted 行 mark_sent 消费（退出 pending，防 flush 循环重投 = 双发）"
            );
            let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
            assert_eq!(audits.len(), 1);
            assert_eq!(audits[0].action, "flush");
            assert_eq!(
                audits[0].result, "unconfirmed",
                "审计单列 unconfirmed：不冒充确认成功（ok）也不冒充失败（failed:e）"
            );
        }
        #[cfg(not(windows))]
        {
            let FlushOutcome::Failed(e) = outcome else {
                panic!("非 Windows 分诊不可达应保持 Failed，实际 {outcome:?}")
            };
            assert!(
                e.contains("已注入未确认（未见会话记录），请检查终端后重试"),
                "非 Windows 保持失败回执文案全句：{e}"
            );
            let (failed_reason, sent_at) = st.store.with(|c| {
                let row = inject_queue::get_conn(c, item_id).unwrap();
                (row.failed_reason, row.sent_at)
            });
            assert_eq!(
                failed_reason.as_deref(),
                Some(e.as_str()),
                "mark_failed 落 failed_reason 且与回执同源"
            );
            assert_eq!(sent_at, None, "确认失败不得落 sent_at");
            let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
            assert_eq!(audits.len(), 1);
            assert_eq!(audits[0].action, "fail");
            assert_eq!(audits[0].result, format!("failed:{e}"));
        }
        assert!(
            st.store
                .with(|c| inject_queue::next_pending_conn(c, "s-e2f"))
                .is_none(),
            "行退出 pending（Submitted 防重投 / Failed 不自动重发，均不残留）"
        );
    }

    /// settle 的 Submitted 落账（D7/T3，跨平台纯核）：mark_sent 消费 + 审计
    /// action=flush result=unconfirmed——「确认送达 ok / 已投递未确认 unconfirmed /
    /// 确认失败 failed:e」三分，不冒充成功也不冒充失败；settle 返回 Ok（非失败）
    #[test]
    fn settle_submitted_marks_sent_and_audits_unconfirmed() {
        let c = mem();
        enq(&c, "s-sub", "已投递未确认消息");
        let fake = FakeInjector::ok();
        let st = state_with(vec![sess("s-sub", SessionStatus::Waiting, 35)], fake);
        let item = inject_queue::next_pending_conn(&c, "s-sub").unwrap();

        assert!(
            settle(&c, &st, &item, false, FlushOutcome::Submitted).is_ok(),
            "Submitted 非失败（settle Ok）"
        );
        let row = inject_queue::get_conn(&c, item.id).unwrap();
        assert!(
            row.sent_at.is_some(),
            "mark_sent 消费（行退出 pending，防重投 = 双发）"
        );
        assert_eq!(row.failed_reason, None, "非失败不落 failed_reason");
        let audits = write_audit::recent_conn(&c, 10);
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "flush");
        assert_eq!(audits[0].result, "unconfirmed");
        assert!(
            inject_queue::next_pending_conn(&c, "s-sub").is_none(),
            "Submitted 行退出 pending（flush 循环不重投）"
        );
    }

    /// 时序锁：注入 Err 短路确认——记录型 probe 在注入失败路径上零调用
    /// （确认只在注入成功后起跑，FakeInjector 调用记录自证注入确已尝试）
    #[test]
    fn inject_failure_short_circuits_confirm_probe() {
        let probe_calls: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = probe_calls.clone();
        let st = state_with_probe(
            vec![sess("s-sc", SessionStatus::Waiting, 25)],
            FakeInjector::failing("定位终端失败：pid 不存在"),
            std::sync::Arc::new(move |t: &str, s: &str, stamp: &str| {
                cap.lock()
                    .unwrap()
                    .push((t.to_string(), s.to_string(), stamp.to_string()));
                true
            }),
        );
        let item_id = st.store.with(|c| enq(c, "s-sc", "时序锁消息"));

        assert!(
            matches!(flush_one(&st, "s-sc", false), FlushOutcome::Failed(_)),
            "注入失败必须返回 Failed（P1-4 四态上抛）"
        );
        assert!(
            probe_calls.lock().unwrap().is_empty(),
            "注入失败必须短路确认（probe 零调用——时序锁）"
        );
        let sent_at = st
            .store
            .with(|c| inject_queue::get_conn(c, item_id).unwrap().sent_at);
        assert_eq!(sent_at, None, "注入失败不得落 sent_at");
        let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "fail");
        assert_eq!(audits[0].result, "failed:定位终端失败：pid 不存在");
    }

    /// 插队端到端不消费 confirm_probe（best-effort 恒 Sent）：probe 恒 false 也不得
    /// 把插队投递打成失败（busy 态无文件戳可查——裁决 A1 插队语义，flush_one 全链路）
    #[test]
    fn jump_delivery_ignores_confirm_probe() {
        let fake = FakeInjector::ok();
        let st = state_with_probe(
            vec![sess("s-jp", SessionStatus::Processing, 26)],
            fake.clone(),
            std::sync::Arc::new(|_, _, _| false),
        );
        let item_id = st.store.with(|c| enq(c, "s-jp", "插队端到端消息"));

        assert_eq!(
            flush_one(&st, "s-jp", true),
            FlushOutcome::Sent,
            "插队确认 best-effort：probe 恒 false 不得改写投递结论"
        );
        let sent_at = st
            .store
            .with(|c| inject_queue::get_conn(c, item_id).unwrap().sent_at);
        assert!(sent_at.is_some(), "插队照常 mark_sent");
        let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "jump");
        assert_eq!(audits[0].result, "ok");
    }

    /// 收尾批 P2（jump 守卫内 still_pending 复查抽函数）：flush_given_if_pending
    /// 两分支——①条目仍 pending → Sent 且注入发生；②条目已被消费（先 mark_sent）
    /// → Deferred 且注入零发生（复查缺失即对已 sent 行再注入=双投，本测是防线钉）。
    /// 调用形态对齐端点闭包：先取 in-flight 守卫，再进复查+投递内核
    #[test]
    fn jump_deliver_under_guard() {
        // ① 条目仍 pending → Sent + 注入 + mark_sent
        let fake = FakeInjector::ok();
        let st = state_with(
            vec![sess("s-jg", SessionStatus::Processing, 40)],
            fake.clone(),
        );
        let item_id = st.store.with(|c| enq(c, "s-jg", "守卫下投递消息"));
        let item = st
            .store
            .with(|c| inject_queue::get_conn(c, item_id))
            .expect("入队行必须可取");
        let _guard = try_acquire_inflight("s-jg").expect("空闲会话必须取到守卫");
        assert_eq!(
            flush_given_if_pending(&st, &item, true),
            FlushOutcome::Sent,
            "仍 pending 必须照发（jump 插队语义）"
        );
        drop(_guard);
        assert_eq!(
            fake.recorded(),
            vec![(40u32, "守卫下投递消息".to_string())],
            "注入必须真实发生"
        );
        let sent_at = st
            .store
            .with(|c| inject_queue::get_conn(c, item_id).unwrap().sent_at);
        assert!(sent_at.is_some(), "Sent 落 mark_sent");

        // ② 条目已被消费（先 mark_sent，模拟间隙内 flush 循环已投递）→ Deferred
        // 且注入零发生
        let fake2 = FakeInjector::ok();
        let st2 = state_with(
            vec![sess("s-jg2", SessionStatus::Processing, 41)],
            fake2.clone(),
        );
        let item2_id = st2.store.with(|c| enq(c, "s-jg2", "已被消费消息"));
        st2.store.with(|c| {
            inject_queue::mark_sent_conn(c, item2_id, chrono::Utc::now().timestamp_millis())
        });
        let item2 = st2
            .store
            .with(|c| inject_queue::get_conn(c, item2_id))
            .expect("已消费行仍可取（行保留作审计痕迹）");
        let _guard2 = try_acquire_inflight("s-jg2").expect("空闲会话必须取到守卫");
        assert_eq!(
            flush_given_if_pending(&st2, &item2, true),
            FlushOutcome::Deferred,
            "已被消费的条目必须让位（端点按 queued/position=0 回执）"
        );
        drop(_guard2);
        assert!(
            fake2.recorded().is_empty(),
            "已消费条目零注入（双投防线的存在意义）"
        );
    }

    // ==== 队列生命周期（裁决 19 停服冻结 + P2-5 对账/兜底 + P1-4 四态上抛） ====

    /// 裁决 19（停止远程 = 冻结队列）：spawn_flush_loop 后 FLUSH_LOOP_HANDLE 槽 live；
    /// abort_flush_loop 后槽空；重复 abort 幂等无害（槽空 take 得 None）。
    /// Important 4：全程持 LOOP_HANDLE_TEST_LOCK——remote::mod 两个真实调
    /// abort_flush_loop 的 stop_server_core 内核测试同锁串行化（同一全局槽，并行会
    /// 假红），删除重试启发式、确定性优先
    #[test]
    fn stop_freezes_flush_loop() {
        let _serial = LOOP_HANDLE_TEST_LOCK.lock().unwrap();
        let st = std::sync::Arc::new(state_with(
            vec![sess("s-frz", SessionStatus::Waiting, 30)],
            FakeInjector::ok(),
        ));
        spawn_flush_loop(st.clone());
        assert!(
            flush_loop_handle_is_live(&FLUSH_LOOP_HANDLE.lock().unwrap()),
            "spawn 后句柄槽必须 live"
        );
        abort_flush_loop();
        assert!(
            FLUSH_LOOP_HANDLE.lock().unwrap().is_none(),
            "abort 后句柄槽必须清空（冻结队列：投递循环随服务同停）"
        );
        // 幂等：重复调用无害
        abort_flush_loop();
        assert!(FLUSH_LOOP_HANDLE.lock().unwrap().is_none());
    }

    /// P2-5 启动对账：重启前遗留 pending（会话现为可输入态 Waiting）→ reconcile_once
    /// 逐会话补投——行 mark_sent + 审计 action=flush（与常规跃迁投递同内核同落账口径，
    /// 数据同源经 flush_one → try_flush → session_source）
    #[test]
    fn reconcile_flushes_pending_on_restart_like_state() {
        let fake = FakeInjector::ok();
        let st = std::sync::Arc::new(state_with(
            vec![sess("s-rec", SessionStatus::Waiting, 31)],
            fake.clone(),
        ));
        let item_id = st.store.with(|c| enq(c, "s-rec", "重启遗留消息"));

        reconcile_once(&st);

        assert_eq!(
            fake.recorded(),
            vec![(31u32, "重启遗留消息".to_string())],
            "对账必须真实补投（不得绕过注入源自立投递）"
        );
        let row = st
            .store
            .with(|c| inject_queue::get_conn(c, item_id))
            .expect("对账后行应保留（审计痕迹）");
        assert!(row.sent_at.is_some(), "对账补投必须 mark_sent");
        let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1, "对账补投恰一条审计");
        assert_eq!(
            audits[0].action, "flush",
            "对账审计与常规跃迁同口径（action=flush）"
        );
        assert_eq!(audits[0].result, "ok");
    }

    /// 灰2 周期兜底门控（P2-5 计数门守宪法扫描预算）：pending=0 → sweep_if_pending
    /// 在纯 SQL 计数门直接短路——零 flush 零解析（假 session_source 零消费）
    #[test]
    fn periodic_sweep_gated_by_pending_count() {
        let fake = FakeInjector::ok();
        let st = std::sync::Arc::new(state_with(
            vec![sess("s-swp", SessionStatus::Waiting, 32)],
            fake.clone(),
        ));

        sweep_if_pending(&st);

        assert!(
            fake.recorded().is_empty(),
            "pending=0 计数门必须跳过（不得触发任何 flush）：{:?}",
            fake.recorded()
        );
    }

    /// 灰2/P2-5 周期兜底正向（评审 Minor 9）：pending>0 → 计数门放行 → reconcile
    /// 补投成功——注入器收到消息、行 mark_sent、审计 action=flush（与启动对账同内核）
    #[test]
    fn periodic_sweep_flushes_when_pending_present() {
        let fake = FakeInjector::ok();
        let st = std::sync::Arc::new(state_with(
            vec![sess("s-swp2", SessionStatus::Waiting, 34)],
            fake.clone(),
        ));
        st.store.with(|c| enq(c, "s-swp2", "兜底补投消息"));

        sweep_if_pending(&st);

        assert_eq!(
            fake.recorded(),
            vec![(34u32, "兜底补投消息".to_string())],
            "pending>0 计数门必须放行（补投经 flush_one 同源内核）"
        );
        let audits = st.store.with(|c| write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1, "兜底补投恰一条审计");
        assert_eq!(audits[0].action, "flush");
        assert_eq!(audits[0].result, "ok");
    }

    /// P1-4 五态上抛（flush_one 薄壳透传内核结论）：黄态 → Deferred；会话消失 →
    /// Suspended（两态行均保持 pending 不落账；delivered/submitted/queued/failed
    /// 的端点映射在 server.rs 端点测试覆盖）
    #[test]
    fn flush_outcome_passthrough() {
        // 黄态（Processing）常规路径 → Deferred
        let st = state_with(
            vec![sess("s-pd", SessionStatus::Processing, 33)],
            FakeInjector::ok(),
        );
        st.store.with(|c| enq(c, "s-pd", "黄态消息"));
        assert_eq!(flush_one(&st, "s-pd", false), FlushOutcome::Deferred);
        assert!(
            st.store
                .with(|c| inject_queue::next_pending_conn(c, "s-pd"))
                .is_some(),
            "Deferred 行保持 pending（等下个可输入态事件）"
        );
        assert!(
            st.store
                .with(|c| write_audit::recent_conn(c, 10))
                .is_empty(),
            "Deferred 不落账不写审计（评审 Minor 9：挂起/等待非终态）"
        );

        // 会话消失（空快照）→ Suspended
        let st2 = state_with(vec![], FakeInjector::ok());
        st2.store.with(|c| enq(c, "s-gone2", "消失消息"));
        assert_eq!(flush_one(&st2, "s-gone2", false), FlushOutcome::Suspended);
        assert!(
            st2.store
                .with(|c| inject_queue::next_pending_conn(c, "s-gone2"))
                .is_some(),
            "Suspended 行保持 pending（红·中断挂起，W2）"
        );
        assert!(
            st2.store
                .with(|c| write_audit::recent_conn(c, 10))
                .is_empty(),
            "Suspended 不落账不写审计（红·中断非终态）"
        );
    }
}
