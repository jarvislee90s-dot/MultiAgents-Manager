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

/// 400 bad_request（缺参 / 空 text / 超长）
fn bad_request() -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "error": "bad_request" })),
    )
        .into_response()
}

/// 403 防御（gate 已拦设备，理论不可达——handler 直取 cookie 失败时兜底）
fn forbidden_defense() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "error": "forbidden" })),
    )
        .into_response()
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
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "error": "internal" })),
                )
                    .into_response();
            }
        };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "no_session" })),
        )
            .into_response();
    };
    // ④ 路由判定（W3 纯核；platform = 本机 OS）。不可注入 → 403（带原因），不入队
    let tool = session.agent_type.tool_id().to_string();
    if let crate::inject::routing::RouteOutcome::NotInjectable {
        reason_code,
        reason,
    } = crate::inject::routing::route(&tool, session.form, session.pid, std::env::consts::OS)
    {
        return (
            StatusCode::FORBIDDEN,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({
                "error": "not_injectable",
                "reason": reason,
                "reasonCode": reason_code,
            })),
        )
            .into_response();
    }
    // ⑤ 组装（W1 来源标记 + 裁决 6 归一在入队时一次完成）并入队（FIFO 保序）
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
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "internal" })),
        )
            .into_response();
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
                    "send",
                    "ok",
                );
                (
                    StatusCode::OK,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "status": "delivered" })),
                )
                    .into_response()
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
                    "send",
                    "unconfirmed",
                );
                (
                    StatusCode::OK,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "status": "submitted" })),
                )
                    .into_response()
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
                    "send",
                    &format!("failed:{e}"),
                );
                (
                    StatusCode::OK,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "status": "failed", "error": e })),
                )
                    .into_response()
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
                    "queue",
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
                (
                    StatusCode::OK,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({
                        "status": "queued",
                        "itemId": item_id,
                        "position": pos,
                    })),
                )
                    .into_response()
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
        "queue",
        "ok",
    );
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(queued),
    )
        .into_response()
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
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "error": "internal" })),
                )
                    .into_response();
            }
        };
    let Some(session) = session else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "no_session" })),
        )
            .into_response();
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
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(body),
    )
        .into_response()
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
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "items": views })),
    )
        .into_response()
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
        (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "status": "failed", "error": e })),
        )
            .into_response()
    };
    // 前查归属（借 retract 语义）+ 取整行（点名投递需要 item 字段），无 pending 项 → 404
    let target = st.store.with(|c| {
        crate::database::dao::inject_queue::pending_for_session_conn(c, &sid)
            .into_iter()
            .find(|i| i.id == item_id)
    });
    let Some(item) = target else {
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response();
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
        crate::inject::queue::FlushOutcome::Sent => (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "status": "delivered" })),
        )
            .into_response(),
        // D7/T3：Submitted 仅直发确认分诊产出（插队以占用排空定论，本臂实际不可达，
        // 为穷尽性保留）——防御性回中性 submitted，不冒充 delivered 也不冒充 failed
        crate::inject::queue::FlushOutcome::Submitted => (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "status": "submitted" })),
        )
            .into_response(),
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
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({
                    "status": "queued",
                    "itemId": item_id,
                    "position": pos,
                })),
            )
                .into_response()
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
        (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "status": "failed", "error": e })),
        )
            .into_response()
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
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response();
    };
    let removed = st
        .store
        .with(|c| crate::database::dao::inject_queue::retract_conn(c, &sid, item_id));
    if !removed {
        // 前查后竞态被他人删走：按 404 语义（DAO 二次校验未命中）
        return (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": "not_found" })),
        )
            .into_response();
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
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
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
    let no_store = [(axum::http::header::CACHE_CONTROL, "no-store")];
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
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            no_store,
            Json(serde_json::json!({ "error": "too_large" })),
        )
            .into_response();
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
                return (
                    StatusCode::NOT_FOUND,
                    no_store,
                    Json(serde_json::json!({ "error": "no_session" })),
                )
                    .into_response();
            }
            Err(e) => {
                log::error!("session-attachment 会话扫描任务异常: {e}");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    no_store,
                    Json(serde_json::json!({ "error": "internal" })),
                )
                    .into_response();
            }
        };
    let cwd = session.project_path.trim().to_string();
    if cwd.is_empty() {
        // 与 resume 的 no_cwd 同源口径（该会话没有项目目录信息）
        return (
            StatusCode::NOT_FOUND,
            no_store,
            Json(serde_json::json!({ "error": "no_cwd" })),
        )
            .into_response();
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                no_store,
                Json(serde_json::json!({ "error": "io" })),
            )
                .into_response();
        }
        Err(e) => {
            log::error!("session-attachment 写盘任务异常: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                no_store,
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response();
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
    (
        StatusCode::OK,
        no_store,
        Json(serde_json::json!({
            "path": path.to_string_lossy(),
            "size": size,
        })),
    )
        .into_response()
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
}

/// 对话框选项屏读（批次丙 T5）：Windows 屏读可见窗口 → 解析编号选项行。
///
/// 返回 None 的所有路径（调用方落回映射表二元卡——红线 3 降级）：
/// 非 Windows / 屏读失败（Err）/ 解析不出连续编号簇（<2 或 >9 项）。
///
/// 屏读只对**已命中审批等待**的会话调用（调用点已判 hit），故不会对空闲会话白读
/// 一屏；失败仅记 debug 日志（不打断审批流）。
fn read_dialog_options(
    session: &crate::session::Session,
) -> Option<Vec<crate::inject::dialog::DialogOption>> {
    #[cfg(windows)]
    {
        match crate::inject::windows_console::read_screen_window(session.pid) {
            Ok(lines) => match crate::inject::dialog::parse_dialog_options(&lines) {
                Some(opts) => {
                    log::debug!(
                        "T5 屏读解析出 {} 个对话框选项（pid={}）",
                        opts.len(),
                        session.pid
                    );
                    Some(opts)
                }
                None => {
                    log::debug!("T5 屏读无连续编号簇（pid={}）→ 降级二元卡", session.pid);
                    None
                }
            },
            Err(e) => {
                log::debug!("T5 屏读失败（pid={}: {e}）→ 降级二元卡", session.pid);
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        // macOS 无屏读能力 → 恒降级（红线 4 同款语义：不假装成功）
        let _ = session;
        None
    }
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
    if session.status != crate::session::SessionStatus::Waiting && !marked {
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
        });
    }
    // T4：标记路径跳过 marker detect（钩子是一等信号，提示文本不落会话文件的
    // 平台上 detect 恒 miss——macOS 红卡由此可达）；marker detect 降级为无标记
    // 时的旧路径（Windows 屏读/文本命中形态）
    let hit = marked
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
        read_dialog_options(&session)
    } else {
        None
    };
    // 选项序列化只取 id+label（key 是投递层机密，不进任何 UI 载荷）；未命中 → options 空
    // （契约：available=false 一律不给选项，移动端据此不渲染审批卡）
    let (options, dialog) = match dialog_options {
        Some(opts) => (
            opts.iter()
                .map(|o| (format!("dialog:{}", o.number), o.label.clone()))
                .collect::<Vec<_>>(),
            true,
        ),
        None => (
            if hit {
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
    Some(ApproveScanHit {
        available: hit,
        options,
        verified_with: mapping.verified_with,
        tool,
        reason: None,
        dialog,
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
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CACHE_CONTROL, "no-store")],
                    Json(serde_json::json!({ "error": "internal" })),
                )
                    .into_response();
            }
        };
    let (available, options, verified_with, tool, reason, dialog) = match scan {
        Some(hit) => (
            hit.available,
            hit.options,
            hit.verified_with,
            Some(hit.tool),
            hit.reason,
            hit.dialog,
        ),
        // 不可批三态（无会话 / 非 Waiting / 无映射）同形：available=false，版本字段空，
        // 无 reason（前端按卡自隐处理，与严格档 reason 提示条区分）
        None => (false, Vec::new(), String::new(), None, None, false),
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
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
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
        })),
    )
        .into_response()
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
/// - 映射表无该工具映射 / optionId 无对应项 / probe-pending 严格档（M9R Task 10：
///   未取证不出键，按键位映射缺失处理）→ 404 no_mapping（降级提示走普通发送）；
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
        if session.status != crate::session::SessionStatus::Waiting && !marked {
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
            let opts = read_dialog_options(&session).ok_or("no_mapping")?;
            if !opts.iter().any(|o| o.number == n) {
                return Err("no_mapping");
            }
            // 合成一个「键 = 数字字符」的选项（复用下方既有投递与审计管线）
            return Ok((
                session,
                tool,
                crate::inject::approve::ApproveOption {
                    id: probe_opt.clone(),
                    label: format!("对话框选项 {n}"),
                    key: n.to_string(),
                },
            ));
        }
        let Some(option) =
            mapping.and_then(|m| crate::inject::approve::option_by_id(&m, &probe_opt).cloned())
        else {
            return Err("no_mapping");
        };
        Ok((session, tool, option))
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-approve 会话扫描任务异常: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response();
        }
    };
    let (session, tool, option) = match lookup {
        Ok(v) => v,
        Err(code) => {
            let status = if code == "not_waiting" {
                StatusCode::CONFLICT
            } else {
                StatusCode::NOT_FOUND
            };
            return (
                status,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": code })),
            )
                .into_response();
        }
    };
    // F2：按键按该会话工具取族规格（先 family_for 再 FALLBACK 兜底，与 Task 5
    // try_flush 同款）——族表未收录的工具（路由层已拦）走快消费者默认口径
    let spec = crate::inject::families::family_for(&tool)
        .unwrap_or(crate::inject::families::FALLBACK_SPEC);
    // M11：会话路由含无头通道时，此处按 approve option 分派 control_message 而非按键
    let injector = st.injector.clone();
    let pid = session.pid;
    let key = option.key.clone();
    let approve_sid = sid.clone();
    // 守卫与投递同生命周期于 spawn_blocking 闭包内（fff9c29 flush 循环事件臂同款，F1
    // 断连双投修复）：handler 断连（弱网/隧道掐断慢投递）不再提前释放守卫——detached
    // 投递全程占位，新触发取不到名额即让位。忙 → None 哨兵：让位不投递亦不落审计
    // （无投递发生不写审计——retract/jump 忙让位同口径）
    let attempt = tokio::task::spawn_blocking(move || {
        // `?` = 守卫忙（try_acquire_inflight 得 None）→ 闭包哨兵返回 None：让位不投递
        let _guard = crate::inject::queue::try_acquire_inflight(&approve_sid)?;
        Some(injector.locate_and_send_key_spec(pid, &key, &spec))
    })
    .await;
    let sent = match attempt {
        Ok(Some(v)) => v,
        Ok(None) => {
            // in-flight 守卫忙（与 flush 循环/直发/插队共用）→ 让位，200 failed 提示
            // 重试（不双投；无投递发生故不写审计——忙让位同口径）
            return (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({
                    "status": "failed",
                    "error": "投递进行中，请稍后重试"
                })),
            )
                .into_response();
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
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "key_sent" })),
            )
                .into_response()
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
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "failed", "error": e })),
            )
                .into_response()
        }
    }
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
    // 通道 B（兜底）：会话消息尾部找最后一条**问答形态** tool-call。
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
    let page = (st.message_source)(&tool, session_id, QUESTION_SCAN_TAIL_LIMIT).ok()?;
    let msgs = &page.messages;
    let last = msgs.iter().rposition(|m| {
        m.kind == "tool-call"
            && m.tool_args
                .as_deref()
                .is_some_and(|a| crate::inject::question::parse_questions(a).is_some())
    })?;
    // 答完判据（可行口径，注释申报）：tool-call 之后**任何** tool-result 在场即视为
    // 已答（AUQ 的 tool_result 无论正常作答/自由文本/Esc 拒绝都会落盘——探测档案 §3
    // 三形态；对应消息粒度比对需回读会话全文，快照尾部窗口内「其后无任何 tool-result」
    // 是保守充分的替代口径）。其后无 tool-result + tool-call 已落盘 = 未答在场
    //（pending 不落盘的场景通道 B 天然不可见，不误触发——空闲态注入风险由双通道
    // 的在场判定收敛，残余风险见探测档案 K11 讨论）。
    if msgs[last + 1..].iter().any(|m| m.kind == "tool-result") {
        return None;
    }
    // 形态判据（工具名只作日志线索——codex/openclaw 等改名不影响接住）
    log::debug!(
        "问答通道 B 形态命中: tool={:?}（工具名不参与判据）",
        msgs[last].tool_name
    );
    let qs = crate::inject::question::parse_questions(msgs[last].tool_args.as_deref()?)?;
    Some((
        session,
        QuestionScanHit {
            questions: qs,
            source: "scan",
        },
    ))
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response();
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
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({
            "available": !questions.is_empty(),
            "answerable": answerable,
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
        })),
    )
        .into_response()
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
}

