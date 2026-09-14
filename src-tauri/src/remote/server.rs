// axum 组装：/m 静态（rust-embed，Task 7 填充资产）+ /m/api/v1/* + gate
//
// 结构契约（评审 Important 1 修复后，勿退化）：
// - API 一律走 `nest("/m/api/v1", api_router)`；gate 是**内层 layer**，覆盖该 nest 下
//   现在与将来注册的所有路由（含内层 fallback）——结构性生效，不依赖 `Router::layer`
//   的「只包裹此前注册的路由」这一顺序陷阱（评审判定的未来绕过：在已 layer 的 Router 上
//   后加 `.fallback()` 会替换掉被 gate 包裹的默认 fallback，未知 API 路径随之裸奔）；
// - 内层 fallback 直接 403：`/m/api/v1/*` 的未知路径不留裸路径，Task 7 追加的顶层
//   静态 fallback 追不进来；
// - 外层 Router 只负责静态侧：Task 7 在其上追加 `.route("/m", ...)`、`/m/assets/*`
//   与顶层 fallback，均**不应**经过 gate（配对页必须无 cookie 可加载）；
// - 内层 gate 看到的 path 已被 nest 剥掉前缀（`/pair` 而非 `/m/api/v1/pair`），
//   放行名单必须写相对路径，详见 gate.rs。

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

/// API 子路由：三条端点 + 内层 fallback（未知 API 路径直接 403）+ gate 内层 layer。
/// 注意：gate 在 `with_state` 之前 `layer`，故它包裹的是**本子路由已注册的全部端点与 fallback**；
/// 之后 Task 7 从外部追加的静态路由不在本子路由内，天然不过闸（结构隔离，非顺序巧合）。
fn api_router(state: Arc<RemoteState>) -> Router<Arc<RemoteState>> {
    Router::new()
        .route("/sessions", get(api::sessions))
        .route("/pair", post(api::pair))
        .route("/heartbeat", post(api::heartbeat))
        // 内层 fallback：nest 前缀下的未知/多余路径不得裸奔——没有它，
        // `/m/api/v1/nope` 会落到外层 fallback（Task 7 的静态兜底 → 200 静态内容），
        // 绕开"所有 /m/api/* 过 gate（403）"这条安全不变量（评审实测确认）
        .fallback(|| async { axum::http::StatusCode::FORBIDDEN })
        .layer(middleware::from_fn_with_state(
            state.clone(),
            super::gate::gate,
        ))
}

