// /m/api/v1/*：sessions（P8 数据同源直调 get_all_sessions）+ pair + heartbeat

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
pub async fn sessions(
    State(st): State<Arc<RemoteState>>,
) -> Json<crate::session::SessionsResponse> {
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
    Json(response)
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
pub async fn heartbeat() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}
