// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ host（P8a/P8b 页头数据）
// + pair/pin（M5 A3 访问密码配对，唯一换 cookie 入口）+ events（M3 Task 6 SSE 实时通道）
// + session-messages（Task 7）+ session-files / file（M3 Task 8 文件路径提取与安全读取）
// + session-send / send-info / queue 系（M7 Task 6 注入三端点）
// + session-approve-options / session-approve（M8 Task 11 审批选项与一键应答）

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
/// 端点与注入确认层（`inject::confirm::session_stamp_hit`）共用——**数据同源**：
/// 同走 `RemoteState.message_source` 缝（生产 = content::read_session_messages
/// 八工具统一出口；测试假源对端点矩阵与确认语义同时生效）。
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
//   POST /session-queue/jump    → delivered | failed{error} | 404 not_found
//   POST /session-queue/retract → {ok:true} | 404 not_found
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
/// - 可输入态（is_input_ready）→ 入队后即刻 flush_one 直发（jump=false），四态精确
///   映射（P1-4）：Sent → 200 delivered；Failed(e) → 200 failed{error}（注入失败
///   回执可重试，W1：失败行已 mark_failed 退出 pending，队列无残留）；
///   Deferred | Suspended → 200 queued（行保持 pending 等会话回来/下个跃迁，语义即
///   排队——Suspended 亦 queued，红·中断挂起不谎报 delivered 也不误报失败）。直发
///   审计按态落 send|queue（flush_one 落账时已并行写 flush 审计——本端点按契约另写
///   send 终态）；
/// - 运行中（is_running 等）→ 留队（黄灯），审计 action=queue，回执 queued+position。
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
    //    防同会话并发双投）。守卫不对称说明（queue.rs Critical 1 同审）：本守卫宿主是
    //    handler 任务——stop 的 abort 只打 serve/accept 任务、不追杀已建立请求，不存在
    //    「守卫先于 detached 投递释放」窗口，无需移入阻塞闭包（flush 循环事件臂必须移）
    if crate::inject::queue::is_input_ready(&session.status) {
        return match crate::inject::queue::try_acquire_inflight(&sid) {
            None => {
                // flush 循环正在投递该会话：本条保持入队（守卫方负责队首送达，
                // 下一跃迁接力本条），按已入队回执——不谎报 delivered
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
            Some(_guard) => {
                let flush_st = st.clone();
                let flush_sid = sid.clone();
                let outcome = tokio::task::spawn_blocking(move || {
                    crate::inject::queue::flush_one(&flush_st, &flush_sid, false)
                })
                .await
                .unwrap_or_else(|e| {
                    log::error!("session-send 直发任务异常: {e}");
                    crate::inject::queue::FlushOutcome::Failed("内部任务异常".to_string())
                });
                // P1-4 四态精确映射（Sent/Failed/Deferred/Suspended →
                // delivered/failed/queued/queued）
                match outcome {
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
                    // Deferred/Suspended：行保持 pending 等会话回来/下个跃迁，语义即
                    // 排队（Suspended 亦 queued）——回查 pending 取该条目实时位次回执
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
                }
            }
        };
    }
    // ⑦ 运行中（黄灯）→ 留队等下一可输入态
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
/// - 四态精确映射（P1-4）：Sent → 200 delivered；Failed(e) → 200 failed{error}
///   （注入失败行已退出 pending）；Deferred | Suspended → 200 queued + itemId/position
///   （行保持 pending 等会话回来/下个跃迁，语义即排队——jump 点名场景 Deferred 实际
///   不可达〔jump 跳过黄态复核〕，Suspended〔会话消失〕亦按 queued 回执）；
/// - in-flight 守卫忙 → 200 failed（该会话投递进行中，提示重试）。
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
    // 先取 in-flight 守卫再做归属查找：顺序颠倒会有窗口——flush 循环在间隙内发出同一
    // 队首项，jump 再点名投递同一条 → 双投。守卫先占住即与循环/直发互斥。
    let Some(_guard) = crate::inject::queue::try_acquire_inflight(&sid) else {
        // 该会话已有投递进行中（与直发/flush 循环共用守卫）——插队让位，提示重试
        return failed_body("该会话投递进行中，请稍后重试".to_string());
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
    let outcome = match tokio::task::spawn_blocking(move || {
        crate::inject::queue::flush_given(&flush_st, &item, true)
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
        crate::inject::queue::FlushOutcome::Failed(e) => failed_body(e),
        // Deferred/Suspended：行保持 pending 等会话回来/下个跃迁，语义即排队——
        // 回查 pending 取该条目实时位次回执
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

// ==== M8 Task 11：审批端点（session-approve-options / session-approve）====
// 契约（JSON camelCase；Json 响应带 no-store——门禁下私有写/读路径）：
//   GET  /session-approve-options?session_id= → 200 {available, options:[{id,label}],
//        verifiedWith, currentVersion(…|null), drift}
//        available = status==Waiting && 映射表有该工具映射 && detect 命中
//        （数据同源快照判定）；工具无映射 / 未命中 / 非 Waiting → available=false
//        （options 空）。options 只含 id+label——**键位不外泄给 UI**。
//   POST /session-approve body {sessionId, optionId} → 200 {"status":"key_sent"}
//        | 404 no_session | 409 not_waiting | 404 no_mapping（降级提示走普通发送）
//        | 200 failed{error}（注入失败 / in-flight 忙，可重试回执）。
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
    if session.status != crate::session::SessionStatus::Waiting {
        return None;
    }
    let tool = session.agent_type.tool_id().to_string();
    let mapping = st
        .store
        .with(crate::inject::approve::load_mappings_conn)
        .into_iter()
        .find(|m| m.tool == tool)?;
    // 严格档（M9R Task 10 裁决：未取证不出键）：probe-pending 映射即使 Waiting+detect
    // 命中也压为不可批——选项不下发，只给降级原因（前端提示条）；drift 判定照常
    // （probe-pending 恒判漂移，提示条与 drift 提示并存不冲突）
    if mapping.verified_with == crate::inject::approve::PROBE_PENDING {
        return Some(ApproveScanHit {
            available: false,
            options: Vec::new(),
            verified_with: mapping.verified_with,
            tool,
            reason: Some(crate::inject::approve::PROBE_PENDING_REASON.to_string()),
        });
    }
    let hit = session
        .last_message
        .as_deref()
        .map(|msg| crate::inject::approve::detect(&mapping, msg))
        .unwrap_or(false);
    // 选项序列化只取 id+label（key 是投递层机密，不进任何 UI 载荷）；未命中 → options 空
    // （契约：available=false 一律不给选项，移动端据此不渲染审批卡）
    let options = if hit {
        mapping
            .options
            .iter()
            .map(|o| (o.id.clone(), o.label.clone()))
            .collect()
    } else {
        Vec::new()
    };
    Some(ApproveScanHit {
        available: hit,
        options,
        verified_with: mapping.verified_with,
        tool,
        reason: None,
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
    let (available, options, verified_with, tool, reason) = match scan {
        Some(hit) => (
            hit.available,
            hit.options,
            hit.verified_with,
            Some(hit.tool),
            hit.reason,
        ),
        // 不可批三态（无会话 / 非 Waiting / 无映射）同形：available=false，版本字段空，
        // 无 reason（前端按卡自隐处理，与严格档 reason 提示条区分）
        None => (false, Vec::new(), String::new(), None, None),
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
/// - 映射表无该工具映射 / optionId 无对应项 → 404 no_mapping（降级提示走
///   普通发送）；
/// - in-flight 守卫忙（flush 循环/直发/插队正在投递该会话）→ 200 failed 提示重试；
/// - 投递 `injector.locate_and_send_key(pid, key)`：**不带 [mobile] 前缀**——按键非文本；
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
        if session.status != crate::session::SessionStatus::Waiting {
            return Err("not_waiting");
        }
        let tool = session.agent_type.tool_id().to_string();
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
    // in-flight 守卫（与 flush 循环/直发/插队共用）：该会话已有投递进行中 → 让位，
    // 200 failed 提示重试（不双投；无投递发生故不写审计——jump 忙让位同口径）
    let Some(_guard) = crate::inject::queue::try_acquire_inflight(&sid) else {
        return (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({
                "status": "failed",
                "error": "该会话投递进行中，请稍后重试"
            })),
        )
            .into_response();
    };
    // M11：会话路由含无头通道时，此处按 approve option 分派 control_message 而非按键
    let injector = st.injector.clone();
    let pid = session.pid;
    let key = option.key.clone();
    let sent =
        match tokio::task::spawn_blocking(move || injector.locate_and_send_key(pid, &key)).await {
            Ok(v) => v,
            Err(e) => {
                log::error!("session-approve 投递任务异常: {e}");
                Err("内部任务异常".to_string())
            }
        };
    // 审计 action 词表（W5 词表 send|queue|flush|jump|retract|approve|reject|fail|key；
    // open 预留：Task 11 resume 审计动作）：approve/reject 语义化；域外 id（KV 定制表
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
