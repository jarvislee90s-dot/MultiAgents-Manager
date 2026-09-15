// axum 组装：/m 静态（rust-embed，Task 7 已装配）+ /m/api/v1/* + gate
//
// 结构契约（评审 Important 1 修复后，勿退化）：
// - API 一律走 `nest("/m/api/v1", api_router)`；gate 是**内层 layer**，覆盖该 nest 下
//   现在与将来注册的所有路由（含内层 fallback）——结构性生效，不依赖 `Router::layer`
//   的「只包裹此前注册的路由」这一顺序陷阱（评审判定的未来绕过：在已 layer 的 Router 上
//   后加 `.fallback()` 会替换掉被 gate 包裹的默认 fallback，未知 API 路径随之裸奔）；
// - 内层 fallback 直接 403：`/m/api/v1/*` 的未知路径不留裸路径，Task 7 追加的顶层
//   静态 fallback 追不进来；
// - 外层 Router 只负责静态侧：Task 7 在其上追加 `.route("/m", ...)` 与顶层
//   fallback（/m/* 静态资产由该 fallback 的路径分流伺服，没有独立的 /m/assets/*
//   路由），均**不应**经过 gate（配对页必须无 cookie 可加载）；
// - 内层 gate 看到的 path 已被 nest 剥掉前缀（`/pair` 而非 `/m/api/v1/pair`），
//   放行名单必须写相对路径，详见 gate.rs。

