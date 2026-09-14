// gate 中间件：/m/api/* 全量过闸（403）——放行名单仅 pair（换 cookie 的入口）
// cookie 解析手写（不加 cookie 依赖）：mam_device=<hex>

use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

use super::server::RemoteState;

/// 设备 cookie 名（唯一来源：此处解析与 api::pair 下发都引用它，避免两处字面量漂移）
pub const COOKIE_NAME: &str = "mam_device";

/// 从 Cookie 头解析设备 id；无 / 空值 → None
pub fn extract_device(headers: &axum::http::HeaderMap) -> Option<String> {
    let prefix = format!("{COOKIE_NAME}=");
    let raw = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .map(str::trim)
        .find_map(|s| s.strip_prefix(prefix.as_str()))?;
    let v = raw.trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// 门禁：pair 放行（token 即凭据），其余请求必须带有效设备 cookie，否则 403
pub async fn gate(State(state): State<Arc<RemoteState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/m/api/v1/pair" {
        return next.run(req).await; // 换 cookie 入口放行（token 即凭据）
    }
    let Some(device) = extract_device(req.headers()) else {
        return unauthorized();
    };
    let now = chrono::Utc::now().timestamp_millis();
    let ok = state
        .store
        .with(|c| crate::remote::pairing::device_valid(c, &device, now));
    if ok {
        // 每次过闸刷新 last_seen（滑动 TTL）
        state
            .store
            .with(|c| crate::remote::pairing::touch_device(c, &device, now));
        next.run(req).await
    } else {
        unauthorized()
    }
}

/// 统一拒绝响应：403 且响应体为空——不泄露"未配对 / 设备失效"的区分
fn unauthorized() -> Response {
    axum::http::StatusCode::FORBIDDEN.into_response()
}
