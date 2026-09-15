// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ host（P8a/P8b 页头数据）
// + pair + heartbeat

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

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
