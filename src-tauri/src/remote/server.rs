// axum 组装：/m 静态（rust-embed，Task 7 填充资产）+ /m/api/v1/* + gate
//
// gate 层作用域说明（Task 7 相关，勿改）：`Router::layer` 只包裹"该 layer 之前已注册的路由"，
// 故 Task 7 之后追加的静态路由（/m、/m/assets/*）不经过 gate——这正是需求：
// 配对页必须在无 cookie 时也能加载。

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use super::api;

pub struct RemoteState {
    pub pairing: std::sync::Mutex<super::pairing::PairingService>,
    /// 会话数据源（P8 同源）：生产 = adapter::get_all_sessions；测试注入
    pub session_source: Box<dyn Fn() -> crate::session::SessionsResponse + Send + Sync>,
    /// 设备存储注入缝：生产 `DeviceStore::global()`；测试 `DeviceStore::memory()`（零接触真实 ~/.mam）
    pub store: super::pairing::DeviceStore,
}

/// 组装路由：gate 需读 RemoteState（store 注入）——用 from_fn_with_state 而非 from_fn
pub fn router(state: Arc<RemoteState>) -> Router {
    Router::new()
        .route("/m/api/v1/sessions", get(api::sessions))
        .route("/m/api/v1/pair", post(api::pair))
        .route("/m/api/v1/heartbeat", post(api::heartbeat))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            super::gate::gate,
        ))
        .with_state(state)
}

