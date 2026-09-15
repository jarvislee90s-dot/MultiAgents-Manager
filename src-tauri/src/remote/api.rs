// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ host（P8a/P8b 页头数据）
// + pair + heartbeat + events（M3 Task 6 SSE 实时通道）

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{sse, IntoResponse, Response, Sse},
    Json,
};
use futures::stream::{Stream, StreamExt as _};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

use super::gate::COOKIE_NAME;
use super::server::RemoteState;

/// GET /m/api/v1/sessions：数据源注入（生产 = adapter::get_all_sessions），
/// `SessionsResponse` 已 camelCase 序列化（前端约定 totalCount）。
///
/// 扫描必须在 `spawn_blocking` 里跑：生产源 `adapter::get_all_sessions` 是**同步阻塞**调用
/// （sysinfo 全进程刷新 + 各工具会话解析，冷启动可达数秒），直接放在 async handler 里会
/// 周期性堵死 tokio worker——移动端 3s 轮询下即为持续拖垮（桌面侧同一函数已按此包，
/// 实机教训见 `commands/session.rs` 的 `get_all_sessions`）。
pub async fn sessions(State(st): State<Arc<RemoteState>>) -> impl IntoResponse {
    // 闭包捕获 state 的 Arc（Send + Sync + 'static）：`session_source` 是 Box<dyn Fn> 不可
    // clone，故整体 move 进阻塞线程池，在池内调用注入源
    let st = st.clone();
    let response = tokio::task::spawn_blocking(move || (st.session_source)())
        .await
        .unwrap_or_else(|e| {
            // JoinError（任务 panic/取消）降级为空响应：移动端拿到 0 会话而非连接错误，
            // 与桌面侧 sessions 命令的降级形态一致
            log::error!("远程会话扫描任务异常: {e}");
            crate::session::SessionsResponse {
                sessions: Vec::new(),
                total_count: 0,
                waiting_count: 0,
            }
        });
    // M2-R2 顺手项：会话数据是设备门禁下的私有数据，禁止中间层/浏览器缓存
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(response),
    )
        .into_response()
}

/// 空快照 JSON（与 SessionsResponse camelCase 序列化同形）：快照序列化理论不可达失败
/// （C 风格字段无 map 键/浮点 NaN）时的防御性降级——前端拿到合法空载荷而非空串
/// （空串会让 JSON.parse 抛错、看板卡死）
const EMPTY_SESSIONS_JSON: &str = r#"{"sessions":[],"totalCount":0,"waitingCount":0}"#;

/// GET /m/api/v1/events（M3 Task 6，C1 后半）：SSE 实时通道。
/// - 首帧 = 全量会话快照（**直调注入源**，数据同源铁律 3），此后只推跃迁边沿
///   （watcher 是唯一去重点，铁律 4——本层不独立去重、来一条推一条）；
/// - gate 由 nest 内层 layer 结构性覆盖（本端点注册在 api_router 内），无需另加 middleware；
/// - 心跳用 `Sse::keep_alive(KeepAlive::default())`：axum 自带 15s 空注释帧
///   （简报的 `: ping` 等效物，无需自拼 interval 流——简报 Step 1 注释"心跳省略实现"即指此）；
/// - 断流：客户端断开 → axum drop 本流 → BroadcastStream 释放 Receiver →
///   receiver_count 归零 → watcher 零订阅者守卫跳过扫描（Task 5 闭环）。
pub async fn events(
    State(st): State<Arc<RemoteState>>,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    // 先订阅、后取快照（顺序刻意，勿换）：subscribe 在快照计算之前，期间产生的跃迁
    // 落进 broadcast 缓冲（容量 64）并在快照帧之后依次送出；若反序，快照与订阅之间
    // 发生的跃迁会永久丢失（重连后的看板状态与真实脱节）
    let rx = st.watcher_tx.subscribe();
    // 快照走 spawn_blocking：session_source 是同步阻塞调用（sysinfo 全进程刷新 + 各工具
    // 会话解析，冷启动可达数秒），直接 await 会周期性堵死 tokio worker——与 sessions
    // handler 同一先例（实机教训见 commands/session.rs 的 get_all_sessions 注释）
    let snapshot = tokio::task::spawn_blocking(move || (st.session_source)())
        .await
        .unwrap_or_else(|e| {
            // JoinError（任务 panic/取消）降级为空快照：移动端拿到 0 会话而非断流
            log::error!("SSE 快照会话扫描任务异常: {e}");
            crate::session::SessionsResponse {
                sessions: Vec::new(),
                total_count: 0,
                waiting_count: 0,
            }
        });
    let snapshot_json = serde_json::to_string(&snapshot).unwrap_or_else(|e| {
        log::warn!("SSE 快照序列化失败，降级为空快照: {e}");
        EMPTY_SESSIONS_JSON.to_string()
    });
    let initial = futures::stream::once(async move {
        Ok(sse::Event::default().event("snapshot").data(snapshot_json))
    });
    let transitions = BroadcastStream::new(rx).filter_map(|msg| async move {
        // Lagged（消费者落后超通道容量）与 Closed 均非致命：broadcast 丢最旧事件是通道
        // 语义（宁可缺边沿也不阻塞 watcher 生产端），重连时的全量快照负责校正——丢弃该条继续流
        let e = msg.ok()?;
        match serde_json::to_string(&e) {
            Ok(data) => Some(Ok(sse::Event::default().event("transition").data(data))),
            Err(err) => {
                // 不产出空 data 帧：前端 JSON.parse("") 会抛错且该帧无信息量
                log::warn!("跃迁事件序列化失败，丢弃该帧: {err}");
                None
            }
        }
    });
    Sse::new(initial.chain(transitions)).keep_alive(sse::KeepAlive::default())
}

