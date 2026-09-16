// gate 中间件：/m/api/* 全量过闸（403）——放行名单仅 /pair 与 /pair/*（换 cookie 的入口）
// cookie 解析手写（不加 cookie 依赖）：mam_device=<hex>
//
// 路径语义（重要，勿按直觉改）：本中间件由 `server.rs` 作为 **nest("/m/api/v1") 的内层 layer**
// 挂载，axum 的 nest 会**剥掉前缀**——这里 `req.uri().path()` 看到的是 `/pair`、`/sessions`、
// `/nope`，而**不是** `/m/api/v1/pair`。放行名单必须用相对路径，写成绝对路径会导致
// pair 被 403（无法换 cookie）或未知路径漏放，两种错法都不报编译错。
// （Task 7 追加静态路由不经过本中间件：那些路由注册在 nest 之外的外层 Router。）

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

/// 门禁：pair 放行（token 即凭据），其余请求必须带有效设备 cookie，否则 403。
/// 路径为 nest 剥前缀后的相对路径（见文件头注释），放行名单是 `"/pair"`
pub async fn gate(State(state): State<Arc<RemoteState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // M4 T2：审批配对三端点（/pair/request|poll|confirm）与 /pair 同为换 cookie 入口，放行
    if path == "/pair" || path.starts_with("/pair/") {
        return next.run(req).await; // 换 cookie 入口放行（token/审批码即凭据）
    }
    let Some(device) = extract_device(req.headers()) else {
        return unauthorized();
    };
    let now = chrono::Utc::now().timestamp_millis();
    // 有效判定与滑动续期合并进**同一个** with 闭包：`with` 持锁不可重入（嵌套必自锁），
    // 分开两次调用不仅多付一次取锁 + SQLite 往返，两段之间还存在 revoke 插入的 TOCTOU 窗口
    // （先判有效、后 touch 之间设备被吊销时，旧写法仍会 touch 已吊销设备）
    let ok = state.store.with(|c| {
        let valid = crate::remote::pairing::device_valid(c, &device, now);
        if valid {
            // 每次过闸刷新 last_seen（滑动 TTL）
            crate::remote::pairing::touch_device(c, &device, now);
        }
        valid
    });
    if ok {
        next.run(req).await
    } else {
        unauthorized()
    }
}

/// 统一拒绝响应：403 且响应体为空——不泄露"未配对 / 设备失效"的区分
fn unauthorized() -> Response {
    axum::http::StatusCode::FORBIDDEN.into_response()
}