use axum::{
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use super::api;

// ============================================================
// /m 静态伺服（Task 7）：rust-embed 嵌入 dist-mobile 构建产物
// ============================================================

/// rust-embed 嵌入 `../dist-mobile/`（相对 src-tauri/，即仓库根的移动端产物目录）。
/// 构建顺序铁律：release 下产物在**编译期**打进二进制，debug 下 `get()` 每次**从磁盘直读**
/// （crate 默认行为，便于 tauri:dev 迭代移动端产物而免重编 Rust）——
/// 因此任何 `cargo check/test/build` 之前必须先 `pnpm build:mobile`，否则入口 404。
#[derive(rust_embed::RustEmbed)]
#[folder = "../dist-mobile/"]
// dist-mobile/.gitkeep 是入库占位文件（构建时由 publicDir 拷贝自动恢复，非伺服资产）：
// exclude 防止它被 rust-embed 嵌进二进制 / 被静态伺服（include-exclude feature 即为此开启）
#[exclude = ".gitkeep"]
struct MobileAssets;

/// SPA 入口文件。控制者裁决（2026-09-14）：`vite build` 对 `mobile.html` 入口产出的就是
/// `dist-mobile/mobile.html`（`rollupOptions.input` 键名改不了 HTML 输出名，Task 5 实测）——
/// 简报原稿的 `serve_asset("index.html")` 会 404，故 /m 与各处回落统一伺服 mobile.html
const MOBILE_ENTRY: &str = "mobile.html";

/// 入口 HTML 响应（/m 精确命中与 /m/* 未命中回落共用）。
/// 抽成非 async 纯函数的原因：若 serve_asset 未命中分支直接 `mobile_index().await`，
/// 会构成相互递归的 async fn（编译不过；且 dist-mobile 未构建时无限循环）。
/// 产物缺失（未跑 pnpm build:mobile）→ 404 显式失败，不挂死
fn entry_response() -> Response {
    match MobileAssets::get(MOBILE_ENTRY) {
        Some(f) => (
            [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
            f.data,
        )
            .into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            "mobile assets missing (run `pnpm build:mobile` first)",
        )
            .into_response(),
    }
}

/// 按扩展名给 MIME：简报清单（js/css/png/json/html）+ 补充（svg=manifest/图标可能引用；
/// webmanifest=若产物出现 PWA manifest 的 .webmanifest 形态）。
/// 未知扩展回落 `application/octet-stream`（选型：二进制下载语义，而非简报默认的
/// text/html——把任意未知内容误标成 HTML 会放大注入面）
fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("webmanifest") => "application/manifest+json",
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

async fn serve_asset(path: &str) -> Response {
    match MobileAssets::get(path) {
        Some(f) => ([(axum::http::header::CONTENT_TYPE, mime_for(path))], f.data).into_response(),
        // SPA 兜底（控制者裁决 1）：/m/<path> 未命中回落入口 HTML——移动端单页 hash 路由，
        // 刷新/直达任意路径都必须能拿到壳页面
        None => entry_response(),
    }
}

/// 顶层静态 fallback 的路径分流（控制者裁决 2，勿退化）：
/// - 非 `/m` 前缀 → 404（根路径与移动看板无关，不给静态兜底）；
/// - `/m/` 尾斜杠与 `/m`（理论上被 route 收口，防御性兜底）→ 入口 HTML；
/// - `/m/api` 裸前缀与 `/m/api/*`（含未知版本前缀如 `/m/api/v2/*`）→ 403：这是
///   「所有 `/m/api/*` 过 gate（403）」安全不变量的**字面收口**（终审 2026-09-14）——
///   这些路径不匹配 nest 的 catch-all（matchit `{*rest}` 要求至少一个非空段，且裸
///   前缀 `/m/api` 连尾斜杠都没有），否则会落到下方 `serve_asset` 的 SPA 回落返回
///   200 入口 HTML，字面违反不变量。`/m/api/v1/` 精确变体另有 router() 上的显式
///   收口条（结构层保证，见其注释）；
/// - `/m/<path>` → `serve_asset(path)`，未命中回落入口 HTML。
///
/// 注意：`/m/api/v1/<已知或未知子路径>` 永远到不了这里——nest 内层 fallback 先 403
/// （结构隔离，见 router()/api_router 注释与 task7_static_routes_* 测试）
async fn static_fallback(uri: axum::http::Uri) -> Response {
    let path = uri.path();
    if path == "/m" || path == "/m/" {
        return entry_response();
    }
    // 「所有 /m/api/* 过 gate」的字面收口：裸前缀 / 尾斜杠 / 未知版本前缀一律 403
    // （须在 serve_asset 的 SPA 回落之前判定，否则 200 静态内容顶替 gate）
    if path == "/m/api" || path.starts_with("/m/api/") {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    match path.strip_prefix("/m/") {
        Some(rest) => serve_asset(rest).await,
        None => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// /m 入口（配对页/看板壳）。产物文件是 mobile.html（见 MOBILE_ENTRY 注释）
async fn mobile_index() -> Response {
    entry_response()
}

pub struct RemoteState {
    pub pairing: std::sync::Mutex<super::pairing::PairingService>,
    /// 会话数据源（P8 同源）：生产 = adapter::get_all_sessions；测试注入
    pub session_source: Box<dyn Fn() -> crate::session::SessionsResponse + Send + Sync>,
    /// 设备存储注入缝：生产 `DeviceStore::global()`；测试 `DeviceStore::memory()`（零接触真实 ~/.mam）
    pub store: super::pairing::DeviceStore,
    /// host 载荷注入缝（M3 Task 1）：生产 = remote::host_info()；测试注入假 json（零 DB）
    pub host_source: Box<dyn Fn() -> serde_json::Value + Send + Sync>,
    /// 会话内容源注入缝（M3 Task 7）：生产 = content::read_session_messages（八工具
    /// 统一出口）；测试注入假源（零接触真实 ~/.zcode ~/.dsh 等数据目录）
    pub message_source: Box<super::content::MessageSourceFn>,
    /// 跃迁事件通道（M3 Task 5）：生产 = watcher::event_sender()（全进程同一通道，
    /// 与 SessionWatcher::start 的循环共享）；测试注入新建空通道即可。
    /// **订阅端消费即去重完成**（铁律 4）：事件只含边沿（见 watcher::diff_transitions）
    pub watcher_tx: tokio::sync::broadcast::Sender<super::watcher::TransitionEvent>,
}

/// API 子路由：三条端点 + 内层 fallback（未知 API 路径直接 403）+ gate 内层 layer。
/// 注意：gate 在 `with_state` 之前 `layer`，故它包裹的是**本子路由已注册的全部端点与 fallback**；
/// 之后 Task 7 从外部追加的静态路由不在本子路由内，天然不过闸（结构隔离，非顺序巧合）。
fn api_router(state: Arc<RemoteState>) -> Router<Arc<RemoteState>> {
    Router::new()
        .route("/sessions", get(api::sessions))
        .route("/host", get(api::host))
        // /events（M3 Task 6）：SSE 长连接，gate 由本子路由的 layer 结构性覆盖
        // （与其余端点同一内层 gate，不需要额外 middleware）
        .route("/events", get(api::events))
        // /session-messages（M3 Task 7）：单会话内容读取（C2 后端，八工具统一出口）
        .route("/session-messages", get(api::session_messages))
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
        // 与"所有 /m/api/* 过 gate（403）"冲突，故显式 403 收口（any：方法无关一律 403）。
        // 终审修复轮保留了此条（未并入 static_fallback）：它挂在 router() 上，对**任意**
        // 外层 fallback 装配（含测试/未来变体的自定义 fallback）结构性生效，不依赖
        // static_fallback 分流的实现自觉；其余 /m/api 变体（裸前缀 / 尾斜杠 / 未知版本）
        // 由 static_fallback 的字面收口兜住
        .route(
            "/m/api/v1/",
            axum::routing::any(|| async { axum::http::StatusCode::FORBIDDEN }),
        )
        .with_state(state)
}

/// 生产装配：`router()`（gate 结构不变量）+ /m 静态入口 + 顶层静态 fallback。
/// 结构契约（评审裁决 2，勿退化）：静态侧**只**以「在 `router()` 返回的 Router 上追加
/// `.route("/m", ...)` 与顶层 `.fallback(...)`」的形态存在——**绝对不要**改成 catch-all
/// 路由（`/m/{*path}`）或在外层注册 `/m/api/v1/...`：catch-all 会先于 nest 内层 403
/// fallback 命中，让未知 API 路径 200 裸奔
/// （task7_static_routes_do_not_uncover_unknown_api_paths 复刻的正是本函数的追加动作）
pub fn router_with_static(state: Arc<RemoteState>) -> Router {
    router(state)
        .route("/m", get(mobile_index))
        .fallback(static_fallback)
}

pub async fn serve(bind: &str, port: u16, state: Arc<RemoteState>) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind((bind, port))
        .await
        .map_err(|e| format!("绑定 {bind}:{port} 失败: {e}"))?;
    // M3 Task 5：事件桥在**绑定成功后**启动（幂等——全局 Once 保证扫描循环全进程只
    // spawn 一次）。放在 bind 之后：端口被占等启动失败不遗留 2s 会话扫描（扫描是重活，
    // 见 adapter::get_all_sessions 的单飞护栏注释）；watcher 生命周期随进程结束，
    // stop_server 不显式停止（关闭远程后循环仍扫描，为已有取舍）
    super::watcher::SessionWatcher::start();
    axum::serve(listener, router_with_static(state))
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
            host_source: Box::new(|| {
                serde_json::json!({
                    "host": { "name": "test-host", "platform": "macos", "version": "0.0.0-test" },
                    "enabledTools": ["claude"]
                })
            }),
            // M3 Task 7：本组测试不触 /session-messages，注入恒 Err 的桩
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            // M3 Task 5：测试用空事件通道（不启动 watcher——零后台扫描）
            watcher_tx: tokio::sync::broadcast::channel(64).0,
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
        // M2-R2 顺手项：会话数据不得被中间层缓存（设备门禁下的私有数据）
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "sessions 响应必须带 Cache-Control: no-store"
        );
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
                host_source: Box::new(|| serde_json::Value::Null), // 本组测试不触 /host
                message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())), // 本组测试不触 /session-messages
                watcher_tx: tokio::sync::broadcast::channel(64).0, // M3 Task 5：空事件通道
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

    // ==== Task 7 静态伺服（router_with_static 真装配；rust-embed debug 态从磁盘直读
    // dist-mobile，零接触真实 ~/.mam；产物 hash 文件名动态取，不硬编码） ====

    /// 嵌入清单里 assets/ 下第一个 .js 产物（vite hash 文件名随构建漂移，禁止硬编码）
    fn first_js_asset() -> String {
        MobileAssets::iter()
            .find(|p| p.starts_with("assets/") && p.ends_with(".js"))
            .expect("dist-mobile 缺少 js 产物：先跑 pnpm build:mobile 再 cargo test")
            .to_string()
    }

    fn header<'a>(r: &'a axum::http::Response<Body>, name: &str) -> &'a str {
        r.headers()
            .get(name)
            .expect("响应缺少头")
            .to_str()
            .expect("头值非可见 ASCII")
    }

    /// 静态伺服全矩阵（含安全不变量回归）：入口 / manifest / 真实产物 MIME /
    /// SPA 回落 / 非 /m 前缀 404 / 未知 API 路径仍 403 / 尾斜杠变体
    #[tokio::test]
    async fn static_serving_matrix() {
        let app = router_with_static(test_state());

        // (1) /m 精确 → 200 入口 HTML（mobile.html 产物：doctype + 移动端标题，防串台桌面 index）
        let r = app
            .clone()
            .oneshot(req("GET", "/m", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(header(&r, "content-type"), "text/html; charset=utf-8");
        let body = body_string(r).await.to_lowercase();
        assert!(
            body.contains("<!doctype html>"),
            "/m 应返回入口 HTML，实际 {body:?}"
        );
        assert!(
            body.contains("mam 远程"),
            "入口应是 mobile.html 产物（含移动端标题）"
        );

        // (2) PWA manifest → 200 + application/json
        let r = app
            .clone()
            .oneshot(req("GET", "/m/manifest-mam.json", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(header(&r, "content-type"), "application/json");

        // (3) 真实 js 产物 → 200 + text/javascript
        let asset = first_js_asset();
        let r = app
            .clone()
            .oneshot(req("GET", &format!("/m/{asset}"), None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "嵌入产物 {asset} 应可伺服");
        assert_eq!(header(&r, "content-type"), "text/javascript");

        // (4) 未知 /m/* 路径 → SPA 兜底回落入口 HTML（200，非 404）
        let r = app
            .clone()
            .oneshot(req("GET", "/m/nope.js", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r)
            .await
            .to_lowercase()
            .contains("<!doctype html>"));

        // (5) 非 /m 前缀 → 404（根路径不给静态兜底）
        let r = app
            .clone()
            .oneshot(req("GET", "/favicon.ico", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);

        // (6) 未知 API 路径 → 仍 403：真装配下 nest 内层 fallback 不被外层静态兜底顶掉
        let r = app
            .clone()
            .oneshot(req("GET", "/m/api/v1/nope", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "静态装配不得让未知 API 路径裸奔");

        // (7) 尾斜杠变体 /m/ → 200 入口（route("/m") 不匹配，由 fallback 分流收口）
        let r = app
            .clone()
            .oneshot(req("GET", "/m/", None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r)
            .await
            .to_lowercase()
            .contains("<!doctype html>"));

        // (8) /m/api 裸前缀三变体 → 403（终审修复轮：「所有 /m/api/* 过 gate」的字面
        // 收口）。修复前 /m/api 与 /m/api/ 不匹配任何 route，落到 SPA 回返 200 入口
        // HTML，字面违反安全不变量；/m/api/v2/* 锁定未来版本前缀同样收口
        for uri in ["/m/api", "/m/api/", "/m/api/v2/anything"] {
            let r = app
                .clone()
                .oneshot(req("GET", uri, None, None))
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "{uri} 必须被 /m/api 字面收口为 403");
            assert!(
                !body_string(r)
                    .await
                    .to_lowercase()
                    .contains("<!doctype html>"),
                "{uri} 不得回落静态入口 HTML"
            );
        }
    }

    /// PNG 产物（icon-mobile.png）MIME 与 200：manifest icons 引用的唯一非 js/css 资产
    #[tokio::test]
    async fn png_asset_served_with_image_mime() {
        let app = router_with_static(test_state());
        let png = MobileAssets::iter()
            .find(|p| p.ends_with(".png"))
            .expect("dist-mobile 缺少 png 产物：先跑 pnpm build:mobile");
        let r = app
            .oneshot(req("GET", &format!("/m/{png}"), None, None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(header(&r, "content-type"), "image/png");
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
            host_source: Box::new(|| serde_json::Value::Null), // 本测试不触 /host
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            watcher_tx: tokio::sync::broadcast::channel(64).0, // M3 Task 5：空事件通道
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

    // ==== M3 Task 6：GET /m/api/v1/events（SSE 实时通道，C1 后半） ====
    // 零污染：会话源经注入缝（假 SessionsResponse）、事件源经注入的空 broadcast 通道。
    // SSE 是长连接流式响应：**不能用 body_string（collect 会等流结束、永久挂起）**，
    // 只逐帧读（BodyExt::frame），够断言首帧快照与增量帧即可，读完即 drop（连接关闭）。

    /// 读 SSE 流的下一帧文本（不等待流结束；帧缺失/非数据帧即断言失败）
    async fn next_frame(body: &mut Body) -> String {
        let frame = body
            .frame()
            .await
            .expect("SSE 流在断言帧之前结束")
            .expect("SSE 帧读取失败");
        let bytes = frame
            .into_data()
            .unwrap_or_else(|_| panic!("SSE 应产生数据帧（非 trailers）"));
        String::from_utf8(bytes.to_vec()).expect("SSE 帧应为 UTF-8")
    }

    /// 预置有效设备（SSE 测试专用：复用 state_with_clock 的注入缝，零接触真实设备表）
    fn persist_device(state: &Arc<RemoteState>, id: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: id.into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });
    }

    /// gate 覆盖（控制者①）：/events 与其余 /m/api/v1/* 同一门禁——无 cookie 必须 403，
    /// 不得因为「SSE 是长连接」而漏过内层 layer
    #[tokio::test]
    async fn sse_events_is_gated() {
        let (state, _clock) = state_with_clock();
        let app = router(state);
        let r = app
            .oneshot(req("GET", "/m/api/v1/events", None, None))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            403,
            "/events 必须过 gate（设备失效与未配对同 403 语义）"
        );
    }

    /// SSE 端点主链（控制者③）：认证后 200 + text/event-stream + 首帧 `event: snapshot`
    /// 携带全量会话（注入源 totalCount=7 ⇒ 数据同源），随后 watcher_tx 上的事件以
    /// `event: transition` + camelCase JSON 送达（wire 契约端到端锁定）
    #[tokio::test]
    async fn sse_events_streams_snapshot_then_transitions() {
        let (state, _clock) = state_with_clock();
        persist_device(&state, "ev");
        let app = router(state.clone());
        let r = app
            .oneshot(req("GET", "/m/api/v1/events", Some("mam_device=ev"), None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            header(&r, "content-type"),
            "text/event-stream",
            "SSE 响应必须声明 text/event-stream（浏览器据此走 EventSource 解析）"
        );
        assert_eq!(
            header(&r, "cache-control"),
            "no-cache",
            "SSE 是设备门禁下的私有实时流，禁止中间层缓存"
        );

        let mut body = r.into_body();
        // 首帧：全量快照（event: snapshot + SessionsResponse JSON，数据来自注入源）
        let first = next_frame(&mut body).await;
        assert!(
            first.starts_with("event: snapshot\n"),
            "首帧必须是 snapshot 事件，实际 {first:?}"
        );
        assert!(
            first.contains("\"totalCount\":7"),
            "首帧快照必须直调 session_source（注入源 totalCount=7），实际 {first:?}"
        );

        // 增量帧：watcher_tx 发一条跃迁 → transition 帧 + camelCase 键（前端按
        // ev.sessionId / ev.agentType / ev.projectName 读取；snake_case 会静默 undefined）
        state
            .watcher_tx
            .send(crate::remote::watcher::TransitionEvent {
                session_id: "s1".into(),
                agent_type: "claude".into(),
                from: "idle".into(),
                to: "processing".into(),
                project_name: "proj".into(),
                last_message: Some("hello".into()),
                ts: 42,
            })
            .unwrap();
        let second = next_frame(&mut body).await;
        assert!(
            second.starts_with("event: transition\n"),
            "增量帧必须是 transition 事件，实际 {second:?}"
        );
        for key in ["\"sessionId\":\"s1\"", "\"to\":\"processing\"", "\"ts\":42"] {
            assert!(
                second.contains(key),
                "transition 帧缺少 {key}（camelCase wire 契约），实际 {second:?}"
            );
        }
        assert!(
            !second.contains("session_id"),
            "transition 帧不得出现 snake_case 键，实际 {second:?}"
        );
    }

    // ==== M3 Task 1：GET /m/api/v1/host（P8a/P8b 页头数据源） ====
    // 零污染：host 载荷经 host_source 注入缝供给（假 json），不触 settings DAO / 全局 DB。

    /// /host 端点矩阵：无 cookie 403（与 sessions 同一 gate，设备失效语义一致）；
    /// 有效设备 200 返回注入载荷 + Cache-Control: no-store（host 同属门禁下私有数据）
    #[tokio::test]
    async fn host_endpoint_is_gated_and_returns_injected_payload() {
        // 重建 state：host_source 注入假载荷（与 sessions_scan_* 重建 state 的先例一致）
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
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 0,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            host_source: Box::new(|| {
                serde_json::json!({
                    "host": { "name": "jarvis-win", "platform": "windows", "version": "9.9.9-test" },
                    "enabledTools": ["claude", "zcode"]
                })
            }),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            watcher_tx: tokio::sync::broadcast::channel(64).0, // M3 Task 5：空事件通道
        });
        let app = router(state.clone());
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "hd".into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });

        // (1) 无 cookie → 403（gate 全量覆盖新端点，与 sessions 语义一致）
        let r = app
            .clone()
            .oneshot(req("GET", "/m/api/v1/host", None, None))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            403,
            "/host 必须过 gate（设备失效同 sessions 的 403 语义）"
        );

        // (2) 有效设备 → 200 + 注入载荷原样透传
        let r = app
            .clone()
            .oneshot(req("GET", "/m/api/v1/host", Some("mam_device=hd"), None))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"name\":\"jarvis-win\"") && body.contains("\"version\":\"9.9.9-test\""),
            "host 载荷应原样透传，实际 {body}"
        );
        assert!(
            body.contains("\"enabledTools\""),
            "enabledTools 数据源随载荷返回"
        );
    }

    /// /session-messages（M3 Task 7）端点矩阵：gate 403 → 缺参 400 → 注入源 200
    /// （载荷 camelCase 且 no-store）→ 读取失败 404（错误细节不外泄）。
    /// 零污染：message_source 注入假源，不触任何真实工具数据目录
    #[tokio::test]
    async fn session_messages_endpoint_is_gated_and_shaped() {
        let captured: Arc<std::sync::Mutex<Vec<(String, String, usize)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
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
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 0,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(move |agent: &str, sid: &str, limit: usize| {
                cap.lock()
                    .unwrap()
                    .push((agent.to_string(), sid.to_string(), limit));
                if sid == "sess_hit" {
                    Ok(vec![
                        crate::remote::content::SessionMessage {
                            seq: 0,
                            role: "user".into(),
                            kind: "user".into(),
                            content: "你好".into(),
                            ts: Some(1000),
                            tool_name: None,
                            tool_args: None,
                            collapsed: false,
                        },
                        crate::remote::content::SessionMessage {
                            seq: 1,
                            role: "assistant".into(),
                            kind: "tool-call".into(),
                            content: "调用 Bash".into(),
                            ts: Some(1001),
                            tool_name: Some("Bash".into()),
                            tool_args: Some(r#"{"cmd":"ls"}"#.into()),
                            collapsed: true,
                        },
                    ])
                } else {
                    Err("内部路径细节不应出现在响应里".to_string())
                }
            }),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
        });
        let app = router(state.clone());
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "cm".into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });

        // (1) 无 cookie → 403（nest 内层 gate 结构性覆盖新路由）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-messages?agent_type=zcode&session_id=sess_hit",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "新端点必须过 gate");

        // (2) 缺 agent_type / 缺 session_id / 空白 session_id → 400
        for uri in [
            "/m/api/v1/session-messages?session_id=sess_hit",
            "/m/api/v1/session-messages?agent_type=zcode",
            "/m/api/v1/session-messages?agent_type=zcode&session_id=%20%20",
        ] {
            let r = app
                .clone()
                .oneshot(req("GET", uri, Some("mam_device=cm"), None))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "缺参必须 400：{uri}");
        }

        // (3) 命中 → 200 + camelCase 载荷 + no-store；参数正确传入注入源（limit 默认 200）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-messages?agent_type=zcode&session_id=sess_hit",
                Some("mam_device=cm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "会话正文是门禁下私有数据，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"messages\"") && body.contains("\"toolName\":\"Bash\""),
            "载荷必须 camelCase（toolName/toolArgs/collapsed），实际 {body}"
        );
        assert!(body.contains("\"toolArgs\"") && body.contains("\"collapsed\":true"));
        assert_eq!(
            captured.lock().unwrap().first().cloned(),
            Some(("zcode".to_string(), "sess_hit".to_string(), 200)),
            "handler 应把 agent_type/session_id/默认 limit 传给内容源"
        );

        // (4) 读取失败 → 404 空语义；limit 查询参数透传
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-messages?agent_type=dsh&session_id=sess_miss&limit=50",
                Some("mam_device=cm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "读取失败必须 404");
        let body = body_string(r).await;
        assert!(!body.contains("内部路径细节"), "错误细节只进日志不外泄");
        assert_eq!(
            captured.lock().unwrap().last().cloned(),
            Some(("dsh".to_string(), "sess_miss".to_string(), 50)),
            "limit 查询参数应透传"
        );
    }
}
