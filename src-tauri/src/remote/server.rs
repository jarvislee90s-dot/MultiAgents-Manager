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

/// 活跃连接句柄表（clippy::type_complexity 门禁适配：复杂类型抽别名，语义同原稿内联形）
type DeviceConns = std::collections::HashMap<String, Vec<(u64, tokio::sync::oneshot::Sender<()>)>>;

/// SSE 连接注册表（M4 T0a）：device_id → 活跃连接句柄表。
/// 断连语义：吊销/停止时对目标设备的全部连接发 oneshot 关闭信号，
/// SSE 流的 `take_until` 收到信号即终止 → axum 关闭该 HTTP 连接；
/// 自然断开（客户端关页）由 CleanupStream 的 Drop 反注册。
/// 锁粒度：单 Mutex 短临界区（register/unregister/disconnect 均无 IO），
/// 不与 store/pairing 锁嵌套（锁序红线：registry 永远最后进最先出）。
#[derive(Default)]
pub struct SseRegistry {
    inner: std::sync::Mutex<DeviceConns>,
    next_id: std::sync::atomic::AtomicU64,
}

impl SseRegistry {
    /// 注册一条连接：返回 (连接 id, 关闭信号接收端)。
    /// 返回的 Receiver 在 disconnect_device/disconnect_all 或 Sender 被 drop 时给出信号
    pub fn register(&self, device: &str) -> (u64, tokio::sync::oneshot::Receiver<()>) {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inner
            .lock()
            .unwrap()
            .entry(device.to_string())
            .or_default()
            .push((id, tx));
        (id, rx)
    }

    /// 自然断开反注册（幂等：未知 id 静默忽略）。
    /// 写法适配（clippy::option_map_unit_fn 门禁）：原稿 `.map(|v| …)` 改 `if let`，语义不变
    pub fn unregister(&self, device: &str, id: u64) {
        if let Some(v) = self.inner.lock().unwrap().get_mut(device) {
            v.retain(|(i, _)| *i != id);
        }
    }

    /// 断开指定设备的全部连接，返回断开数
    pub fn disconnect_device(&self, device: &str) -> usize {
        self.inner
            .lock()
            .unwrap()
            .remove(device)
            .map(|v| {
                // 编译适配（oneshot::Sender::send 消费 self，不能按引用迭代 send）：
                // 先取长度，再按值迭代逐个 send
                let n = v.len();
                for (_, tx) in v {
                    let _ = tx.send(());
                }
                n
            })
            .unwrap_or(0)
    }

    /// 断开全部设备连接（停止远程 / 全部吊销），返回断开数
    pub fn disconnect_all(&self) -> usize {
        let mut map = self.inner.lock().unwrap();
        let n: usize = map.values().map(Vec::len).sum();
        // 编译适配（send 消费 Sender，需持所有权迭代）：drain 等价于原稿的
        // 「逐个 send 后 map.clear()」
        for (_, v) in map.drain() {
            for (_, tx) in v {
                let _ = tx.send(());
            }
        }
        n
    }

    /// 该设备是否存在活跃连接（Task 7 花名册在线口径数据源）
    pub fn has(&self, device: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .get(device)
            .is_some_and(|v| !v.is_empty())
    }
}

/// 自然断开清理包装：axum drop SSE 流时反注册注册表项（不留陈旧句柄泄漏）。
/// pub(crate) + 字段同可见性（编译适配）：api.rs（兄弟模块）按简报原稿以字段字面量构造
pub(crate) struct CleanupStream<S> {
    pub(crate) inner: S,
    pub(crate) reg: std::sync::Arc<SseRegistry>,
    pub(crate) device: String,
    pub(crate) conn_id: u64,
}
impl<S: futures::Stream> futures::Stream for CleanupStream<S> {
    type Item = S::Item;
    fn poll_next(
        // 编译适配（unused_mut，-D warnings 门禁）：map_unchecked_mut 按值消费 self，
        // 绑定无需 mut
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // Safety: 无 Unpin 约束需求经字段投影转发（inner 已被 take_until 包装为 Unpin 流链）
        unsafe { self.map_unchecked_mut(|s| &mut s.inner).poll_next(cx) }
    }
}
impl<S> Drop for CleanupStream<S> {
    fn drop(&mut self) {
        self.reg.unregister(&self.device, self.conn_id);
    }
}

/// via 判定域名源接缝类型（clippy type_complexity 收敛别名）
pub type ViaHostsSource = dyn Fn() -> Option<(Vec<String>, Vec<String>)> + Send + Sync;

pub struct RemoteState {
    /// 会话数据源（P8 同源）：生产 = adapter::get_all_sessions；测试注入
    pub session_source: Box<dyn Fn() -> crate::session::SessionsResponse + Send + Sync>,
    /// 设备存储注入缝：生产 `DeviceStore::global()`；测试 `DeviceStore::memory()`（零接触真实 ~/.mam）
    pub store: super::pairing::DeviceStore,
    /// host 载荷注入缝（M3 Task 1）：生产 = remote::host_info()；测试注入假 json（零 DB）
    pub host_source: Box<dyn Fn() -> serde_json::Value + Send + Sync>,
    /// 会话内容源注入缝（M3 Task 7）：生产 = content::read_session_messages（八工具
    /// 统一出口）；测试注入假源（零接触真实 ~/.zcode ~/.dsh 等数据目录）
    pub message_source: Box<super::content::MessageSourceFn>,
    /// 文件路径源注入缝（M3 Task 8）：生产 = files::extract_file_paths（复用
    /// content 层读取的泛化提取）；测试注入假源（返回固定路径表）
    pub path_source: Box<super::files::PathSourceFn>,
    /// 跃迁事件通道（M3 Task 5）：生产 = watcher::event_sender()（全进程同一通道，
    /// 与 SessionWatcher::start 的循环共享）；测试注入新建空通道即可。
    /// **订阅端消费即去重完成**（铁律 4）：事件只含边沿（见 watcher::diff_transitions）
    pub watcher_tx: tokio::sync::broadcast::Sender<super::watcher::TransitionEvent>,
    /// SSE 连接注册表（M4 T0a）：吊销/停止即时断连 + 在线口径数据源
    pub sse_registry: std::sync::Arc<SseRegistry>,
    /// 设备上限注入缝（M4 T2c）：生产 = 读 remote.max_devices KV；测试注入常量
    /// （零 DAO 接触——端点测试不触碰真实 ~/.mam）
    pub max_devices_source: Box<dyn Fn() -> usize + Send + Sync>,
    /// per-IP 限速状态机（M5 A3）：POST /pair/pin 锁内查改；内存态重启即清
    pub pin_limiter: std::sync::Mutex<crate::remote::pin::PinRateLimiter>,
    /// PIN 源注入缝（M5 A3）：生产 = pin::get_pin（全局 KV）；测试注入固定值（零 DB）
    pub pin_source: Box<dyn Fn() -> Option<String> + Send + Sync>,
    /// 时钟注入缝（M5 A3）：生产 = chrono 毫秒；测试注入可推进原子量——限速锁定
    /// 到期测试用它推进时间（与 pairing 时代 state_with_clock 同目的，零 sleep）
    pub now_source: Box<dyn Fn() -> i64 + Send + Sync>,
    /// 隧道域名并集注入缝（M5 A3，gate 回环豁免消费）：生产 = 从 tunnel::snapshot()
    /// 抽当前隧道地址的域名部分（A5 双通道聚合时改聚合实现，签名不变）；测试注入固定域名。
    /// **None = 隧道快照错误终态（fail-closed 哨兵，评审 Important 1）**：豁免的 Host 条件
    /// 依赖域名名单，快照错误时无从判定 Host 是否隧道域名——gate 收到 None 必须**完全
    /// 跳过本机豁免**（回环 + 任意 Host 都不免费），而非把 None 当空名单（那是 fail-open：
    /// Host 条件恒满足 → 回环流量全豁免）
    pub tunnel_hosts_source: Box<dyn Fn() -> Option<Vec<String>> + Send + Sync>,
    /// via 分通道域名注入缝（M5 A3，/pair/pin 配对时刻消费）：生产 = snapshot 按
    /// mode 分拣 quick/named 域名；测试注入固定域名。与 tunnel_hosts_source 同源分形——
    /// gate 豁免只要"是否隧道域名"并集，via 需要通道区分
    pub via_hosts_source: Box<ViaHostsSource>,
    /// 注入器缝（M7 Task 5 方案 A 提前缝合）：生产 = RealInjector（macOS 三通道执行层；
    /// Windows 占位，Task 15 补真实现）；测试可替换 FakeInjector。
    /// 消费方：inject::queue::flush_one（flush 投递）+ Task 6 的 session-send 直发与
    /// flush 循环接线（路由/handler/serve 挂载届时合入）
    pub injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
}