/// 组装路由：gate 需读 RemoteState（store 注入）——用 from_fn_with_state 而非 from_fn。
/// API 结构化嵌套（Important 1）：`nest` 使 gate 只作用于 `/m/api/v1/*` 且覆盖其全子树；
/// 未知 API 路径由内层 fallback 403 收口，外层（Task 7 静态资源）永不可见 API 路径。
pub fn router(state: Arc<RemoteState>) -> Router {
    Router::new()
        .nest("/m/api/v1", api_router(state.clone()))
        // 裸前缀带尾斜杠 `/m/api/v1/` 实测**不**匹配 nest 的 catch-all（matchit 的 `{*rest}`
        // 要求至少一个非空段，也不做尾斜杠归一化），会落到外层静态 fallback → 200。
        // 与"所有 /m/api/* 过 gate（403）"冲突，故显式 403 收口（any：方法无关一律 403）
        .route(
            "/m/api/v1/",
            axum::routing::any(|| async { axum::http::StatusCode::FORBIDDEN }),
        )
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
        // 五属性全断言（评审 Important 3）：device id 为 32 位 hex；属性完整串比对——
        // 此前只查 3 项，删掉 Max-Age 或改成 Max-Age=1 全套测试仍绿（180 天不变量失守）。
        // 期望值由 DEVICE_TTL_MS 推导，不写魔法数字；顺序与实现一致（属性顺序即响应语义）
        let device = cookie
            .split("mam_device=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(
            device.len() == 32 && device.chars().all(|c| c.is_ascii_hexdigit()),
            "device_id 应为 32 位 hex，实际 {device:?}"
        );
        assert_eq!(
            cookie,
            format!(
                "mam_device={device}; Path=/m; HttpOnly; SameSite=Lax; Max-Age={}",
                crate::remote::pairing::DEVICE_TTL_MS / 1000
            ),
            "cookie 必须同时具备 mam_device=hex / Path=/m / HttpOnly / SameSite=Lax / Max-Age=180d"
        );
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

    /// gate 拒绝不可区分性（评审 Important 4）："无 cookie"与"cookie 无效（未配对 id）"
    /// 必须给出**完全一致**的响应——状态码、响应体、以及无 Set-Cookie 等额外头。
    /// 若变异为"两种失败写入不同响应体/附带头"，即构成设备有效性预言机——本测试锁死该不变量
    #[tokio::test]
    async fn gate_rejections_are_indistinguishable() {
        let (state, _clock) = state_with_clock();
        let app = router(state);
        // (a) 无 cookie
        let no_cookie = snapshot(
            app.clone()
                .oneshot(req("GET", "/m/api/v1/sessions", None, None))
                .await
                .unwrap(),
        )
        .await;
        // (b) cookie 无效：形如设备 id 但从未配对
        let unknown_device = snapshot(
            app.clone()
                .oneshot(req(
                    "GET",
                    "/m/api/v1/sessions",
                    Some("mam_device=0123456789abcdef0123456789abcdef"),
                    None,
                ))
                .await
                .unwrap(),
        )
        .await;
        // (c) cookie 存在但值为空 —— 同属"无效凭据"
        let empty_value = snapshot(
            app.clone()
                .oneshot(req("GET", "/m/api/v1/sessions", Some("mam_device="), None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            no_cookie, unknown_device,
            "无 cookie 与无效 cookie 响应不可区分"
        );
        assert_eq!(
            unknown_device, empty_value,
            "空值 cookie 与无效 cookie 响应不可区分"
        );
        assert_eq!(
            no_cookie,
            (403, String::new(), false),
            "gate 拒绝一律 403 空体无 cookie，不得给有效性预言机"
        );
    }

    /// 结构契约锁定 + 绕过回归实测（评审 Important 1 核心验收）：
    /// 在 `router()` 返回值上**复刻 Task 7 的追加动作**——`.route("/m", ...)`（配对页）
    /// 与顶层 `.fallback(...)`（静态资源兜底）——然后确认：
    /// - 未知 API 路径 `/m/api/v1/nope` 无 cookie 仍 **403**（修复前：被后加 fallback 替换掉
    ///   被 gate 包裹的默认 fallback → 200 静态内容，门禁整体绕过）；
    /// - `/m`、`/m/assets/x.js`（模拟静态）无 cookie 可访问（配对页必须能加载）。
    #[tokio::test]
    async fn task7_static_routes_do_not_uncover_unknown_api_paths() {
        // 模拟 Task 7：静态路由 + 顶层静态 fallback 追加在 router() 之后
        let app = router(test_state())
            .route("/m", get(|| async { "pair-page" }))
            .fallback(|| async { "static-fallback" });

        // (1) 未知 API 路径：内层 fallback 403，绝不被顶层静态兜底接管
        for uri in [
            "/m/api/v1/nope",       // 未知子路径
            "/m/api/v1/",           // 裸前缀带尾斜杠（nest catch-all 不匹配，显式 403 收口）
            "/m/api/v1/sessions/x", // 已知端点下的多余段
        ] {
            let r = app
                .clone()
                .oneshot(req("GET", uri, None, None))
                .await
                .unwrap();
            assert_eq!(
                r.status(),
                403,
                "追加静态 fallback 后 {uri} 仍须过 gate 且不被静态兜底接管"
            );
        }

        // (2) 配对页本身不受 gate 约束（无 cookie 可加载）
        let r = app
            .clone()
            .oneshot(req("GET", "/m", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(body_string(r).await, "pair-page");

        // (3) 静态资产同理放行
        let r = app
            .clone()
            .oneshot(req("GET", "/m/assets/x.js", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(body_string(r).await, "static-fallback");

        // (4) 已知 API 路径（sessions）无 cookie 仍 403——gate 未被结构变更放宽
        let r = app
            .oneshot(req("GET", "/m/api/v1/sessions", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
    }

    /// 追加静态路由后 pair 仍可换 cookie（放行名单随 nest 剥前缀改为相对 `/pair`
    /// 的回归锁定——若名单仍写绝对路径，此测试 403 失败）
    #[tokio::test]
    async fn pair_still_reachable_after_task7_static_appended() {
        let app = router(test_state())
            .route("/m", get(|| async { "pair-page" }))
            .fallback(|| async { "static" });
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/pair",
                None,
                Some(r#"{"token":"tok-x"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "nest 内层 gate 的相对放行名单必须保住 pair"
        );
        assert!(r.headers().get("set-cookie").is_some());
    }

    /// 阻塞源不得卡住 async runtime（评审 Important 2）：注入一个**同步 sleep 300ms** 的
    /// session_source，另起一个轻量 async 任务测量其在扫描期间的调度延迟。
    /// 修复前（handler 里直调同步源）：扫描占满单线程 runtime → 轻量任务被推迟约 300ms；
    /// 修复后（spawn_blocking）：轻量任务应在数十 ms 内完成。
    /// 阈值取宽松的 150ms（CI 抖动容忍），仍能区分 300ms 级的阻塞
    #[tokio::test]
    async fn sessions_scan_does_not_stall_async_runtime() {
        // 重建 state 以注入阻塞源（其余注入缝与 test_state 一致：内存库、预发行 token）
        let state = Arc::new(RemoteState {
            pairing: std::sync::Mutex::new({
                let mut svc = PairingService::new(
                    600_000,
                    PairingClock {
                        now: Box::new(|| 1000),
                        token: Box::new(|| "tok-x".to_string()),
                    },
                );
                svc.issue();
                svc
            }),
            session_source: Box::new(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                crate::session::SessionsResponse {
                    sessions: vec![],
                    total_count: 42,
                    waiting_count: 0,
                }
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
        });
        // 预置有效设备，令 gate 放行（否则不会走到 session_source，测试失去意义）
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "rt".into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });
        let app = router(state);
        let start = std::time::Instant::now();
        let scan = tokio::spawn({
            let app = app.clone();
            async move {
                app.oneshot(req(
                    "GET",
                    "/m/api/v1/sessions",
                    Some("mam_device=rt"),
                    None,
                ))
                .await
                .unwrap()
            }
        });
        // 与扫描并发的最轻任务：若同步扫描占了 runtime，它要等扫描结束才能跑
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let light_elapsed = start.elapsed();
        let r = scan.await.unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"totalCount\":42"));
        assert!(
            light_elapsed < std::time::Duration::from_millis(150),
            "阻塞扫描期间轻量任务被推迟了 {light_elapsed:?}——session_source 未走 spawn_blocking"
        );
    }
}
