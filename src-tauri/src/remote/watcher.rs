// SessionWatcher：2s 周期快照 diff → broadcast（跃迁判定唯一去重点）
// 消费者（SSE/推送/桌面通知）只收边沿事件，不独立去重

use std::collections::HashMap;
use std::sync::{Once, OnceLock};
use std::time::Duration;

use tokio::sync::broadcast;

use crate::session::{Session, SessionStatus};

/// 事件通道容量（broadcast 语义：消费者滞后超容量时丢最旧事件——宁可缺边沿也不阻塞
/// watcher 生产；SSE 首帧全量快照可校正状态）
pub const EVENT_CHANNEL_CAPACITY: usize = 64;

/// 扫描周期（A3：状态变化 2 秒级到达）
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, serde::Serialize)]
pub struct TransitionEvent {
    pub session_id: String,
    pub agent_type: String,
    pub from: String,
    pub to: String,
    pub project_name: String,
    pub last_message: Option<String>,
    pub ts: i64,
}

/// 事件通道单例槽：通道全进程唯一（RemoteState 装配与扫描循环共用同一 sender）
static EVENT_TX: OnceLock<broadcast::Sender<TransitionEvent>> = OnceLock::new();

/// 扫描循环单次启动闩：重复 start 不得重复 spawn——多个 watcher 会让同一跃迁被广播
/// 多份，破坏「去重唯一来源」铁律（消费者只认边沿、不独立去重）
static LOOP_STARTED: Once = Once::new();

/// 事件 sender（幂等：首次调用建通道，之后恒返回同一 sender），**不启动**扫描循环。
/// RemoteState.watcher_tx 装配用：即便服务器从未启动成功也不会遗留后台扫描
pub fn event_sender() -> broadcast::Sender<TransitionEvent> {
    channel_at(&EVENT_TX)
}

/// 通道幂等创建内核（可测核心，无全局依赖）
fn channel_at(
    slot: &OnceLock<broadcast::Sender<TransitionEvent>>,
) -> broadcast::Sender<TransitionEvent> {
    slot.get_or_init(|| broadcast::channel(EVENT_CHANNEL_CAPACITY).0)
        .clone()
}

pub struct SessionWatcher;

impl SessionWatcher {
    /// 启动 2s 周期 watcher，返回事件 broadcast sender（与 event_sender 同一通道）。
    /// 幂等：循环只 spawn 一次，重复调用返回既有 sender、不重复起任务（见 LOOP_STARTED）。
    /// 调用点必须在**服务器实际监听成功后**（serve 内 bind 之后）；绑定失败不留后台扫描。
    pub fn start() -> broadcast::Sender<TransitionEvent> {
        start_with(&EVENT_TX, &LOOP_STARTED, |tx| {
            // 见 M2 SERVER_HANDLE 同款裁决（remote/mod.rs）：裸 tokio::spawn 在无
            // runtime 上下文的线程上 panic（no reactor running），
            // tauri::async_runtime::spawn 内部先 enter 再 spawn，任意线程可用
            tauri::async_runtime::spawn(watch_loop(tx));
        })
    }
}

/// 启动内核（可测核心，通道槽 / 单次闩 / spawn 全注入）：通道幂等创建 + 循环只 spawn 一次
fn start_with(
    tx_slot: &OnceLock<broadcast::Sender<TransitionEvent>>,
    loop_once: &Once,
    spawn: impl FnOnce(broadcast::Sender<TransitionEvent>),
) -> broadcast::Sender<TransitionEvent> {
    let tx = channel_at(tx_slot);
    loop_once.call_once(|| spawn(tx.clone()));
    tx
}

/// 周期任务主体：sleep → 扫描 → diff → 广播 → 刷新基线。
/// 扫描在阻塞线程池执行（与 api.rs 的 sessions handler 同先例）：get_all_sessions 是
/// 同步重活（sysinfo 全进程刷新 + 各工具会话解析），直接 await 会周期性堵死 tokio worker
async fn watch_loop(tx: broadcast::Sender<TransitionEvent>) {
    let mut prev: Vec<Session> = Vec::new();
    loop {
        tokio::time::sleep(SCAN_INTERVAL).await;
        let resp = match tokio::task::spawn_blocking(crate::adapter::get_all_sessions).await {
            Ok(resp) => resp,
            Err(e) => {
                // 扫描任务 panic（如 DB 锁中毒）：保留旧基线，下轮重试，任务不退出
                log::error!("SessionWatcher 会话扫描任务异常: {e}");
                continue;
            }
        };
        for e in diff_transitions(&prev, &resp.sessions) {
            // 无订阅者时 send 返回 Err：正常（尚无可消费的 SSE 连接），忽略
            let _ = tx.send(e);
        }
        prev = resp.sessions;
    }
}