pub async fn serve(bind: &str, port: u16, state: Arc<RemoteState>) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind((bind, port))
        .await
        .map_err(|e| format!("绑定 {bind}:{port} 失败: {e}"))?;
    axum::serve(listener, router(state))
        .await
        .map_err(|e| format!("serve: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    // 简报测试原稿直接使用 `PairingService` / `PairingClock` 短名，但 `use super::*`
    // 只引入 server.rs 自身条目；此处补 import（编译硬阻断，非行为偏离）
    use crate::remote::pairing::{PairingClock, PairingService};

    fn test_state() -> Arc<RemoteState> {
        let mut pairing = PairingService::new(
            600_000,
            PairingClock {
                now: Box::new(|| 1000),
                token: Box::new(|| "tok-x".to_string()),
            },
        );
        // 偏离简报原稿一处（测试语义硬阻断）：原稿未调用 issue()，服务内无活跃 token，
        // 则 pair("tok-x") 必返 Invalid（Task 2 状态机：未知密文与无 token 同拒），
        // 步骤 3 的 200 断言不可能成立。此处先 issue() 使 "tok-x" 活跃——语义等价于
        // "用户已点击生成配对码"，正是被测流程的前置状态。
        pairing.issue();
        Arc::new(RemoteState {
            pairing: std::sync::Mutex::new(pairing),
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 7,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(), // 内存库——测试不碰真实 ~/.mam
        })
    }

    async fn body_string(b: axum::http::Response<Body>) -> String {
        String::from_utf8(b.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn gate_403_matrix_and_pair_flow() {
        let app = crate::remote::server::router(test_state());
        // 1) 无 cookie 访问 sessions → 403
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/m/api/v1/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        // 2) pair 错 token → 403
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"wrong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        // 3) pair 正确 token → 200 + Set-Cookie mam_device
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"tok-x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let cookie = r
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            cookie.contains("mam_device=")
                && cookie.contains("HttpOnly")
                && cookie.contains("SameSite=Lax")
        );
        let device = cookie
            .split("mam_device=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        // 4) 带 cookie 访问 sessions → 200，数据来自注入源（total_count=7）
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/m/api/v1/sessions")
                    .header("cookie", format!("mam_device={device}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"totalCount\":7"));
        // 5) 同 token 重放 → 403（一次性）
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"tok-x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
    }

    // ==== 追加测试（简报单个之外；理由：锁定简报未覆盖但已裁决的契约） ====

    /// 可推进时钟的状态（复用注入缝，测试仍零接触真实数据目录）
    fn state_with_clock() -> (Arc<RemoteState>, Arc<std::sync::atomic::AtomicI64>) {
        let t = Arc::new(std::sync::atomic::AtomicI64::new(1000));
        let now = t.clone();
        let mut svc = PairingService::new(
            600_000,
            PairingClock {
                now: Box::new(move || now.load(std::sync::atomic::Ordering::SeqCst)),
                token: Box::new(|| "tok-x".to_string()),
            },
        );
        svc.issue(); // 前置状态：已有活跃配对码
        (
            Arc::new(RemoteState {
                pairing: std::sync::Mutex::new(svc),
                session_source: Box::new(|| crate::session::SessionsResponse {
                    sessions: vec![],
                    total_count: 7,
                    waiting_count: 0,
                }),
                store: crate::remote::pairing::DeviceStore::memory(),
            }),
            t,
        )
    }

    /// 构造请求的收敛助手（cookie / body 可选）
    fn req(
        method: &str,
        uri: &str,
        cookie: Option<&str>,
        body: Option<&str>,
    ) -> axum::http::Request<Body> {
        let mut b = axum::http::Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        if let Some(c) = cookie {
            b = b.header("cookie", c);
        }
        b.body(match body {
            Some(s) => Body::from(s.to_string()),
            None => Body::empty(),
        })
        .unwrap()
    }

    /// 响应快照：(状态码, 响应体, 是否带 Set-Cookie)——用于比对拒绝三态是否可区分
    async fn snapshot(r: axum::http::Response<Body>) -> (u16, String, bool) {
        let status = r.status().as_u16();
        let has_cookie = r.headers().contains_key("set-cookie");
        (status, body_string(r).await, has_cookie)
    }

    /// 不变量③（简报未覆盖）：Invalid / Expired / Used 三态对客户端完全一致——
    /// 403 + 空响应体 + 无 Set-Cookie，不给 token 有效性预言机
    #[tokio::test]
    async fn pair_rejections_are_indistinguishable() {
        let (state, clock) = state_with_clock();
        let app = router(state.clone());
        let pair_req = |tok: &str| req("POST", "/m/api/v1/pair", None, Some(tok));
        let mut observed = vec![];

        // (a) Invalid：未知密文
        observed.push(
            snapshot(
                app.clone()
                    .oneshot(pair_req(r#"{"token":"nope"}"#))
                    .await
                    .unwrap(),
            )
            .await,
        );
        // (b) Expired：时钟推进越过 TTL（accept 在 used 判定前返回）
        clock.store(1000 + 600_001, std::sync::atomic::Ordering::SeqCst);
        observed.push(
            snapshot(
                app.clone()
                    .oneshot(pair_req(r#"{"token":"tok-x"}"#))
                    .await
                    .unwrap(),
            )
            .await,
        );
        // (c) Used：回到有效窗口重新发行，配对成功后重放
        clock.store(1000, std::sync::atomic::Ordering::SeqCst);
        state.pairing.lock().unwrap().issue();
        let r = app
            .clone()
            .oneshot(pair_req(r#"{"token":"tok-x"}"#))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "前置：重新发行后应配对成功");
        observed.push(
            snapshot(
                app.clone()
                    .oneshot(pair_req(r#"{"token":"tok-x"}"#))
                    .await
                    .unwrap(),
            )
            .await,
        );

        assert_eq!(observed[0], observed[1], "Invalid 与 Expired 响应不可区分");
        assert_eq!(observed[1], observed[2], "Expired 与 Used 响应不可区分");
        assert_eq!(
            observed[0],
            (403, String::new(), false),
            "拒绝一律 403 空体无 cookie"
        );
    }

    /// gate 的滑动 TTL：超窗设备拒绝、活跃设备过闸即刷新 last_seen
    #[tokio::test]
    async fn gate_refreshes_last_seen_and_rejects_stale_device() {
        let (state, _clock) = state_with_clock();
        let now = chrono::Utc::now().timestamp_millis();
        let dev = |id: &str, paired_at: i64| crate::remote::pairing::NewDevice {
            id: id.into(),
            name: String::new(),
            ua: String::new(),
            origin_ip: String::new(),
            paired_at,
        };
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &dev("stale", now - crate::remote::pairing::DEVICE_TTL_MS - 1),
            )
            .unwrap();
            crate::remote::pairing::persist_device(c, &dev("fresh", now - 5_000)).unwrap();
        });
        let app = router(state.clone());

        // 超窗（last_seen 早于 TTL 窗口）→ 403
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/sessions",
                Some("mam_device=stale"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);

        // 活跃 → 200，且 gate 把 last_seen_at 推到当前时刻（滑动续期）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/sessions",
                Some("mam_device=fresh"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let seen = state.store.with(|c| {
            c.query_row(
                "SELECT last_seen_at FROM remote_devices WHERE id = 'fresh'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
        });
        assert!(seen > now - 5_000, "gate 应刷新 last_seen_at，实际 {seen}");
    }

    /// heartbeat 同样受 gate 保护；带有效 cookie 回 pong
    #[tokio::test]
    async fn heartbeat_is_gated_and_returns_pong() {
        let (state, _clock) = state_with_clock();
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "hb".into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });
        let app = router(state);
        let r = app
            .clone()
            .oneshot(req("POST", "/m/api/v1/heartbeat", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/heartbeat",
                Some("mam_device=hb"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"ok\":true"));
    }

    /// Task 7 契约锁定（控制者实测定论）：`Router::layer` 只包裹"此前注册"的路由——
    /// 之后追加的静态路由（/m 配对页、/m/assets/*）无 cookie 也能加载，
    /// 故 Task 7 直接在 router() 返回值上 `.route("/m", ...)` 即可，勿改全局 fallback
    #[tokio::test]
    async fn routes_added_after_gate_layer_are_not_gated() {
        let app = router(test_state()).route("/m", get(|| async { "pair-page" }));
        let r = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/m")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "静态配对页不得被 gate 拦截");
        assert_eq!(body_string(r).await, "pair-page");
    }
}
