// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ host（P8a/P8b 页头数据）
// + pair + events（M3 Task 6 SSE 实时通道）+ session-messages（Task 7）
// + session-files / file（M3 Task 8 文件路径提取与安全读取）

use axum::{
    extract::{ConnectInfo, Query, State},
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

// 审批状态机产出枚举（M4 T2 三 handler 的 match 对象）
use super::approval::{ConfirmOutcome, CreateRejection, PollOutcome};
use super::server::{CleanupStream, RemoteState};

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
///   receiver_count 归零 → watcher 零订阅者守卫跳过扫描（Task 5 闭环）；
/// - 吊销/停止即时断连（M4 T0a）：连接注册进 sse_registry，信号经 take_until 终止流
pub async fn events(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
) -> Sse<impl Stream<Item = Result<sse::Event, Infallible>>> {
    // 先订阅、后取快照（顺序刻意，勿换）：subscribe 在快照计算之前，期间产生的跃迁
    // 落进 broadcast 缓冲（容量 64）并在快照帧之后依次送出；若反序，快照与订阅之间
    // 发生的跃迁会永久丢失（重连后的看板状态与真实脱节）
    let rx = st.watcher_tx.subscribe();
    // M4 T0a：注册连接（gate 已过闸，此处必能取到设备 id；防御性 None 时跳过注册只服务）
    let device = super::gate::extract_device(&headers).unwrap_or_default();
    let (conn_id, close_rx) = st.sse_registry.register(&device);
    // 注册表句柄先行克隆：st 整体 move 进下方 spawn_blocking 闭包（既有代码不动）
    let registry = st.sse_registry.clone();
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
    let base = initial.chain(transitions);
    // 吊销/停止信号到达即终止流（take_until：close_rx 被 send 或 Sender drop 均触发）
    let guarded = base.take_until(close_rx);
    let cleaned = CleanupStream {
        inner: guarded,
        reg: registry,
        device,
        conn_id,
    };
    Sse::new(cleaned).keep_alive(sse::KeepAlive::default())
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
    // M4 T2c：上限三入口统一门——直通扫码也拒（spec：否则扫码绕过上限）。
    // max 经注入缝取（生产读 KV；测试注入常量——不直读全局 DAO）。
    // 门在 accept **之前**：token 不被消费，腾位后原码仍可用。
    // 注意本 handler 返回 `Result<Response, StatusCode>`——带 body 的 403 须包 `Ok(...)`
    let max = (st.max_devices_source)();
    if st
        .store
        .with(|c| crate::remote::pairing::device_count(c) >= max)
    {
        super::events::audit("pair_rejected_cap", &format!("max={max}"));
        return Ok((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "cap_full" })),
        )
            .into_response());
    }
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
        // M4 T2：直通路径落设备名（花名册展示——否则 roster 只剩 id 前 8 位可读）
        name: "直通扫码".to_string(),
        ua,
        origin_ip: String::new(),
        // via 运行时判定（回环+Host 快照）属 Task A3——接入前空串占位（列 DEFAULT 同值）
        via: String::new(),
        paired_at: now,
    };
    // 持久化失败不阻断本次配对（下次 gate 校验会失败）——保持简报行为；
    // 成功则 cookie 必须下发**生效 id**（M5 A1 upsert：同指纹命中会沿用旧行 id，
    // 若仍下发 accept() 新生成 id，该 cookie 指向不存在的行，重连浏览器永久 403）
    let effective_id = st.store.with(|c| {
        crate::remote::pairing::persist_device(c, &dev).unwrap_or_else(|_| device_id.clone())
    });
    // HttpOnly + SameSite=Lax + Path=/m + 180d（dsh 七不变量之 cookie 语义）——
    // 拼装收口到 pairing::device_cookie（与 pair-poll / pair-confirm 三处共用，防漂移）
    Ok((
        [(
            axum::http::header::SET_COOKIE,
            crate::remote::pairing::device_cookie(&effective_id),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response())
}

// ============================================================
// 审批配对三端点（M4 T2）：/pair/request | /pair/poll | /pair/confirm
// 均不过闸（gate 放行 /pair/*）——凭据 = 桌面批准或 4 位码，成功后换设备 cookie
// ============================================================

#[derive(Deserialize)]
pub struct PairRequestBody {
    pub name: Option<String>,
}

/// POST /m/api/v1/pair/request（M4 T2a）：新设备请求接入（未过闸端点）。
/// 成功即桌面通知（emit remote-pair-request）+ 审计留痕
pub async fn pair_request(
    State(st): State<Arc<RemoteState>>,
    ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PairRequestBody>,
) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ip = addr.ip().to_string();
    let name = req.name.unwrap_or_default();
    let mut svc = st.approval.lock().unwrap();
    match svc.create(&name, &ua, &ip, now) {
        Ok(r) => {
            super::events::emit_ui(
                "remote-pair-request",
                serde_json::json!({
                    "name": r.name, "ip": r.ip, "expiresAt": r.expires_at,
                }),
            );
            super::events::audit(
                "pair_request",
                &format!("id={} name={} ip={}", r.id, r.name, r.ip),
            );
            Json(serde_json::json!({ "requestId": r.id, "expiresAt": r.expires_at }))
                .into_response()
        }
        Err(e) => {
            let err = match e {
                CreateRejection::QueueFull => "queue_full",
                CreateRejection::IpBusy => "ip_busy",
            };
            super::events::audit("pair_request_rejected", &format!("ip={ip} reason={err}"));
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({ "error": err })),
            )
                .into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairPollBody {
    pub request_id: String,
}

/// POST /m/api/v1/pair/poll：手机轮询审批结果（approved 幂等重发 Set-Cookie——丢包重 poll 不掉凭证）
pub async fn pair_poll(
    State(st): State<Arc<RemoteState>>,
    Json(req): Json<PairPollBody>,
) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let outcome = st.approval.lock().unwrap().poll(&req.request_id, now);
    match outcome {
        PollOutcome::Approved { device, name } => {
            // E2E S7 实测缺陷修复：approved 落库前复核设备上限——批准后满员、
            // 旧 requestId 幂等重 poll 会经本路径绕过「三入口同门」把第 4 台
            // 设备落库（Task 7 评审 Minor TOCTOU 被端到端坐实）。满员时维持
            // pending 观感（腾位后下次 poll 自动补上凭证）。
            // 注意取值顺序：max 必须在 store.with 之外求值——with 持 DB 锁不可重入，
            // 闭包内再走 get_setting 会自死锁（E2E 实测进程冻结的根因）
            let max = (st.max_devices_source)();
            let cap = st
                .store
                .with(|c| crate::remote::pairing::device_count(c) >= max);
            if cap {
                super::events::audit("pair_poll_deferred_cap", &format!("device={device}"));
                return Json(serde_json::json!({ "status": "pending" })).into_response();
            }
            super::events::audit("pair_polled", &format!("device={device}"));
            persist_and_cookie(&st, &device, &name, now)
        }
        PollOutcome::Pending { expires_at } => {
            Json(serde_json::json!({ "status": "pending", "expiresAt": expires_at }))
                .into_response()
        }
        PollOutcome::Expired => Json(serde_json::json!({ "status": "expired" })).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairConfirmBody {
    pub request_id: String,
    pub code: String,
}

/// POST /m/api/v1/pair/confirm：4 位确认码等效授权（限试 3 次，spec T2b）
pub async fn pair_confirm(
    State(st): State<Arc<RemoteState>>,
    Json(req): Json<PairConfirmBody>,
) -> Response {
    let now = chrono::Utc::now().timestamp_millis();
    let max = (st.max_devices_source)();
    let outcome = st
        .approval
        .lock()
        .unwrap()
        .confirm(&req.request_id, &req.code, now);
    // Ok 路径补上限门（confirm 内部不触 DB——状态机纯内存；满员时码对了也拒）
    match outcome {
        ConfirmOutcome::Ok { device, name } => {
            let cap = st
                .store
                .with(|c| crate::remote::pairing::device_count(c) >= max);
            if cap {
                return Json(serde_json::json!({ "ok": false, "error": "cap_full" }))
                    .into_response();
            }
            super::events::audit("pair_confirmed", &format!("device={device}"));
            persist_and_cookie(&st, &device, &name, now)
        }
        ConfirmOutcome::Wrong(left) => {
            // 敌意试码留痕（spec T2e）：前两次失败也要有审计，否则 exhausted 才是首条记录
            super::events::audit(
                "pair_confirm_wrong",
                &format!("id={} tries_left={left}", req.request_id),
            );
            Json(serde_json::json!({ "ok": false, "error": "wrong", "triesLeft": left }))
                .into_response()
        }
        ConfirmOutcome::Exhausted => {
            super::events::audit("pair_confirm_exhausted", &req.request_id);
            Json(serde_json::json!({ "ok": false, "error": "exhausted" })).into_response()
        }
        ConfirmOutcome::Expired | ConfirmOutcome::NotFound => {
            Json(serde_json::json!({ "ok": false, "error": "expired" })).into_response()
        }
    }
}

/// 批准/确认通过的公共落库 + 下发 cookie（设备名来自请求——花名册展示用；
/// api::pair 直通路径传「直通扫码」）
fn persist_and_cookie(st: &Arc<RemoteState>, device_id: &str, name: &str, now: i64) -> Response {
    let dev = crate::remote::pairing::NewDevice {
        id: device_id.to_string(),
        name: name.to_string(),
        ua: String::new(),
        origin_ip: String::new(),
        // via 运行时判定（回环+Host 快照）属 Task A3——接入前空串占位（列 DEFAULT 同值）
        via: String::new(),
        paired_at: now,
    };
    // cookie 下发生效 id（M5 A1 upsert 语义，理由同 api::pair）；
    // 持久化失败不阻断配对（下次 gate 校验会失败）——回退请求侧生成 id
    let effective_id = st.store.with(|c| {
        crate::remote::pairing::persist_device(c, &dev).unwrap_or_else(|_| device_id.to_string())
    });
    (
        [(
            axum::http::header::SET_COOKIE,
            crate::remote::pairing::device_cookie(&effective_id),
        )],
        Json(serde_json::json!({ "ok": true, "status": "approved" })),
    )
        .into_response()
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

/// GET /m/api/v1/session-files?agent_type=&session_id=（M3 Task 8）
/// GET /m/api/v1/session-files?agent_type=&session_id=&limit=（M3 Task 8 / M3+ 富化）
/// 该会话工具调用涉及的文件表：`{files: [{path, lastSeq, lastTs, hits}], truncated}`
/// （结构化条目 + 出现序排序，泛化提取见 files::extract_file_paths）。移动端
/// 详情页一份数据两用：正文路径链接化（取 path 集）+ 文件面板列表（全字段）。
/// - 缺参（agent_type / session_id）或空串 → 400 BAD_REQUEST；
/// - limit 缺省 200（clamp [1,1000] 由 read_session_messages_impl 内部完成，直接透传）；
/// - truncated = 该档位窗口下头部被截断（还有更早文件），面板据此提示；
/// - 提取失败 / 会话不存在 → 200 空表 `{files: [], truncated: false}`——文件面板
///   是增强能力，失败不阻塞详情页，也无从区分「无文件」与「读不到」（不给探测面）；
/// - 提取要读一遍会话消息流（文件/SQLite IO），`spawn_blocking` 包裹
///   （sessions handler 同一先例），不堵 tokio worker。
pub async fn session_files(
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
    // 追溯档位（M3+ 用户裁决 3）：缺省 200，与详情页默认 limit 同标尺
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    // path_source 不可 clone（Box<dyn Fn>）：整体 move 进阻塞线程池调用（与
    // session_messages handler 的写法一致）
    let st = st.clone();
    let (files, truncated) =
        tokio::task::spawn_blocking(move || (st.path_source)(agent.as_str(), sid.as_str(), limit))
            .await
            .map_err(|e| {
                log::error!("文件路径提取任务异常: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    Ok((
        // 门禁下的私有数据（会话涉及的文件路径），禁止中间层缓存（sessions 同规）
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "files": files, "truncated": truncated })),
    )
        .into_response())
}

/// file 端点的读取结果三分（403/404 语义分流在 handler 尾部，读取全程在阻塞线程池）：
/// - Found：cwd 内常规文件（字节 + mime）；
/// - NoSession：session_id 不在会话快照中 → 404；
/// - Rejected：越界 / 不存在 / 过大 / 目录 / cwd 缺失 → 一律 403（错误细节只进
///   日志——越界与不存在不可区分，不给外部探测面预言机，进度台账 #12 口径）。
enum FileReadOutcome {
    Found(Vec<u8>, String),
    NoSession,
    Rejected(String),
}

/// GET /m/api/v1/file?session_id=&path=（M3 Task 8）
/// 会话项目目录或用户主目录内的安全文件读取（read_file_safe：限 cwd∪home、
/// 只读、双阈值 500KB/5MB；主目录内敏感目录拒绝——2026-09-16 用户裁决放宽
/// 到主目录，kimi 等工具常引用 ~/Downloads 的图片）。
/// - 缺参 / 空白 → 400；session_id 不在会话快照 → 404；
/// - 图片 mime → 二进制响应 + Content-Type；文本 → JSON `{content, mime, size}`；
/// - 拒绝（越界/不存在/超限/敏感目录彼此不可区分）→ 403 + log::warn；
/// - cwd 查找与文件读取同在一个 `spawn_blocking` 里：会话快照源是同步阻塞调用
///   （sysinfo 全进程刷新，实机教训见 commands/session.rs），文件 IO 同为重活。
pub async fn read_file(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, StatusCode> {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let Some(path) = params
        .get("path")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let st = st.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        // 数据同源铁律 3：cwd 直调注入的会话源（生产 = adapter::get_all_sessions，
        // 与看板同一份快照），不另立查找函数
        let resp = (st.session_source)();
        let Some(session) = resp.sessions.into_iter().find(|s| s.id == sid) else {
            return FileReadOutcome::NoSession;
        };
        // home 读真实主目录（放宽边界：cwd ∪ home，敏感目录仍拒）；
        // 取不到 home（极端环境）则按 None 走 fail-closed 的 cwd-only 边界
        let home = dirs::home_dir();
        match crate::remote::files::read_file_safe(
            &session.project_path,
            &path,
            home.as_deref().and_then(|h| h.to_str()),
        ) {
            Ok((bytes, mime)) => FileReadOutcome::Found(bytes, mime),
            Err(e) => FileReadOutcome::Rejected(e),
        }
    })
    .await
    .map_err(|e| {
        log::error!("文件读取任务异常: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    match outcome {
        FileReadOutcome::Found(bytes, mime) if mime.starts_with("image/") => Ok((
            // 门禁下的私有文件内容，禁止中间层缓存（sessions 同规）
            [
                (axum::http::header::CONTENT_TYPE, mime),
                (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
                // 终审 Important 2：图片二进制直出必须带嗅探防护双头。SVG 以顶层
                // 文档加载时可执行内嵌脚本（同源脚本可 fetch 会话数据，cookie
                // SameSite=Lax 不防同源攻击）——CSP 断脚本与一切子资源 +
                // nosniff 防 MIME 嗅探把图片内容误判为可执行文档
                (
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    "default-src 'none'".to_string(),
                ),
                (
                    axum::http::header::X_CONTENT_TYPE_OPTIONS,
                    "nosniff".to_string(),
                ),
            ],
            bytes,
        )
            .into_response()),
        FileReadOutcome::Found(bytes, mime) => {
            let text = String::from_utf8_lossy(&bytes);
            Ok((
                [(axum::http::header::CACHE_CONTROL, "no-store".to_string())],
                Json(serde_json::json!({
                    "content": text,
                    "mime": mime,
                    "size": bytes.len(),
                })),
            )
                .into_response())
        }
        FileReadOutcome::NoSession => Err(StatusCode::NOT_FOUND),
        FileReadOutcome::Rejected(e) => {
            // 探测面最小化（进度台账 #12）：越界 / 不存在 / 超限对外一律同 403 空体，
            // 差异只进服务端日志
            log::warn!("file 读取被拒: {e}");
            Err(StatusCode::FORBIDDEN)
        }
    }
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
        Ok(pg) => Ok((
            // 门禁下的私有会话正文，禁止中间层缓存（sessions/host 同规）
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            // Bug 1（M3 验收）：truncated = 头部截断标记，移动端据此提供
            // 「加载更早消息」（放大 limit 重拉时后端字节窗同放大）
            Json(serde_json::json!({
                "messages": pg.messages,
                "truncated": pg.truncated,
            })),
        )
            .into_response()),
        Err(e) => {
            log::warn!("session-messages 读取失败（agent={agent}）: {e}");
            Err(StatusCode::NOT_FOUND)
        }
    }
}