/// 纯函数：快照 diff → 边沿事件（可单测；去重唯一来源）。
/// 语义（控制者裁决 2026-09-15，对简报的修订）：**只对 prev 中已存在且状态变化的会话
/// 发事件**——新增会话（prev 无）不发（新卡由全量快照自然呈现，事件语义是「状态变化」；
/// 也避免服务开启瞬间给全部在跑会话刷一轮伪跃迁）。
/// 键取 (工具, 会话 id)：与 adapter::dedup_sessions 同一唯一性口径——id 只在工具内唯一，
/// 不同工具撞 id 不得互相顶掉基线
pub fn diff_transitions(prev: &[Session], curr: &[Session]) -> Vec<TransitionEvent> {
    let prev_map: HashMap<(&'static str, &str), &Session> = prev
        .iter()
        .map(|s| ((s.agent_type.tool_id(), s.id.as_str()), s))
        .collect();
    let now = chrono::Utc::now().timestamp_millis();
    curr.iter()
        .filter_map(|c| {
            let p = prev_map.get(&(c.agent_type.tool_id(), c.id.as_str()))?;
            if p.status != c.status {
                Some(TransitionEvent {
                    session_id: c.id.clone(),
                    agent_type: c.agent_type.tool_id().to_string(),
                    from: status_wire(&p.status),
                    to: status_wire(&c.status),
                    project_name: c.project_name.clone(),
                    last_message: c.last_message.clone(),
                    ts: now,
                })
            } else {
                None
            }
        })
        .collect()
}

/// 状态 wire 字符串（serde 单源）：SessionStatus 带 `#[serde(rename_all = "lowercase")]`，
/// 其 serde 序列化即 wire 形态（与 /sessions 快照的 status 字段同一路径，前端
/// src/types/session.ts 的 SessionStatus 联合类型消费）。
/// 不用 `format!("{:?}", variant).to_lowercase()`：那是「变体名小写 == wire 字符串」的
/// 隐式约定（先例警告与穷尽 match 先例见 session/model.rs:20 的 tool_id 注释）
fn status_wire(s: &SessionStatus) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            // 理论不可达（C 风格枚举 + derive(Serialize) 必产出 String）：防御性降级为
            // 空串而非 panic——watcher 循环不得因单个事件构造异常而中断
            log::warn!("SessionStatus 序列化未产出字符串，事件状态字段降级为空");
            String::new()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use crate::session::{AgentType, ProcessForm};

    fn session(id: &str, status: SessionStatus, project: &str) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::Claude,
            project_name: project.into(),
            project_path: String::new(),
            title: None,
            git_branch: None,
            github_url: None,
            status,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid: 1,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        }
    }

    fn ev(id: &str) -> TransitionEvent {
        TransitionEvent {
            session_id: id.into(),
            agent_type: "claude".into(),
            from: "idle".into(),
            to: "processing".into(),
            project_name: "p".into(),
            last_message: None,
            ts: 1,
        }
    }

    /// 控制者裁决（2026-09-15，对简报的修订）：新增会话**不产生**跃迁事件——
    /// 事件语义是「状态变化」，新卡由全量快照自然呈现。故 c 不再计入 events
    #[test]
    fn diff_emits_only_state_changes() {
        let prev = vec![
            session("a", SessionStatus::Processing, "p1"),
            session("b", SessionStatus::Idle, "p2"),
        ];
        let curr = vec![
            session("a", SessionStatus::Waiting, "p1"), // changed → emit
            session("b", SessionStatus::Idle, "p2"),    // same → skip
            session("c", SessionStatus::Processing, "p3"), // new → skip（裁决）
        ];
        let events = diff_transitions(&prev, &curr);
        assert_eq!(
            events.len(),
            1,
            "仅 a（状态变化）触发事件；b 同态、c 新增均不发"
        );
        assert_eq!(events[0].session_id, "a");
        assert_eq!(events[0].from, "processing");
        assert_eq!(events[0].to, "waiting");
        assert_eq!(
            events[0].agent_type, "claude",
            "agent_type 用 tool_id() 穷尽映射"
        );
        assert_eq!(events[0].project_name, "p1");
        assert!(events[0].ts > 0, "ts 应为当前毫秒时间戳");
    }

    #[test]
    fn diff_no_change_no_event() {
        let prev = vec![session("a", SessionStatus::Idle, "p")];
        let curr = vec![session("a", SessionStatus::Idle, "p")];
        assert!(diff_transitions(&prev, &curr).is_empty());
    }

    /// 显式锁定「新增不发事件」语义（控制者裁决⑤）：首轮空基线、已有会话旁新增、
    /// 消失会话三个场景均不得产生事件
    #[test]
    fn diff_skips_new_sessions() {
        // 首轮基线为空（watcher 冷启动）：全部当前会话都是"新增"→ 零事件，
        // 否则服务开启瞬间会给所有在跑会话刷一轮伪跃迁
        let first = vec![session("a", SessionStatus::Processing, "p1")];
        assert!(
            diff_transitions(&[], &first).is_empty(),
            "空基线首轮不得产生事件"
        );
        // 已有基线旁出现新会话 → 只发既有会话的变化，新会话不发
        let prev = vec![session("a", SessionStatus::Idle, "p1")];
        let curr = vec![
            session("a", SessionStatus::Waiting, "p1"),
            session("new", SessionStatus::Processing, "p9"),
        ];
        let events = diff_transitions(&prev, &curr);
        assert_eq!(events.len(), 1, "仅既有会话 a 的变化计事件");
        assert_eq!(events[0].session_id, "a");
        // 会话从当前快照消失（进程退出）→ 不产生事件（diff 只遍历 curr）
        assert!(
            diff_transitions(&prev, &[]).is_empty(),
            "消失会话不产生事件（当前快照已无该会话）"
        );
    }

    /// 状态 wire 字符串与前端 SessionStatus 联合类型（src/types/session.ts）一致：
    /// 全变体枚举断言，serde rename 漂移即刻变红
    #[test]
    fn status_wire_matches_frontend_union() {
        for (s, want) in [
            (SessionStatus::Waiting, "waiting"),
            (SessionStatus::Processing, "processing"),
            (SessionStatus::Thinking, "thinking"),
            (SessionStatus::Compacting, "compacting"),
            (SessionStatus::Idle, "idle"),
            (SessionStatus::Finished, "finished"),
        ] {
            assert_eq!(
                status_wire(&s),
                want,
                "wire 字符串必须与前端 SessionStatus 联合类型逐字一致"
            );
        }
    }

    /// 全局通道单例：event_sender 每次返回同一通道——RemoteState.watcher_tx（订阅端）与
    /// 扫描循环（生产端）必须指向同一通道，否则 SSE 永远收不到事件（静默断链）。
    /// 零污染：只建一个空通道，不 spawn、不触 DB/文件系统
    #[test]
    fn event_sender_is_singleton_channel() {
        let a = event_sender();
        let b = event_sender();
        assert!(
            a.same_channel(&b),
            "event_sender 必须返回全进程唯一通道（Task 6 的 SSE 订阅依赖此不变量）"
        );
    }

    /// 幂等启动内核：第二次 start 必须复用既有 sender 且不再 spawn——
    /// 重复 spawn 会让同一跃迁广播多份，破坏「去重唯一来源」铁律。
    /// 零污染：槽与闩都是局部变量、spawn 是计数闭包，不触全局 EVENT_TX、不扫真实会话
    #[test]
    fn start_with_spawns_once_and_reuses_sender() {
        let tx_slot: OnceLock<broadcast::Sender<TransitionEvent>> = OnceLock::new();
        let loop_once = Once::new();
        let spawns = Arc::new(AtomicUsize::new(0));
        let count = |spawns: &Arc<AtomicUsize>| {
            let spawns = spawns.clone();
            move |_tx: broadcast::Sender<TransitionEvent>| {
                spawns.fetch_add(1, Ordering::SeqCst);
            }
        };
        let s1 = start_with(&tx_slot, &loop_once, count(&spawns));
        let s2 = start_with(&tx_slot, &loop_once, count(&spawns));
        assert_eq!(
            spawns.load(Ordering::SeqCst),
            1,
            "第二次 start 必须复用既有 watcher，不得重复 spawn"
        );
        assert!(
            tx_slot.get().is_some(),
            "启动后槽内应有 sender（后续 start 的复用来源）"
        );
        // 同一通道证明：s1 发出、s2 订阅可收
        let mut rx = s2.subscribe();
        s1.send(ev("x")).unwrap();
        assert_eq!(rx.try_recv().unwrap().session_id, "x");
    }
}
