// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ host（P8a/P8b 页头数据）
// + pair/pin（M5 A3 访问密码配对，唯一换 cookie 入口）+ events（M3 Task 6 SSE 实时通道）
// + session-messages（Task 7）+ session-files / file（M3 Task 8 文件路径提取与安全读取）
// + session-send / send-info / queue 系（M7 Task 6 注入三端点）
// + session-approve-options / session-approve（M8 Task 11 审批选项与一键应答）
// + session-open（M6R–M9R Task 11 一键 resume，R5）

use axum::body::Bytes;
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

use super::server::{CleanupStream, RemoteState};

/// 看板隐藏过滤（APP 软归档=「叉」语义，2026-09-20 体验批二修订）：hidden →
/// 一律从响应剔除（仅手机看板；桌面走 invoke 不经此端点）。**无自动回归**——
/// 初版「非绿∨未读即回归」判据把 W4 持久未读（转绿插行、24h 过期、已读才删）
/// 误当活动信号，导致带未读的会话归档后 3s 内必被拉回（实测废弃）。叉掉 =
/// 不再跟踪管理，恢复只有历史页「移回看板」一条路。counts 按过滤后重算。
/// GET /sessions 与 SSE snapshot 帧共用本函数——两个入口口径一致，隐藏卡片
/// 不会经另一通道诈尸。
fn apply_board_hidden(
    st: &Arc<RemoteState>,
    mut resp: crate::session::SessionsResponse,
) -> crate::session::SessionsResponse {
    let hidden: std::collections::HashSet<String> = (st.board_hidden_ids)().into_iter().collect();
    if hidden.is_empty() {
        return resp;
    }
    resp.sessions.retain(|s| !hidden.contains(&s.id));
    resp.total_count = resp.sessions.len();
    resp.waiting_count = resp
        .sessions
        .iter()
        .filter(|s| matches!(s.status, crate::session::SessionStatus::Waiting))
        .count();
    resp
}

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
    let response =
        tokio::task::spawn_blocking(move || apply_board_hidden(&st, (st.session_source)()))
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
    // handler 同一先例（实机教训见 commands/session.rs 的 get_all_sessions 注释）。
    // 同过 apply_board_hidden 过滤（与 GET /sessions 同函数）：软归档隐藏卡片
    // 不得经 SSE snapshot 通道诈尸
    let snapshot =
        tokio::task::spawn_blocking(move || apply_board_hidden(&st, (st.session_source)()))
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

/// POST /pair/pin 请求体（M5 A3）：PIN 必填，设备名可选（缺省/空回落默认名）
#[derive(Deserialize)]
pub struct PairPinBody {
    pub pin: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// POST /m/api/v1/pair/pin（M5 A3）：访问密码换设备 cookie——密码制唯一配对入口。
/// 处理序（计划 §3.1）：① 限速 check（锁内查改）→ ② PIN 源读取 → ③ 校验 →
/// ④ record_success + 设备上限门（PIN 正确**之后**判定——先验 PIN 再谈名额，不向前者
/// 泄露名额信息）→ ⑤ upsert + cookie 下发 → ⑥ 200。
/// 状态码契约（响应体 camelCase）：
/// - 锁定期内（即使 PIN 正确）→ 429 `{"retryAfter": 秒}`；
/// - PIN 未设置（KV 空）→ 401 `{"error":"pin_not_set"}`（不计失败——无密可对；
///   A5 会在开通道时自动生成，本任务只留语义）；
/// - PIN 错 / 格式非法 → 记失败 + 401 `{"error":"invalid_pin","remaining": n}`
///   （n = 5 - 已错次数；移动端展示「还可尝试 N 次」是计划内预言机披露）；
/// - 正确 → 200 `{"ok":true}` + Set-Cookie（upsert 生效 id，属性同既有 cookie 契约）。
pub async fn pair_pin(
    State(st): State<Arc<RemoteState>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(req): Json<PairPinBody>,
) -> Response {
    use crate::remote::pin::RateDecision;
    let ip = addr.ip().to_string();
    // 限速时钟走注入缝（测试可推进）；设备时间戳走真实时钟（gate 滑动 TTL 域，见 persist）
    let now = (st.now_source)();
    // ① 限速过闸：锁定期内即使 PIN 正确也拒（429），不泄露任何 PIN 有效性信息。
    // decision 先绑定出锁（评审 Minor 2）：MutexGuard 临时值随 let 语句结束即释放，
    // audit 与响应构建不持限速器锁（对齐"audit 不持锁"风格）
    let decision = st.pin_limiter.lock().unwrap().check(&ip, now);
    if let RateDecision::Locked { retry_after_secs } = decision {
        super::events::audit(
            "pair_pin_locked",
            &format!("ip={ip} retry_after={retry_after_secs}s"),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "retryAfter": retry_after_secs })),
        )
            .into_response();
    }
    // ② PIN 源读取：未设置 → 401 pin_not_set，不计失败
    let Some(expected) = (st.pin_source)() else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "pin_not_set" })),
        )
            .into_response();
    };
    // ③ 校验：格式非法与比对失败同语义（同记失败——不给"格式预言机"）。
    // 比对两侧都 trim（评审 Minor 7）：与 validate_pin 的 trim 对称，防将来 KV 存入
    // 带空白值（set_pin 不再经本端点校验路径时）造成恒不等
    if !crate::remote::pin::validate_pin(&req.pin) || req.pin.trim() != expected.trim() {
        // 记失败与读剩余次数在同一锁临界区（限速器锁内查改，契约如此）
        let remaining = {
            let mut limiter = st.pin_limiter.lock().unwrap();
            limiter.record_failure(&ip, now);
            limiter.remaining_attempts(&ip)
        };
        super::events::audit("pair_pin_wrong", &format!("ip={ip} remaining={remaining}"));
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid_pin", "remaining": remaining })),
        )
            .into_response();
    }
    // ④ PIN 正确：清零该 IP 计数 → 设备上限门（沿用既有直通上限语义：403 + cap_full）
    st.pin_limiter.lock().unwrap().record_success(&ip);
    let max = (st.max_devices_source)();
    let cap_full = st
        .store
        .with(|c| crate::remote::pairing::device_count(c) >= max);
    if cap_full {
        super::events::audit("pair_pin_rejected_cap", &format!("max={max}"));
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "cap_full" })),
        )
            .into_response();
    }
    // via 判定（配对时刻），两级：
    // ① PID 实锤优先（2026-09-18）：来连套接字归属 MAM 账本的 cloudflared 通道
    //    → 直接定 quick/named（不依赖域名名单，编外/未知归属走②）；
    // ② Host 判定：Host + 来源 IP 对照隧道快照域名；**None 哨兵**（名单不可信：
    //    错误终态 / 运行中而域名缺失）→ 保守标 lan，绝不判本机。
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let via = match super::conn_owner::tunnel_channel_for_conn(addr.port()) {
        Some(kind) => kind,
        None => match (st.via_hosts_source)() {
            None => "lan",
            Some((quick_hosts, named_hosts)) => {
                super::gate::classify_via(addr.ip(), host, &quick_hosts, &named_hosts)
            }
        },
    };
    // 设备名：自报 trim 收敛 40 字符（与 rename_device / 审批自报名同口径），空回落默认名
    let mut name: String = req
        .name
        .unwrap_or_default()
        .trim()
        .chars()
        .take(40)
        .collect();
    if name.is_empty() {
        name = "新设备".to_string();
    }
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let device_id = super::random_hex_16();
    let paired_at = chrono::Utc::now().timestamp_millis();
    super::events::audit("pair_pin_ok", &format!("via={via} ip={ip}"));
    persist_and_cookie(&st, &device_id, &name, &ua, &ip, via, paired_at)
}

/// 配对通过的公共落库 + 下发 cookie（M5 A3 起为 /pair/pin 专用；真实 ua/ip 入指纹，
/// via 为配对时刻的分类结果）。
/// cookie 下发生效 id（M5 A1 upsert 语义：命中旧行时必须下发旧行 id——若下发新生成 id，
/// 该 cookie 指向不存在的行，重连浏览器永久 403）；
/// 持久化失败不阻断配对（下次 gate 校验会失败）——回退请求侧生成 id
fn persist_and_cookie(
    st: &Arc<RemoteState>,
    device_id: &str,
    name: &str,
    ua: &str,
    origin_ip: &str,
    via: &str,
    now: i64,
) -> Response {
    let dev = crate::remote::pairing::NewDevice {
        id: device_id.to_string(),
        name: name.to_string(),
        ua: ua.to_string(),
        origin_ip: origin_ip.to_string(),
        via: via.to_string(),
        paired_at: now,
    };
    let effective_id = st.store.with(|c| {
        crate::remote::pairing::persist_device(c, &dev).unwrap_or_else(|_| device_id.to_string())
    });
    (
        [(
            axum::http::header::SET_COOKIE,
            crate::remote::pairing::device_cookie(&effective_id),
        )],
        Json(serde_json::json!({ "ok": true })),
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
/// - Rejected：敏感目录 / 不存在 / 过大 / 目录 / 读取失败 → 一律 403，**响应体
///   带结构化原因**（M5 P2-a 用户裁决：原因写在报错处便于排障。原「空体防探测」
///   随边界全盘放开退役——原因仅暴露给已过闸设备，PIN + cookie 已是门槛）。
enum FileReadOutcome {
    Found(Vec<u8>, String),
    NoSession,
    Rejected(crate::remote::files::FileRejectReason),
}

/// GET /m/api/v1/file?session_id=&path=（M3 Task 8；M5 P2-a 全盘语义）
/// 安全文件只读读取（read_file_safe：任意路径、敏感目录黑名单、只读、
/// 双阈值 500KB/5MB——2026-09-18 用户裁决放开项目外文件预览）。
/// - 缺参 / 空白 → 400；session_id 不在会话快照 → 404；
/// - 图片 mime → 二进制响应 + Content-Type；文本 → JSON `{content, mime, size}`；
/// - 拒绝（敏感目录/不存在/超限/目录/IO）→ 403 + `{"error":"<reason>"}`（snake_case
///   原因码：sensitive/too_large/not_found/not_file/io，前端据此分診文案）+ log::warn；
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
        // 敏感黑名单基准：经注入缝取真实 home（M5 P2-a 追记——直接传 None 曾使
        // 主目录内黑名单整段失效，见 read_file_safe 文档）
        let home = (st.home_source)();
        match crate::remote::files::read_file_safe(&session.project_path, &path, home.as_deref()) {
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
        FileReadOutcome::Rejected(reason) => {
            // 原因细分（M5 P2-a 用户裁决「把看不了的原因写在报错的地方」）：
            // 403 + snake_case 原因码，仅已过闸设备可见；日志保留中文明细
            log::warn!("file 读取被拒: {}（{reason:?}）", reason.message());
            Ok((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({ "error": reason })),
            )
                .into_response())
        }
    }
}

/// 「按 (agent, session) 读消息」聚合内核（M9R Task 5 抽取）：session-messages
/// 端点与注入确认层（`inject::confirm::session_stamp_hit`）——**数据同源（同一
/// content 读路径）**：同走 `RemoteState.message_source` 缝；生产 confirm_probe
/// 缝直调 `content::read_session_messages`（八工具统一出口，不经本函数），core
/// 函数为端点侧封装（测试假源对端点矩阵与确认语义同时生效）。
pub(crate) fn read_session_messages_core(
    st: &RemoteState,
    agent: &str,
    sid: &str,
    limit: usize,
) -> Result<crate::remote::content::MessagesPage, String> {
    (st.message_source)(agent, sid, limit)
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
        // M9R Task 5：改调聚合内核 read_session_messages_core（端点与确认层共用，
        // 行为不变——内核即原 message_source 直调）
        read_session_messages_core(&st, agent_ref.as_str(), sid.as_str(), limit)
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

// ==== M7 Task 6：注入三端点（session-send / send-info / queue 系）====
// 契约（JSON camelCase；所有 Json 响应带 Cache-Control: no-store——门禁下私有写路径）：
//   POST /session-send          → delivered | queued{itemId,position} | failed{error} | 400 | 404 | 403
//   GET  /session-send-info     → {injectable, reasonCode?, reason?, channels, visibility}
//   GET  /session-queue         → {items:[{id,content,enqueuedAt,position}]}
//   POST /session-queue/jump    → delivered | queued{itemId,position} | failed{error} | 400 | 404 | 403
//   POST /session-queue/retract → {ok:true} | failed{error}（守卫忙让位） | 400 | 404 | 403
// 零污染：DB 依赖全部经 RemoteState.store（测试 = 内存库）；会话快照走注入源
// （数据同源铁律）；注入器走 st.injector 缝（端点测试用 FakeInjector）。

/// 单条发送正文上限（chars 计）：移动端输入框软上限的服务端兜底
pub const MAX_SEND_CHARS: usize = 10_000;

/// POST /m/api/v1/session-send 请求体（camelCase；字段全 default——缺参不触发
/// axum 提取器 422，由 handler 统一按契约给 400 bad_request）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSendReq {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub text: String,
    /// 只入队标志（D6 修改重发）：语义为——**queueOnly 仅影响入队决策**（true 时
    /// 跳过下方直发尝试，即使快照显示可输入也强制入队，防「文件说闲、TUI 实忙」
    /// 窗口的变相插队）；**一旦入队，flush 循环对 queueOnly 项与普通队列项完全
    /// 同权**（转闲按序自动放行，不做任何区别对待）；**队列存储不携带该标志**
    /// （无需持久化——它是本次请求的分派意图，不是条目属性）。缺省/false = 普通发送
    /// （可输入态照旧直发），既有调用面请求体形态不变。
    #[serde(default)]
    pub queue_only: Option<bool>,
}

/// POST /session-queue/jump 与 /retract 请求体（camelCase；item_id 走 Option——
/// 缺参/非法由 handler 统一 400，不让 serde 提取器抢答）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemReq {
    #[serde(default)]
    pub session_id: String,
    pub item_id: Option<i64>,
}

/// 状态码 + JSON 载荷的统一 no-store 响应（本文件各端点带状态码的 Json 回执共用
/// 样板——门禁下私有读写路径一律 `Cache-Control: no-store`，见文件头 M7 契约注释）
fn json_no_store(status: StatusCode, body: serde_json::Value) -> Response {
    (
        status,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(body),
    )
        .into_response()
}

/// 400 bad_request（缺参 / 空 text / 超长）
fn bad_request() -> Response {
    json_no_store(
        StatusCode::BAD_REQUEST,
        serde_json::json!({ "error": "bad_request" }),
    )
}

/// 403 防御（gate 已拦设备，理论不可达——handler 直取 cookie 失败时兜底）
fn forbidden_defense() -> Response {
    json_no_store(
        StatusCode::FORBIDDEN,
        serde_json::json!({ "error": "forbidden" }),
    )
}

/// 设备侧公共前置：cookie 提取（防御 403 的判定源）+ 花名查询（查无回落 unknown）。
/// 返回 `None` = 无有效 cookie（调用方统一给 403 防御响应；不在此构造 Response，
/// 规避 clippy::result_large_err 的大 Err 变体）
fn device_identity(st: &RemoteState, headers: &axum::http::HeaderMap) -> Option<(String, String)> {
    let device_id = super::gate::extract_device(headers)?;
    let device_name = st
        .store
        .with(|c| super::pairing::device_name(c, &device_id));
    Some((device_id, device_name))
}

/// 端点侧审计（send / queue / retract 共用）：统一走 inject::audit_write
/// （DB 落库 + events::audit 日志并行，M4 T2e 惯例）
#[allow(clippy::too_many_arguments)]
fn endpoint_audit(
    st: &Arc<RemoteState>,
    device_id: &str,
    device_name: &str,
    agent_type: &str,
    session_id: &str,
    content: &str,
    action: &str,
    result: &str,
) {
    st.store.with(|c| {
        crate::inject::audit_write(
            c,
            st,
            device_id,
            device_name,
            agent_type,
            session_id,
            content,
            action,
            result,
        )
    });
}

/// 快照中按 session_id 找会话（spawn_blocking 内调用：session_source 是同步阻塞扫描）。
/// 数据同源铁律：直调注入源（与看板/内容读取同一份快照），不另立查找函数
fn find_session_sync(st: &Arc<RemoteState>, session_id: &str) -> Option<crate::session::Session> {
    (st.session_source)()
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)
}

/// 端点审计 action 选择（丁T3 裁2，**单点**）：`/` 开头消息记 `slash`，其余按投递
/// 路径记 `send` / `queue`。
///
/// # 为什么在端点层选（而不是在 queue::settle 也改）
///
/// 一次 `session-send` 最多落**两类**审计行，职责不同（既有架构，见 `inject::queue`
/// 的 settle 注释）：
/// - **端点行**（本函数消费）：用户动作记录——「谁（设备）用什么动作对哪个会话做了
///   什么」；每条成功入队的请求**恰好一条**（send/queue/slash），是审计页按设备检索
///   的入口；
/// - **落账行**（`settle` 写 flush/jump/fail）：投递机制记录——这一条是「直发还是
///   排队放行」「投递成功还是失败」。
///
/// 裁2 要求的「溯源走审计页（action=slash + 设备名）」落在**用户动作**这一层：把
/// 机制行的 flush/jump 也改写成 slash 会让「排队放行 / 插队 / 失败」这些运维判据
/// 消失（同一会话的 slash 与普通消息将无法在账上区分投递路径）。故本函数只改端点行
/// 的 action，机制行保持既有词表——两行都在账上，`summary` 里是同一段 `/plan` 文本
/// （裸注入，无签名，原样可读）。
///
/// 判据与注入形态同源：`normalize::is_slash_message` 同时决定「是否裸注入」——
/// compose 与审计不可能脱节（同一函数两处消费）。
fn audit_action_for(text: &str, base: &'static str) -> &'static str {
    if crate::inject::normalize::is_slash_message(text) {
        "slash"
    } else {
        base
    }
}

/// POST /m/api/v1/session-send（W4 直发/入队分派）：
/// - 缺参 / 空 text / 超 MAX_SEND_CHARS → 400 bad_request；
/// - 设备 cookie 缺失 → 403 防御（gate 已拦，理论不可达）；
/// - 会话不在快照 → 404 no_session；路由判不可注入 → 403 not_injectable（带
///   reasonCode/reason——W1 定位失败语义的前置闸，不入队）；
/// - 可输入态（is_input_ready）→ 入队后即刻 flush_one 直发（jump=false），五态精确
///   映射（P1-4 + D7/T3）：Sent → 200 delivered；Submitted → 200 submitted（已投递
///   未确认中性回执——注入 Ok + 戳未中 + 屏读无滞留草稿 = 消息已被 TUI 收进内部
///   队列，不冒充 delivered 也不误报失败诱导重试，验收问题 #5）；Failed(e) → 200
///   failed{error}（注入失败 / 滞留补回车失败 / 补回车后仍未落盘的真失败回执可重试，
///   W1：失败行已 mark_failed 退出 pending，队列无残留）；
///   Deferred | Suspended → 200 queued（行保持 pending 等会话回来/下个跃迁，语义即
///   排队——Suspended 亦 queued，红·中断挂起不谎报 delivered 也不误报失败）。直发
///   审计按态落 send|queue（flush_one 落账时已并行写 flush 审计——本端点按契约另写
///   send 终态；Submitted 的 send 审计 result=unconfirmed，与确认送达 ok 区分）；
/// - 运行中（is_running 等）→ 留队（黄灯），审计 action=queue，回执 queued+position。
///
/// queueOnly 请求标志（D6 修改重发，语义详见 SessionSendReq::queue_only）：true 时
/// 跳过直发尝试（上述可输入态直发分支整体不触达）强制落留队臂——审计 action=queue、
/// 回执 queued{itemId,position} 与「运行中留队」完全同路径；入队后 flush 循环对该项
/// 与普通队列项同权（转闲按序自动放行），队列存储不携带该标志。缺省/false 行为不变。
///
/// 直发也走队列（先入队再 flush 队首）：与既有 pending 项保持 FIFO 串行
/// （W2 单会话不变量），成功/失败行都退出 pending，队列无残留。
pub async fn session_send(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionSendReq>,
) -> Response {
    // ① 参数校验（trim 判空，原文上送——归一在入队时统一做）
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() || req.text.trim().is_empty() || req.text.chars().count() > MAX_SEND_CHARS {
        return bad_request();
    }
    // ② 设备身份（防御 403 + 花名）
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // ③ 会话查找（spawn_blocking：扫描是重活）+ 路由判定
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let session =
        match tokio::task::spawn_blocking(move || find_session_sync(&probe_st, &probe_sid)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("session-send 会话扫描任务异常: {e}");
                return json_no_store(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                );
            }
        };
    let Some(session) = session else {
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "no_session" }),
        );
    };
    // ④ 路由判定（W3 纯核；platform = 本机 OS）。不可注入 → 403（带原因），不入队
    let tool = session.agent_type.tool_id().to_string();
    if let crate::inject::routing::RouteOutcome::NotInjectable {
        reason_code,
        reason,
    } = crate::inject::routing::route(&tool, session.form, session.pid, std::env::consts::OS)
    {
        return json_no_store(
            StatusCode::FORBIDDEN,
            serde_json::json!({
                "error": "not_injectable",
                "reason": reason,
                "reasonCode": reason_code,
            }),
        );
    }
    // ⑤ 组装（裁决 6 归一在入队时一次完成）+ 入队（FIFO 保序）。
    //
    // 丁T3 裁2：compose_injection 内部**按内容分流**——普通消息 = `{正文} [mobile 设备名]`
    // （签名**后置**），`/` 开头消息 = **裸注入无签名**（前后缀都会破坏命令与参数，问题 8
    // 实锤）。分流为什么放在 compose 内：它是注入文本的唯一组装出口，队列存的就是它的
    // 产物（flush 层无需再判一次，也就不会有「入队形态与投递形态不一致」的漂移面）。
    // 审计 action 由同一判据（`is_slash_message`）给出 slash，见下方
    // [`audit_action_for`] 的职责划分。
    let content = crate::inject::normalize::compose_injection(&device_name, &req.text);
    // D6 修改重发：queueOnly=true 只跳过 ⑥ 的直发尝试（语义见 SessionSendReq::queue_only
    // 注释），入队与 ⑦ 运行中留队完全同路径同审计口径（action=queue）——flush 循环对
    // queueOnly 项与普通队列项同权（转闲按序自动放行），队列存储不携带该标志
    let queue_only = req.queue_only.unwrap_or(false);
    let now = chrono::Utc::now().timestamp_millis();
    let (item_id, position) = st.store.with(|c| {
        let id = crate::database::dao::inject_queue::enqueue_conn(
            c,
            &sid,
            &tool,
            &device_id,
            &device_name,
            &content,
            now,
        );
        let pos =
            crate::database::dao::inject_queue::pending_for_session_conn(c, &sid).len() as i64;
        (id, pos)
    });
    if item_id == 0 {
        // DAO 写失败哨兵（enqueue_conn 失败返回 0 并 log）：不下发 delivered 谎报
        return json_no_store(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": "internal" }),
        );
    }
    let queued = serde_json::json!({
        "status": "queued",
        "itemId": item_id,
        "position": position,
    });
    // ⑥ 可输入态 → 直发（flush_one 投递内核，jump=false；in-flight 守卫与 flush 循环共用，
    //    防同会话并发双投）。守卫与投递同生命周期于 spawn_blocking 闭包内（fff9c29 flush
    //    循环事件臂同款，F1 断连双投修复）——handler 断连（弱网/隧道掐断慢投递）不再
    //    提前释放守卫，detached 投递期间新触发经 INFLIGHT 互斥让位。
    //    D6：queueOnly=true 时整个分支不触达（in-flight 守卫取用、五态回执映射、
    //    send/unconfirmed/failed 直发审计全部跳过），直接落 ⑦ 入队路径
    if !queue_only && crate::inject::queue::is_input_ready(&session.status) {
        let flush_st = st.clone();
        let flush_sid = sid.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            // 守卫必须是闭包第一条语句：宿主与投递同栈，handler future 因断连被 drop
            // 也不会提前释放——detached 旧投递全程占位，新触发取不到名额即让位
            let Some(_guard) = crate::inject::queue::try_acquire_inflight(&flush_sid) else {
                // flush 循环 / detached 旧投递正在投递该会话：本条保持入队（进行中的
                // 那次已覆盖该会话队首，下一跃迁接力本条），Deferred → 下方 queued 映射
                // ——不谎报 delivered
                return crate::inject::queue::FlushOutcome::Deferred;
            };
            crate::inject::queue::flush_one(&flush_st, &flush_sid, false)
        })
        .await
        .unwrap_or_else(|e| {
            log::error!("session-send 直发任务异常: {e}");
            crate::inject::queue::FlushOutcome::Failed("内部任务异常".to_string())
        });
        // P1-4 五态精确映射（D7/T3 增 Submitted）：Sent/Submitted/Failed/
        // Deferred/Suspended → delivered/submitted/failed/queued/queued；守卫忙
        // 让位归 Deferred，与黄态/挂起同臂——单臂收敛（原 handler 帧守卫的
        // Some/None 双臂已删，审计 queue|ok 口径不变）
        return match outcome {
            crate::inject::queue::FlushOutcome::Sent => {
                endpoint_audit(
                    &st,
                    &device_id,
                    &device_name,
                    &tool,
                    &sid,
                    &content,
                    audit_action_for(&req.text, "send"),
                    "ok",
                );
                json_no_store(StatusCode::OK, serde_json::json!({ "status": "delivered" }))
            }
            // D7/T3 中性回执（验收问题 #5）：注入 Ok + 戳未中 + 屏读无滞留草稿 =
            // 消息已被 TUI 收进内部队列（已投递未确认）——不冒充 delivered（未确认
            // 落盘）也不冒充 failed（防重警示会诱导重试 = 双发）；行已 mark_sent
            // 消费退出 pending（settle 落账 result=unconfirmed），队列无残留
            crate::inject::queue::FlushOutcome::Submitted => {
                endpoint_audit(
                    &st,
                    &device_id,
                    &device_name,
                    &tool,
                    &sid,
                    &content,
                    audit_action_for(&req.text, "send"),
                    "unconfirmed",
                );
                json_no_store(StatusCode::OK, serde_json::json!({ "status": "submitted" }))
            }
            // 注入/确认失败（行已 mark_failed，队列无残留——回执可重试，W1/W4）
            crate::inject::queue::FlushOutcome::Failed(e) => {
                endpoint_audit(
                    &st,
                    &device_id,
                    &device_name,
                    &tool,
                    &sid,
                    &content,
                    audit_action_for(&req.text, "send"),
                    &format!("failed:{e}"),
                );
                json_no_store(
                    StatusCode::OK,
                    serde_json::json!({ "status": "failed", "error": e }),
                )
            }
            // E1① 撤回窗口防护中止（理论不可达臂：本变体仅插队路径产出，穷尽性保留
            // ——防御性回 failed，error=「未投递：<原因>，请人工确认」如实透出）
            crate::inject::queue::FlushOutcome::NotDelivered(reason) => {
                endpoint_audit(
                    &st,
                    &device_id,
                    &device_name,
                    &tool,
                    &sid,
                    &content,
                    audit_action_for(&req.text, "send"),
                    &format!("aborted:{reason}"),
                );
                json_no_store(
                    StatusCode::OK,
                    serde_json::json!({
                        "status": "failed",
                        "error": format!("未投递：{reason}，请人工确认"),
                    }),
                )
            }
            // Deferred/Suspended（含守卫忙让位）：行保持 pending 等会话回来/下个跃迁，
            // 语义即排队（Suspended 亦 queued）——回查 pending 取该条目实时位次回执
            crate::inject::queue::FlushOutcome::Deferred
            | crate::inject::queue::FlushOutcome::Suspended => {
                endpoint_audit(
                    &st,
                    &device_id,
                    &device_name,
                    &tool,
                    &sid,
                    &content,
                    audit_action_for(&req.text, "queue"),
                    "ok",
                );
                // position = 该条目在 pending 队列中的位次（第 1 位 = 1，评审 Minor 5
                // ——len() 会把队首误报为 1 条以后）；并发消费致条目已不在 pending →
                // 0（**前端 Task 8 须容忍 0**：语义为「已不在队列，由投递循环接力」）
                let pos = st.store.with(|c| {
                    crate::database::dao::inject_queue::pending_for_session_conn(c, &sid)
                        .iter()
                        .position(|i| i.id == item_id)
                        .map_or(0, |p| p as i64 + 1)
                });
                json_no_store(
                    StatusCode::OK,
                    serde_json::json!({
                        "status": "queued",
                        "itemId": item_id,
                        "position": pos,
                    }),
                )
            }
        };
    }
    // ⑦ 运行中（黄灯）→ 留队等下一可输入态（D6：queueOnly=true 的可输入态会话
    //    也落此臂——审计 action=queue 与「运行中留队」同口径，回执同 queued{itemId,position}）
    endpoint_audit(
        &st,
        &device_id,
        &device_name,
        &tool,
        &sid,
        &content,
        audit_action_for(&req.text, "queue"),
        "ok",
    );
    json_no_store(StatusCode::OK, queued)
}

/// routing Channel → wire 小写字符串（枚举未派生 serde，端点侧手工映射防漂移）
fn channel_wire(c: &crate::inject::routing::Channel) -> &'static str {
    use crate::inject::routing::Channel;
    match c {
        Channel::Tmux => "tmux",
        Channel::Iterm2 => "iterm2",
        Channel::TerminalApp => "terminal_app",
        Channel::WindowsConsole => "windows_console",
    }
}

/// routing Visibility → wire 字符串（与移动端 SendInfo 联合类型对齐）
fn visibility_wire(v: &crate::inject::routing::Visibility) -> &'static str {
    use crate::inject::routing::Visibility;
    match v {
        Visibility::Realtime => "realtime",
        Visibility::AfterRefresh => "after_refresh",
    }
}

/// GET /m/api/v1/session-send-info?session_id=（W4 输入区可用性矩阵）：
/// 会话不在快照 → 404 no_session；路由判定 → 200 {injectable, channels, visibility}
/// 或 {injectable:false, reasonCode, reason}。快照是同步阻塞扫描，spawn_blocking 包裹。
pub async fn session_send_info(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    let probe_st = st.clone();
    let session =
        match tokio::task::spawn_blocking(move || find_session_sync(&probe_st, &sid)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("session-send-info 会话扫描任务异常: {e}");
                return json_no_store(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                );
            }
        };
    let Some(session) = session else {
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "no_session" }),
        );
    };
    let tool = session.agent_type.tool_id().to_string();
    let body =
        match crate::inject::routing::route(&tool, session.form, session.pid, std::env::consts::OS)
        {
            crate::inject::routing::RouteOutcome::Injectable {
                candidates,
                visibility,
            } => serde_json::json!({
                "injectable": true,
                "channels": candidates.iter().map(channel_wire).collect::<Vec<_>>(),
                "visibility": visibility_wire(&visibility),
            }),
            crate::inject::routing::RouteOutcome::NotInjectable {
                reason_code,
                reason,
            } => {
                serde_json::json!({
                    "injectable": false,
                    "reasonCode": reason_code,
                    "reason": reason,
                })
            }
        };
    json_no_store(StatusCode::OK, body)
}

/// GET /m/api/v1/session-queue?session_id=（W4 排队视图）：该会话全部待发消息
/// FIFO（position = 1 起的队位；content 已是入队时 compose 完成的最终注入文本）。
/// 轻量 SQLite 查询（pending_for_session），无需 spawn_blocking（host handler 同口径）。
pub async fn session_queue(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    let items = st
        .store
        .with(|c| crate::database::dao::inject_queue::pending_for_session_conn(c, &sid));
    let views: Vec<serde_json::Value> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            serde_json::json!({
                "id": it.id,
                "content": it.content,
                "enqueuedAt": it.enqueued_at,
                "position": i + 1,
            })
        })
        .collect();
    json_no_store(StatusCode::OK, serde_json::json!({ "items": views }))
}

/// POST /m/api/v1/session-queue/jump（裁决 12 插队）：按 itemId 点名该会话 pending 中
/// 的条目（可非队首）即刻投递（黄态照发）——flush_given 内核，settle 落账审计 action=jump。
/// - 缺参 → 400；无此 pending 项 → 404 not_found；
/// - 五态精确映射（P1-4 + D7/T3）：Sent → 200 delivered；Failed(e) → 200 failed{error}
///   （注入失败行已退出 pending）；Submitted → 200 submitted（防御性臂：Submitted
///   仅直发分诊产出，插队以占用排空定论，本臂实际不可达）；Deferred | Suspended →
///   200 queued + itemId/position
///   （行保持 pending 等会话回来/下个跃迁，语义即排队——jump 点名场景 Deferred 的黄态
///   臂实际不可达〔jump 跳过黄态复核〕，守卫忙让位与会话消失〔Suspended〕亦按 queued）；
/// - in-flight 守卫忙 → 200 queued{itemId,position}（**回执契约变化，F1 裁决**：旧忙时
///   回 failed「该会话投递进行中」逼客户端重试，现改 queued——条目保持 pending 语义即
///   排队，让位给进行中的那次投递。前端兼容性论证：移动端 handleJump 对非 delivered
///   回执一律走 reconcileQueued 对账，条目仍在队即恢复排队视图（= queued 直认），不存在
///   对 jump 忙 failed 文案的交互依赖，前端 Task 8 已兼容）。守卫取在 spawn_blocking
///   闭包内（fff9c29 flush 循环事件臂同款，F1 断连双投修复）。
pub async fn session_queue_jump(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<QueueItemReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() || req.item_id.is_none_or(|id| id <= 0) {
        return bad_request();
    }
    let item_id = req.item_id.unwrap_or(0);
    if device_identity(&st, &headers).is_none() {
        return forbidden_defense();
    }
    let failed_body = |e: String| {
        json_no_store(
            StatusCode::OK,
            serde_json::json!({ "status": "failed", "error": e }),
        )
    };
    // 前查归属（借 retract 语义）+ 取整行（点名投递需要 item 字段），无 pending 项 → 404
    let target = st.store.with(|c| {
        crate::database::dao::inject_queue::pending_for_session_conn(c, &sid)
            .into_iter()
            .find(|i| i.id == item_id)
    });
    let Some(item) = target else {
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "not_found" }),
        );
    };
    let flush_st = st.clone();
    let flush_sid = sid.clone();
    let outcome = match tokio::task::spawn_blocking(move || {
        // 守卫必须是闭包第一条语句（fff9c29 flush 循环事件臂同款，F1 断连双投修复）：
        // 宿主与投递同栈，handler 断连（弱网/隧道掐断慢投递）不再提前释放守卫——
        // detached 旧投递全程占位，新触发取不到名额即让位
        let Some(_guard) = crate::inject::queue::try_acquire_inflight(&flush_sid) else {
            // 该会话已有投递进行中（与直发/flush 循环共用守卫）——让位：条目保持
            // pending 语义即排队，回执 queued（**裁决变化**：旧忙时回 failed「投递
            // 进行中」提示重试；移动端 handleJump 对非 delivered 走 reconcileQueued
            // 对账，queued 直认恢复排队视图，无 failed 交互依赖——前端 Task 8 已兼容）
            return crate::inject::queue::FlushOutcome::Deferred;
        };
        // 守卫下再校验 pending 归属（守卫入闭包的伴随义务）：上方前查已不在守卫下，
        // 间隙内 flush 循环可能已投递本条并释放守卫——不复查会对已 sent 条目再注入
        // （双投）。复查+投递已抽为 flush_given_if_pending 可测内核（收尾批 P2，
        // 单测 jump_deliver_under_guard）：复查 SQL 短临界区、注入在锁外；已被消费
        // → Deferred（queued/position=0，语义「已不在队列，由投递循环接力」，
        // 前端 Task 8 须容忍 0）
        crate::inject::queue::flush_given_if_pending(&flush_st, &item, true)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-queue/jump 任务异常: {e}");
            crate::inject::queue::FlushOutcome::Failed("内部任务异常".to_string())
        }
    };
    match outcome {
        crate::inject::queue::FlushOutcome::Sent => {
            json_no_store(StatusCode::OK, serde_json::json!({ "status": "delivered" }))
        }
        // D7/T3：Submitted 仅直发确认分诊产出（插队以占用排空定论，本臂实际不可达，
        // 为穷尽性保留）——防御性回中性 submitted，不冒充 delivered 也不冒充 failed
        crate::inject::queue::FlushOutcome::Submitted => {
            json_no_store(StatusCode::OK, serde_json::json!({ "status": "submitted" }))
        }
        // E1① 撤回窗口防护中止：正文**未注入**（输入行有疑似被撤回的残留，注入即
        // 拼接危害）——行已 mark_failed 退出 pending，回 failed{error} 如实透出
        // 「未投递：<原因>，请人工确认」（前端 handleJump 对非 delivered 走对账，
        // 行不在队 → 中性 gone 收敛；原因文案在 failed 回执可见）
        crate::inject::queue::FlushOutcome::NotDelivered(reason) => {
            failed_body(format!("未投递：{reason}，请人工确认"))
        }
        crate::inject::queue::FlushOutcome::Failed(e) => failed_body(e),
        // Deferred/Suspended（含守卫忙让位/已被他方消费）：行保持 pending 或已退出，
        // 语义即排队——回查 pending 取该条目实时位次回执
        crate::inject::queue::FlushOutcome::Deferred
        | crate::inject::queue::FlushOutcome::Suspended => {
            // position = 条目位次（第 1 位 = 1，评审 Minor 5）；并发消费致条目已不在
            // pending → 0（**前端 Task 8 须容忍 0**：语义为「已不在队列，由投递循环接力」）
            let pos = st.store.with(|c| {
                crate::database::dao::inject_queue::pending_for_session_conn(c, &sid)
                    .iter()
                    .position(|i| i.id == item_id)
                    .map_or(0, |p| p as i64 + 1)
            });
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "queued",
                    "itemId": item_id,
                    "position": pos,
                }),
            )
        }
    }
}

/// POST /m/api/v1/session-queue/retract（W4 撤回）：前查该会话 pending 中的目标条目
/// （取审计字段）→ DAO retract_conn（内部再按 (session_id, id) 二次校验，防竞态误删）
/// → 审计 action=retract → 200 {ok:true}；无此 pending 项 → 404 not_found。
/// P2-6：撤回与投递共守卫（照 jump 先例，device_identity 校验后、pending 前查之前取
/// in-flight 名额）——撤回正删的行可能正是投递内核已取走待落账的队首，共守卫使两者
/// 串行化，杜绝「撤回成功回执但消息仍被注入」的竞态；忙时短回执（200 failed 提示重试）。
/// F1 披露：本端点守卫**留在 handler 帧**是有意为之——撤回临界区是纯 DB 工作，与守卫
/// 同栈同生命周期，不存在「handler 断连 drop future 提前释放守卫、detached 投递仍在
/// 跑」的窗口（jump 已因 detached 投递入闭包，本端点无此形态，勿照搬）。
pub async fn session_queue_retract(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<QueueItemReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() || req.item_id.is_none_or(|id| id <= 0) {
        return bad_request();
    }
    let item_id = req.item_id.unwrap_or(0);
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    let failed_body = |e: String| {
        json_no_store(
            StatusCode::OK,
            serde_json::json!({ "status": "failed", "error": e }),
        )
    };
    // P2-6 共守卫（与 flush 循环/直发/插队互斥）：投递进行中 → 撤回让位，短回执提示重试
    let Some(_guard) = crate::inject::queue::try_acquire_inflight(&sid) else {
        return failed_body("投递进行中，请稍后重试".to_string());
    };
    // 前查：拿条目供审计（agent_type / 已 compose 的 content），同时完成归属校验
    let target = st.store.with(|c| {
        crate::database::dao::inject_queue::pending_for_session_conn(c, &sid)
            .into_iter()
            .find(|i| i.id == item_id)
    });
    let Some(item) = target else {
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "not_found" }),
        );
    };
    let removed = st
        .store
        .with(|c| crate::database::dao::inject_queue::retract_conn(c, &sid, item_id));
    if !removed {
        // 前查后竞态被他人删走：按 404 语义（DAO 二次校验未命中）
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "not_found" }),
        );
    }
    endpoint_audit(
        &st,
        &device_id,
        &device_name,
        &item.agent_type,
        &sid,
        &item.content,
        "retract",
        "ok",
    );
    json_no_store(StatusCode::OK, serde_json::json!({ "ok": true }))
}

// ==== 移动端附件上传（2026-09-20 用户裁决）====
// 存储 = <会话 cwd>/.mam-attachments/<session_id>/（用户项目目录下——所有工具读
// 工作区内文件天然零审批）；git 零污染 = 首份写入时幂等追加 .git/info/exclude
// （本地管理区，非 .gitignore 跟踪文件）；非 git 项目跳过。消息侧以
// <image|file path> 内联标记引用（文件池既有约定，自动入池「我上传的」）。

/// 移动端附件上传（query 传 session_id + 文件名；body = 原始字节）。
/// 错误契约（与既有端点同族）：400 参数空；403 防御；404 {"error":"no_session"|"no_cwd"}
/// （no_cwd 与 resume 同源口径）；413 {"error":"too_large"}；200 {path,size}。
/// 安全边界：cwd 由服务端解析（客户端零路径输入），文件名服务端消毒——杜绝任意写。
pub async fn session_attachment(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> Response {
    // ① 参数校验（trim 判空——与 session-send 同口径）
    let sid = params.get("session_id").map(|s| s.trim().to_string());
    let Some(sid) = sid.filter(|s| !s.is_empty()) else {
        return bad_request();
    };
    let Some(raw_name) = params
        .get("name")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    // ② 容量上限（结构化 413；Content-Length 预检可测，DefaultBodyLimit 为硬兜底）
    if body.len() > crate::remote::attachments::MAX_ATTACHMENT_BYTES {
        return json_no_store(
            StatusCode::PAYLOAD_TOO_LARGE,
            serde_json::json!({ "error": "too_large" }),
        );
    }
    // ③ 设备身份（防御 403 + 花名）
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // ④ 会话查找（spawn_blocking：扫描是重活）——cwd 服务端解析的唯一来源
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let session =
        match tokio::task::spawn_blocking(move || find_session_sync(&probe_st, &probe_sid)).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                return json_no_store(
                    StatusCode::NOT_FOUND,
                    serde_json::json!({ "error": "no_session" }),
                );
            }
            Err(e) => {
                log::error!("session-attachment 会话扫描任务异常: {e}");
                return json_no_store(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                );
            }
        };
    let cwd = session.project_path.trim().to_string();
    if cwd.is_empty() {
        // 与 resume 的 no_cwd 同源口径（该会话没有项目目录信息）
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "no_cwd" }),
        );
    }
    let tool = session.agent_type.tool_id().to_string();
    // ⑤ 写盘（spawn_blocking：同步 IO）+ 审计
    let write_cwd = cwd.clone();
    let write_sid = sid.clone();
    let write_name = raw_name.clone();
    let write_body = body;
    let size = write_body.len();
    let written = tokio::task::spawn_blocking(move || {
        crate::remote::attachments::write_attachment(
            std::path::Path::new(&write_cwd),
            &write_sid,
            &write_name,
            &write_body,
        )
    })
    .await;
    let path = match written {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            log::error!("session-attachment 落盘失败: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "io" }),
            );
        }
        Err(e) => {
            log::error!("session-attachment 写盘任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    endpoint_audit(
        &st,
        &device_id,
        &device_name,
        &tool,
        &sid,
        &raw_name,
        "attachment",
        "ok",
    );
    // 数据管理索引（C5）：追加一条（服务端写入；home 不可得则跳过——索引只服务
    // 桌面管理面，丢失不影响附件本身）。测试态 home_source=None 自动跳过
    if let Some(home) = (st.home_source)() {
        crate::remote::attachments::append_index_entry(
            std::path::Path::new(&home),
            &crate::remote::attachments::AttachmentIndexEntry {
                project: cwd,
                session: sid,
                tool,
                name: crate::remote::attachments::sanitize_file_name(&raw_name),
                path: path.to_string_lossy().into_owned(),
                size: size as u64,
                ts: (st.now_source)(),
            },
        );
    }
    json_no_store(
        StatusCode::OK,
        serde_json::json!({
            "path": path.to_string_lossy(),
            "size": size,
        }),
    )
}

// ==== M8 Task 11：审批端点（session-approve-options / session-approve）====
// 契约（JSON camelCase；Json 响应带 no-store——门禁下私有写/读路径）：
//   GET  /session-approve-options?session_id= → 200 {available, options:[{id,label}],
//        verifiedWith, currentVersion(…|null), drift, reason(…|null)}
//        available = **（status==Waiting ∨ 审批等待标记存在）** && 映射表有该工具
//        映射 && （标记路径跳过 detect；无标记才走 detect 命中）（T4 口径：钩子
//        事件落的等待标记是一等信号——审批等待期文件推导常判 processing，提示文本
//        又不落会话文件，标记使 macOS 红卡可达；标记路径直接出全选项）
//        ；工具无映射 / 未命中 / 非 Waiting 且无标记 → available=false
//        （options 空）；严格档（M9R Task 10）：映射 verified_with==probe-pending →
//        available=false + reason=降级文案（选项空，前端只渲染提示条）。
//        options 只含 id+label——**键位不外泄给 UI**。
//   POST /session-approve body {sessionId, optionId} → 200 {"status":"key_sent"}
//        | 404 no_session | 409 not_waiting | 404 no_mapping（降级提示走普通发送；
//        成因三态：无该工具映射 / optionId 无对应项 / probe-pending 严格档——
//        未取证=映射缺失，M9R Task 10）
//        | 200 failed{error}（注入失败 / in-flight 忙，可重试回执）。
//        等待门与 GET 同口径：**Waiting ∨ 标记**（T4）；无标记 + 非 Waiting 仍
//        409，标记不扩大放行面（回归锁 approve_without_mark_still_409_on_processing）。
// 锁纪律（M4）：KV 映射读取并入 `st.store.with` 短临界区（load_mappings_conn 直用
// 传入 conn、不自取锁，锁内只 SQL）；session_source 调用与 store.with **顺序执行不
// 嵌套**——生产 session_source 内部会锁同一把全局 DB，嵌套即自锁死锁。

/// 审批对话框的**键序档**（R1-2：按对话框模型分族，不按工具）。
///
/// 实机证据（2026-09-21，两处独立探测 + 本机复验）表明「同一工具的不同对话框
/// 键序不同」：
///
/// | 对话框 | 数字键 | 可靠路径 | 证据 |
/// |---|---|---|---|
/// | claude **AskUserQuestion** | ✅ 即选即交 | 数字 | K1 实机（批次乙） |
/// | claude **计划批准** | ❌ 无效 | ↓×k + Enter | R1-2 独立探测三样本 + 本机复验 |
/// | kimi **计划批准**（Ready to build） | ⚠️ **不可依赖** | ↓×k + Enter | R1-1 独立探测：`'2'+Enter` 误批准、`'3'` 单键拒绝——行为不一致 |
/// | codex Implement this plan | ✅ 有效 | 数字 | 本轮实机（`'1'` 关闭对话框并开工） |
///
/// 故 approve 侧不再「一律发数字」，改由本档决定：**数字直选** vs **导航确认**。
///
/// # 未覆盖的实机面（丁T2 复评 M2，如实申报——勿把本档读成「已验证可达」）
///
/// 本档只承载「**该发什么键**」的规格（来自上表的独立探测），**不含**「键到达后终端
/// 是否按预期响应」的端到端验证：
/// - kimi 计划批准框：R1-1 的证伪证明了「数字不可靠」（故改导航），但**导航序列在本
///   任务的真实会话里未复跑**（无受控会话窗口）——「点 Reject 必达」这一验收断言
///   **不可由本轮证据支持**，属未覆盖实机面；
/// - kimi 侧另有已知变体未纳入探测：R1-3 实测的 `▶ Write this file?` **四选项**工具
///   批准框（本批只做 plan_review，该变体走降级提示条，不出现该框的键序面）；
/// - claude 计划批准框的导航序列有 R1-2 三样本 + 本机复验（**已覆盖**）；codex 的
///   数字直选有 T5 实机（**已覆盖**）。
///
/// 结论口径：kimi 的卡**出卡与降级链**已由端点测试覆盖（`kimi_plan_approval_card_*`
/// / `kimi_approve_card_carries_no_plan_body`），**键序生效性**属未覆盖实机面——
/// 复验走项目技能 `win-console-inject-probe`（单工具规格定案表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApproveDialogKeys {
    /// 数字键即选即交（codex 计划批准）——实机验证可靠
    DigitDirect,
    /// **数字优先+屏读验证+导航回退**（批次戊 E2①，用户终裁 CL-3：claude 计划批准框
    /// 数字**直接选中**——丁复审/戊探E 的「数字无效 ×5」系**注入早于选项渲染**的
    /// 假阴性）。序列：注入前屏读确认选项簇已渲染（未就绪在
    /// [`crate::inject::timing::MENU_POLL_TOTAL_MS`] 窗内轮询——裁18 通用规则）→
    /// 发数字 → 屏读验证「对话框已消失」（[`crate::inject::timing::DIGIT_VERIFY_POLL_TOTAL_MS`]
    /// 窗）→ 未生效走导航回退（既有 [`Self::NavigateConfirm`] 序列）。
    DigitFirstWithVerify,
    /// 导航确认：`↓×k + Enter`（kimi 计划批准类）——数字不可依赖（'2'+Enter 误批准
    /// 实证，未被推翻），必须按解析到的当前高亮位算步进。**kimi 恒定本档，数字禁令
    /// 是永久性安全锁（E2 加锁）**。
    NavigateConfirm,
}

/// 会话工具 → 审批对话框键序档（R1-2 定档；E2① claude 改数字优先双路径）。
///
/// **收窄口径**：codex 数字直选（实测有效）；claude 数字优先+验证回退（用户终裁，
/// 渲染等待规则消除假阴性）；kimi 导航确认（数字禁令）。未收录工具保守走
/// [`ApproveDialogKeys::NavigateConfirm`]？**不**——未知工具本就没有映射表
/// （`no_mapping` 404），到不了这里；此处只覆盖有映射的两家 + kimi。
fn approve_dialog_keys(tool: &str) -> ApproveDialogKeys {
    match tool {
        // claude：数字直接选中（用户终裁）+ 渲染等待 + 屏读验证 + 导航回退
        "claude" => ApproveDialogKeys::DigitFirstWithVerify,
        // codex：实测数字有效（Implement this plan → '1' 提交并开工）
        "codex" => ApproveDialogKeys::DigitDirect,
        // kimi：数字禁令（'2'+Enter 误批准实证）→ 导航确认（E2 锁：不得改数字档）
        _ => ApproveDialogKeys::NavigateConfirm,
    }
}

/// 审批选项扫描产物（映射存在时的载荷；available=false 时 options 恒空）
struct ApproveScanHit {
    available: bool,
    /// 选项表（只含 id+label——键位不外泄给 UI）
    options: Vec<(String, String)>,
    verified_with: String,
    /// 工具标识（currentVersion 探测的数据源）
    tool: String,
    /// 严格档降级原因（M9R Task 10）：verified_with==probe-pending 时下发
    /// [`crate::inject::approve::PROBE_PENDING_REASON`]（前端 ApproveCard 只渲染提示条）；
    /// 其余不可批形态（非 Waiting/未命中）不给 reason（前端按自隐处理）
    reason: Option<String>,
    /// T5：本次选项是否来自**对话框屏读**（true → options 的 id 形如 `dialog:<n>`，
    /// 提交时注入数字键 n；false → 映射表二元项，走既有 approve/reject 键）
    dialog: bool,
    /// T8：审批点关联的计划内容（None = 无计划消息在场 → 前端只渲染选项）
    plan_body: Option<ApprovePlanBody>,
    /// R1-3：**降级警示**——命中审批但未读到对话框选项（终端可能正显示多选对话框，
    /// 二元键可能错位）。下发文案，前端在二元卡渲染脚注。
    degraded_hint: Option<String>,
    /// 丁T2：**计划待确认预期态**在场（消息尾部派生，见 [`plan_pending_tail_index`]）。
    /// true 且无屏读选项 → 前端渲染「计划待确认」条 +「检查终端对话框」按钮（而不是
    /// 二元/空选项组）——codex 的 `Implement this plan?` 不落状态、不落标记，这是它
    /// 唯一的入口。
    plan_pending: bool,
}

/// R1-3 降级警示文案（前端 `degrade-hint` 脚注原文；中文，与既有 reason 提示同风格）
const DEGRADED_DIALOG_HINT: &str =
    "未读到终端对话框选项——终端可能正显示多选项，二元键可能错位，建议到终端确认";

/// 丁T2 kimi 的「无屏读即不出键」降级文案（available=false + reason → 前端提示条）。
/// 依据 R1-1 实机证伪：kimi 计划批准框的数字通道**不可依赖**（`'2'+Enter` 误批准、
/// `'3'` 单键拒绝），故本工具**没有任何**可安全下发的映射表二元键——屏读失败即不出手。
const KIMI_NO_DIALOG_REASON: &str =
    "该审批需在终端对话框中选择，暂未读到选项——请在终端处理（数字键不可靠，不代按）";

/// 丁T2：**计划确认类对话框族**（codex / kimi）——两家的计划确认框都是「屏读出选项、
/// 数字/导航代按」形态，且**映射表二元键对计划框均未取证**：
///
/// - codex：映射表 `y/esc` 只对**补丁审批**（"Would you like to make the following
///   edits?"）实机取证（M8R 0.154.0）；计划框实测的是**数字直选**（T5 档案 `'1'` 关框
///   并开工），键位语义不同，不能拿 `y` 顶。
/// - kimi：R1-1 独立探测证实计划框数字通道不可依赖，映射表无可用键位。
///
/// 故本族在两处收窄（见 `approve_options_scan`）：① 端点门放宽到「计划预期态」；
/// ② 计划预期态命中且**未读到对话框选项**时不下发映射表/空选项键，改下发
/// 「计划待确认」条（`plan_pending`）。
///
/// claude **不纳入**：它的计划批准走既有 `Waiting + detect`（映射表 `approve="1"` 有
/// M8R 双场景取证），且其映射表在计划框外还有 Write 审批等已取证场景——放宽会改既有
/// 行为，属本批范围外。
fn plan_dialog_family(tool: &str) -> bool {
    matches!(tool, "codex" | "kimi")
}

/// 丁T2 复评 F3-4：**审批卡计划聚合**的工具面（与 [`plan_dialog_family`] 的分工见下）。
///
/// 任务书 T2 对 kimi 的成文要求：**kimi 的计划审批卡不含 plan 正文**——正文由 §2.2 的
/// kimi 正文卡承担（双卡矩阵按家分列差异，不是「各卡渲染同一份数据」）。故 kimi **不**
/// 进本族（审批卡载荷 `plan` 恒 null），其正文只由消息流的 `kind="plan"` 正文卡承载
/// （`content::kimi_plan_cards`，本批保留）。
///
/// 与 [`plan_dialog_family`] 的**分工**（两个族不是一回事，勿合并）：
/// - `plan_dialog_family`：**门与键位**的族（决定「计划预期态能否放宽 Waiting 门」
///   与「无屏读选项时不出映射键」）——codex **与** kimi 都在内（kimi 的
///   「Write this file?」变体要靠预期态门接住，键位面同样无键可发）；
/// - `approve_plan_family`（本函数）：**审批卡正文聚合**的族——只有 codex（claude 也
///   在内，它走的是「正文进审批卡」的既有 T8 行为）。
///
/// 为什么 claude 在内：批次丙 T8 的 claude 计划聚合（ExitPlanMode 的 plan 正文进审批卡）
/// 是**已上线且用户已接受**的行为（见 `plan_from_page` 注释与
/// `approve_card_aggregates_claude_plan_body` 回归锁）——本批只修它「被静默失效」的
/// 回归（F3-3），不改其形态。
fn approve_plan_family(tool: &str) -> bool {
    matches!(tool, "claude" | "codex")
}

/// 丁T2：**计划待确认预期态**判定（纯函数，从消息尾部派生，**无新存储**）。
///
/// 返回尾部最后一条计划类消息的下标——其**之后**不存在下列消息：
/// - `kind="user"`：用户已注入/发言 → 计划提案已被消费（任务书口径：「下一个用户
///   消息注入即清除」）；
/// - `kind="tool-call"` / `kind="tool-result"`：工具已继续执行 → 对话框已关闭
///   （kimi 批准后 wire 立即落 `tool.call ExitPlanMode` + `tool.result`，而用户消息
///   要等下一轮才出现——没有这条判据，kimi 的预期态会在批准后一直挂着）。
///
/// 计划类消息 = `kind="plan"`（claude 的 ExitPlanMode 升格 / codex 的
/// `<proposed_plan>` 升格 / kimi 的 `interaction.request.display.plan`）或
/// `kind="plan-file"`（kimi 的 plan 文件引用与 `display.path`——kimi 的「Write this
/// file?」变体（R1-3 实测）没有 plan_review，只有计划文件卡，纳入才覆盖得到）。
///
/// 判据刻意**不看状态**：codex 计划提案之后不落 Waiting（`codex_parser` 的兜底红已
/// 废），状态面没有信号——这正是本预期态存在的理由。
///
/// # 未覆盖面（丁T2 复审 N4，如实申报——勿把本判据读成「覆盖全部清除路径」）
///
/// 三条清除信号（user / tool-call / tool-result）是从**实机 wire 序列**归出来的，
/// 但**并非穷尽**：
/// - **codex 选「No, stay in Plan mode」后**：既不注入用户消息、也**不必然**产工具事件
///   （模型停在计划模式等下一轮输入）→ 尾部计划卡常在 → 预期态可能**长挂**（前端
///   「计划待确认」条持续显示，直到用户注入或有工具事件）。
/// - **取证状态（复审核查）**：203 条真机 rollout 上宽松档命中 5 条——4 条是孤立
///   `plan-file`（F3-5 刻意保留的纳入面）、1 条是真计划尾；**未观察到上述样本**，
///   故只能标为**待取证**（结论不超证据：不为其加判据、也不为它改行为）。
/// - 若将来取证到该形态，收窄点候选：要求计划卡**紧邻前一条不是 tool-result**
///   （排除「刚读完计划文件就报出正文」），或引入「codex plan 模式的模式位回读」
///   （依赖 T4 的模式栏能力，属跨任务面）。
///
/// 与 [`pending_question_tail_index`] 同族（同一份消息页、同一「其后无后续形态」骨架），
/// 两者都由 `approve_options_scan` 在**同一次**读页里消费。
pub(crate) fn plan_pending_tail_index(
    msgs: &[crate::remote::content::SessionMessage],
) -> Option<usize> {
    let last = msgs
        .iter()
        .rposition(|m| m.kind == "plan" || m.kind == "plan-file")?;
    if msgs[last + 1..]
        .iter()
        .any(|m| matches!(m.kind.as_str(), "user" | "tool-call" | "tool-result"))
    {
        return None;
    }
    Some(last)
}

/// 丁T2 复评 F3-5：**严判据**——供**问答卡的压制**使用（审批侧仍用宽松的
/// [`plan_pending_tail_index`]）。两者的唯一差别是**计划类消息的认定面**：
/// 本函数只认 `kind="plan"`（正文卡），**不认孤立的 `kind="plan-file"`**。
///
/// # 为什么必须分档（评审 I3 的误报面）
///
/// `kind="plan-file"` 是**跨工具形态判据**：任何工具结果只要提到 `plans/<名>.md`
/// （`content::extract_plan_file_ref`）就产卡——它**不是**「有一个计划在等确认」的
/// 证据，只是「某处出现过计划文件路径」。把它算作预期态，会出现：一次普通工具结果
/// 提了个 plans 目录下的文件 → 已消费的计划被**重新点成**预期态。
///
/// 两条消费通路的**误报代价不对称**（这是分档的全部理由）：
/// - **审批侧**（宽松档）：误报产出的是一张「计划待确认」提示条 —— 用户点检查、
///   屏读没读到选项、得到降级提示；**可用性不受损**（审批卡本就只有该条路径）；
///   而漏报的代价是「kimi 的 Write this file? 变体（只有文件卡、无 plan_review）
///   完全无入口」——故审批侧**保留** plan-file 纳入。
/// - **问答侧**（本档）：误报的后果是**把可作答的问答卡压掉**（`question_scan_sync`
///   直接 return None）——用户看到的是「问题在终端等我，但手机上没有卡」，
///   属**可用性受损**方向；而漏报只是回到本批之前的既有行为（问答卡照常出）。
///   故问答侧收窄到「正文卡在场」这一更强的信号。
///
/// # 「正文卡在场」为什么仍是有效的压制信号
///
/// kimi 的计划确认框（`plan_review`）在 wire 里**必然**带 `display.plan` 内联全文
/// （本机全库实录：20 条 `plan_review` 全部带 `plan` 字段）→ 必然产正文卡。
/// 故「尾部有正文卡且其后无用户消息/工具事件」= 真计划确认在场，压制成立；
/// 孤立的文件卡（如 `Wrote N bytes to …/plans/x.md` 的历史产物）不再压制。
///
/// 判据骨架与宽松档逐字相同（尾部位置 + 其后无 user/tool-call/tool-result）——
/// **只换计划类消息的认定面**，两档不漂移。
pub(crate) fn plan_pending_strict_tail_index(
    msgs: &[crate::remote::content::SessionMessage],
) -> Option<usize> {
    let last = msgs.iter().rposition(|m| m.kind == "plan")?;
    if msgs[last + 1..]
        .iter()
        .any(|m| matches!(m.kind.as_str(), "user" | "tool-call" | "tool-result"))
    {
        return None;
    }
    Some(last)
}

/// 丁T2 复审 N1：计划预期态下 POST 的**输入面判据**（纯函数，可测）。
///
/// `plan_pending` 为真时**只接受形态良好的 `dialog:<n>`**（`n` 可被 `u32` 解析——与
/// 端点 dialog 分支的 `n_str.parse::<u32>()` 同判据）；其余 id（映射表的
/// `approve`/`reject`/KV 定制 id，以及裸 `dialog:`/带空格的变体）一律拒绝。非预期态
/// 恒接受（既有路径零回归——`Waiting`/`标记` 态下 `plan_pending` 恒 false，见其计算式
/// 的短路守卫）。
///
/// **为什么必须同口径**：GET 在计划预期态下绝不下发映射键位（options 空），而 GET 的
/// options 是 POST 的**唯一合法输入面**。若 POST 仍接受映射 id，则 GET 的「绝不下发」
/// 只是前端可见面的自律，端点上留有一条键位通路（本仓库既有的 M-4 残余面的扩大）。
///
/// **本判据的充分性边界（如实申报）**：它只覆盖**输入形态**这一半——「该编号此刻真在
/// 屏幕选项表内」由 dialog 分支的**现场重解析**保证（`read_dialog_options` + 编号成员
/// 检查，防 GET→POST 之间对话框变化）。故本函数是**必要**条件而非充分条件
/// （`dialog:0` 形态合法但编号不在表内 → 端点仍拒）。抽成纯函数的理由：端点两种拒绝
/// 都收敛成 `no_mapping`，状态码分不开，不变式只能靠单测钉住。
fn plan_pending_accepts_option_id(plan_pending: bool, option_id: &str) -> bool {
    if !plan_pending {
        return true;
    }
    option_id
        .strip_prefix("dialog:")
        .is_some_and(|n| n.parse::<u32>().is_ok())
}

/// 对话框选项屏读（批次丙 T5；丁T3 起**改经 `RemoteState.dialog_probe` 缝**）。
///
/// 返回 None 的所有路径（调用方落回映射表二元卡——红线 3 降级）：
/// 非 Windows / 屏读失败 / 解析不出连续编号簇（<2 或 >9 项）。
///
/// **丁T3 改造理由（为什么不再直调 `windows_console::read_screen_window`）**：丁T3 的
/// 控制类注入守卫（[`crate::inject::dialog::blocks_control_injection`]）与本函数消费
/// 的是**同一份屏读结论**——两处各自直调 = 两份实现 + 两条降级链，而它们的松紧必须
/// 一致（「屏读见到簇」在审批侧意味着「可下发 dialog 选项」，在守卫侧意味着「拒绝控制
/// 类注入」；一处松一处紧就会出现「卡上有按钮但模式钮被拒」这类自相矛盾界面）。经缝
/// 之后只有一个屏读入口（生产实现 = `inject::dialog::probe_screen_dialog`），且测试可
/// 用假体驱动两条路径（真实屏读需要 conhost，CI 不可观测——参见 [`super::server::
/// DialogProbeFn`] 的缝理由）。
///
/// 屏读只对**已命中审批等待**的会话调用（调用点已判 hit），故不会对空闲会话白读一屏。
fn read_dialog_options(
    st: &Arc<RemoteState>,
    session: &crate::session::Session,
) -> Option<Vec<crate::inject::dialog::DialogOption>> {
    let opts = (st.dialog_probe)(session.id.as_str(), session.pid);
    if opts.is_none() {
        log::debug!("T5 屏读无对话框选项（pid={}）→ 降级二元卡", session.pid);
    }
    opts
}

/// **渲染等待版**的对话框屏读（E2①，裁18 通用规则「注入前屏读确认选项簇已渲染」）：
/// 单次屏读可能撞上重绘/未渲染（戊探E 定案：数字「无效」×5 的根因是注入早于渲染的
/// 假阴性）——在 [`crate::inject::timing::MENU_POLL_TOTAL_MS`] 窗内按
/// [`crate::inject::timing::POLL_STEP_MS`] 轮询，首个 `Some` 即返回（D20(a) 命中即停）；
/// 窗尽仍 `None` → `None`（照旧降级，不盲发数字）。
///
/// **仅 claude 数字路径消费**（渲染等待对数字档是正确性前提）；导航档不需要（不依赖
/// 渲染时序），且端点对已知失败形态不该白等一窗——故不替换 [`read_dialog_options`]
/// 的其他调用点。
fn read_dialog_options_render_wait(
    st: &Arc<RemoteState>,
    session: &crate::session::Session,
) -> Option<Vec<crate::inject::dialog::DialogOption>> {
    read_dialog_options_render_wait_with(
        || read_dialog_options(st, session),
        crate::inject::timing::poll_rounds(crate::inject::timing::MENU_POLL_TOTAL_MS),
    )
}

/// 渲染等待的**可注入内核**（评审修复抽出：`rounds` 参数化——门禁内小窗可测，
/// 不用真等 1.5s）。`probe` 返回首个 `Some` 即命中（D20(a) 命中即停）；窗尽仍
/// `None` → `None`（调用方照旧降级/拒绝——**不注入**，数字档的「未渲染不注入」面）。
fn read_dialog_options_render_wait_with(
    mut probe: impl FnMut() -> Option<Vec<crate::inject::dialog::DialogOption>>,
    rounds: u32,
) -> Option<Vec<crate::inject::dialog::DialogOption>> {
    let rounds = rounds.max(1);
    for i in 0..rounds {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(
                crate::inject::timing::POLL_STEP_MS,
            ));
        }
        if let Some(opts) = probe() {
            return Some(opts);
        }
    }
    None
}

/// 审批选项扫描（同步，spawn_blocking 内调用）：会话快照（数据同源，与看板同一份）→
/// Waiting 判定 → 映射表（KV 经 store 缝）按工具找映射（tool_id 匹配）→ **严格档判定
/// （probe-pending 恒不可批，未取证不出键）** → detect 命中判定。
/// 返回 `None` = 不可批（会话不存在 / 非 Waiting / 无映射——统一 false，不给存在性
/// 预言机）；`Some` = 映射存在，携带 detect 结果 / 选项表 / verifiedWith / tool /
/// 严格档 reason。
/// 复合键说明（Task 5 教训）：本端点无 agent_type 入参，快照里按 id 找**第一个**匹配；
/// id 跨工具撞名时取第一个匹配——契约如此。
fn approve_options_scan(st: &Arc<RemoteState>, session_id: &str) -> Option<ApproveScanHit> {
    let session = (st.session_source)()
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)?;
    let tool = session.agent_type.tool_id().to_string();
    // T4 红卡接铃铛：Waiting 门改「Waiting ∨ 等待标记」（issue #74 根因①——审批
    // 等待期文件推导判 processing，标记是钩子事件落的一等信号；经 store.with 传
    // 连接，测试内存库零接触真实 ~/.mam，生产 Global 与状态链写侧同库）
    let marked = st
        .store
        .with(|conn| crate::database::dao::approval_wait::has(conn, &tool, session_id));
    // ===== 丁T2：计划预期态（codex/kimi 计划确认类对话框的唯一入口）=====
    //
    // 问题 4 根因：codex 的 `<proposed_plan>` 落盘后**没有状态变化**（codex 不落
    // Waiting——`codex_parser` 的兜底红已废），于是详情页不知道「终端正在等一个计划
    // 确认」→ 无提示条、无入口去点「检查终端对话框」。修法：**从消息尾部派生预期态**
    // （无新存储，任务书硬要求）——尾部存在计划提案消息且其后无用户消息/工具事件。
    //
    // kimi 同样受益（问题 3）：它的计划审批有 `interaction.request → Waiting` 红灯，
    // 走既有 Waiting 门；但「Write this file?」变体（R1-3 实测：无 plan_review，只有
    // 计划文件卡）不落 Waiting——预期态把它一并接住。
    //
    // **短路序（必须在 Waiting 门之前）**：codex 计划待确认时状态不是 Waiting，先判
    // Waiting 门会直接 None 掉——那正是问题 4 的根因。
    //
    // **成本**：为守短路序，`tail_page` 的读取**提前**到 Waiting 门之前——但只对
    // [`plan_dialog_family`]（codex/kimi）提前；其余工具仍走原路径（门后才读，零额外
    // IO）。下方丁T1 的尾部待决问答判定消费**同一页**（一次读，两处判据）。
    let plan_family = plan_dialog_family(&tool);
    let early_page = if plan_family {
        (st.message_source)(&tool, session_id, QUESTION_SCAN_TAIL_LIMIT).ok()
    } else {
        None
    };
    // **计划对话框族专属**：尾部计划提案 → 预期态在场（其后无 user / 无工具事件）。
    // 其余工具恒 false（不消费该判据——改它们的门等于改既有行为，超出本批范围）。
    let plan_pending = plan_family
        && early_page
            .as_ref()
            .is_some_and(|p| plan_pending_tail_index(&p.messages).is_some());
    if session.status != crate::session::SessionStatus::Waiting && !marked && !plan_pending {
        return None;
    }
    // T8 硬约束①：问题等待标记在场 → 审批卡不可用（问答模式不出允许/拒绝——
    // 问题标记同样强制 Waiting，会话满足上方 Waiting 门，须显式排除；隔离用例
    // question_mark_never_triggers_approve_card，server.rs）
    let question_marked = st
        .store
        .with(|conn| crate::database::dao::question_wait::has(conn, &tool, session_id));
    if question_marked {
        return None;
    }
    let mapping = st
        .store
        .with(crate::inject::approve::load_mappings_conn)
        .into_iter()
        .find(|m| m.tool == tool)?;
    // ===== 丁T1 复评 F-2：硬约束① 的无标记分支（codex / opencode）=====
    //
    // 根因：标记分支（上方 question_marked）只对**有钩子通道**的工具成立——claude
    // 的 PreToolUse∧AskUserQuestion 落 question_wait_marks；**codex / opencode 没有
    // 钩子**，标记永不落库，于是它们的问答在场只由「状态链语义红」体现。而
    // `hit = marked || detect(mapping, last_message)` 的 detect 是**纯文本子串**命中，
    // 问答文本由模型自由生成——模型把问题写成 "Would you like to run the following…?"
    // 时 detect 即命中，审批卡会借语义红误出（评审 I-3；这是裁3/裁9 的安全面）。
    //
    // 修法：用与问答端点**同一份判据**（[`pending_question_tail_index`]）判定本会话
    // 是否有**待决问答**在尾部——有则审批不可用（与硬约束① 同语义，只是数据源从
    // hook 标记换成消息尾部形态）。
    //
    // **成本与短路顺序（F-2 要求写明）**：
    // - 该判定需一次 `message_source` 读页；`read_plan_for_approval`（T8 计划聚合）
    //   也要读——故本函数把两者**合并为同一次读**（`page` 只取一次，plan 聚合改为
    //   吃已取到的页；见下方 `plan_from_page`）。零额外 IO。
    // - **什么时候读**：只在 `hit` 候选成立时才需要（不命中审批时读页是白付）。
    //   但 `hit` 自身依赖 detect——我们不可能先知道 hit。收敛口径：**Waiting 门
    //   已过**（本函数开头，非 Waiting 且无标记已在 L1531 返回 None）才走到这里，
    //   即「会话确实在等用户」——此时读一页判「等的是问答还是审批」是必要代价
    //   （一页 = 40 条尾部窗口，与问答端点同量级；且审批卡本就是低频事件）。
    //   未过 Waiting 门的会话（绝大多数轮询）**不读页**。
    // - **与严格档的先后**：本判定放在严格档之前（probe-pending 是「键位未取证」
    //   的另一根轴，但问答在场时连提示条都不该出——审批卡整体不适用）。严格档
    //   短路序「在 detect 之前」的既有约束不受影响（本判定在其更前，且不消费
    //   detect 结果）。
    // - **按工具收窄（复评 F2-2）**：仅对**无问答标记通道**的工具启用——claude 有
    //   标记通道（`question_marked` 已在其前面拦下），且其「AUQ 被纯文本打断、
    //   tool_result 不落盘」形态会让本判据假阳性 → 排除；判据与依据见
    //   [`tool_lacks_question_mark_channel`]。
    //   丁T2：`tail_page` 的取用已提前到上方 Waiting 门处（计划预期态门需要它；
    //   两者**同一次读**，这里只消费已取到的页）。
    //
    //   ===== 丁T2 复评 F3-3（Important）：读页与「是否套用问答判据」**解耦** =====
    //
    //   根因（评审 git 史对照确凿）：`d8db990` 时 `tail_page` 是**无条件读页**；
    //   `ed1a868`（T1 复审补丁）把它收窄成「仅 `tool_lacks_question_mark_channel`」——
    //   **claude 恒 None** → `plan_body` 恒 None → 批次丙 T8 的 claude 计划聚合
    //   （ExitPlanMode 的 plan 正文进审批卡）**静默失效**。
    //
    //   修法：读页条件覆盖**两个消费方**，判据套用条件各自独立——
    //   - ① 读页条件 = 「需要问答尾部判据」∨「需要计划聚合」（`needs_question_tail
    //     || needs_plan_body`）；claude 靠后者重新拿到页；
    //   - ② **套用问答判据**的条件仍是 `tool_lacks_question_mark_channel`（T1 裁决的
    //     安全面，一字不退——claude 的「AUQ 被纯文本打断、tool_result 不落盘」假阳性
    //     形态不受影响）。
    let needs_question_tail = tool_lacks_question_mark_channel(&tool);
    let needs_plan_body = approve_plan_family(&tool);
    let tail_page = early_page.or_else(|| {
        if needs_question_tail || needs_plan_body {
            (st.message_source)(&tool, session_id, QUESTION_SCAN_TAIL_LIMIT).ok()
        } else {
            None
        }
    });
    if needs_question_tail {
        if let Some(page) = tail_page.as_ref() {
            if pending_question_tail_index(&page.messages).is_some() {
                log::debug!(
                    "审批端点：尾部存在待决问答（无标记分支，{tool}）→ 审批不可用（硬约束①）"
                );
                return None;
            }
        }
    }
    // 严格档（M9R Task 10 裁决：未取证不出键）：probe-pending 映射即使 Waiting+detect
    // 命中也压为不可批——选项不下发，只给降级原因（前端提示条）；drift 判定照常
    // （probe-pending 恒判漂移，提示条与 drift 提示并存不冲突）。
    // **短路序勿动**：本判定在 detect 之前，detect 未命中同样下发 reason——提示条语义
    // =键位取证状态（未取证），非「审批中」判定；若把短路「修」到 detect 之后，会让
    // detect-miss 的真审批会话完全无提示（键位随时可能被 KV 定制回 probe-pending）
    if mapping.verified_with == crate::inject::approve::PROBE_PENDING {
        return Some(ApproveScanHit {
            available: false,
            options: Vec::new(),
            verified_with: mapping.verified_with,
            tool,
            reason: Some(crate::inject::approve::PROBE_PENDING_REASON.to_string()),
            dialog: false,
            plan_body: None,
            // 严格档是「未取证不出键」，与「降级警示」不同轴——此处不给降级文案
            degraded_hint: None,
            // **口径记录（丁T2 复评 M3）**：严格档先于一切「出卡」判定——`plan_pending`
            // 在此**硬置 false**（而非透传上方算出的值）。含义：probe-pending 的映射
            // 即使尾部确有计划提案，也不出「计划待确认」条（available=false + reason 的
            // 提示条优先，与预期态条互斥）。这是**刻意的降级收敛**——前端 ApproveCard
            // 在 `available=false` 时只渲染提示条（`hintOnly` 分支先于 planPending
            // 分支返回），两条不会同屏；硬置 false 让载荷自身也与前端行为一致，
            // 避免「载荷说预期态在场、前端却按提示条渲染」的表里不一。
            // 影响面：仅「KV 定制把映射改成 probe-pending ∧ codex/kimi 有计划提案」
            // 这一组合——此时用户看到的是键位未取证提示，而不是计划待确认条。
            plan_pending: false,
        });
    }
    // T4：标记路径跳过 marker detect（钩子是一等信号，提示文本不落会话文件的
    // 平台上 detect 恒 miss——macOS 红卡由此可达）；marker detect 降级为无标记
    // 时的旧路径（Windows 屏读/文本命中形态）
    //
    // 丁T2：计划预期态也是**一等信号**（与标记同级）——预期态在场即视为「终端正在
    // 等这个计划确认」，跳过 detect（codex 计划提案的最后一轮消息是计划本体，审批
    // 框标题文本根本不落会话文件，detect 对它恒 miss——这与 macOS 上标记的必要性
    // 同构）。kimi 的计划框同理：wire 落 `interaction.request`，屏上标题
    // "Ready to build with this plan?" 不落任何文件。
    let hit = marked
        || plan_pending
        || session
            .last_message
            .as_deref()
            .map(|msg| crate::inject::approve::detect(&mapping, msg))
            .unwrap_or(false);
    // ===== 批次丙 T5：N 选项审批对话框屏读解析 =====
    //
    // 图2/图3 根因：审批对话框是 N 选一（claude 计划批准 1/2/3、codex Implement
    // this plan 1/2/3、kimi Ready to build 1=Approve/2=Reject/3=Revise），而映射表
    // 只建模二元 approve/reject → 全部降级二元卡、用户盲发数字碰运气。
    //
    // 修法：命中审批等待时**屏读可见窗口**（Windows；多行块，输入行尾读法读不到）
    // → 解析编号选项行 → 下发 N 选项（前端渲染编号按钮，点按注入该数字键）。
    //
    // **降级链（红线 3，任一步失败都不猜、不盲出键）**：
    // ① 非 Windows / 屏读失败 → 落回映射表二元卡；
    // ② 屏读成功但解析不出连续编号簇（<2 项或 >9 项）→ 同上回落；
    // ③ 解析成功且选项数 ≥ 2 → 下发对话框选项（**覆盖**映射表二元项——真实选项
    //    文本比「允许/拒绝」二元更准，这正是本任务的修复目标）。
    let dialog_options = if hit {
        read_dialog_options(st, &session)
    } else {
        None
    };
    // ===== 丁T2：kimi 的「无键位可发」防线（R1-1 安全裁决）=====
    //
    // kimi 的映射表 options 恒空（见 DEFAULT_MAPPINGS_JSON 注释：R1-1 实机证伪数字
    // 通道不可依赖；默认表不落任何键位；KV 定制可覆盖——届时 `mapping.options` 非空，
    // 本分支自然不再命中）。命中审批但没有对话框选项时分两态收敛：
    //
    // - **计划预期态在场**（Ready to build / Write this file? 两种计划确认框——R1-3
    //   实测两形态）→ 走与 codex **同一形态**：available=true + 零 options +
    //   `planPending=true` → 前端「计划待确认」条 +「检查终端对话框」按钮（用户点检查
    //   重试屏读；对话框刚绘制出来的窗口期一次重试就能拿到 N 选项）。比死胡同提示条
    //   多给一条**可操作路径**，且与 codex 的卡形态统一（两家的计划确认语义相同）。
    // - **无计划预期态**（如带标记的 Write/command 审批）→ 降级提示条
    //   （available=false + reason）：无键可发，指引去终端（§2.8）。
    //
    // **只在 kimi 落这条短路**（收窄口径）：claude/codex 的映射表有实证键位，通用降级
    // 路径（二元卡 + degradedHint / 计划预期态空选项）是它们既有的正确行为，不得改动。
    if tool == "kimi" && dialog_options.is_none() && !plan_pending {
        log::debug!("审批端点：kimi 命中审批但未读到对话框选项（无计划预期态）→ 不出键（R1-1 数字通道不可依赖）");
        // 计划正文**不下发**（F3-4：任务书要求 kimi 审批卡不含 plan 正文——正文由消息流
        // 的 kind="plan" 正文卡承担；见 [`approve_plan_family`] 的分工说明）
        return Some(ApproveScanHit {
            available: false,
            options: Vec::new(),
            verified_with: mapping.verified_with,
            tool,
            reason: Some(KIMI_NO_DIALOG_REASON.to_string()),
            dialog: false,
            plan_body: None,
            degraded_hint: None,
            // 无预期态的降级路径（上面条件已排除 plan_pending）
            plan_pending: false,
        });
    }
    // ===== R1-3：降级态必须**明确警示**（终审 Important）=====
    //
    // 计划红线 3 要求「解析失败降级二元卡 + **防重警示**」，但原实现只回落映射二元项、
    // 无任何警示。危害面实证：终端若是多选对话框而用户点了二元「允许」→ 注入的键
    // 可能错位命中**非预期选项**（幽灵标记误选选项 1 即此）。
    //
    // 警示条件：**命中审批（hit）但屏读/解析没拿到对话框选项** —— 即「终端可能正显示
    // 一个我们没读到的多选对话框」。此时下发 `degradedHint`，前端在二元卡上渲染脚注。
    //
    // 丁T2 收窄：**计划待确认形态不叠加本警示**（`!plan_pending_without_keys`）——该
    // 形态前端**零按钮**（没有可点错的二元键），警示文案「二元键可能错位」在那里是
    // 无的放矢；该形态自己的降级表达是「点检查未命中 → 脚注提示」（前端
    // `approve-plan-check-miss`）。两处提示不重复。
    let plan_pending_without_keys = plan_pending && dialog_options.is_none();
    let degraded = hit && dialog_options.is_none() && !plan_pending_without_keys;
    // 选项序列化只取 id+label（key 是投递层机密，不进任何 UI 载荷）；未命中 → options 空
    // （契约：available=false 一律不给选项，移动端据此不渲染审批卡）
    //
    // 丁T2 第三档：**计划预期态 ∧ 无对话框选项 ∧ kimi** 已在上面短路掉；剩下
    // 「计划预期态 ∧ 无对话框选项 ∧ codex」→ 映射表的 y/esc 是**补丁审批**键位
    // （M8R 0.154.0 实证），对计划框未取证 → 也不能拿它顶（见 [`plan_dialog_family`]）。
    // 该形态下发 available=true + 空 options + `planPending=true`：前端渲染「计划待确认」
    // 条 +「检查终端对话框」按钮（不是二元键）。
    let (options, dialog) = match dialog_options {
        Some(opts) => (
            opts.iter()
                .map(|o| (format!("dialog:{}", o.number), o.label.clone()))
                .collect::<Vec<_>>(),
            true,
        ),
        None => (
            if hit && !plan_pending_without_keys {
                mapping
                    .options
                    .iter()
                    .map(|o| (o.id.clone(), o.label.clone()))
                    .collect()
            } else {
                Vec::new()
            },
            false,
        ),
    };
    // ===== 批次丙 T8：审批点 plan 聚合 =====
    //
    // 问题 9：计划批准时 plan 内容与审批卡分离（计划在消息流/plan 文件里，批准卡孤立）
    // ——happy 的 ExitPlanMode View 以 plan markdown 为**卡片主体**，是标准答案。
    //
    // 聚合口径（按会话工具取源，全部走既有数据面，无新增读路径）：
    // - claude：消息流里最近的 `kind="plan"` 消息（T1 已把 ExitPlanMode input.plan
    //   升格为一等计划消息）；
    // - codex：同上（T4 已把 `<proposed_plan>` 标签消息升格——两类来源在消息层已统一
    //   为 kind="plan"，故本处**不需要**按工具分支）；
    // - kimi：**不下发**（丁T2 复评 F3-4——任务书成文要求「kimi 计划审批卡不含 plan
    //   正文」，正文由 §2.2 的 kimi 正文卡承担；见 [`approve_plan_family`]）。
    //
    // 只取**最近一条**（审批针对的是最新计划）；命中即随 available 载荷下发。
    // 丁T1 复评 F-2：计划聚合吃**上方已取到的同一页**（tail_page）——原先自读一页的
    // 写法会把消息读两次；读失败（None）与无计划同收敛（降级不阻塞审批）
    //
    // 丁T2 复评 F3-3：`approve_plan_family` 同时是**读页条件**的一半（见上方
    // `needs_plan_body`）——claude 因此重新拿到页（T8 聚合回归修复）。
    let plan_body = if hit && approve_plan_family(&tool) {
        tail_page.as_ref().and_then(|p| plan_from_page(&p.messages))
    } else {
        None
    };
    Some(ApproveScanHit {
        available: hit,
        options,
        verified_with: mapping.verified_with,
        tool,
        reason: None,
        dialog,
        plan_body,
        degraded_hint: if degraded {
            Some(DEGRADED_DIALOG_HINT.to_string())
        } else {
            None
        },
        plan_pending,
    })
}

/// 审批点 plan 聚合载荷（批次丙 T8）
struct ApprovePlanBody {
    /// 计划 markdown 全文（claude/codex：消息流 kind="plan" 消息）
    /// 或计划文件路径（kimi：T7 的 kind="plan-file" 卡）
    content: String,
    /// true = content 是**文件路径**（前端走预览读全文）；false = 直接是 markdown
    is_file: bool,
}

// 计划聚合原用独立的 APPROVE_PLAN_TAIL_LIMIT=40（与问答通道 B 同口径）；丁T1 复评
// F-2 把两者合并为**同一次读**后，窗口统一取 `QUESTION_SCAN_TAIL_LIMIT`（同为 40，
// 覆盖面不变：计划批准必然发生在计划渲染之后不久）。

/// 从**已取到的消息页**里找审批点关联的计划内容（批次丙 T8）：尾部窗口找最近一条
/// 计划类消息（`kind="plan"` 给 markdown 全文 / `kind="plan-file"` 给文件路径）。
///
/// 丁T1 复评 F-2：由原先自读一页的 `read_plan_for_approval`（**该函数已在本次重构中
/// 删除**，勿再引用）改为吃页的纯函数——审批端点现在把「待决问答判定」与「plan
/// 聚合」合并到**同一次** `message_source` 读（两次读同一份数据纯属浪费，且两次读
/// 之间会话可能变化导致两处结论不一致）。
///
/// 无计划消息 → None（前端不渲染计划主体，仅显示选项——降级不阻塞审批；
/// 这正是 happy「兜底渲染」原则的应用）。
///
/// **丁T2 补充：同一事件的「正文优先」**（kimi 专属形态）——丁T2 起 kimi 的一条
/// `interaction.request(plan_review)` 会**同时**产正文卡（`kind="plan"`）与文件卡
/// （`kind="plan-file"`，见 `content::kimi_plan_cards` 的固定顺序：正文在前）。
/// 原来的「取最近一条」此时会选中**后出的文件卡**（路径），把已经拿到的 markdown
/// 正文白白降级成路径提示。故：若最新的计划类消息是文件卡、且其**紧邻前一条**正是
/// 正文卡，则取正文（同一事件的两半，markdown 是更完整的那半）。
///
/// **为什么用「紧邻」而不是「窗口内任意正文」**：避免把**更早**的正文卡（上一版计划）
/// 与**最新**的文件卡（这一版计划）错配成一对——紧邻性是 `kimi_plan_cards` 的确定性
/// 产物，只有同一事件的两半才满足。
fn plan_from_page(msgs: &[crate::remote::content::SessionMessage]) -> Option<ApprovePlanBody> {
    // 从尾部向前找最近一条计划类消息
    let last = msgs
        .iter()
        .rposition(|m| matches!(m.kind.as_str(), "plan" | "plan-file"))?;
    // 同一事件「正文优先」：最新是文件卡 ∧ 紧邻前一条是正文卡 → 取正文
    if msgs[last].kind == "plan-file" && last > 0 && msgs[last - 1].kind == "plan" {
        return Some(ApprovePlanBody {
            content: msgs[last - 1].content.clone(),
            is_file: false,
        });
    }
    let m = &msgs[last];
    Some(ApprovePlanBody {
        content: m.content.clone(),
        is_file: m.kind == "plan-file",
    })
}

/// GET /m/api/v1/session-approve-options?session_id=（M8 审批选项卡数据源）：
/// 会话快照与映射解析在 spawn_blocking（扫描与 KV 读取均为同步阻塞调用）；
/// currentVersion 的 CLI 探测（首调 spawn `<cli> --version`）带**双重超时保护**：
/// 探测内核自身 3s 有界等待（灰1，超时结果入缓存），端点另有 5s tokio 护栏
/// （spawn_blocking 任务级，Task 10 评审提示：探测卡死不拖垮端点）——超时/任务异常
/// 按 None 处理并 log::warn。
pub async fn session_approve_options(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    let probe_st = st.clone();
    let scan =
        match tokio::task::spawn_blocking(move || approve_options_scan(&probe_st, &sid)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("session-approve-options 会话扫描任务异常: {e}");
                return json_no_store(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                );
            }
        };
    let (
        available,
        options,
        verified_with,
        tool,
        reason,
        dialog,
        plan_body,
        degraded_hint,
        plan_pending,
    ) = match scan {
        Some(hit) => (
            hit.available,
            hit.options,
            hit.verified_with,
            Some(hit.tool),
            hit.reason,
            hit.dialog,
            hit.plan_body,
            hit.degraded_hint,
            hit.plan_pending,
        ),
        // 不可批三态（无会话 / 非 Waiting / 无映射）同形：available=false，版本字段空，
        // 无 reason（前端按卡自隐处理，与严格档 reason 提示条区分）
        None => (
            false,
            Vec::new(),
            String::new(),
            None,
            None,
            false,
            None,
            None,
            false,
        ),
    };
    // CLI 版本探测（仅映射存在时；进程级缓存，首调一次 spawn）：5s 超时保护
    let current_version = match tool.as_deref() {
        Some(t) => {
            let probe_tool = t.to_string();
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                tokio::task::spawn_blocking(move || {
                    crate::inject::approve::cached_cli_version(&probe_tool)
                }),
            )
            .await
            {
                Ok(Ok(v)) => v,
                Ok(Err(e)) => {
                    log::warn!("CLI 版本探测任务异常（tool={t}）: {e}");
                    None
                }
                Err(_) => {
                    log::warn!("CLI 版本探测超时（tool={t}），currentVersion 按未知处理");
                    None
                }
            }
        }
        None => None,
    };
    // drift：映射存在时按 verifiedWith vs current（探测失败按 "unknown"）判定
    let drift = tool.is_some()
        && crate::inject::approve::is_version_drift(
            &verified_with,
            current_version.as_deref().unwrap_or("unknown"),
        );
    json_no_store(
        StatusCode::OK,
        serde_json::json!({
            "available": available,
            "options": options
                .iter()
                .map(|(id, label)| serde_json::json!({ "id": id, "label": label }))
                .collect::<Vec<_>>(),
            "verifiedWith": verified_with,
            "currentVersion": current_version,
            "drift": drift,
            // 严格档降级原因（M9R Task 10）：probe-pending 时为中文提示原文，其余 null
            // （前端 ApproveOptionsView.reason?: string，null 不触发提示条渲染）
            "reason": reason,
            // T5：选项来自对话框屏读（true → 前端渲染「对话框选项」风格 + 编号徽标；
            // id 形如 dialog:<n>，点按注入数字键 n）
            "dialog": dialog,
            // T8：审批点 plan 聚合（计划确认类审批卡主体聚合 plan 内容——happy 的
            // ExitPlanMode View 以 plan markdown 为卡片主体，用户不必再去消息流翻）
            // null = 无计划消息在场（前端只渲染选项，降级不阻塞审批）
            "plan": match &plan_body {
                Some(b) => serde_json::json!({ "content": b.content, "isFile": b.is_file }),
                None => serde_json::Value::Null,
            },
            // R1-3：降级警示（命中审批但未读到对话框选项）——前端在二元卡渲染脚注；
            // null = 未降级（读到对话框选项，或本就非审批态）
            "degradedHint": degraded_hint,
            // 丁T2：**计划待确认预期态**（消息尾部派生）——codex/kimi 的计划确认框不落
            // 状态/标记，这是它唯一的可见信号。前端据此渲染「计划待确认」条 +「检查终端
            // 对话框」按钮（`available=true` 且此字段 true 且 `dialog=false` → 空 options
            // 不是错误，是「还没读到选项，点检查重试」）。
            "planPending": plan_pending,
        }),
    )
}

/// POST /m/api/v1/session-approve 请求体（camelCase；字段全 default——缺参不触发
/// axum 提取器 422，由 handler 统一按契约给 400 bad_request）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionApproveReq {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub option_id: String,
}

/// POST /m/api/v1/session-approve（M8 红卡一键应答）：
/// - 缺参 → 400；设备 cookie 缺失 → 403 防御（gate 已拦，理论不可达）；
/// - 会话不在快照 → 404 no_session；非 Waiting → 409 not_waiting（映射解析在其后，
///   运行中会话不付出 KV 读取代价）；
///
/// **等待门（丁T2 F3-2 起）**：与 GET **同口径** = `Waiting ∨ 审批标记 ∨ 计划预期态`
/// （计划预期态按 `plan_dialog_family` 收窄到 codex/kimi；判据用与 GET 同一份
/// [`plan_pending_tail_index`]）。放宽的理由见该函数文档（codex 计划待确认的真机状态
/// 是 Idle + 无标记——只放宽 GET 会让卡片挂上后点按必 409）。
///
/// **计划预期态下的输入面（丁T2 复审 N1）**：`plan_pending` 为真时**只接受
/// `dialog:<n>`**，映射表 id（`approve`/`reject`/KV 定制的任意 id）一律 404 no_mapping
/// ——与 GET 的「该态绝不下发映射键位」对称（GET 的 options 是 POST 的唯一合法输入面）。
///
/// **残余面如实申报（丁T1 复评 M-4）**：本端点**不走 detect**（既有契约：客户端只
/// POST 它从 GET 拿到的 option id，而 GET 已把 detect / 待决问答两道门走过——见
/// `approve_options_scan`）。因此「旁路客户端**直发** POST + 合法 optionId」不在
/// 防线内：问答待决（语义红）或审批文本未命中的会话，理论上仍能被直发 POST 触发
/// 映射键位。前端唯一调用点是 ApproveCard（`available=false` 时零按钮、零 POST
/// 入口），正常路径不可达。若将来需要收口，落点是本 handler 的 Waiting 门之后加
/// 同款判定（代价：每次 POST 多一次消息读——目前刻意不做，见 T1 复评裁决）。
///
/// **M-4 残余的边界（N1 收口后）**：`plan_pending` 态的映射键位通路**已关闭**（N1）；
/// 残余面仍限「Waiting/标记态 ∧ 未命中 detect ∧ 旁路直发」这一组合——与 T1 复评时
/// 相比**未扩大**（F3-2 曾把它扩到计划预期态，N1 已收回）。
///
/// - 映射表无该工具映射 / optionId 无对应项 / probe-pending 严格档（M9R Task 10：
///   未取证不出键，按键位映射缺失处理）/ 计划预期态下的非 dialog id（N1）→ 404
///   no_mapping（降级提示走普通发送）；
/// - in-flight 守卫忙（flush 循环/直发/插队/detached 旧投递正在投递该会话）→ 200
///   failed 提示重试。守卫取在 spawn_blocking 闭包内（fff9c29 flush 循环事件臂同款，
///   F1 断连双投修复——handler 断连不再提前释放守卫）；忙让位无投递发生，不写审计
///   （既有口径：与 retract/jump 忙让位一致）；
/// - 投递 `injector.locate_and_send_key_spec(pid, key, spec)`（F2：spec = 该会话工具
///   的族规格，先 `families::family_for` 再 `FALLBACK_SPEC` 兜底，与 Task 5 try_flush
///   同款——KV 自定义映射若配方向键，B 族 codex 须 VK+scan 键形态，无族默认的 A 族
///   形态〔vk=0 字符流〕会被静默吞）：**不带 [mobile] 前缀**——按键非文本；
///   成功 → 200 key_sent；失败 → 200 failed{error}（可重试回执）。
///   // M11：会话路由含无头通道时，此处按 approve option 分派 control_message 而非按键
///   （选项卡 UI 与端点契约通道无关，届时只扩这一处分派）
/// - 审计：action=approve/reject 语义化，域外 id 收敛为 "key"（log::warn 留痕），
///   result=ok/failed:{e}，channel=injector.name()，content=optionId（协议标识，
///   stable 于展示文案定制）。
pub async fn session_approve(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionApproveReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    let option_id = req.option_id.trim().to_string();
    if sid.is_empty() || option_id.is_empty() {
        return bad_request();
    }
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // 会话查找 + Waiting 复核 + 映射/选项解析（一个 spawn_blocking：扫描与 KV 读取
    // 均为同步阻塞；session_source 与 store.with 顺序执行不嵌套——生产 session_source
    // 内部自取全局 DB 锁，嵌套即自锁死锁，M4 纪律）
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let probe_opt = option_id.clone();
    let lookup = match tokio::task::spawn_blocking(move || {
        // 复合键说明（Task 5 教训）：本端点无 agent_type 入参，快照里按 id 找第一个匹配；
        // id 跨工具撞名时取第一个匹配——契约如此
        let Some(session) = (probe_st.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == probe_sid)
        else {
            return Err("no_session");
        };
        // T4：Waiting 门改「Waiting ∨ 等待标记」（与 approve-options 同口径——
        // 审批等待期状态判 processing，标记经 store.with 点查）
        let tool = session.agent_type.tool_id().to_string();
        let marked = probe_st
            .store
            .with(|conn| crate::database::dao::approval_wait::has(conn, &tool, &probe_sid));
        // ===== 丁T2 复评 F3-2（Critical）：POST 门并入**计划预期态** =====
        //
        // 根因：GET 的门已放宽到「Waiting ∨ 标记 ∨ 计划预期态」，而 POST 仍只有
        // 「Waiting ∨ 标记」——codex 计划待确认 = **Idle + 无标记**（真机状态见
        // `plan_pending_tail_index` 文档），于是卡片挂上后**点 N 选项必回 409
        // not_waiting**（前端显示「会话不在等待状态」，用户点了等于没点）。
        //
        // 修法：与 GET **同一份判据、同一条收窄**（[`plan_pending_tail_index`] +
        // [`plan_dialog_family`]）——绝不复制粘贴出第二份实现（两处口径漂移正是本
        // 缺陷的成因模式）。
        //
        // **成本与短路序（评审要求写明）**：只在**计划对话框族**（codex/kimi）上读页，
        // 且只在「Waiting 门即将失败」时才读（`!marked && status != Waiting` 的短路
        // 前置条件在下方 `||` 求值顺序里天然成立——Rust 的 `||` 短路）：
        // `Waiting || marked || plan_pending` 中前两项为真时**不会**走到读页分支。
        // 非计划族工具（claude/opencode/…）恒 false，**零额外 IO**（T1 的「claude
        // 不受尾部判据影响」裁决同样保持——这里读的是计划预期态，不是问答判据）。
        let plan_pending = plan_dialog_family(&tool)
            && !marked
            && session.status != crate::session::SessionStatus::Waiting
            && (probe_st.message_source)(&tool, &probe_sid, QUESTION_SCAN_TAIL_LIMIT)
                .ok()
                .is_some_and(|p| plan_pending_tail_index(&p.messages).is_some());
        if session.status != crate::session::SessionStatus::Waiting && !marked && !plan_pending {
            return Err("not_waiting");
        }
        // T8 硬约束①：问题等待标记在场 → 审批应答不可用（409 not_waiting 同形收敛，
        // 键位永不出手——问答会话的键位面只有问答端点承载）
        if probe_st
            .store
            .with(|conn| crate::database::dao::question_wait::has(conn, &tool, &probe_sid))
        {
            return Err("not_waiting");
        }
        let mapping = probe_st
            .store
            .with(crate::inject::approve::load_mappings_conn)
            .into_iter()
            .find(|m| m.tool == tool);
        // 严格档（M9R Task 10 裁决：未取证不出键）：probe-pending 映射按键位映射缺失
        // 处理（与无映射同收敛 no_mapping 404，降级提示走普通发送）——未取证键位永不
        // 经本端点出手
        if mapping
            .as_ref()
            .is_none_or(|m| m.verified_with == crate::inject::approve::PROBE_PENDING)
        {
            return Err("no_mapping");
        }
        // ===== 批次丙 T5：对话框选项（id 形如 `dialog:<n>`）=====
        //
        // 屏读解析出的选项走**数字键**注入（n = 屏上编号）。安全性：只有「当前屏
        // 幕上确实存在该连续编号簇」才下发（GET 与 POST 同源解析——POST 现场重解析，
        // 防 GET→POST 之间对话框变化导致注入陈旧数字；重解析失败即拒，不盲发）。
        if let Some(n_str) = probe_opt.strip_prefix("dialog:") {
            let n: u32 = n_str.parse().map_err(|_| "no_mapping")?;
            // **现场重解析**（防 GET→POST 之间对话框变化导致注入陈旧键；同时导航
            // 需要**当前高亮位**——那是此刻屏幕上的状态，不能用 GET 时的快照）。
            // E2① 渲染等待（裁18，仅 claude 数字路径）：数字档的假阴性根因是「注入
            // 早于选项渲染」——claude 的重解析在 MENU_POLL_TOTAL_MS 窗内轮询，簇
            // 未渲染不注入；codex/kimi 维持单次读（导航档不依赖渲染时序，且避免
            // 端点对已知失败形态白等一窗）
            let digit_first = matches!(
                approve_dialog_keys(&tool),
                ApproveDialogKeys::DigitFirstWithVerify
            );
            // E2① 验证信号：数字档 → Some(n)（投递后屏读验证 + 导航回退）；其余档 None
            let digit_verify = if digit_first { Some(n) } else { None };
            let opts = if digit_first {
                read_dialog_options_render_wait(&probe_st, &session)
            } else {
                read_dialog_options(&probe_st, &session)
            }
            .ok_or("no_mapping")?;
            if !opts.iter().any(|o| o.number == n) {
                return Err("no_mapping");
            }
            // R1-1/R1-2：按对话框模型选键序档——claude/kimi 的计划批准类对话框
            // 数字键无效或不可依赖（可能误批准）→ 走导航确认；codex 数字有效 → 直选
            let keys = match approve_dialog_keys(&tool) {
                ApproveDialogKeys::DigitDirect => vec![n.to_string()],
                // E2① 数字优先：首段发数字（生效与否由投递段屏读验证 + 导航回退）
                ApproveDialogKeys::DigitFirstWithVerify => vec![n.to_string()],
                ApproveDialogKeys::NavigateConfirm => {
                    crate::inject::dialog::navigation_sequence(&opts, n).map_err(|e| {
                        log::debug!("R1 导航序列构造失败（{tool}）: {e}");
                        "no_mapping"
                    })?
                }
            };
            return Ok((
                session,
                tool,
                crate::inject::approve::ApproveOption {
                    id: probe_opt.clone(),
                    label: format!("对话框选项 {n}"),
                    // key 字段承载**多键序列**（逗号分隔的既有约定见 families 注释？
                    // 不——此处改为下方 seq 直传，key 保留首键仅为审计可读）
                    key: keys.join(","),
                },
                keys,
                digit_verify,
            ));
        }
        let Some(option) =
            mapping.and_then(|m| crate::inject::approve::option_by_id(&m, &probe_opt).cloned())
        else {
            return Err("no_mapping");
        };
        // ===== 丁T2 复审 N1（Important）：计划预期态下**只放行 dialog 选项** =====
        //
        // 契约不对称（复审读码 + 插桩实测确认）：GET 在计划预期态下**绝不下发映射键位**
        // （`plan_pending_without_keys` → options 空；kimi 分支同理），而 POST 越过
        // `not_waiting` 门后，非 `dialog:` 的 optionId 会落到这里取映射表键位——旁路
        // 客户端 POST `{optionId:"approve"}` 实测注入 `y`（codex 计划框的补丁审批键位）。
        // 修前该组合是 409（更安全）；F3-2 放宽门后变成了可达 → 属**本轮引入的面**，
        // 故本轮收口。
        //
        // 为什么 GET 与 POST 在这一态必须同口径：GET 的 options 是 POST 的**唯一合法
        // 输入面**（前端只 POST 它从 GET 拿到的 id）。GET 既然判定「此态无可安全下发的
        // 映射键位」（计划框的键位语义与映射表二元键不同——见 `approve_plan_family`
        // 文档），POST 就必须拒绝同类的映射键位 id；否则 GET 的「绝不下发」只是前端
        // 可见面的自律，端点上仍有一条键位通路（本仓库既有的 M-4 残余面的扩大）。
        //
        // 判据走纯函数 [`plan_pending_accepts_option_id`]（**判据即行为**，不是并行声明
        // ——端点两种拒绝都收敛成 `no_mapping`，从状态码无法区分，故把不变式抽出来
        // 让单测直接钉住）。`dialog:<n>` 在上方分支已 return，走到这里只可能是映射 id。
        if !plan_pending_accepts_option_id(plan_pending, &probe_opt) {
            log::debug!(
                "审批端点：计划预期态下不接受映射键位（{probe_opt}）→ no_mapping（N1 契约对齐）"
            );
            return Err("no_mapping");
        }
        let keys = vec![option.key.clone()];
        Ok((session, tool, option, keys, None))
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-approve 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let (session, tool, option, keys, digit_verify) = match lookup {
        Ok(v) => v,
        Err(code) => {
            let status = if code == "not_waiting" {
                StatusCode::CONFLICT
            } else {
                StatusCode::NOT_FOUND
            };
            return json_no_store(status, serde_json::json!({ "error": code }));
        }
    };
    // F2：按键按该会话工具取族规格（先 family_for 再 FALLBACK 兜底，与 Task 5
    // try_flush 同款）——族表未收录的工具（路由层已拦）走快消费者默认口径
    let spec = crate::inject::families::family_for(&tool)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    // M11：会话路由含无头通道时，此处按 approve option 分派 control_message 而非按键
    let injector = st.injector.clone();
    let pid = session.pid;
    let approve_sid = sid.clone();
    // E2①：数字优先验证信号（Some(n) = claude 数字档——发数字后屏读验证，未生效
    // 导航回退；来源=lookup 的 dialog 分支，非数字档恒 None）
    let digit_verify: Option<u32> = digit_verify;
    // 守卫与投递同生命周期于 spawn_blocking 闭包内（fff9c29 flush 循环事件臂同款，F1
    // 断连双投修复）：handler 断连（弱网/隧道掐断慢投递）不再提前释放守卫——detached
    // 投递全程占位，新触发取不到名额即让位。忙 → None 哨兵：让位不投递亦不落审计
    // （无投递发生不写审计——retract/jump 忙让位同口径）
    let attempt = tokio::task::spawn_blocking(move || {
        // `?` = 守卫忙（try_acquire_inflight 得 None）→ 闭包哨兵返回 None：让位不投递
        let _guard = crate::inject::queue::try_acquire_inflight(&approve_sid)?;
        // 逐键投递（导航确认是多键序列：↓×k + Enter）；首错即停
        // （半途失败不可盲目重试全序列——已发键已生效，前端按 failed 提示核对终端）
        let mut result = Ok(());
        for key in &keys {
            if let Err(e) = injector.locate_and_send_key_spec(pid, key, &spec) {
                result = Err(e);
                break;
            }
            // 导航键之间有间隔，给 TUI 重绘时间（实测 ↓ 后立即 Enter 偶有竞态）
            std::thread::sleep(std::time::Duration::from_millis(
                crate::inject::families::SUBMIT_DELAY_MS,
            ));
        }
        // ===== E2① 屏读验证 + 导航回退（仅 claude 数字档，且首段投递成功）=====
        if let (Some(n), true) = (digit_verify, result.is_ok()) {
            let (fallback_keys, send_err) = verify_digit_then_fallback(
                n,
                || crate::inject::dialog::probe_screen_dialog(pid),
                |key| injector.locate_and_send_key_spec(pid, key, &spec),
                || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::timing::POLL_STEP_MS,
                    ));
                },
                || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ));
                },
                crate::inject::timing::poll_rounds(
                    crate::inject::timing::DIGIT_VERIFY_POLL_TOTAL_MS,
                ),
            );
            match (fallback_keys, send_err) {
                (DigitVerifyOutcome::FellBack { keys }, Some(e)) => {
                    result = Err(e);
                    log::warn!(
                        "E2 导航回退中途发送失败（已发 {keys:?}）——请人工核对终端（pid={pid}）"
                    );
                }
                (DigitVerifyOutcome::Confirmed, None) => {
                    log::debug!("E2 数字直选生效（对话框已消失，pid={pid}）")
                }
                (DigitVerifyOutcome::FellBack { keys }, None) => {
                    log::info!("E2 数字直选未生效（对话框仍在），导航回退 {keys:?}（pid={pid}）")
                }
                (DigitVerifyOutcome::NavigationRefused(why), None) => {
                    log::warn!("E2 导航回退未出手（{why}）——请人工核对终端（pid={pid}）")
                }
                // 不可达组合（Confirmed/NavigationRefused 不发回退键→无 send_err）
                // ——穷尽性防御臂，出现即说明内核被误改
                (outcome, Some(e)) => {
                    log::warn!("E2 验证结论异常（{outcome:?}，{e}）——请人工核对终端（pid={pid}）");
                    result = Err(e);
                }
            }
        }
        Some(result)
    })
    .await;
    let sent = match attempt {
        Ok(Some(v)) => v,
        Ok(None) => {
            // in-flight 守卫忙（与 flush 循环/直发/插队共用）→ 让位，200 failed 提示
            // 重试（不双投；无投递发生故不写审计——忙让位同口径）
            return json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "failed",
                    "error": "投递进行中，请稍后重试"
                }),
            );
        }
        Err(e) => {
            log::error!("session-approve 投递任务异常: {e}");
            Err("内部任务异常".to_string())
        }
    };
    // 审计 action 词表（W5 词表 send|queue|flush|jump|retract|approve|reject|fail|key|open；
    // open = Task 11 一键 resume（session-open 端点），Task 7 的预留标注已兑现）：
    // approve/reject 语义化；域外 id（KV 定制表
    // 可含任意 id）不进词表——收敛为新增词表动作 "key" 并 log::warn 留痕，防自由文本
    // 污染审计 action 列（AuditLogSection 前端「原样小写展示」契约不受影响）
    let action = match option.id.as_str() {
        "approve" => "approve",
        "reject" => "reject",
        // R1-4：对话框选项（`dialog:N`）语义上是**批准动作**（用户在真实选项里选了一个），
        // 归 approve——与 C-23 文档「审计 action=approve、content=dialog:N」一致。
        // 原实现落 "key"+warn，属域外 id 兜底路径的误伤。
        d if d.starts_with("dialog:") => "approve",
        other => {
            log::warn!("审批动作域外 id：{other}，审计记 key");
            "key"
        }
    };
    match sent {
        Ok(()) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &option_id,
                action,
                "ok",
            );
            json_no_store(StatusCode::OK, serde_json::json!({ "status": "key_sent" }))
        }
        Err(e) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &option_id,
                action,
                &format!("failed:{e}"),
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({ "status": "failed", "error": e }),
            )
        }
    }
}

// ==== 历史会话区端点（spec 2026-09-20-mobile-archive-history §6.1）====

/// GET /sessions-archived 响应条目（camelCase）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedSessionDto {
    pub session_id: String,
    pub agent_type: String,
    pub project_path: String,
    pub project_name: String,
    pub title: Option<String>,
    pub last_status: String,
    pub last_seen_at: String,
    /// 软归档活会话标记（体验批二）：true = 看板隐藏中的活会话（APP 形态），
    /// 详情页动作是「移回看板」而非「在桌面端打开」
    pub hidden_alive: bool,
}

/// GET /m/api/v1/sessions-archived?days=1|3|7：懒加载归档列表。
/// days 非法夹取 1；活板同 id 查期排除（不删行）；last_seen 降序；
/// projects = 结果集内 distinct 项目名按该项目最大 last_seen 降序（下拉选项源）。
/// 时间比较一律 parse_from_rfc3339 解析后比（不裸串比较）；畸形时间行防御性排除。
pub async fn sessions_archived(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let days = match params.get("days").and_then(|s| s.parse::<i64>().ok()) {
        Some(3) => 3,
        Some(7) => 7,
        _ => 1,
    };
    let st2 = st.clone();
    let resp = tokio::task::spawn_blocking(move || {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        let live = (st2.session_source)();
        let live_ids: std::collections::HashSet<String> =
            live.sessions.iter().map(|s| s.id.clone()).collect();
        let mut items: Vec<(chrono::DateTime<chrono::Utc>, ArchivedSessionDto)> =
            (st2.archive_source)()
                .into_iter()
                .filter_map(|row| {
                    if live_ids.contains(&row.session_id) {
                        return None;
                    }
                    let seen = chrono::DateTime::parse_from_rfc3339(&row.last_seen)
                        .ok()?
                        .with_timezone(&chrono::Utc);
                    if seen < cutoff {
                        return None;
                    }
                    Some((
                        seen,
                        ArchivedSessionDto {
                            session_id: row.session_id,
                            agent_type: row.agent_type,
                            project_path: row.project_path,
                            project_name: row.project_name,
                            title: row.title,
                            last_status: row.last_status,
                            last_seen_at: row.last_seen,
                            hidden_alive: false,
                        },
                    ))
                })
                .collect();
        // 软归档活会话合成条目（体验批二）：看板隐藏集合 ∩ 活快照——会话还活着，
        // 历史页把它列出来（hiddenAlive 标记，详情页动作=移回看板），不列则手机上
        // 找不到任何入口找回它。畸形 last_activity_at 按当前时间兜底（排序占位）
        let hidden: std::collections::HashSet<String> =
            (st2.board_hidden_ids)().into_iter().collect();
        for s in &live.sessions {
            if !hidden.contains(&s.id) {
                continue;
            }
            let seen = chrono::DateTime::parse_from_rfc3339(&s.last_activity_at)
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            items.push((
                seen,
                ArchivedSessionDto {
                    session_id: s.id.clone(),
                    agent_type: s.agent_type.tool_id().to_string(),
                    project_path: s.project_path.clone(),
                    project_name: s.project_name.clone(),
                    title: s.title.clone(),
                    last_status: format!("{:?}", s.status).to_lowercase(),
                    last_seen_at: s.last_activity_at.clone(),
                    hidden_alive: true,
                },
            ));
        }
        // hiddenAlive 排最前（活会话优先处理），其余按 last_seen 降序
        items.sort_by_key(|(seen, dto)| (!dto.hidden_alive, std::cmp::Reverse(*seen)));
        // 项目聚合：distinct 项目名，按该项目条目最大 last_seen 降序
        let mut best: Vec<(String, chrono::DateTime<chrono::Utc>)> = Vec::new();
        for (seen, dto) in &items {
            match best.iter_mut().find(|(n, _)| *n == dto.project_name) {
                Some(e) => {
                    if e.1 < *seen {
                        e.1 = *seen;
                    }
                }
                None => best.push((dto.project_name.clone(), *seen)),
            }
        }
        best.sort_by_key(|b| std::cmp::Reverse(b.1));
        (
            items.into_iter().map(|(_, dto)| dto).collect::<Vec<_>>(),
            best.into_iter().map(|(n, _)| n).collect::<Vec<_>>(),
        )
    })
    .await;
    match resp {
        Ok((archived, projects)) => json_no_store(
            StatusCode::OK,
            serde_json::json!({ "archived": archived, "projects": projects }),
        ),
        Err(e) => {
            log::error!("sessions-archived 任务异常: {e}");
            json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            )
        }
    }
}

/// DELETE /m/api/v1/sessions-archived?session_id=… 或 ?all=1：手动管理
/// （spec 裁决 8：只增不删+手动）。归档管理不写 write_audit（W5 词表为注入动作域）。
pub async fn sessions_archived_delete(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let all = params.get("all").is_some_and(|v| v == "1");
    let sid = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if !all && sid.is_none() {
        return bad_request();
    }
    let target: Option<String> = if all { None } else { sid };
    let deleted = (st.archive_delete)(target.as_deref());
    json_no_store(
        StatusCode::OK,
        serde_json::json!({ "ok": true, "deleted": deleted }),
    )
}

// ==== 看板关闭/软归档端点（2026-09-20 体验批二）====

/// {sessionId} 请求体（camelCase；close/hide/unhide 三端点共用）
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActionReq {
    #[serde(default)]
    pub session_id: String,
}

/// POST /session-close {sessionId}：CLI 会话硬杀（桌面 kill_session 同内核）。
/// pid 取自活快照——id 本机口径，不回显远端输入（resume.rs 安全口径同源）；
/// App 形态不支持杀（会话寄生宿主 APP 共进程）→ 软归档走 /session-hide。
/// 审计：会话命中即写 close（ok/failed，词表 +close——远程杀进程是敏感动作）；
/// 未命中不落账（无可 acting 对象，口径同「校验失败不落账」）。
pub async fn session_close(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionActionReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() {
        return bad_request();
    }
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    let st2 = st.clone();
    let sid2 = sid.clone(); // 移动副本进闭包，原值保留给审计（session-open 同一写法）
                            // Err 携带 (工具串, 原因)：工具串供审计（活快照命中才有，no_session 为空串）
    let outcome: Result<(String, ()), (String, String)> = tokio::task::spawn_blocking(move || {
        let Some(s) = (st2.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == sid2)
        else {
            return Err((String::new(), "no_session".to_string()));
        };
        let tool = s.agent_type.tool_id().to_string();
        if matches!(s.form, crate::session::ProcessForm::App) {
            return Err((tool, "form_not_supported".to_string()));
        }
        match (st2.session_close)(s.pid) {
            Ok(()) => Ok((tool, ())),
            Err(e) => Err((tool, e)),
        }
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("session-close 任务异常: {e}");
        Err((String::new(), "internal".to_string()))
    });
    let audit = |tool: &str, result: &str| {
        // 审计走 store 内的 DB 连接（record_conn）：与 session-open 同一落账通道，
        // 端点测试经 store.with(recent_conn) 可回读（全局 record() 会写真实 ~/.mam）
        st.store.with(|conn| {
            crate::database::dao::write_audit::record_conn(
                conn,
                chrono::Utc::now().timestamp_millis(),
                &device_id,
                &device_name,
                tool,
                &sid,
                "process",
                "close",
                "远程关闭终端进程",
                result,
            );
        });
    };
    match outcome {
        Ok((tool, ())) => {
            audit(&tool, "ok");
            json_no_store(StatusCode::OK, serde_json::json!({ "ok": true }))
        }
        Err((tool, reason)) => match reason.as_str() {
            "no_session" => json_no_store(
                StatusCode::NOT_FOUND,
                serde_json::json!({ "error": "no_session" }),
            ),
            "form_not_supported" => {
                audit(&tool, "failed");
                json_no_store(
                    StatusCode::BAD_REQUEST,
                    serde_json::json!({ "error": "form_not_supported" }),
                )
            }
            other => {
                audit(&tool, "failed");
                log::error!("session-close 杀进程失败: {other}");
                json_no_store(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({ "error": "internal" }),
                )
            }
        },
    }
}

/// POST /session-hide {sessionId}：APP 形态软归档 =「叉」（看板隐藏，不杀进程、
/// 可逆）。**任意状态可归档**（叉不挑颜色；2026-09-20 修订：原绿态门 + 自动回归
/// 判据与 W4 持久未读冲突，归档 3s 内必被拉回，废弃）。与桌面端「叉」同源：
/// 触发 unread_mark_read（删未读池行 + 已读 tombstone），未读不再把卡片拉回；
/// 恢复唯一路径 = 历史页「移回看板」（/session-unhide）。CLI 会话走 /session-close。
pub async fn session_hide(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionActionReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() {
        return bad_request();
    }
    let Some(_) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    let st2 = st.clone();
    let outcome: Result<(), (String, String)> = tokio::task::spawn_blocking(move || {
        let Some(s) = (st2.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == sid)
        else {
            return Err((String::new(), "no_session".to_string()));
        };
        if !matches!(s.form, crate::session::ProcessForm::App) {
            return Err((String::new(), "form_not_supported".to_string()));
        }
        let tool = s.agent_type.tool_id().to_string();
        // 与桌面端「叉」同源：删未读池行 + 已读 tombstone——未读是 24h 持久标记，
        // 不消费它会被看板过滤外的任何「未读即活动」逻辑反复拉回
        (st2.unread_mark_read)(&tool, &s.id);
        (st2.board_hidden_hide)(&s.id);
        Ok(())
    })
    .await
    .unwrap_or_else(|e| {
        log::error!("session-hide 任务异常: {e}");
        Err((String::new(), "internal".to_string()))
    });
    match outcome {
        Ok(()) => json_no_store(StatusCode::OK, serde_json::json!({ "ok": true })),
        Err((_, reason)) => {
            let status = match reason.as_str() {
                "no_session" => StatusCode::NOT_FOUND,
                "form_not_supported" => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            json_no_store(status, serde_json::json!({ "error": reason }))
        }
    }
}

/// POST /session-unhide {sessionId}：解除软归档（移回看板）。幂等：不在隐藏集
/// 也返回 200（removed=0）；若会话已死，其归档行照常在历史页可见（语义无损）。
pub async fn session_unhide(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionActionReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() {
        return bad_request();
    }
    let Some(_) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    let st2 = st.clone();
    let removed = tokio::task::spawn_blocking(move || (st2.board_hidden_unhide)(&sid))
        .await
        .unwrap_or(0);
    json_no_store(
        StatusCode::OK,
        serde_json::json!({ "ok": true, "removed": removed }),
    )
}

// ==== 批次乙 T8：问答端点（session-question / session-question/answer）====
// 契约（JSON camelCase；Json 响应带 no-store——门禁下私有写/读路径）：
//   GET  /session-question?session_id= → 200 {available, questions:[{header, question,
//        multiSelect, options:[{label, description}]}], source("mark"|"scan"|null)}
//        available 判定走**双通道**（实机取证 2026-09-21 探测档案：pending 的
//        tool_use 不落盘，问题 UI 弹出期间会话 JSONL 停在 user 消息，纯文件路径
//        实时不可达——任务书原识别路径「等待态 + last_message 为 AUQ 调用」不可用，
//        最小偏差双通道，主控裁决）：
//        - **通道 A（主，实时）**：hook 事件——PreToolUse∧AskUserQuestion 经 helper
//          问答通道落「问题等待标记」（question_wait_marks，tool_input 随标记落库）；
//          标记在场即 available（跳过状态判定，T4 审批标记同款一等信号口径），
//          载荷优先取标记 payload；
//        - **通道 B（兜底）**：无标记 / 标记无有效载荷 → 扫描会话消息尾部，取最后
//          一条 AskUserQuestion tool-call：其**之后不再有 tool-result** 才视为未答
//          可用（可行口径——「标记/状态已退」需要跨轮状态，端点单次快照内以
//          tool-result 在场为答完判据并注释；pending 未落盘场景通道 B 天然不可见，
//          不误触发）→ 解析 tool_args 出 questions。历史进入页面 / 钩子未信任安装
//          即此通道的价值面。
//        审批等待标记在场 → **恒不可用**（硬约束①：审批标记不得触发问答卡）。
//        questions.length>1：available=true 但前端按只读卡渲染（翻页键序未测——
//        探测档案「结论不超证据」），注入面由 answer 端点拒绝。
//   POST /session-question/answer body {sessionId, action:"select"|"toggle"|
//        "submit"|"cancel", index?} → 200 {"status":"key_sent"} | 200 failed{error}
//        （注入失败 / in-flight 忙让位，可重试回执）| 409 no_question（问答不在场，
//        含会话不在快照与审批标记隔离——统一不给存在性预言机）| 409
//        multi_questions（多问题只读，不出手）| 400 bad_request（缺参/域外 action/
//        select|toggle 缺 index）| 400 bad_index（序号越界 / submit 用在单选题）。
//        注入序列（探测定案，见 inject::question 模块注释）：select/toggle → 数字单键
//        （单选数字即提交**无回车无 Esc**；多选数字=切换勾选）；submit → down ×(n+1)
//        → enter → '1'（三段式——Enter 当提交是反直觉反例，实测抓获）；cancel → esc。
//        序列逐键走 `injector.locate_and_send_key_spec(pid, key, spec)`（族规格同
//        approve 端点 F2 口径）；全程持有 in-flight 守卫（复用 queue 的
//        try_acquire_inflight——防与 flush/直发/审批对同一会话双投）。
//        审计：action=answer（词表追加），content=动作摘要（select#2 / submit / …），
//        result=ok/failed:{e}；忙让位/校验失败不落审计（无投递发生，approve 同口径）。
// 锁纪律（M4）：DB 读取并入 store.with 短临界区；session_source 与 store.with 顺序
// 执行不嵌套（approve 端点同款）。

/// 通道 B 的消息尾部扫描窗口（条）：问答 tool-call 是回合尾部事件，40 条足够覆盖
/// 回答前的往返且远小于 read_recent_lines 的 512KB 预算
const QUESTION_SCAN_TAIL_LIMIT: usize = 40;

/// 该工具的问答键序是否未验（只读档，批次丙 T3）：未验工具的任何应答动作都拒绝，
/// 端点据此回 409 `tool_readonly`（前端渲染只读卡 + 引导终端作答），与参数错误
/// （400 bad_index）分诊开。判据单一事实源在 `inject::question::question_key_profile`
/// （claude/opencode/kimi/codex 四家均已实测升格；zcode/dsh/workbuddy/openclaw 未验）
fn question_profile_is_read_only(tool: &str) -> bool {
    crate::inject::question::question_key_profile(tool)
        == crate::inject::question::QuestionKeyProfile::ReadOnly
}

/// 问答扫描产物（可用时载荷）
struct QuestionScanHit {
    questions: Vec<crate::inject::question::Question>,
    /// "mark" = 通道 A（hook 标记载荷）；"scan" = 通道 B（会话消息兜底）
    source: &'static str,
}

/// 问答扫描内核（同步，spawn_blocking 内调用；返回 None = 问答不在场，不给存在性
/// 预言机之外的信息）。双通道 + 隔离的完整口径见上方端点契约注释。
fn question_scan_sync(
    st: &Arc<RemoteState>,
    session_id: &str,
) -> Option<(crate::session::Session, QuestionScanHit)> {
    let session = find_session_sync(st, session_id)?;
    let tool = session.agent_type.tool_id().to_string();
    // 硬约束①：审批等待标记在场 → 问答卡不可用（审批会话的键位面只有审批端点承载）
    let approval_marked = st
        .store
        .with(|conn| crate::database::dao::approval_wait::has(conn, &tool, session_id));
    if approval_marked {
        return None;
    }
    // ===== 丁T2 扩面：kimi 的审批/问答互斥（无标记分支）=====
    //
    // 缺口（批次丙的隔离规则只覆盖标记面）：kimi 的审批**未必落标记**——`~/.mam/mam.db`
    // 的 `approval_wait_marks` 至今只有 claude 一行，因为 kimi 的 helper 部署/进程在场
    // 都是前提（B2 勘察结论）。而 kimi 的计划审批在 wire 里是**一等结构化事件**
    // （`interaction.request(kind=approval)`，丁T2 已映射为消息流的 plan 卡）——
    // 它的在场必须与标记同效地压掉问答卡，否则会出现「同一屏既出审批选项又出问答选项」
    // 的双卡错位（裁9 的安全面）。
    //
    // 判据：与审批端点**同一族**的纯函数，但**用更严的那一档**
    // （[`plan_pending_strict_tail_index`]，丁T2 复评 F3-5）——见该函数的文档：
    // 问答卡的压制方向是**可用性受损**，误报代价高于审批侧，故只认「正文卡（kind="plan"）
    // 在场」或「kimi 的 approval 事件源」，**不认孤立的 plan-file 卡**。
    // 只在 kimi 上启用（问题 3 的实际形态）；codex 留待有实证再扩。
    if tool == "kimi" {
        // 与审批端点同样的读页代价（一页 40 条）；失败 = 无判据 = 不拦（fail-open 到
        // 既有行为，不改问答端点的可达性——读失败时问答端点本来也拉不到通道 B 数据）
        let page = (st.message_source)(&tool, session_id, QUESTION_SCAN_TAIL_LIMIT).ok();
        if let Some(page) = page.as_ref() {
            if plan_pending_strict_tail_index(&page.messages).is_some() {
                log::debug!(
                    "问答端点：kimi 尾部存在计划待确认预期态（严判据）→ 问答卡不可用（审批优先）"
                );
                return None;
            }
        }
    }
    // 通道 A（主）：hook 问题标记——载荷优先取标记 payload（30s TTL 事件文件已被
    // 覆盖写，payload 是标记写入时刻的定格，读它免竞态）
    if st
        .store
        .with(|conn| crate::database::dao::question_wait::has(conn, &tool, session_id))
    {
        let payload = st
            .store
            .with(|conn| crate::database::dao::question_wait::payload_of(conn, &tool, session_id));
        if let Some(qs) = payload
            .as_deref()
            .and_then(crate::inject::question::parse_questions)
        {
            return Some((
                session,
                QuestionScanHit {
                    questions: qs,
                    source: "mark",
                },
            ));
        }
        // 标记在场但 payload 缺失/不可解析（helper 旧版 / 64KB 截断丢弃）→ 落通道 B
    }
    // 通道 B（兜底）：会话消息尾部找最后一条**问答形态** tool-call，且其后无结果。
    //
    // **判据从工具名改为 args 形态（批次丙 T3）**：原实现按
    // `tool_name == "AskUserQuestion"` 精确匹配——claude 独有。实测（2026-09-21
    // 问卷交互跨工具矩阵 research/refs/phase2-消息注入/2026-09-20-问卷交互跨工具
    // 矩阵.md）：codex 的 `request_user_input`、opencode 的 `question` **工具名不同
    // 但 args 形态相同**——顶层 `questions[]` 且元素含 question+options[]
    //（codex rollout：function_call.arguments 字符串，09:30:45 pending 即落盘；
    // opencode SQLite part：state.input 同形）。故判据 = 「args 可被
    // `parse_questions` 解析」，工具名只作日志线索（形态判据天然接住三家，也接住
    // claude 未来的改名）。
    //
    // 风险边界（如实申报）：形态判据**只看结构**——任何工具若恰好也带
    // `{"questions":[{question,options[]}]}` 形状的入参都会被判为问答。这是刻意的
    // 取舍：① 该形态在本机四家矩阵里是问答工具独有的（无已知碰撞）；② 误判后果
    // 是「出一张只读问答卡」，而漏判后果是「JSON 裸奔、用户无法远程作答」——前者
    // 轻于后者。若后续出现碰撞，收窄点在 `parse_questions` 的结构要求上。
    //
    // 丁T1 复评 F-2：判定抽为 [`pending_question_tail_index`]（审批端点**复用同一
    // 判据**做硬约束①的「无标记通道工具」分支——见
    // [`tool_lacks_question_mark_channel`]；claude 有标记通道，不叠加本判据）
    let page = (st.message_source)(&tool, session_id, QUESTION_SCAN_TAIL_LIMIT).ok()?;
    let last = pending_question_tail_index(&page.messages)?;
    // 形态判据（工具名只作日志线索——codex/openclaw 等改名不影响接住）
    log::debug!(
        "问答通道 B 形态命中: tool={:?}（工具名不参与判据）",
        page.messages[last].tool_name
    );
    let qs = crate::inject::question::parse_questions(page.messages[last].tool_args.as_deref()?)?;
    Some((
        session,
        QuestionScanHit {
            questions: qs,
            source: "scan",
        },
    ))
}

/// 会话消息尾部「**待决问答**」判定（丁T1 复评 F-2 抽出的可复用纯函数）：
/// 返回最后一条问答形态 tool-call 的下标——其 args 可被
/// `crate::inject::question::parse_questions` 解析（形态判据，工具名不参与），
/// **且其后无任何 tool-result**（已答判据）；两个条件任一不满足 → None。
///
/// 抽出的动机：硬约束①「问答在场 → 审批不可用」原先只查 hook 问题标记
/// （`question_wait::has`），而 **codex / opencode 没有钩子通道**——标记永不落库，
/// 这两家的问答在场只能靠本函数识别（同一份消息尾部扫描）。问答端点与审批端点
/// 共用本函数 = 两处口径永不漂移（F-2 的核心诉求）。
///
/// 答完判据（可行口径，注释申报）：tool-call 之后**任何** tool-result 在场即视为
/// 已答（AUQ 的 tool_result 无论正常作答/自由文本/Esc 拒绝都会落盘——探测档案 §3
/// 三形态；对应消息粒度比对需回读会话全文，快照尾部窗口内「其后无任何 tool-result」
/// 是保守充分的替代口径）。其后无 tool-result + tool-call 已落盘 = 未答在场
///（pending 不落盘的场景通道 B 天然不可见，不误触发——空闲态注入风险由双通道
/// 的在场判定收敛，残余风险见探测档案 K11 讨论）。
///
/// 代价：调用方需先取一次消息页（`message_source`）——审批端点把它与
/// `plan_from_page` 合并为同一次读（见 `approve_options_scan`）。
pub(crate) fn pending_question_tail_index(
    msgs: &[crate::remote::content::SessionMessage],
) -> Option<usize> {
    let last = msgs.iter().rposition(|m| {
        m.kind == "tool-call"
            && m.tool_args
                .as_deref()
                .is_some_and(|a| crate::inject::question::parse_questions(a).is_some())
    })?;
    if msgs[last + 1..].iter().any(|m| m.kind == "tool-result") {
        return None;
    }
    Some(last)
}

/// 该工具是否**没有**问答 hook 标记通道——即「问答在场」只能靠消息尾部形态识别。
/// 审批端点用本函数把丁T1 新增的尾部判据门**按工具收窄**（复评 F2-2）。
///
/// **为什么必须收窄（评审 Important-2 根因）**：未收窄前该门对所有工具生效，而
/// claude 本来就有标记通道（`adapter::apply_hook_event_to_session` 对
/// PreToolUse/PermissionRequest/Notification ∧ tool_name==AskUserQuestion 写
/// `question_wait_marks`；端点更早的 `question_marked` 检查已承担硬约束①）。
/// 给 claude 再叠一道尾部判据不仅冗余，还有**明确的假阳性面**：claude 存在
/// 「AUQ 被纯文本打断、`tool_result` 永不落盘」的真实形态（评审在真实库找到
/// `0c41365d-…`：line 38 纯文本作答、line 30 的 AUQ 永无 result）——此时尾部判据
/// 会把早已不在场的 AUQ 当成「待决」，其后的真审批会被静默压成
/// `available=false, reason=null`（前端自隐：用户既看不到按钮也看不到提示）。
/// 故 **claude 排除**（它走既有 `question_marked` 隔离路径）。
///
/// **判定依据（本机核实，逐条给证据）**——判据是「有无**问答标记**通道」，注意
/// 与「有无 hooks」不是同一问题：codex/kimi 都有 hooks，但都没有「问答」这个语义
/// 的钩子事件。写标记的唯一入口是 `adapter::is_question_entry_event`，其
/// **工具名判据恒等于 `AskUserQuestion`**（claude 的官方工具名，常量
/// `hook_listener::ASK_USER_QUESTION_TOOL`）：
/// - **codex → 纳入**：问答工具名是 `request_user_input` ≠ `AskUserQuestion`，
///   判据恒 false。本机 `~/.mam/events/` 实证：82 个事件文件中唯一带
///   `tool_name=request_user_input` 的那条是 `PreToolUse`（codex 注册面），
///   走通用分支，不产 `QuestionEntry`。
/// - **opencode → 纳入**：`hook_supported()` 恒 false（无 hooks）。
/// - **kimi → 纳入**：kimi 确有 hooks（`hook_events()` = PermissionRequest /
///   PermissionResult），payload 也**确实**携带 `tool_name`（本机 `~/.mam/events/`
///   5 条 kimi 形态事件实测：`tool_name ∈ {Write, ExitPlanMode}`，另有一条空串）——
///   但 kimi 的 `[[hooks]]` 注册面**只有这两个事件**，而写问题标记要求事件名 ∈
///   {PreToolUse, PermissionRequest, Notification} **且** tool_name ==
///   `AskUserQuestion`。kimi 的问答工具在 wire 里**恰好就叫 `AskUserQuestion`**
///   （本机 544 个 wire.jsonl 全库扫描：`tool.call.name == "AskUserQuestion"` **39 次**，
///   且 39/39 都有配对 `tool.result`），理论上 PermissionRequest(AUQ) 能命中判据——
///   但**本机 `question_wait_marks` 表为空**（0 行，`mam.db` 实证），即从未落过 kimi
///   问题标记。原因是 PermissionRequest 的实际语义边界：kimi 的 `AskUserQuestion`
///   是**会话内交互工具（interaction.request）而非权限工具**——本机 wire 全库
///   39 次 AUQ 都未被 PermissionRequest 事件覆盖（kimi 的事件只覆盖
///   Write/ExitPlanMode 这类需权限的工具）。
///   故 kimi 的问答在场**同样只能靠尾部形态识别** → 纳入。
///   **保守取向**：即便将来 kimi 对 AUQ 也发 PermissionRequest（标记通道打通），
///   纳入本门只是多一道冗余判据（`question_marked` 已经在更前面拦下），
///   而漏纳会留下「问答在场时审批误出」的安全缺口——两害相权取纳入。
/// - 其余工具（workbuddy / zcode / dsh / openclaw）无 hook 问答通道 → 纳入
///   （它们的问答形态即使未来出现，也同样只能走尾部识别）。
fn tool_lacks_question_mark_channel(tool: &str) -> bool {
    tool != "claude"
}

/// GET /m/api/v1/session-question?session_id=（T8 问答卡数据源）：扫描在
/// spawn_blocking（会话扫描/消息读取均为同步阻塞调用）。不可用（无会话/审批标记
/// 隔离/双通道均未命中）→ available=false + questions 空（前端卡自隐，approve 同构）
pub async fn session_question(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    let probe_st = st.clone();
    let scan = match tokio::task::spawn_blocking(move || question_scan_sync(&probe_st, &sid)).await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-question 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let (questions, source, tool_id) = match scan {
        Some((session, hit)) => (
            hit.questions,
            hit.source,
            session.agent_type.tool_id().to_string(),
        ),
        None => (Vec::new(), "", String::new()),
    };
    // T3：工具键序档（前端据此决定渲染可作答按钮还是只读卡）——
    // `answerable=false` 时前端渲染只读卡 + 引导终端作答（zcode/dsh 等未实测工具；
    // codex 已于 2026-09-21 实机补测升格为可作答）
    let answerable = !tool_id.is_empty() && !question_profile_is_read_only(&tool_id);
    // **丁T5 §2.4**：自由作答（卡内输入框）的支持面——只有 claude 的自由作答序列
    // 已实机定案。前端据此决定「渲染输入框 + 作为回答发送」还是「渲染引导文案去终端」
    // （§2.8 降级：序列未定案的工具**不假装能发**）。
    // 独立于 `answerable` 的理由：codex/opencode 的**点选**已实测（answerable=true）
    // 但**自由作答**未定案——两者是不同的能力面，不能用一个布尔表示。
    let free_text_supported = crate::inject::question::free_text_supported(&tool_id);
    // **批次戊 E4-E6 + 2026-09-24**：多题交互能力（kimi K-5 / codex Tab 备注 /
    // opencode tab 切页 / claude Next+回车切题——用户实机取证 2.1.278）。
    let multi_question = matches!(tool_id.as_str(), "kimi" | "codex" | "opencode" | "claude");
    // **切换题目**（2026-09-23 错位修复 + 2026-09-24 claude 接入）：多题卡多选题的
    // 显式切页动作——opencode（tab=前向切页，戊探A ①定案）与 claude（走位到尾部
    // 推进行 `Next` + 回车 + 读屏分类，阶段机 run_advance_stages）。kimi/codex 的
    // 多题切页键未验 → 旗标 false，前端对这两家的多选题渲染「请到终端切题」引导。
    // 单题卡无页可切，恒 false。
    let advance = matches!(tool_id.as_str(), "opencode" | "claude") && questions.len() > 1;
    json_no_store(
        StatusCode::OK,
        serde_json::json!({
            "available": !questions.is_empty(),
            "answerable": answerable,
            // 前端契约：`freeText` 缺省按 false 处理（旧后端不识别则走降级文案）
            "freeText": free_text_supported,
            // E4-E6：多题交互旗标（缺省按 false → 只读卡）
            "multiQuestion": multi_question,
            // 切换题目能力旗标（缺省按 false → 不渲染切换钮，旧后端前向兼容）
            "advance": advance,
            "questions": questions
                .iter()
                .map(|q| serde_json::json!({
                    "header": q.header,
                    "question": q.question,
                    "multiSelect": q.multi_select,
                    "options": q
                        .options
                        .iter()
                        .map(|o| serde_json::json!({
                            "label": o.label,
                            "description": o.description,
                        }))
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
            // 识别通道（诊断用；不可用时 null）
            "source": if source.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(source)
            },
        }),
    )
}

/// POST /m/api/v1/session-question/answer 请求体（camelCase；字段全 default——缺参
/// 不触发 axum 提取器 422，由 handler 统一按契约给 400 bad_request）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionQuestionAnswerReq {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub action: String,
    /// 选项序号（0 起 wire 口径；UI 编号从 1 起，前端换算）。select/toggle 必填
    #[serde(default)]
    pub index: Option<usize>,
    /// 自由作答文本（丁T5 §2.4 入口 1；仅 action="freeText" 消费，其余动作忽略）。
    /// 走 `inject::normalize::normalize_newlines` 归一（注入通道唯一出口的安全面）
    /// 后再进阶段机——**不带** `[mobile]` 签名（签名语义是「一条新消息」，作答文本
    /// 不是消息，加签名会污染答案）。
    #[serde(default)]
    pub text: Option<String>,
    /// **题目序号**（批次戊 E4-E6 多题交互：0 起，缺省 0）——多题卡逐题作答时
    /// 指定 select/toggle 作用在哪一题的选项表上。单题卡缺省即第 0 题（旧客户端
    /// 零破坏）；越界 → 400 bad_index。
    #[serde(default)]
    pub question_index: Option<usize>,
}

/// 问答阶段机的**阶段名**（回执里回报「走到哪一段停住」，前端据此显示进度/中止原因）。
/// 取值是 `inject::question` 编排各段的**稳定标识**（与中止文案里的段名同源）。
pub const QUESTION_STAGE_SUBMIT_ROW: &str = "submit-row";
/// 见 [`QUESTION_STAGE_SUBMIT_ROW`]
pub const QUESTION_STAGE_REVIEW: &str = "review";
/// 见 [`QUESTION_STAGE_SUBMIT_ROW`]
pub const QUESTION_STAGE_CONFIRM: &str = "confirm";
/// 见 [`QUESTION_STAGE_SUBMIT_ROW`]
pub const QUESTION_STAGE_RECEIPT: &str = "receipt";
/// 见 [`QUESTION_STAGE_SUBMIT_ROW`]
pub const QUESTION_STAGE_FREE_ROW: &str = "free-row";
/// 见 [`QUESTION_STAGE_SUBMIT_ROW`]
pub const QUESTION_STAGE_FREE_TEXT: &str = "free-text";
/// claude 多选**闭环切勾**段的段名（2026-09-24：`run_toggle_stages` 的中止段名——
/// 前端 `QUESTION_STAGE_LABELS` 同步收词）。命名对齐既有「-row」走位段风格。
pub const QUESTION_STAGE_TOGGLE: &str = "toggle-row";
/// claude 多题**切题**段的段名（2026-09-24：`run_advance_stages` 的段名——前端
/// `QUESTION_STAGE_LABELS` 同步收词）。
pub const QUESTION_STAGE_ADVANCE: &str = "advance";

/// 屏读轮询步长（毫秒）——D20 起五路共用一处（原名 `MENU_POLL_STEP_MS`，已随常量族迁移）
use crate::inject::timing::POLL_STEP_MS;
/// **阶段机的轮询预算**（毫秒；单段）——**定义已迁至** [`crate::inject::timing`]
/// （D20 的单一事实源），此处只 re-export 保持既有引用点不破。
/// 实测依据与「为什么不随三窗调整」的理由见 `timing::QUESTION_STAGE_POLL_TOTAL_MS` 的文档。
/// **每段的轮询步长**同为 `timing::POLL_STEP_MS`（100ms，五路同节奏）。
pub use crate::inject::timing::QUESTION_STAGE_POLL_TOTAL_MS;

/// POST /m/api/v1/session-question/answer（T8 问答应答；**丁T5 起提交/自由作答走阶段机**）：
/// 参数校验 → 会话/隔离/双通道复核（spawn_blocking）→ **按动作分派**：
///
/// - `select` / `toggle` / `cancel`：**单键直发**（既有语义不变——一次动作一个键，
///   无后续阶段可推进，故无需阶段机）；序列由 `inject::question` 构造；
/// - `submit`（多选提交，§2.3 裁4）：**阶段机闭环**
///   [`crate::inject::question::run_submit_stages`]——提交屏在场 → 闭环走位到 Submit 行
///   → 回车 → **屏读确认 Review 屏** → 抄屏上编号确认 → 屏读核验终态。任一段不符即
///   中止且不再发键（**禁止**盲发序列后显示「已发送按键」）；
/// - `freeText`（自由作答，§2.4 裁3）：**阶段机闭环**
///   [`crate::inject::question::run_free_text_stages`]——屏读定位 `Type something` 行
///   → 发定位数字（仅移动焦点）→ 复核 → **文本走字符通道** → 回车。仅 claude 支持
///   （其余工具序列未实测 → 409 `tool_readonly`，前端降级「请在终端作答」）。
///
/// # 回执形态（丁T5 起；**前向兼容**）
///
/// `status` 保持既有三词（旧前端只认这三个，故**不新增 status 值**）：
/// - `key_sent`：按键已投递到终端。**单键动作**（select/toggle/cancel）用它，语义与
///   批次丙一致；**阶段机动作**（submit/freeText）走完全链后用 `key_sent` + 新增
///   `done:true` 标记（见下）；
/// - `failed`：投递失败 / 忙让位（可重试）——**含阶段机中止**：`error` 是带段名的
///   中文文案（用户看得懂卡在哪一段、为什么），`aborted:true` + `stage` 供新前端
///   精确渲染（旧前端按 `failed` 展示 `error` 文案，行为不变）。
///
/// **新增字段全部是「附加」而非「改写」**（前向兼容的做法）：
/// | 字段 | 类型 | 何时出现 | 旧前端 |
/// |---|---|---|---|
/// | `stage` | string | 阶段机动作的中止回执 | 忽略（读 `error` 文案即可） |
/// | `aborted` | bool | 阶段机动作的中止回执（`true`） | 忽略（`status=failed` 已够） |
/// | `done` | bool | 阶段机动作走完全链（`true`） | 忽略（`status=key_sent` 已够） |
/// | `verified` | bool \| null | 阶段机动作的终态回执核验：`true`=屏读到终态锚；
///   `false`=读到屏但未见锚；`null`=读屏不可用（**均不谎报完成**） | 忽略 |
///
/// 为什么**不**加 `status:"in_progress"` 之类的新值：`status` 是旧前端的分支键
/// （`key_sent` / `failed` 两分支），加新值会让旧前端落进 else → 弹错误或卡在忙碌态。
/// 「进行中」是**前端的呈现态**（请求未返回期间显示进行中），不需要后端下发状态——
/// 后端这一趟是同步阻塞的（投递全过程在 handler 内完成才返回）。
///
/// 审计：action=`answer`（既有词，不新增），content=`answer::<stage>` 形态——
/// 阶段机的**段名进审计摘要**（「走到哪」是事后排查的关键信息），result=`ok` /
/// `aborted:<stage>` / `failed:<e>`。
pub async fn session_question_answer(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionQuestionAnswerReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    let action = match crate::inject::question::AnswerAction::parse(req.action.trim()) {
        Some(a) => a,
        None => return bad_request(),
    };
    // select/toggle 必带序号；freeText 必带非空文本；submit/cancel 不消费附加参数
    if matches!(
        action,
        crate::inject::question::AnswerAction::Select
            | crate::inject::question::AnswerAction::Toggle
    ) && req.index.is_none()
    {
        return bad_request();
    }
    // 自由作答文本：空/纯空白 → 400（阶段机也有同样的守卫，但入口先拦能省一次会话扫描
    // ——且 400 比 200 failed 更符合「参数就不对」的语义）
    let free_text: Option<String> = if action == crate::inject::question::AnswerAction::FreeText {
        let raw = req.text.as_deref().unwrap_or_default();
        // **两道判空**（顺序要紧）：
        // ① 原文 trim 后为空 → 用户没输入任何可见字符（含「只敲了回车/空格」）→ 400。
        //    必须先判原文：归一会把换行变成**字面 `\n` 两字符**，只看归一产物的话
        //    「只敲了一个回车」会变成一段「合法的可见文本」被当成答案发出去——
        //    那不是用户的意思。
        if raw.trim().is_empty() {
            return bad_request();
        }
        // ② 归一（注入通道唯一出口的安全面）：换行 → 字面 `\n`、剥 C0/DEL/C1。
        //    之后**不加** `[mobile]` 签名——作答文本不是消息。
        let norm = crate::inject::normalize::normalize_newlines(raw);
        // 归一**后**再判一次：纯控制字符输入（如只有 ESC）归一会把它剥光 → 空
        if norm.trim().is_empty() {
            return bad_request();
        }
        Some(norm)
    } else {
        None
    };
    if sid.is_empty() {
        return bad_request();
    }
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // 会话查找 + 双通道复核 + 序列构造（一个 spawn_blocking；锁纪律同 approve）
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let lookup = match tokio::task::spawn_blocking(move || {
        let Some((session, hit)) = question_scan_sync(&probe_st, &probe_sid) else {
            return Err("no_question");
        };
        let tool_id = session.agent_type.tool_id();
        // 「结论不超证据」→ 批次戊 E4-E6 + 2026-09-24 更新：kimi（K-5 数字直选+自动
        // 推进+Review 汇总屏）/ codex（数字即答+Tab 备注+末题 submit all）/ opencode
        // （tab 切页+Confirm 页提交）/ claude（空格切勾+Next 回车切题——用户实机取证
        // 2.1.278）的多题形态定案 → **交互放行**；其余工具维持只读（本分支是直调
        // API 的兜底防线）
        let multi_question = hit.questions.len() > 1;
        if multi_question && !matches!(tool_id, "kimi" | "codex" | "opencode" | "claude") {
            return Err("multi_questions");
        }
        // claude 的切题动作（2026-09-24）：阶段机（走位到 Next+回车+分类），只对
        // 多题载荷有意义——单题载荷上的 advance 是参数错（400，防误触走位提交）
        if action == crate::inject::question::AnswerAction::Advance
            && tool_id == "claude"
            && !multi_question
        {
            return Err("bad_request");
        }
        // 多题：select/toggle 作用在 `questionIndex` 指定的题（0 起缺省 0；越界 400）
        let q_idx = req.question_index.unwrap_or(0);
        if q_idx >= hit.questions.len() {
            return Err("bad_index");
        }
        let q = hit.questions.into_iter().nth(q_idx).unwrap_or_else(|| {
            // unreachable（上面已判界内），防御性占位——序列构造会因选项越界拒绝
            crate::inject::question::Question {
                header: String::new(),
                question: String::new(),
                multi_select: false,
                options: Vec::new(),
            }
        });
        // T3：按**会话工具**分发键序（claude 全键序 / opencode·kimi 单选已验 /
        // codex 等未验工具只读——「未验不出键」，见 question_key_profile 的表）。
        // **丁T5**：先过**可用性门**（`action_supported`——submit/freeText 的键序
        // 依赖屏读，没有静态序列，但「支不支持」仍要判），再对**单键动作**取序列
        // （submit/freeText 取到的会是 Err，那正是「无静态序列」的表达——它们走阶段机）
        // **多题 submit 走专用门**（2026-09-23）：它作用于整张问卷而非 q_idx 指定的
        // 那一题——单题防呆「submit 仅用于多选题」在多题载荷下会把「第 1 题是单选」
        // 的问卷提交误拒 400，故按工具档直判（kimi/opencode 放行）
        if multi_question && action == crate::inject::question::AnswerAction::Submit {
            crate::inject::question::multi_question_submit_supported(tool_id).map_err(|e| {
                log::debug!("问答动作不可用（{tool_id}/多题 submit）: {}", e.reason());
                match e {
                    crate::inject::question::ActionRefusal::BadParameter(_) => "bad_index",
                    crate::inject::question::ActionRefusal::ToolUnverified(_) => "tool_readonly",
                }
            })?;
        } else {
            crate::inject::question::action_supported(tool_id, action, req.index, &q).map_err(
                |e| {
                    // 拒绝码分两档（见 `ActionRefusal`）：参数问题 → 400 bad_index（改参数
                    // 即可重试）；工具未实测 → 409 tool_readonly（前端渲染只读卡引到终端）
                    log::debug!("问答动作不可用（{tool_id}/{action:?}）: {}", e.reason());
                    match e {
                        crate::inject::question::ActionRefusal::BadParameter(_) => "bad_index",
                        crate::inject::question::ActionRefusal::ToolUnverified(_) => {
                            "tool_readonly"
                        }
                    }
                },
            )?;
        }
        // 单键动作才取静态序列；阶段机动作（submit/freeText/**claude 的 toggle 与
        // advance**）序列留空（由编排产生）。claude toggle 的静态数字路径 2026-09-24
        // 废止（用户实机推翻 K8）——切勾走 run_toggle_stages（空格 + 闭环 + 屏读校验）；
        // claude advance 走 run_advance_stages（走位到 Next + 回车 + 分类）。
        // **E4② kimi 多题 DigitAdvance**（A3 禁令）：多题形态的 Select = `[数字]`
        // （直选+自动推进下一题，**禁尾 Enter**——Enter 会误作用下一题），
        // 不走单题的两段式 `[数字, enter]`
        let seq = match action {
            crate::inject::question::AnswerAction::Submit
            | crate::inject::question::AnswerAction::FreeText => Vec::new(),
            crate::inject::question::AnswerAction::Toggle if tool_id == "claude" => Vec::new(),
            crate::inject::question::AnswerAction::Advance if tool_id == "claude" => Vec::new(),
            crate::inject::question::AnswerAction::Select
                if tool_id == "kimi" && multi_question =>
            {
                crate::inject::question::kimi_digit_advance_keys(req.index, &q).map_err(|e| {
                    log::debug!("问答键序不可用（kimi 多题）: {e}");
                    "bad_index"
                })?
            }
            _ => crate::inject::question::answer_key_sequence_for(tool_id, action, req.index, &q)
                .map_err(|e| {
                log::debug!("问答键序不可用（{tool_id}）: {e}");
                if question_profile_is_read_only(tool_id) {
                    "tool_readonly"
                } else {
                    "bad_index"
                }
            })?,
        };
        Ok((session, seq, q, tool_id))
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-question/answer 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let (session, sequence, q_for_plan, q_tool) = match lookup {
        Ok(v) => v,
        Err(code) => {
            // 校验失败零注入零审计（approve guards 同口径）：
            // - no_question（409）：会话不在快照 / 审批标记隔离 / 双通道均未命中
            //   ——统一 409（不给存在性预言机；审批标记在场时本就不得出问答键）
            // - multi_questions（409）：多问题只读（探测未测面不出手）
            // - tool_readonly（409，T3）：该工具问答键序未实测（zcode/dsh 等）→ 只读卡
            // - bad_index（400）：select/toggle 序号越界 / submit 用在单选题
            // - bad_request（400）：域外用法（claude 单题载荷上的 advance——切题只对
            //   多题有意义，防误触走位提交）
            let status = if code == "bad_index" || code == "bad_request" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::CONFLICT
            };
            return json_no_store(status, serde_json::json!({ "error": code }));
        }
    };
    // F2：序列按键按该会话工具取族规格（先 family_for 再 FALLBACK 兜底）——"down"
    // 的 A/B 族形态分发（claude=VT 序列 / codex=VK 键）由 spec 承载
    let tool = session.agent_type.tool_id().to_string();
    let spec = crate::inject::families::family_for(&tool)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    let injector = st.injector.clone();
    let pid = session.pid;
    let answer_sid = sid.clone();
    // 阶段机计划（走位上限依赖选项数——在会话扫描之后才有，故在此构造）
    let stage_plan = StagePlan::for_action(action, req.index, &q_for_plan, q_tool);
    let probe_st2 = st.clone();
    // `tool` 进闭包（分派器要按工具取规格与日志），审计用的副本另留一份
    let tool_for_dispatch = tool.clone();
    // 切勾目标身份核验的题干（评审 I1）：多题卡当前题的 question 文本，随计划
    // 传给分派器（run_toggle_stages 第 1 段与屏面题干比对，不一致=已手动切题→中止）
    let expected_question = q_for_plan.question.clone();
    // 守卫与投递同生命周期于 spawn_blocking 闭包内（fff9c29 同款）：忙 → None 哨兵
    // 让位，不投递亦不落审计（无投递发生，approve/retract/jump 忙让位同口径）
    let attempt = tokio::task::spawn_blocking(move || {
        let _guard = crate::inject::queue::try_acquire_inflight(&answer_sid)?;
        Some(dispatch_question_action(
            &probe_st2,
            &tool_for_dispatch,
            pid,
            &spec,
            injector.as_ref(),
            &sequence,
            free_text.as_deref(),
            &stage_plan,
            &expected_question,
        ))
    })
    .await;
    let outcome = match attempt {
        Ok(Some(v)) => v,
        Ok(None) => {
            // in-flight 守卫忙（与 flush 循环/直发/审批共用）→ 让位，200 failed 提示
            // 重试（不双投；无投递发生故不写审计——忙让位同口径）
            return json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "failed",
                    "error": "投递进行中，请稍后重试"
                }),
            );
        }
        Err(e) => {
            log::error!("session-question/answer 投递任务异常: {e}");
            QuestionDispatch::Internal("内部任务异常".to_string())
        }
    };
    let audit_label = action.audit_label(req.index);
    match outcome {
        QuestionDispatch::KeySent { stage } => {
            // 审计摘要：单键动作保持批次丙的 `select#2` / `submit` 形态**逐字不变**
            // （既有用例与审计页的读法都依赖它）；阶段机动作才追加 `::<stage>`
            // （段名进审计是事后排查的关键信息）。用 `match` 而不是
            // `format!("{}::{}", stage.unwrap_or(..))`，正是为了让「单键不追加」这件事
            // 在代码上直接可读。
            let audit_content = match stage {
                None => audit_label.clone(),
                Some(s) => format!("{audit_label}::{s}"),
            };
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &audit_content,
                "answer",
                "ok",
            );
            let mut body = serde_json::json!({ "status": "key_sent" });
            if let Some(s) = stage {
                // 阶段机动作：附加字段（旧前端忽略；见端点文档的表）
                body["done"] = serde_json::json!(true);
                body["stage"] = serde_json::json!(s);
            }
            json_no_store(StatusCode::OK, body)
        }
        QuestionDispatch::StageDone {
            stage,
            receipt_seen,
        } => {
            // 阶段机走完整条闭环：status 仍是 key_sent（旧前端语义不变），但带上
            // done/stage/verified——`verified` 的三态见端点文档（**不谎报完成**）
            let result = match receipt_seen {
                Some(true) => "ok",
                Some(false) => "ok:receipt-unseen",
                None => "ok:receipt-unverifiable",
            };
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &format!("{audit_label}::{stage}"),
                "answer",
                result,
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "key_sent",
                    "done": true,
                    "stage": stage,
                    "verified": receipt_seen,
                }),
            )
        }
        QuestionDispatch::ToggleDone { checked, verified } => {
            // 切勾闭环：status=key_sent + checked/verified——前端用 `checked` 同步
            // 本地勾选态（屏读真值，替代「盲翻本地 Set」）；verified=false 表示
            // 键已发出但无法屏读核验（不谎报成功）
            let result = if verified {
                "ok"
            } else {
                "ok:toggle-unverified"
            };
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                format!("{audit_label}::{}", QUESTION_STAGE_TOGGLE).as_str(),
                "answer",
                result,
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "key_sent",
                    "done": true,
                    "stage": QUESTION_STAGE_TOGGLE,
                    "checked": checked,
                    "verified": verified,
                }),
            )
        }
        QuestionDispatch::AdvanceDone { advanced } => {
            // 切题闭环：status=key_sent + advanced（false = 已在 Review 屏零按键——
            // 前端**不**推进，停在确认卡）
            let result = if advanced { "ok" } else { "ok:already-review" };
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                format!("{audit_label}::{}", QUESTION_STAGE_ADVANCE).as_str(),
                "answer",
                result,
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "key_sent",
                    "done": true,
                    "stage": QUESTION_STAGE_ADVANCE,
                    "advanced": advanced,
                }),
            )
        }
        QuestionDispatch::Aborted { stage, error } => {
            // **中止**：与 failed 同槽（status=failed），但带 aborted/stage——前端能
            // 精确渲染「进行到哪一段停住」，旧前端按 failed 的 error 文案走（不变）
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &format!("{audit_label}::{stage}"),
                "answer",
                &format!("aborted:{stage}"),
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "failed",
                    "aborted": true,
                    "stage": stage,
                    "error": error,
                }),
            )
        }
        QuestionDispatch::Failed(error) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &audit_label,
                "answer",
                &format!("failed:{error}"),
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({ "status": "failed", "error": error }),
            )
        }
        QuestionDispatch::Internal(error) => json_no_store(
            StatusCode::OK,
            serde_json::json!({ "status": "failed", "error": error }),
        ),
    }
}

/// 阶段机动作的**静态计划**（端点侧构造、传给分派器；纯数据，可测）。
///
/// 存在的意义：把「这个动作要不要走阶段机、走位上限多少」这条判据从 handler 的分支
/// 里提出来。走位上限依赖**选项数**，而选项数在会话扫描之后才知道——故计划在扫描后
/// 构造（`for_action`），但它的形状（哪三档）与选项数无关，写在一处便于测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagePlan {
    /// 单键动作（select/toggle/cancel）：一次投递一个键，无后续阶段
    SingleKey,
    /// 多选提交阶段机；`max_down_steps` = 走位上限（选项数 + 2）
    Submit { max_down_steps: usize },
    /// 自由作答阶段机
    FreeText,
    /// **kimi 多选/多题提交阶段机**（批次戊 E4）：Review 在场判读 → tab →
    /// Review 汇总屏 → 屏上编号确认 → 终态
    KimiSubmit,
    /// **kimi Other 自由作答阶段机**（批次戊 E4）：Other 行数字 → 打字 → 回车保存
    /// → Review 汇总屏 → 确认
    KimiFreeText,
    /// **codex Tab 备注阶段机**（批次戊 E5）：弹窗 footer 锚判读 → Tab → 打字 →
    /// Enter 提交（当前高亮项+备注）→ 终态
    CodexNotes,
    /// **opencode own answer 阶段机**（批次戊 E6）：行序定位 → enter 开行 →
    /// 裸打字守卫（屏读确认占位行）→ 打字 → enter 提交
    OpencodeOwnAnswer,
    /// **opencode 多选提交阶段机**（批次戊 E6，2026-09-23 接线）：首段屏读
    /// 「Confirm 已在场则跳过 tab」→（不在场才 tab）→ Confirm 页 → enter 提交
    OpencodeSubmit,
    /// **claude 多选闭环切勾阶段机**（2026-09-24）：屏读定位 → 方向键走位（每步
    /// 复核）→ 空格 → 屏读校验翻转。`target` = 0 起的模型选项下标（数字路径已被
    /// 用户实机推翻，切勾必须经此编排）
    ClaudeToggle { target: usize },
    /// **claude 多题切题阶段机**（2026-09-24）：走位到尾部推进行（多题为 `Next`）
    /// + 回车 + 读屏分类（下一题页 / Review 确认屏）。`max_steps` = 走位上限
    ClaudeAdvance { max_steps: usize },
}

impl StagePlan {
    /// 按动作取计划。`index` 仅对单键动作有意义（这里不看它——它已在参数校验里消费）。
    ///
    /// **走位上限 = 选项数 + 2** 的推导：多选界面的行序是「模型选项 1..n」→
    /// TUI 追加的 `Type something` 行（第 n+1 行）→ `Submit` 行（第 n+2 行）。
    /// 从第 1 行最多需要 n+1 次 ↓ 到 Submit 行；再加 1 次容错（重绘竞态下一次按键
    /// 未生效的情形不会白跑——上限只是**死循环兜底**，正常路径在每步复核里提前停手）。
    fn for_action(
        action: crate::inject::question::AnswerAction,
        index: Option<usize>,
        q: &crate::inject::question::Question,
        tool: &str,
    ) -> Self {
        use crate::inject::question::AnswerAction as A;
        match (action, tool) {
            // 2026-09-24：claude 多选切勾走阶段机（数字路径废止）——target 由请求
            // 的 index 给（参数校验已保证 toggle 必带 index，此处兜底 0 不可达）
            (A::Toggle, "claude") => Self::ClaudeToggle {
                target: index.unwrap_or(0),
            },
            // 2026-09-24：claude 多题切题走阶段机（走位上限与 submit 同推导——
            // 走位目标是尾部推进行，行序同为「选项 1..n + Type something + 推进行」）
            (A::Advance, "claude") => Self::ClaudeAdvance {
                max_steps: q.options.len() + 2,
            },
            // E4：kimi 的两条阶段机（键序依赖屏读，由编排产生）
            (A::Submit, "kimi") => Self::KimiSubmit,
            // E6：opencode 多选提交阶段机（2026-09-23 接线——此前 opencode 的 Submit
            // 误落下面的 claude Submit 行走位形态，屏读判据在 opencode 屏上必失败）
            (A::Submit, "opencode") => Self::OpencodeSubmit,
            (A::FreeText, "kimi") => Self::KimiFreeText,
            (A::FreeText, "codex") => Self::CodexNotes,
            (A::FreeText, "opencode") => Self::OpencodeOwnAnswer,
            (A::Submit, _) => Self::Submit {
                max_down_steps: q.options.len() + 2,
            },
            (A::FreeText, _) => Self::FreeText,
            // Advance 是单键纯导航（tab），与 select/toggle/cancel 同通道
            (A::Advance | A::Select | A::Toggle | A::Cancel, _) => Self::SingleKey,
        }
    }
}

/// 问答动作的**分派结果**（端点回执合成的唯一输入；见 `session_question_answer` 文档）。
#[derive(Debug, Clone, PartialEq)]
enum QuestionDispatch {
    /// 单键动作投递成功（`stage` = `None`——单键动作没有阶段语义）
    KeySent { stage: Option<&'static str> },
    /// 阶段机走完整条闭环；`receipt_seen` 三态见端点文档（**不谎报完成**）
    StageDone {
        stage: &'static str,
        receipt_seen: Option<bool>,
    },
    /// **claude 多选切勾闭环**的结论（2026-09-24）：`checked` = 屏读核验到的目标行
    /// 新勾选态（`None` = 键已发出但读不到屏无法核验——不谎报也不误报失败）；
    /// `verified` = 勾选态确实翻转
    ToggleDone {
        checked: Option<bool>,
        verified: bool,
    },
    /// **claude 多题切题闭环**的结论（2026-09-24）：`advanced` = 终端已前移（下一题
    /// 页或 Review 确认屏——前端推进到下一题/确认卡）；`false` = 已在 Review 屏、
    /// 零按键（「返回题目」在 claude 上不可达，显式报告不发键乱试）
    AdvanceDone { advanced: bool },
    /// 阶段机**中止**在某一段（`error` 是带段名的中文文案）
    Aborted { stage: &'static str, error: String },
    /// 投递/读屏的**非阶段**失败（单键动作投递失败等，可重试）
    Failed(String),
    /// 内部任务异常（spawn_blocking panic 等）
    Internal(String),
}

/// E2① 数字直选「**屏读验证 + 导航回退**」的结论（审计/断言用）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum DigitVerifyOutcome {
    /// 验证窗内对话框消失 = 数字已生效（零额外按键）
    Confirmed,
    /// 数字未生效 → 导航回退（`keys` = 已发出的回退键序）
    FellBack { keys: Vec<String> },
    /// 回退构造失败（无高亮/目标越界——不猜起点，导航档同一保守面）
    NavigationRefused(String),
}

/// E2① 数字直选「屏读验证 + 导航回退」**内核**（评审修复抽出：原实现写在
/// spawn_blocking 闭包里只有实机能覆盖——抽取理由与 [`dispatch_question_action`]
/// 同源，参照 E4-E6 阶段机先例）。读屏/发键/等待全走闭包，门禁内脚本化可测。
///
/// # 语义（spec §3 / 词典 §1 计划批准框行——用户终裁 CL-3）
///
/// 1. **验证段**：至多 `poll_rounds` 拍（生产 =
///    [`crate::inject::timing::poll_rounds(DIGIT_VERIFY_POLL_TOTAL_MS)`]），每拍
///    `poll_settle` → `read` 一屏：**对话框消失 = 数字已生效**（D20 命中即停）；
///    仍在场 → 留作回退起点（最后一份选项表含当前高亮位）；读屏不可用同样返回
///    `None`，与「对话框已消失」同走 Confirmed 口径。
/// 2. **回退段**（窗尽仍在场才走）：从最后一份选项表算循环步进
///    （[`crate::inject::dialog::navigation_sequence`]——不猜起点），逐键 `send_key`
///    （键间 `key_settle` 给 TUI 重绘时间）；**回退后不再二次验证**（一次回退是计划
///    口径，连环验证会拖长投递链）。回退中首键失败即停（半途失败不可盲目重试全序列）。
///
/// 返回 (结论, 回退键发送失败原因 Option)——send_err 非空时调用方把投递结果置 Err。
#[allow(clippy::too_many_arguments)]
fn verify_digit_then_fallback<Rd, Snd, PollSettle, KeySettle>(
    target: u32,
    mut read: Rd,
    mut send_key: Snd,
    mut poll_settle: PollSettle,
    mut key_settle: KeySettle,
    poll_rounds: u32,
) -> (DigitVerifyOutcome, Option<String>)
where
    Rd: FnMut() -> Option<Vec<crate::inject::dialog::DialogOption>>,
    Snd: FnMut(&str) -> Result<(), String>,
    PollSettle: FnMut(),
    KeySettle: FnMut(),
{
    let rounds = poll_rounds.max(1);
    let mut last_opts: Option<Vec<crate::inject::dialog::DialogOption>> = None;
    for _ in 0..rounds {
        poll_settle();
        match read() {
            // 对话框已消失 = 数字已生效（提交完成）
            None => return (DigitVerifyOutcome::Confirmed, None),
            Some(opts) => last_opts = Some(opts),
        }
    }
    // 窗尽仍在场 → 导航回退
    let Some(opts) = last_opts else {
        // 全窗屏读不可用：无回退起点（不猜位置）——调用方按「无法回退」如实记 warn
        return (
            DigitVerifyOutcome::NavigationRefused("验证窗内屏读恒失败，无法回退".to_string()),
            None,
        );
    };
    match crate::inject::dialog::navigation_sequence(&opts, target) {
        Ok(seq) => {
            let mut sent = Vec::new();
            for key in &seq {
                if let Err(e) = send_key(key) {
                    return (DigitVerifyOutcome::FellBack { keys: sent }, Some(e));
                }
                sent.push(key.clone());
                key_settle();
            }
            (DigitVerifyOutcome::FellBack { keys: sent }, None)
        }
        Err(e) => (DigitVerifyOutcome::NavigationRefused(e), None),
    }
}

/// **动作分派器**：按 [`StagePlan`] 把动作路由到「单键直发」或两条阶段机之一。
///
/// 抽成独立函数（而不是写在 handler 的 `match` 里）的理由：它是**在持 in-flight 守卫
/// 的闭包内**运行的——写进 `spawn_blocking` 闭包里就只有实机能覆盖。此处把它做成
/// `fn(&RemoteState, ...) -> QuestionDispatch`，端点测试用假缝（`screen_probe` +
/// `injector`）即可覆盖「段推进 / 段中止 / 回执三态」全部路径。
///
/// # 中止的段名映射（回执的 `stage` 字段从哪来）
///
/// 阶段机的 `Err` 文案里**已经含段名**（中文，面向用户），但程序化的回执需要一个稳定
/// 标识——故此处按**已发出的键数**反推段名（编排各段的键序列是确定的：走位段发
/// `down`、提交段发 `enter`、确认段发数字；自由作答是定位数字 → 文本 → 回车）。
/// 反推失败（键序不在预期形态内）→ 用阶段机的首个可能的段名兜底（宁可粗一点，
/// 也不编一个假的精确位置——文案里的中文说明才是给用户的）。
#[allow(clippy::too_many_arguments)]
fn dispatch_question_action(
    st: &Arc<RemoteState>,
    tool: &str,
    pid: u32,
    spec: &crate::inject::families::FamilySpec,
    injector: &dyn crate::inject::engine::Injector,
    sequence: &[String],
    free_text: Option<&str>,
    plan: &StagePlan,
    expected_question: &str,
) -> QuestionDispatch {
    match plan {
        StagePlan::SingleKey => {
            // 逐键投递；首错即停（半途失败不可盲目重试全序列——已发键已生效，
            // 前端按 failed{error} 提示用户核对终端状态后重试）
            for key in sequence {
                if let Err(e) = injector.locate_and_send_key_spec(pid, key, spec) {
                    return QuestionDispatch::Failed(e);
                }
            }
            QuestionDispatch::KeySent { stage: None }
        }
        StagePlan::Submit { max_down_steps } => {
            // 三段（+回执）：提交屏在场 → 闭环走位 → 回车 → Review 屏 → 确认 → 终态。
            // **每段都在发键前屏读**；轮询/读屏全部经 `RemoteState.screen_probe` 缝。
            let probe = |stage: &'static str| -> Option<Vec<String>> {
                (st.screen_probe)(tool, pid).or_else(|| {
                    log::debug!("问答阶段机：{stage} 段屏读不可用（tool={tool} pid={pid}）");
                    None
                })
            };
            let mut terminal = crate::inject::mode::Closures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_submit_stages(
                || poll_question_stage(|| probe("submit-row"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_review_stage(|| probe("review"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_receipt_stage(|| probe("receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
                *max_down_steps,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_RECEIPT,
                    receipt_seen: o.receipt_seen,
                },
                // **中止分类**（复评 F6-2）：屏读形态不符 → Aborted（带段名）；
                // 投递失败 → Failed（**不带** aborted——语义等同批次丙的投递失败，
                // 用户要做的是查通道而不是查终端形态）
                Err(e) => dispatch_abort(e),
            }
        }
        StagePlan::ClaudeToggle { target } => {
            // 闭环切勾：题屏在场 → 方向键走位（每步复核）→ 空格 → 屏读校验翻转。
            // 轮询/读屏全部经 `RemoteState.screen_probe` 缝（与 Submit 段同装配）。
            let probe = |stage: &'static str| -> Option<Vec<String>> {
                (st.screen_probe)(tool, pid).or_else(|| {
                    log::debug!("问答切勾阶段机：{stage} 段屏读不可用（tool={tool} pid={pid}）");
                    None
                })
            };
            let mut terminal = crate::inject::mode::Closures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_toggle_stages(
                *target,
                expected_question,
                || poll_question_stage(|| probe("toggle-row"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::ToggleDone {
                    checked: o.checked_now,
                    verified: o.verified,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        StagePlan::ClaudeAdvance { max_steps } => {
            // 多题切题：走位到尾部推进行（Next）→ 回车 → 读屏分类（下一题 / Review）。
            let probe = |stage: &'static str| -> Option<Vec<String>> {
                (st.screen_probe)(tool, pid).or_else(|| {
                    log::debug!("问答切题阶段机：{stage} 段屏读不可用（tool={tool} pid={pid}）");
                    None
                })
            };
            let mut terminal = crate::inject::mode::Closures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_advance_stages(
                || poll_question_stage(|| probe("advance"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
                *max_steps,
            );
            match out {
                Ok(o) => QuestionDispatch::AdvanceDone {
                    advanced: o.advanced,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        StagePlan::FreeText => {
            let Some(text) = free_text else {
                // 防御：FreeText 计划却没带文本（参数校验已拦 400，不可达）
                return QuestionDispatch::Failed("自由作答缺少文本".to_string());
            };
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::question::FreeTextClosures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                // **文本走字符通道**（`locate_and_inject_spec`）——裁3 的安全面：
                // 用户文本绝不进键通道。注意本调用**不带** `[mobile]` 签名（签名是
                // 「一条新消息」的语义，作答文本不是消息；归一已在 handler 做过）。
                send_text: |t: &str| {
                    injector.locate_and_inject_spec(pid, t, spec)?;
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ));
                    Ok(())
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_free_text_stages(
                text,
                || poll_question_stage(|| probe("free-row"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_receipt_stage(|| probe("free-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_FREE_TEXT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        // ===== 批次戊 E4：kimi 多选/多题提交阶段机 =====
        StagePlan::KimiSubmit => {
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::mode::Closures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_kimi_submit_stages(
                || probe("kimi-initial"),
                || poll_question_stage(|| probe("kimi-review"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_receipt_stage(|| probe("kimi-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_RECEIPT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        // ===== 批次戊 E4：kimi Other 自由作答阶段机 =====
        StagePlan::KimiFreeText => {
            let Some(text) = free_text else {
                return QuestionDispatch::Failed("自由作答缺少文本".to_string());
            };
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::question::FreeTextClosures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                // 文本走字符通道（同 claude 自由作答：用户文本绝不进键通道、不带签名）
                send_text: |t: &str| {
                    injector.locate_and_inject_spec(pid, t, spec)?;
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ));
                    Ok(())
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_kimi_free_text_stages(
                text,
                || probe("kimi-other"),
                || poll_question_stage(|| probe("kimi-review"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_receipt_stage(|| probe("kimi-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_FREE_TEXT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        // ===== 批次戊 E5：codex Tab 备注阶段机 =====
        StagePlan::CodexNotes => {
            let Some(text) = free_text else {
                return QuestionDispatch::Failed("自由作答缺少文本".to_string());
            };
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::question::FreeTextClosures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                send_text: |t: &str| {
                    injector.locate_and_inject_spec(pid, t, spec)?;
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ));
                    Ok(())
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_codex_notes_stages(
                text,
                || probe("codex-notes"),
                || poll_receipt_stage(|| probe("codex-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_FREE_TEXT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        // ===== 批次戊 E6：opencode own answer 阶段机 =====
        StagePlan::OpencodeOwnAnswer => {
            let Some(text) = free_text else {
                return QuestionDispatch::Failed("自由作答缺少文本".to_string());
            };
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::question::FreeTextClosures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                send_text: |t: &str| {
                    injector.locate_and_inject_spec(pid, t, spec)?;
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ));
                    Ok(())
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_opencode_own_answer_stages(
                text,
                || poll_receipt_stage(|| probe("oc-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_FREE_TEXT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
        // ===== 批次戊 E6：opencode 多选提交阶段机（2026-09-23 接线）=====
        StagePlan::OpencodeSubmit => {
            let probe = |_: &'static str| -> Option<Vec<String>> { (st.screen_probe)(tool, pid) };
            let mut terminal = crate::inject::mode::Closures {
                read: || probe("read"),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                    }
                    r
                },
                settle: || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                },
            };
            let out = crate::inject::question::run_opencode_submit_stages(
                // 首段「Confirm 已在场判读」的单次读屏——与编排内的发键后读屏同缝
                || probe("oc-initial"),
                || poll_question_stage(|| probe("oc-confirm"), QUESTION_STAGE_POLL_TOTAL_MS),
                || poll_receipt_stage(|| probe("oc-receipt"), QUESTION_STAGE_POLL_TOTAL_MS),
                &mut terminal,
            );
            match out {
                Ok(o) => QuestionDispatch::StageDone {
                    stage: QUESTION_STAGE_RECEIPT,
                    receipt_seen: o.receipt_seen,
                },
                Err(e) => dispatch_abort(e),
            }
        }
    }
}

/// 阶段机中止 → 分派结果（**分类的单点**：见 `inject::question::StageAbort`）。
///
/// 屏读形态不符 → [`QuestionDispatch::Aborted`]（回执 `failed{aborted:true, stage}`）；
/// 投递失败 → [`QuestionDispatch::Failed`]（回执 `failed{error}`，**不带** aborted，
/// 审计 `failed:<e>`——与批次丙的失败口径逐字一致）。
fn dispatch_abort(e: crate::inject::question::StageAbort) -> QuestionDispatch {
    use crate::inject::question::StageAbortKind;
    match e.kind {
        StageAbortKind::Screen => QuestionDispatch::Aborted {
            stage: stage_from_abort(&e.message),
            error: e.message,
        },
        StageAbortKind::Delivery => QuestionDispatch::Failed(e.message),
    }
}

/// **中止段名反推**（回执 `stage` 字段；纯函数，可测）。
///
/// 依据 = 阶段机各段的中止文案里点名的**关键判据词**（那些文案是面向用户的，词面
/// 稳定且与判据一一对应；比按键计数更直接——按键计数在「还没发键就中止」的段上
/// 区分不出来）。反推不出 → 用 `submit-row`/`free-row` 这类**最靠前的段**兜底
/// （宁可粗，不编假精度）。
fn stage_from_abort(err: &str) -> &'static str {
    // 切题路径（2026-09-24，先于其余路径匹配——切题的走位/分类文案须归 advance 段）
    if err.contains("切题") || err.contains("下一题") {
        return QUESTION_STAGE_ADVANCE;
    }
    // 切勾路径（2026-09-24，先于提交路径匹配——「仍未把焦点移到目标选项行」会被
    // 下方 Submit 行臂误收，切勾的走位目标是选项行不是推进行）
    if err.contains("选项块")
        || err.contains("问答屏")
        || err.contains("目标选项行")
        || err.contains("勾选")
        || err.contains("空格")
        || err.contains("唯一焦点行")
    {
        return QUESTION_STAGE_TOGGLE;
    }
    // 提交路径（按「中止点从后往前」匹配：越靠后的段越具体）
    // E4：kimi 的 Review 汇总屏中止文案先于通用「确认项」词匹配（否则被 confirm 段误收）
    if err.contains("Review 汇总屏") || err.contains("未出现 Review 确认屏") {
        return QUESTION_STAGE_REVIEW;
    }
    if err.contains("读不到编号") || err.contains("确认项") {
        return QUESTION_STAGE_CONFIRM;
    }
    if err.contains("未出现 Review 确认屏") || err.contains("Review 确认屏") {
        return QUESTION_STAGE_REVIEW;
    }
    if err.contains("Submit 行") || err.contains("未把焦点移到") {
        return QUESTION_STAGE_SUBMIT_ROW;
    }
    // 自由作答路径
    if err.contains("自由作答行") || err.contains("未出现自由作答") {
        // 「见不到自由作答行」的中止可能发生在定位段（还没发键）或定位后复核
        // （已发定位键）——两者的用户动作不同（前者去终端看问题在不在，后者核对
        // 焦点），故按文案里的复核特征词再分一次
        if err.contains("已见不到自由作答行") {
            return QUESTION_STAGE_FREE_TEXT;
        }
        return QUESTION_STAGE_FREE_ROW;
    }
    if err.contains("作答文本") {
        return QUESTION_STAGE_FREE_TEXT;
    }
    // 兜底：不知道停在哪一段（理论上不可达——上面覆盖了所有中止文案）
    log::debug!("问答阶段机中止：段名反推未命中（{err}）");
    QUESTION_STAGE_SUBMIT_ROW
}

/// **阶段轮询的生产实现**（单段）：读屏直到 `probe` 返回有效形态。
///
/// 与 `poll_menu_stage` 的差别：**判据是调用方给的闭包**（各段的锚不同），本函数只管
/// 「读的节奏与窗尽语义」。窗尽 → `Ok(None)`（调用方按各段语义决定「不出现」是中止
/// 还是可接受）。读屏本身失败也按「本轮没读到」处理（下一轮再试）——**只有整窗都
/// 读不到**才是 `Ok(None)`，由调用方的文案说明「读不到屏幕」。
///
/// **D20 起窗用「拍数」表达**（[`poll_rounds`] 把总窗换算成 步长 × 拍数，与其余四路
/// 同一口径）——墙钟 deadline 在门禁里只能真等满窗，拍数则可在测试里确定性驱动。
fn poll_question_stage<P>(probe: P, total_ms: u64) -> Result<Option<Vec<String>>, String>
where
    P: Fn() -> Option<Vec<String>>,
{
    Ok(crate::inject::timing::bounded_poll(
        crate::inject::timing::poll_rounds(total_ms),
        probe,
        || std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS)),
    ))
}

/// **Review 屏轮询**：窗内等到**该屏形态可行动**（`inject::question::probe_review_screen`
/// 返回 `Ready`）才返回；`Fatal`（Review 屏在场但形态异常）立即上抛——等下去不会自洽。
/// 窗尽 → `Ok(None)`（编排据此中止且**不发确认键**）。
///
/// **三态语义**（`Ready` 返回 / `Fatal` 上抛 / `NotYet` 重试）比 [`bounded_poll`] 的二值
/// 产物多一态，故此处不套用该内核（硬塞会引入「用空串当哨兵」那类隐式约定，比重复
/// 五行的循环更难读）；窗仍按同一口径用**拍数**表达（[`poll_rounds`]）。
fn poll_review_stage<P>(probe: P, total_ms: u64) -> Result<Option<Vec<String>>, String>
where
    P: Fn() -> Option<Vec<String>>,
{
    use crate::inject::question::ScreenStep;
    let rounds = crate::inject::timing::poll_rounds(total_ms).max(1);
    for i in 0..rounds {
        if let Some(lines) = probe() {
            match crate::inject::question::probe_review_screen(&lines) {
                ScreenStep::Ready(l) => return Ok(Some(l)),
                ScreenStep::Fatal(why) => return Err(format!("{why}；请人工核对终端")),
                ScreenStep::NotYet(_) => {}
            }
        }
        if i + 1 < rounds {
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
    }
    Ok(None)
}

/// **终态回执轮询**：窗内等到屏上出现终态回执行
/// （`inject::question::probe_answered_receipt`）→ `Ok(Some(屏))`。
///
/// 窗尽仍未见 → `Ok(None)`（**不是失败**：回执可能被后续输出刷走，也可能该版本文案
/// 不同；调用方据此下发 `verified=false` + 请人工核对，见端点文档）。
/// 读屏不可用（缝返回 `None` 全程）同样收敛为 `Ok(None)`——我们**没有证据**，故不声称成功。
fn poll_receipt_stage<P>(probe: P, total_ms: u64) -> Result<Option<Vec<String>>, String>
where
    P: Fn() -> Option<Vec<String>>,
{
    Ok(crate::inject::timing::bounded_poll(
        crate::inject::timing::poll_rounds(total_ms),
        || probe().filter(|lines| crate::inject::question::probe_answered_receipt(lines)),
        || std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS)),
    ))
}

// ==== M6R–M9R Task 11：一键 resume 端点（R5，B 兜底可见性半部）====
// 契约（JSON camelCase；Json 响应带 no-store——门禁下私有写路径）：
//   POST /session-open body {sessionId} → 200 {"status":"opening"}
//     | 404 {"error":"no_session"}（会话不在快照）
//     | 404 {"error":"no_resume_command"}（工具未入 resume 命令表——未查证不出手）
//     | 404 {"error":"no_cwd"}（会话无项目目录）
//     | 200 {"status":"failed","error":…}（spawn 出手失败，可重试回执，session-approve 同口径）。
// 审计 action="open"（W5 词表追加项，Task 7「预留：Task 11」兑现）：仅在 spawn
// 出手时落账（ok / failed:{e}）——404 校验失败不写审计（session-send 同口径）。
// 终端选择在核心（inject::resume）：Windows 优先 wt、回退 conhost；macOS 优先
// iTerm2、次选 Terminal.app。spawn 缝（RemoteState.resume_spawner）使端点测试
// 零真开窗。

/// POST /m/api/v1/session-open 请求体（camelCase；缺参不触发 axum 提取器 422，
/// 由 handler 统一按契约给 400 bad_request）
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpenReq {
    #[serde(default)]
    pub session_id: String,
}

/// POST /m/api/v1/session-open（R5 一键 resume）：查找会话 → inject::resume 核心
/// （spawner 缝消费 RemoteState.resume_spawner）→ 按错误契约分诊。查找与 spawn
/// 出手同在一个 spawn_blocking（会话扫描是重活，sessions handler 同一先例）。
pub async fn session_open(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionOpenReq>,
) -> Response {
    // ① 参数校验（trim 判空——与 session-send 同口径）
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() {
        return bad_request();
    }
    // ② 设备身份（防御 403 + 花名）
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // ③ 查找 + 核心（spawn_blocking：扫描是同步阻塞调用；核心内 wt 探测进程级缓存）。
    // sid 移动副本进闭包，原值保留给审计（session-send 同一写法）
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let outcome = match tokio::task::spawn_blocking(
        move || -> Result<(String, Result<(), String>), String> {
            // 复合键口径（Task 5 教训）：快照里按 id 找第一个匹配——契约如此
            let session = match (probe_st.session_source)()
                .sessions
                .into_iter()
                .find(|s| s.id == probe_sid)
            {
                Some(s) => s,
                // 归档回退（spec §6.2）：死会话经登记表复活。id 仍取自本机数据
                // （登记行），不回显远端输入——resume.rs 安全口径不变；构造的
                // Session 仅 resume 链消费的三字段有效（id/agent_type/project_path），
                // 其余字段为中性缺省（不上面板）
                None => {
                    let Some(row) = (probe_st.archive_source)()
                        .into_iter()
                        .find(|r| r.session_id == probe_sid)
                    else {
                        return Err("no_session".to_string());
                    };
                    let Some(agent_type) =
                        crate::database::agent_type_from_tool_id(&row.agent_type)
                    else {
                        // 未知 tool_id：resume_command 必返 None，提前以既有哨兵回退
                        return Err("no_resume_command".to_string());
                    };
                    crate::session::Session {
                        id: row.session_id,
                        agent_type,
                        project_name: row.project_name,
                        project_path: row.project_path,
                        title: row.title,
                        git_branch: None,
                        github_url: None,
                        status: crate::session::SessionStatus::Waiting,
                        last_message: None,
                        last_message_role: None,
                        last_activity_at: row.last_seen.clone(),
                        pid: 0,
                        cpu_usage: 0.0,
                        active_subagent_count: 0,
                        form: crate::session::ProcessForm::Cli,
                        jump_supported: false,
                        unread: false,
                    }
                }
            };
            let tool = session.agent_type.tool_id().to_string();
            let r = crate::inject::resume::open_session_terminal_with(
                &session,
                probe_st.resume_spawner.as_ref(),
            );
            Ok((tool, r))
        },
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-open 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let (tool, result) = match outcome {
        Ok(v) => v,
        Err(code) => {
            // 会话不在快照：无工具可审计（session-send 的 no_session 同口径不落账）
            return json_no_store(StatusCode::NOT_FOUND, serde_json::json!({ "error": code }));
        }
    };
    // ④ 错误契约分诊：哨兵串映射 404（校验失败不写审计、spawner 未被调用）；
    // 其余为 spawn 出手后的失败 → 200 failed（可重试回执）+ 审计 failed
    match result {
        Ok(()) => {
            // 审计 content = resume 命令摘要（audit_write 内 summarize 截断）
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &resume_command_for_audit(&tool, &sid),
                "open",
                "ok",
            );
            json_no_store(StatusCode::OK, serde_json::json!({ "status": "opening" }))
        }
        Err(e) if e == "no_resume_command" || e == "no_cwd" => {
            json_no_store(StatusCode::NOT_FOUND, serde_json::json!({ "error": e }))
        }
        Err(e) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &resume_command_for_audit(&tool, &sid),
                "open",
                &format!("failed:{e}"),
            );
            json_no_store(
                StatusCode::OK,
                serde_json::json!({ "status": "failed", "error": e }),
            )
        }
    }
}

/// 审计 content 的 resume 命令摘要（与核心同一命令表产物；spawner 出手成功时
/// 核心已构造过一次——此处再取一次纯表查询，代价可忽略，换取审计与出手同源）
fn resume_command_for_audit(tool: &str, sid: &str) -> String {
    crate::inject::resume::resume_command(tool, sid).unwrap_or_default()
}

// ============================================================
// 批次丙 T6：模式切换端点（GET 当前档 + POST 切档）
// ============================================================

/// 当前模式扫描产物（GET /session-mode；丁T4 起携带结构表）
struct ModeScanHit {
    /// 工具标识（前端据结构表决定渲染分支）
    tool: String,
    /// 屏读到的当前模式档（None = 屏读失败/形态漂移/无回读源 → 前端「请人工核对」）
    current: Option<crate::inject::mode::MamMode>,
    /// 该工具是否有屏读回显（红线 4 的判据；顶层兼容视图用）
    readback: bool,
    /// 切换机制（旧 `switchKind` 字段的取值来源）
    kind: crate::inject::mode::ModeSwitchKind,
    /// 模式栏结构（裁5：二维两组 / 单轴一组 / 无）
    structure: crate::inject::mode::ModeStructure,
}

/// 模式扫描（同步，spawn_blocking 内调用）：会话查找 → 结构表 → 屏读（一次）解析
/// 当前模式档。非 Windows / 屏读失败 / 形态认不出 → current=None（降级）。
///
/// **屏读只做一次**（阻塞 FFI 的代价不可重复付）：四家的**模式组**共用这一份行集；
/// 权限组无底栏源（实测）→ GET 侧恒 null。
fn mode_scan_sync(st: &Arc<RemoteState>, session_id: &str) -> Option<ModeScanHit> {
    let session = (st.session_source)()
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)?;
    let tool = session.agent_type.tool_id().to_string();
    let structure = crate::inject::mode::mode_structure(&tool);
    let kind = crate::inject::mode::switch_kind(&tool);
    let readback = crate::inject::mode::mode_readback_supported(&tool);
    let current = read_mode_from_screen(st, &session, &tool, readback);
    Some(ModeScanHit {
        tool,
        current,
        readback,
        kind,
        structure,
    })
}

/// 屏读当前模式（批次丙 T6；丁T4 改**分族解析**）。仅对支持回显的工具调用。
/// 非 Windows / 屏读失败 / 该族词表认不出 → None（调用方降级为「档未知」+
/// 人工核对提示，红线 4：不假装成功）。
///
/// **D20(c) 的瞬时快照**：这是注入**前**的决策依据（`before` 用于推算步进组的应到档）
/// 与 GET 的当前档显示，故**单次读即返回、不轮询**——给快照加轮询会把「此刻是什么档」
/// 变成「等 N 秒后的某个档」，语义相反（见 `inject::timing` 模块文档的合规映射表）。
///
/// **走 [`RemoteState::screen_probe`] 缝**（与回读轮询同一个能力缝）：生产装配 = 真屏读，
/// 测试注入脚本化屏序列——否则「注入前的快照」与「注入后的轮询」两段控制流在门禁里
/// 都只剩真机能覆盖。
fn read_mode_from_screen(
    st: &Arc<RemoteState>,
    session: &crate::session::Session,
    tool: &str,
    readback: bool,
) -> Option<crate::inject::mode::MamMode> {
    if !readback {
        return None;
    }
    match (st.screen_probe)(session.id.as_str(), session.pid) {
        Some(lines) => {
            let m = crate::inject::mode::parse_mode_from_screen(tool, &lines);
            log::debug!("模式屏读（{tool} pid={}）→ {:?}", session.pid, m);
            m
        }
        None => {
            // 能力缺失（非 Windows）或屏读失败：**无法判定**（不是「默认档」）
            log::debug!("模式屏读不可用（{tool} pid={}）→ 档未知", session.pid);
            None
        }
    }
}

/// GET /m/api/v1/session-mode?session_id=（T6；丁T4 扩二维结构 + 回读全开）：
/// 返回**结构表**（二维两组 / 单轴一组 / 无）+ 每组的当前档与可选性 + 该工具的
/// 切换机制。会话不存在 → 404 no_session。
///
/// **降级语义（红线 4）**：`current` 为 null 表示「未知」（屏读失败/形态漂移/该组
/// 无回读源）——前端必须显示「请人工核对终端模式」，不得假装知道。丁T4 起该字段
/// 是**每组一份**（`groups[].current`），顶层 `current` 保留为旧客户端的兼容视图。
///
/// **前向兼容（旧前端不破）**：顶层字段（`current`/`currentLabel`/`readback`/
/// `switchKind`）**原样保留**，取值口径 = 模式组（旧前端只认一个轴，而模式组正是
/// 旧实现的语义面）；新字段（`structure`/`groups`）是纯增量。旧后端对新前端同样
/// 兼容（新前端在字段缺失时回落到「单轴渲染 + 顶层 current」，见 `ModeBar.tsx`）。
/// 两端都为对方留了缺省，是「不强制同时升级」的最低要求。
///
/// 屏读**只做一次**（读一屏的代价是阻塞 FFI），四家的模式组共用这一份行集；
/// 权限组无底栏源（实测）→ 恒 null。
/// 权限档「**上次切换**」记忆（2026-09-23 codex 模式切换改造）：codex/kimi 权限组
/// 无被动回读源（底栏不印档位文本）→ GET 的权限组 `current` 原本恒 null、前端恒显
/// 「模式未知」。本表记录**每次 verified=true 的权限组切换**（session_id → 档 wire
/// 词），GET 从此回放（无记录 = null，前端照旧显示「模式未知」）。
///
/// 已知边界（如实登记，台账「codex 模式切换改造」节）：用户在终端手改档位后记忆
/// 会失真——前端以「上次切换」标注明示口径，不声称实时。**持久化（2026-09-23 二轮
/// 用户反馈「模式未知」）**：内存 + settings KV（`mode:perm-tier:<sid>`）双写，重启
/// 后内存清零但 GET 从 KV 回放——不再每次部署/重启都退回「模式未知」。
static PERMISSION_TIER_MEMORY: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

fn permission_tier_kv_key(sid: &str) -> String {
    format!("mode:perm-tier:{sid}")
}

/// 记录一次 verified 的权限组切换结果（内存 + settings KV 双写；KV 走
/// `store.with`——测试注入 memory 库，零接触真实 ~/.mam）
pub(crate) fn remember_permission_tier(
    store: &super::pairing::DeviceStore,
    sid: &str,
    tier_wire: &str,
) {
    if let Ok(mut m) = PERMISSION_TIER_MEMORY.lock() {
        m.insert(sid.to_string(), tier_wire.to_string());
    }
    let key = permission_tier_kv_key(sid);
    store.with(|c| crate::database::dao::settings::set_setting_conn(c, &key, tier_wire));
}

/// 回放某会话最近一次 verified 的权限档（内存未命中查 KV 并回填；都无 / 解析失败 → None）
pub(crate) fn recall_permission_tier(
    store: &super::pairing::DeviceStore,
    sid: &str,
) -> Option<crate::inject::mode::MamMode> {
    if let Some(wire) = PERMISSION_TIER_MEMORY.lock().ok()?.get(sid).cloned() {
        return crate::inject::mode::MamMode::parse(&wire);
    }
    let key = permission_tier_kv_key(sid);
    let wire = store.with(|c| crate::database::dao::settings::get_setting_conn(c, &key))?;
    let mode = crate::inject::mode::MamMode::parse(&wire)?;
    if let Ok(mut m) = PERMISSION_TIER_MEMORY.lock() {
        m.insert(sid.to_string(), wire);
    }
    Some(mode)
}

pub async fn session_mode(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    let scan_sid = sid.clone();
    let probe_st = st.clone();
    let scan = match tokio::task::spawn_blocking(move || mode_scan_sync(&probe_st, &scan_sid)).await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-mode 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let Some(hit) = scan else {
        return json_no_store(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "no_session" }),
        );
    };
    let kind = match hit.kind {
        crate::inject::mode::ModeSwitchKind::ShiftTabCycle => "shiftTab",
        crate::inject::mode::ModeSwitchKind::SlashCommand => "slashCommand",
        crate::inject::mode::ModeSwitchKind::Unsupported => "unsupported",
    };
    // 组载荷：模式组带屏读到的当前档；权限组从「上次切换」记忆回放
    // （无被动回读源——verified 切换写入 [`PERMISSION_TIER_MEMORY`]，无记录 = null）
    let groups: Vec<serde_json::Value> = hit
        .structure
        .groups()
        .into_iter()
        .map(|g| {
            let current = if g.id == crate::inject::mode::ModeGroupId::Mode {
                hit.current
            } else {
                recall_permission_tier(&st.store, &sid)
            };
            serde_json::json!({
                "id": g.id.wire(),
                "label": g.label,
                "step": g.step,
                "readback": g.readback,
                // 按钮布局（tiers=逐档钮 / toggle=单钮循环——codex 模式组）
                "layout": g.layout.wire(),
                "current": current.map(|m| m.wire()),
                "currentLabel": current.map(|m| g.tiers.iter().find(|t| t.mode == m).map(|t| t.label).unwrap_or(m.label())),
                "tiers": g.tiers.iter().map(|t| serde_json::json!({
                    "mode": t.mode.wire(),
                    "label": t.label,
                    "selectable": t.selectable,
                    "reason": t.reason,
                })).collect::<Vec<_>>(),
                // 裁7：退役旧档**如实展示**（前端不渲染为可点按钮）
                "legacy": g.legacy.iter().map(|l| serde_json::json!({
                    "label": l.label,
                    "note": l.note,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    // 顶层兼容视图 = 模式组（旧前端只认一个轴，而模式组正是旧实现的语义面）
    let mode_group = hit.structure.group(crate::inject::mode::ModeGroupId::Mode);
    let top_current = mode_group
        .map(|g| if g.readback { hit.current } else { None })
        .unwrap_or(hit.current);
    // E3④：问答待决旗标（前端置灰数据源）——二维家的切档注入含回车，问答待决时
    // 会被问答框误消费（codex 交默认答案 / kimi 误选推进待决态）；前端据此置灰 +
    // 原因文案（spec §4 待决拦截的前端半边）。判定与 POST 守卫同源
    // （[`pending_question_tail_index`]）。
    let question_pending = if matches!(hit.kind, crate::inject::mode::ModeSwitchKind::SlashCommand)
    {
        let ms = st.clone();
        let p_sid = sid.clone();
        let p_tool = hit.tool.clone();
        tokio::task::spawn_blocking(move || {
            (ms.message_source)(&p_tool, &p_sid, QUESTION_SCAN_TAIL_LIMIT)
                .map(|p| pending_question_tail_index(&p.messages).is_some())
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    } else {
        false
    };
    json_no_store(
        StatusCode::OK,
        serde_json::json!({
            "tool": hit.tool,
            "current": top_current.map(|m| m.wire()),
            "currentLabel": top_current.map(|m| m.label()),
            "readback": mode_group.map(|g| g.readback).unwrap_or(hit.readback),
            "switchKind": kind,
            "questionPending": question_pending,
            // 丁T4 增量：结构 + 两组（旧前端忽略未知字段，零破坏）
            "structure": hit.structure.wire(),
            "groups": groups,
        }),
    )
}

/// 对话框在场拒绝对控制类注入的中文回执（丁T3 §2.7 裁8/9 的任务书成文语义：
/// 「终端有待决对话框，请先处理」）。前端 ModeBar 直显 `reason`（与 no_mechanism
/// 的 `error` 码分诊并列——码供程序分支，文案供用户阅读）。
const DIALOG_BLOCKS_CONTROL_REASON: &str = "终端有待决对话框，请先处理";

/// POST /m/api/v1/session-mode 请求体（camelCase；字段全 default 防 422）。
///
/// **前向兼容**：`group` 是丁T4 的**新增可选字段**——旧客户端只发
/// `{sessionId, target}`，由 [`crate::inject::mode::resolve_group`] 按档位归组
/// （规则见该函数文档）；新客户端发 `group` 时必须是该工具结构里存在的组。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeReq {
    #[serde(default)]
    pub session_id: String,
    /// 目标档（wire 词：plan/default/acceptEdits/bypass/readOnly）
    #[serde(default)]
    pub target: String,
    /// 目标组（wire 词：mode/permission；缺省 = 按 target 推断）
    #[serde(default)]
    pub group: Option<String>,
}

/// 两段式注入的**菜单轮询预算**（毫秒）——第一段 `/permissions` 回车后，轮询等菜单画出，
/// 读不到就在本窗内重试，窗末仍读不到则**中止第二段并如实回执**（不盲发方向键）。
///
/// **取值与依据已迁至** [`crate::inject::timing::MENU_POLL_TOTAL_MS`]（D20 的单一事实源：
/// 值、自裁说明与 `#[ignore]` 实测项的指向都在那里）——此处 re-export 保持既有引用点
/// （[`menu_stages`] / [`session_mode_switch`] 的文档锚）不破。
/// **D20 起 600ms → 1500ms**：旧值依据是「M9R 人工敲命令的时序」，不代表 MAM 注入路径
/// （多了文本分块 + `SUBMIT_DELAY_MS` + TUI 重绘）——用户实机观察②即低估的表现。
pub use crate::inject::timing::MENU_POLL_TOTAL_MS;

/// **第三段（Full Access 二次确认框）**的轮询预算（毫秒）——第二段提交后等确认框画出。
///
/// **取值与依据已迁至** [`crate::inject::timing::CONFIRM_POLL_TOTAL_MS`]。
/// **与菜单轮询的关键差别是超时的语义**（这一条留在本模块，因为它约束的是调用方）：
/// 菜单读不到 = 中止（功能不可用）；确认框读不到 = **不当作失败**——codex 二进制里有
/// `Continue and don't warn again.`（用户此前关过该警告就不会再有确认框），此时应当
/// 继续走成功回执核验，而不是报错（见 [`menu_stages`] 的文档）。
pub use crate::inject::timing::CONFIRM_POLL_TOTAL_MS;

/// **成功回执核验**的轮询预算（毫秒）——提交后等工具打印 `Permissions updated to …`
/// / `Permission mode: …`。
///
/// **取值与依据已迁至** [`crate::inject::timing::RECEIPT_POLL_TOTAL_MS`]。
/// 窗末仍未见 → `receipt_seen=false` → 回执 `verified:false` + 「请人工核对」，
/// **不当作失败**（回执可能被后续输出刷走）。
pub use crate::inject::timing::RECEIPT_POLL_TOTAL_MS;

/// POST /m/api/v1/session-mode（T6 切档；丁T4 二维 + 菜单两段式 + Full Access 第三段）：
/// 校验 → **对话框在场守卫（丁T3 接入①）** → 机制分派 → 注入 → 回读/回执核验 → 审计 mode。
///
/// # 注入形态（§2.6 逐格）
///
/// - **步进组**（claude/opencode）：注入 **shift+tab 单键**（一次切一档，目标档不
///   参与按键构造）；切完**回读**，与环序推算的应到档比对；
/// - **直达文本**（codex 模式组 `/plan`；kimi 模式组 `/plan on|off`；kimi 权限组
///   `/yolo`、`/auto`）：文本注入 + 回车提交；
/// - **菜单两段式/三段式**（codex 权限组 `/permissions`；kimi 权限组「总是询问」）：
///   见下节；
/// - **无机制**（含 codex「退出计划模式」、退役档）：409 `no_mechanism` + `reason`.
///
/// # 菜单路径的分段（第二段 / 第三段 / 回执核验）
///
/// - **第二段**：等菜单画出 → **闭环导航**（[`crate::inject::mode::navigate_until_highlighted`]：
///   每发一个方向键重新屏读复核高亮位移，**只有**高亮确实落在目标档行才 `enter`）；
/// - **第三段**（**仅** codex × 权限组 × Full Access，实机取证档案 §3：1/2/3 档无此段）：
///   等 `Enable full access?` 确认框 → 定位**唯一**肯定项 → 同一套闭环 → `enter`。
///   窗内未见确认框**不当作失败**（用户可能此前关过该警告，二进制有
///   `Continue and don't warn again.`）→ 继续走回执核验；
/// - **回执核验**（所有档位）：屏读工具自己的成功回执行
///   （codex `• Permissions updated to Full Access` / kimi `Permission mode: Always Ask`）
///   → 见到且含目标档 ⇒ `verified=true`（工具自证比档位回读更强，一票通过）；未见
///   ⇒ `verified=false` + 「请人工核对」，**不当作失败**。见 [`receipt_and_verdict`]。
///
/// # 菜单路径与丁T3 守卫如何共存（**本任务最需要想清楚的一处**）
///
/// **冲突**：菜单路径的第一段打开菜单后，屏幕上**必然**出现一个待选菜单——而丁T3 的
/// 守卫判据 `blocks_control_injection` 正是「屏上存在可选簇 ⇒ 拒绝控制类注入」。
/// 若第二段再走一次守卫，它会**必然**拒绝自己刚打开的菜单（死锁：永远切不了权限档）。
///
/// **解法（三句话，逐句可验）**：
/// 1. **一次请求 = 一次注入 = 一道守卫**：各段在**同一个 handler 调用内**串行完成，
///    守卫在**任何注入之前**判一次（判的是「用户点按钮那一刻终端上有没有别的对话框」）。
///    第二段投递的是**我们自己刚打开的那个菜单**，不是用户点按钮时就存在的对话框——
///    它不是守卫要防的对象，所以**不重入守卫**。若把守卫挪到第二段前，判据会把
///    自家菜单判成「待决对话框」，菜单路径永久不可用（这正是本任务要避免的自相矛盾）。
/// 2. **各段有硬锚，不靠「屏上有没有簇」**：第二段的定位用**权限菜单专用词表 + 关键词
///    包含 + 计数互斥**（[`crate::inject::mode::locate_menu_items`]，判据依据 = 实机
///    取证档案 §1/§4/§5 的逐字原文）并过一致性闸（`menu_items_coherent`）；第三段的
///    定位用**确认框的肯定项关键词**（[`crate::inject::mode::FULL_ACCESS_AFFIRMATIVE_KEYWORD`]）。
///    认不出就直接**中止且不投递任何键**（如实回执）。也就是说各段**只对「确实是那个
///    菜单/那个确认框」的屏**出手；屏上是别的对话框（包括用户在两次注入之间手动触发
///    的）时，锚匹配不上 → 中止。
/// 3. **导航走闭环**（[`crate::inject::mode::navigate_until_highlighted`]）：每一步都
///    重新屏读、复核高亮存在且位移恰好 1 行且标签合法；`enter` 只在标签等于目标行时
///    发出。**不再**走审批路径的 `navigation_sequence_directional`（那条是「一次算步进
///    + 盲发」，正是本批被实机证据推翻的部分；审批路径仍在用它，勿动）。
///
/// **与 approve 端点 `dialog:N` 路径的关系（为什么不复用那条路）**：approve 的
/// 导航序列是**一次请求内的单段**（屏上已有对话框，投递 ↓×k+Enter 即可），而权限档
/// 切换需要**先开菜单**——若拆成两次请求（前端先 POST 开菜单、再 POST 导航），中间
/// 用户可介入、菜单可消失，且第二段会被守卫拒（就是上面第 1 条的死锁）。故各段收在
/// 同一 handler 内：守卫只过一道，菜单生命周期完全在同一临界区。
///
/// **守卫覆盖面的如实申报**：本函数的菜单路径**不再重入**守卫，因此「第一段与第二段
/// 之间**用户手动**在终端里触发了另一个对话框」这条极窄窗口不在守卫覆盖内。窗口长度
/// = 第一段回车到菜单屏读到（≤ `MENU_POLL_TOTAL_MS`），且此间 MAM 侧持有
/// `INFLIGHT` 守卫（同一会话的其它注入被让位），实际可达性极低。**不假装这是全覆盖**。
///
/// # codex `/plan` 运行中不可用（§2.6 表末）→ 如实回执
///
/// 判据 = [`crate::inject::mode::codex_plan_busy`]（只对 codex × 模式组 × Plan；
/// 「运行中」复用 [`crate::inject::queue::is_running`] 三态口径）。命中 → 200
/// `{status:"failed", error:"…"}`，**零注入**（codex 自己会回 `Plan mode unavailable
/// right now.`，MAM 提前拦下并说清楚）。**不落审计**（无投递发生，与忙让位同口径）。
///
/// # 回执如实（裁5 + 红线 4）
///
/// 回读成功且命中 → `verified=true`；回读成功但**不是**目标档 →
/// `verified=false` + `hint` 报出两边（不假装成功）；无法判定（无回读源/屏读失败）
/// → `verified=false` + 人工核对提示。`dialogChecked` 同丁T3。
///
/// **D20(a) 起回读是动态轮询**（旧实现是「固定睡 150ms 后单次读」——用户实机观察①
/// 报出的「回读与预期不符」正由此而来）。三态语义与上面**逐字一致**，只是判定取自
/// **窗内最后一拍**（命中则提前停）；窗与步长见
/// [`crate::inject::timing::MODE_READBACK_POLL_TOTAL_MS`] / `timing::POLL_STEP_MS`。
///
/// # 对话框在场守卫（丁T3 §2.7，裁8/9）—— `blocked_by_dialog`
///
/// **问题 5 实锤**：审计 17:36–38 连点 17 次模式钮，全部落进终端上那个待决对话框
/// ——`shift+tab` 被对话框当成导航键、斜杠命令文本成了选择题的输入，用户的意图
/// （切档）与终端的理解（「选第一项」）完全脱节，且**回合被打断**。
///
/// 修法：投递前屏读可见窗口，[`crate::inject::dialog::blocks_control_injection`] 判
/// 「编号选项对话框在场」→ **拒绝本轮注入**，回 409 `blocked_by_dialog` + 中文文案
/// （与 `no_mechanism` 同用 409 CONFLICT：二者都是「当前状态不允许这个动作」，不是
/// 参数错误也不是授权问题）。**Key 路（shift+tab）与 Text 路（斜杠命令）与 Menu 路
/// （两段式的第一段）同受此门**——三路都在本函数内、都在投递之前，守卫位置天然
/// 覆盖（不是两处判据；第二段为何不再重入见上节）。
///
/// **零注入零审计**：拒绝发生在任何注入调用与任何 `endpoint_audit` 之前（与
/// session-approve / session-question 的「校验失败不投递不落审计」同口径）——账实
/// 一致的前提是「账只记真发生过的事」。
///
/// **检测能力缺失时的处置（明确裁决）**：探针返回 `None`（非 Windows 无屏读 /
/// AttachConsole 失败 / 解析无簇）→ **放行**，并在回执里带 `dialogChecked:false`
/// 如实标注「本次未做在场检测」。理由见
/// [`crate::inject::dialog::blocks_control_injection`] 的文档：能力缺失 ≠ 对话框在场，
/// 把二者混同会让 macOS 的模式钮永久不可用、且回执文案会说假话（「终端有待决对话框」
/// 是我们并不知道的事）。`dialogChecked:true` 时该字段为真，前端可据此选择是否向用户
/// 说明守卫已生效。
/// POST /m/api/v1/session-mode/menu 请求体（camelCase；字段全 default——缺参不触发
/// axum 提取器 422，由 handler 统一按契约给 400）。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeMenuReq {
    #[serde(default)]
    pub session_id: String,
    /// 动作：`open`（开菜单并读回选项）/ `pick`（按用户点选的屏上编号敲键）
    #[serde(default)]
    pub action: String,
    /// `pick` 用：用户点选的**屏上编号**（1..=9）
    #[serde(default)]
    pub number: Option<u32>,
}

/// picker 端点的错误码（与 `session_mode_switch` 分诊口径一致）
fn mode_menu_err(status: StatusCode, code: &str, reason: Option<&str>) -> Response {
    let mut body = serde_json::json!({ "error": code });
    if let Some(r) = reason {
        body["reason"] = serde_json::json!(r);
    }
    json_no_store(status, body)
}

/// **`POST /m/api/v1/session-mode/menu`**——codex 权限组的「终端菜单单选题」两动作
/// （2026-09-23 用户方案）。
///
/// # 为什么需要这条端点（而不是复用 `session-mode/switch`）
///
/// 旧路径是「后端猜目标档的屏上编号 → 自己敲」。用户实机走查暴露的问题：档位编号会
/// 随 Guardian 配置前移（`Approve for me` 缺席时 `Full Access` 从 4 变 3），而前端文案
/// 与后端逻辑都按「4→1」写死——**猜编号这件事本身不可靠**。用户方案把这一步交给用户：
/// MAM 只负责**读回终端菜单的选项表**（编号 = 屏上实读值、文本 = 屏上原文），用户点哪
/// 一项就敲哪个数字键。于是「猜」被消灭，而不是被修得更准。
///
/// # 两动作
///
/// - `action:"open"`：清场（残留 overlay + 输入行纯净）→ `/permissions` + enter →
///   等 ≥0.5s → 读回菜单选项表 → `{status:"menu", options:[…]}`；
/// - `action:"pick"`：**先屏读确认菜单/确认框确实在屏**（不在 → 零投递，如实拒绝）
///   → 发用户点选的数字键（无回车）→ 若屏上出现 Full Access 确认框 →
///   `{status:"confirm", options:[…]}`（二阶段继续交给用户点）；否则做回执核验 →
///   `{status:"done", verified, hint, …}`。
///
/// # 守卫与审计
///
/// 复用 `mode_switch_block` 三态（对话框在场 / kimi 问答待决 / codex 模式组运行中），
/// 但 codex 自己的权限菜单/确认框从「对话框在场」判据里**豁免**（否则 picker 会被自己
/// 的守卫拦死——菜单的编号行本身就是编号簇）。审计经 `endpoint_audit`（action=`mode`，
/// 摘要含「终端菜单第 N 项」）。
pub async fn session_mode_menu(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionModeMenuReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    if sid.is_empty() {
        return bad_request();
    }
    let action = req.action.trim().to_string();
    if action != "open" && action != "pick" {
        return bad_request();
    }
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // 会话查找（与 switch 端点同纪律：一个 spawn_blocking）
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let found = tokio::task::spawn_blocking(move || {
        (probe_st.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == probe_sid)
    })
    .await;
    let session = match found {
        Ok(Some(s)) => s,
        Ok(None) => return mode_menu_err(StatusCode::NOT_FOUND, "no_session", None),
        Err(e) => {
            log::error!("session-mode/menu 会话扫描任务异常: {e}");
            return mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    let tool = session.agent_type.tool_id().to_string();
    if tool != "codex" {
        // 单选面板目前**只有 codex**（kimi 菜单无屏上编号，数字直达不适用；其余工具
        // 无权限菜单）。如实拒绝，不假装支持。
        return mode_menu_err(
            StatusCode::CONFLICT,
            "no_mechanism",
            Some("终端菜单选择仅支持 codex（其它工具的权限档不提供屏上编号）"),
        );
    }
    if session.pid == 0 {
        return mode_menu_err(
            StatusCode::CONFLICT,
            "no_mechanism",
            Some("该会话没有可读屏的终端进程"),
        );
    }
    // ===== 守卫（三态，与 switch 端点同源；codex 自身 overlay 豁免）=====
    let pid = session.pid;
    let guard_st = st.clone();
    let guard_sid = sid.clone();
    let guard = tokio::task::spawn_blocking(move || {
        let dialog = (guard_st.dialog_probe)(guard_sid.as_str(), pid);
        let screen = (guard_st.screen_probe)(guard_sid.as_str(), pid);
        (dialog, screen)
    })
    .await;
    let (dialog_options, screen_lines) = match guard {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-mode/menu 守卫探测任务异常: {e}");
            (None, None)
        }
    };
    // codex 自身 overlay 在场 → 从「对话框在场」判据豁免（见函数文档）
    let own_overlay = screen_lines
        .as_ref()
        .map(|l| {
            let lowered: Vec<String> = l.iter().map(|x| x.to_lowercase()).collect();
            crate::inject::mode::codex_overlay_present(&lowered)
        })
        .unwrap_or(false);
    let dialog_checked = dialog_options.is_some();
    let dialog_blocked =
        crate::inject::dialog::blocks_control_injection(dialog_options.as_deref()) && !own_overlay;
    let is_running = crate::inject::queue::is_running(&session.status);
    // codex 模式组的运行中拦截不适用本端点（本端点只服务权限组）；对话框在场仍拦
    // （用户消息/自由文本不该在别的工具对话框待决时打进终端）
    if dialog_blocked {
        log::debug!("picker 被拒：屏读见编号选项对话框（非 codex 自身 overlay）");
        return mode_menu_err(
            StatusCode::CONFLICT,
            "blocked_by_dialog",
            Some(DIALOG_BLOCKS_CONTROL_REASON),
        );
    }
    let _ = is_running;
    let spec = crate::inject::families::family_for(&tool)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    let injector = st.injector.clone();
    if action == "open" {
        // ===== open：清场 → 开菜单 → 读回选项表 =====
        let do_sid = sid.clone();
        // 闭包返回 `Result<Result<Vec<..>, String>, String>`：外 Err = 忙让位（哨兵），
        // 内 Err = 真失败（与 `menu_stages` 的两层语义同源）
        let r = tokio::task::spawn_blocking(
            move || -> Result<Result<Vec<crate::inject::dialog::DialogOption>, String>, String> {
                let Some(_guard) = crate::inject::queue::try_acquire_inflight(&do_sid) else {
                    return Ok(Err(String::new())); // 忙让位：空文案，外层据此给 busy 回执
                };
                let key_delay = || {
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::families::SUBMIT_DELAY_MS,
                    ))
                };
                let mut terminal = crate::inject::mode::Closures {
                    read: || read_screen_lines(pid),
                    send: |key: &str| {
                        let r = injector.locate_and_send_key_spec(pid, key, &spec);
                        if r.is_ok() {
                            key_delay();
                        }
                        r
                    },
                    settle: key_delay,
                };
                let out = crate::inject::mode::run_codex_menu_open(
                    || {
                        injector
                            .locate_and_inject_spec(pid, "/permissions", &spec)
                            .and_then(|()| {
                                std::thread::sleep(std::time::Duration::from_millis(
                                    crate::inject::families::SUBMIT_DELAY_MS,
                                ));
                                injector.locate_and_send_key_spec(pid, "enter", &spec)
                            })
                    },
                    || poll_menu_options(pid),
                    || {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::timing::MODE_STEP_MIN_GAP_MS,
                        ))
                    },
                    &mut terminal,
                );
                Ok(out)
            },
        )
        .await;
        let (result_str, response) = match r {
            Ok(Ok(Ok(opts))) => {
                let items: Vec<serde_json::Value> = opts
                    .iter()
                    .map(|o| {
                        serde_json::json!({
                            "number": o.number,
                            "label": o.label,
                            "highlighted": o.highlighted,
                        })
                    })
                    .collect();
                (
                    "ok".to_string(),
                    json_no_store(
                        StatusCode::OK,
                        serde_json::json!({
                            "status": "menu",
                            "options": items,
                            "dialogChecked": dialog_checked,
                        }),
                    ),
                )
            }
            Ok(Ok(Err(e))) if e.is_empty() => (
                "busy".to_string(),
                json_no_store(
                    StatusCode::OK,
                    serde_json::json!({
                        "status": "failed",
                        "error": "投递进行中，请稍后重试"
                    }),
                ),
            ),
            Ok(Ok(Err(e))) => (
                format!("failed:{e}"),
                json_no_store(
                    StatusCode::OK,
                    serde_json::json!({ "status": "failed", "error": e }),
                ),
            ),
            Ok(Err(e)) => {
                log::error!("session-mode/menu open 任务异常: {e}");
                (
                    "internal".to_string(),
                    mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None),
                )
            }
            Err(e) => {
                log::error!("session-mode/menu open 任务异常: {e}");
                (
                    "internal".to_string(),
                    mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None),
                )
            }
        };
        endpoint_audit(
            &st,
            &device_id,
            &device_name,
            &tool,
            &sid,
            "打开权限菜单（终端菜单单选）",
            "mode",
            &result_str,
        );
        return response;
    }
    // ===== pick：硬前置（overlay 在屏）→ 发数字键 → 确认框 / 回执 =====
    let Some(number) = req.number else {
        return bad_request();
    };
    let pick_sid = sid.clone();
    let pick_tool = tool.clone();
    let r = tokio::task::spawn_blocking(move || -> Result<PickOutcome, String> {
        let Some(_guard) = crate::inject::queue::try_acquire_inflight(&pick_sid) else {
            return Err("投递进行中，请稍后重试".to_string());
        };
        let key_delay = || {
            std::thread::sleep(std::time::Duration::from_millis(
                crate::inject::families::SUBMIT_DELAY_MS,
            ))
        };
        let mut terminal = crate::inject::mode::Closures {
            read: || read_screen_lines(pid),
            send: |key: &str| {
                let r = injector.locate_and_send_key_spec(pid, key, &spec);
                if r.is_ok() {
                    key_delay();
                }
                r
            },
            settle: key_delay,
        };
        let pick = crate::inject::mode::run_codex_menu_pick(
            number,
            || read_screen_lines(pid),
            || poll_confirm_cluster(pid),
            || {
                std::thread::sleep(std::time::Duration::from_millis(
                    crate::inject::timing::MODE_STEP_MIN_GAP_MS,
                ))
            },
            &mut terminal,
        )?;
        match pick {
            crate::inject::mode::MenuPick::Confirm(cluster) => {
                if cluster.is_empty() {
                    // 确认框在屏但选项读不到 → 如实回执（前端提示重新读取）
                    return Ok(PickOutcome::ConfirmUnreadable);
                }
                let opts: Vec<crate::inject::dialog::DialogOption> = cluster;
                Ok(PickOutcome::Confirm(opts))
            }
            crate::inject::mode::MenuPick::Done { screen } => {
                // 回执核验：屏上是否出现「成功切到某档」的回执行。**目标档未知**
                // （用户点的是屏上编号，后端不知道对应哪个 wire 档）——故按
                // 「锚在屏」判，并把回执行原文回给前端显示（用户自己看得见切到哪档）。
                let seen = crate::inject::mode::permission_receipt_seen(&pick_tool, &screen);
                Ok(PickOutcome::Done {
                    receipt: seen,
                    screen_tail: screen.iter().rev().take(6).cloned().collect::<Vec<_>>(),
                })
            }
        }
    })
    .await;
    let (result_str, response) = match r {
        Ok(Ok(o)) => {
            let body = match &o {
                PickOutcome::Confirm(opts) => {
                    let items: Vec<serde_json::Value> = opts
                        .iter()
                        .map(|x| {
                            serde_json::json!({
                                "number": x.number,
                                "label": x.label,
                                "highlighted": x.highlighted,
                            })
                        })
                        .collect();
                    serde_json::json!({
                        "status": "confirm",
                        "options": items,
                        "dialogChecked": dialog_checked,
                    })
                }
                PickOutcome::ConfirmUnreadable => serde_json::json!({
                    "status": "confirm",
                    "options": [],
                    "hint": "终端已弹出风险确认框，但选项未读到——请点「重新读取」再选",
                    "dialogChecked": dialog_checked,
                }),
                PickOutcome::Done { receipt, .. } => serde_json::json!({
                    "status": "done",
                    // 用户点的是屏上编号，后端不知道对应哪个 wire 档 —— 见 PickOutcome 文档
                    "verified": receipt.is_some(),
                    "hint": match receipt {
                        Some(line) => format!("终端回执：{line}"),
                        None => "已按你点选的编号投递，但未在屏上读到成功回执——请人工核对终端".to_string(),
                    },
                    "dialogChecked": dialog_checked,
                }),
            };
            ("ok".to_string(), json_no_store(StatusCode::OK, body))
        }
        Ok(Err(e)) => (
            format!("failed:{e}"),
            mode_menu_err(StatusCode::OK, "pick_failed", Some(&e)),
        ),
        Err(e) => {
            log::error!("session-mode/menu pick 任务异常: {e}");
            (
                "internal".to_string(),
                mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None),
            )
        }
    };
    endpoint_audit(
        &st,
        &device_id,
        &device_name,
        &tool,
        &sid,
        &format!("终端菜单第 {number} 项（权限切换）"),
        "mode",
        &result_str,
    );
    response
}

/// pick 的走向（端点内部类型）
enum PickOutcome {
    /// 屏上出现 Full Access 二次确认框——选项表交用户点
    Confirm(Vec<crate::inject::dialog::DialogOption>),
    /// 确认框在屏但选项读不到
    ConfirmUnreadable,
    /// 无确认框：`receipt` = 屏上读到的成功回执行原文（None = 未读到）
    Done {
        receipt: Option<String>,
        #[allow(dead_code)]
        screen_tail: Vec<String>,
    },
}

/// codex 菜单选项的生产轮询（picker 的 `open` 用）：窗内读回选项表。
#[cfg(windows)]
fn poll_menu_options(pid: u32) -> Result<Option<Vec<crate::inject::dialog::DialogOption>>, String> {
    let rounds =
        crate::inject::timing::poll_rounds(crate::inject::timing::CODEX_MENU_OPEN_POLL_TOTAL_MS)
            .max(1);
    for i in 0..rounds {
        match crate::inject::windows_console::read_screen_window(pid) {
            Ok(lines) => {
                let lowered: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
                if let Some(opts) = crate::inject::mode::codex_menu_options(&lines, &lowered) {
                    return Ok(Some(opts));
                }
            }
            Err(e) => log::debug!("菜单选项屏读失败（pid={pid}: {e}）"),
        }
        if i + 1 < rounds {
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
    }
    Ok(None)
}

/// 非 Windows：无屏读 → 恒 None（如实降级，与其它屏读路径同口径）。
#[cfg(not(windows))]
fn poll_menu_options(
    _pid: u32,
) -> Result<Option<Vec<crate::inject::dialog::DialogOption>>, String> {
    Ok(None)
}

/// **`GET /m/api/v1/session-mode/menu?session_id=`**——**纯屏读、零注入**：读回当前
/// 屏上的菜单/确认框选项表（供移动端面板的「重新读取」重同步）。
///
/// 与 POST 的 `open` 动作的区别：本端点**不注入任何东西**（不 `/permissions`、不清场），
/// 只回答「此刻屏上有什么」。用途：
/// - 用户手动在终端开了菜单，想在手机上点选；
/// - 上一步读屏竞态（面板显示的选项与终端不一致）时重同步。
///
/// 响应：`{status:"menu", options:[…]}` / `{status:"confirm", options:[…]}` /
/// `{status:"none"}`（屏上无 overlay）。
pub async fn session_mode_menu_read(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(sid) = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return bad_request();
    };
    if device_identity(&st, &headers).is_none() {
        return forbidden_defense();
    }
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let found = tokio::task::spawn_blocking(move || {
        (probe_st.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == probe_sid)
    })
    .await;
    let session = match found {
        Ok(Some(s)) => s,
        Ok(None) => return mode_menu_err(StatusCode::NOT_FOUND, "no_session", None),
        Err(e) => {
            log::error!("session-mode/menu 读取任务异常: {e}");
            return mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None);
        }
    };
    if session.agent_type.tool_id() != "codex" {
        return mode_menu_err(
            StatusCode::CONFLICT,
            "no_mechanism",
            Some("终端菜单选择仅支持 codex"),
        );
    }
    let pid = session.pid;
    let read_sid = sid.clone();
    let read_st = st.clone();
    let r = tokio::task::spawn_blocking(move || -> (String, Vec<serde_json::Value>) {
        let Some(lines) = (read_st.screen_probe)(read_sid.as_str(), pid) else {
            return ("none".to_string(), Vec::new());
        };
        let lowered: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
        match crate::inject::mode::codex_overlay_kind(&lowered) {
            Some(crate::inject::mode::CodexOverlay::PermissionMenu) => {
                let items = crate::inject::mode::codex_menu_options(&lines, &lowered)
                    .unwrap_or_default()
                    .iter()
                    .map(|o| {
                        serde_json::json!({
                            "number": o.number,
                            "label": o.label,
                            "highlighted": o.highlighted,
                        })
                    })
                    .collect();
                ("menu".to_string(), items)
            }
            Some(crate::inject::mode::CodexOverlay::FullAccessConfirm) => {
                // 确认框：用簇解析（与 poll_confirm_cluster 同源）
                let items = crate::inject::dialog::parse_dialog_clusters(&lines)
                    .into_iter()
                    .find(|c| {
                        c.iter().any(|o| {
                            o.label
                                .to_lowercase()
                                .contains(crate::inject::mode::FULL_ACCESS_AFFIRMATIVE_KEYWORD)
                        })
                    })
                    .unwrap_or_default()
                    .iter()
                    .map(|o| {
                        serde_json::json!({
                            "number": o.number,
                            "label": o.label,
                            "highlighted": o.highlighted,
                        })
                    })
                    .collect();
                ("confirm".to_string(), items)
            }
            None => ("none".to_string(), Vec::new()),
        }
    })
    .await;
    match r {
        Ok((status, items)) => json_no_store(
            StatusCode::OK,
            serde_json::json!({ "status": status, "options": items }),
        ),
        Err(e) => {
            log::error!("session-mode/menu 读取任务异常: {e}");
            mode_menu_err(StatusCode::INTERNAL_SERVER_ERROR, "internal", None)
        }
    }
}

pub async fn session_mode_switch(
    State(st): State<Arc<RemoteState>>,
    headers: axum::http::HeaderMap,
    Json(req): Json<SessionModeReq>,
) -> Response {
    let sid = req.session_id.trim().to_string();
    let target = req.target.trim().to_string();
    // 裁7：退役档在此**不可解析**（MamMode::parse 无对应 wire 词）→ 400
    let Some(mode) = crate::inject::mode::MamMode::parse(&target) else {
        return bad_request();
    };
    let requested_group = match req.group.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(g) => match crate::inject::mode::ModeGroupId::parse(g) {
            Some(id) => Some(id),
            None => return bad_request(),
        },
    };
    if sid.is_empty() {
        return bad_request();
    }
    let Some((device_id, device_name)) = device_identity(&st, &headers) else {
        return forbidden_defense();
    };
    // 会话查找 + 组推断 + 序列构造（一个 spawn_blocking；锁纪律同 approve）
    let probe_st = st.clone();
    let probe_sid = sid.clone();
    let lookup = match tokio::task::spawn_blocking(move || -> Result<_, String> {
        let session = (probe_st.session_source)()
            .sessions
            .into_iter()
            .find(|s| s.id == probe_sid)
            .ok_or_else(|| "no_session".to_string())?;
        let tool = session.agent_type.tool_id().to_string();
        // 组推断（旧客户端不带 group 的兼容口径；显式 group 必须存在于结构）
        let group = crate::inject::mode::resolve_group(&tool, requested_group, mode)
            .map_err(|_| "no_mechanism".to_string())?;
        // 该组里这一档是否可选（不可选 = 如实拒绝，附组内给出的原因）
        let spec = crate::inject::mode::mode_structure(&tool)
            .group(group)
            .ok_or_else(|| "no_mechanism".to_string())?;
        if let Some(tier) = spec.tiers.iter().find(|t| t.mode == mode) {
            if !tier.selectable {
                log::debug!("切档不可选（{tool}/{}）：{mode:?}", group.wire());
                return Err("no_mechanism".to_string());
            }
        }
        let plan = crate::inject::mode::mode_switch_plan(&tool, group, mode).map_err(|r| {
            log::debug!("切档不可用（{tool}/{}）：{r:?}", group.wire());
            "no_mechanism".to_string()
        })?;
        // 组内目标档的**屏显标签**（回执与按钮标签同源）
        let label = spec
            .tiers
            .iter()
            .find(|t| t.mode == mode)
            .map(|t| t.label)
            .unwrap_or_else(|| mode.label());
        let readback = crate::inject::mode::mode_readback_supported(&tool);
        // 切档前的当前档（步进组的预期档要按环序算）——屏读一次
        let before = read_mode_from_screen(&probe_st, &session, &tool, readback);
        Ok((session, tool, group, plan, readback, label, before))
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-mode 会话扫描任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    let (session, tool, group, plan, readback, label, before) = match lookup {
        Ok(v) => v,
        Err(code) => {
            let status = if code == "no_session" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            return json_no_store(status, serde_json::json!({ "error": code }));
        }
    };
    // ===== 丁T3 接入① + 批次戊 E3②：投递前拦截判定（单点 [`mode_switch_block`]）=====
    //
    // 屏读在**投递之前**（与机制分派无关——Key/Text/Menu 三路都要过这道门），且在任何
    // 注入调用与任何审计写入之前：拒绝 = 零注入零审计（与 approve/question 端点的
    // 校验失败口径一致）。屏读是阻塞 FFI，故放进 spawn_blocking（与既有端点纪律同）。
    //
    // 探针 None（能力缺失/屏读失败/无簇）→ 放行 + 回执带 dialogChecked:false 如实
    // 标注未检测（裁决与理由见本函数文档与 blocks_control_injection）。
    //
    // **E3② 增 kimi 问答待决判据**：kimi 问答框无编号簇（屏读判不到），「待决」改由
    // 消息尾部形态识别（[`pending_question_tail_index`]，与问答端点同一份判据）——
    // 待决时 kimi 的切档注入（恒含回车）会被问答框解释为「选中高亮项」污染待决态
    // （戊探D ×2）→ **含回车整条硬拒绝**（spec §4）。codex 的问答弹窗是编号对话框，
    // 落在对话框在场格，无需第二判据。判定表：busy 一律放行（裁17）。
    let probe_st = st.clone();
    let probe_pid = session.pid;
    let probe_sid_for_log = sid.clone();
    let dialog_options = match tokio::task::spawn_blocking(move || {
        (probe_st.dialog_probe)(probe_sid_for_log.as_str(), probe_pid)
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            // 探针任务本身 panic（join 失败）：按「无法判定」处理（放行 + 未检测标注），
            // 不把基础设施异常转成对用户的拒绝
            log::error!("session-mode 对话框在场探测任务异常: {e}");
            None
        }
    };
    let dialog_checked = dialog_options.is_some();
    let dialog_blocked = crate::inject::dialog::blocks_control_injection(dialog_options.as_deref());
    // kimi × 问答待决（消息尾部形态；只对 kimi 读页——判定表只消费 kimi 格）
    let question_pending = if tool == "kimi" {
        let ms = st.clone();
        let p_sid = sid.clone();
        let p_tool = tool.clone();
        match tokio::task::spawn_blocking(move || {
            (ms.message_source)(&p_tool, &p_sid, QUESTION_SCAN_TAIL_LIMIT)
                .map(|p| pending_question_tail_index(&p.messages).is_some())
                .unwrap_or(false)
        })
        .await
        {
            Ok(v) => v,
            Err(e) => {
                log::error!("session-mode 问答待决判定任务异常: {e}");
                false
            }
        }
    } else {
        false
    };
    let is_running = crate::inject::queue::is_running(&session.status);
    match crate::inject::mode::mode_switch_block(
        &tool,
        group,
        mode,
        is_running,
        dialog_blocked,
        question_pending,
    ) {
        Some(crate::inject::mode::SwitchBlock::Dialog) => {
            log::debug!(
                "T3 控制类注入被拒：sid={sid} 屏读见编号选项对话框（模式切换零投递零审计）"
            );
            return json_no_store(
                StatusCode::CONFLICT,
                serde_json::json!({
                    "error": "blocked_by_dialog",
                    "reason": DIALOG_BLOCKS_CONTROL_REASON,
                }),
            );
        }
        Some(crate::inject::mode::SwitchBlock::QuestionPending) => {
            log::debug!(
                "E3 待决拦截：sid={sid} kimi 问答待决（切档注入含回车=污染待决态）→ 硬拒绝"
            );
            return json_no_store(
                StatusCode::CONFLICT,
                serde_json::json!({
                    "error": "blocked_by_question",
                    "reason": "终端正有待答的问题——切权限/模式的命令会误选答案，请先在问答卡作答",
                }),
            );
        }
        Some(crate::inject::mode::SwitchBlock::CodexPlanBusy) => {
            log::debug!(
                "codex 模式组运行中拦截（sid={sid} status={:?}）",
                session.status
            );
            return json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "failed",
                    "error": "codex 运行中不接受模式切换（shift+tab），请等回合结束后重试",
                }),
            );
        }
        None => {}
    }
    let spec = crate::inject::families::family_for(&tool)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    let injector = st.injector.clone();
    let pid = session.pid;
    let switch_sid = sid.clone();
    // 注入闭包内的工具名副本（闭包 move 走了 tool，审计与回读还要用原值）
    let tool_for_inject = tool.clone();
    let group_for_inject = group;
    // ===== 注入：三路（Key / Text / Menu 两段式/三段式）=====
    //
    // 忙让位（None 哨兵）表达为外层 Option 的 `?`：None = 忙（不投递亦不落审计），
    // Some(inner) = 真投递（inner 是投递结果）
    let attempt = tokio::task::spawn_blocking(move || {
        let _guard = crate::inject::queue::try_acquire_inflight(&switch_sid)?;
        Some(match plan {
            crate::inject::mode::ModeSwitchPlan::Key(key) => injector
                .locate_and_send_key_spec(pid, key, &spec)
                .map(|()| InjectAttempt::Plain),
            crate::inject::mode::ModeSwitchPlan::Text(cmd) => {
                // 斜杠命令按**纯文本注入 + 提交回车**（不走 [mobile] 前缀——那是用户
                // 消息的语义；斜杠命令是控制指令，加前缀会让命令失效）
                injector
                    .locate_and_inject_spec(pid, cmd, &spec)
                    .and_then(|()| {
                        std::thread::sleep(std::time::Duration::from_millis(
                            crate::inject::families::SUBMIT_DELAY_MS,
                        ));
                        injector.locate_and_send_key_spec(pid, "enter", &spec)
                    })
                    .map(|()| InjectAttempt::Plain)
            }
            crate::inject::mode::ModeSwitchPlan::Menu { open, target } => {
                if tool_for_inject == "codex" {
                    // codex：**编排全权接管**（2026-09-23 数字直达）——段 0 残留防护、
                    // 段 1 开菜单都在 [`run_codex_permission_stages`] 内（open_menu
                    // 闭包注入本命令）。此处**不得**再预注入一遍（同命令两次投递 =
                    // 菜单被 esc/回车错序搅乱——正是本批要消灭的事故形态）。
                    menu_stages(
                        &tool_for_inject,
                        group_for_inject,
                        target,
                        open,
                        pid,
                        &spec,
                        &injector,
                    )
                } else {
                    // kimi 等：第一段先开菜单（命令 + 回车），再进闭环编排
                    let opened = injector
                        .locate_and_inject_spec(pid, open, &spec)
                        .and_then(|()| {
                            std::thread::sleep(std::time::Duration::from_millis(
                                crate::inject::families::SUBMIT_DELAY_MS,
                            ));
                            injector.locate_and_send_key_spec(pid, "enter", &spec)
                        });
                    match opened {
                        Ok(()) => menu_stages(
                            &tool_for_inject,
                            group_for_inject,
                            target,
                            open,
                            pid,
                            &spec,
                            &injector,
                        ),
                        Err(e) => Err(e),
                    }
                }
            }
        })
    })
    .await;
    let result = match attempt {
        Ok(Some(Ok(attempt))) => Ok(attempt),
        // 忙让位（None 哨兵）：不落审计（无投递发生）
        Ok(None) => {
            return json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": "failed",
                    "error": "投递进行中，请稍后重试"
                }),
            );
        }
        Ok(Some(Err(e))) => Err(e),
        Err(e) => {
            log::error!("session-mode 投递任务异常: {e}");
            return json_no_store(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({ "error": "internal" }),
            );
        }
    };
    // 组+档进审计摘要（二维工具的组是语义的一部分：只记「切换至默认」无法区分
    // 是模式组的默认还是权限组的默认）
    let audit_content = if group == crate::inject::mode::ModeGroupId::Mode {
        format!("切换模式至 {label}")
    } else {
        format!("切换{}至 {label}", group.label())
    };
    let (result_str, status_line) = match &result {
        Ok(_) => ("ok".to_string(), "key_sent"),
        Err(e) => (format!("failed:{e}"), "failed"),
    };
    endpoint_audit(
        &st,
        &device_id,
        &device_name,
        &tool,
        &sid,
        &audit_content,
        "mode",
        &result_str,
    );
    match result {
        Err(e) => json_no_store(
            StatusCode::OK,
            serde_json::json!({
                "status": status_line,
                "error": e,
                // 丁T3：失败态也如实携带守卫信息（前端可区分「拒于对话框」与「投递失败」）
                "dialogChecked": dialog_checked,
            }),
        ),
        Ok(attempt) => {
            // ===== 回读确认（裁5：切换后必须知道切到了哪；红线 4：不假装成功）=====
            //
            // **D20(a)：回读必须动态轮询**（宪法 §5.(c) 第 8 条 / 计划 §2.9）。旧实现是
            // 「固定睡 150ms 后单次屏读」→ 屏幕重绘未及就读，读到旧档即报「回读与预期
            // 不符」（用户实机观察①：终端其实已切、刷新浏览器即一致）。内核
            // `mode::poll_mode_readback` 逐拍屏读直到**读到判据本身**（命中即停），
            // 窗尽用最后一次判定定结论——三态语义（命中/不符/未及确认）原样保留。
            //
            // 屏读是阻塞 FFI → 整个轮询放进 spawn_blocking（与其它屏读点同纪律）。回读
            // 发生在投递**之后**：此时 INFLIGHT 已释放（闭包已返回），理论上别的路径可
            // 并发注入——如实申报：回读描述的是「本次投递后**我方读到的**屏」，不承诺
            // 期间无第三方操作（这与 approve 的 A1 确认同口径：确认是证据，不是锁）。
            let verify_st = st.clone();
            let verify_sid = sid.clone();
            let verify_tool = tool.clone();
            let verify_pid = session.pid;
            let expected = crate::inject::mode::expected_mode_after(&tool, group, mode, before);
            // 窗（D20(b) 的有界）：步长 × 拍数全部取自单一事实源 `inject::timing`。
            // **「该组有没有判据」不由本处判**：无回读源时 `expected_mode_after` 给 None，
            // 内核据此只读一拍（判据单点在 `mode::poll_mode_readback` 的「短路径 ②」）
            let rounds = crate::inject::timing::poll_rounds(
                crate::inject::timing::MODE_READBACK_POLL_TOTAL_MS,
            );
            let settle_ms = crate::inject::timing::POLL_STEP_MS;
            let outcome = match tokio::task::spawn_blocking(move || {
                crate::inject::mode::poll_mode_readback(
                    verify_tool.as_str(),
                    expected,
                    rounds,
                    || {
                        read_screen_for_readback(
                            &verify_st,
                            verify_sid.as_str(),
                            verify_pid,
                            readback,
                        )
                    },
                    || {
                        std::thread::sleep(std::time::Duration::from_millis(settle_ms));
                    },
                )
            })
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    log::error!("session-mode 回读任务异常: {e}");
                    crate::inject::mode::ModeReadbackOutcome {
                        verdict: crate::inject::mode::ModeVerify::Unverifiable,
                        observed: None,
                        reads: 0,
                    }
                }
            };
            let observed = outcome.observed;
            let verdict = outcome.verdict;
            // T4 复评 I2：两段式（Menu）的「无法判定」要多带一句「屏读推算、未回读确认」
            // ——定位误判的现实后果是「可能切错档」，不能只回「看不见当前档」
            // 判据在 mode::ModeSwitchPlan::is_two_stage（内核单点）——不在此处写
            // matches!，将来加第三种两段式变体时回执不会静默漏掉限定
            let mode_shot_from_screen = plan.is_two_stage();
            let (verified, hint) =
                mode_verify_receipt(verdict, group, label, mode_shot_from_screen);
            // 菜单路的**额外证据**：工具自己打印的成功回执行（实机原文逐字可见）。
            // 这是**比档位回读更强**的一手证据（回读描述「屏上现在的档位」，
            // 回执描述「工具自己宣布切换完成」）——故它对回执有一票之力：见到且
            // 含目标档 → 直接 verified=true（屏读回读失败也不能抹掉工具的自证）；
            // 未见到 → **不当作失败**，但要把「请人工核对」如实带上（见下）。
            let (verified, hint) = match attempt {
                InjectAttempt::Plain => (verified, hint),
                InjectAttempt::Menu { receipt_seen } => {
                    receipt_and_verdict(receipt_seen, verified, hint, label)
                }
            };
            // 权限档记忆：verified=true 的权限组切换写入「上次切换」表
            // （GET 的权限组 current 回放数据源；见 [`PERMISSION_TIER_MEMORY`]）
            if verified && group == crate::inject::mode::ModeGroupId::Permission {
                remember_permission_tier(&st.store, &sid, mode.wire());
            }
            json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "status": status_line,
                    "verified": verified,
                    "hint": hint,
                    // 屏读到的当前档（前端可直接用它刷新显示，省一次 GET）
                    "current": observed.map(|m| m.wire()),
                    "currentLabel": observed.map(|m| m.label()),
                    // 丁T3：本次是否真的做过对话框在场检测——false = 平台无屏读或屏读失败
                    "dialogChecked": dialog_checked,
                }),
            )
        }
    }
}

/// 菜单路的回执合成：**工具自证**（成功回执行）与**档位回读**两路证据合并。
///
/// 三种组合，逐条写明（判据与文案同源，不散两处）：
/// - 回执**见到**（`Some(true)`）→ `verified=true`、无 hint。工具自己宣布切到了目标档
///   （`• Permissions updated to Full Access` / `Permission mode: Always Ask`），
///   这是比档位回读更强的一手证据（回读只是「屏上现在写着什么档」，可能被后续输出
///   干扰；回执是「工具宣布完成」）——故它**一票通过**，回读失败也抹不掉；
/// - 回执**未见**（`Some(false)`）→ `verified=false` + 「请人工核对」**限定**（即便档位
///   回读命中）：回执可能被后续输出刷走，此时档位回读仍是一手证据——但它只说明
///   「屏上此刻是目标档」，而裁5 要的是「切换真的完成了」。宁可让用户多看一眼，
///   也不把「可能切到但没收到确认」说成成功。**这不是失败**（第二段/第三段都已按
///   闭环完成投递，没有键被盲发）；
/// - **无法核验**（`None`，非 Windows 无屏读）→ 保留档位回读的结论，并在 hint 里
///   追加「本次未核验工具回执」。
fn receipt_and_verdict(
    receipt_seen: Option<bool>,
    verified: bool,
    hint: serde_json::Value,
    label: &str,
) -> (bool, serde_json::Value) {
    match receipt_seen {
        Some(true) => (true, serde_json::Value::Null),
        Some(false) => {
            // 档位回读的既有 hint（若有：不符态会报出「预期/实际」两边）**保留**，
            // 只追加「缺工具回执」这一条——两路证据各说各的，不是二选一
            let base = hint.as_str().map(|s| format!("{s}；")).unwrap_or_default();
            (
                false,
                serde_json::json!(format!(
                    "{base}已按屏读定位完成「{label}」切换投递，但未在屏上见到该工具的成功回执——请人工核对终端"
                )),
            )
        }
        None => (verified, hint),
    }
}

/// 菜单路径的**注入结果**（三路共用；`Menu` 带出核验结果供回执使用）。
/// `Menu` 构造点在 Windows 菜单路径内——非 Windows 编译下按先例条件化 allow。
#[cfg_attr(not(windows), allow(dead_code))]
enum InjectAttempt {
    /// Key / Text 路：投递完成，无额外核验
    Plain,
    /// Menu 路：三/四段全部完成，`receipt_seen` = 是否屏读到工具的成功回执行
    /// （`None` = 非 Windows 无屏读 → 无法核验；回执据此如实说明）
    Menu { receipt_seen: Option<bool> },
}

/// 菜单路径的**第二段 + 第三段 + 成功回执核验**（**只在持 INFLIGHT 的注入闭包内调用**）。
///
/// **按工具分派**（2026-09-23）：codex 走 [`run_codex_permission_stages`] 的**数字
/// 直达**（本函数同时负责其**段 1 开菜单**——端点侧不再预注入）；其余（kimi）先由
/// 端点开菜单、再进 [`run_menu_stages`] 的闭环导航。
///
/// 逐段（每段都可独立中止，中止原因带阶段名——用户看得懂「卡在哪一段、为什么」）：
/// 1. **第二段（权限菜单）**：codex = 菜单锚窗内读目标档**屏上编号** → 发数字键
///    （无回车）；kimi = 轮询等菜单画出（[`MENU_POLL_TOTAL_MS`]）→ **闭环导航**
///    （[`crate::inject::mode::navigate_until_highlighted`]：每发一个方向键重新屏读复核，
///    只有高亮确实落在目标档行才发 `enter`）；
/// 2. **第三段（仅 codex × Full Access）**：轮询等 `Enable full access?` 确认框
///    （[`CONFIRM_POLL_TOTAL_MS`]）→ 肯定项**屏上编号**直达（实测按 `1`）；
/// 3. **成功回执核验**（所有档位）：轮询屏读工具的成功回执行
///    （[`RECEIPT_POLL_TOTAL_MS`]）→ `receipt_seen`。
///
/// # 确认框超时**不当作失败**（与菜单超时的语义差别）
///
/// 菜单超时 = 中止（功能不可用，如实回执）；确认框超时 = **可能是用户此前关过该警告**
/// （codex 二进制有 `Continue and don't warn again.` 文案）→ 若真关过，提交后会
/// **直接完成**，此时屏上只有成功回执行、没有确认框。故此处继续走第 3 步按回执
/// 核验结果如实回执（这正是「不假装成功」的正确形态：**有证据才说成功**）。
///
/// # 中止条件（任一命中即 Err，**此后不再投递任何键**）
///
/// 菜单轮询窗内读不到屏/读不到档位表；目标档标签多行（混入正文）；闭环导航的每一条
/// 保守面（kimi，见该函数文档）；确认框形态异常（肯定项不唯一/多个簇）；键投递本身失败。
/// 返回的 Err 文案即端点的失败回执。
#[allow(clippy::too_many_arguments)]
fn menu_stages(
    tool: &str,
    group: crate::inject::mode::ModeGroupId,
    target: crate::inject::mode::MamMode,
    open_cmd: &str,
    pid: u32,
    spec: &crate::inject::families::FamilySpec,
    injector: &std::sync::Arc<dyn crate::inject::engine::Injector>,
) -> Result<InjectAttempt, String> {
    #[cfg(not(windows))]
    {
        // 非 Windows 无屏读 → 菜单路径无法定位（数字直达与闭环导航都必须屏读）。
        // **如实回执**：不盲发（猜错会选到别的档——与导航的保守面同源）。
        let _ = (tool, group, target, open_cmd, pid, spec, injector);
        Err("本平台无屏读，权限菜单无法定位（命令已发送，请在终端选择档位）".to_string())
    }
    #[cfg(windows)]
    {
        // codex 走**数字直达**编排（2026-09-23 用户实测：菜单内按档位数字键直达；
        // 方向键闭环退役——kimi 菜单无屏上编号，仍走闭环）
        if tool == "codex" {
            let started = std::time::Instant::now();
            let key_delay = || {
                std::thread::sleep(std::time::Duration::from_millis(
                    crate::inject::families::SUBMIT_DELAY_MS,
                ))
            };
            let mut terminal = crate::inject::mode::Closures {
                read: || read_screen_lines(pid),
                send: |key: &str| {
                    let r = injector.locate_and_send_key_spec(pid, key, spec);
                    if r.is_ok() {
                        key_delay();
                    }
                    r
                },
                settle: key_delay,
            };
            let outcome = crate::inject::mode::run_codex_permission_stages(
                target,
                // 段 1：开菜单（机制表下发的 open 命令 + 回车）——残留防护在编排段 0
                || {
                    injector
                        .locate_and_inject_spec(pid, open_cmd, spec)
                        .and_then(|()| {
                            std::thread::sleep(std::time::Duration::from_millis(
                                crate::inject::families::SUBMIT_DELAY_MS,
                            ));
                            injector.locate_and_send_key_spec(pid, "enter", spec)
                        })
                },
                || poll_menu_digit(pid, target),
                || poll_confirm_cluster(pid),
                || poll_receipt(pid, tool, target, RECEIPT_POLL_TOTAL_MS),
                // 相邻步骤硬性 ≥0.5s（用户指令 2026-09-23：轮询+硬控并存取最大）
                || {
                    log::info!(
                        "codex 权限切换：步骤间硬性等待 {}ms",
                        crate::inject::timing::MODE_STEP_MIN_GAP_MS
                    );
                    std::thread::sleep(std::time::Duration::from_millis(
                        crate::inject::timing::MODE_STEP_MIN_GAP_MS,
                    ));
                },
                &mut terminal,
            )?;
            log::info!(
                "codex 权限直达完成（目标 {}；菜单键 {}；确认框 {}；回执 {:?}；总耗时 {}ms，含步骤间硬性 ≥{}ms 间隔）",
                target.label(),
                outcome.menu_keys.join(","),
                if outcome.confirm_done {
                    "已走完"
                } else {
                    "未出现"
                },
                outcome.receipt_seen,
                started.elapsed().as_millis(),
                crate::inject::timing::MODE_STEP_MIN_GAP_MS,
            );
            return Ok(InjectAttempt::Menu {
                receipt_seen: outcome.receipt_seen,
            });
        }
        let read = || read_screen_lines(pid);
        let key_delay = || {
            std::thread::sleep(std::time::Duration::from_millis(
                crate::inject::families::SUBMIT_DELAY_MS,
            ))
        };
        // **段编排在内核**（`mode::run_menu_stages`）——「第三段只对 codex×Full Access
        // 走」「确认框缺席不算失败」「回执核验」这些控制流是判据的一部分，写在
        // `#[cfg(windows)]` 里就只剩实机能覆盖。此处只提供**生产侧的能力**：
        // 轮询（`poll`）、读屏/发键/等待（`MenuTerminal` 的三面）。
        let mut terminal = crate::inject::mode::Closures {
            read,
            // `|key: &str|` 的类型标注是**必须**的：省略参数类型时闭包的生命周期会被
            // 推断成某个具体生命周期，而 `MenuTerminal::send(&str)` 要求对**任意**生命
            // 周期成立（higher-ranked）——缺标注即 `implementation of FnMut is not general
            // enough` 编译错。
            send: |key: &str| {
                let r = injector.locate_and_send_key_spec(pid, key, spec);
                if r.is_ok() {
                    key_delay();
                }
                r
            },
            settle: key_delay,
        };
        let outcome = crate::inject::mode::run_menu_stages(
            tool,
            group,
            target,
            |plan, optional| poll_menu_stage(pid, plan, optional),
            || poll_receipt(pid, tool, target, RECEIPT_POLL_TOTAL_MS),
            &mut terminal,
        )?;
        log::debug!(
            "模式菜单：闭环完成（菜单 {} 键：{}；确认框 {}；回执 {:?}）",
            outcome.menu_keys.len(),
            outcome.menu_keys.join(","),
            if outcome.confirm_done {
                "已走完"
            } else {
                "未出现"
            },
            outcome.receipt_seen
        );
        Ok(InjectAttempt::Menu {
            receipt_seen: outcome.receipt_seen,
        })
    }
}

/// codex 数字直达的**生产轮询**：轮询读屏直到 [`crate::inject::mode::
/// codex_permission_digit_probe`] 读到目标档的屏上编号。
///
/// 窗尽未读到 → `Err`（菜单必须出现且目标档可读，否则功能不可用——与
/// [`poll_menu_stage`] 的 `optional=false` 同语义，advice 同文案）。
#[cfg(windows)]
fn poll_menu_digit(pid: u32, target: crate::inject::mode::MamMode) -> Result<String, String> {
    use crate::inject::mode::PollStep;
    let rounds =
        crate::inject::timing::poll_rounds(crate::inject::timing::CODEX_MENU_OPEN_POLL_TOTAL_MS)
            .max(1);
    let mut last: Option<String> = None;
    for i in 0..rounds {
        match crate::inject::windows_console::read_screen_window(pid) {
            Ok(lines) => match crate::inject::mode::codex_permission_digit_probe(&lines, target) {
                PollStep::Ready(digit) => return Ok(digit),
                PollStep::Fatal(why) => {
                    return Err(format!("{why}；请人工核对终端（命令已发送，档位未切）"))
                }
                PollStep::NotYet(why) => last = Some(why),
            },
            Err(e) => last = Some(format!("权限菜单屏读失败（{e}）")),
        }
        if i + 1 < rounds {
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
    }
    Err(format!(
        "{}；请人工核对终端（命令已发送，档位未切）",
        last.unwrap_or_else(|| "权限菜单窗内未读屏".to_string())
    ))
}

/// codex Full Access 二阶段确认框的**生产轮询**：产出确认框**簇**（供编排取肯定项
/// 的屏上编号直达——与 [`poll_menu_stage`] 的 `optional=true` 同窗同「缺席不当作
/// 失败」语义，但返回簇而不是屏幕行）。
///
/// **平台缝**：屏读实现仅 Windows（`inject/mod.rs` 门控）——非 Windows 侧由下方
/// 桩降级（Err 明示不支持，不装作读到空屏）。
#[cfg(windows)]
fn poll_confirm_cluster(
    pid: u32,
) -> Result<Option<Vec<crate::inject::dialog::DialogOption>>, String> {
    use crate::inject::mode::PollStep;
    let rounds = crate::inject::timing::poll_rounds(CONFIRM_POLL_TOTAL_MS).max(1);
    let mut last: Option<String> = None;
    for i in 0..rounds {
        match crate::inject::windows_console::read_screen_window(pid) {
            Ok(lines) => {
                match crate::inject::mode::confirm_box_probe(
                    &lines,
                    "codex",
                    crate::inject::mode::FULL_ACCESS_AFFIRMATIVE_KEYWORD,
                ) {
                    PollStep::Ready((cluster, _)) => return Ok(Some(cluster)),
                    PollStep::Fatal(why) => {
                        return Err(format!("{why}；请人工核对终端（Full Access 可能未生效）"))
                    }
                    PollStep::NotYet(why) => last = Some(why),
                }
            }
            Err(e) => last = Some(format!("确认框屏读失败（{e}）")),
        }
        if i + 1 < rounds {
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
    }
    log::debug!(
        "Full Access 确认框：窗内未出现（{}）→ 按回执核验",
        last.unwrap_or_default()
    );
    Ok(None)
}

/// [`poll_confirm_cluster`] 的非 Windows 桩：屏读实现仅 Windows——macOS 等平台
/// 走不到这里（codex 菜单拾取端点在非 Windows 拒绝注入），编译期兜底防 cfg 断链，
/// Err 文案如实（不装作读到空屏）。
#[cfg(not(windows))]
fn poll_confirm_cluster(
    _pid: u32,
) -> Result<Option<Vec<crate::inject::dialog::DialogOption>>, String> {
    Err("确认框屏读簇仅 Windows 支持".to_string())
}

/// **平台中立读屏缝**（丁T3 同思路）：仅 Windows 有屏读实现；非 Windows 一律
/// `None`——上层闭包按「读不到屏」自然降级（阶段机中止零按键 / 二元卡），不因
/// 平台差断编译。Windows 侧等价 `windows_console::read_screen_window(pid).ok()`。
#[cfg(windows)]
fn read_screen_lines(pid: u32) -> Option<Vec<String>> {
    crate::inject::windows_console::read_screen_window(pid).ok()
}

/// [`read_screen_lines`] 的非 Windows 桩（见上）。
#[cfg(not(windows))]
fn read_screen_lines(_pid: u32) -> Option<Vec<String>> {
    None
}

/// 菜单路径的**生产轮询**：按阶段取预算，轮询读屏直到 `plan` 可行动。
///
/// - `optional = false`（第二段菜单）：窗尽未出现 → `Err`（菜单必须出现，否则功能不可用）；
/// - `optional = true`（第三段确认框）：窗尽未出现 → `Ok(None)`（**不当作失败**，
///   理由见 [`menu_stages`] 文档的「确认框超时不当作失败」）。
///
/// `Fatal`（形态异常）两种模式都立即 `Err`——等下去也不会自洽。
///
/// **D20 起窗用「拍数」表达**（[`poll_rounds`]；与其余四路同口径）：墙钟 deadline 在
/// 门禁里只能真等满窗，拍数则可在测试里确定性驱动。
#[cfg(windows)]
fn poll_menu_stage(
    pid: u32,
    plan: &crate::inject::mode::MenuNavPlan<'_>,
    optional: bool,
) -> Result<Option<Vec<String>>, String> {
    use crate::inject::mode::PollStep;
    let total_ms = if optional {
        CONFIRM_POLL_TOTAL_MS
    } else {
        MENU_POLL_TOTAL_MS
    };
    let advice = if optional {
        "Full Access 可能未生效"
    } else {
        "命令已发送，档位未切"
    };
    let rounds = crate::inject::timing::poll_rounds(total_ms).max(1);
    // 「为什么还没等到」：每轮**重算**并记在 Option 里（首轮必写；用 Option 而非
    // 初始空串是因为「初值从未被读」的写法会被 lint 抓——本变量只承载最后一轮的
    // 未就绪原因）
    let mut last: Option<String> = None;
    for i in 0..rounds {
        match crate::inject::windows_console::read_screen_window(pid) {
            Ok(lines) => match plan.probe(&lines) {
                PollStep::Ready(_) => return Ok(Some(lines)),
                PollStep::Fatal(why) => return Err(format!("{why}；请人工核对终端（{advice}）")),
                PollStep::NotYet(why) => last = Some(why),
            },
            Err(e) => last = Some(format!("{}屏读失败（{e}）", plan.stage())),
        }
        if i + 1 < rounds {
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
    }
    // `rounds ≥ 1` 已保证至少走过一轮，故 `last` 必有值；`unwrap_or_else` 只是
    // 不去写一个「不可能发生」的 panic（防御式，文案本身仍如实）
    let last = last.unwrap_or_else(|| format!("{}窗内未读屏", plan.stage()));
    if optional {
        // **不当作失败**：用户可能关过该警告（二进制 `Continue and don't warn
        // again.`）→ 由回执核验如实判定
        log::debug!("{}：窗内未出现（{last}）→ 按回执核验", plan.stage());
        return Ok(None);
    }
    Err(format!("{last}；请人工核对终端（{advice}）"))
}

/// 轮询屏读**工具自己的成功回执行**（`receipt_seen`）。
///
/// 窗末仍未见 → `Ok(None)`（**不是失败**：回执可能被后续输出刷走，也可能根本没打印）
/// ——调用方据此下发 `verified:false` + 「请人工核对」（见 [`receipt_and_verdict`]）。
/// 屏读本身失败（读不到屏）也收敛为 `Ok(None)`：我们**没有证据**，故不声称成功。
/// `Err` 只用于「读屏任务本身异常」这一条不会发生的路径（保留 `Result` 与
/// [`crate::inject::mode::run_menu_stages`] 的签名一致）。
#[cfg(windows)]
fn poll_receipt(
    pid: u32,
    tool: &str,
    target: crate::inject::mode::MamMode,
    total_ms: u64,
) -> Result<Option<Vec<String>>, String> {
    let label = crate::inject::mode::menu_target_label(tool, target).unwrap_or("");
    // D20：窗 = 步长 × 拍数（与其余四路同口径）
    let out = crate::inject::timing::bounded_poll(
        crate::inject::timing::poll_rounds(total_ms),
        || {
            crate::inject::windows_console::read_screen_window(pid)
                .ok()
                .filter(|lines| {
                    crate::inject::mode::permission_receipt_verified(tool, lines, label)
                })
        },
        || std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS)),
    );
    if out.is_none() {
        log::debug!("菜单路径：窗内未见成功回执行（{tool}/{label}）→ 按未核验回执");
    }
    Ok(out)
}

/// **投递后的屏读能力**（按 pid 读一屏**原文**）——回读轮询的读入口。
///
/// 与 [`read_mode_from_screen`] 的关系：同一次屏读，区别是**投递后**没有 `&Session`
/// 可借（会话快照已 move 进查表闭包）——故只带 sid/pid 与工具名。
///
/// **走 [`RemoteState::screen_probe`] 缝**（丁T5 引入的屏读**能力**缝，生产装配 =
/// `inject::windows_console::read_screen_window`）：回读轮询的**控制流**（读几拍、
/// 命中即停、窗尽取最后一次判定）是本任务的新判据，缝上能力才能在门禁里用脚本化屏
/// 序列钉住它——本批两次栽在「真机路径没有自动化替代」上，同因同改。
///
/// **返回原文而不是解析结果**：解析
/// （[`crate::inject::mode::parse_mode_from_screen`]）是**判据**，属轮询内核
/// （[`crate::inject::mode::poll_mode_readback`]）——在这里就解析掉的话，端点侧又得为
/// 「读到什么算命中」写一遍判据（同一判据两处实现的老路）。
///
/// `readback=false`（该工具无回读源）→ 直接 `None`：**不读屏**——没有回读源时读到的
/// 任何东西都不构成判据。
fn read_screen_for_readback(
    st: &Arc<RemoteState>,
    sid: &str,
    pid: u32,
    readback: bool,
) -> Option<Vec<String>> {
    if !readback {
        return None;
    }
    match (st.screen_probe)(sid, pid) {
        Some(lines) => Some(lines),
        None => {
            log::debug!("模式回读屏读不可用（sid={sid} pid={pid}）→ 本轮无读数");
            None
        }
    }
}

/// 回读结论 → 回执字段 `(verified, hint)`。
///
/// **文案与判据同源**（不写两遍）：三种结论各有自己的说法——
/// - 命中：`verified=true`、无 hint（前端显示「已切换到 X」）；
/// - **不符**：`verified=false` + 报出「预期 A / 实际 B」（不假装成功，也不含糊说
///   「请核对」——用户需要知道差在哪，才能判断是环序漂移还是注入没生效）；
/// - 无法判定：`verified=false` + 人工核对提示（无回读源/屏读失败）。
///
/// # `mode_shot_from_screen`（T4 复评 I2）：菜单路径的「无法判定」要多带一句限定
///
/// 对**菜单路径**（`mode_shot_from_screen=true`）来说，「无法判定」这一态的含义比单段
/// 注入更弱一层：档位是靠**屏读定位 + 闭环步进**到达的（`locate_menu_items` +
/// `navigate_until_highlighted`），而定位的判据是**文案匹配**（实机取证档支撑，但版本
/// 升级会让文案漂移）——定位一旦误判，方向键就可能落在**别的档**上。此时若只回「无法
/// 确认当前档」，用户会以为「可能没切或切了但看不见」，而真实风险是「**可能切错档**」。
/// 裁5 的「不假装成功」要覆盖这一态，故菜单路径的「无法判定」文案追加「本次为屏读
/// 推算、未回读确认」——把风险说全，让用户知道该去终端核**对**（而不是只核有无）。
///
/// 单段（Key/Text）不追加：它的目标档由命令/按键直接决定（`/plan on` 就是进 Plan，
/// shift+tab 就是前进一档），不存在「定位误判」这条额外风险源。
///
/// **与 [`receipt_and_verdict`] 的关系**：本函数只说「档位回读」这一路的结论；菜单路径
/// 还有**工具自证**（成功回执行）这一路更强的证据，两路在 [`receipt_and_verdict`] 合并。
fn mode_verify_receipt(
    verdict: crate::inject::mode::ModeVerify,
    group: crate::inject::mode::ModeGroupId,
    label: &str,
    mode_shot_from_screen: bool,
) -> (bool, serde_json::Value) {
    use crate::inject::mode::ModeVerify;
    match verdict {
        ModeVerify::Confirmed => (true, serde_json::Value::Null),
        ModeVerify::Mismatch { expected, observed } => (
            false,
            serde_json::json!(format!(
                "回读到的档与预期不符（预期「{}」、实际「{}」）——请人工核对终端{}",
                expected.label(),
                observed.label(),
                if mode_shot_from_screen {
                    "（本次为屏读推算、未回读确认）"
                } else {
                    ""
                }
            )),
        ),
        ModeVerify::Unverifiable => (
            false,
            serde_json::json!(format!(
                "{}的当前档无法自动确认（无回读源或屏读失败）——请人工核对终端{}",
                if group == crate::inject::mode::ModeGroupId::Mode {
                    "模式".to_string()
                } else {
                    format!("{}「{}」", group.label(), label)
                },
                if mode_shot_from_screen {
                    "；本次为屏读推算、未回读确认"
                } else {
                    ""
                }
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::content::SessionMessage;

    /// **E2① 键序档锁**（用户终裁 CL-3 + kimi 数字禁令）：
    /// claude=数字优先+验证回退（渲染等待消除假阴性）；codex=数字直选；
    /// **kimi 恒导航确认**（'2'+Enter 误批准实证未被推翻——数字档是永久禁令）。
    /// 还原动作（变异）：把 kimi 格改成任何数字档 → 本测试先红（安全锁）。
    #[test]
    fn approve_dialog_keys_locks_e2() {
        assert_eq!(
            approve_dialog_keys("claude"),
            ApproveDialogKeys::DigitFirstWithVerify,
            "claude 数字直接选中（用户终裁）+渲染等待+验证回退"
        );
        assert_eq!(approve_dialog_keys("codex"), ApproveDialogKeys::DigitDirect);
        assert_eq!(
            approve_dialog_keys("kimi"),
            ApproveDialogKeys::NavigateConfirm,
            "kimi 计划批准框数字禁令（'2'+Enter 误批准实证）——永久导航档"
        );
        assert_eq!(
            approve_dialog_keys("opencode"),
            ApproveDialogKeys::NavigateConfirm,
            "未取证工具保守导航（到不了此路，纵深防御）"
        );
    }

    /// **2026-09-23 接线回归锁**：opencode 多选 submit 必须分派到
    /// [`StagePlan::OpencodeSubmit`]（首段「Confirm 已在场跳过 tab」→ enter），
    /// 而不是落进 claude 的 Submit 行走位形态（屏读判据在 opencode 屏上必失败）；
    /// advance 分派到单键通道。还原动作：删掉 `(Submit, "opencode")` 分支 → 本测试先红。
    #[test]
    fn stage_plan_dispatch_opencode_submit_is_wired() {
        let q = crate::inject::question::parse_questions(
            r#"{"questions":[{"question":"q","multiSelect":true,"options":[{"label":"a"},{"label":"b"}]}]}"#,
        )
        .unwrap()
        .remove(0);
        assert_eq!(
            StagePlan::for_action(
                crate::inject::question::AnswerAction::Submit,
                None,
                &q,
                "opencode"
            ),
            StagePlan::OpencodeSubmit,
            "opencode 多选 submit 走 OpencodeSubmit 阶段机"
        );
        assert_eq!(
            StagePlan::for_action(
                crate::inject::question::AnswerAction::Submit,
                None,
                &q,
                "claude"
            ),
            StagePlan::Submit { max_down_steps: 4 },
            "claude 多选 submit 维持 Submit 行走位形态（选项 2 + 2）"
        );
        assert_eq!(
            StagePlan::for_action(
                crate::inject::question::AnswerAction::Toggle,
                Some(1),
                &q,
                "claude"
            ),
            StagePlan::ClaudeToggle { target: 1 },
            "claude 多选 toggle 走闭环切勾阶段机（2026-09-24 数字路径废止）"
        );
        assert_eq!(
            StagePlan::for_action(
                crate::inject::question::AnswerAction::Toggle,
                Some(0),
                &q,
                "opencode"
            ),
            StagePlan::SingleKey,
            "opencode toggle 维持单键（enter 切勾，戊探A 定案）"
        );
        assert_eq!(
            StagePlan::for_action(
                crate::inject::question::AnswerAction::Advance,
                None,
                &q,
                "opencode"
            ),
            StagePlan::SingleKey,
            "advance 是单键纯导航（tab），与 select/toggle/cancel 同通道"
        );
    }

    // ==== 批次戊 E2①（评审修复）：DigitFirstWithVerify 执行流三例 ====

    /// 选项表夹具（3 项、高亮在第 1 项——claude 计划批准框形态）
    fn three_options_highlight_first() -> Vec<crate::inject::dialog::DialogOption> {
        vec![
            crate::inject::dialog::DialogOption {
                number: 1,
                label: "Yes, and use auto mode".into(),
                highlighted: true,
            },
            crate::inject::dialog::DialogOption {
                number: 2,
                label: "Yes, manually approve edits".into(),
                highlighted: false,
            },
            crate::inject::dialog::DialogOption {
                number: 3,
                label: "Tell Claude what to change".into(),
                highlighted: false,
            },
        ]
    }

    /// **① 数字成功**：首拍屏读即「对话框消失」（probe → None）→ `Confirmed`、
    /// **零回退键**（数字已生效，不再发任何键）。
    #[test]
    fn e2_digit_verify_confirmed_when_dialog_gone() {
        let sent: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = sent.clone();
        let (outcome, send_err) = verify_digit_then_fallback(
            3,
            || None, // 对话框已消失
            |k: &str| {
                captured
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(k.to_string());
                Ok(())
            },
            || {},
            || {},
            3,
        );
        assert_eq!(
            outcome,
            DigitVerifyOutcome::Confirmed,
            "对话框消失 = 数字生效"
        );
        assert_eq!(send_err, None);
        assert!(
            sent.lock().unwrap_or_else(|e| e.into_inner()).is_empty(),
            "数字已生效 → 零回退键"
        );
    }

    /// **② 数字无效转导航**：窗内对话框恒在场（probe 恒 Some）→ `FellBack`，回退键序
    /// = 从**最后一份选项表**算循环步进（高亮在 1、目标 3 → ↓×2+Enter）。
    /// 还原动作（变异）：内核删掉回退段（窗尽直接返回 Confirmed）→ 本测试先红。
    #[test]
    fn e2_digit_verify_falls_back_to_navigation_when_still_present() {
        let sent: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = sent.clone();
        let (outcome, send_err) = verify_digit_then_fallback(
            3,
            || Some(three_options_highlight_first()), // 恒在场（渲染假阴性形态）
            |k: &str| {
                captured
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(k.to_string());
                Ok(())
            },
            || {},
            || {},
            2,
        );
        assert_eq!(send_err, None);
        match outcome {
            DigitVerifyOutcome::FellBack { keys } => assert_eq!(
                keys,
                vec!["down", "down", "enter"],
                "回退键序 = 循环步进（高亮 1 → 目标 3）"
            ),
            other => panic!("窗尽仍在场必须转导航回退：{other:?}"),
        }
        assert_eq!(
            sent.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            ["down", "down", "enter"],
            "回退键真实发出（经 send_key 闭包）"
        );
    }

    /// **③ 未渲染不注入**：渲染等待窗内 probe 恒 `None`（选项簇未画出）→ 返回
    /// `None` → 端点 `no_mapping` 409 → **零注入**（数字档的假阴性防线——注入早于
    /// 渲染正是丁复审「数字无效 ×5」的根因）；中途画出的（None→Some）→ 命中即返回。
    #[test]
    fn e2_render_wait_never_injects_before_rendered() {
        let calls = std::cell::Cell::new(0u32);
        // 窗内恒未渲染 → None（不注入）
        let got = read_dialog_options_render_wait_with(
            || {
                calls.set(calls.get() + 1);
                None
            },
            2,
        );
        assert_eq!(got, None, "窗尽未渲染 → None（端点据此拒绝注入）");
        assert_eq!(calls.get(), 2, "读满窗（每拍都探测）");
        // 中途渲染（第 2 拍画出）→ 命中即停（D20(a)）
        let calls2 = std::cell::Cell::new(0u32);
        let got2 = read_dialog_options_render_wait_with(
            || {
                calls2.set(calls2.get() + 1);
                if calls2.get() >= 2 {
                    Some(three_options_highlight_first())
                } else {
                    None
                }
            },
            5,
        );
        assert_eq!(got2.map(|o| o.len()), Some(3), "渲染出现即命中");
        assert_eq!(calls2.get(), 2, "命中即停（不多读）");
    }

    /// 纯核夹具：消息构造（kind 是唯一参与判据的字段）
    fn msg(kind: &str) -> SessionMessage {
        SessionMessage {
            seq: 0,
            role: if kind == "user" { "user" } else { "assistant" }.to_string(),
            kind: kind.to_string(),
            content: "x".to_string(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: false,
        }
    }

    fn kinds(msgs: &[&str]) -> Vec<SessionMessage> {
        msgs.iter().map(|k| msg(k)).collect()
    }

    // ==== 丁T2：计划待确认预期态（`plan_pending_tail_index`，无新存储）====

    /// 实机形态（codex 17:19 rollout）：user → assistant → plan（尾部）→ 预期态在场。
    /// 判据 = 返回**最后一条计划类消息的下标**（供审批端点确认「审的是最新那份计划」）。
    #[test]
    fn plan_pending_detected_on_tail_plan() {
        assert_eq!(
            plan_pending_tail_index(&kinds(&["user", "assistant", "plan"])),
            Some(2)
        );
        // plan-file 同为计划类（kimi 的「Write this file?」变体只有文件卡——R1-3 实测）
        assert_eq!(
            plan_pending_tail_index(&kinds(&["user", "plan-file"])),
            Some(1)
        );
        // 混合：取**最近**一条（正文卡与文件卡并存时，后出的是文件卡——kimi 双卡顺序）
        assert_eq!(
            plan_pending_tail_index(&kinds(&["plan", "plan-file"])),
            Some(1)
        );
    }

    /// 预期态的三类清除信号（任务书「下一个用户消息注入或检查未命中时清除」的
    /// 前半——「检查未命中」由端点侧表达为「屏读拿不到选项」的降级，不在这里）：
    /// - 用户消息（注入 `Implement the plan.` 即清除，实机 17:19 line 149）；
    /// - 工具事件（kimi 批准后 wire 立即落 ExitPlanMode 的 tool.call/result——
    ///   用户消息要等下一轮，没有这条判据 kimi 的卡会在批准后挂死）。
    #[test]
    fn plan_pending_cleared_by_user_or_tool_activity() {
        for tail in [
            vec!["plan", "user"],
            vec!["plan", "tool-call"],
            vec!["plan", "tool-result"],
            vec!["plan", "assistant", "user"],
            vec!["plan-file", "tool-call"],
            // 工具事件之后再出计划 → 新的预期态（取最近的计划）
            vec!["plan", "tool-result", "plan"],
        ] {
            let msgs = kinds(&tail);
            let expect = if tail.last() == Some(&"plan") {
                Some(2)
            } else {
                None
            };
            assert_eq!(
                plan_pending_tail_index(&msgs),
                expect,
                "尾序 {tail:?} 的预期态判定"
            );
        }
    }

    /// 无计划消息 / 空页 → None（零误报：普通会话不得凭空出「计划待确认」条）
    #[test]
    fn plan_pending_absent_without_plan_message() {
        assert_eq!(plan_pending_tail_index(&[]), None);
        assert_eq!(
            plan_pending_tail_index(&kinds(&["user", "assistant", "thinking", "tool-call"])),
            None
        );
        // thinking / assistant 之后的计划**不**清除预期态（模型输出不消费提案）
        assert_eq!(
            plan_pending_tail_index(&kinds(&["plan", "thinking", "assistant"])),
            Some(0)
        );
    }

    /// 丁T2 复评 F3-5：**两档判据的差异面**（宽松档供审批门/计划聚合，严档供问答压制）。
    /// 唯一差别 = 计划类消息的认定面：严档不认孤立的 `plan-file`。
    #[test]
    fn strict_plan_pending_excludes_isolated_plan_file_only() {
        // 孤立文件卡：宽松档**认**（审批侧需要——kimi 的 Write this file? 变体只有文件卡），
        // 严档**不认**（问答侧不能因「工具结果提了个计划文件路径」压掉可作答的卡）
        let isolated = kinds(&["user", "plan-file"]);
        assert_eq!(
            plan_pending_tail_index(&isolated),
            Some(1),
            "宽松档认文件卡"
        );
        assert_eq!(
            plan_pending_strict_tail_index(&isolated),
            None,
            "严档不认孤立文件卡（F3-5）"
        );

        // 正文卡在场 → 两档在**在场性**上一致（kimi 的 plan_review **必然**带 display.plan
        // 内联全文，故正文卡必在场）。**只比在场性、不比下标**：两档的下标分别指向
        // 「最后一条计划类消息」与「最后一条正文卡」，而消费方都只用 `is_some()`
        // （下标仅作日志定位，无行为依赖——`plan_from_page` 自己另取）。
        for tail in [
            vec!["user", "plan"],
            vec!["user", "plan", "plan-file"],
            vec!["plan"],
        ] {
            let msgs = kinds(&tail);
            assert_eq!(
                plan_pending_tail_index(&msgs).is_some(),
                plan_pending_strict_tail_index(&msgs).is_some(),
                "正文卡形态两档必须同判（在场性）：{tail:?}"
            );
            assert!(plan_pending_strict_tail_index(&msgs).is_some());
        }

        // 清除信号面两档也一致（其后有 user/tool 事件 → 都 None）
        for tail in [
            vec!["plan", "user"],
            vec!["plan", "tool-result"],
            vec!["plan-file", "tool-call"],
        ] {
            let msgs = kinds(&tail);
            assert_eq!(plan_pending_tail_index(&msgs), None, "{tail:?}");
            assert_eq!(plan_pending_strict_tail_index(&msgs), None, "{tail:?}");
        }
    }

    /// 丁T2 复评 F3-3：**读页条件与判据套用条件解耦**的表征锁——`approve_plan_family`
    /// 必须覆盖 claude（T8 聚合的消费方），`plan_dialog_family` 必须**不**覆盖 claude
    /// （它的审批门走既有 waiting 路径）。两族的成员差异是刻意的，勿合并。
    #[test]
    fn approve_plan_family_and_dialog_family_are_distinct() {
        assert!(
            approve_plan_family("claude"),
            "claude 是 T8 聚合的消费方（F3-3 回归修复）"
        );
        assert!(approve_plan_family("codex"));
        assert!(!approve_plan_family("kimi"), "kimi 审批卡不带正文（F3-4）");
        assert!(!approve_plan_family("opencode"));

        assert!(
            !plan_dialog_family("claude"),
            "claude 的门不放宽（走既有 waiting/标记路径——T1 裁决）"
        );
        assert!(plan_dialog_family("codex"));
        assert!(plan_dialog_family("kimi"));
    }

    // ==== 丁T2 复审 N1：计划预期态下 POST 的输入面判据 ====

    /// N1 纯核：`plan_pending` 为真**只接受** `dialog:<n>`；非预期态恒接受（零回归）。
    ///
    /// 这条锁对应「GET 不下发 → POST 不接受」的对称契约（GET/POST 的两种拒绝在端点上
    /// 都收敛成 `no_mapping`，状态码分不开，故用纯函数把不变式钉住）。
    #[test]
    fn plan_pending_post_accepts_dialog_options_only() {
        // 预期态：只放行屏读选项
        assert!(plan_pending_accepts_option_id(true, "dialog:1"));
        assert!(plan_pending_accepts_option_id(true, "dialog:9"));
        for mapped in ["approve", "reject", "allow", "y", ""] {
            assert!(
                !plan_pending_accepts_option_id(true, mapped),
                "预期态必须拒绝映射表 id（含空串/自定义 id）：{mapped}"
            );
        }
        // **形态判据要严**（防变体绕过）：只有 `dialog:<可解析 u32>` 才放行
        // （`dialog:0` 形态合法但编号不在屏读表内 → 端点现场重解析仍拒——本函数是
        // 必要条件而非充分条件，见其文档）
        assert!(plan_pending_accepts_option_id(true, "dialog:0"));
        for sneaky in [
            "dialog",
            "dialogx:1",
            "xdiialog:1",
            " dialog:1",
            "dialog:",
            "dialog:abc",
            "dialog:1.5",
            "dialog:-1",
        ] {
            assert!(
                !plan_pending_accepts_option_id(true, sneaky),
                "只有形态良好的 `dialog:<n>` 才放行：{sneaky}"
            );
        }
        // 非预期态：既有路径零回归（Waiting/标记态的映射键照常接受）
        for mapped in ["approve", "reject", "dialog:1", "", "anything"] {
            assert!(
                plan_pending_accepts_option_id(false, mapped),
                "非预期态不得改变输入面（零回归）：{mapped}"
            );
        }
    }

    // ==== 丁T2 复审 N2：计划待确认判据的**跨语言共享夹具** ====

    /// N2 真锁：读 `tests/fixtures/plan_pending_cases.json`（与前端 vitest 共用的**唯一
    /// 事实源**）逐例驱动本节判据，断言与共享表里的期望一致。
    ///
    /// **为什么这是「真锁」而不是「人工镜像」**：两侧测试都从**同一个文件**取夹具与期望
    /// ——判据行为变化时，若只改一侧实现而不更新本文件，该侧必红（另一侧仍绿，但漂移
    /// 一定被抓，且抓它的期望来自共享表而非各自硬编码）。这是跨语言可达的最强约束形态。
    ///
    /// 读文件失败的处置：**panic 带明确信息**（不静默跳过）——先例见
    /// `services::pet::error::tests::rpc_codes_have_i18n_keys` 读 `../src/i18n/locales/zh.json`。
    ///
    /// 覆盖：宽松档（审批门/计划聚合）与严档（问答压制，F3-5）。共享表的用例设计为
    /// **两档同判**（不含「孤立 plan-file」——那是两档唯一的分叉点，由
    /// `strict_plan_pending_excludes_isolated_plan_file_only` 专门覆盖），
    /// 故本用例对两个函数都断言同一期望。
    #[test]
    fn plan_pending_cross_language_fixture_cases() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/plan_pending_cases.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "无法读取跨语言共享夹具（{}）: {e}——本判据的一致性测试要求该文件存在",
                path.display()
            )
        });
        let root: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("plan_pending_cases.json 不是合法 JSON: {e}"));
        let cases = root
            .get("cases")
            .and_then(|c| c.as_array())
            .unwrap_or_else(|| panic!("共享夹具缺少 cases 数组"));

        let mut checked_true = 0usize;
        let mut checked_false = 0usize;
        let mut checked_lenient_only = 0usize;
        for case in cases {
            let note = case.get("note").and_then(|n| n.as_str()).unwrap_or("");
            let kinds: Vec<&str> = case
                .get("kinds")
                .and_then(|k| k.as_array())
                .unwrap_or_else(|| panic!("用例缺 kinds（{note}）"))
                .iter()
                .map(|k| k.as_str().unwrap_or_default())
                .collect();
            let expect = case
                .get("pending")
                .and_then(|p| p.as_bool())
                .unwrap_or_else(|| panic!("用例缺 pending 布尔（{note}）"));
            // `lenient_only`：该用例只在**宽松档**成立（两档唯一的分叉点 = 孤立 plan-file，
            // 见 `plan_pending_strict_tail_index` 文档）——两侧都按本字段分别断言两档
            let lenient_only = case
                .get("lenient_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let msgs: Vec<SessionMessage> = kinds.iter().map(|k| msg(k)).collect();
            assert_eq!(
                plan_pending_tail_index(&msgs).is_some(),
                expect,
                "宽松档与共享夹具不符（kinds={kinds:?}；{note}）"
            );
            assert_eq!(
                plan_pending_strict_tail_index(&msgs).is_some(),
                expect && !lenient_only,
                "严档与共享夹具不符（kinds={kinds:?}；lenient_only={lenient_only}；{note}）"
            );
            if expect {
                checked_true += 1;
            } else {
                checked_false += 1;
            }
            if lenient_only {
                assert!(expect, "lenient_only 只能标在宽松档为真的用例上（{note}）");
                checked_lenient_only += 1;
            }
        }
        // 用例集自检：两类都非空 + 分叉面有覆盖（防「夹具被删空后测试恒绿」的退化）
        assert!(
            checked_true > 0 && checked_false > 0,
            "共享夹具必须同时含在场与不在场用例（实得 true={checked_true} false={checked_false}）"
        );
        assert_eq!(
            checked_lenient_only, 1,
            "两档分叉面（孤立 plan-file）必须恰有一例（防分叉逻辑被静默删掉）"
        );

        // 工具族收窄：非 codex/kimi 一律不参与（两侧同名单）
        let tools = root
            .get("non_plan_tools")
            .and_then(|t| t.as_array())
            .unwrap_or_else(|| panic!("共享夹具缺少 non_plan_tools"));
        for t in tools {
            let t = t.as_str().unwrap_or_default();
            assert!(
                !plan_dialog_family(t),
                "共享夹具的 non_plan_tools 含计划对话框族成员：{t}"
            );
        }
    }

    // ==== 丁T2：审批卡计划聚合的「同一事件正文优先」 ====

    /// 计划聚合夹具（content 即 kind 语义的可辨串）
    fn plan_pair(kinds: &[&str]) -> Vec<SessionMessage> {
        kinds
            .iter()
            .map(|k| {
                let mut m = msg(k);
                m.content = match *k {
                    "plan" => "# markdown 正文",
                    "plan-file" => "C:/u/plans/p.md",
                    _ => "x",
                }
                .to_string();
                m
            })
            .collect()
    }

    /// 丁T2：kimi 的一条 `interaction.request(plan_review)` 同产正文卡与文件卡
    /// （`content::kimi_plan_cards` 固定顺序：正文在前、文件卡在后）——聚合必须取
    /// **正文**（markdown 比路径更完整），不得因文件卡更靠后而降级成路径提示。
    #[test]
    fn plan_from_page_prefers_body_adjacent_to_file_card() {
        // kimi 形态：tool-result(...plan-file) → [plan, plan-file]（同事件两半）
        let b = plan_from_page(&plan_pair(&["user", "plan", "plan-file"])).unwrap();
        assert_eq!(b.content, "# markdown 正文");
        assert!(!b.is_file, "同事件的正文优先（不降级为路径）");

        // 只有文件卡（kimi 的 T7 引用形态 / display.plan 缺失）→ 仍给路径
        let b = plan_from_page(&plan_pair(&["user", "plan-file"])).unwrap();
        assert_eq!(b.content, "C:/u/plans/p.md");
        assert!(b.is_file);

        // 只有正文卡（claude/codex）→ 给 markdown
        let b = plan_from_page(&plan_pair(&["user", "plan"])).unwrap();
        assert_eq!(b.content, "# markdown 正文");
        assert!(!b.is_file);
    }

    /// **错配反锁**：紧邻性判据防「上一版正文 + 这一版文件卡」被错凑成一对——
    /// 中间隔着别类消息（不是同一事件的两半）时，文件卡照旧给路径。
    #[test]
    fn plan_from_page_does_not_pair_distant_body_with_file_card() {
        let b = plan_from_page(&plan_pair(&["plan", "tool-result", "plan-file"])).unwrap();
        assert_eq!(
            b.content, "C:/u/plans/p.md",
            "正文与文件卡不紧邻（中间隔了别类消息）→ 不配对，取最新一条（文件卡）"
        );
        assert!(b.is_file);

        // 无计划类消息 → None（零误报）
        assert!(plan_from_page(&plan_pair(&["user", "assistant", "tool-call"])).is_none());
        assert!(plan_from_page(&[]).is_none());
    }

    /// 计划对话框族收窄（codex/kimi）——放宽审批门**只对这两家**（改 claude 的门
    /// 等于改既有行为，超出本批范围；opencode 无独立计划机制）
    #[test]
    fn plan_dialog_family_is_codex_and_kimi_only() {
        assert!(plan_dialog_family("codex"));
        assert!(plan_dialog_family("kimi"));
        for other in [
            "claude",
            "opencode",
            "zcode",
            "dsh",
            "workbuddy",
            "openclaw",
            "",
        ] {
            assert!(
                !plan_dialog_family(other),
                "{other} 不属计划对话框族（门不放宽——既有行为零改动）"
            );
        }
    }

    // ===== 丁T3 裁2：斜杠命令审计 action 选择的纯核锁 =====

    /// `audit_action_for` 表驱动（裁2 的判据与注入形态单点同源）：
    /// `/` 开头 → slash；其余 → 传入的基准动作。
    #[test]
    fn audit_action_for_routes_slash_only_for_slash_commands() {
        for (text, base, want) in [
            ("/permissions", "send", "slash"),
            ("/plan", "queue", "slash"),
            ("\x1b/permissions", "send", "slash"),
            ("/", "queue", "slash"),
            ("普通消息", "send", "send"),
            ("普通消息", "queue", "queue"),
            ("价格 /permissions 是多少", "queue", "queue"),
            (" /permissions", "send", "send"),
        ] {
            assert_eq!(
                audit_action_for(text, base),
                want,
                "判据格：{text:?}（基准 {base}）"
            );
        }
    }

    // ==== 丁T4 复评 I2：回执文案的「屏读推算」限定 ====

    /// **两段式（Menu）的「无法判定」必须带「本次为屏读推算、未回读确认」**（裁5 的
    /// 「不假装」要覆盖「可能切错档」这一态，不只是「不知道切没切」）。
    ///
    /// 还原动作：把 `mode_verify_receipt` 的 `mode_shot_from_screen` 参数去掉（或恒传
    /// false）→ 本测试先红（hint 里缺那句限定）。
    #[test]
    fn two_stage_receipt_carries_screen_inference_caveat() {
        use crate::inject::mode::{MamMode, ModeGroupId, ModeVerify};
        let group = ModeGroupId::Permission;
        // 两段式 × 无法判定（权限组无回读源 → 实机常态）
        let (verified, hint) =
            mode_verify_receipt(ModeVerify::Unverifiable, group, "完全信任", true);
        assert!(!verified, "无回读源 → 不得声称已确认");
        let h = hint.as_str().unwrap();
        assert!(
            h.contains("屏读推算"),
            "两段式必须说明档位是推算来的（可能切错档）：{h}"
        );
        assert!(h.contains("未回读确认"), "并说明未回读：{h}");
        assert!(h.contains("请人工核对终端"), "仍要给核对指引：{h}");
        // 单段（Key/Text）× 无法判定：**不带**该限定（目标档由命令直接决定，
        // 不存在「定位误判」这条额外风险源）——成对锁，防有人把限定一律加上去
        let (_, hint_single) =
            mode_verify_receipt(ModeVerify::Unverifiable, group, "完全信任", false);
        let hs = hint_single.as_str().unwrap();
        assert!(
            !hs.contains("屏读推算"),
            "单段注入没有定位误判风险 → 不加该限定：{hs}"
        );
        // 两段式 × 不符：同样带限定（此时「预期」本身就是推算值）
        let (verified, hint) = mode_verify_receipt(
            ModeVerify::Mismatch {
                expected: MamMode::Bypass,
                observed: MamMode::ReadOnly,
            },
            group,
            "完全信任",
            true,
        );
        assert!(!verified);
        let h = hint.as_str().unwrap();
        assert!(
            h.contains("预期「完全信任」") && h.contains("实际「只读」"),
            "{h}"
        );
        assert!(h.contains("屏读推算"), "不符态也要说明预期是推算的：{h}");
        // 命中：无 hint（前端显示「已切换」）——两段式**永远走不到这一态**（无回读源），
        // 但判据本身要保持三态齐全
        let (verified, hint) = mode_verify_receipt(ModeVerify::Confirmed, group, "完全信任", true);
        assert!(verified);
        assert!(hint.is_null());
    }

    // ==== 丁T4 收尾：成功回执核验 → 回执合成的三条判据（`receipt_and_verdict`）====

    /// **工具自证 = 一票通过**（比档位回读更强的一手证据）：见到
    /// `• Permissions updated to <目标档>` → `verified=true`、无 hint——**即便档位回读
    /// 那一路判了「无法判定」**（权限组无回读源，实机常态）。
    ///
    /// 还原动作：把 `receipt_and_verdict` 的 `Some(true)` 分支改成 `(verified, hint)`
    /// （即让回读覆盖回执）→ 本测试先红（verified 会是 false）。
    #[test]
    fn menu_receipt_seen_wins_over_readback_unverifiable() {
        // 实机形态：权限组无底栏回读 → verdict=Unverifiable → hint 是「请人工核对」
        let (rb_verified, rb_hint) = mode_verify_receipt(
            crate::inject::mode::ModeVerify::Unverifiable,
            crate::inject::mode::ModeGroupId::Permission,
            "完全信任",
            true,
        );
        assert!(!rb_verified, "无回读源时档位回读这一路本就是 false");
        let (verified, hint) = receipt_and_verdict(Some(true), rb_verified, rb_hint, "完全信任");
        assert!(
            verified,
            "见到工具自己的成功回执 → 一票通过（工具宣布切换完成，强于「屏上现在写着什么」）"
        );
        assert!(hint.is_null(), "命中时无 hint（前端显示「已切换」）");
    }

    /// **未见回执 → verified=false + 「请人工核对」**，但**不是失败**（`status` 仍
    /// `key_sent`，编排本身已成功走完）；且**保留档位回读那一路的说法**（两路证据各说
    /// 各的，不是二选一）。
    ///
    /// 还原动作：把 `Some(false)` 分支改成 `(verified, hint)` → 第一条断言先红
    /// （档位回读命中时 verified 会是 true，等于把「可能切到但没收到确认」说成成功）。
    #[test]
    fn menu_receipt_absent_reports_unverified_but_keeps_readback_note() {
        // ① 档位回读命中（有回读源时才会发生）——但回执未见 → 仍不得声称成功
        let (verified, hint) =
            receipt_and_verdict(Some(false), true, serde_json::Value::Null, "只读");
        assert!(
            !verified,
            "未见工具回执 → 不把「屏上此刻写着目标档」说成「切换已完成」（裁5）"
        );
        let h = hint.as_str().unwrap();
        assert!(h.contains("未在屏上见到该工具的成功回执"), "{h}");
        assert!(h.contains("请人工核对终端"), "要给核对指引：{h}");
        assert!(h.contains("只读"), "点名目标档（用户知道该核对什么）：{h}");
        // ② 档位回读那一路有话说（不符态）→ 两句话都在（不是把回读的话丢掉）
        let (_, rb_hint) = mode_verify_receipt(
            crate::inject::mode::ModeVerify::Mismatch {
                expected: crate::inject::mode::MamMode::Bypass,
                observed: crate::inject::mode::MamMode::ReadOnly,
            },
            crate::inject::mode::ModeGroupId::Permission,
            "完全信任",
            true,
        );
        let (verified, hint) = receipt_and_verdict(Some(false), false, rb_hint, "完全信任");
        assert!(!verified);
        let h = hint.as_str().unwrap();
        assert!(
            h.contains("预期「完全信任」") && h.contains("实际「只读」"),
            "回读的话保留：{h}"
        );
        assert!(
            h.contains("未在屏上见到该工具的成功回执"),
            "回执的话也保留：{h}"
        );
        // ③ 无法核验（非 Windows 无屏读）→ 原样透传档位回读的结论
        let (verified, hint) = receipt_and_verdict(None, true, serde_json::Value::Null, "只读");
        assert!(verified, "无屏读时回执核验不参与，交给档位回读");
        assert!(hint.is_null());
    }
}

/// 丁T3 **实机三场景占位**（`#[ignore]`——常规门禁只编译不跑）。
///
/// 三场景都无法在无真实 CLI 会话的前提下完成（需要真实 conhost 窗口 + 真实 TUI
/// 进入对话框态 + 人工观察），故按批次丁计划「实机测试一律 `#[ignore]`」的口径：
/// **先落占位与观测点清单，把实跑时要抄录/断言的东西写死在注释里**。
///
/// 跑法（实机显式，单线程避免终端互相干扰）：
/// `cargo test --lib t3_live_probe -- --ignored --nocapture --test-threads=1`
///
/// **占位的边界（如实申报）**：本模块与 `inject::question::live_probe_tests` 同款——
/// 只做**前置可满足性检查与探测指引打印**，不发起任何注入或 HTTP 请求（那些需要受控
/// 会话与人工观察窗口，由本机人工按清单执行）。三场景的自动化替代面已在门禁内覆盖：
/// ①的守卫两路（假体屏读）见 `remote::server::tests::mode_switch_*`；②的前端分流见
/// `tests/mobile/MessageComposer.test.tsx` 的「丁T3 卡片在场分流」族；③的裸注入与
/// 审计见 `remote::server::tests::slash_message_*`。**未被自动化覆盖的只有「真终端
/// 上是否真的不再发生图5/6/7」这一终极观测**——那正是本模块要人工去跑的部分。
#[cfg(test)]
mod t3_live_probe_tests {
    /// 场景① **待决点模式钮 → 拒**（图5/问题 5）。
    ///
    /// 前置：Windows + claude（或 codex）已装；真 conhost 窗口里跑一个会话，
    /// 让终端停在**待决对话框**态（claude：计划批准框 `❯ 1. Yes, and use auto mode`
    /// ——探测档案 `screen-t5-claude-plan-before.txt`；或 AUQ 多选题；codex：
    /// `Implement this plan?`）。MAM 远程服务开启，手机端在详情页。
    ///
    /// 观测点（逐条抄录）：
    /// 1. 点模式栏的切档钮 → 回执必须是 **409 `blocked_by_dialog`** + 文案
    ///    「终端有待决对话框，请先处理」（不再是「已发送切换」）；
    /// 2. 终端屏幕**逐行比对无变化**（对话框仍开、高亮项未动——实机注入过的证据是
    ///    shift+tab 会移动高亮/切档、斜杠命令会变成选择题输入）；
    /// 3. 审计页**无新行**（拒绝=零注入零审计；这是与「投递失败」可分的关键口径）；
    /// 4. 处理掉对话框（在终端选一项）后再点同一钮 → 正常投递（守卫不得把正常路径
    ///    也拒掉——门禁内已有 `mode_switch_proceeds_when_dialog_absent` 锁）。
    #[test]
    #[ignore = "实机验证：真 claude/codex 会话停在待决对话框 + 点模式钮 → 409 blocked_by_dialog + 屏幕逐行无变化 + 零审计"]
    fn t3_dialog_blocks_mode_switch_live_probe() {
        eprintln!(
            "丁T3 实机探测占位（场景① 待决点模式钮 → 拒）\n\
             \n\
             前置检查：\n\
             - 平台 = {}（屏读是 Windows 能力；非 Windows 下守卫按「无法判定」放行，\
             本场景不可观测——那不是缺陷，是能力边界，回执会带 dialogChecked:false）\n\
             - claude CLI 版本 = {:?}\n\
             - codex CLI 版本 = {:?}\n\
             \n\
             步骤（人工）：\n\
             1. 真 conhost 里起 claude，要求它给一个计划并停在批准框（屏上应见 \
             「Claude has written up a plan… ❯ 1. Yes, and use auto mode」）；\n\
             2. 手机端进该会话详情页，点模式栏「切换模式」或 codex 的「计划」钮；\n\
             3. 抄录回执 JSON（期望 409 + error=blocked_by_dialog + reason=终端有待决对话框，请先处理）；\n\
             4. 抄录终端屏幕（与操作前逐行比对——必须完全相同）；\n\
             5. 查审计页/审计表（期望**无** action=mode 新行）；\n\
             6. 在终端选掉对话框，再点同一钮（期望 200 key_sent，恢复正常）。\n\
             \n\
             定案后落点：本占位改为断言式（读审计表 + 屏读前后比对），或升级为 \
             tests/m9r_e2e.rs 的真 HTTP 全链用例（先例 e2e_http_full_chain）。",
            std::env::consts::OS,
            crate::inject::approve::cached_cli_version("claude"),
            crate::inject::approve::cached_cli_version("codex"),
        );
    }

    /// 场景② **composer 发送 → 自由作答**（图6/问题 6：多选框 Enter=切换高亮项）。
    ///
    /// 前置：Windows + claude；终端停在 **AskUserQuestion 单选题**（屏上应见
    /// `N. Type something.` 那一行 + 编号选项）。
    ///
    /// 观测点：
    /// 1. 手机端输入区的 placeholder/提示条应显示「作为回答发送」（问答卡在场分流）；
    /// 2. 点发送 → **不回执 delivered**，而是提示条 + 诚实回执（自由作答序列尚未
    ///    实机定案，本端不代发——见 `MessageComposer` 的常量文案）；
    /// 3. 终端屏幕**无变化**（关键：放行的旧行为会让 `1` 被勾选/反勾——图6 实锤）；
    /// 4. 在**问答卡输入框**作答（T5 交付）或在终端作答，终端正常进入下一题/收尾。
    ///
    /// **本任务只交付「分流与拦截」**（任务书原文），自由作答注入序列的实机定案
    /// 归后续批次——定案后本场景的第 2 条观测改写成「点发送 → 注入自由作答序列 →
    /// `Type something` 行被选中并提交答案」。
    #[test]
    #[ignore = "实机验证：真 claude 单选 AUQ 待决 + composer 发送 → 分流提示（不代发）+ 终端屏幕无变化（不做成勾选）"]
    fn t3_composer_free_text_split_live_probe() {
        eprintln!(
            "丁T3 实机探测占位（场景② composer 发送 → 自由作答分流）\n\
             \n\
             前置检查：\n\
             - 平台 = {}\n\
             - claude CLI 版本 = {:?}\n\
             \n\
             步骤（人工）：\n\
             1. 真 conhost 里起 claude，让它提一个单选问题（AskUserQuestion），\
             终端停在对话框上；\n\
             2. 手机端进详情页，观察输入区提示条（期望 data-presence=question，\
             placeholder=「输入内容将作为回答发送（对话框待决）」）；\n\
             3. 输入任意自由文本并点发送 → 抄录回执（期望拦截文案，**零** session-send \
             请求到达服务端）；\n\
             4. 抄录终端屏幕（与操作前逐行比对——**不得**出现任何选项被勾选/反勾，\
             这是问题 6 的验收点）；\n\
             5. 用问答卡作答（或终端作答）验证正常路径不受影响。\n\
             \n\
             **未覆盖面申报**：自由作答注入序列（claude 定位 Type something→文本→Enter；\
             codex tab notes；opencode 选 own answer→文本→Enter；kimi Other/feedback）\
             全部待实机定案（批次丁计划 §2.4）——本任务按任务书只做分流与拦截，\
             定案后在此补第三段观测（序列注入生效）。",
            std::env::consts::OS,
            crate::inject::approve::cached_cli_version("claude"),
        );
    }

    /// 场景③ **移动端 `/permissions` → 触发 + 审计**（图7/问题 8：前缀毁命令）。
    ///
    /// 前置：Windows + codex（`/permissions` 是 codex 的权限档命令，批次丙 T6 有
    /// 命令证据）；终端**空闲可输入**（无待决对话框——否则会先撞场景①的守卫）。
    ///
    /// 观测点：
    /// 1. 手机端 composer 输入 `/permissions` 并发送 → 回执 delivered/queued；
    /// 2. **终端屏幕上命令原样出现**（`/permissions`，**无** `[mobile …]` 签名——
    ///    有签名就会变成「找不到命令」或被当普通消息，图7 实锤）；
    /// 3. codex 的模式/权限切换发生（底栏权限文本变化，或弹出权限菜单——后者属
    ///    T4 的两段式范围，本场景只验「命令被识别」）；
    /// 4. 审计页按设备名可查：新行 `action=slash`、`summary=/permissions`、
    ///    `device_name=<该设备>`（裁2：终端不留痕，溯源只此一处）。
    #[test]
    #[ignore = "实机验证：真 codex 空闲会话 + 移动端发 /permissions → 终端原样收到裸命令 + 审计 action=slash 可按设备查"]
    fn t3_slash_bare_injection_and_audit_live_probe() {
        eprintln!(
            "丁T3 实机探测占位（场景③ 移动端 /permissions → 触发 + 审计）\n\
             \n\
             前置检查：\n\
             - 平台 = {}\n\
             - codex CLI 版本 = {:?}\n\
             \n\
             步骤（人工）：\n\
             1. 真 conhost 里起 codex（版本需与批次丙 T6 的命令证据档一致），\
             确认终端空闲可输入（无待决对话框）；\n\
             2. 手机端 composer 发 `/permissions`；\n\
             3. 抄录终端屏幕（期望出现裸 `/permissions`，**无** [mobile …] 签名）；\n\
             4. 按命令的实际效果观察（权限菜单/底栏文本变化——如实抄录，\
             两段式选择属 T4 范围，此处只验命令被识别）；\n\
             5. 打开桌面端审计页，按设备名过滤，确认新行 action=slash + \
             summary=/permissions（**这是斜杠命令唯一的溯源留痕**）。\n\
             \n\
             回归面：同设备再发一条普通消息 → 终端应见 `正文 [mobile 设备名]`（签名在尾）\
             （签名**在尾部**）——两个形态并存是裁2 的完整语义；且该消息的确认戳\
             不因尾部签名与其他同设备消息假命中（门禁内已由 \
             inject::confirm::tests::stamp_never_false_hits_across_same_device_messages \
             锁住）。",
            std::env::consts::OS,
            crate::inject::approve::cached_cli_version("codex"),
        );
    }
}

/// 丁T4 收尾 **实机形态复核**占位（`#[ignore]`——常规门禁只编译不跑）。
///
/// # 本模块的两条用例各去取什么（**与代码一致，不做超出证据的宣称**）
///
/// 丁T4 收尾的核心改动（权限菜单关键词定位、闭环导航、Full Access 第三段）**依据的是
/// 用户手工实机取证**（`research/refs/phase2-消息注入/2026-09-22-codex-kimi权限菜单与
/// FullAccess三段式-用户实机取证.md`），该档是**用户逐字转抄的屏幕原文**，不是 MAM
/// 自己注入路径下的产物。两条用例各自去补一层：
///
/// 1. [`t4_permission_menu_screen_shape_live_probe`]：菜单/确认框/回执行的**逐行原文**
///    与 `menu_labels`、`FULL_ACCESS_AFFIRMATIVE_KEYWORD`、两个回执行锚是否**逐字一致**
///    （版本升级后最先漂移的就是文案）；
/// 2. [`t4_four_tools_switch_and_readback_live_probe`]：**MAM 自己的注入路径**下逐家切一轮
///    ——取证档是「人手敲命令」，不覆盖 MAM 的文本分块、`SUBMIT_DELAY_MS`、轮询窗与
///    闭环按键节拍。
///
/// 两条都**只做前置可满足性检查与探测指引打印**，不发起任何注入或 HTTP 请求（那需要
/// 受控会话与人工观察窗口）。**未被自动化覆盖的只有「真终端上这套键序是否真的切对档」
/// 这一终极观测**——其余每一格都有门禁内的自动化替代（见下表的对应关系）。
///
/// 跑法（实机显式，单线程避免终端互相干扰）：
/// `cargo test --lib t4_live_probe -- --ignored --nocapture --test-threads=1`
#[cfg(test)]
mod t4_live_probe_tests {
    /// **权限菜单 / Full Access 确认框 / 成功回执行的实机形态复核**（丁T4 收尾新增）。
    ///
    /// # 为什么必须有这条（本批的两次夹具教训）
    ///
    /// T4 首版的菜单定位判据基于**推演**（「权限菜单不带编号，用行首标签匹配」），实机
    /// 取证显示 codex 的菜单行是 `› 2. Ask for approval (current) …`——标签**不在行首**
    /// → 真机定位 0 项、权限切换**完全不可用**（如实中止，不误选）。现在的判据改成
    /// 「关键词包含 + 计数互斥」，夹具全部换成取证档的**逐字原文**；但**版本升级仍会
    /// 让文案漂移**（档位词、`(current)` 后缀、`continue` 肯定项、回执行锚），故需要一条
    /// 把「屏上原文 ↔ 词表 ↔ 锚」逐字对账的用例。
    ///
    /// # 前置（逐家）
    ///
    /// - **Windows + 真 conhost 窗口**（屏读是 Windows 能力；非 Windows 下菜单路径恒
    ///   `Err`「本平台无屏读」，本场景不可观测——那是能力边界，回执会如实说明）；
    /// - codex / kimi 已装且**空闲可输入**（无待决对话框——否则先撞丁T3 守卫，那是另一条
    ///   用例）；Guardian 开启与否会影响 `Approve for me` 是否在场（两种都合法）；
    /// - MAM 远程服务开启（若要跑注入路径的两条）。
    ///
    /// # 观测点（**逐条抄录「屏上原文」，与下面的期望值逐字比对**）
    ///
    /// | # | 动作（人工在终端里做） | 期望屏上原文 | 期望解析结果 |
    /// |---|---|---|---|
    /// | 1 | codex 手打 `/permissions` 回车 | 标题 `Update Model Permissions`；四档 `1. Read Only` … `4. Full Access`（Guardian 关则无 `Approve for me`）；高亮行带 `› `；当前档带 ` (current)` 后缀；描述在**同行右侧列**（折行续行是纯缩进） | 喂 `locate_menu_items(&lines, menu_labels("codex"))` → **4 项**（或 3 项，Guardian 关）、顺序 = 屏上序、恰好一项 `highlighted` |
    /// | 2 | kimi 手打 `/permission` 回车 | 标题 `Select permission mode`；提示行 `↑↓ navigate · Enter select · Esc cancel`；三档 `Always Ask` / `Ask When Needed` / `Never Ask`，**每档下面紧跟一行描述**；高亮带 `❯ `；当前档带 ` ← current` 后缀 | 喂 `locate_menu_items(&lines, menu_labels("kimi"))` → **3 项**、第 1 项 `highlighted`（若默认档不是「总是询问」则高亮随实际档移动） |
    /// | 3 | codex 在菜单里选到 `4. Full Access` 回车 | 二次确认框标题 `Enable full access?`；`› 1. Yes, continue anyway  Apply full access for this session` / `  2. Cancel                Go back without enabling full access` | `MenuNavPlan::ConfirmAffirmative{tool:"codex", keyword:"continue"}.probe(&lines)` → `Ready`，目标行标签 = 屏上第 1 项的原文；若该版本把肯定项改成别的词（如 `Continue and don't warn again.`），记下实际词并回填 `FULL_ACCESS_AFFIRMATIVE_KEYWORD` |
    /// | 4 | codex 提交后 | `• Permissions updated to Full Access`（1/2/3 档同形，档名不同） | `permission_receipt_verified("codex", &lines, "Full Access")` → `true`；锚文本若有变（如 `Permissions updated to` 改了大小写/词序），回填 `CODEX_PERMISSION_RECEIPT_ANCHOR` |
    /// | 5 | kimi 任选一档回车 | `Permission mode: Always Ask`（档名随所选） | `permission_receipt_verified("kimi", &lines, "Always Ask")` → `true`；锚变了回填 `KIMI_PERMISSION_RECEIPT_ANCHOR` |
    /// | 6 | codex 切 **1/2/3 档**（不选 Full Access） | **不出现** `Enable full access?`，直接回执行 | 这印证 [`crate::inject::mode::needs_full_access_confirm`] 的「只对 Bypass 为真」；若某版本给 1/2/3 也加了确认框 → 那是**判据失效**，必须回填 |
    ///
    /// # 门禁内的自动化替代（跑本用例之前先确认它们绿）
    ///
    /// | 观测点 | 自动化替代（夹具 = 取证档逐字原文） |
    /// |---|---|
    /// | 1 | `inject::mode::tests::locate_codex_menu_items_from_real_screen` / `numbering_is_not_a_criterion` / `distractor_lines_are_handled_by_the_two_gates` |
    /// | 2 | `inject::mode::tests::locate_kimi_menu_items_from_real_screen` |
    /// | 3 | `full_access_confirm_locates_affirmative` / `full_access_confirm_refuses_ambiguous_or_missing` / `full_access_confirm_beats_menu_overlay` |
    /// | 4 / 5 | `permission_receipt_requires_anchor_and_target_label` |
    /// | 6 | `full_access_confirm_is_codex_permission_bypass_only` |
    ///
    /// # 本用例的边界（如实申报）
    ///
    /// 只打印前置检查（平台 + 两家版本）与上述清单，**不做自动断言**——真机观测需要人工
    /// 在终端上看着屏幕逐条对。抄录完成后把差异回填本模块文档的表与对应的常量/词表。
    #[test]
    #[ignore = "实机验证：codex /permissions 与 kimi /permission 的菜单/确认框/回执行逐行原文 → 对账 menu_labels、FULL_ACCESS_AFFIRMATIVE_KEYWORD 与两个回执锚"]
    fn t4_permission_menu_screen_shape_live_probe() {
        eprintln!(
            "丁T4 收尾 实机形态复核（权限菜单 / Full Access 确认框 / 成功回执行）\n\
             \n\
             前置检查：\n\
             - 平台 = {}（屏读是 Windows 能力；非 Windows 下菜单路径恒 Err「本平台无屏读」）\n\
             - codex 版本 = {:?}\n\
             - kimi 版本 = {:?}\n\
             \n\
             当前代码依据的原文（**取自用户实机取证档，本用例就是去核它是否仍逐字成立**）：\n\
             - codex 档位词 = Read Only / Ask for approval / Approve for me / Full Access；\n\
               高亮标记 `›`(U+203A)；当前档后缀 ` (current)`；标题 `Update Model Permissions`；\n\
             - kimi 档位词 = Always Ask / Ask When Needed / Never Ask；高亮标记 `❯`(U+276F)；\n\
               当前档后缀 ` ← current`；标题 `Select permission mode`；\n\
             - 第三段肯定项关键词 = `continue`（覆盖 `Yes, continue anyway` 与\n\
               `Continue and don't warn again.`）；仅 codex × 权限组 × Full Access 有此段；\n\
             - 回执行锚：codex `permissions updated to`（原文 `• Permissions updated to X`）、\n\
               kimi `permission mode:`（原文 `Permission mode: X`）。\n\
             \n\
             步骤（人工，逐条；抄录「屏上原文」那一列）：\n\
             1. codex 手打 /permissions 回车 → 抄录弹窗逐行原文（标题/档位行/描述列/高亮标记/\n\
                当前档后缀），确认四档词与上面逐字一致；\n\
             2. 同一份原文喂 crate::inject::mode::locate_menu_items(&lines, menu_labels(\"codex\"))\n\
                → 期望项数 = 屏上档位数、恰好一项 highlighted；不一致就把真实原文回填\n\
                inject::mode 的夹具与 menu_labels；\n\
             3. 在菜单里移到第 4 项回车 → 抄录确认框逐行原文 → 喂\n\
                MenuNavPlan::ConfirmAffirmative{{tool:\"codex\", keyword:\"continue\"}}.probe(&lines)\n\
                → 期望 Ready；若肯定项不是 continue 类文案，回填 FULL_ACCESS_AFFIRMATIVE_KEYWORD；\n\
             4. 确认后抄录成功回执行 → 喂 permission_receipt_verified 核对锚；\n\
             5. 再切 1/2/3 档各一次 → 确认**不出现**确认框（档案 §3），只有回执行；\n\
             6. kimi 手打 /permission → 同样抄录与对账（三档 + 每档下的描述行 + ← current 后缀）。\n\
             \n\
             定案后落点：inject::mode 的 menu_labels / 测试夹具 / FULL_ACCESS_AFFIRMATIVE_KEYWORD\n\
             / CODEX_PERMISSION_RECEIPT_ANCHOR / KIMI_PERMISSION_RECEIPT_ANCHOR，以及本模块文档的表。",
            std::env::consts::OS,
            crate::inject::approve::cached_cli_version("codex"),
            crate::inject::approve::cached_cli_version("kimi"),
        );
    }

    /// 四家各切一轮 + 回读逐例核对（§2.6 表 + 裁5 验收：切换后回显当前模式）。
    ///
    /// # 与上一条的分工
    ///
    /// 上一条核「屏上原文 ↔ 词表/锚」；本条约核「**MAM 注入路径**下这套键序真的能切对档」
    /// ——取证档是用户**手敲命令**得到的（人工按 ↑/Enter），它证明的是「菜单长什么样、
    /// 高亮在哪」，**不代表 MAM 的注入节拍**（文本分块、`SUBMIT_DELAY_MS`、轮询窗、
    /// 闭环每步的重绘复核）。这两层证据缺一不可。
    ///
    /// # 前置（逐家）
    ///
    /// - Windows + 真 conhost 窗口（屏读是 Windows 能力；非 Windows 下回读恒 null，
    ///   本场景不可观测——回执会如实带 verified=false + 人工核对提示）；
    /// - 四家 CLI 已装（claude / codex / kimi / opencode），MAM 远程服务开启，
    ///   手机端在该会话详情页；
    /// - 终端**空闲可输入**（无待决对话框——否则先撞丁T3 守卫，那是另一条用例）。
    ///
    /// # 观测点（逐家逐例抄录；「屏上原文 → GET current → 回执 verified+hint」三列）
    ///
    /// | # | 工具 | 动作 | 期望屏上 | 期望回执 |
    /// |---|---|---|---|---|
    /// | 1 | opencode | 点「切换模式」一次 | `Plan`（原 `Build`） | `verified=true` + GET 回显「计划」 |
    /// | 2 | claude | 点「切换模式」一次（从 acceptEdits 起） | `⏸ plan mode on` | `verified=true` + 回显「计划」 |
    /// | 3 | codex | 点模式组「计划」 | 底栏 `Plan mode (shift+tab to cycle)` | `verified=true` + 回显「计划」 |
    /// | 4 | kimi | 点模式组「计划」 | 底栏 `plan` 前缀 | `verified=true` + 回显「计划」 |
    /// | 5 | codex | 点权限组「只读」 | 菜单弹出 → 高亮逐步移到 `Read Only`（闭环每步复核） | `verified=true`（见到 `• Permissions updated to Read Only`）、`hint=null` |
    /// | 6 | codex | 点权限组「完全信任」 | 菜单 → `Full Access` → **二次确认框** → 确认 → 回执行 | `verified=true`（见到 `• Permissions updated to Full Access`）；**若用户关过该警告**（确认框不出现）→ 仍应走通回执核验（不应报失败） |
    /// | 7 | kimi | 点权限组「总是询问」 | 菜单弹出 → 高亮移到 `Always Ask` | `verified=true`（见到 `Permission mode: Always Ask`） |
    /// | 8 | kimi | 点权限组「永不询问」 | 底栏出现 `Never Ask`（若有） | `/auto` 直达（无回读源 → 回读那一路 null，`verified` 由**回执核验**决定） |
    ///
    /// # 必须如实抄录的反例（如实申报面）
    ///
    /// - **kimi 漂移**（§2.6 表注）：计划批准后**自动切出 plan**——批准一次后立刻 GET 一次
    ///   模式，屏上 `plan` 前缀应消失；若解析器仍报 Plan，即为漂移未被捕获
    ///   （本用例的观测量，不是期望的成功态）；
    /// - **codex 运行中**：在 codex 干活时点模式组「计划」→ 期望 **200 failed**
    ///   「运行中不接受 /plan」且终端**无变化**（如实回执，不是静默失败）；
    /// - **codex 退出计划模式**：模式组「默认」在 UI 上是**不可点**的灰字（带原因）
    ///   ——这条本身就是验收点（若它能点，说明前端渲染漏了 selectable 判断）；
    /// - **权限菜单的每一步都要在终端肉眼确认高亮确实移动了**——闭环若中止，回执会给出
    ///   具体原因（「高亮未移动，终端可能未响应」/「位移了 N 行」等），把它原样抄下来：
    ///   那正是这条用例最有价值的观测（说明实机的按键节拍与我们假设的不同）。
    ///
    /// **本用例的边界（如实申报）**：只打印前置检查与清单，**不做自动断言**——真机
    /// 观测需要人工在终端上看着屏幕逐条对（自动化替代面见下表的对应关系）。抄录完成后
    /// 把差异回填本模块文档的表与 `inject::timing` 的对应常量（**D20 起时序常量的
    /// 单一事实源在那里**），再决定是否升级为 `tests/m9r_e2e.rs` 的真 HTTP 全链用例。
    ///
    /// **时序读数归 D20 探针**（分工：本用例核**形态与档位**，D20 探针量**耗时**）：
    /// 「动作 → 判据出现」的毫秒数由 `inject::timing` 的 `d20_live_probe_tests` 三条
    /// 用例承担（跑法 `cargo test --lib d20_live_probe -- --ignored --nocapture`），
    /// 回填落点是那里的常量 + `timing::tests::timing_constants_are_pinned`。
    ///
    /// # 门禁内的自动化替代（跑这里之前先确认它们绿）
    ///
    /// | 观测点 | 自动化替代 | 位置 |
    /// |---|---|---|
    /// | 1–4 底栏 → 当前档 | 分族解析单测（夹具 = T6 真机屏幕原文逐字） | `inject/mode.rs` `parse_*_real_footers` |
    /// | claude 环序 | `cycle_next_follows_measured_ring` | 同上 |
    /// | 两组结构 / 裁7 legacy / 裁6 默认 | GET 载荷断言 + 前端渲染断言 | `remote/server.rs` `session_mode_reports_*` + `tests/mobile/ModeBar.test.tsx` |
    /// | 菜单路径守卫只过一道 | `session_mode_switch_menu_guard_runs_once_before_first_stage` | `remote/server.rs` |
    /// | 5–7 菜单/确认框的定位与闭环 | `locate_*_from_real_screen`、`closed_loop_*`、`full_access_confirm_*`、`stage_flow_*` | `inject/mode.rs` |
    /// | 8 回执核验与回执合成 | `permission_receipt_requires_anchor_and_target_label` + `menu_receipt_*` | `inject/mode.rs` / `remote/api.rs` |
    /// | codex 运行中门 | `session_mode_switch_reports_codex_plan_busy` | `remote/server.rs` |
    #[test]
    #[ignore = "实机验证：opencode/claude/codex/kimi 各切一轮（含 codex 三段式 Full Access、kimi 两段式），回读与回执逐例核对；含 kimi 批准漂移与 codex 运行中两个反例"]
    fn t4_four_tools_switch_and_readback_live_probe() {
        eprintln!(
            "丁T4 实机探测占位（四家各切一轮 + 回读/回执逐例）\n\
             \n\
             前置检查：\n\
             - 平台 = {}（屏读是 Windows 能力；非 Windows 下回读恒 null，回执带 verified=false）\n\
             - claude 版本 = {:?}\n\
             - codex 版本 = {:?}\n\
             - kimi 版本 = {:?}\n\
             - opencode 版本 = {:?}\n\
             \n\
             步骤（人工，逐家；抄录三列：屏上原文 / GET current / 回执 verified+hint）：\n\
             1. opencode：点「切换模式」→ 底栏 Build→Plan；GET 回显「计划」；\n\
             2. claude：点「切换模式」→ 底栏 ⏸ plan mode on；GET 回显「计划」；\n\
             3. codex：点模式组「计划」→ 底栏 Plan mode (shift+tab to cycle)；回显「计划」；\n\
             4. kimi：点模式组「计划」→ 底栏 plan 前缀；回显「计划」；\n\
             5. codex：点权限组「只读」→ 菜单弹出，闭环逐步把高亮移到 Read Only → 回执\n\
                `verified=true`（见到 • Permissions updated to Read Only）；\n\
             6. codex：点权限组「完全信任」→ 菜单移到 Full Access → **二次确认框**\n\
                （Enable full access?）→ 闭环移到 Yes, continue anyway → 提交 →\n\
                回执 `verified=true`（见到 • Permissions updated to Full Access）；\n\
             7. kimi：点权限组「总是询问」→ 菜单闭环移到 Always Ask → 回执 verified=true\n\
                （见到 Permission mode: Always Ask）；\n\
             8. kimi：点权限组「永不询问」→ /auto 直达；回读那一路 null，verified 由回执核验决定；\n\
             9. 反例 A（kimi 漂移）：在 kimi 计划批准框选 Approve → 立刻 GET 一次 →\n\
                底栏 plan 前缀应消失（若仍报 Plan = 漂移未捕获，如实记入台账）；\n\
             10. 反例 B（codex 运行中）：codex 干活时点模式组「计划」→ 期望 200 failed\n\
                 「运行中不接受 /plan」且终端无变化；\n\
             11. 反例 C（codex 退出计划）：模式组「默认」应为**灰字不可点**（带原因），\n\
                 点不动才是对的。\n\
             \n\
             **中止分支的抄录要求**：闭环的任一中止都会带中文原因（哪一段、为什么、建议怎么\n\
             做）——把它连同当时的屏幕原文一起抄下来。中止**不是缺陷**（安全面：宁可不切也\n\
             不盲提交），但它是校准自裁常量的唯一证据（D20 起三窗与回读窗的取值、\n\
             自裁说明与实测项都在 inject::timing——见该模块的模块文档与\n\
             d20_live_probe_tests；本用例只管形态与档位）。\n\
             \n\
             定案后落点：把逐例差异回填本模块两条用例的文档表与对应常量，再决定是否升级为\n\
             tests/m9r_e2e.rs 的真 HTTP 全链用例（先例 e2e_http_full_chain）。",
            std::env::consts::OS,
            crate::inject::approve::cached_cli_version("claude"),
            crate::inject::approve::cached_cli_version("codex"),
            crate::inject::approve::cached_cli_version("kimi"),
            crate::inject::approve::cached_cli_version("opencode"),
        );
    }
}
