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
/// `SessionsResponse` 已 camelCase 序列化（前端约定 totalCount）
pub async fn sessions(State(st): State<Arc<RemoteState>>) -> impl IntoResponse {
    Json((st.session_source)())
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
    let mut svc = st.pairing.lock().unwrap();
    match svc.accept(&req.token) {
        AcceptResult::Ok { device_id } => {
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
            // HttpOnly + SameSite=Lax + 180d（dsh 七不变量之 cookie 语义）
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
        _ => Err(StatusCode::FORBIDDEN),
    }
}

/// POST /m/api/v1/heartbeat：gate 已刷新 last_seen（touch），此处仅回 pong 供客户端保活判定
pub async fn heartbeat() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true }))
}