/// API 子路由：业务端点 + /pair/pin + 内层 fallback（未知 API 路径直接 403）+ gate 内层 layer。
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
        // /session-files（M3 Task 8）：会话涉及的文件路径表（链接化数据源）
        .route("/session-files", get(api::session_files))
        // /file（M3 Task 8）：会话 cwd 内安全文件读取（预览）
        .route("/file", get(api::read_file))
        // M5 A3：访问密码端点——密码制唯一换 cookie 入口（gate 放行名单同步收口为
        // /pair/pin 精确相等；旧 /pair 直通与 /pair/* 审批路由已删除，未知路径落
        // 内层 fallback 403）
        .route("/pair/pin", post(api::pair_pin))
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
    // M5 A3：来源 IP 记录——into_make_service_with_connect_info 注入 ConnectInfo
    // extension（pair_pin 的限速键/指纹与 gate 本机豁免判定依赖；oneshot 测试在请求侧自补）
    axum::serve(
        listener,
        router_with_static(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .map_err(|e| format!("serve: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    // M4 T0a（编译硬阻断补 import，先例同上）：SSE 流逐帧消费需要 StreamExt::next
    use futures::StreamExt as _;

    fn test_state() -> Arc<RemoteState> {
        Arc::new(RemoteState {
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 7,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(), // 内存库——测试不碰真实 ~/.mam
            // M7 Task 5：注入器缝——本组测试不触 flush 路径，用生产占位
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            host_source: Box::new(|| {
                serde_json::json!({
                    "host": { "name": "test-host", "platform": "macos", "version": "0.0.0-test" },
                    "enabledTools": ["claude"]
                })
            }),
            // M3 Task 7：本组测试不触 /session-messages，注入恒 Err 的桩
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            // 本组测试不触 /session-files /file：注入恒空的路径源
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            // M3 Task 5：测试用空事件通道（不启动 watcher——零后台扫描）
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            // M4 T0a（brief 指定）：本任务新增字段，测试用空注册表即可
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            // M5 A3 新字段：限速器全新；PIN 恒 "1234"；时钟真实毫秒；无隧道域名
            // （豁免面关闭——既有"无 cookie → 403"断言不受本机豁免影响：空 Host fail closed）
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
        })
    }

    async fn body_string(b: axum::http::Response<Body>) -> String {
        String::from_utf8(b.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn gate_403_matrix_and_pin_pair_flow() {
        let app = crate::remote::server::router(test_state());
        // 1) 无 cookie 访问 sessions → 403（请求无 Host 头——空 Host fail closed，
        //    不会误入本机豁免；测试环境也不注入 ConnectInfo，按非本地处理）
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
        // 2) PIN 错 → 401 invalid_pin（非 403：配对入口的错误语义是可鉴别的 401+计数）
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair/pin")
                    .header("content-type", "application/json")
                    // pair_pin 提取 ConnectInfo（来源 IP 入限速与指纹）——oneshot 请求侧自补
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .body(Body::from(r#"{"pin":"9999"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
        // 3) PIN 正确 → 200 + Set-Cookie mam_device
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair/pin")
                    .header("content-type", "application/json")
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .body(Body::from(r#"{"pin":"1234"}"#))
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
        // 5) PIN 重放再配对 → 仍 200（与旧一次性 token 语义相反：访问密码常驻，
        //    同指纹重绑命中旧行——不新增设备）
        let r = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/m/api/v1/pair/pin")
                    .header("content-type", "application/json")
                    .extension(axum::extract::ConnectInfo(
                        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                    ))
                    .body(Body::from(r#"{"pin":"1234"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "PIN 非一次性——重放可再次配对");
    }

    // ==== 追加测试（简报单个之外；理由：锁定简报未覆盖但已裁决的契约） ====

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
        // M5 A1 评审修复随记：api::pair 现以 ConnectInfo 取来源 IP 入指纹——
        // oneshot 不注入该 extension，测试请求侧自补（与 post_json 同一适配）
        b = b.extension(axum::extract::ConnectInfo(
            "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
        ));
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

    /// gate 的滑动 TTL：超窗设备拒绝、活跃设备过闸即刷新 last_seen
    #[tokio::test]
    async fn gate_refreshes_last_seen_and_rejects_stale_device() {
        let state = test_state();
        let now = chrono::Utc::now().timestamp_millis();
        // M5 A1 upsert 按指纹（sha256(ua|ip)）去重：ua/ip 全空的设备会互相撞键合并成一行，
        // 故按 id 派生合成值保证 stale/fresh 两行并存（与真机"不同设备"语义一致）
        let dev = |id: &str, paired_at: i64| crate::remote::pairing::NewDevice {
            id: id.into(),
            name: String::new(),
            ua: format!("ua-{id}"),
            origin_ip: format!("ip-{id}"),
            via: String::new(),
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

    /// gate 拒绝不可区分性（评审 Important 4）："无 cookie"与"cookie 无效（未配对 id）"
    /// 必须给出**完全一致**的响应——状态码、响应体、以及无 Set-Cookie 等额外头。
    /// 若变异为"两种失败写入不同响应体/附带头"，即构成设备有效性预言机——本测试锁死该不变量
    #[tokio::test]
    async fn gate_rejections_are_indistinguishable() {
        let state = test_state();
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

    /// 追加静态路由后 pair/pin 仍可换 cookie（放行名单随 nest 剥前缀改为相对
    /// `/pair/pin` 的回归锁定——若名单仍写绝对路径，此测试 403 失败）
    #[tokio::test]
    async fn pair_still_reachable_after_task7_static_appended() {
        let app = router(test_state())
            .route("/m", get(|| async { "pair-page" }))
            .fallback(|| async { "static" });
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/pair/pin",
                None,
                Some(r#"{"pin":"1234"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "nest 内层 gate 的相对放行名单必须保住 pair/pin"
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
            session_source: Box::new(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                crate::session::SessionsResponse {
                    sessions: vec![],
                    total_count: 42,
                    waiting_count: 0,
                }
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            host_source: Box::new(|| serde_json::Value::Null), // 本测试不触 /host
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            // 本组测试不触 /session-files /file：注入恒空的路径源
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0, // M3 Task 5：空事件通道
            // M4 T0a：本组测试不触 SSE 断连，空注册表即可
            sse_registry: Arc::new(SseRegistry::default()),
            // M4 T2（brief 指定）：审批队列固定生成器（code 恒 "0000"——本组测试不触
            // /pair/* 则不被消费）；上限注入常量 3 = 默认上限（零 DAO 接触）
            max_devices_source: Box::new(|| 3),
            // M5 A3 新字段：同 test_state 口径（限速器全新 / PIN "1234" / 真实时钟 / 无隧道域名）
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
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
                    via: String::new(),
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

    /// 预置有效设备（SSE 测试专用：复用 state_with_clock 的注入缝，零接触真实设备表）。
    /// M5 A1：ua/ip 按 id 派生——upsert 按指纹去重，全空值会在同库多次预置时撞键合并
    fn persist_device(state: &Arc<RemoteState>, id: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: id.into(),
                    name: String::new(),
                    ua: format!("ua-{id}"),
                    origin_ip: format!("ip-{id}"),
                    via: String::new(),
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
        let state = test_state();
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
        let state = test_state();
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

    // ---- SseRegistry 单元（M4 T0a）----

    #[test]
    fn sse_registry_register_then_disconnect_device() {
        let reg = SseRegistry::default();
        let (id1, mut rx1) = reg.register("dev-a");
        let (_id2, rx2) = reg.register("dev-a");
        let (_id3, _rx3) = reg.register("dev-b");
        assert!(reg.has("dev-a") && reg.has("dev-b"));
        // 断 dev-a：两条连接全断，dev-b 不受影响
        assert_eq!(reg.disconnect_device("dev-a"), 2);
        assert!(rx1.try_recv().is_ok());
        drop(rx2); // 第二条 receiver 被 send 唤醒后丢弃即可（Sender 已 send）
        assert!(!reg.has("dev-a") && reg.has("dev-b"));
        // 自然断开清理：unregister 幂等
        reg.unregister("dev-a", id1);
        reg.unregister("dev-a", id1); // 不 panic
        assert_eq!(reg.disconnect_all(), 1); // 只剩 dev-b
    }

    #[test]
    fn sse_registry_sender_dropped_when_entry_replaced_by_disconnect() {
        let reg = SseRegistry::default();
        // 编译适配（E0596）：try_recv 需 &mut self，原稿 rx 未加 mut
        let (_id, mut rx) = reg.register("dev");
        reg.disconnect_device("dev");
        // 断连后 Sender 已移除；receiver 侧已收到信号
        // 断言写法适配（clippy::redundant_pattern_matching 门禁）：matches! → is_ok()，语义不变
        assert!(rx.try_recv().is_ok());
    }

    // ---- 吊销即时断流集成（SSE 流随 disconnect 终止）----

    #[tokio::test]
    async fn sse_stream_ends_when_device_disconnected() {
        let state = test_state();
        // 直通配对拿有效 cookie（复用既有 pair 流程的简化版：直接 persist 一个设备）
        state.store.with(|c| {
            let _ = crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "dev-sse".into(),
                    name: String::new(),
                    ua: String::new(),
                    origin_ip: String::new(),
                    via: String::new(),
                    // 偏离简报原稿一处（测试语义硬阻断）：原稿 paired_at: 0——persist 以
                    // paired_at 充当 last_seen_at，gate 按真实时钟做滑动 TTL 判定，
                    // 0 必被 403 拒绝。改为当前时刻，语义等价于「刚配对的活跃设备」
                    // （既有 persist_device 测试助手同一取值先例）
                    paired_at: chrono::Utc::now().timestamp_millis(),
                },
            );
        });
        let app = super::router_with_static(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/m/api/v1/events")
                    .header("cookie", "mam_device=dev-sse")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let mut stream = resp.into_body().into_data_stream();
        // 读到首帧（snapshot）证明流已建立
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("首帧超时")
            .unwrap();
        assert!(first.is_ok());
        // 吊销 → 流必须结束（next 返回 None）
        assert_eq!(state.sse_registry.disconnect_device("dev-sse"), 1);
        let end = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("断连后流未在 2s 内结束");
        assert!(end.is_none());
    }

    // ==== M5 A4：吊销收窄（gate 级回归） ====

    /// M5 A4 吊销收窄回归：热重启半程（stop revoke=false）后设备 cookie 仍过闸
    /// （「改绑定/改端口热重启不掉线」），显性关闭（revoke=true）后同 cookie 403。
    /// 真实 stop_server 触碰全局 DB / 电源锁 / 隧道进程（零污染红线禁测）——此处以
    /// stop_server_core 注入与生产 stop_server 完全同形的 store/registry 闭包，
    /// 等价锁定「revoke 取值 → 设备有效性」这一收窄语义核心（重启后的监听生效半边
    /// 由 serve/start 既有路径承担，gate 与设备表不受重启影响的判据即本测试）
    #[tokio::test]
    async fn hot_restart_without_revoke_keeps_device_cookie_valid() {
        let state = test_state();
        persist_device(&state, "hn");
        let app = router(state.clone());
        // 初始：cookie 过闸
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/sessions",
                Some("mam_device=hn"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);

        // 热重启半程：停监听不吊销（闭包与生产 stop_server(false) 同形）
        let st_reg = state.clone();
        let st_store = state.clone();
        crate::remote::stop_server_core(
            None,
            false,
            move || {
                st_reg.sse_registry.disconnect_all();
            },
            move || {
                let _ = st_store.store.with(crate::remote::pairing::revoke_all);
            },
            || {},
            || {},
        );
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/sessions",
                Some("mam_device=hn"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "revoke=false（热重启）后 cookie 必须仍过闸（设备不掉线）"
        );

        // 显性关闭：吊销 → 同一 cookie 403（收窄前后对照）
        let st_reg = state.clone();
        let st_store = state.clone();
        crate::remote::stop_server_core(
            None,
            true,
            move || {
                st_reg.sse_registry.disconnect_all();
            },
            move || {
                let _ = st_store.store.with(crate::remote::pairing::revoke_all);
            },
            || {},
            || {},
        );
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/sessions",
                Some("mam_device=hn"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "revoke=true（显性关闭）后设备全吊销");
    }

    // ==== M3 Task 1：GET /m/api/v1/host（P8a/P8b 页头数据源） ====
    // 零污染：host 载荷经 host_source 注入缝供给（假 json），不触 settings DAO / 全局 DB。

    /// /host 端点矩阵：无 cookie 403（与 sessions 同一 gate，设备失效语义一致）；
    /// 有效设备 200 返回注入载荷 + Cache-Control: no-store（host 同属门禁下私有数据）
    #[tokio::test]
    async fn host_endpoint_is_gated_and_returns_injected_payload() {
        // 重建 state：host_source 注入假载荷（与 sessions_scan_* 重建 state 的先例一致）
        let state = Arc::new(RemoteState {
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 0,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            host_source: Box::new(|| {
                serde_json::json!({
                    "host": { "name": "jarvis-win", "platform": "windows", "version": "9.9.9-test" },
                    "enabledTools": ["claude", "zcode"]
                })
            }),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            // 本组测试不触 /session-files /file：注入恒空的路径源
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0, // M3 Task 5：空事件通道
            // M4 T0a：本组测试不触 SSE 断连，空注册表即可
            sse_registry: Arc::new(SseRegistry::default()),
            // M4 T2（brief 指定）：审批队列固定生成器（code 恒 "0000"——本组测试不触
            // /pair/* 则不被消费）；上限注入常量 3 = 默认上限（零 DAO 接触）
            max_devices_source: Box::new(|| 3),
            // M5 A3 新字段：同 test_state 口径（限速器全新 / PIN "1234" / 真实时钟 / 无隧道域名）
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
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
                    via: String::new(),
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
            session_source: Box::new(|| crate::session::SessionsResponse {
                sessions: vec![],
                total_count: 0,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(move |agent: &str, sid: &str, limit: usize| {
                cap.lock()
                    .unwrap()
                    .push((agent.to_string(), sid.to_string(), limit));
                if sid == "sess_hit" {
                    Ok(crate::remote::content::MessagesPage {
                        messages: vec![
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
                        ],
                        truncated: false,
                    })
                } else {
                    Err("内部路径细节不应出现在响应里".to_string())
                }
            }),
            // 本测试不触 /session-files：注入恒空的路径源
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            // M4 T0a：本组测试不触 SSE 断连，空注册表即可
            sse_registry: Arc::new(SseRegistry::default()),
            // M4 T2（brief 指定）：审批队列固定生成器（code 恒 "0000"——本组测试不触
            // /pair/* 则不被消费）；上限注入常量 3 = 默认上限（零 DAO 接触）
            max_devices_source: Box::new(|| 3),
            // M5 A3 新字段：同 test_state 口径（限速器全新 / PIN "1234" / 真实时钟 / 无隧道域名）
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
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
                    via: String::new(),
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
        // Bug 1 修复（M3 验收）：载荷携带 truncated（头部截断标记），移动端据此决定
        // 是否提供「加载更早消息」
        assert!(
            body.contains("\"truncated\":false"),
            "载荷必须含 truncated 字段，实际 {body}"
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

    // ==== M3 Task 8：GET /m/api/v1/file + /session-files（安全文件读取与路径提取） ====
    // 零污染：session_source 注入 tempdir 项目目录的会话，被读文件均为 tempdir 内
    // 现造文件；path_source 注入固定路径表——不触任何真实数据目录

    /// file / session-files 端点矩阵：gate 403 → 缺参 400 → 会话不存在 404 →
    /// cwd 内文本 200（JSON 载荷 + no-store）→ 图片 200（二进制 + Content-Type）→
    /// 越界 403 → 超限 403（与越界不可区分，探测面最小化，进度台账 #12）
    #[tokio::test]
    async fn file_endpoints_are_gated_and_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();
        std::fs::write(tmp.path().join("hello.txt"), "hello mam").unwrap();
        std::fs::write(tmp.path().join("pic.png"), [0x89u8, b'P', b'N', b'G']).unwrap();
        std::fs::write(tmp.path().join("big.txt"), vec![b'a'; 500 * 1024 + 1]).unwrap();
        let session = crate::session::Session {
            id: "sess_file".into(),
            agent_type: crate::session::AgentType::Claude,
            project_name: "proj".into(),
            project_path: cwd.clone(),
            title: None,
            git_branch: None,
            github_url: None,
            status: crate::session::SessionStatus::Idle,
            last_message: None,
            last_message_role: None,
            last_activity_at: "2026-09-15T00:00:00Z".into(),
            pid: 1,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: crate::session::ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        };
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| {
                (
                    vec![crate::remote::files::FileEntry {
                        path: "/absent/proj/src/main.rs".to_string(),
                        last_seq: 1,
                        last_ts: None,
                        hits: 1,
                        modified: false,
                        origin: crate::remote::files::FileOrigin::ToolRead,
                    }],
                    false,
                )
            }),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            // M4 T0a：本组测试不触 SSE 断连，空注册表即可
            sse_registry: Arc::new(SseRegistry::default()),
            // M4 T2（brief 指定）：审批队列固定生成器（code 恒 "0000"——本组测试不触
            // /pair/* 则不被消费）；上限注入常量 3 = 默认上限（零 DAO 接触）
            max_devices_source: Box::new(|| 3),
            // M5 A3 新字段：同 test_state 口径（限速器全新 / PIN "1234" / 真实时钟 / 无隧道域名）
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
        });
        let app = router(state.clone());
        persist_device(&state, "fe");

        // (1) gate：无 cookie 访问 file / session-files → 403（nest 内层 gate 结构性覆盖）
        for uri in [
            "/m/api/v1/file?session_id=sess_file&path=hello.txt",
            "/m/api/v1/session-files?agent_type=claude&session_id=sess_file",
        ] {
            let r = app
                .clone()
                .oneshot(req("GET", uri, None, None))
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "新端点必须过 gate：{uri}");
        }

        // (2) 缺参 / 空白参数 → 400
        for uri in [
            "/m/api/v1/file?path=hello.txt",
            "/m/api/v1/file?session_id=sess_file",
            "/m/api/v1/file?session_id=%20&path=hello.txt",
        ] {
            let r = app
                .clone()
                .oneshot(req("GET", uri, Some("mam_device=fe"), None))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "缺参必须 400：{uri}");
        }

        // (3) 会话不在快照中 → 404
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_miss&path=hello.txt",
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "session_id 不在快照必须 404");

        // (4) cwd 内文本 → 200 JSON {content, mime, size} + no-store（相对路径形态）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_file&path=hello.txt",
                Some("mam_device=fe"),
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
            "门禁下的私有文件内容禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"content\":\"hello mam\"")
                && body.contains("\"mime\":\"text/plain\"")
                && body.contains("\"size\":9"),
            "文本载荷形状 content/mime/size 三键，实际 {body}"
        );

        // (5) 图片 → 200 二进制 + Content-Type: image/png（绝对路径形态 + 含空格文件名
        //     的 URL 编码变体一并覆盖查询串解码）
        std::fs::write(tmp.path().join("with space.png"), [0x89u8, b'P']).unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                &format!(
                    "/m/api/v1/file?session_id=sess_file&path={}",
                    uri_encode(&tmp.path().join("with space.png").to_string_lossy())
                ),
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "含空格的绝对路径（URL 编码）应可读取");
        assert_eq!(header(&r, "content-type"), "image/png");
        // 终审 Important 2：所有图片二进制响应必须带嗅探防护双头——SVG 以顶层文档
        // 加载时可执行内嵌脚本（同源脚本可 fetch 会话数据，SameSite=Lax 不防同源），
        // CSP 断脚本/取资源 + nosniff 防 MIME 嗅探误判（png 与 svg 同一分支，双头断言一致）
        assert_eq!(header(&r, "content-security-policy"), "default-src 'none'");
        assert_eq!(header(&r, "x-content-type-options"), "nosniff");
        let bytes = r.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.as_ref(), &[0x89, b'P'], "图片走二进制直传");

        // (5b) SVG 直出（终审 Important 2 的直接场景）：image/svg+xml 同样带
        //      CSP + nosniff 双头——内容可含脚本的图片 mime 是防护重点
        std::fs::write(
            tmp.path().join("icon.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        )
        .unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_file&path=icon.svg",
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "cwd 内 svg 应可读取");
        assert_eq!(header(&r, "content-type"), "image/svg+xml");
        assert_eq!(
            header(&r, "content-security-policy"),
            "default-src 'none'",
            "SVG 直出必须断一切脚本与子资源"
        );
        assert_eq!(header(&r, "x-content-type-options"), "nosniff");

        // (6) 不存在（cwd 外任意路径）→ 403 + 原因码 not_found（M5 P2-a：原因
        // 写在报错处，仅已过闸设备可见）。探针用确定不存在的合成路径——全盘
        // 放开语义下 /etc/passwd 在 macOS 真实存在会 200（原探针环境依赖：
        // Windows 过 macOS 挂，合并验证期抓获修正）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_file&path=/__mam_nonexistent__/passwd",
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "不存在必须 403");
        let b = body_string(r).await;
        assert!(b.contains("not_found"), "403 体必须带原因码 not_found: {b}");

        // (7) 超限 → 403 + 原因码 too_large（与 (6) 可区分——M5 P2-a 裁决：
        // 原因写在报错处，替代旧「空体不可区分」口径）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_file&path=big.txt",
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "超限必须 403");
        let b = body_string(r).await;
        assert!(b.contains("too_large"), "403 体必须带原因码 too_large: {b}");

        // (8) /session-files：缺参 400 → 命中 200 {files:[...]} + no-store
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-files?session_id=sess_file",
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-files?agent_type=claude&session_id=sess_file",
                Some("mam_device=fe"),
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
            "文件路径表是门禁下私有数据，禁止中间层缓存"
        );
        // M3+ 富化形状：结构化条目（camelCase）+ truncated
        let files_body = body_string(r).await;
        assert!(
            files_body.contains("\"path\":\"/absent/proj/src/main.rs\"")
                && files_body.contains("\"lastSeq\":1")
                && files_body.contains("\"hits\":1")
                && files_body.contains("\"truncated\":false"),
            "session-files 应透传注入路径源的结构化条目，实际 {files_body}"
        );
    }

    /// 查询参数值的最小 URL 编码（测试助手：空格 → %20；其余字符测试数据不含）
    fn uri_encode(s: &str) -> String {
        s.replace(' ', "%20")
    }

    /// 从 Set-Cookie 取设备 id（`mam_device=<id>; Path=/m; ...`）
    fn cookie_device_id(resp: &axum::http::Response<Body>) -> String {
        let v = resp
            .headers()
            .get("set-cookie")
            .expect("应携带 Set-Cookie")
            .to_str()
            .unwrap()
            .to_string();
        v.split(';')
            .next()
            .unwrap()
            .strip_prefix("mam_device=")
            .unwrap()
            .to_string()
    }

    // ==== M5 A3：/pair/pin 端点 + gate 本机豁免（回环 + 本地 Host 双条件）====
    // 安全是本任务的存在意义：穿透测试锁定「隧道流量无法借回环穿透」。

    /// A3 专用 state：PIN 源 / 设备上限 / 隧道域名 / 可推进时钟全注入（零 DB 零真实隧道）。
    /// 返回时钟句柄供限速到期测试推进（state_with_clock 同目的，零 sleep）
    fn a3_state(
        pin: Option<&str>,
        max_devices: usize,
        quick_hosts: &[&str],
        named_hosts: &[&str],
        tunnel_error: bool,
    ) -> (Arc<RemoteState>, Arc<std::sync::atomic::AtomicI64>) {
        let t = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000));
        let now = t.clone();
        let quick: Vec<String> = quick_hosts.iter().map(|s| s.to_string()).collect();
        let named: Vec<String> = named_hosts.iter().map(|s| s.to_string()).collect();
        let q_tunnel = quick.clone();
        let n_tunnel = named.clone();
        let pin_owned: Option<String> = pin.map(|s| s.to_string());
        (
            Arc::new(RemoteState {
                session_source: Box::new(|| crate::session::SessionsResponse {
                    sessions: vec![],
                    total_count: 7,
                    waiting_count: 0,
                }),
                store: crate::remote::pairing::DeviceStore::memory(),
                // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
                injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
                host_source: Box::new(|| serde_json::Value::Null),
                message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
                path_source: Box::new(|_, _, _| (Vec::new(), false)),
                watcher_tx: tokio::sync::broadcast::channel(64).0,
                sse_registry: Arc::new(SseRegistry::default()),
                max_devices_source: Box::new(move || max_devices),
                pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
                pin_source: Box::new(move || pin_owned.clone()),
                now_source: Box::new(move || now.load(std::sync::atomic::Ordering::SeqCst)),
                tunnel_hosts_source: Box::new(move || {
                    if tunnel_error {
                        return None; // 评审 Important 1：错误态哨兵——gate 必须跳过豁免
                    }
                    Some(q_tunnel.iter().chain(n_tunnel.iter()).cloned().collect())
                }),
                via_hosts_source: Box::new(move || Some((quick.clone(), named.clone()))),
            }),
            t,
        )
    }

    /// A3 自由请求构造：来源地址 / Host 头 / UA / cookie / JSON body 全可指定。
    /// host 不给 = 请求**不带 Host 头**（axum oneshot 不会自动补——正好锁定"空 Host fail closed"）
    #[allow(clippy::too_many_arguments)]
    fn http_req(
        method: &str,
        uri: &str,
        addr: &str,
        host: Option<&str>,
        ua: Option<&str>,
        cookie: Option<&str>,
        body: Option<&str>,
    ) -> axum::http::Request<Body> {
        let mut b = axum::http::Request::builder().method(method).uri(uri);
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        if let Some(h) = host {
            b = b.header("host", h);
        }
        if let Some(u) = ua {
            b = b.header("user-agent", u);
        }
        if let Some(c) = cookie {
            b = b.header("cookie", c);
        }
        // axum oneshot 不注入 ConnectInfo extension——请求侧自补（post_json 同一适配）
        b = b.extension(axum::extract::ConnectInfo(
            addr.parse::<std::net::SocketAddr>().unwrap(),
        ));
        b.body(match body {
            Some(s) => Body::from(s.to_string()),
            None => Body::empty(),
        })
        .unwrap()
    }

    /// POST /pair/pin 收敛助手（无 cookie——配对入口本就无凭据）
    fn pin_post(
        addr: &str,
        host: Option<&str>,
        ua: Option<&str>,
        body: &str,
    ) -> axum::http::Request<Body> {
        http_req(
            "POST",
            "/m/api/v1/pair/pin",
            addr,
            host,
            ua,
            None,
            Some(body),
        )
    }

    /// 穿透矩阵（安全关键，变异自证：去掉 is_local_access 的 Host 条件——只查回环——
    /// 断言 1 必红：127.0.0.1 + 隧道 Host 的公网隧道流量会被免密放进看板）：
    /// 1. 127.0.0.1 + Host=隧道域名（模拟 cloudflared 本机回环转发公网流量）+ 无 cookie → 403；
    /// 2. 回环（127.0.0.1 / ::1）+ 本地 Host → 免密直达 /sessions 200；
    /// 3. 非回环 + 本地 Host + 无 cookie → 403（豁免只信来源回环）；
    /// 4. 回环 + 缺 Host 头 → 403（空 Host fail closed）。
    #[tokio::test]
    async fn gate_local_exempt_requires_loopback_and_local_host() {
        let (state, _t) = a3_state(Some("1234"), 3, &["mam-test.trycloudflare.com"], &[], false);
        let app = router(state);

        // 1) 穿透：回环 + 隧道 Host → 403 不得免密（带端口的 Host 形态归一后同样拦下）
        for host in [
            "mam-test.trycloudflare.com",
            "mam-test.trycloudflare.com:443",
        ] {
            let r = app
                .clone()
                .oneshot(http_req(
                    "GET",
                    "/m/api/v1/sessions",
                    "127.0.0.1:40000",
                    Some(host),
                    None,
                    None,
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(
                r.status(),
                403,
                "隧道域名 Host 的回环流量不得免密（穿透防线）：{host}"
            );
        }

        // 2) 本机免密：回环 + 本地 Host → 直达看板 200（数据来自注入源 totalCount=7）
        for addr in ["127.0.0.1:40001", "[::1]:40002"] {
            let r = app
                .clone()
                .oneshot(http_req(
                    "GET",
                    "/m/api/v1/sessions",
                    addr,
                    Some("localhost:9420"),
                    None,
                    None,
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "{addr} 本机免密应直达看板");
            assert!(body_string(r).await.contains("\"totalCount\":7"));
        }

        // 3) 非回环 + 本地 Host → 403
        let r = app
            .clone()
            .oneshot(http_req(
                "GET",
                "/m/api/v1/sessions",
                "10.0.0.5:40003",
                Some("localhost:9420"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "非回环来源不得借本地 Host 免密");

        // 4) 回环 + 缺 Host 头 → 403（fail closed）
        let r = app
            .oneshot(http_req(
                "GET",
                "/m/api/v1/sessions",
                "127.0.0.1:40004",
                None,
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "缺 Host 头必须按非本地处理（fail closed）");
    }

    /// 快照错误态 fail-closed（评审 Important 1）：隧道快照处于错误终态时，豁免依赖的
    /// 域名名单无从判定——gate 收到 None 哨兵必须**完全跳过本机豁免**：
    /// 回环 + 本地 Host（正常态本应免密 200）→ 仍要求 cookie（403）；
    /// 有效 cookie 的请求不受影响（fail-closed 只收紧豁免路径，不断 cookie 路径）。
    /// 变异锚点：若把 None 当空名单处理（fail-open），断言 1 必红
    #[tokio::test]
    async fn gate_local_exempt_disabled_when_tunnel_snapshot_degraded() {
        let (state, _t) = a3_state(Some("1234"), 3, &["mam-test.trycloudflare.com"], &[], true);
        // 预置有效设备（cookie 路径的对照组；last_seen 取当前时刻——滑动 TTL 窗口内）
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: "degraded".into(),
                    name: String::new(),
                    ua: "ua-degraded".into(),
                    origin_ip: "ip-degraded".into(),
                    via: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });
        let app = router(state);

        // 1) 回环 + 本地 Host + 无 cookie → 403（豁免被快照错误完全关闭）
        let r = app
            .clone()
            .oneshot(http_req(
                "GET",
                "/m/api/v1/sessions",
                "127.0.0.1:41000",
                Some("localhost:9420"),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            403,
            "快照错误 ∧ 回环 → 不豁免（fail-closed，不得当空名单 fail-open）"
        );

        // 2) 同请求带有效 cookie → 200（fail-closed 只影响豁免，不影响 cookie 认证路径）
        let r = app
            .oneshot(http_req(
                "GET",
                "/m/api/v1/sessions",
                "127.0.0.1:41000",
                Some("localhost:9420"),
                None,
                Some("mam_device=degraded"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "有效 cookie 在快照错误态照常过闸");
    }

    /// 配对流（矩阵 4）：PIN 对 → 200 + Set-Cookie（携带 upsert 返回 id）+ 落行；
    /// via 按 Host 四分支（quick 域名 → quick / 本地 Host 非回环 → lan / named 域名 →
    /// named / 回环+本地 → local）；两台不同 UA/IP 设备 → 各自一行；同 UA+IP 重绑 →
    /// 同行 id（cookie 刷新）——A1 upsert 语义在新端点上存活
    #[tokio::test]
    async fn pin_pair_flow_via_four_branches_two_devices_and_rejoin() {
        let (state, _t) = a3_state(
            Some("1234"),
            10,
            &["mam-test.trycloudflare.com"],
            &["mam.example.com"],
            false,
        );
        let app = router_with_static(state.clone());

        // 设备 A：经 quick 隧道域名 → via=quick；cookie 五属性全断言（评审 Important 3 既有口径）
        let r = app
            .clone()
            .oneshot(pin_post(
                "203.0.113.7:51000",
                Some("mam-test.trycloudflare.com"),
                Some("ua-A"),
                r#"{"pin":"1234","name":"我的手机"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "PIN 正确必须 200");
        let cookie = r
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let id_a = cookie
            .split("mam_device=")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        assert!(
            id_a.len() == 32 && id_a.chars().all(|c| c.is_ascii_hexdigit()),
            "device id 应为 32 位 hex，实际 {id_a:?}"
        );
        assert_eq!(
            cookie,
            format!(
                "mam_device={id_a}; Path=/m; HttpOnly; SameSite=Lax; Max-Age={}",
                crate::remote::pairing::DEVICE_TTL_MS / 1000
            ),
            "cookie 必须同时具备 mam_device=hex / Path=/m / HttpOnly / SameSite=Lax / Max-Age=180d"
        );
        let row_a: (String, String, String, String) = state.store.with(|c| {
            c.query_row(
                "SELECT via, name, ua, origin_ip FROM remote_devices WHERE id = ?1",
                [&id_a],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
        });
        assert_eq!(row_a.0, "quick", "Host==quick 域名 → via=quick");
        assert_eq!(row_a.1, "我的手机", "自报名落库");
        assert_eq!(row_a.2, "ua-A", "真实 UA 落库");
        assert_eq!(row_a.3, "203.0.113.7", "ConnectInfo 来源 IP 落库");

        // 设备 B：局域网直连（非回环 + 本地 Host）→ via=lan；缺 name → 默认名
        let r = app
            .clone()
            .oneshot(pin_post(
                "10.0.0.8:51001",
                Some("192.168.1.9:9420"),
                Some("ua-B"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let id_b = cookie_device_id(&r);
        let row_b: (String, String) = state.store.with(|c| {
            c.query_row(
                "SELECT via, name FROM remote_devices WHERE id = ?1",
                [&id_b],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        });
        assert_eq!(row_b.0, "lan", "非隧道 Host 的非回环来源 → via=lan");
        assert_eq!(row_b.1, "新设备", "缺省名回落默认名");

        // 设备 C：经 named 域名 → via=named
        let r = app
            .clone()
            .oneshot(pin_post(
                "198.51.100.3:51002",
                Some("mam.example.com:443"),
                Some("ua-C"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let id_c = cookie_device_id(&r);
        let via_c: String = state.store.with(|c| {
            c.query_row(
                "SELECT via FROM remote_devices WHERE id = ?1",
                [&id_c],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(via_c, "named", "Host==named 域名（带端口归一）→ via=named");

        // 设备 D：本机回环 + 本地 Host → via=local
        let r = app
            .clone()
            .oneshot(pin_post(
                "127.0.0.1:51003",
                Some("localhost:9420"),
                Some("ua-D"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let id_d = cookie_device_id(&r);
        let via_d: String = state.store.with(|c| {
            c.query_row(
                "SELECT via FROM remote_devices WHERE id = ?1",
                [&id_d],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(via_d, "local", "回环 + 非隧道 Host → via=local");

        // 四台设备 = 四行（UA/IP 互异，指纹各不同）
        let rows: i64 = state.store.with(|c| {
            c.query_row("SELECT COUNT(*) FROM remote_devices", [], |r| r.get(0))
                .unwrap()
        });
        assert_eq!(rows, 4, "四台不同 UA/IP 设备必须四行");

        // 设备 A 同 UA + 同来源 IP 重绑 → upsert 命中同行 id（cookie 刷新指向旧行），不新增
        let r = app
            .oneshot(pin_post(
                "203.0.113.7:51000",
                Some("mam-test.trycloudflare.com"),
                Some("ua-A"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            cookie_device_id(&r),
            id_a,
            "同指纹重绑 Set-Cookie 必须携带旧行 id"
        );
        let rows: i64 = state.store.with(|c| {
            c.query_row("SELECT COUNT(*) FROM remote_devices", [], |r| r.get(0))
                .unwrap()
        });
        assert_eq!(rows, 4, "重绑不新增行");
    }

    /// 限速矩阵（矩阵 5）：连错 4 次 → 401 且 remaining 递减（4,3,2,1）；第 5 次错 → 401
    /// （remaining 0）；第 6 次请求（即使 PIN 对）→ 429 + retryAfter=600；时钟推进过锁期
    /// （now_source 注入缝）→ 正确 PIN 配对成功
    #[tokio::test]
    async fn pin_rate_limit_locks_after_five_failures_then_expires() {
        let (state, t) = a3_state(Some("1234"), 3, &[], &[], false);
        let app = router(state.clone());
        for want in [4, 3, 2, 1] {
            let r = app
                .clone()
                .oneshot(pin_post(
                    "10.9.9.9:6000",
                    Some("192.168.1.9:9420"),
                    Some("ua-x"),
                    r#"{"pin":"0000"}"#,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 401, "第 {} 次错应 401", 5 - want);
            let body = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["error"], "invalid_pin");
            assert_eq!(v["remaining"], want, "remaining 应递减为 {want}");
        }
        // 第 5 次错 → 401（remaining 0——次数披露到此为止，此后一律 429）
        let r = app
            .clone()
            .oneshot(pin_post(
                "10.9.9.9:6000",
                Some("192.168.1.9:9420"),
                Some("ua-x"),
                r#"{"pin":"0000"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "第 5 次错仍是 401 invalid_pin");
        let body = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["remaining"], 0, "第 5 次失败后剩余 0");

        // 第 6 次：即使 PIN 正确 → 429 + retryAfter（锁内正确 PIN 也拒）
        let r = app
            .clone()
            .oneshot(pin_post(
                "10.9.9.9:6000",
                Some("192.168.1.9:9420"),
                Some("ua-x"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 429, "锁定期内正确 PIN 也必须 429");
        assert!(
            r.headers().get("set-cookie").is_none(),
            "429 不得下发任何凭证"
        );
        let body = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["retryAfter"], 600, "整锁 10 分钟 → retryAfter 600 秒");

        // 时钟推进过锁期（600_001ms）→ 正确 PIN 配对成功（配对产物落行可验证）
        t.fetch_add(600_001, std::sync::atomic::Ordering::SeqCst);
        let r = app
            .oneshot(pin_post(
                "10.9.9.9:6000",
                Some("192.168.1.9:9420"),
                Some("ua-x"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "锁定到期后正确 PIN 可配对");
        let id = cookie_device_id(&r);
        let n: i64 = state.store.with(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM remote_devices WHERE id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap()
        });
        assert_eq!(n, 1, "成功配对应落一行设备记录");
    }

    /// 上限门（矩阵 6）：满员（max=1，已配一台）→ 第二台 PIN 对也拒——沿用既有直通
    /// 上限语义（403 + {"error":"cap_full"}）；门在 PIN 正确**之后**判定（先验 PIN 再谈
    /// 名额）；腾位后同 PIN 可配
    #[tokio::test]
    async fn pin_pair_rejected_when_device_cap_full() {
        let (state, _t) = a3_state(Some("1234"), 1, &[], &[], false);
        let app = router(state.clone());
        // 第一台占满名额
        let r = app
            .clone()
            .oneshot(pin_post(
                "10.0.0.1:7000",
                Some("192.168.1.9:9420"),
                Some("ua-1"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "前置：第一台配对成功");
        let id1 = cookie_device_id(&r);
        // 第二台：PIN 正确但满员 → 403 cap_full
        let r = app
            .clone()
            .oneshot(pin_post(
                "10.0.0.2:7001",
                Some("192.168.1.9:9420"),
                Some("ua-2"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        let body = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"], "cap_full", "上限语义沿用仓库既有 cap_full 契约");
        // 腾位后同 PIN 可配
        state
            .store
            .with(|c| crate::remote::pairing::revoke_device(c, &id1).unwrap());
        let r = app
            .oneshot(pin_post(
                "10.0.0.2:7001",
                Some("192.168.1.9:9420"),
                Some("ua-2"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "腾位后同 PIN 应可配对");
    }

    /// pin_not_set（矩阵 7）：KV 空 → 401 {"error":"pin_not_set"}，**不计失败**——
    /// 连发 5 次错误 PIN 也不进入锁定（无密可对；A5 开通道时自动生成，本任务只留语义）。
    /// pin 源用可变槽位：切到 Some 后正确 PIN 立即可配，证明 5 次未计失败
    #[tokio::test]
    async fn pin_not_set_is_unauthorized_and_records_no_failure() {
        let pin_slot: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
        let slot = pin_slot.clone();
        let (state, _t) = {
            let t = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000));
            let now = t.clone();
            (
                Arc::new(RemoteState {
                    session_source: Box::new(|| crate::session::SessionsResponse {
                        sessions: vec![],
                        total_count: 0,
                        waiting_count: 0,
                    }),
                    store: crate::remote::pairing::DeviceStore::memory(),
                    // M7 Task 5：注入器缝——端点测试不触 flush 路径，用生产占位（Windows 为 Err 桩）
                    injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
                    host_source: Box::new(|| serde_json::Value::Null),
                    message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
                    path_source: Box::new(|_, _, _| (Vec::new(), false)),
                    watcher_tx: tokio::sync::broadcast::channel(64).0,
                    sse_registry: Arc::new(SseRegistry::default()),
                    max_devices_source: Box::new(|| 3),
                    pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
                    pin_source: Box::new(move || slot.lock().unwrap().clone()),
                    now_source: Box::new(move || now.load(std::sync::atomic::Ordering::SeqCst)),
                    tunnel_hosts_source: Box::new(|| Some(Vec::new())),
                    via_hosts_source: Box::new(|| None),
                }),
                t,
            )
        };
        let app = router(state);
        for _ in 0..5 {
            let r = app
                .clone()
                .oneshot(pin_post(
                    "10.8.8.8:6100",
                    Some("192.168.1.9:9420"),
                    Some("ua-y"),
                    r#"{"pin":"0000"}"#,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 401);
            let body = axum::body::to_bytes(r.into_body(), 4096).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["error"], "pin_not_set", "未设置 PIN 必须 pin_not_set");
        }
        // 切 PIN 源后正确 PIN 立即可配——若 pin_not_set 被计失败，5 次早已锁定 429
        *pin_slot.lock().unwrap() = Some("1234".to_string());
        let r = app
            .oneshot(pin_post(
                "10.8.8.8:6100",
                Some("192.168.1.9:9420"),
                Some("ua-y"),
                r#"{"pin":"1234"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "pin_not_set 不计失败——5 次后不得锁定");
    }

    /// 名单收口（矩阵 8）：旧 /pair 直通端点与审批三端点全部死亡——POST 一律 403
    /// （旧路由删除后落内层 fallback；gate 放行名单不再含 /pair*）；/pair/pin/extra
    /// 多余段同样 403（名单精确相等，不是 /pair 前缀）
    #[tokio::test]
    async fn legacy_pair_endpoints_are_closed() {
        let (state, _t) = a3_state(Some("1234"), 3, &[], &[], false);
        let app = router(state);
        for (uri, body) in [
            ("/m/api/v1/pair", r#"{"pin":"1234"}"#),
            ("/m/api/v1/pair/request", r#"{"name":"x"}"#),
            ("/m/api/v1/pair/poll", r#"{"requestId":"r"}"#),
            (
                "/m/api/v1/pair/confirm",
                r#"{"requestId":"r","code":"1234"}"#,
            ),
            ("/m/api/v1/pair/pin/extra", r#"{"pin":"1234"}"#),
        ] {
            let r = app
                .clone()
                .oneshot(http_req(
                    "POST",
                    uri,
                    "10.0.0.9:8000",
                    Some("192.168.1.9:9420"),
                    Some("ua"),
                    None,
                    Some(body),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 403, "{uri} 必须死亡（旧端点下线 + 名单收口）");
        }
    }
}