#[derive(Deserialize)]
pub struct PairReq {
    pub token: String,
}

/// POST /m/api/v1/pair：一次性 token 换设备 cookie。
/// 不变量③：Invalid / Expired / Used 一律 403、响应体无差异（不给有效性预言机）
pub async fn pair(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PairReq>,
) -> Result<Response, StatusCode> {
    use crate::remote::pairing::AcceptResult;
    let now = chrono::Utc::now().timestamp_millis();
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // 先把 pairing 锁收窄到 accept() 一步：accept 返回后立即释放 MutexGuard，再做阻塞 DB 写。
    // 原写法持着 pairing 锁调 persist_device（锁序 pairing→DB 阻塞段），Task 4 的 stop_server
    // 是 store→pairing 顺序，虽非嵌套暂不反转，但两条顺序并存即是未来死锁的种子（评审 Minor）
    let device_id = {
        let mut svc = st.pairing.lock().unwrap();
        match svc.accept(&req.token) {
            AcceptResult::Ok { device_id } => device_id,
            // 不变量③：Invalid / Expired / Used 一律 403、响应体无差异（不给有效性预言机）
            _ => return Err(StatusCode::FORBIDDEN),
        }
        // svc（MutexGuard）在此作用域结束处 drop：后续 persist 不再持 pairing 锁
    };
    let dev = crate::remote::pairing::NewDevice {
        id: device_id.clone(),
        name: String::new(),
        ua,
        origin_ip: String::new(),
        paired_at: now,
    };
    // 持久化失败不阻断本次配对（下次 gate 校验会失败）——保持简报行为
    st.store.with(|c| {
        let _ = crate::remote::pairing::persist_device(c, &dev);
    });
    // HttpOnly + SameSite=Lax + Path=/m + 180d（dsh 七不变量之 cookie 语义）
    let cookie = format!(
        "{COOKIE_NAME}={device_id}; Path=/m; HttpOnly; SameSite=Lax; Max-Age={}",
        crate::remote::pairing::DEVICE_TTL_MS / 1000
    );
    Ok((
        [(axum::http::header::SET_COOKIE, cookie)],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response())
}

/// POST /m/api/v1/heartbeat：gate 已刷新 last_seen（touch），此处仅回 pong 供客户端保活判定
/// M3 保活判定预留，当前 sessions 轮询 gate touch 已覆盖
pub async fn heartbeat() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}

/// GET /m/api/v1/host（M3 Task 1）：移动看板页头品牌行（P8a 版本 + P8b 本机名 +
/// P8d enabledTools 数据源）。gate 内自动覆盖（nest 内层 layer，无需另加 middleware）；
/// host 信息运行期不变，移动端挂载时拉一次即可，无需轮询。
/// 直调 host_source 注入缝（生产 = remote::host_info()，与 remote_status 的 host 部分同源，
/// 禁止复制聚合逻辑）——读 settings DAO 是轻量 SQLite 查询，无需 spawn_blocking
pub async fn host(State(st): State<Arc<RemoteState>>) -> impl IntoResponse {
    // 门禁下的私有数据（本机名/启用工具），与会话数据同样禁止中间层缓存
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json((st.host_source)()),
    )
        .into_response()
}

/// GET /m/api/v1/session-messages?agent_type=&session_id=&limit=（M3 Task 7，C2 后端）
/// 八工具统一内容出口（P9）：`{messages: [{seq, role, content, kind, ts, toolName?,
/// toolArgs?, collapsed}]}`（SessionMessage camelCase 序列化）。
/// - 缺参（agent_type / session_id）或空串 → 400 BAD_REQUEST；
/// - 读取失败（会话不存在 / 存储不可读 / 未知工具）→ 404 NOT_FOUND，错误细节只进
///   日志不外泄（不向外部暴露内部路径/存储布局）；
/// - `before` 游标不做（Task 7 裁决）：M3 不做向上翻页，更早内容由前端以更大 limit 重拉；
/// - 文件/SQLite IO 是重活，`spawn_blocking` 包读取（sessions handler 同一先例），
///   不堵 tokio worker。
pub async fn session_messages(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, StatusCode> {
    let Some(agent) = params
        .get("agent_type")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    // message_source 不可 clone（Box<dyn Fn>）：整体 move 进阻塞线程池调用（与
    // sessions handler 捕获 session_source 的写法一致）；agent/sid 移动副本进闭包，
    // 原值保留给失败分支的日志
    let st = st.clone();
    let agent_ref = agent.clone();
    let result = tokio::task::spawn_blocking(move || {
        (st.message_source)(agent_ref.as_str(), sid.as_str(), limit)
    })
    .await
    .map_err(|e| {
        log::error!("会话内容读取任务异常: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match result {
        Ok(msgs) => Ok((
            // 门禁下的私有会话正文，禁止中间层缓存（sessions/host 同规）
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "messages": msgs })),
        )
            .into_response()),
        Err(e) => {
            log::warn!("session-messages 读取失败（agent={agent}）: {e}");
            Err(StatusCode::NOT_FOUND)
        }
    }
}