/// POST /m/api/v1/session-question/answer（T8 问答应答）：参数校验 → 会话/隔离/双通道
/// 复核（spawn_blocking）→ 序列构造（inject::question 纯函数）→ in-flight 守卫下逐键
/// 投递（spawn_blocking 闭包内取守卫，断连双投洞封闭——approve 同款）→ 审计 answer。
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
    // select/toggle 必带序号；submit/cancel 不消费序号（带了也无害，不校验）
    if matches!(
        action,
        crate::inject::question::AnswerAction::Select
            | crate::inject::question::AnswerAction::Toggle
    ) && req.index.is_none()
    {
        return bad_request();
    }
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
        // 「结论不超证据」（探测档案：多问题翻页键序未测）——多问题不出手，前端
        // 渲染只读卡引导终端作答；本分支是直调 API 的兜底防线
        if hit.questions.len() != 1 {
            return Err("multi_questions");
        }
        let q = hit.questions.into_iter().next().unwrap_or_else(|| {
            // unreachable（上面已判 len==1），防御性占位——序列构造会因选项越界拒绝
            crate::inject::question::Question {
                header: String::new(),
                question: String::new(),
                multi_select: false,
                options: Vec::new(),
            }
        });
        // T3：按**会话工具**分发键序（claude 全键序 / opencode·kimi 单选已验 /
        // codex 等未验工具只读——「未验不出键」，见 question_key_profile 的表）
        let tool_id = session.agent_type.tool_id();
        let seq = crate::inject::question::answer_key_sequence_for(tool_id, action, req.index, &q)
            .map_err(|e| {
                // 未验工具的只读拒绝走 multi_questions 之外的码：前端按 409 只读卡兜底
                log::debug!("问答键序不可用（{tool_id}）: {e}");
                if question_profile_is_read_only(tool_id) {
                    "tool_readonly"
                } else {
                    "bad_index"
                }
            })?;
        Ok((session, seq))
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            log::error!("session-question/answer 会话扫描任务异常: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response();
        }
    };
    let (session, sequence) = match lookup {
        Ok(v) => v,
        Err(code) => {
            // 校验失败零注入零审计（approve guards 同口径）：
            // - no_question（409）：会话不在快照 / 审批标记隔离 / 双通道均未命中
            //   ——统一 409（不给存在性预言机；审批标记在场时本就不得出问答键）
            // - multi_questions（409）：多问题只读（探测未测面不出手）
            // - tool_readonly（409，T3）：该工具问答键序未实测（zcode/dsh 等）→ 只读卡
            // - bad_index（400）：select/toggle 序号越界 / submit 用在单选题
            let status = if code == "bad_index" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::CONFLICT
            };
            return (
                status,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": code })),
            )
                .into_response();
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
    // 守卫与投递同生命周期于 spawn_blocking 闭包内（fff9c29 同款）：忙 → None 哨兵
    // 让位，不投递亦不落审计（无投递发生，approve/retract/jump 忙让位同口径）
    let attempt = tokio::task::spawn_blocking(move || {
        let _guard = crate::inject::queue::try_acquire_inflight(&answer_sid)?;
        // 序列逐键投递；首错即停（半途失败不可盲目重试全序列——已发键已生效，
        // 前端按 failed{error} 提示用户核对终端状态后重试）
        let mut result = Ok(());
        for key in &sequence {
            if let Err(e) = injector.locate_and_send_key_spec(pid, key, &spec) {
                result = Err(e);
                break;
            }
        }
        Some(result)
    })
    .await;
    let sent = match attempt {
        Ok(Some(v)) => v,
        Ok(None) => {
            // in-flight 守卫忙（与 flush 循环/直发/审批共用）→ 让位，200 failed 提示
            // 重试（不双投；无投递发生故不写审计——忙让位同口径）
            return (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({
                    "status": "failed",
                    "error": "投递进行中，请稍后重试"
                })),
            )
                .into_response();
        }
        Err(e) => {
            log::error!("session-question/answer 投递任务异常: {e}");
            Err("内部任务异常".to_string())
        }
    };
    let audit_label = action.audit_label(req.index);
    match sent {
        Ok(()) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &audit_label,
                "answer",
                "ok",
            );
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "key_sent" })),
            )
                .into_response()
        }
        Err(e) => {
            endpoint_audit(
                &st,
                &device_id,
                &device_name,
                &tool,
                &sid,
                &audit_label,
                "answer",
                &format!("failed:{e}"),
            );
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "failed", "error": e })),
            )
                .into_response()
        }
    }
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
            let Some(session) = (probe_st.session_source)()
                .sessions
                .into_iter()
                .find(|s| s.id == probe_sid)
            else {
                return Err("no_session".to_string());
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
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response();
        }
    };
    let (tool, result) = match outcome {
        Ok(v) => v,
        Err(code) => {
            // 会话不在快照：无工具可审计（session-send 的 no_session 同口径不落账）
            return (
                StatusCode::NOT_FOUND,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": code })),
            )
                .into_response();
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
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "opening" })),
            )
                .into_response()
        }
        Err(e) if e == "no_resume_command" || e == "no_cwd" => (
            StatusCode::NOT_FOUND,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
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
            (
                StatusCode::OK,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "status": "failed", "error": e })),
            )
                .into_response()
        }
    }
}

/// 审计 content 的 resume 命令摘要（与核心同一命令表产物；spawner 出手成功时
/// 核心已构造过一次——此处再取一次纯表查询，代价可忽略，换取审计与出手同源）
fn resume_command_for_audit(tool: &str, sid: &str) -> String {
    crate::inject::resume::resume_command(tool, sid).unwrap_or_default()
}
