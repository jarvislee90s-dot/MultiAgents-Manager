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

/// A1 写入确认探针缝类型（M9R Task 5，clippy type_complexity 收敛别名，对齐
/// [`ViaHostsSource`] 先例）：参数 = (tool, session_id, stamp)。
pub type ConfirmProbeFn = dyn Fn(&str, &str, &str) -> bool + Send + Sync;

/// 对话框在场探针缝类型（丁T3 §2.7，对齐 [`ConfirmProbeFn`] 先例）：
/// 参数 = (session_id, pid)；返回 `Some(选项表)` = 屏读确认**编号选项对话框在场**。
///
/// **语义与 [`crate::inject::dialog::blocks_control_injection`] 同源**：`None` 表示
/// 「无法判定或确实无对话框」（两义同收敛，能力缺失不阻断——裁决见该函数文档）。
///
/// **为什么需要这条缝**：屏读是 Windows 专有 FFI 能力（`read_screen_window` 需要真实
/// conhost 与目标 pid），端点测试进程没有可 attach 的控制台 → 真实实现在 CI 上恒 `None`
/// → 「在场即拒」这条红线**没有任何自动化证据**。缝把「屏读结果」变成可注入的输入，
/// 使两种形态都能在门禁里断言：在场（假体返回真机屏幕原文解析出的选项表）→ 拒绝且
/// 零投递零审计；不在场（假体返回 None）→ 照常投递。
///
/// 生产装配 = [`crate::inject::dialog::probe_screen_dialog`] 的同一实现（单点，
/// 见 `remote/mod.rs`）。
pub type DialogProbeFn =
    dyn Fn(&str, u32) -> Option<Vec<crate::inject::dialog::DialogOption>> + Send + Sync;

/// **可见窗口屏读**探针缝类型（丁T5）：参数 = (session_id, pid)；返回**逐行屏幕文本**
/// （`read_screen_window` 的产物形态；`None` = 读不到屏/平台无屏读能力）。
///
/// # 为什么需要第二条缝（与 [`DialogProbeFn`] 的关系）
///
/// [`DialogProbeFn`] 把「屏读 + 编号选项簇解析」的**结论**（是否在场）缝出来，够用
/// 于「在场即拒」那种**单一布尔判据**。而丁T5 的提交/自由作答阶段机要在**多次读屏
/// 之间**推进状态（提交行 → Review 屏 → 终态；定位 → 文本 → 终态），且每段的判据
/// 各不相同（各自的锚文本）——若仍只缝「结论」，端点的轮询就得为每一段复制一遍
/// 解析逻辑，那正是「同一判据两处实现」的老路。
///
/// 故本缝缝的是**能力**（读一屏）而不是结论：判据留在内核
/// （`inject::question` 的各 `probe_*`），端点只提供「怎么读」与「读的节奏」。
/// 测试用脚本化屏序列注入，则整条编排（分段、复核、中止）在门禁里可断言，
/// 而不是只有真机能覆盖。
pub type ScreenProbeFn = dyn Fn(&str, u32) -> Option<Vec<String>> + Send + Sync;

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
    /// 消费方：inject::queue::flush_one（flush 投递）+ session-send 直发（Task 6 已接线：
    /// 路由注册 / serve 挂 flush 循环 / 审计写口共用）——channel 名（审计）也取自本缝
    pub injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
    /// R5 一键 resume 终端 spawn 缝（Task 11）：生产 = inject::resume::spawn_terminal
    /// （真开窗）；测试注入记录型假 spawner（零真开窗）。消费方：session-open 端点
    /// （inject::resume::open_session_terminal_with 的 spawner 参数）
    pub resume_spawner: std::sync::Arc<crate::inject::resume::SpawnFn>,
    /// A1 写入确认缝（M9R Task 5）：参数 = (tool, session_id, stamp)。生产 =
    /// 会话消息读路径查 24 字符尾戳（与 /session-messages 数据同源；读失败 =
    /// 未命中，诚实口径）；测试恒 true（确认失败用例就地覆盖恒 false）。
    /// 消费方：inject::confirm（flush_one 直发/插队确认轮询全经本缝，queue 测试
    /// 零接触真实文件）——与 injector 缝同模式（生产装配无法捕获自身 Arc，
    /// 故闭包内直调读路径）。
    pub confirm_probe: std::sync::Arc<ConfirmProbeFn>,
    /// 对话框在场探针缝（丁T3 §2.7）：参数 = (session_id, pid)。生产 =
    /// `inject::dialog::probe_screen_dialog`（Windows 屏读可见窗口 + 编号选项簇解析；
    /// 非 Windows 恒 None）；测试注入假体（在场/不在场两形态可断言，零真实窗口）。
    /// 消费方：`remote::api::session_mode_switch` 的控制类注入守卫（见
    /// [`DialogProbeFn`] 的缝理由与 [`crate::inject::dialog::blocks_control_injection`]
    /// 的裁决）。**注意 pid 也从参数传入**：端点侧已从会话快照取到，不让假体去猜。
    pub dialog_probe: std::sync::Arc<DialogProbeFn>,
    /// **可见窗口屏读**能力缝（丁T5）：参数 = (session_id, pid)，返回逐行屏幕文本。
    /// 生产 = `inject::windows_console::read_screen_window`（Windows）；非 Windows 恒
    /// None（无屏读 → 阶段机如实中止并引导终端，见 `inject::question` 各段的中止文案）。
    /// 消费方：`remote::api::session_question_answer` 的提交/自由作答阶段机（缝的形态
    /// 理由见 [`ScreenProbeFn`] 文档）。**测试注入脚本化屏序列**——否则
    /// 「每段屏读复核」这条控制流只有实机能覆盖（本批已多次栽在这上面）。
    pub screen_probe: std::sync::Arc<ScreenProbeFn>,
    /// 敏感黑名单主目录基准注入缝（M5 P2-a 追记）：生产 = `dirs::home_dir()`；
    /// 测试注入 tempdir home（零接触真实主目录）。**端点必须消费它**——
    /// 3d22e2e 曾传 None 使 ~/.ssh 等黑名单整段失效（单元测试全绿而生产裸奔）
    pub home_source: Box<dyn Fn() -> Option<String> + Send + Sync>,
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
        // M7 Task 6：注入三端点（PIN 门禁内层 gate 结构性覆盖——新端点不需要各自
        // 鉴权代码；session-send 直发/入队 + send-info 可用性 + queue 视图/插队/撤回）
        .route("/session-send", post(api::session_send))
        .route("/session-send-info", get(api::session_send_info))
        .route("/session-queue", get(api::session_queue))
        .route("/session-queue/jump", post(api::session_queue_jump))
        .route("/session-queue/retract", post(api::session_queue_retract))
        // M8 Task 11：审批端点（红卡一键批准/拒绝——选项可用性 + 按键应答；PIN 门禁
        // 内层 gate 结构性覆盖，新端点不需要各自鉴权代码）
        .route(
            "/session-approve-options",
            get(api::session_approve_options),
        )
        .route("/session-approve", post(api::session_approve))
        // 批次乙 T8：问答端点（AskUserQuestion 问答卡——可用性/题目结构 + 应答注入
        // 序列；PIN 门禁内层 gate 结构性覆盖，新端点不需要各自鉴权代码）
        .route("/session-question", get(api::session_question))
        .route(
            "/session-question/answer",
            post(api::session_question_answer),
        )
        // 批次丙 T6：模式端点（当前档屏读 + 切档注入；PIN 门禁内层 gate 结构性
        // 覆盖，新端点不需要各自鉴权代码）
        .route("/session-mode", get(api::session_mode))
        .route("/session-mode/switch", post(api::session_mode_switch))
        // 2026-09-23 用户方案：codex 权限组的「终端菜单单选题」——open/pick 两动作
        // （POST）+ 纯屏读重同步（GET，零注入）
        .route("/session-mode/menu", post(api::session_mode_menu))
        .route("/session-mode/menu", get(api::session_mode_menu_read))
        // 2026-09-20：移动端附件上传（落盘会话工作目录 .mam-attachments/<会话>/，
        // 路径随消息内联标记注入；PIN 门禁内层 gate 结构性覆盖；20MB 显式上限——
        // axum 默认 2MB；超限时 handler 先按 Content-Length 预检给结构化 413）
        .route(
            "/session-attachment",
            post(api::session_attachment).layer(axum::extract::DefaultBodyLimit::max(
                crate::remote::attachments::MAX_ATTACHMENT_BYTES,
            )),
        )
        // M6R–M9R Task 11：一键 resume 端点（R5，PIN 门禁内层 gate 结构性覆盖，
        // 新端点不需要各自鉴权代码；spawn 缝注入使测试零真开窗）
        .route("/session-open", post(api::session_open))
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
    // M7 Task 6：注入 flush 循环接线（Task 5 交付的循环体在此合入编译）——订阅同一
    // watcher 跃迁通道（broadcast 多订阅端各自独立游标，与 SSE 消费互不影响），
    // 会话回到可输入态时逐条投递该会话的待发队列；与 session-send 直发共用
    // in-flight 守卫（同会话并发双投防护）
    crate::inject::queue::spawn_flush_loop(state.clone());
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
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
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
            home_source: Box::new(|| None),
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
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
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
            home_source: Box::new(|| None),
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
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
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
            home_source: Box::new(|| None),
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
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
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
            home_source: Box::new(|| None),
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
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
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
            home_source: Box::new(|| None),
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

        // (6) 不存在（两平台都不存在的人造路径——勿用 /etc/passwd 之类真实系统
        // 路径：Linux CI 上它真实存在，全盘放开语义下 200 是正确行为而非失败）
        // → 403 + 原因码 not_found（M5 P2-a：原因写在报错处，仅已过闸设备可见）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/file?session_id=sess_file&path=/no-such-mam-fixture/nope.txt",
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

    /// 敏感黑名单端到端回归锁（M5 P2-a 追记）：/file 端点必须**消费** home_source。
    /// 3d22e2e 曾把端点调用改传 None，使主目录内黑名单在生产链路上整段失效——
    /// 当时单元测试全绿（read_file_safe 每个用例都显式传 Some(home)，无从暴露
    /// 接线缺失），本锁补上这一层：探针 = 调用标记（照
    /// session_messages_endpoint_is_gated_and_shaped 的捕获注入先例），端点漏接
    /// home_source 时 hit 恒 false，断言直接变红。
    #[tokio::test]
    async fn file_endpoint_rejects_sensitive_paths_inside_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let proj = home.join("Desktop").join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let ok_file = proj.join("ok.txt");
        std::fs::write(&ok_file, "fine").unwrap();
        let secret = home.join(".ssh").join("id_rsa");
        std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
        std::fs::write(&secret, "PRIVATE").unwrap();
        let home_s = home.to_str().unwrap().to_string();

        let session = crate::session::Session {
            id: "sess_home".into(),
            agent_type: crate::session::AgentType::Claude,
            project_name: "proj".into(),
            project_path: proj.to_str().unwrap().to_string(),
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
        let hit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let h = hit.clone();
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            // 探针 + 注入 tmpdir home（零接触真实主目录）
            home_source: Box::new(move || {
                h.store(true, std::sync::atomic::Ordering::SeqCst);
                Some(home_s.clone())
            }),
        });
        let app = router(state.clone());
        persist_device(&state, "fe");

        // (1) 主目录内凭据 → 403 + 原因码 sensitive
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                &format!(
                    "/m/api/v1/file?session_id=sess_home&path={}",
                    uri_encode(&secret.to_string_lossy())
                ),
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert!(
            hit.load(std::sync::atomic::Ordering::SeqCst),
            "/file 必须消费 home_source（漏接 = 生产黑名单失效，3d22e2e 回归形态）"
        );
        assert_eq!(r.status(), 403, "主目录内凭据必须拒绝");
        let b = body_string(r).await;
        assert!(b.contains("sensitive"), "403 体必须带原因码 sensitive: {b}");

        // (2) 主目录内普通文件 → 200（黑名单不误伤；同时证明 (1) 不是全盘拒绝）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                &format!(
                    "/m/api/v1/file?session_id=sess_home&path={}",
                    uri_encode(&ok_file.to_string_lossy())
                ),
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "主目录内普通文件必须放行");

        // (3) **区分度断言**：主目录**外**含敏感段名的路径必须放行——
        // 这正是「基准被真正消费」的判据：若端点传 None（3d22e2e 形态），
        // read_file_safe 会走全段保守分支把 .ssh 段一并拒掉（403）；
        // 只有消费了 home 基准、且裁决边界（主目录外不设路径级防线）未被扩大，
        // 才会放行。本条同时是「T1 fail-closed 不得吞掉接线错误」的防回归锁。
        let outside_secret = tmp.path().join("outside").join(".ssh").join("id_rsa");
        std::fs::create_dir_all(outside_secret.parent().unwrap()).unwrap();
        std::fs::write(&outside_secret, "PRIVATE").unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                &format!(
                    "/m/api/v1/file?session_id=sess_home&path={}",
                    uri_encode(&outside_secret.to_string_lossy())
                ),
                Some("mam_device=fe"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "主目录外路径不设路径级防线（2026-09-18 裁决）——基准被消费时必须放行"
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
                // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
                resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
                // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
                confirm_probe: std::sync::Arc::new(|_, _, _| true),
                // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
                // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
                dialog_probe: std::sync::Arc::new(|_, _| None),
                screen_probe: std::sync::Arc::new(|_, _| None),
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
                home_source: Box::new(|| None),
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
                    // R5 一键 resume spawn 缝（Task 11）：本组测试不触 session-open，注 no-op 桩
                    resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| {
                        Ok(())
                    }),
                    // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
                    confirm_probe: std::sync::Arc::new(|_, _, _| true),
                    // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
                    // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
                    dialog_probe: std::sync::Arc::new(|_, _| None),
                    screen_probe: std::sync::Arc::new(|_, _| None),
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
                    home_source: Box::new(|| None),
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

    // ==== M7 Task 6：session-send / send-info / queue 端点（PIN 门禁内 + 注入器缝）====
    // 零污染：DB 依赖全部经 RemoteState.store = DeviceStore::memory()（Task 6 缝演进：
    // flush_one / 队列 DAO / 审计全走 st.store——生产 Global 语义不变，测试内存库）；
    // 会话快照走注入源；注入器用 FakeInjector。不触真实 ~/.mam。

    /// 注入器假体（记录 locate_and_inject 调用）；fail=Some 时恒 Err（直发失败回执用）。
    /// Task 11：补 key_calls 记录 locate_and_send_key 调用（审批按键注入路径）。
    /// F2：补 key_spec_calls 记录 locate_and_send_key_spec 收到的族规格（trait 默认实现
    /// 只透传不记录——覆写以断言「审批路径族传递」；键位经委托旧方法照旧入 key_calls，
    /// 既有键位断言不受影响，fail 语义同源）
    struct FakeInjector {
        calls: std::sync::Mutex<Vec<(u32, String)>>,
        key_calls: std::sync::Mutex<Vec<(u32, String)>>,
        key_spec_calls: std::sync::Mutex<Vec<(u32, String, crate::inject::families::TuiFamily)>>,
        fail: Option<&'static str>,
    }

    impl FakeInjector {
        fn ok() -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                key_calls: std::sync::Mutex::new(Vec::new()),
                key_spec_calls: std::sync::Mutex::new(Vec::new()),
                fail: None,
            })
        }
        fn failing(reason: &'static str) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                calls: std::sync::Mutex::new(Vec::new()),
                key_calls: std::sync::Mutex::new(Vec::new()),
                key_spec_calls: std::sync::Mutex::new(Vec::new()),
                fail: Some(reason),
            })
        }
        fn recorded(&self) -> Vec<(u32, String)> {
            self.calls.lock().unwrap().clone()
        }
        fn recorded_keys(&self) -> Vec<(u32, String)> {
            self.key_calls.lock().unwrap().clone()
        }
        fn recorded_key_specs(&self) -> Vec<(u32, String, crate::inject::families::TuiFamily)> {
            self.key_spec_calls.lock().unwrap().clone()
        }
    }

    impl crate::inject::engine::Injector for FakeInjector {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String> {
            self.calls.lock().unwrap().push((pid, text.to_string()));
            match self.fail {
                Some(e) => Err(e.to_string()),
                None => Ok(()),
            }
        }
        fn locate_and_send_key(&self, pid: u32, key: &str) -> Result<(), String> {
            self.key_calls.lock().unwrap().push((pid, key.to_string()));
            match self.fail {
                Some(e) => Err(e.to_string()),
                None => Ok(()),
            }
        }
        fn locate_and_send_key_spec(
            &self,
            pid: u32,
            key: &str,
            spec: &crate::inject::families::FamilySpec,
        ) -> Result<(), String> {
            self.key_spec_calls
                .lock()
                .unwrap()
                .push((pid, key.to_string(), spec.family));
            self.locate_and_send_key(pid, key)
        }
    }

    /// 会话夹具（字段形状对齐 file_endpoints_* 既有构造）
    fn inj_sess(
        id: &str,
        agent_type: crate::session::AgentType,
        pid: u32,
        status: crate::session::SessionStatus,
    ) -> crate::session::Session {
        crate::session::Session {
            id: id.into(),
            agent_type,
            project_name: "proj".into(),
            project_path: "/tmp/proj".into(),
            title: None,
            git_branch: None,
            github_url: None,
            status,
            last_message: None,
            last_message_role: None,
            last_activity_at: "2026-09-18T00:00:00Z".into(),
            pid,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: crate::session::ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        }
    }

    /// Task 6 专用 state：夹具与 [`inject_state_with_probe`] 同一套，确认缝缺省
    /// 恒命中（首轮即中，零延迟零等待）+ 指定注入器
    fn inject_state(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
    ) -> Arc<RemoteState> {
        inject_state_full(
            injector,
            std::sync::Arc::new(|_, _, _| true),
            std::sync::Arc::new(|_, _| None),
        )
    }

    /// 对话框在场探针可注入的 state（丁T3 模式切换守卫用例）：其余缝缺省（确认恒
    /// 命中、无对话框）。
    fn inject_state_with_dialog(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
    ) -> Arc<RemoteState> {
        inject_state_full(injector, std::sync::Arc::new(|_, _, _| true), dialog_probe)
    }

    /// inject_state 变体：confirm_probe 可注入（D7/T3 直发未确认端点测试用——
    /// probe 恒 false + Windows 屏读假 pid 无滞留草稿 → Submitted 中性回执）。
    ///
    /// Task 6 专用夹具会话（sess_a Waiting / sess_b Processing / sess_c workbuddy
    /// 黑盒 / sess_d zcode headless / sess_e Waiting 供失败回执测试与直发测试错开会话 /
    /// sess_f Processing 备用 / sess_i Waiting 独占——busy 直发测试专用 / sess_t3
    /// Waiting 独占——D7/T3 直发未确认端点测试专用）+ 指定注入器；其余缝与
    /// test_state 同口径（内存库，零接触真实 ~/.mam）。
    /// **守卫 id 立规（复检裁决，全测试集适用）**：①守卫持到测尾（或长窗口占用）的
    /// 测试必须占**全测试集唯一** id；②两个夹具不得共享同一 id 字符串——INFLIGHT
    /// 按裸 id 字符串全局占用，跨夹具撞 id 即跨夹具串键（sess_h 曾被本夹具 busy
    /// 测试与 approve_state 的 approve_sends_key 双方使用，实测 2/30 假红；本夹具侧
    /// 已改名 sess_i 让 sess_h 归 approve 族独占；sess_t3 同规——T3 端点测试 ~5s
    /// 轮询窗内守卫全程占用，撞 id 会把对方挤成 queued 假红）
    fn inject_state_with_probe(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        confirm_probe: std::sync::Arc<crate::remote::server::ConfirmProbeFn>,
    ) -> Arc<RemoteState> {
        inject_state_full(injector, confirm_probe, std::sync::Arc::new(|_, _| None))
    }

    /// 全参数建造器（丁T3 起三缝可分）：确认探针 + 对话框在场探针。
    fn inject_state_full(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        confirm_probe: std::sync::Arc<crate::remote::server::ConfirmProbeFn>,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
    ) -> Arc<RemoteState> {
        let sessions = vec![
            inj_sess(
                "sess_a",
                crate::session::AgentType::Claude,
                11,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_b",
                crate::session::AgentType::Claude,
                12,
                crate::session::SessionStatus::Processing,
            ),
            inj_sess(
                "sess_c",
                crate::session::AgentType::WorkBuddy,
                13,
                crate::session::SessionStatus::Idle,
            ),
            inj_sess(
                "sess_d",
                crate::session::AgentType::ZCode,
                14,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_e",
                crate::session::AgentType::Claude,
                15,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_f",
                crate::session::AgentType::Claude,
                16,
                crate::session::SessionStatus::Processing,
            ),
            inj_sess(
                "sess_i",
                crate::session::AgentType::Claude,
                19,
                crate::session::SessionStatus::Waiting,
            ),
            {
                // D7/T3 直发未确认端点测试独占（守卫 id 立规，见本函数 doc）
                inj_sess(
                    "sess_t3",
                    crate::session::AgentType::Claude,
                    25,
                    crate::session::SessionStatus::Waiting,
                )
            },
        ];
        Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: sessions.clone(),
                total_count: sessions.len(),
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector,
            // R5 一键 resume spawn 缝（Task 11）：本夹具不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：参数化（inject_state 缺省恒命中）
            confirm_probe,
            // 丁T3：对话框在场探针（同参数化）——缺省 None = 无法判定 ⇒ 控制类注入
            // 照常投递；「在场即拒」用例经 inject_state_with_dialog 注入假体
            dialog_probe,
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        })
    }

    /// 预置带花名的有效设备（丁T3 裁2 起 `[mobile <名>]` 是**尾**签名；审计设备名
    /// 列同源）
    fn persist_named_device(state: &Arc<RemoteState>, id: &str, name: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        state.store.with(|c| {
            crate::remote::pairing::persist_device(
                c,
                &crate::remote::pairing::NewDevice {
                    id: id.into(),
                    name: name.into(),
                    ua: format!("ua-{id}"),
                    origin_ip: format!("ip-{id}"),
                    via: String::new(),
                    paired_at: now,
                },
            )
            .unwrap();
        });
    }

    // ==== M8 Task 11：session-approve-options / session-approve 端点 ====
    // 零污染：设备表/队列/审计/KV 全走 RemoteState.store = DeviceStore::memory()；
    // 会话快照与注入器走注入缝；approve 映射 KV 经 store 缝读取（生产 Global=全局库
    // 同语义、测试 memory 自建库，缺省键回默认表），定制映射由各测试在内存库 seed。
    // 不触真实 ~/.mam。

    /// Task 11 专用 state：会话夹具与 inject_state 同一套（sess_a Waiting / **sess_t5m**
    /// Processing——原 id 叫 sess_b，但它与 `inject_state` 的 sess_b **撞了裸 id**
    /// 且本族的 `approve_endpoints_honor_wait_mark` 会真投递：INFLIGHT 按裸 id 全局
    /// 占用 → 并行跑时两条测试互抢守卫，`queue_jump_and_retract` 因此偶发假红
    /// （丁T5 的变异验证轮复现：单跑恒绿、并行 3/3 红）。按「守卫 id 立规」② 改名
    /// 独占。 / sess_d zcode Waiting 无映射 / sess_e-f 备用），但 sess_a 可携带
    /// last_message 供 detect 命中（inj_sess 夹具的 last_message 恒 None——approve_state
    /// 局部变体按需补设）；另加 sess_g（Waiting，独占 id）：in-flight 守卫按 session_id
    /// 全局占用，审批 POST 测试错开 id 防并行挤占（Task 6 夹具同规）。sess_h（Waiting，
    /// 独占 id，与 sess_a 同携命中 last_message）：approve_sends_key 独占——复检终修后
    /// sess_h 全测试集唯一归本族（inject_state 侧已改名 sess_i）。sess_j（Waiting，
    /// 独占 id）：audit_action_vocab 独占（Task 7 P3c 审计动作词表）。sess_p（Waiting
    /// claude，独占 id）：approve 忙让位回归独占（F1，守卫持到测尾）。sess_q（Waiting
    /// codex，独占 id）：审批族规格传递断言独占（F2）——p/q 为全测试集未占用字母
    /// （sess_m-sess_o 已归 open_state 族）。其余缝与
    /// inject_state 同口径（内存库，零接触真实 ~/.mam）。
    /// **守卫 id 立规（复检裁决，全测试集适用）**：①守卫持到测尾的测试必须占**全测试集
    /// 唯一** id；②两个夹具不得共享同一 id 字符串——INFLIGHT 按裸 id 字符串全局占用，
    /// 跨夹具撞 id 即跨夹具串键（详见 inject_state doc）
    fn approve_state(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        sess_a_last: Option<&str>,
    ) -> Arc<RemoteState> {
        approve_state_with_dialog(injector, sess_a_last, std::sync::Arc::new(|_, _| None))
    }

    /// 丁T3 F4-2：approve_state + 可注入对话框探针（审批侧 `read_dialog_options` 经
    /// 该缝取屏读结论——补上 T5 的 dialog 分支在门禁内的自动化证据：真实屏读需要
    /// conhost，CI 恒 None 时那两条断言只在有窗口的机器上才走得到）。
    fn approve_state_with_dialog(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        sess_a_last: Option<&str>,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
    ) -> Arc<RemoteState> {
        let sessions = vec![
            {
                let mut s = inj_sess(
                    "sess_a",
                    crate::session::AgentType::Claude,
                    11,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = sess_a_last.map(str::to_string);
                s
            },
            inj_sess(
                "sess_t5m",
                crate::session::AgentType::Claude,
                12,
                crate::session::SessionStatus::Processing,
            ),
            inj_sess(
                "sess_c",
                crate::session::AgentType::WorkBuddy,
                13,
                crate::session::SessionStatus::Idle,
            ),
            inj_sess(
                "sess_d",
                crate::session::AgentType::ZCode,
                14,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_e",
                crate::session::AgentType::Claude,
                15,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_f",
                crate::session::AgentType::Claude,
                16,
                crate::session::SessionStatus::Processing,
            ),
            inj_sess(
                "sess_g",
                crate::session::AgentType::Claude,
                17,
                crate::session::SessionStatus::Waiting,
            ),
            {
                // Important 3：approve_sends_key 独占会话——last_message 与 sess_a 同源
                // 命中串（detect 依赖），pid 独立（18）供按键注入断言
                let mut s = inj_sess(
                    "sess_h",
                    crate::session::AgentType::Claude,
                    18,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = sess_a_last.map(str::to_string);
                s
            },
            // Task 7 audit_action_vocab 独占会话（Waiting claude；全测试集唯一 id——
            // 守卫 id 立规；POST 不走 detect，无需 last_message）
            inj_sess(
                "sess_j",
                crate::session::AgentType::Claude,
                20,
                crate::session::SessionStatus::Waiting,
            ),
            {
                // M9R Task 10 probe_pending_strict_policy 独占会话（Waiting codex，
                // last_message 恒为 codex 补丁审批框标题原文——证明 detect 命中下严格档
                // 仍压为不可批）；全测试集唯一 id（守卫 id 立规）
                let mut s = inj_sess(
                    "sess_k",
                    crate::session::AgentType::Codex,
                    21,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = Some("Would you like to make the following edits?".to_string());
                s
            },
            {
                // M9R 质量评审补锁 probe_pending_hint_even_on_detect_miss 独占会话
                // （Waiting codex，last_message 与任何 marker 无关——证明 detect 未命中
                // 下严格档 hint 仍下发，短路序定格）；全测试集唯一 id（守卫 id 立规）
                let mut s = inj_sess(
                    "sess_l",
                    crate::session::AgentType::Codex,
                    22,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = Some("无关文本".to_string());
                s
            },
            {
                // F1 approve 忙让位测试独占会话（Waiting claude，全测试集唯一 id——守卫
                // id 立规：本测守卫持到测尾；sess_p 为全测试集未占用字母，open_state
                // 夹具的 sess_m/sess_n 已被 session-open 族占用，避开）；last_message
                // 与 sess_a 同源（POST 审批不消费 detect，保持夹具一致性而已）
                let mut s = inj_sess(
                    "sess_p",
                    crate::session::AgentType::Claude,
                    23,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = sess_a_last.map(str::to_string);
                s
            },
            // F2 审批族规格测试独占会话（Waiting codex，全测试集唯一 id，避让 open_state
            // 既有 sess_m-sess_o）：POST 审批只查 Waiting+映射+选项（不消费 detect），
            // last_message 留 None 即可——缺省 KV 回默认表，codex 映射（verified 0.154.0）
            // 非严格档，approve="y" 可出键
            inj_sess(
                "sess_q",
                crate::session::AgentType::Codex,
                24,
                crate::session::SessionStatus::Waiting,
            ),
        ];
        Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: sessions.clone(),
                total_count: sessions.len(),
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector,
            // R5 一键 resume spawn 缝（Task 11）：本夹具不触 session-open，注 no-op 桩
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            // A1 写入确认缝（M9R Task 5）：测试恒命中（首轮即中，零延迟零等待）
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3 F4-2：对话框探针参数化（缺省「无法判定」；dialog 分支用例经
            // approve_state_with_dialog 注入**真机屏幕原文**假体）
            dialog_probe,
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        })
    }

    /// 直发可输入态（Waiting）：200 delivered + 注入器收到 compose 产物（裁决 6 归一）
    /// + 审计 action=send result=ok channel=fake + 队列无 pending 残留
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_delivers_when_input_ready() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_a","text":"你好\n继续"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "发送回执是门禁下私有数据，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"delivered\""),
            "可输入态直发应 delivered：{body}"
        );
        // 注入器收到 (pid=11, "你好\n继续 [mobile 测试设备]")——真实换行归一为字面 \n
        //（裁决 6）+ 丁T3 裁2 签名**后置**
        assert_eq!(
            fake.recorded(),
            vec![(11u32, "你好\\n继续 [mobile 测试设备]".to_string())],
            "直发必须携带 W1 来源标记与归一正文"
        );
        // 审计：最新一条 action=send result=ok channel=fake
        //（flush_one 落账并行写的 action=flush 审计紧随其后——Task 6 最小演进：直发终态
        // 由端点另行落 send 审计，flush 落账审计保持 Task 5 原样）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "send");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].channel, "fake");
        assert_eq!(audits[0].session_id, "sess_a");
        assert_eq!(audits[0].device_name, "测试设备");
        // 队列无残留（直发行 mark_sent 退出 pending）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(
            body_string(r).await.contains("\"items\":[]"),
            "直发后队列不得有 pending 残留"
        );
    }

    /// 运行中（Processing 黄态）：200 queued + itemId/position + 审计 action=queue +
    /// 注入器不被调用 + GET session-queue 可见该项（content 为 compose 产物）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_queues_when_running() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_b","text":"排队消息"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["status"], "queued");
        let item_id = v["itemId"].as_i64().expect("queued 回执必须带 itemId");
        assert!(item_id > 0, "itemId 应为入队行 id");
        assert_eq!(v["position"], 1, "首条排队 position=1");
        assert!(
            fake.recorded().is_empty(),
            "运行中会话不得直发（等 agent 交回输入框）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "queue");
        assert_eq!(audits[0].result, "ok");
        // GET session-queue 可见该项（content = 入队时 compose 完成的最终文本）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_b",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let body = body_string(r).await;
        assert!(
            body.contains(&format!("\"id\":{item_id}"))
                && body.contains("排队消息 [mobile 测试设备]")
                && body.contains("\"position\":1")
                && body.contains("\"enqueuedAt\":"),
            "排队视图应含 id/content/enqueuedAt/position，实际 {body}"
        );
    }

    /// D6 修改重发只入队：可输入态（Waiting）+ queueOnly=true → 回执 queued（不直发），
    /// 注入器不被调用；审计 action=queue（与「运行中留队」同口径，无 send/failed 直发
    /// 审计）；条目在 GET session-queue 可见（flush 循环转闲按序自动放行——与普通
    /// 队列项同权）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_queue_only_skips_direct_delivery() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_a","text":"修改后重发","queueOnly":true}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["status"], "queued", "queueOnly=true 可输入态也不得直发");
        let item_id = v["itemId"].as_i64().expect("queued 回执必须带 itemId");
        assert_eq!(v["position"], 1, "首条排队 position=1");
        assert!(
            fake.recorded().is_empty(),
            "queueOnly=true 必须跳过直发尝试（防「文件说闲、TUI 实忙」窗口变相插队）"
        );
        // 审计：落 ⑦ 留队臂，action=queue 与「运行中留队」同口径
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "queue");
        assert_eq!(audits[0].result, "ok");
        // 条目在队列可见（不携带 queueOnly 标志，等 flush 循环自动放行）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains(&format!("\"id\":{item_id}"))
                && body.contains("修改后重发 [mobile 测试设备]"),
            "queueOnly 条目应留在队列等自动放行：{body}"
        );
    }

    /// D6 防回归对照：queueOnly 显式 false（与缺省同义）→ 可输入态直发行为不变
    /// （200 delivered + 注入器收到 compose 产物），既有调用面零漂移
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_queue_only_false_keeps_direct_delivery() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_a","text":"普通发送","queueOnly":false}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(
            body_string(r).await.contains("\"status\":\"delivered\""),
            "queueOnly=false 可输入态照旧直发"
        );
        assert_eq!(
            fake.recorded(),
            vec![(11u32, "普通发送 [mobile 测试设备]".to_string())],
            "queueOnly=false 直发行为不得漂移"
        );
    }

    // ===== 丁T3 裁2：斜杠命令裸注入 + 审计 action=slash（问题 8）=====

    /// **斜杠命令直发（可输入态）**：注入文本必须**裸**（无签名——问题 8 实锤：
    /// 前缀会毁掉 `/permissons`；后缀同样破坏命令与参数），审计 action=**slash**
    /// 且 device_name 在账（裁2：终端不留痕，溯源只此一处）。
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn slash_message_bare_injects_and_audits_slash() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_a","text":"/permissions"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(body.contains("\"status\":\"delivered\""), "{body}");
        assert_eq!(
            fake.recorded(),
            vec![(11u32, "/permissions".to_string())],
            "斜杠命令必须裸注入（不加签名——签名会破坏命令解析）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        // 端点行（action=slash）在前，flush 落账行（机制记录 action=flush）紧随其后
        assert_eq!(
            audits[0].action, "slash",
            "端点审计 action=slash（裁2 词表新增）"
        );
        assert_eq!(audits[0].result, "ok");
        assert_eq!(
            audits[0].device_name, "测试设备",
            "slash 的溯源靠设备名在账"
        );
        assert_eq!(audits[0].session_id, "sess_a");
        assert_eq!(
            audits[0].summary, "/permissions",
            "摘要即裸命令原文（无签名可读）"
        );
        // 机制行保持既有词表（投递路径判据不因 slash 消失——见 audit_action_for 注）
        assert_eq!(audits[1].action, "flush");
    }

    /// **斜杠命令走队列（运行中会话）**：同样裸注入入队（队列存的就是 compose 产物）
    /// + 端点审计 action=slash（不是 queue——用户动作是发命令，投递机制另记）。
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn slash_message_queued_keeps_bare_form_and_slash_action() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                // sess_b = Processing（留队臂）
                Some(r#"{"sessionId":"sess_b","text":"/plan"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["status"], "queued");
        assert!(fake.recorded().is_empty(), "留队臂不投递");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "slash", "留队臂的端点审计同样记 slash");
        // 队内内容 = 裸命令（flush 放行时直接照发，不会再加工）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_b",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"content\":\"/plan\""),
            "队列条目必须是裸命令（flush 直接照发）：{body}"
        );
        assert!(
            !body.contains("[mobile"),
            "斜杠命令的队列条目不得带签名：{body}"
        );
    }

    /// D6：queueOnly=true 且会话 running（Processing 黄态）→ 照常入队（与普通入队
    /// 同路径同回执同审计），注入器不被调用
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_queue_only_when_running_queues_normally() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_b","text":"运行中也入队","queueOnly":true}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["status"], "queued");
        assert_eq!(v["position"], 1, "与普通入队同回执形态");
        assert!(fake.recorded().is_empty(), "运行中本就不直发");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "queue");
        assert_eq!(audits[0].result, "ok");
    }

    /// 拒绝矩阵：workbuddy → 403 blackbox；zcode → 403 headless_only（均不入队）；
    /// 未知会话 → 404 no_session；空/全空白 text 与超长（MAX_SEND_CHARS+1）→ 400
    #[tokio::test]
    async fn send_rejects_not_injectable_and_missing() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // workbuddy 黑盒 → 403 not_injectable + reasonCode=blackbox（原因写在报错处，
        // 仅已过闸设备可见——M5 P2-a 同口径）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_c","text":"hi"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        let body = body_string(r).await;
        assert!(
            body.contains("not_injectable") && body.contains("blackbox"),
            "403 体必须带 not_injectable + reasonCode：{body}"
        );
        // zcode 走无头（M11）→ 403 headless_only
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_d","text":"hi"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        assert!(body_string(r).await.contains("headless_only"));
        // 不存在的会话 → 404 no_session（W1：定位失败不入队）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"nope","text":"hi"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_session"));
        // 空 / 全空白 text → 400 bad_request
        for payload in [
            r#"{"sessionId":"sess_a","text":""}"#,
            r#"{"sessionId":"sess_a","text":"   "}"#,
        ] {
            let r = app
                .clone()
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-send",
                    Some("mam_device=mm"),
                    Some(payload),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "{payload}");
            assert!(body_string(r).await.contains("bad_request"));
        }
        // 超长（MAX_SEND_CHARS + 1 chars）→ 400
        let payload = serde_json::json!({
            "sessionId": "sess_a",
            "text": "字".repeat(crate::remote::api::MAX_SEND_CHARS + 1),
        })
        .to_string();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(&payload),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "超长正文必须 400");
        // 全程无任何投递、无任何入队
        assert!(fake.recorded().is_empty());
        let pending = state
            .store
            .with(|c| crate::database::dao::inject_queue::pending_for_session_conn(c, "sess_c"));
        assert!(pending.is_empty(), "拒绝路径不得入队");
    }

    /// guard-busy 回归锁：直发遇 in-flight 占用 → 按 queued 回执（让位，不双投）。
    /// 守卫持到测尾——占全测试集唯一 id sess_i（复检终修：曾用 sess_h 与 approve_state
    /// 夹具的 approve_sends_key 跨夹具撞 id 串键实测 2/30 假红；立规见 inject_state doc）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_input_ready_busy_inflight_falls_back_to_queue() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let _busy = crate::inject::queue::try_acquire_inflight("sess_i").unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_i","text":"你好"}"#),
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"queued\""),
            "in-flight 占用时直发应让位排队：{body}"
        );
        assert_eq!(fake.recorded().len(), 0, "占用期间不得注入");
    }

    /// guard-busy 回归锁（F1 新语义）：jump 遇 in-flight 占用 → 200 queued{itemId,position}
    /// （**裁决变化**：旧忙时回 failed「投递进行中」提示重试，现改 queued——条目保持
    /// pending 语义即排队，让位给进行中的那次投递；移动端 handleJump 对非 delivered 走
    /// reconcileQueued 对账，queued 直认恢复排队视图，无 failed 交互依赖）。守卫持到
    /// 测尾——sess_f 为 inject_state 夹具内 jump 忙测试专用 id
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn queue_jump_busy_inflight_falls_back_to_queue() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 先入队一条（sess_f Processing）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_f","text":"第一条"}"#),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        let id = v["itemId"].as_i64().unwrap();
        let _busy = crate::inject::queue::try_acquire_inflight("sess_f").unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-queue/jump",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"sess_f","itemId":{id}}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["status"], "queued",
            "in-flight 占用时 jump 应让位排队回执 queued：{body}"
        );
        assert_eq!(v["itemId"], id, "queued 回执必须携带点名条目 id");
        assert_eq!(v["position"], 1, "唯一 pending 条目队位 1");
        assert_eq!(fake.recorded().len(), 0, "占用期间不得注入");
        // 忙让位不落审计：账内恰一条审计 = 入队时 send 端点的 queue|ok（端点契约），
        // 不得出现 jump/fail——jump 审计只出自 settle（busy 让位未执行 flush_given）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(
            audits.len(),
            1,
            "忙让位不落审计（仅入队时的 queue|ok 一条）"
        );
        assert_eq!(audits[0].action, "queue");
        assert_eq!(audits[0].result, "ok");
    }

    /// 插队 + 撤回：黄态入队两条 → jump 第二条 delivered（插队语义：黄态照发，注入器
    /// 收到第二条 compose 产物）+ 审计 action=jump → retract 第一条 ok:true → queue 空
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn queue_jump_and_retract() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 入队两条（sess_b Processing 黄态）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_b","text":"第一条"}"#),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        let id1 = v["itemId"].as_i64().unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_b","text":"第二条"}"#),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        let id2 = v["itemId"].as_i64().unwrap();
        assert_eq!(v["position"], 2, "第二条排位 2");

        // jump 第二条 → delivered（插队语义：黄态照发）
        let payload = serde_json::json!({ "sessionId": "sess_b", "itemId": id2 }).to_string();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-queue/jump",
                Some("mam_device=mm"),
                Some(&payload),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"status\":\"delivered\""));
        // 注入器收到的是插队目标（第二条）的 compose 产物
        assert_eq!(
            fake.recorded(),
            vec![(12u32, "第二条 [mobile 测试设备]".to_string())],
            "插队必须照发目标条目（运行中 TUI 把消息放进自身输入缓冲）"
        );
        // 审计 action=jump（settle 落账写入；此刻 retract 尚未发生，最新一条即 jump）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "jump");
        assert_eq!(audits[0].result, "ok");

        // retract 第一条 → ok:true
        let payload = serde_json::json!({ "sessionId": "sess_b", "itemId": id1 }).to_string();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-queue/retract",
                Some("mam_device=mm"),
                Some(&payload),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"ok\":true"));
        // 队列空（第一条被撤、第二条已发）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_b",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert!(
            body_string(r).await.contains("\"items\":[]"),
            "撤回 + 插队发完后队列应为空"
        );
    }

    /// P1-4 端点精确映射（jump Suspended 分支）：点名快照中不存在的会话的 pending 项 →
    /// flush_given 挂起（红·中断不投递，W2）→ 200 queued + itemId/position（行保持
    /// pending 等会话回来，由 flush 循环/对账接力——Suspended 亦按排队回执）
    #[tokio::test]
    async fn queue_jump_suspended_returns_queued() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 直接入队一个快照外会话的 pending 项（session-send 对快照外会话 404 no_session
        // 拦在入队前，故走 DAO 缝构造——同一内存库，零接触真实 ~/.mam）
        let ghost_id = state.store.with(|c| {
            crate::database::dao::inject_queue::enqueue_conn(
                c,
                "sess_ghost",
                "claude",
                "mm",
                "测试设备",
                "幽灵消息",
                1000,
            )
        });
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-queue/jump",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"sess_ghost","itemId":{ghost_id}}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["status"], "queued",
            "Suspended 必须按 queued 回执（行保持 pending，不谎报 delivered）"
        );
        assert_eq!(v["itemId"], ghost_id, "queued 回执携带点名条目 id");
        assert_eq!(
            v["position"], 1,
            "唯一 pending 项 position=1（回查实时队列）"
        );
        assert!(
            fake.recorded().is_empty(),
            "会话消失不得注入（pid 无从定位）"
        );
        // 行保持 pending：GET session-queue 仍可见该项
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-queue?session_id=sess_ghost",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert!(
            body_string(r).await.contains("幽灵消息"),
            "挂起行必须保持 pending（等会话回来由 flush 循环/对账接力）"
        );
    }

    /// P2-6 撤回与投递共守卫：该会话 in-flight 占用时 retract → 200 failed +
    /// 「投递进行中，请稍后重试」短回执。守卫必须取在 pending 前查**之前**
    /// （照 jump 先例）——本测用无 pending 行的会话：若守卫位置错误（后查），响应会是
    /// 404 not_found 而非 failed，测试即红
    #[tokio::test]
    async fn queue_retract_busy_inflight_returns_failed() {
        let fake = FakeInjector::ok();
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 独占会话 id（sess_r 不在夹具快照、无 pending 行）——in-flight 守卫按
        // session_id 全局占用，错开防止与并行测试互相挤占
        let _busy = crate::inject::queue::try_acquire_inflight("sess_r").unwrap();
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-queue/retract",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_r","itemId":1}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("投递进行中，请稍后重试"),
            "in-flight 占用时撤回应 200 failed 短回执：{body}"
        );
        assert_eq!(fake.recorded().len(), 0, "占用期间不得注入");
    }

    /// 直发注入失败：注入器恒 Err → 200 {"status":"failed","error":…}（W4 可重试回执）
    /// + 审计 action=send result=failed:… + 队列无残留（W1：失败行 mark_failed 退出 pending）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_reports_inject_failure() {
        let fake = FakeInjector::failing("定位终端失败：pid 不存在");
        let state = inject_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_e","text":"失败回执"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "注入失败以 200 failed 回执表达（非 5xx）");
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("定位终端失败：pid 不存在"),
            "失败回执必须携带注入器错误原文：{body}"
        );
        assert_eq!(
            fake.recorded().len(),
            1,
            "失败发生在投递阶段（注入器已被调用）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "send");
        assert!(
            audits[0].result.starts_with("failed:"),
            "失败审计 result=failed:<原因>，实际 {}",
            audits[0].result
        );
        let pending = state
            .store
            .with(|c| crate::database::dao::inject_queue::pending_for_session_conn(c, "sess_e"));
        assert!(
            pending.is_empty(),
            "失败行必须退出 pending（W1：定位失败不入队重试）"
        );
    }

    /// D7/T3 端点分诊（Windows）：注入 Ok + 戳未中 + 屏读（假 pid）无滞留草稿 →
    /// 200 {"status":"submitted"}（中性回执，不冒充 delivered 也不冒充 failed——
    /// 验收问题 #5：failed 文案会诱导重试 = 双发）+ 审计 action=send
    /// result=unconfirmed 与确认送达 ok 区分（flush_one 落账并行写的 flush 审计
    /// 同为 unconfirmed）+ 行 mark_sent 退出 pending（队列无残留，flush 循环不
    /// 重投 = 防双发）。probe 恒 false 时直发确认走满族规格轮询窗（claude 快族
    /// 5s，端点无超时缝），本例为全测试集唯一 5s 级用例（申报：套件「无 5s 级
    /// 慢测」纪律的已知例外，见 T3 报告）；会话用全测试集唯一 id sess_t3——
    /// ~5s 轮询期间 in-flight 守卫全程占用（守卫按裸 id 全局串键，守卫 id 立规）
    #[cfg(windows)]
    #[tokio::test]
    async fn send_reports_submitted_when_stamp_missed_no_stuck_draft() {
        let fake = FakeInjector::ok();
        let state = inject_state_with_probe(fake.clone(), std::sync::Arc::new(|_, _, _| false));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-send",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t3","text":"中性回执"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "submitted 回执同为门禁下私有数据，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"submitted\""),
            "未确认但已投递 → 中性 submitted 回执：{body}"
        );
        assert_eq!(
            fake.recorded().len(),
            1,
            "前提自证：注入确实发生（分诊发生在注入成功之后）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(
            audits.len(),
            2,
            "端点 send 终态审计 + flush_one 落账 flush 审计"
        );
        assert_eq!(audits[0].action, "send");
        assert_eq!(
            audits[0].result, "unconfirmed",
            "端点 send 审计单列 unconfirmed（不冒充 ok 也不冒充 failed:e）"
        );
        assert_eq!(audits[1].action, "flush");
        assert_eq!(audits[1].result, "unconfirmed");
        let pending = state
            .store
            .with(|c| crate::database::dao::inject_queue::pending_for_session_conn(c, "sess_t3"));
        assert!(
            pending.is_empty(),
            "Submitted 行已 mark_sent 消费，队列无残留（flush 循环不重投 = 防双发）"
        );
    }

    /// send-info 可用性矩阵：可注入会话 → injectable=true + channels/visibility
    /// （channels 随本机平台——routing platform = std::env::consts::OS，macOS 三通道 /
    /// windows 单通道，断言按编译平台取期望）；workbuddy → injectable=false + blackbox；
    /// 另锁定缺参 400 与未知会话 404 no_session
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn send_info_matrix() {
        let state = inject_state(FakeInjector::ok());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-send-info?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["injectable"], true);
        let channels: Vec<&str> = v["channels"]
            .as_array()
            .expect("channels 应为数组")
            .iter()
            .map(|c| c.as_str().expect("channel 应为字符串"))
            .collect();
        let want: &[&str] = if cfg!(target_os = "macos") {
            &["tmux", "iterm2", "terminal_app"]
        } else if cfg!(windows) {
            &["windows_console"]
        } else {
            &[]
        };
        assert_eq!(
            channels, want,
            "channels 应为 wire 小写字符串数组（本机平台）"
        );
        assert_eq!(v["visibility"], "realtime");
        // workbuddy 黑盒 → injectable=false + reasonCode=blackbox + reason 文案
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-send-info?session_id=sess_c",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["injectable"], false);
        assert_eq!(v["reasonCode"], "blackbox");
        assert!(v["reason"].is_string(), "不可注入必须带 reason 文案");
        // 缺参 → 400；未知会话 → 404 no_session
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-send-info",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-send-info?session_id=nope",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_session"));
    }

    /// 审批默认表 marker 命中句（DEFAULT_MAPPINGS_JSON claude.prompt_markers 含
    /// "do you want to proceed"——M9R 评审 F3/D4 收紧后句式）
    const APPROVE_HIT_MSG: &str = "Do you want to proceed?";

    /// 审批选项（可批）：sess_a Waiting + last_message 命中 → 200 available=true +
    /// options 恰为 允许/拒绝 两项（**无 key 字段**——键位不外泄给 UI）+
    /// reason=null（非严格档不可批形态不下发降级文案，前端按自隐处理）+
    /// verifiedWith=2.1.251（M8R Task 10 取证回填；drift 按 verified/current 同源重算，
    /// 本机 claude 探测命中同版则 false）
    #[tokio::test]
    async fn approve_options_available() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 无 cookie → 403（nest 内层 gate 结构性覆盖新端点）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_a",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "审批选项端点必须过 gate");
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_a",
                Some("mam_device=mm"),
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
            "审批可用性是门禁下私有数据，禁止中间层缓存"
        );
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true);
        let options = v["options"].as_array().expect("options 应为数组");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0]["id"], "approve");
        assert_eq!(options[0]["label"], "允许");
        assert_eq!(options[1]["id"], "reject");
        assert_eq!(options[1]["label"], "拒绝");
        for o in options {
            assert!(
                o.get("key").is_none(),
                "选项载荷不得携带 key（键位不外泄给 UI）：{o}"
            );
        }
        // Task 13 取证回填后：claude verified_with = "2.1.251"（Windows 本机实测）。
        // currentVersion 来自真实 spawn 探测（机器相关：灰1 后裸名直 spawn 失败回退
        // cmd /c 垫片，再失败才 null → "unknown"）——drift 断言按同源纯函数重算期望，
        // 保持 hermetic；codex 保持 probe-pending 的恒漂移断言见 inject::approve::tests
        let current = v["currentVersion"].as_str();
        let expect_drift =
            crate::inject::approve::is_version_drift("2.1.251", current.unwrap_or("unknown"));
        assert_eq!(v["verifiedWith"], "2.1.251");
        assert_eq!(v["drift"], expect_drift, "drift 与 verified/current 一致");
        assert!(
            fake.recorded_keys().is_empty(),
            "查询选项端点不得触发任何按键注入"
        );
    }

    /// 审批选项（不可批）：非 Waiting（sess_t5m Processing）→ available=false；
    /// Waiting 但 last_message 与 marker 无关 → available=false；last_message=None
    /// （sess_e 夹具原样）→ available=false（None 不命中）
    #[tokio::test]
    async fn approve_options_not_waiting_or_no_hit() {
        let state_a_hit = approve_state(FakeInjector::ok(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state_a_hit, "mm", "测试设备");
        let app = router(state_a_hit);
        // sess_t5m Processing → available=false（Waiting 判定先于映射解析）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_t5m",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "非 Waiting 不得可批");
        assert!(v["options"].as_array().unwrap().is_empty());
        // sess_a Waiting 但 last_message 与任何 marker 无关 → available=false
        let state_a_miss = approve_state(FakeInjector::ok(), Some("无关"));
        persist_named_device(&state_a_miss, "mm", "测试设备");
        let app_miss = router(state_a_miss);
        let r = app_miss
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "marker 未命中不得可批");
        assert!(v["options"].as_array().unwrap().is_empty());
        // last_message=None（sess_e）→ available=false（detect 对 None 不命中）
        let r = app_miss
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_e",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "无 last_message 不得可批");
    }

    /// 审批选项（无映射）：sess_d zcode（Waiting）工具无映射 → available=false
    #[tokio::test]
    async fn approve_options_no_mapping() {
        let state = approve_state(FakeInjector::ok(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_d",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "工具无映射不得可批");
        assert!(v["options"].as_array().unwrap().is_empty());
    }

    /// T4 红卡接铃铛：等待标记路径——sess_t5m（Processing claude、无 last_message，
    /// 旧判定下必 available=false）seed 审批等待标记 → GET available=true 且选项齐
    /// （键位零泄漏）；POST approve 越过 409 not_waiting 直达键位分发（键位 "1"）。
    /// 标记经 store.with 播种（内存库，零接触真实 ~/.mam）；state 实例按测试隔离
    #[tokio::test]
    async fn approve_endpoints_honor_wait_mark() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::approval_wait::mark(conn, "claude", "sess_t5m", 1_000, "测试标记")
        });
        let app = router(state.clone());
        // GET：Processing + 标记 → available=true（跳过 detect）+ 键位零泄漏
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_t5m",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true, "标记存在即 available（跳过 detect）");
        let options = v["options"].as_array().expect("标记路径应出全选项");
        assert_eq!(options.len(), 2, "标记路径跳过 marker detect 直接出全选项");
        for o in options {
            assert!(o.get("key").is_none(), "键位不外泄");
        }
        // POST：Processing + 标记 → 越过 409 not_waiting，键位照常分发
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5m","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "标记路径越过 409 not_waiting");
        assert!(
            fake.recorded_keys().iter().any(|k| k.1 == "1"),
            "批准键位应经注入器分发：{:?}",
            fake.recorded_keys()
        );
    }

    /// T4 回归锁：无标记 + 非 Waiting → 409 not_waiting 守卫保持（标记不扩大放行面）
    #[tokio::test]
    async fn approve_without_mark_still_409_on_processing() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5m","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "无标记 Processing 仍 409 not_waiting");
    }

    /// 审批应答（批准）：sess_h optionId=approve → 200 key_sent + FakeInjector 收到
    /// (pid=18, "1")（**无 [mobile] 前缀**——按键非文本）+ 审计 action=approve result=ok。
    /// 独占会话 sess_h——契约行为不变（按键映射/无前缀/审计）；sess_h 全测试集唯一归
    /// 本族（inject_state 侧 busy 测试已改用 sess_i，复检终修，立规见两夹具 doc）
    #[tokio::test]
    async fn approve_sends_key() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_h","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "审批回执是门禁下私有数据，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\""),
            "批准应回执 key_sent：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(18u32, "1".to_string())],
            "按键注入必须带映射键位且无 [mobile] 前缀"
        );
        assert!(fake.recorded().is_empty(), "审批走按键通道，不走文本注入");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "approve");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].channel, "fake");
        assert_eq!(audits[0].session_id, "sess_h");
    }

    /// 审批应答（拒绝）：optionId=reject → 200 key_sent + FakeInjector 收到 (pid=17, "esc")
    /// + 审计 action=reject。用独占会话 sess_g：in-flight 守卫按 session_id 全局占用，
    /// 与 approve_sends_key（sess_a，契约指定）错开，防并行测试互抢守卫（Task 6 夹具同规）
    #[tokio::test]
    async fn approve_reject_audits_reject() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_g","optionId":"reject"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\""),
            "拒绝应回执 key_sent：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(17u32, "esc".to_string())],
            "拒绝键位 esc 必须来自映射表"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "reject", "拒绝审计 action=reject");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].session_id, "sess_g");
    }

    /// 审防 guard 矩阵：非法 optionId → 404 no_mapping；非 Waiting（sess_b）→ 409
    /// not_waiting；不存在会话 → 404 no_session；缺参 → 400 bad_request
    #[tokio::test]
    async fn approve_guards() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 非法 optionId（映射表中无此项）→ 404 no_mapping（降级提示走普通发送）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_a","optionId":"bogus"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_mapping"));
        // 非 Waiting（sess_t5m Processing）→ 409 not_waiting
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5m","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("not_waiting"));
        // 不存在会话 → 404 no_session
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"nope","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_session"));
        // 缺参（空 sessionId / 空 optionId）→ 400 bad_request
        for payload in [
            r#"{"sessionId":"","optionId":"approve"}"#,
            r#"{"sessionId":"sess_a","optionId":"  "}"#,
        ] {
            let r = app
                .clone()
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-approve",
                    Some("mam_device=mm"),
                    Some(payload),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "{payload}");
            assert!(body_string(r).await.contains("bad_request"));
        }
        // 全程无任何投递
        assert!(fake.recorded_keys().is_empty());
    }

    /// guard-busy 回归锁（F1）：approve 遇 in-flight 占用 → 200 failed{「投递进行中，
    /// 请稍后重试」} 且零按键出手。守卫取在 spawn_blocking 闭包内（断连双投洞封闭）——
    /// 手动占位模拟「detached 旧投递进行中」，新触发必须让位。审计口径：忙让位无投递
    /// 发生不落审计（与 retract/jump 忙让位同口径）。独占会话 sess_p（守卫持到测尾，
    /// 全测试集唯一 id 立规）
    #[tokio::test]
    async fn approve_busy_inflight_returns_failed_without_key() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let _busy = crate::inject::queue::try_acquire_inflight("sess_p").unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_p","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("投递进行中，请稍后重试"),
            "in-flight 占用时 approve 应 200 failed 让位：{body}"
        );
        assert!(
            fake.recorded_keys().is_empty() && fake.recorded_key_specs().is_empty(),
            "占用期间不得出手任何按键（零注入）"
        );
        // 忙让位不落审计（无投递发生——retract/jump 忙让位同口径）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "忙让位不落审计");
    }

    /// F2 审批族规格（服务端断言「审批路径族传递」）：codex 会话审批按键必须携带 B 族
    /// 规格（TuiFamily::Crossterm）送达注入器——构造层 key_records_for_family_dispatch
    /// （windows_console.rs，Task 3）已断言 codex 方向键按族走 VK 形态（vk=0x26），本测
    /// 锁端点侧族参数真实下传：缺族时 KV 自定义方向键会按 A 族默认形态（vk=0 字符流）
    /// 被 crossterm 静默吞。FakeInjector 覆写 locate_and_send_key_spec 记录收到的族；
    /// 键位经委托照旧入 key_calls（无 [mobile] 前缀契约一并回归）。独占会话 sess_q
    /// （默认表 codex 映射 verified 0.154.0 非严格档，approve="y"）
    #[tokio::test]
    async fn approve_passes_family_spec_to_injector() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_q","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\""),
            "codex 审批应回执 key_sent：{body}"
        );
        assert_eq!(
            fake.recorded_key_specs(),
            vec![(
                24u32,
                "y".to_string(),
                crate::inject::families::TuiFamily::Crossterm
            )],
            "审批按键必须携带 codex 的 B 族（Crossterm）规格送达注入器"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(24u32, "y".to_string())],
            "键位经 spec 覆写委托照旧记录（无 [mobile] 前缀）"
        );
        assert!(fake.recorded().is_empty(), "审批走按键通道，不走文本注入");
    }

    // ==== 批次乙 T8：session-question / session-question/answer 端点 ====
    // 零污染：标记（question/approval）/审计/KV 全走 RemoteState.store = memory()；
    // 会话快照与注入器走注入缝；通道 B 消息走 message_source 注入缝。不触真实 ~/.mam。
    // **守卫 id 立规（approve_state 同款）**：每个 POST 用例独占会话 id（sess_u..
    // sess_aj 为全测试集未占用段——approve/inject/open 三族夹具已占 sess_a..sess_q /
    // sess_i / sess_t3 / sess_att）。

    /// 探测档案 §3 单选真实夹具（缩录；questions JSON 原样）
    const Q_SINGLE_PAYLOAD: &str = r#"{"questions":[{"header":"Next step","multiSelect":false,"options":[{"description":"Explain how AskUserQuestion works.","label":"Tool demo"},{"description":"Start a coding or file task in this directory.","label":"Start a task"},{"description":"You have no further request for now.","label":"Nothing yet"}],"question":"This is a demo question — what would you like to do next?"}]}"#;

    /// 探测档案 §3 多选真实夹具（缩录）
    const Q_MULTI_PAYLOAD: &str = r#"{"questions":[{"header":"Favorite fruits","multiSelect":true,"options":[{"description":"A sweet, crisp fruit.","label":"Apple"},{"description":"A soft, tropical fruit.","label":"Banana"},{"description":"A juicy summer fruit.","label":"Peach"}],"question":"Which fruits are your favorites? (Select all that apply)"}]}"#;

    /// 多问题数组夹具（questions.length=2——只读形态，注入面由端点拒绝）
    const Q_TWO_QUESTIONS_PAYLOAD: &str = r#"{"questions":[{"header":"A","question":"First?","options":[{"label":"a1"},{"label":"a2"}]},{"header":"B","question":"Second?","options":[{"label":"b1"},{"label":"b2"}]}]}"#;

    /// 丁T2：kimi 映射表条目在默认表里的取证版本号（`kimi --version` 本机实测
    /// 2.0.2；该版本正是 R1-1 探测与 wire 形态取证的版本）——断言用，防默认表被误改
    const KIMI_NEVER_TESTED: &str = "2.0.2";

    /// 丁T2：kimi 问答形态载荷（本机 wire 实测缩录：`interaction.request(kind=question)`
    /// 的 request.questions 与 AUQ tool.call args 同形——后者才是消息流里的载体）
    const KIMI_Q_PAYLOAD: &str = r#"{"questions":[{"question":"Which output folder should the build use?","header":"Output dir","options":[{"label":"dist","description":"d"},{"label":"out","description":"o"}],"multiSelect":false}]}"#;

    /// 丁T2：kimi **多问题**载荷（本机 wire 实录 3 题缩录为 2 题——多题只读锁的夹具；
    /// kimi 的 answers map 按**题干文本**键控，故题干是稳定回读点）
    const KIMI_TWO_Q_PAYLOAD: &str = r#"{"questions":[{"question":"是否确认执行？","header":"清空确认","options":[{"label":"确认","description":"删除后重下"},{"label":"不清空","description":"直接下载"}]},{"question":"日期边界如何理解？","header":"日期边界","options":[{"label":"≥2026-07-01","description":"含7月"},{"label":"仅8月","description":"不含7月"}],"multiSelect":false}]}"#;

    /// 通道 B 的 AUQ tool-call 消息条目（content.rs SessionMessage 直构）
    fn auq_tool_call(seq: i64, args: &str) -> crate::remote::content::SessionMessage {
        crate::remote::content::SessionMessage {
            seq,
            role: "assistant".into(),
            kind: "tool-call".into(),
            content: "AskUserQuestion".into(),
            ts: None,
            tool_name: Some("AskUserQuestion".into()),
            tool_args: Some(args.into()),
            collapsed: true,
        }
    }

    /// 通道 B 的 tool-result 消息条目（答完判据的反例形态）
    fn tool_result_msg(seq: i64, content: &str) -> crate::remote::content::SessionMessage {
        crate::remote::content::SessionMessage {
            seq,
            role: "assistant".into(),
            kind: "tool-result".into(),
            content: content.into(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: true,
        }
    }

    fn user_msg(seq: i64) -> crate::remote::content::SessionMessage {
        crate::remote::content::SessionMessage {
            seq,
            role: "user".into(),
            kind: "user".into(),
            content: "继续".into(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: false,
        }
    }

    /// T8 专用 state：会话夹具 + 问题/审批标记由各测试经 store.with 播种（内存库）。
    /// 全部 claude；id 语义见各测试。通道 B 缺省为 Err 桩（不触消息源）。
    fn question_state(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
    ) -> Arc<RemoteState> {
        question_state_with_msgs(
            injector,
            Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
        )
    }

    /// question_state 变体：message_source 可注入（通道 B 用例）。
    /// 会话清单：sess_u（Waiting，标记夹具 GET）/ sess_v-w-x-y（POST select/toggle/
    /// submit/cancel 各自独占）/ sess_z（busy 独占）/ sess_aa（多问题 409）/
    /// sess_ab（标记载荷损坏 → 回落通道 B）/ sess_ac（bad_index 400）/
    /// sess_ad（仅审批标记——隔离反差用）/ sess_ae（问题标记 + detect 命中文案——
    /// 问题标记压审批卡的最强隔离形态）/ sess_af（无标记——no_question 409）/
    /// sess_ai / sess_aj（Processing 无标记——通道 B 可用/已答反例）/
    /// sess_ak（**阶段机中途投递失败**：首错即停 + `failed:<e>` 审计独占——原为
    /// 批次乙「submit 首错即停」用例的夹具，本批重构时该用例被误删（复评 Important-2），
    /// 丁T5 复评已补回同语义覆盖并继续用本夹具）
    fn question_state_with_msgs(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        message_source: Box<crate::remote::content::MessageSourceFn>,
    ) -> Arc<RemoteState> {
        question_state_full(injector, message_source, std::sync::Arc::new(|_, _| None))
    }

    /// question_state 第三变体（丁T5）：`screen_probe` 缝可注入（阶段机用例的脚本化
    /// 屏序列）。其余与会话清单同 [`question_state_with_msgs`]。
    fn question_state_full(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        message_source: Box<crate::remote::content::MessageSourceFn>,
        screen_probe: std::sync::Arc<crate::remote::server::ScreenProbeFn>,
    ) -> Arc<RemoteState> {
        let sess = |id: &str, pid: u32, status: crate::session::SessionStatus| {
            inj_sess(id, crate::session::AgentType::Claude, pid, status)
        };
        let sessions = vec![
            sess("sess_u", 31, crate::session::SessionStatus::Waiting),
            sess("sess_v", 32, crate::session::SessionStatus::Waiting),
            sess("sess_w", 33, crate::session::SessionStatus::Waiting),
            sess("sess_x", 34, crate::session::SessionStatus::Waiting),
            sess("sess_y", 35, crate::session::SessionStatus::Waiting),
            sess("sess_z", 36, crate::session::SessionStatus::Waiting),
            sess("sess_aa", 37, crate::session::SessionStatus::Waiting),
            sess("sess_ab", 38, crate::session::SessionStatus::Waiting),
            sess("sess_ac", 39, crate::session::SessionStatus::Waiting),
            sess("sess_ad", 40, crate::session::SessionStatus::Waiting),
            {
                // 最强隔离形态：问题标记 + last_message 恰为审批 marker 命中句——
                // 证明问题标记压审批卡不依赖 detect 未达
                let mut s = sess("sess_ae", 41, crate::session::SessionStatus::Waiting);
                s.last_message = Some(APPROVE_HIT_MSG.to_string());
                s
            },
            sess("sess_af", 42, crate::session::SessionStatus::Waiting),
            sess("sess_ai", 44, crate::session::SessionStatus::Processing),
            sess("sess_aj", 45, crate::session::SessionStatus::Processing),
            // 复评 Minor 1：submit 首错即停 + failed 审计独占会话（守卫 id 立规）
            sess("sess_ak", 46, crate::session::SessionStatus::Waiting),
            // 丁T1 回归锁独占会话（全测试集唯一 id，守卫 id 立规）：问题待决的**语义红**
            // ——状态链（codex request_user_input 配对 / opencode question 部件）推出
            // Waiting，但**没有**任何等待标记（这是本批新引入的形态）。last_message 为
            // 真实问答文本（2026-09-21 rollout 实录形态），**不含**任何审批 marker——
            // 锁「审批卡不得借红灯误出」
            {
                let mut s = inj_sess(
                    "sess_al",
                    crate::session::AgentType::Codex,
                    47,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = Some("构建产物放在哪个目录？".to_string());
                s
            },
            {
                let mut s = inj_sess(
                    "sess_am",
                    crate::session::AgentType::OpenCode,
                    48,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = Some("Which folder should hold build output?".to_string());
                s
            },
            // 丁T1 复评 F-2 独占会话（全测试集唯一 id）：**最强对照形态**——问题待决
            // 的语义红 + last_message **恰为审批 marker 命中句**（模型把问题写成审批
            // 措辞的自然形态）。这正是 F-2 要拦的场景：detect 纯文本命中会误出审批卡，
            // 唯一的拦截来自「尾部存在待决问答」判定（question_pending_red_* 用例）
            {
                let mut s = inj_sess(
                    "sess_an",
                    crate::session::AgentType::Codex,
                    49,
                    crate::session::SessionStatus::Waiting,
                );
                // codex 默认映射 marker 之一（DEFAULT_MAPPINGS_JSON）
                s.last_message = Some("Would you like to run the following command?".to_string());
                s
            },
            {
                let mut s = inj_sess(
                    "sess_ao",
                    crate::session::AgentType::OpenCode,
                    50,
                    crate::session::SessionStatus::Waiting,
                );
                s.last_message = Some("Would you like to run the following command?".to_string());
                s
            },
            // 丁T1 复评 F-3 独占会话（全测试集唯一 id）：走**真实 opencode reader**
            // （tempdir 内的 opencode.db）验证销卡信号——sess_ap 待决（应 available=true）、
            // sess_aq 已答（completed，应 available=false）。**必须是 OpenCode 类型**：
            // message_source 按 tool_id 派发到 opencode reader
            inj_sess(
                "sess_ap",
                crate::session::AgentType::OpenCode,
                51,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_aq",
                crate::session::AgentType::OpenCode,
                52,
                crate::session::SessionStatus::Waiting,
            ),
            // 丁T1 复评 F2-2 独占会话（全测试集唯一 id）：**claude 假阳性反锁**——
            // 尾部有「AUQ 形态 tool-call 无 result」（评审实测的 claude 真实形态：
            // 0c41365d-… 的 AUQ 被纯文本作答、tool_result 永不落盘）+ last_message
            // 命中 claude 审批 marker。claude 有钩子问答标记通道，故**不叠加**尾部
            // 判据 → 审批必须照常可用（不被静默压制）
            {
                let mut s = inj_sess(
                    "sess_ar",
                    crate::session::AgentType::Claude,
                    53,
                    crate::session::SessionStatus::Waiting,
                );
                // claude 默认映射 marker 之一（DEFAULT_MAPPINGS_JSON）
                s.last_message = Some("Do you want to proceed?".to_string());
                s
            },
            // ===== 丁T2 计划双卡族（问题 3/4）：sess_as..sess_ax =====
            // 全测试集唯一 id（守卫 id 立规）。**kimi** 四例（sess_au..sess_ax）与
            // **codex** 两例（sess_as/sess_at）：计划预期态是 codex/kimi 的计划确认
            // 类对话框专属门（见 remote/api.rs 的 plan_dialog_family）。
            //
            // sess_as：codex **Processing**（实机计划提案后 codex 不落 Waiting）+
            // 尾部计划提案 → 门放宽到预期态（问题 4 主用例）
            inj_sess(
                "sess_as",
                crate::session::AgentType::Codex,
                54,
                crate::session::SessionStatus::Processing,
            ),
            // sess_at：codex 同形但计划之后已有用户消息 → 预期态清除（反向锁）
            inj_sess(
                "sess_at",
                crate::session::AgentType::Codex,
                55,
                crate::session::SessionStatus::Processing,
            ),
            // sess_au：kimi 计划审批（Waiting——wire interaction.request 的红灯）→
            // 审批卡主用例；sess_av：kimi 计划后已有工具事件 → 预期态清除
            inj_sess(
                "sess_au",
                crate::session::AgentType::Kimi,
                56,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_av",
                crate::session::AgentType::Kimi,
                57,
                crate::session::SessionStatus::Processing,
            ),
            // sess_aw：kimi 审批在场（尾部计划提案）→ 问答卡必须不可用（互斥主用例）
            // sess_ax：kimi 计划后已有工具事件（审批窗口关闭）→ 问答必须照常可用（反向锁）
            inj_sess(
                "sess_aw",
                crate::session::AgentType::Kimi,
                58,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_ax",
                crate::session::AgentType::Kimi,
                59,
                crate::session::SessionStatus::Waiting,
            ),
            // 丁T2 多题只读扩面（sess_ay/sess_az）：codex 与 kimi 的多问题待决
            // （message_source 按 sid 分派，见 multi_question_readonly_covers_*）
            inj_sess(
                "sess_ay",
                crate::session::AgentType::Codex,
                60,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_az",
                crate::session::AgentType::Kimi,
                61,
                crate::session::SessionStatus::Waiting,
            ),
            // 丁T2 kimi 降级态（sess_ba）：Waiting + 审批标记但**无计划预期态**
            // （带标记的 Write/command 审批）→ available=false + 提示条（无键可发）
            inj_sess(
                "sess_ba",
                crate::session::AgentType::Kimi,
                62,
                crate::session::SessionStatus::Waiting,
            ),
            // ===== 丁T2 复评修复族（F3-2/F3-3/F3-5）：sess_bb..sess_bf =====
            // 全测试集唯一 id（守卫 id 立规）。
            // sess_bb：codex **Idle**（真机计划提案后的状态）+ 尾部计划提案 → POST 门
            //   必须与 GET 同口径放行（F3-2 主用例）
            inj_sess(
                "sess_bb",
                crate::session::AgentType::Codex,
                63,
                crate::session::SessionStatus::Idle,
            ),
            // sess_bc：codex Idle + 尾部**非**计划（用户消息在前）→ POST 必须 409
            //   （反向锁：门放宽不得变成「Idle 即放行」）
            inj_sess(
                "sess_bc",
                crate::session::AgentType::Codex,
                64,
                crate::session::SessionStatus::Idle,
            ),
            // sess_bd：claude Waiting + 尾部计划（ExitPlanMode 升格产物）→ 审批卡必须
            //   带 plan 正文（F3-3 回归锁——ed1a868 起静默失效的修复）
            inj_sess(
                "sess_bd",
                crate::session::AgentType::Claude,
                65,
                crate::session::SessionStatus::Waiting,
            ),
            // sess_be：kimi Waiting + 尾部计划 → 审批卡 `plan` 必须 **null**（F3-4 收口）
            inj_sess(
                "sess_be",
                crate::session::AgentType::Kimi,
                66,
                crate::session::SessionStatus::Waiting,
            ),
            // sess_bf：kimi Waiting + 尾部**只有孤立 plan-file**（无正文卡）→ 问答卡
            //   必须**仍可用**（F3-5 反锁：孤立文件卡不得压掉问答卡）
            inj_sess(
                "sess_bf",
                crate::session::AgentType::Kimi,
                67,
                crate::session::SessionStatus::Waiting,
            ),
            // sess_bg：codex Idle + 尾部计划提案 —— **N1 用例独占**（守卫 id 立规：
            //   N1 的 POST 遍历会成功投递吗？不会——预期态一律 404，不出手；但为
            //   避免与 F3-2 用例共享 id 时 in-flight 守卫串键，仍占唯一 id）
            inj_sess(
                "sess_bg",
                crate::session::AgentType::Codex,
                68,
                crate::session::SessionStatus::Idle,
            ),
            // sess_bh：claude Waiting（标记路径）—— **N1 反向锁独占**（该用例会真投递，
            //   必须独占 id：INFLIGHT 按裸 id 字符串全局占用，跨用例撞 id 即串键）
            inj_sess(
                "sess_bh",
                crate::session::AgentType::Claude,
                69,
                crate::session::SessionStatus::Waiting,
            ),
            // ===== N3 短路序计数族（sess_bi..sess_bk）=====
            // sess_bi：codex **Waiting** + 计划族 → 门由状态满足（零读页臂）
            inj_sess(
                "sess_bi",
                crate::session::AgentType::Codex,
                70,
                crate::session::SessionStatus::Waiting,
            ),
            // sess_bj：codex **Idle** + 审批标记（由用例播种）→ 门由标记满足（零读页臂）
            inj_sess(
                "sess_bj",
                crate::session::AgentType::Codex,
                71,
                crate::session::SessionStatus::Idle,
            ),
            // sess_bk：**claude** Idle + 尾部计划 → 族收窄（零读页臂，T1 零额外 IO 口径）
            inj_sess(
                "sess_bk",
                crate::session::AgentType::Claude,
                72,
                crate::session::SessionStatus::Idle,
            ),
            // ===== 丁T5 阶段机族（sess_t5a..sess_t5h）=====
            // **守卫 id 立规**：阶段机用例会真投递，INFLIGHT 按裸 id 全局占用——
            // 每个 POST 用例必须独占一个 id（与批次丙 sess_u..sess_bk 全段互异）。
            // 全 claude Waiting（阶段机只对 claude 放行）。
            sess("sess_t5a", 84, crate::session::SessionStatus::Waiting),
            sess("sess_t5b", 85, crate::session::SessionStatus::Waiting),
            sess("sess_t5c", 86, crate::session::SessionStatus::Waiting),
            sess("sess_t5d", 87, crate::session::SessionStatus::Waiting),
            sess("sess_t5e", 88, crate::session::SessionStatus::Waiting),
            sess("sess_t5f", 89, crate::session::SessionStatus::Waiting),
            sess("sess_t5g", 90, crate::session::SessionStatus::Waiting),
            sess("sess_t5h", 91, crate::session::SessionStatus::Waiting),
            // 工具面用例（非 claude 的拒绝路径不走阶段机，但同样要独占 id）
            inj_sess(
                "sess_t5i",
                crate::session::AgentType::Codex,
                92,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_t5j",
                crate::session::AgentType::OpenCode,
                93,
                crate::session::SessionStatus::Waiting,
            ),
            inj_sess(
                "sess_t5k",
                crate::session::AgentType::Kimi,
                94,
                crate::session::SessionStatus::Waiting,
            ),
        ];
        Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: sessions.clone(),
                total_count: sessions.len(),
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector,
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe,
            host_source: Box::new(|| serde_json::Value::Null),
            message_source,
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        })
    }

    /// 问答可用性（通道 A 标记路径）：sess_u 播种问题标记（探测档案单选夹具）→
    /// 200 available=true + source="mark" + questions 结构（header/question/
    /// multiSelect/options[{label,description}]）+ **无 key 字段**（键位不外泄给 UI）+
    /// gate：无 cookie → 403。
    #[tokio::test]
    async fn question_options_available_via_mark() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_u",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        // 无 cookie → 403（nest 内层 gate 结构性覆盖新端点）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_u",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "问答端点必须过 gate");
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_u",
                Some("mam_device=mm"),
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
            "问答可用性是门禁下私有数据，禁止中间层缓存"
        );
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["source"], "mark");
        let qs = v["questions"].as_array().expect("questions 应为数组");
        assert_eq!(qs.len(), 1, "探测档案单选夹具 = 单问题");
        assert_eq!(qs[0]["header"], "Next step");
        assert_eq!(
            qs[0]["question"],
            "This is a demo question — what would you like to do next?"
        );
        assert_eq!(qs[0]["multiSelect"], false);
        let opts = qs[0]["options"].as_array().unwrap();
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0]["label"], "Tool demo");
        assert_eq!(opts[0]["description"], "Explain how AskUserQuestion works.");
        assert_eq!(opts[2]["label"], "Nothing yet");
        assert!(
            fake.recorded_keys().is_empty() && fake.recorded().is_empty(),
            "查询端点不得触发任何注入"
        );
    }

    /// 问答应答（单选 select）：sess_v → 200 key_sent + FakeInjector 收到 (pid=32, "2")
    /// 恰一键（**无回车无 Esc**——探测 K1/K2：数字直接提交，后补 Esc 中断模型回合）+
    /// 审计 action=answer result=ok content=select#2。
    #[tokio::test]
    async fn question_answer_select_sends_digit() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_v",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_v","action":"select","index":1}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "问答应答回执是门禁下私有数据，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\""),
            "单选 select 应回执 key_sent：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(32u32, "2".to_string())],
            "select#2 = 单个数字键（无回车无 Esc，探测 K1/K2 定案）：{:?}",
            fake.recorded_keys()
        );
        assert!(fake.recorded().is_empty(), "问答走按键通道，不走文本注入");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1);
        assert_eq!(
            audits[0].action, "answer",
            "问答审计 action=answer（词表追加）"
        );
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].channel, "fake");
        assert_eq!(audits[0].session_id, "sess_v");
        assert_eq!(
            audits[0].summary, "select#2",
            "摘要 = 动作#UI编号（从 1 起）"
        );
    }

    /// 问答应答（多选 toggle）：sess_w → 200 key_sent + (pid=33, "1") 恰一键
    /// （切换勾选不提交——探测 K8）+ 审计 answer。
    #[tokio::test]
    async fn question_answer_toggle_sends_digit() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_w",
                1_000,
                "等待回答",
                Some(Q_MULTI_PAYLOAD),
            )
        });
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_w","action":"toggle","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"status\":\"key_sent\""));
        assert_eq!(
            fake.recorded_keys(),
            vec![(33u32, "1".to_string())],
            "toggle#1 = 单个数字键（切换勾选，探测 K8）：{:?}",
            fake.recorded_keys()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "answer");
        assert_eq!(audits[0].summary, "toggle#1");
    }

    // ==== 丁T5：提交与自由作答的**阶段机端点用例** ====
    //
    // 阶段机要「每段屏读复核」，而真实屏读需要 conhost（CI 恒 None）→ 经
    // `RemoteState.screen_probe` 缝注入**脚本化屏序列**：每一屏都是**真机屏幕原文**
    // （2026-09-21 探测档案 `C-s8-*` / `C-s7-*` 逐字），序列按「按键会重绘」的时序
    // 推进。于是「段推进 / 段中止 / 回执三态」全部在门禁里可断言。
    //
    // **推进纪律**：屏序列由**假注入器**推（每次 `locate_and_send_key_spec` /
    // `locate_and_inject_spec` 调用推一格）——这模拟生产的时序（按键 → TUI 重绘），
    // 而屏读缝本身**只读当前屏**（不推进）。两者分开才能在测试里表达「按键被吞」
    // （发了键但屏不变）这类形态：那时把两次注入映射到同一屏即可。
    mod stage_screen {
        use std::sync::{Arc, Mutex};

        /// 屏序列 + 推进计数（按键推一格）。`Arc` 便于注入到假注入器与屏读缝两处。
        pub struct Script {
            pub screens: Vec<Vec<String>>,
            pub pos: Mutex<usize>,
        }

        impl Script {
            pub fn new(screens: Vec<Vec<String>>) -> Arc<Self> {
                Arc::new(Self {
                    screens,
                    pos: Mutex::new(0),
                })
            }
            /// 当前屏（不推进）；序列为空 → None（= 读不到屏）
            pub fn current(&self) -> Option<Vec<String>> {
                let p = *self.pos.lock().unwrap();
                self.screens
                    .get(p.min(self.screens.len().saturating_sub(1)))
                    .cloned()
            }
            /// 按键 → TUI 重绘（推一格；到末屏则停——生产侧「屏不再变」）
            pub fn advance(&self) {
                let mut p = self.pos.lock().unwrap();
                if *p + 1 < self.screens.len() {
                    *p += 1;
                }
            }
            /// 屏读缝：只读当前屏（`None` = 读不到屏）
            pub fn probe(self: &Arc<Self>) -> Arc<crate::remote::server::ScreenProbeFn> {
                let me = self.clone();
                Arc::new(move |_sid: &str, _pid: u32| me.current())
            }
        }
    }

    /// 推屏的假注入器（在既有 [`FakeInjector`] 的记账之外，把每次注入映射成一次重绘）。
    /// 直接复用 `FakeInjector` + 一个 `Script`：本包装只做「转发 + 推进」。
    struct AdvancingInjector {
        inner: Arc<FakeInjector>,
        script: Arc<stage_screen::Script>,
        /// 失败开关（透传给内层；本包装只需知道「是否推进」——失败时不推进，因为
        /// 没送到终端就不会重绘）
        failing: bool,
    }

    impl crate::inject::engine::Injector for AdvancingInjector {
        fn name(&self) -> &'static str {
            self.inner.name()
        }
        fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String> {
            let r = self.inner.locate_and_inject(pid, text);
            if r.is_ok() && !self.failing {
                self.script.advance();
            }
            r
        }
        fn locate_and_send_key(&self, pid: u32, key: &str) -> Result<(), String> {
            let r = self.inner.locate_and_send_key(pid, key);
            if r.is_ok() && !self.failing {
                self.script.advance();
            }
            r
        }
        fn locate_and_send_key_spec(
            &self,
            pid: u32,
            key: &str,
            spec: &crate::inject::families::FamilySpec,
        ) -> Result<(), String> {
            let r = self.inner.locate_and_send_key_spec(pid, key, spec);
            if r.is_ok() && !self.failing {
                self.script.advance();
            }
            r
        }
        fn locate_and_inject_spec(
            &self,
            pid: u32,
            text: &str,
            spec: &crate::inject::families::FamilySpec,
        ) -> Result<(), String> {
            let r = self.inner.locate_and_inject_spec(pid, text, spec);
            if r.is_ok() && !self.failing {
                self.script.advance();
            }
            r
        }
    }

    /// 阶段机用例的 state：`question_state_full` + 脚本化屏读缝（会话清单同）。
    fn question_state_scripted(
        injector: Arc<dyn crate::inject::engine::Injector>,
        script: Arc<stage_screen::Script>,
    ) -> Arc<RemoteState> {
        question_state_full(
            injector,
            Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            script.probe(),
        )
    }

    /// 阶段机用例的真机屏幕夹具（探测档案 2026-09-21 原文，逐字；与
    /// `inject::question::tests` 的同名夹具同源——两处都是「一份原文两处引用」，
    /// 端点侧需要独立可用的副本，否则测试模块间要提权互引）。
    mod screen_fixtures {
        fn lines(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| s.to_string()).collect()
        }
        /// `C-s8-cursor-submit-20260921-015844.png`（焦点在 Submit 行）
        pub fn submit_focused() -> Vec<String> {
            lines(&[
                " ← ☒ Favorite fruits  ✔Submit  →",
                "",
                " Which fruits are your favorites? (Select all that apply)",
                "",
                " 1. [✓] Apple",
                " A sweet, crisp fruit available in many varieties.",
                " 2. [ ] Banana",
                " A soft, tropical fruit rich in potassium.",
                " 3. [ ] Peach",
                " A juicy summer fruit with a stone pit.",
                " 4. [ ] Type something",
                " ❯   Submit",
                " 5. Chat about this",
                "",
                " Enter to select · ↑/ to navigate · Esc to cancel",
            ])
        }
        /// `C-s8-submitted-20260921-015906.png`（Review 确认屏）
        pub fn review() -> Vec<String> {
            lines(&[
                " ← ☒ Favorite fruits  ✔Submit  →",
                "",
                " Review your answers",
                "",
                " ● Which fruits are your favorites? (Select all that apply)",
                " → Banana, Apple",
                "",
                " Ready to submit your answers?",
                "",
                " 1. Submit answers",
                " 2. Cancel",
            ])
        }
        /// `C-s8-final-20260921-015941.png`（终态）
        pub fn answered() -> Vec<String> {
            lines(&[
                " ● User answered Claude's questions:)",
                " L  • Which fruits are your favorites? (Select all that apply) → Banana, Apple",
                "",
                " Thought for 2s (ctrl+o to expand)",
            ])
        }
        /// 普通输出屏（无任何阶段锚）
        pub fn plain() -> Vec<String> {
            lines(&["  some output", " > "])
        }
        /// `C-s7-q-ui-20260921-015440.png`（单选自由作答屏）
        pub fn free_row() -> Vec<String> {
            lines(&[
                " ☐ Preferred drink",
                "",
                " Which drink do you prefer?",
                "",
                " 1. Coffee",
                " 2. Tea",
                " 3. Type something.",
                "",
                " 4. Chat about this",
            ])
        }
        /// 焦点已落在自由作答行（`C-s7-text-in-input-*` 形态）
        pub fn free_row_focused() -> Vec<String> {
            lines(&[
                " ☐ Preferred drink",
                "",
                " Which drink do you prefer?",
                "",
                " 1. Coffee",
                " 2. Tea",
                " ❯ 3. Type something.",
                "",
                " 4. Chat about this",
            ])
        }
    }

    /// 阶段机用例的**装配**：脚本屏序列 + 推屏假注入器 + 注入缝的 state + 已配对设备。
    /// 返回 `(state, fake, script)`——`fake` 用于断言「实际发了哪些键/哪些文本」。
    fn stage_rig(
        screens: Vec<Vec<String>>,
        failing: bool,
    ) -> (
        Arc<RemoteState>,
        Arc<FakeInjector>,
        Arc<stage_screen::Script>,
    ) {
        let script = stage_screen::Script::new(screens);
        let inner = if failing {
            FakeInjector::failing("注入通道拒绝")
        } else {
            FakeInjector::ok()
        };
        let adv = Arc::new(AdvancingInjector {
            inner: inner.clone(),
            script: script.clone(),
            failing,
        });
        let state = question_state_scripted(adv, script.clone());
        persist_named_device(&state, "mm", "测试设备");
        (state, inner, script)
    }

    /// 播种问题标记（阶段机用例的公共前置；`sid` 由各用例独占——守卫 id 立规）。
    fn mark_question(state: &Arc<RemoteState>, tool: &str, sid: &str, payload: &str) {
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                tool,
                sid,
                1_000,
                "等待回答",
                Some(payload),
            )
        });
    }

    /// **丁T5 端到端①（多选提交 happy path）**：提交屏（焦点已在 Submit 行）→ 回车 →
    /// Review 屏 → 抄屏上编号 '1' → 终态屏。
    ///
    /// 断言：键序 = `[enter, "1"]`（**零 down**——焦点本就在提交行）；回执带
    /// `done:true` + `verified:true`（屏读确认了终态）；审计摘要 = `submit::receipt`
    /// （段名进摘要）。还原动作：把 `run_submit_stages` 换回批次丙的盲发序列
    /// （down×4 + enter + '1'）→ 第一句键序断言先红。
    #[tokio::test]
    async fn question_submit_stage_machine_happy_path() {
        let (state, fake, _script) = stage_rig(
            vec![
                screen_fixtures::submit_focused(),
                screen_fixtures::review(),
                screen_fixtures::answered(),
            ],
            false,
        );
        mark_question(&state, "claude", "sess_t5a", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5a","action":"submit"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\"")
                && body.contains("\"done\":true")
                && body.contains("\"verified\":true"),
            "闭环走完的回执 = key_sent + done + verified=true：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(84u32, "enter".to_string()), (84u32, "1".to_string())],
            "键序 = [enter, '1']（焦点已在 Submit 行 → 零走位；确认键抄自 Review 屏）：{:?}",
            fake.recorded_keys()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "answer");
        assert_eq!(
            audits[0].summary, "submit::receipt",
            "审计摘要带段名（走完的段 = receipt）"
        );
        assert_eq!(audits[0].result, "ok");
    }

    /// **丁T5 端到端②（提交路径中止：未见 Review 屏）**——问题 7 的正解。
    ///
    /// 脚本：提交屏 → 回车后屏不变（Review 屏未出现，轮询窗尽）。断言：发了 enter、
    /// **没发任何数字**；回执 `status:"failed"` + `aborted:true` + `stage:"review"`
    /// + 中文 error（用户可读）；审计 result = `aborted:review`。
    /// 还原动作：把 Review 段改回「不等屏读直接发 '1'」→ 第二句键序断言先红。
    #[tokio::test]
    async fn question_submit_stage_machine_aborts_when_review_absent() {
        // 两屏都是提交屏：回车推进到第二屏（还是提交屏）→ Review 轮询窗尽
        let (state, fake, _script) = stage_rig(
            vec![
                screen_fixtures::submit_focused(),
                screen_fixtures::submit_focused(),
            ],
            false,
        );
        mark_question(&state, "claude", "sess_t5b", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5b","action":"submit"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "中止走 200 failed 槽（可重试语义）");
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"")
                && body.contains("\"aborted\":true")
                && body.contains("\"stage\":\"review\""),
            "中止回执须可程序分诊到段（failed+aborted+stage）：{body}"
        );
        assert!(
            body.contains("Review 确认屏"),
            "error 文案须点名缺什么（用户可读）：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(85u32, "enter".to_string())],
            "**绝不发确认数字**（Review 屏不在场）：{:?}",
            fake.recorded_keys()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "answer");
        assert_eq!(audits[0].summary, "submit::review", "段名进审计摘要");
        assert_eq!(audits[0].result, "aborted:review");
    }

    /// **丁T5 端到端③（走位失败中止）**：屏上只有提交屏但**焦点在选项行**（每次 ↓
    /// 后屏不变 = 终端吞键）→ 走位上限耗尽 → 中止且**不发回车**。
    ///
    /// 断言：键全是 `down`（一个不落都是走位键）、**没有 enter**；`stage:"submit-row"`。
    /// 还原动作：删掉走位复核（发满就回车）→ 第二句断言先红。
    #[tokio::test]
    async fn question_submit_stage_machine_aborts_when_walk_stalls() {
        // 焦点永在选项行（屏不推进 → 每次读都是同一屏）
        let mut stuck = screen_fixtures::submit_focused();
        for l in stuck.iter_mut() {
            if l.contains("❯   Submit") {
                *l = "    Submit".to_string(); // 提交行在场但无焦点标记
            }
            if l.contains("1. [✓] Apple") {
                *l = format!(" ❯{l}");
            }
        }
        let (state, fake, _script) = stage_rig(vec![stuck], false);
        mark_question(&state, "claude", "sess_t5c", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5c","action":"submit"}"#),
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"stage\":\"submit-row\"") && body.contains("\"aborted\":true"),
            "走位失败的中止段 = submit-row：{body}"
        );
        let keys = fake.recorded_keys();
        assert!(
            !keys.iter().any(|(_, k)| k == "enter"),
            "走位未到位 ⇒ **绝不发回车**：{keys:?}"
        );
        assert!(
            keys.iter().all(|(_, k)| k == "down") && !keys.is_empty(),
            "只发过走位键（且确实发过——上限 3+2=5 次）：{keys:?}"
        );
    }

    /// **丁T5 端到端④（终态回执未见 → 不谎报完成）**：闭环走完（提交屏 → Review →
    /// 确认已发），但终态屏读窗内只有普通输出 → `verified:false`（**仍是 key_sent +
    /// done**——键确实发出去了，事实是「投递完成、未见回执」）。
    ///
    /// 还原动作：把 `StageDone` 的 verified 改成恒 true → 第一句断言先红。
    #[tokio::test]
    async fn question_submit_stage_machine_receipt_unseen_not_lied() {
        let (state, fake, _script) = stage_rig(
            vec![
                screen_fixtures::submit_focused(),
                screen_fixtures::review(),
                screen_fixtures::plain(),
            ],
            false,
        );
        mark_question(&state, "claude", "sess_t5d", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5d","action":"submit"}"#),
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"done\":true") && body.contains("\"verified\":false"),
            "闭环走完但未见终态回执 = done:true + verified:false（不谎报）：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(87u32, "enter".to_string()), (87u32, "1".to_string())]
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(
            audits[0].result, "ok:receipt-unseen",
            "审计如实记「已投递但未核验到回执」"
        );
    }

    /// **丁T5 复评 F6-2（恢复既有覆盖）：阶段机中途投递失败 = 「投递失败」而非「中止」**。
    ///
    /// 覆盖两条既有断言（父提交 `b92a658` 的
    /// `question_answer_submit_first_error_stops_sequence_and_audits_failed` 锁的是
    /// 同一件事，本批重构时被误删——复评 Important-2）：
    /// ① **首错即停**：走位段第一个 `down` 就 Err → **后续键一个都不再投**；
    /// ② 回执/审计语义：`status:"failed"` **不带** `aborted`（投递失败 ≠ 阶段判据中止），
    ///    审计 `failed:<e>` 前缀口径逐字同批次丙。
    ///
    /// 用 `stage_rig(..., failing=true)`——该参数此前从未被传过 `true`（复评指出为
    /// 死参数），本用例是它唯一的消费者。
    /// 还原动作：把 `StageAbortKind::Delivery` 并回 `Screen`（回执变
    /// `aborted:true`）→ 第二句断言先红；去掉首错即停（继续投后续键）→ 第一句先红。
    #[tokio::test]
    async fn question_submit_stage_machine_delivery_failure_stops_and_audits_failed() {
        // 提交屏（焦点在选项行）→ 需要一个 down 才到位 → 第一个 down 即失败
        let mut stuck = screen_fixtures::submit_focused();
        for l in stuck.iter_mut() {
            if l.contains("❯   Submit") {
                *l = "    Submit".to_string();
            }
            if l.contains("1. [✓] Apple") {
                *l = format!(" ❯{l}");
            }
        }
        let (state, fake, _script) = stage_rig(vec![stuck], true); // failing=true
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_ak",
                1_000,
                "等待回答",
                Some(Q_MULTI_PAYLOAD),
            )
        });
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ak","action":"submit"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "投递失败走 200 failed 槽（可重试）");
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("注入通道拒绝"),
            "投递失败须 200 failed 并透传错误文案：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(46u32, "down".to_string())],
            "**首错即停**：走位第一个 down 即 Err，恰一条、后续键不再出手：{:?}",
            fake.recorded_keys()
        );
        assert!(
            !body.contains("\"aborted\""),
            "投递失败**不得**报成阶段中止（aborted 是「屏上形态不符」的语义，两者用户动作不同）：{body}"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "answer");
        assert_eq!(
            audits[0].result, "failed:下箭头投递失败（注入通道拒绝）",
            "审计 result = failed:错误文案（与批次丙同前缀口径）"
        );
        assert_eq!(audits[0].summary, "submit", "投递失败不追加段名");
        assert_eq!(audits[0].session_id, "sess_ak");
    }

    /// **丁T5 端到端⑤（读屏不可用 → 零投递中止）**：`screen_probe` 恒 `None`
    /// （非 Windows / 屏读失败）→ 第 1 段就中止，**零按键**。这是安全面：读不到屏
    /// 就绝不猜着发键。
    /// 还原动作：把第 1 段的屏读检查删掉（默认形态成立）→ 本用例先红（会发 enter）。
    #[tokio::test]
    async fn question_submit_stage_machine_aborts_with_zero_keys_when_screen_unavailable() {
        let (state, fake, _script) = stage_rig(vec![], false); // 空序列 → 恒 None
        mark_question(&state, "claude", "sess_t5e", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5e","action":"submit"}"#),
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"aborted\":true") && body.contains("\"stage\":\"submit-row\""),
            "读不到屏的中止段 = submit-row：{body}"
        );
        assert!(
            fake.recorded_keys().is_empty() && fake.recorded().is_empty(),
            "读不到屏 ⇒ 零投递（一个键都不猜）：keys={:?} texts={:?}",
            fake.recorded_keys(),
            fake.recorded()
        );
    }

    /// **丁T5 端到端⑥（自由作答 happy path，裁3 安全面）**：单选屏 → 定位 `Type
    /// something`（屏上编号 3）→ 文本（**字符通道**）→ 回车 → 终态。
    ///
    /// 断言：键通道 = `['3', 'enter']`（**无 Esc 无第二数字**）；文本通道恰好一次且
    /// 内容 = 用户文本、**不带** `[mobile]` 签名；回执 `done:true` +
    /// `verified:true`；审计摘要 = `freeText::free-text`（**不含正文**）。
    /// 还原动作：把文本改走 `locate_and_send_key_spec`（键通道）→ 第二句断言先红。
    #[tokio::test]
    async fn question_free_text_stage_happy_path_sends_text_via_text_channel() {
        let (state, fake, _script) = stage_rig(
            vec![
                screen_fixtures::free_row(),
                screen_fixtures::free_row_focused(),
                screen_fixtures::free_row_focused(),
                screen_fixtures::answered(),
            ],
            false,
        );
        mark_question(&state, "claude", "sess_t5f", Q_SINGLE_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5f","action":"freeText","text":"green tea please"}"#),
            ))
            .await
            .unwrap();
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\"")
                && body.contains("\"done\":true")
                && body.contains("\"verified\":true"),
            "自由作答闭环回执：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(89u32, "3".to_string()), (89u32, "enter".to_string())],
            "键通道 = [定位数字(屏上编号 3), 提交回车]——**无 Esc 无第二数字**：{:?}",
            fake.recorded_keys()
        );
        assert_eq!(
            fake.recorded(),
            vec![(89u32, "green tea please".to_string())],
            "文本恰一次、走字符通道、**不带 [mobile] 签名**：{:?}",
            fake.recorded()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "answer");
        assert_eq!(audits[0].summary, "freeText::free-text");
        assert!(
            !audits[0].summary.contains("green"),
            "审计摘要**不得承载用户正文**：{}",
            audits[0].summary
        );
    }

    /// **丁T5 端到端⑦（自由作答文本归一）**：换行/控制字符经 `normalize_newlines`
    /// 后的形态进字符通道（**不含裸换行 / 控制字符**——注入通道安全面）。
    /// 还原动作：把归一那行删掉（直接投原文）→ 断言先红（文本里会出现裸换行）。
    #[tokio::test]
    async fn question_free_text_text_is_normalized_before_injection() {
        let (state, fake, _script) = stage_rig(
            vec![
                screen_fixtures::free_row(),
                screen_fixtures::free_row_focused(),
                screen_fixtures::free_row_focused(),
                screen_fixtures::answered(),
            ],
            false,
        );
        mark_question(&state, "claude", "sess_t5g", Q_SINGLE_PAYLOAD);
        let app = router(state.clone());
        // JSON 里的 \n 是**真换行**（JSON 转义），\u001b 是真 ESC
        let payload =
            r#"{"sessionId":"sess_t5g","action":"freeText","text":"第一行\n第二行\u001b[31m"}"#;
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(payload),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let texts = fake.recorded();
        assert_eq!(texts.len(), 1, "文本恰一次：{texts:?}");
        let sent = &texts[0].1;
        assert!(
            sent.contains("第一行\\n第二行") && sent.ends_with("[31m"),
            "换行 → 字面 \\n、ESC 剥除（归一）：{sent:?}"
        );
        assert!(
            !sent.chars().any(|c| (c as u32) <= 0x1F),
            "注入文本零 C0 残留（通道安全面）：{sent:?}"
        );
        assert!(
            !sent.contains("[mobile"),
            "作答文本**不带** mobile 签名（签名是消息语义，作答不是消息）：{sent:?}"
        );
    }

    /// **丁T5 端到端⑧（自由作答的空文本 → 400）**：入口参数校验拦下（比 200 failed
    /// 更准确——「参数就不对」）。
    /// 还原动作：删掉 handler 的 freeText 空文本检查 → 本用例先红（变成 200）。
    #[tokio::test]
    async fn question_free_text_empty_text_is_bad_request() {
        let (state, fake, _script) = stage_rig(vec![screen_fixtures::free_row()], false);
        mark_question(&state, "claude", "sess_t5h", Q_SINGLE_PAYLOAD);
        let app = router(state.clone());
        for payload in [
            r#"{"sessionId":"sess_t5h","action":"freeText"}"#,
            r#"{"sessionId":"sess_t5h","action":"freeText","text":"   "}"#,
            r#"{"sessionId":"sess_t5h","action":"freeText","text":"\n\t"}"#,
        ] {
            let r = app
                .clone()
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-question/answer",
                    Some("mam_device=mm"),
                    Some(payload),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "空/纯空白文本 → 400：{payload}");
        }
        assert!(
            fake.recorded_keys().is_empty() && fake.recorded().is_empty(),
            "400 路径零投递零审计"
        );
    }

    /// 单工具 state（阶段机工具面用例共用）：会话清单仅一条 + 脚本屏读缝。
    fn single_tool_scripted_state(
        tool: crate::session::AgentType,
        sid: &str,
        pid: u32,
        screens: Vec<Vec<String>>,
    ) -> (
        Arc<RemoteState>,
        Arc<FakeInjector>,
        Arc<stage_screen::Script>,
    ) {
        let script = stage_screen::Script::new(screens);
        let inner = FakeInjector::ok();
        let adv = Arc::new(AdvancingInjector {
            inner: inner.clone(),
            script: script.clone(),
            failing: false,
        });
        let session = inj_sess(sid, tool, pid, crate::session::SessionStatus::Waiting);
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: adv,
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: script.probe(),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        });
        persist_named_device(&state, "mm", "测试设备");
        (state, inner, script)
    }

    /// **丁T5 端到端⑨（自由作答的工具面）→ 批次戊 E4/E5/E6 更新**：kimi（Other 行）
    /// /codex（Tab 备注）/opencode（own answer）均已升格走各自阶段机。本用例改钉
    /// opencode freeText 的**阶段机中止**：喂无 opencode 锚的屏（claude free_row 形态）
    /// → 第 1 段定位不到 own answer 行 → 200 failed{aborted:true}（零投递）。
    #[tokio::test]
    async fn question_free_text_refused_for_unverified_tools() {
        for (tool, tool_id, sid, pid) in [(
            crate::session::AgentType::OpenCode,
            "opencode",
            "sess_t5j",
            93u32,
        )] {
            let (state, inner, _s) =
                single_tool_scripted_state(tool, sid, pid, vec![screen_fixtures::free_row()]);
            mark_question(&state, tool_id, sid, Q_SINGLE_PAYLOAD);
            let app = router(state.clone());
            let r = app
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-question/answer",
                    Some("mam_device=mm"),
                    Some(&format!(
                        r#"{{"sessionId":"{sid}","action":"freeText","text":"hi"}}"#
                    )),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "opencode freeText 走阶段机（非 409）");
            let body = body_string(r).await;
            assert!(
                body.contains("aborted"),
                "无 own answer 锚 → 阶段机中止：{body}"
            );
            assert!(
                inner.recorded_keys().is_empty() && inner.recorded().is_empty(),
                "第 1 段定位即中止 → 零投递"
            );
        }
    }

    /// **丁T5 复评 F6-3（多选卡的自由作答在端点被拒）**：claude（工具支持）但题目是
    /// **多选** → 409 `tool_readonly` + 零投递。
    ///
    /// 依据（实机截图，复评核对）：多选屏的自由作答行渲染为 `4. [ ] Type something`
    /// （带勾选框；`C-s8-cursor-submit-20260921-015844.png` 第 4 行），而
    /// `locate_free_text_row` 的判据是「剥编号后以 `Type something` 开头」→ 不匹配 →
    /// 定位恒失败。**在端点就拒绝**（而不是让用户白等一次必然中止的全链）。
    ///
    /// 还原动作：把 `action_supported` 的 FreeText 分支去掉 `free_text_shape_supported`
    /// 检查 → 本用例先红（会 200 并走到 free-row 段中止）。
    #[tokio::test]
    async fn question_free_text_refused_on_multi_select_question() {
        // 用多选夹具（Q_MULTI_PAYLOAD）+ **单选题不会触发**——这正是本用例的区分点
        let (state, inner, _s) = single_tool_scripted_state(
            crate::session::AgentType::Claude,
            "sess_t5mft",
            95u32,
            vec![screen_fixtures::free_row()],
        );
        mark_question(&state, "claude", "sess_t5mft", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5mft","action":"freeText","text":"hi"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "多选卡的自由作答 → 409（不假装能发）");
        let body = body_string(r).await;
        assert!(
            body.contains("tool_readonly"),
            "拒绝码须可程序分诊（细分原因见后端日志与前端分診文案）：{body}"
        );
        // 契约说明：409 体**只回错误码**（既有口径——不给存在性预言机、不泄露细节）；
        // 「多选题的自由作答请到终端完成」这句在 `ActionRefusal` 里，进 debug 日志与
        // 前端分診（`tool_readonly` → 中文文案），**不进响应体**。本断言锁住这个口径。
        assert!(
            inner.recorded_keys().is_empty() && inner.recorded().is_empty(),
            "拒绝路径零投递：keys={:?} texts={:?}",
            inner.recorded_keys(),
            inner.recorded()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(
            audits.is_empty(),
            "校验失败零审计（approve/question 同口径）"
        );
    }

    /// **丁T5 端到端⑩（多选提交的工具面）→ 批次戊 E4 更新**：kimi 多选提交升格为
    /// **阶段机**（run_kimi_submit_stages：Review 在场判读 → tab → Review 汇总屏 →
    /// 屏上编号确认）。喂 claude 形态屏（无 kimi 锚）时阶段机在 review 段中止——
    /// 回执 200 failed{aborted:true, stage:review}，已发键恰为 ["tab"]（Review 不在
    /// 场的第一步），不发确认键。
    #[tokio::test]
    async fn question_submit_kimi_goes_through_stage_machine() {
        let (state, inner, _s) = single_tool_scripted_state(
            crate::session::AgentType::Kimi,
            "sess_t5k",
            94u32,
            vec![screen_fixtures::submit_focused()],
        );
        mark_question(&state, "kimi", "sess_t5k", Q_MULTI_PAYLOAD);
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_t5k","action":"submit"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "kimi 提交走阶段机（非 409 只读）");
        let body = body_string(r).await;
        assert!(
            body.contains("aborted"),
            "屏无 kimi 锚 → 阶段机中止：{body}"
        );
        assert!(body.contains("review"), "中止段 = review：{body}");
        assert_eq!(
            inner.recorded_keys(),
            vec![(94u32, "tab".to_string())],
            "Review 不在场 → 先发 tab，之后中止（不发确认键）：{:?}",
            inner.recorded_keys()
        );
    }

    /// 问答应答（取消）：sess_y → 200 key_sent + (pid=35, "esc") 恰一键（探测 K3：
    /// Esc=取消/拒绝整个问题）+ 审计 answer。
    #[tokio::test]
    async fn question_answer_cancel_sends_esc() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_y",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_y","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"status\":\"key_sent\""));
        assert_eq!(
            fake.recorded_keys(),
            vec![(35u32, "esc".to_string())],
            "cancel = 单键 esc（探测 K3）：{:?}",
            fake.recorded_keys()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "answer");
        assert_eq!(audits[0].summary, "cancel");
    }

    /// guard-busy 回归（F1 同款）：问答应答遇 in-flight 占用 → 200 failed{「投递进行中，
    /// 请稍后重试」} + 零注入 + 不落审计。sess_z 独占（守卫持到测尾，守卫 id 立规）。
    #[tokio::test]
    async fn question_answer_busy_inflight_returns_failed_without_key() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_z",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        let _busy = crate::inject::queue::try_acquire_inflight("sess_z").unwrap();
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_z","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("投递进行中，请稍后重试"),
            "in-flight 占用时问答应 200 failed 让位：{body}"
        );
        assert!(
            fake.recorded_keys().is_empty() && fake.recorded().is_empty(),
            "占用期间不得出手任何键（零注入）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "忙让位不落审计");
    }

    /// 多问题只读（「结论不超证据」——探测档案：questions.length>1 翻页键序未测）：
    /// sess_aa → 409 multi_questions + 零注入。GET 照常 available=true（前端按只读
    /// 卡渲染，见 QuestionCard vitest）。
    ///
    /// **丁T2 扩面**：多题只读对 **codex / kimi 同样生效**（任务书「确认这条对 kimi 也
    /// 生效即可」）——本用例在 claude 之外补两家：端点侧的 `questions.len() != 1` 判据
    /// 在**工具分发之前**（`answer_key_sequence_for` 之前），故与键序档无关；本锁把它
    /// 钉住，防未来某家升格档位时误放行多题注入。逐题注入列入下批（需实机定导航序）。
    #[tokio::test]
    async fn multi_question_readonly_covers_codex_and_kimi() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                // codex：待决多题 tool-call（其后无 tool-result）
                "sess_ay" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![auq_tool_call(0, Q_TWO_QUESTIONS_PAYLOAD)],
                    truncated: false,
                }),
                // kimi：同形（本机 wire 的 3 问题实录缩录为 2 题）
                "sess_az" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![auq_tool_call(0, KIMI_TWO_Q_PAYLOAD)],
                    truncated: false,
                }),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // GET：两家多题 GET 照常可用（数据源不变）
        for sid in ["sess_ay", "sess_az"] {
            let r = app
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/m/api/v1/session-question?session_id={sid}"),
                    Some("mam_device=mm"),
                    None,
                ))
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["available"], true, "{sid}：多题 GET 照常可用");
            assert_eq!(v["questions"].as_array().unwrap().len(), 2);
        }
        // **批次戊 E4 更新**：kimi 多题升格为**交互**（K-5 数字直选+自动推进+Review
        // 汇总屏）——select 走 DigitAdvance 单数字键（**禁尾 Enter**，A3），200 key_sent；
        // codex 多题在 E4 时点仍只读（E5 升格），409 tool_readonly。
        // claude 多题只读由 question_answer_multi_questions_refused 继续钉住。
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_az","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "kimi 多题 select：交互放行（DigitAdvance）"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(61u32, "1".to_string())],
            "kimi 多题 select = 单个数字（无尾随 Enter——A3 禁令锁）：{:?}",
            fake.recorded_keys()
        );
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ay","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        // codex 多题（E4 时点）：单选题的 select 走既有 SingleDigitSubmit 档
        // （数字即交，200 key_sent）；多题完整交互（Tab 备注/切题）在 E5 升格
        assert_eq!(r.status(), 200, "codex 单选题 select：既有数字直选档放行");
        assert!(
            body_string(r).await.contains("key_sent"),
            "codex 单选题 select 照常 key_sent"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(61u32, "1".to_string()), (60u32, "1".to_string())],
            "kimi 多题 DigitAdvance + codex 单选直选，各恰一键"
        );
    }

    /// 多问题只读（claude 侧既有回归锁，丁T2 保留原样）
    #[tokio::test]
    async fn question_answer_multi_questions_refused() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_aa",
                1_000,
                "等待回答",
                Some(Q_TWO_QUESTIONS_PAYLOAD),
            )
        });
        let app = router(state.clone());
        // GET：available=true（只读展示的数据源）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_aa",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["questions"].as_array().unwrap().len(), 2);
        // POST：拒绝出手
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_aa","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("multi_questions"));
        assert!(fake.recorded_keys().is_empty(), "多问题零注入");
    }

    /// guard 矩阵：缺 index / 域外 action / 空 sessionId → 400 bad_request；越界
    /// index 与单选 submit → 400 bad_index；无标记会话与不存在会话 → 409 no_question；
    /// 全程零注入。
    #[tokio::test]
    async fn question_answer_guards() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_ac",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        // select 缺 index → 400
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ac","action":"select"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        assert!(body_string(r).await.contains("bad_request"));
        // 域外 action → 400
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ac","action":"bogus","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        // 空 sessionId → 400
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        // 越界 index（单选 3 选项取 #99）→ 400 bad_index
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ac","action":"select","index":99}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        assert!(body_string(r).await.contains("bad_index"));
        // submit 用在单选题 → 400 bad_index（单选无独立提交步）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ac","action":"submit"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        assert!(body_string(r).await.contains("bad_index"));
        // 无标记会话（sess_af）→ 409 no_question
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_af","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("no_question"));
        // 不存在会话 → 409 no_question（同形收敛，不给存在性预言机）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"nope","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("no_question"));
        assert!(fake.recorded_keys().is_empty(), "校验失败全程零注入");
    }

    /// 硬约束①（方向一）：**审批标记不得触发问答卡**——sess_ad 仅播审批标记 →
    /// 问答 GET available=false、POST answer 409 no_question、零注入；同一会话
    /// 审批 GET 照常 available（既有审批行为零回归）。
    #[tokio::test]
    async fn question_endpoints_blocked_by_approval_mark() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::approval_wait::mark(conn, "claude", "sess_ad", 1_000, "等待审批")
        });
        let app = router(state.clone());
        // 问答 GET：不可用
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ad",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "审批标记在场时问答卡不可用（硬约束①）"
        );
        // 问答 POST：409 no_question + 零注入
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ad","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("no_question"));
        assert!(fake.recorded_keys().is_empty());
        // 同会话审批 GET：available=true（审批行为零回归——标记路径跳过 detect）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_ad",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true, "审批标记路径零回归");
    }

    /// 硬约束①（方向二）：**问题标记不得触发审批红卡**——sess_ae 播问题标记且
    /// last_message 恰为审批 marker 命中句（最强隔离形态：不依赖 detect 未达）→
    /// 审批 GET available=false、审批 POST 409 not_waiting、零审批键；问答 GET
    /// 照常 available（问答行为零回归）。
    #[tokio::test]
    async fn question_mark_never_triggers_approve_card() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_ae",
                1_000,
                "等待回答",
                Some(Q_SINGLE_PAYLOAD),
            )
        });
        let app = router(state.clone());
        // 审批 GET：available=false（问题标记显式排除——Waiting 门会被叠加层满足，
        // 必须由问题标记检查拦下）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_ae",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "问题标记在场时审批卡不可用（硬约束①；detect 命中被显式压过）"
        );
        assert!(v["options"].as_array().unwrap().is_empty());
        // 审批 POST：409 not_waiting + 零审批键
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ae","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("not_waiting"));
        assert!(fake.recorded_keys().is_empty(), "审批键零出手");
        // 问答 GET：available=true（问答行为零回归）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ae",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true);
    }

    /// **丁T1 回归锁（本批引入的新矛盾，任务书硬要求）**：codex / opencode 的问题
    /// 待决被状态链推出**语义红 Waiting** 之后，审批卡**不得**借机误出。
    ///
    /// 与既有 `question_mark_never_triggers_approve_card`（问题**标记** + detect 命中
    /// 的隔离）互补：本用例是**无标记 + detect 未命中**形态（问题标记只有 claude
    /// 钩子路径会写，codex/opencode 无钩子通道）——会话靠状态链的语义红满足 approve 的
    /// Waiting 门，此时拦截来自 detect 门（问题文本不含审批 marker → hit=false）。
    /// 锁住的正是「新红灯不会把审批卡带出来」这条边。
    ///
    /// **detect 命中形态另有用例**：`question_pending_red_beats_approval_marker_text`
    /// （丁T1 复评 F-2，模型把问题写成审批措辞时 detect 会命中，靠尾部待决问答判定拦）。
    ///
    /// 断言（GET = 卡的数据源，**「approve 不可用」的判定面**）：available=false
    /// （detect miss）+ options 空 + reason null（自隐契约，与严格档提示条区分）。
    ///
    /// **POST 面如实申报**：`session-approve` POST **不走 detect**（既有契约——
    /// 客户端只 POST 它从 GET 拿到的 option id，GET 已把 detect 门走过；见
    /// `audit_action_vocab`「POST 不走 detect，无需 last_message」与 F2 用例）。
    /// 因此 POST 对本用例的两会话仍会按映射出键（codex 有默认映射）——这是 **T1 之前
    /// 就存在**的契约面，不在本任务改动范围内（改它会让既有端点契约测试全红）。
    /// 用户可见面已由 GET 的 available=false 关死：前端 ApproveCard 只在 available=true
    /// 时渲染按钮，无按钮即无 POST 入口。残余面（旁路客户端直接 POST）记录在案。
    #[tokio::test]
    async fn question_pending_red_never_triggers_approve_card() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        for sid in ["sess_al", "sess_am"] {
            // 审批 GET：不可用（问题文本不含审批 marker → detect miss）
            let r = app
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/m/api/v1/session-approve-options?session_id={sid}"),
                    Some("mam_device=mm"),
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(
                v["available"], false,
                "{sid}：问答待决的语义红不得触发审批卡（detect 门必须护住）"
            );
            assert!(
                v["options"].as_array().unwrap().is_empty(),
                "{sid}：不可用一律不下发选项"
            );
            assert_eq!(
                v["reason"],
                serde_json::Value::Null,
                "{sid}：非严格档不可批不得带 reason（ApproveCard 自隐契约）"
            );
        }
        assert!(fake.recorded_keys().is_empty(), "本用例不触任何注入路径");
        // 问答 GET：照常可用（问答行为零回归）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_u",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }

    /// **丁T1 复评 F-2 主用例**：「问答在场 → 审批不可用」在 **detect 会命中**时也必须
    /// 成立（最严形态，评审 I-3）。
    ///
    /// 场景：模型把问题写成审批措辞（"Would you like to run the following command?"，
    /// 完全自然的问法）——`detect` 是**纯文本子串**命中，此时 `hit` 本会为 true，
    /// 审批卡借语义红误出。拦截只能来自「尾部存在待决问答」判定
    /// （`pending_question_tail_index`，与问答端点同一份判据）。
    ///
    /// 夹具构造：message_source 对 sess_an / sess_ao 返回「问答形态 tool-call 且其后
    /// 无 tool-result」的消息页（= 待决问答在场）。
    /// 断言：GET available=false + options 空 + reason null（与 detect-miss 形态同收敛，
    /// 不给存在性预言机）；问答 GET 照常 available=true（问答零回归）。
    #[tokio::test]
    async fn question_pending_red_beats_approval_marker_text() {
        let fake = FakeInjector::ok();
        // 待决问答页（AUQ 形态 tool-call，其后无 tool-result）
        let page = crate::remote::content::MessagesPage {
            messages: vec![user_msg(0), auq_tool_call(1, Q_SINGLE_PAYLOAD)],
            truncated: false,
        };
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                if sid == "sess_an" || sid == "sess_ao" {
                    Ok(page.clone())
                } else {
                    Err("无消息".to_string())
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        for sid in ["sess_an", "sess_ao"] {
            // 审批 GET：detect 本会命中（last_message 是 codex marker 原文），但尾部
            // 待决问答 → 硬约束①（无标记分支）压为不可用
            let r = app
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/m/api/v1/session-approve-options?session_id={sid}"),
                    Some("mam_device=mm"),
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(
                v["available"], false,
                "{sid}：尾部待决问答在场时审批必须不可用（即使 last_message 命中审批 marker）"
            );
            assert!(
                v["options"].as_array().unwrap().is_empty(),
                "{sid}：不可用一律不下发选项"
            );
            assert_eq!(
                v["reason"],
                serde_json::Value::Null,
                "{sid}：硬约束① 的不可用不给 reason（与严格档提示条区分）"
            );
            // 问答 GET：照常可用（问答行为零回归；同一份判据在两处口径一致）
            let r = app
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/m/api/v1/session-question?session_id={sid}"),
                    Some("mam_device=mm"),
                    None,
                ))
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["available"], true, "{sid}：问答照常可用");
            assert_eq!(v["source"], "scan", "{sid}：通道 B 识别口径");
        }
        assert!(fake.recorded_keys().is_empty(), "本用例不触任何注入路径");
    }

    /// F-2 反向锁（防收窄过头）：**已答**的问答（tool-call 后随 tool-result）不再算
    /// 「问答在场」——此时 detect 命中的真审批会话必须照常可用（否则新判定会把真
    /// 审批卡压死）。夹具复用 sess_an 但消息页换成「tool-call + tool-result」。
    #[tokio::test]
    async fn answered_question_tail_does_not_block_approve() {
        let fake = FakeInjector::ok();
        let page = crate::remote::content::MessagesPage {
            messages: vec![
                auq_tool_call(0, Q_SINGLE_PAYLOAD),
                tool_result_msg(1, "Your questions have been answered: \"q\"=\"Tool demo\"."),
            ],
            truncated: false,
        };
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                if sid == "sess_an" {
                    Ok(page.clone())
                } else {
                    Err("无消息".to_string())
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // sess_an：Waiting + last_message 命中 codex marker + 已答问答尾部 → 审批照常可用
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_an",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], true,
            "已答问答不再构成「问答在场」——真审批会话照常可用（零回归）"
        );
        assert!(
            !v["options"].as_array().unwrap().is_empty(),
            "可批时下发选项"
        );
        // 问答 GET：已答 → 不可用（问答端点既有口径零回归）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_an",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "已答 → 问答不可用（既有口径）");
    }

    /// **丁T1 复评 F2-2 反锁**：新门按工具收窄——claude **不**受尾部判据压制。
    ///
    /// 场景（评审 Important-2 的假阳性形态，取自真实库 `0c41365d-…`）：claude 的
    /// AUQ 被用户用**纯文本**作答，`tool_result` 永不落盘 → 消息尾部看起来像
    /// 「待决问答」，实则早已不在场。若新门对 claude 也生效，其后落在 40 条窗口内的
    /// **真审批**会被静默压成 `available=false, reason=null`（前端自隐：用户既看不到
    /// 按钮也看不到提示）——本用例锁住「claude 不受该门影响」。
    ///
    /// 夹具：sess_ar（claude，Waiting）+ last_message = claude 审批 marker 原文 +
    /// message_source 返回「AUQ tool-call 无 tool-result」的尾部。
    /// 断言：审批 GET **available=true**（照常出选项），问答 GET 仍走既有通道 B。
    #[tokio::test]
    async fn claude_tail_question_does_not_suppress_approve() {
        let fake = FakeInjector::ok();
        let page = crate::remote::content::MessagesPage {
            messages: vec![auq_tool_call(0, Q_SINGLE_PAYLOAD)],
            truncated: false,
        };
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                if sid == "sess_ar" {
                    Ok(page.clone())
                } else {
                    Err("无消息".to_string())
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_ar",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], true,
            "claude 走既有 question_marked 隔离路径：尾部 AUQ 形态（纯文本作答、无 result）\
             不得把真审批压成不可用（F2-2 收窄）"
        );
        assert!(
            !v["options"].as_array().unwrap().is_empty(),
            "claude 可批时必须下发选项（不被静默压制）"
        );
    }

    /// 通道 B（兜底）：sess_ai（Processing、无标记）message_source 注入「user 消息 +
    /// 已落盘 AUQ tool-call（无 tool-result）」→ GET available=true source="scan" +
    /// questions 可解析；POST select → (pid=44, "1") 出键。
    #[tokio::test]
    async fn question_channel_b_scan_available_and_answerable() {
        let fake = FakeInjector::ok();
        let page = crate::remote::content::MessagesPage {
            messages: vec![user_msg(0), auq_tool_call(1, Q_SINGLE_PAYLOAD)],
            truncated: false,
        };
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                if sid == "sess_ai" {
                    Ok(page.clone())
                } else {
                    Err("无消息".to_string())
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ai",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true, "未答 AUQ tool-call 在场 → 可用");
        assert_eq!(v["source"], "scan", "通道 B 识别口径");
        assert_eq!(v["questions"][0]["header"], "Next step");
        // POST select：通道 B 会话照常出手
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_ai","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("\"status\":\"key_sent\""));
        assert_eq!(
            fake.recorded_keys(),
            vec![(44u32, "1".to_string())],
            "通道 B select 出键：{:?}",
            fake.recorded_keys()
        );
    }

    /// 通道 B 答完判据（可行口径）：tool-call 之后任何 tool-result 在场 → 不再可用。
    /// sess_aj 消息 = AUQ tool-call + tool_result → GET available=false、POST 409。
    #[tokio::test]
    async fn question_channel_b_not_available_after_tool_result() {
        let fake = FakeInjector::ok();
        let page = crate::remote::content::MessagesPage {
            messages: vec![
                auq_tool_call(0, Q_SINGLE_PAYLOAD),
                tool_result_msg(1, "Your questions have been answered: \"q\"=\"Tool demo\"."),
            ],
            truncated: false,
        };
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                if sid == "sess_aj" {
                    Ok(page.clone())
                } else {
                    Err("无消息".to_string())
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_aj",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "tool_result 在场 = 已答，不再可用（可行口径：任何后随 tool-result 即判答完）"
        );
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_aj","action":"cancel"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("no_question"));
        assert!(fake.recorded_keys().is_empty());
    }

    // ==== 丁T2：计划双卡 / 计划待确认 / kimi 审批互斥（问题 3/4/10/12）====
    // 夹具族：在 question_state_with_msgs 的会话清单上再加 sess_as..sess_ax（全测试集
    // 唯一 id，守卫 id 立规）。message_source 按 sid 分派返回各用例的消息页。

    /// 计划类消息条目（`kind="plan"`——claude/codex/kimi 三家的正文卡在消息层同构）
    fn plan_msg(seq: i64, content: &str) -> crate::remote::content::SessionMessage {
        crate::remote::content::SessionMessage {
            seq,
            role: "assistant".into(),
            kind: "plan".into(),
            content: content.into(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: false,
        }
    }

    /// 计划文件卡条目（`kind="plan-file"`，content=路径）
    fn plan_file_msg(seq: i64, path: &str) -> crate::remote::content::SessionMessage {
        crate::remote::content::SessionMessage {
            seq,
            role: "assistant".into(),
            kind: "plan-file".into(),
            content: path.into(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: false,
        }
    }

    /// codex 实机消息尾（17:19 rollout 缩录）：<proposed_plan> 升格而成的 plan 卡之后
    /// **无用户消息、无工具事件** = 计划待确认预期态在场。
    fn codex_plan_pending_page() -> crate::remote::content::MessagesPage {
        crate::remote::content::MessagesPage {
            messages: vec![
                user_msg(0),
                plan_msg(1, "# 《末班车》情感救赎版改写方案\n\n## Summary\n改写重点…"),
            ],
            truncated: false,
        }
    }

    /// 计划之后有用户消息（"Implement the plan." 实机形态）→ 预期态清除
    fn codex_plan_consumed_page() -> crate::remote::content::MessagesPage {
        crate::remote::content::MessagesPage {
            messages: vec![user_msg(0), plan_msg(1, "# 计划"), user_msg(2), user_msg(3)],
            truncated: false,
        }
    }

    /// 计划之后有工具事件（kimi 批准后 wire 立即落 ExitPlanMode 的 tool.call/result）
    fn plan_then_tool_event_page() -> crate::remote::content::MessagesPage {
        crate::remote::content::MessagesPage {
            messages: vec![
                user_msg(0),
                plan_msg(1, "# 计划"),
                plan_file_msg(2, "C:/u/.kimi-code/sessions/s/agents/main/plans/a.md"),
                crate::remote::content::SessionMessage {
                    seq: 3,
                    role: "assistant".into(),
                    kind: "tool-call".into(),
                    content: "ExitPlanMode".into(),
                    ts: None,
                    tool_name: Some("ExitPlanMode".into()),
                    tool_args: Some("{}".into()),
                    collapsed: true,
                },
                tool_result_msg(4, "Exited plan mode."),
            ],
            truncated: false,
        }
    }

    /// codex 计划待确认全链（问题 4）：**非 Waiting**（Processing，实机 codex 不落红）
    /// + 尾部计划提案 → 审批端点门放宽到「计划预期态」：
    /// - GET available=true、`planPending=true`、options 空（y/esc 是补丁审批键位，对
    ///   计划框未取证——不得下发）、plan 正文聚合（T8 机制，前端渲染计划全文）；
    /// - 计划之后有用户消息 → 预期态消失 → available=false（任务书「下一个用户消息
    ///   注入即清除」，无新存储）。
    #[tokio::test]
    async fn codex_plan_pending_opens_approve_gate() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                "sess_as" => Ok(codex_plan_pending_page()),
                "sess_at" => Ok(codex_plan_consumed_page()),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // sess_as：Processing + 尾部计划提案 → 门放宽
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_as",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], true,
            "计划预期态必须打开审批端点门（否则详情页无从「检查终端对话框」）"
        );
        assert_eq!(
            v["planPending"], true,
            "预期态随载荷下发（前端提示条的数据源）"
        );
        assert!(
            v["options"].as_array().unwrap().is_empty(),
            "计划框未读到屏读选项时不得下发映射表键位（y/esc 是补丁审批键，未取证）"
        );
        assert_eq!(
            v["plan"]["content"], "# 《末班车》情感救赎版改写方案\n\n## Summary\n改写重点…",
            "计划正文聚合（T8 机制——点检查前用户先看到计划全文）"
        );
        assert_eq!(v["plan"]["isFile"], false);
        // sess_at：计划之后有用户消息 → 预期态清除
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_at",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "计划后已有用户消息（Implement the plan.）→ 预期态清除（无新存储，尾部派生）"
        );
        assert_eq!(v["planPending"], false);
        assert!(fake.recorded_keys().is_empty(), "本用例不触任何注入路径");
    }

    /// kimi 计划审批卡（问题 3 主体）：
    /// - Waiting（实机 `interaction.request → Waiting` 红灯）+ 尾部 plan 卡（正文+文件卡）
    ///   → 门过、`planPending=true`；
    /// - **Windows 屏读拿到选项时**走 `dialog:<n>` N 选项（NavigateConfirm 键序）；
    /// - **屏读不到选项时**（CI / 非 Windows / 对话框未绘制）→ kimi 专属两态收敛：
    ///   **有计划预期态 → `available=true` + 零 options + `planPending=true`**（前端
    ///   「计划待确认」条 +「检查终端对话框」按钮——与 codex 同形态）；
    ///   无预期态（带标记的 Write/command 审批）→ `available=false` + reason 提示条。
    ///   **两态都绝不下发映射表键位**（R1-1 证伪 kimi 数字通道，默认表 options 恒空）。
    /// - **计划之后有工具事件**（批准后 wire 落 ExitPlanMode tool.call/result）→ 预期态
    ///   清除 → available=false（否则批准后卡片一直挂着）。
    ///
    /// 屏读依赖真实窗口（本测试进程无 conhost 目标）——故分支断言「屏读命中 vs 未命中」
    /// 两种合法形态，两者都必须满足「零映射键」这条硬约束。
    #[tokio::test]
    async fn kimi_plan_approval_card_never_emits_mapping_keys() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                "sess_au" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![
                        user_msg(0),
                        plan_msg(1, "# Plan: Create hi.txt\n\n## Goal\nCreate `hi.txt`."),
                        plan_file_msg(
                            2,
                            "C:/u/.kimi-code/sessions/wd_x/session_y/agents/main/plans/p.md",
                        ),
                    ],
                    truncated: false,
                }),
                "sess_av" => Ok(plan_then_tool_event_page()),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_au",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["verifiedWith"], KIMI_NEVER_TESTED,
            "kimi 映射已入表（版本随实机）"
        );
        assert_eq!(v["planPending"], true, "尾部署名计划 ⇒ 预期态在场");
        // F3-4：**kimi 审批卡一律不带 plan 正文**（任务书成文要求，两种形态都成立）
        assert_eq!(
            v["plan"],
            serde_json::Value::Null,
            "kimi 审批卡不得下发 plan 正文（F3-4；正文归 §2.2 的 kimi 正文卡）"
        );
        let options = v["options"].as_array().unwrap();
        if v["dialog"] == true {
            // 形态一：屏读命中（有真实窗口的机器）——选项全是 dialog:<n>
            assert!(!options.is_empty(), "dialog 模式下选项非空");
            for o in options {
                assert!(
                    o["id"].as_str().unwrap().starts_with("dialog:"),
                    "选项只能来自屏读（dialog:<n>）：{o}"
                );
            }
        } else {
            // 形态二：屏读未命中（CI / 非 Windows / 对话框未绘制）——「计划待确认」条：
            // available=true（这是**可操作**的卡：点检查重试）+ 零 options（无键可发）。
            // available=false + reason 是**无计划预期态**的降级
            // （见 kimi_approval_without_plan_falls_back_to_hint），两态在此分界。
            assert!(
                options.is_empty(),
                "kimi 屏读失败不得下发任何映射键位（R1-1 数字通道不可依赖）"
            );
            assert!(
                v["reason"].is_null(),
                "计划待确认条不走严格档 reason 通道（reason 是 available=false 的）"
            );
        }
        // sess_av：计划之后有工具事件 → 预期态清除（批准后卡片不得挂死）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_av",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["planPending"], false,
            "计划之后已有工具事件（批准后 wire 落 ExitPlanMode 回执）→ 预期态清除"
        );
        assert!(fake.recorded_keys().is_empty(), "本用例不触任何注入路径");
    }

    /// kimi 审批的**另一态**：命中审批但**无计划预期态**（带标记的 Write/command 审批
    /// ——R1-3 实测 kimi 有 `▶ Write this file?` 四选项工具批准框）→ 屏读不到选项时
    /// 不给「计划待确认」条（那不是计划），走**降级提示条**（available=false + reason）：
    /// 无键可发（R1-1），指引去终端（§2.8）。
    ///
    /// 夹具：sess_ba = kimi Waiting + **审批等待标记**（走标记路径跳过 detect）+ 消息页
    /// 无计划（历史工具事件）→ 预期态不成立。
    #[tokio::test]
    async fn kimi_approval_without_plan_falls_back_to_hint() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                // 无计划类消息（纯工具往返）→ 预期态不成立
                "sess_ba" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![
                        user_msg(0),
                        crate::remote::content::SessionMessage {
                            seq: 1,
                            role: "assistant".into(),
                            kind: "tool-call".into(),
                            content: "Write".into(),
                            ts: None,
                            tool_name: Some("Write".into()),
                            tool_args: Some(r#"{"path":"hi.txt"}"#.into()),
                            collapsed: true,
                        },
                    ],
                    truncated: false,
                }),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::approval_wait::mark(conn, "kimi", "sess_ba", 1_000, "工具审批")
        });
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_ba",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        if v["available"] == true {
            // 屏读命中（真实窗口机器）：dialog 选项齐即可（本用例只锁「无计划不误导」）
            assert_eq!(v["dialog"], true);
            assert_eq!(v["planPending"], false, "无计划消息 → 预期态不成立");
            assert!(v["plan"].is_null(), "无计划消息 → 不下发 plan 主体");
        } else {
            assert_eq!(
                v["planPending"], false,
                "无计划预期态 → 不得渲染「计划待确认」条（那不是计划）"
            );
            assert!(
                v["reason"].as_str().is_some_and(|s| !s.is_empty()),
                "无计划预期态的降级必须给中文提示条（available=false + reason）"
            );
            assert!(
                v["options"].as_array().unwrap().is_empty(),
                "无键可发（R1-1）——提示条形态零选项"
            );
        }
        assert!(fake.recorded_keys().is_empty());
    }

    // ==== 丁T2 复评修复（F3-2/F3-3/F3-4/F3-5）====

    /// **F3-2（Critical）**：`POST /session-approve` 的门必须与 GET **同口径**——
    /// codex 计划待确认的真机状态是 **Idle + 无标记**，只放宽 GET 会让卡片挂上后
    /// **点 N 选项必回 409 not_waiting**（用户点了等于没点）。
    ///
    /// 夹具：sess_bb = codex **Idle** + 尾部计划提案（真机形态）。
    /// 断言：GET available=true + planPending；POST `dialog:1` 直达投递（200 key_sent
    /// + 注入器收到 VK 数字 '1'——本测试进程无真实窗口，屏读恒失败，故用**映射表键位
    /// 不可用**的 codex 计划形态……见下注）。
    ///
    /// **注（为什么断言「投递被尝试」而非具体键）**：计划框的真实交互依赖屏读
    /// （Windows 可见窗口），CI 无窗口 → `dialog_options=None` → 选项为空；故本用例
    /// 断言的是 **POST 越过了 not_waiting 门**（错误码不再是 not_waiting，而是走到
    /// 映射/选项层），这才是 F3-2 的缺陷面。键序列投递的成功路径由既有
    /// `approve_sends_key`（Waiting 会话）与 `dialog:<n>` 用例覆盖。
    #[tokio::test]
    async fn codex_plan_pending_opens_approve_post_gate() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                "sess_bb" => Ok(codex_plan_pending_page()),
                "sess_bc" => Ok(codex_plan_consumed_page()),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 前置：GET 可用（与既有 codex 用例一致）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_bb",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true, "GET 门（既有）");
        // **F3-2 核心断言 + N1 收口**：POST 映射表 id → 必须**越过 not_waiting 门**
        // （否则 F3-2 未修），但被 N1 拒为 no_mapping（404）——两个错误码的差异正是
        // 「门已放宽 ∧ 输入面已收窄」的判据：not_waiting 来自门之前，no_mapping 来自
        // 门之后。**零注入**是 N1 的核心（修前该组合注入 `y`）。
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bb","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        let status = r.status();
        let body = body_string(r).await;
        assert!(
            !body.contains("not_waiting"),
            "F3-2：计划预期态下 POST 必须越过 not_waiting 门（实得 {body}）"
        );
        assert_eq!(
            status, 404,
            "N1：越过门后映射表 id 必须被拒（no_mapping；修前是 200 key_sent + 注入 y）：{body}"
        );
        assert!(body.contains("no_mapping"), "{body}");
        assert!(
            fake.recorded_keys().is_empty(),
            "N1：计划预期态下映射键位零注入（修前实测注入 (63,\"y\")）：{:?}",
            fake.recorded_keys()
        );
        // 反向锁：尾无计划（计划已被用户消息消费）→ POST 照旧 409 not_waiting
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bc","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            409,
            "反向锁：codex 的 Idle 本身不等于放行——尾部无可确认计划时 POST 仍须 409"
        );
        assert!(body_string(r).await.contains("not_waiting"));
        assert!(
            fake.recorded_keys().is_empty(),
            "反向锁：全用例零注入（409 与 404 两条路径都不出手）"
        );
    }

    /// **N1（复审 Important）**：计划预期态下 POST **只放行 `dialog:<n>`**。
    ///
    /// 端点上两种拒绝都收敛成 `no_mapping`，故本用例锁的是**「映射 id 被拒 + 零注入」**
    /// 这一半（可观测面）；另一半「`dialog:<n>` 仍被接受」由纯函数单测
    /// `plan_pending_post_accepts_dialog_options_only` 锁住——**CI 无法观测 dialog 的
    /// 通过路径**（屏读依赖真实可见窗口：非 Windows 恒 None，Windows 上假 pid 的
    /// AttachConsole 必失败），如实申报。
    ///
    /// 对照面（kimi 同锁）：kimi 的计划审批同样在预期态下——其映射表 options 本就为空，
    /// 但 KV 定制可能给出非空表，故 N1 的收敛点在 kimi 上同样成立（本用例用 codex，
    /// 因为它的默认表**有** `approve="y"`，正是插桩实测命中的键）。
    #[tokio::test]
    async fn plan_pending_post_rejects_mapped_option_ids() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                "sess_bg" => Ok(codex_plan_pending_page()),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 逐一遍历默认表全部 id + 若干域外 id：预期态下一律 no_mapping + 零注入
        for option_id in [
            "approve",
            "reject",
            "bogus",
            "y",
            "esc",
            "dialog",
            "dialogx:1",
        ] {
            let r = app
                .clone()
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-approve",
                    Some("mam_device=mm"),
                    Some(&format!(
                        r#"{{"sessionId":"sess_bg","optionId":"{option_id}"}}"#
                    )),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 404, "{option_id}：预期态必须拒为 no_mapping");
            assert!(body_string(r).await.contains("no_mapping"), "{option_id}");
        }
        assert!(
            fake.recorded_keys().is_empty(),
            "N1：预期态下任何映射 id 都零注入：{:?}",
            fake.recorded_keys()
        );
    }

    /// **N1 反向锁**：`Waiting` 态的**既有映射键路径零回归**——同一端点、同一会话族，
    /// 状态换成 Waiting（或带审批标记）时映射表 id 照常投递。
    ///
    /// 用 sess_bh（claude Waiting + 审批标记）——它的映射表
    /// `approve="1"` 是 M8R 实证键位，POST 必须 200 key_sent 且注入 `1`。
    #[tokio::test]
    async fn waiting_state_post_still_delivers_mapped_keys() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                "sess_bh" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![user_msg(0), plan_msg(1, "# 执行计划")],
                    truncated: false,
                }),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::approval_wait::mark(conn, "claude", "sess_bh", 1_000, "计划批准")
        });
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bh","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(body_string(r).await.contains("key_sent"));
        assert_eq!(
            fake.recorded_keys(),
            vec![(69u32, "1".to_string())],
            "N1 反向锁：非预期态（标记路径）的映射键位路径零回归"
        );
    }

    /// **N3（复审 Minor）**：F3-2 的**短路序/零 IO 口径**必须有回归锁——复审实测
    /// 「删掉 `&& !marked && status != Waiting` 两个守卫（变成无条件读页）」时**全套 lib
    /// 仍全绿**，即成本口径无保护（代码与注释都对，但将来有人调 `&&` 顺序不会红）。
    ///
    /// 修法：用**计数 message_source** 断言读页次数——这是本条口径唯一可观测的面。
    /// 五个场景覆盖全部短路臂与唯一的价值臂：
    /// - **Waiting ∧ 计划族**（sess_bi：codex Waiting）→ 门由状态满足 → **零读页**；
    /// - **标记态 ∧ 计划族**（sess_bj：codex Idle + 审批标记）→ 门由标记满足 → **零读页**；
    /// - **非计划族**（sess_bk：claude Idle + 尾部计划）→ 族收窄 → **零读页**；
    /// - **计划族 ∧ Idle ∧ 无标记 ∧ 尾部有计划**（sess_bb）→ 读**恰好一次**（价值臂）；
    /// - **计划族 ∧ Idle ∧ 无标记 ∧ 尾部无计划**（sess_bc）→ 读恰好一次后判 false
    ///   → 409（确认「读一次就够」）。
    ///
    /// **计数语义**：计数 `message_source` 的**调用次数**（不是成功次数）——桩返回 Err
    /// 也计数（`ok()` 的失败路径同样是一次调用尝试）。用 Err 桩让用例不依赖消息内容，
    /// 把断言收敛到「读没读」这一个自由度；需要「尾部有计划」的两例才给真页。
    #[tokio::test]
    async fn post_plan_pending_read_page_only_when_needed() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;

        let fake = FakeInjector::ok();
        let calls = StdArc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |_, sid: &str, _| {
                counter.fetch_add(1, Ordering::SeqCst);
                // 只有「尾部有计划」的两个 id 给真页，其余给 Err（Err = 判据不成立）
                match sid {
                    "sess_bb" => Ok(codex_plan_pending_page()),
                    "sess_bc" => Ok(codex_plan_consumed_page()),
                    _ => Err("计数桩：不提供内容".to_string()),
                }
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());

        // ① Waiting ∧ 计划族（codex）→ 状态门满足，`||` 短路在读页之前
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bi","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        let s1 = r.status();
        assert_ne!(s1, 404, "Waiting 态映射存在 → 不该 no_mapping");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "N3：Waiting 态满足门 → 零读页（`status != Waiting` 守卫）"
        );

        // ② 标记态 ∧ 计划族（codex Idle + 审批标记）→ 门由标记满足
        state.store.with(|conn| {
            crate::database::dao::approval_wait::mark(conn, "codex", "sess_bj", 1_000, "工具审批")
        });
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bj","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_ne!(r.status(), 409, "标记态不该 409 not_waiting");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "N3：标记态满足门 → 零读页（`!marked` 守卫）"
        );

        // ③ 非计划族（claude Idle + 尾部计划）→ 族收窄，**恒不读页**
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bk","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "非计划族 + 非 Waiting → 既有 409 口径");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "N3：非计划族工具恒不读页（`plan_dialog_family` 守卫——claude 零额外 IO）"
        );

        // ④ **价值臂**：计划族 ∧ Idle ∧ 无标记 ∧ 尾部有计划 → 读**恰好一次**并放行
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bb","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            404,
            "预期态下映射 id 被 N1 拒（此处只验读页次数）"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "N3 价值臂：唯一需要读页的形态 → 恰好一次（F3-2 的用武之地）"
        );

        // ⑤ 计划族 ∧ Idle ∧ 无标记 ∧ 尾部无计划 → 读一次后判 false → 409（不重读）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_bc","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "N3：尾部无计划的计划族会话 → 读一次即判 false（不重读）"
        );
    }

    /// **丁T2 互斥（无标记路径）**：kimi 的审批在场 → 问答卡不可用（硬约束① 扩到
    /// kimi 新卡）。
    ///
    /// **可达态说明（为什么不构造「计划 + 待决 AUQ」）**：kimi 的交互是**阻塞式**——
    /// 问答未答完时模型不可能提出计划，故「尾部计划提案 ∧ 尾部待决 AUQ」在真实 wire
    /// 里不可达（真实序列：AUQ tool.call → interaction.request → resolved → tool.result；
    /// 计划审批序列：Write plan → interaction.request(plan_review) → resolved →
    /// ExitPlanMode）。**真实可达的错位双卡**来自**陈旧问答标记**：kimi 的
    /// `question_wait_marks` 行靠清除事件删除，MAM 未运行/清除事件丢失时会残留
    /// （`mam.db` 现状即 `question_wait_marks` 表空、kimi 无标记通道的实证背景）——
    /// 此时**批准在等计划确认、问答卡却按陈旧标记冒出来**，正是本门要拦的形态。
    ///
    /// 夹具：kimi 会话 + 播种问答标记（通道 A）+ 尾部计划提案（预期态）→ 问答必须
    /// 不可用、审批照常可用；反向锁：尾部计划之后有工具事件（审批窗口已关）→ 问答
    /// 照常可用（陈旧标记仍走既有通道 A —— 本门**不**扩大压制面）。
    #[tokio::test]
    async fn kimi_plan_pending_blocks_question_card() {
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(|_, sid: &str, _| match sid {
                // 审批在场：尾部是计划提案（其前可能有历史工具事件，不影响判据）
                "sess_aw" => Ok(crate::remote::content::MessagesPage {
                    messages: vec![
                        user_msg(0),
                        plan_msg(1, "# 上一版计划"),
                        tool_result_msg(2, "Wrote 400 bytes to …/plans/a.md"),
                        plan_msg(3, "# 新版计划（审批中）"),
                    ],
                    truncated: false,
                }),
                // 审批已关：计划之后有工具事件（ExitPlanMode 回执）→ 预期态清除
                "sess_ax" => Ok(plan_then_tool_event_page()),
                _ => Err("无消息".to_string()),
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        // 陈旧问答标记（通道 A 形态）：两会话都播——sess_aw 应被预期态门拦下，
        // sess_ax 应照常出卡（反向锁：不扩大压制面）
        for sid in ["sess_aw", "sess_ax"] {
            state.store.with(|conn| {
                crate::database::dao::question_wait::mark(
                    conn,
                    "kimi",
                    sid,
                    1_000,
                    "等待回答",
                    Some(KIMI_Q_PAYLOAD),
                )
            });
        }
        let app = router(state.clone());
        // sess_aw：kimi 审批在场（尾部计划）→ 问答不可用（陈旧标记被压）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_aw",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "kimi 审批在场（尾部计划提案）→ 问答卡不可用（硬约束① 扩面，无标记路径）"
        );
        assert!(v["questions"].as_array().unwrap().is_empty());
        // 问答 POST 同源拒绝（双卡错位的注入面同样关死）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-question/answer",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_aw","action":"select","index":0}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        assert!(body_string(r).await.contains("no_question"));
        assert!(fake.recorded_keys().is_empty(), "问答键零出手");
        // **审批侧不在本用例断言（如实申报）**：播种的陈旧问答标记会先撞上既有
        // 硬约束①（`question_marked → 审批不可用`，批次乙 T8 的隔离规则），故本夹具下
        // 审批也判不可用。**该优先级未动**（本轮不改批次丙的隔离裁决）——代价是
        // 「陈旧标记 + 计划待确认」时两张卡都不出；受益面是「真问答在场时审批卡绝不
        // 误出」的安全面保持原样。审批侧可用性由
        // `kimi_plan_approval_card_never_emits_mapping_keys`（无标记形态）覆盖。
        //
        // sess_ax 反向锁：审批窗口已关 → 问答照常可用（陈旧标记走既有通道 A）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ax",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], true,
            "审批窗口已关（计划后有工具事件）→ 问答照常可用（门不扩大压制面）"
        );
        assert_eq!(v["source"], "mark");
    }

    /// **丁T1 复评 F-3 端到端**：opencode 的问答销卡信号——走**真实 reader**
    /// （`read_opencode_messages_impl` + tempdir 内的 opencode.db），不复刻 reader 产物。
    ///
    /// 根因回顾：opencode reader 原先只产 tool-call 不产 tool-result → 问答端点的
    /// 「其后无 tool-result」判据恒真 → **已答完的问题仍 available=true，卡片不消失**。
    /// 修法：reader 对问答类部件的终态（completed/error）补一条 tool-result。
    ///
    /// 夹具（2026-09-21 本机 part 表实测形态）：sess_ap = running（待决）、
    /// sess_aq = completed + metadata.answers（已答）。message_source 经
    /// `read_session_messages_impl` 指向 tempdir home（零接触真实 ~/.local）。
    #[tokio::test]
    async fn opencode_question_availability_follows_part_status() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join(".local/share/opencode/opencode.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (message_id TEXT, session_id TEXT, data TEXT, time_created INTEGER);",
        )
        .unwrap();
        let q_input = r#"{"questions":[{"header":"Build output folder","question":"Which folder?","options":[{"label":"dist","description":"d"},{"label":"out","description":"o"}]}]}"#;
        for (ses, mid, status, extra) in [
            ("sess_ap", "m_ap", "running", String::new()),
            (
                "sess_aq",
                "m_aq",
                "completed",
                r#","metadata":{"answers":[["out"]],"truncated":false},"output":"{\"answers\":[[\"out\"]]}""#
                    .to_string(),
            ),
        ] {
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, 100, '{\"role\":\"assistant\"}')",
                rusqlite::params![mid, ses],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO part (message_id, session_id, data, time_created) VALUES (?1, ?2, ?3, 1783326720870)",
                rusqlite::params![
                    mid,
                    ses,
                    format!(
                        r#"{{"type":"tool","tool":"question","state":{{"status":"{status}","input":{q_input}{extra}}}}}"#
                    )
                ],
            )
            .unwrap();
        }
        drop(conn);

        // 会话夹具：question_state 已含 sess_ap/aq（Waiting），此处把 pid 对上；
        // message_source 走真实 reader（home = tempdir）
        let home = tmp.path().to_path_buf();
        let fake = FakeInjector::ok();
        let state = question_state_with_msgs(
            fake.clone(),
            Box::new(move |tool: &str, sid: &str, limit: usize| {
                crate::remote::content::read_session_messages_impl(
                    &home, None, None, tool, sid, limit,
                )
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());

        // 待决（running）→ available=true（卡该出，且不得被自己误销）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ap",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], true,
            "待决的 opencode 问答必须可用（待决窗口内不得被误销）"
        );
        assert_eq!(v["source"], "scan");

        // 已答（completed + answers）→ available=false（销卡信号生效）
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_aq",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "已答的 opencode 问答必须不可用（F-3：reader 补的 tool-result 销卡信号）"
        );
        // 已答 → 审批侧也不被问答压制（硬约束① 的反向：已答不再构成「问答在场」）
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_aq",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        // opencode 不在默认映射表 → 无映射 → available=false（但不是被问答压的）；
        // 关键断言是它不 panic 且路径可达（映射缺失与问答隔离是两根轴）
        assert_eq!(v["reason"], serde_json::Value::Null);
        assert!(fake.recorded_keys().is_empty(), "本用例不触注入");
    }
    #[tokio::test]
    async fn question_mark_bad_payload_falls_through() {
        let fake = FakeInjector::ok();
        let state = question_state(fake.clone());
        persist_named_device(&state, "mm", "测试设备");
        state.store.with(|conn| {
            crate::database::dao::question_wait::mark(
                conn,
                "claude",
                "sess_ab",
                1_000,
                "等待回答",
                Some("not-json"),
            )
        });
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-question?session_id=sess_ab",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            v["available"], false,
            "标记载荷损坏且通道 B 未命中 → 不可用（不误报）"
        );
    }

    /// P3 审计动作词表（Task 7 P3c）：KV 定制映射含域外 id=other 的选项 → POST
    /// session-approve 照发键位（x）→ 审计 action 收敛为 "key"（W5 词表 send|queue|
    /// flush|jump|retract|approve|reject|fail|key|open——open 已随 Task 11 一键
    /// resume 兑现，批次乙 T8 再追加 answer（问答端点，锁定见 question_answer_* 族），
    /// 批次丙 T6 追加 mode，**丁T3 追加 slash**（裁2 斜杠命令裸注入的溯源动作，
    /// 锁定见 `slash_message_bare_injects_and_audits_slash` / `slash_message_queued_keeps_bare_form_and_slash_action` 两用例）——
    /// 之外的域外 id 不得原样进审计
    /// action 列——key 是本次新增的收敛动作）且不 panic；域外 warn 在实现侧 log，
    /// 测试不断言日志。
    /// KV 经内存库 seed（DeviceStore 缝，零接触真实 ~/.mam）；sess_j 全测试集唯一
    /// id（守卫 id 立规）。前端 AuditLogSection「action 原样小写展示」契约不受影响。
    #[tokio::test]
    async fn audit_action_vocab() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        // 定制映射 seed 进本测试自己的内存库（其他测试的 memory 库互不可见，零互染）
        state.store.with(|c| {
            crate::database::dao::settings::set_setting_conn(
                c,
                crate::inject::approve::KV_KEY,
                r#"[{"tool":"claude","verified_with":"2.1.251",
  "prompt_markers":["do you want to proceed"],
  "options":[{"id":"approve","label":"允许","key":"1"},
             {"id":"reject","label":"拒绝","key":"esc"},
             {"id":"other","label":"其他","key":"x"}]}]"#,
            )
        });
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_j","optionId":"other"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"key_sent\""),
            "域外 id 命中定制映射照发键位：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(20u32, "x".to_string())],
            "域外 id 选项的键位照映射投递"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].action, "key", "域外 id 审计 action 收敛为 key");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].session_id, "sess_j");
    }

    /// 严格档（M9R Task 10 裁决：未取证不出键，probe-pending 恒判漂移）：KV seed
    /// verified_with="probe-pending" 的 codex 定制映射（复用 Task 7 audit_action_vocab 的
    /// store.with 内存库 seed 模式，零接触真实 ~/.mam）→
    /// - GET session-approve-options：available=false + options 空 + reason=「键位待实测确认，
    ///   请用普通发送」（前端 ApproveCard 契约：available=false 且带 reason → 只渲染提示条）
    ///   ——即使 sess_k 处 Waiting 且 last_message 命中 marker；
    /// - POST session-approve：404 no_mapping（未取证=映射缺失，降级走普通发送），键位
    ///   永不出手。
    /// sess_k 全测试集唯一 id（守卫 id 立规）。
    #[tokio::test]
    async fn probe_pending_strict_policy() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        // probe-pending 定制映射 seed 进本测试自己的内存库（其他测试的 memory 库互不可见）
        state.store.with(|c| {
            crate::database::dao::settings::set_setting_conn(
                c,
                crate::inject::approve::KV_KEY,
                r#"[{"tool":"codex","verified_with":"probe-pending",
  "prompt_markers":["would you like to make the following"],
  "options":[{"id":"approve","label":"允许","key":"y"},
             {"id":"reject","label":"拒绝","key":"esc"}]}]"#,
            )
        });
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // GET：Waiting + detect 命中（sess_k last_message 即 codex 弹框标题原文）仍压为不可批
        let r = app
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_k",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false, "probe-pending 严格档恒不可批");
        assert!(
            v["options"].as_array().unwrap().is_empty(),
            "严格档不下发选项（键位与选项均不出键）"
        );
        assert_eq!(
            v["reason"], "键位待实测确认，请用普通发送",
            "严格档必须下发降级原因（前端提示条渲染契约）"
        );
        assert_eq!(v["verifiedWith"], "probe-pending");
        assert_eq!(v["drift"], true, "probe-pending 恒判漂移");
        // POST：404 no_mapping（未取证=映射缺失），零按键投递
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-approve",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_k","optionId":"approve"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_mapping"));
        assert!(fake.recorded_keys().is_empty(), "严格档不得有任何按键投递");
    }

    /// 质量评审补锁：非严格不可批三形态（非 Waiting / detect 未命中 / 无映射）的
    /// reason 恒为 null——锁定前端 ApproveCard 契约「available=false 且无 reason →
    /// 卡自隐」（reason 只属严格档 probe-pending 语义，不得挪作普通不可批提示）。
    #[tokio::test]
    async fn non_strict_unavailable_reason_is_null() {
        let state_hit = approve_state(FakeInjector::ok(), Some(APPROVE_HIT_MSG));
        persist_named_device(&state_hit, "mm", "测试设备");
        let app_hit = router(state_hit);
        // 非 Waiting（sess_b Processing）→ reason null
        let r = app_hit
            .clone()
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_b",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false);
        assert_eq!(
            v["reason"],
            serde_json::Value::Null,
            "非 Waiting 不得带 reason（自隐契约）"
        );
        // 无映射（sess_d zcode）→ reason null
        let r = app_hit
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_d",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false);
        assert_eq!(
            v["reason"],
            serde_json::Value::Null,
            "无映射不得带 reason（自隐契约）"
        );
        // detect 未命中（state_miss：sess_a last_message="无关"）与无 last_message
        // （sess_e 夹具原样恒 None）→ reason null
        let state_miss = approve_state(FakeInjector::ok(), Some("无关"));
        persist_named_device(&state_miss, "mm", "测试设备");
        let app_miss = router(state_miss);
        for sid in ["sess_a", "sess_e"] {
            let r = app_miss
                .clone()
                .oneshot(req(
                    "GET",
                    &format!("/m/api/v1/session-approve-options?session_id={sid}"),
                    Some("mam_device=mm"),
                    None,
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200, "{sid}");
            let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
            assert_eq!(v["available"], false, "{sid}");
            assert_eq!(
                v["reason"],
                serde_json::Value::Null,
                "{sid} 不可批不得带 reason（自隐契约）"
            );
        }
    }

    /// 质量评审补锁（现实现选定行为定格）：严格档 hint 与 detect 命中无关——probe-pending
    /// 映射 + last_message 与任何 marker 无关（sess_l）仍下发 reason。提示条语义=键位
    /// 取证状态（未取证），非「审批中」判定；防后人把严格档短路「修」到 detect 之后
    /// （那会让 detect-miss 的真审批会话完全无提示）。
    #[tokio::test]
    async fn probe_pending_hint_even_on_detect_miss() {
        let fake = FakeInjector::ok();
        let state = approve_state(fake.clone(), Some(APPROVE_HIT_MSG));
        // probe-pending 定制映射 seed 进本测试自己的内存库（与其他测试零互染）
        state.store.with(|c| {
            crate::database::dao::settings::set_setting_conn(
                c,
                crate::inject::approve::KV_KEY,
                r#"[{"tool":"codex","verified_with":"probe-pending",
  "prompt_markers":["would you like to make the following"],
  "options":[{"id":"approve","label":"允许","key":"y"}]}]"#,
            )
        });
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_l",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], false);
        assert!(
            v["options"].as_array().unwrap().is_empty(),
            "detect 未命中选项恒空"
        );
        assert_eq!(
            v["reason"], "键位待实测确认，请用普通发送",
            "detect 未命中严格档 hint 仍下发（短路序定格）"
        );
        assert!(fake.recorded_keys().is_empty(), "查询端点零按键投递");
    }

    // ==== M6R–M9R Task 11：session-open 端点（R5 一键 resume）====
    // 零污染 + 零真开窗：spawn 缝（RemoteState.resume_spawner）注入记录型假
    // spawner；会话快照/设备表/审计全走注入缝与内存库，不触真实 ~/.mam。

    /// Task 11 记录型假 spawner：克隆记录 SpawnSpec 不真开窗（R5 硬约束：
    /// spawner 缝使端点测试不真开窗；Windows 实开窗验证归用户手工/后续验收）
    struct RecordingSpawner(
        std::sync::Arc<std::sync::Mutex<Vec<crate::inject::resume::SpawnSpec>>>,
    );

    impl RecordingSpawner {
        fn new() -> Self {
            Self(std::sync::Arc::new(std::sync::Mutex::new(Vec::new())))
        }
        fn seam(&self) -> std::sync::Arc<crate::inject::resume::SpawnFn> {
            let log = self.0.clone();
            std::sync::Arc::new(move |spec: &crate::inject::resume::SpawnSpec| {
                log.lock().unwrap().push(spec.clone());
                Ok(())
            })
        }
        /// 评审 I3：恒败 spawner（照常记录后报 Err）——驱动 200-failed 分支测试
        fn seam_failing(
            &self,
            err: &'static str,
        ) -> std::sync::Arc<crate::inject::resume::SpawnFn> {
            let log = self.0.clone();
            std::sync::Arc::new(move |spec: &crate::inject::resume::SpawnSpec| {
                log.lock().unwrap().push(spec.clone());
                Err(err.to_string())
            })
        }
        fn recorded(&self) -> Vec<crate::inject::resume::SpawnSpec> {
            self.0.lock().unwrap().clone()
        }
    }

    /// Task 11 专用 state：会话夹具独占 id（守卫 id 立规的防串键纪律同源）——
    /// sess_m（claude Waiting 正常 cwd）/ sess_n（claude Waiting 空白 cwd）/
    /// sess_o（workbuddy Idle，未入 resume 命令表）；spawner 注入记录型假体。
    /// 其余缝与 inject_state 同口径（内存库，零接触真实 ~/.mam）。
    fn open_state(spawner: std::sync::Arc<crate::inject::resume::SpawnFn>) -> Arc<RemoteState> {
        let mut sess_m = inj_sess(
            "sess_m",
            crate::session::AgentType::Claude,
            30,
            crate::session::SessionStatus::Waiting,
        );
        sess_m.project_path = "/tmp/proj-m".into();
        let mut sess_n = inj_sess(
            "sess_n",
            crate::session::AgentType::Claude,
            31,
            crate::session::SessionStatus::Waiting,
        );
        sess_n.project_path = "  ".into();
        let sess_o = inj_sess(
            "sess_o",
            crate::session::AgentType::WorkBuddy,
            32,
            crate::session::SessionStatus::Idle,
        );
        let sessions = vec![sess_m, sess_n, sess_o];
        Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: sessions.clone(),
                total_count: sessions.len(),
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            resume_spawner: spawner,
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        })
    }

    /// 一键 resume 出手：200 opening + no-store + spawner 收到命令表产物 +
    /// 审计 action=open result=ok；无 cookie → 403（PIN gate 照旧覆盖）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_open_endpoint_opens_and_audits() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        // 无 cookie → 403（nest 内层 gate 结构性覆盖新端点）
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                None,
                Some(r#"{"sessionId":"sess_m"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "session-open 必须过 PIN 门禁");
        // 有 cookie → 200 opening + no-store
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_m"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "打开回执是门禁下私有写路径，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"opening\""),
            "一键 resume 应回执 opening：{body}"
        );
        // spawner 恰被调用一次，携带命令表产物（wt/conhost 分支的平台差异不断言）
        let recorded = spawner_rec.recorded();
        assert_eq!(recorded.len(), 1, "spawner 恰被调用一次");
        match &recorded[0] {
            crate::inject::resume::SpawnSpec::Windows { args, cwd, .. } => {
                assert!(
                    args.iter().any(|a| a == "claude --resume sess_m"),
                    "spawn 计划必须携带 claude 的 resume 命令：{:?}",
                    recorded[0]
                );
                // 评审 I1：cwd 进 spec（conhost 分支的 current_dir 消费点）
                assert_eq!(cwd, "/tmp/proj-m", "spawn 计划必须携带项目目录");
            }
            crate::inject::resume::SpawnSpec::MacosApplescript { .. } => {
                panic!("Windows 运行时不得派发 AppleScript 变体");
            }
        }
        // 审计 action=open（Task 7 预留兑现）result=ok
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "open");
        assert_eq!(audits[0].result, "ok");
        assert_eq!(audits[0].session_id, "sess_m");
        assert_eq!(audits[0].device_name, "测试设备");
        assert_eq!(audits[0].agent_type, "claude");
    }

    /// 无 cwd：404 no_cwd + spawner 不出手 + 不写审计（校验失败不落账口径）
    #[tokio::test]
    async fn session_open_endpoint_no_cwd() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_n"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert_eq!(body_string(r).await, "{\"error\":\"no_cwd\"}");
        assert!(spawner_rec.recorded().is_empty(), "无 cwd 不得出手 spawn");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "校验失败不写审计");
    }

    /// 无映射（workbuddy 未入命令表）：404 no_resume_command + spawner 不出手
    #[tokio::test]
    async fn session_open_endpoint_no_resume_command() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_o"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert_eq!(body_string(r).await, "{\"error\":\"no_resume_command\"}");
        assert!(
            spawner_rec.recorded().is_empty(),
            "未查证工具绝不出手 spawn"
        );
    }

    /// 会话不在快照：404 no_session；缺参：400（与 session-send 同口径）
    #[tokio::test]
    async fn session_open_endpoint_no_session_and_bad_request() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam());
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_zzz"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert_eq!(body_string(r).await, "{\"error\":\"no_session\"}");
        // 缺参 → 400
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":""}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        assert!(spawner_rec.recorded().is_empty());
    }

    /// 评审 I3：spawn 出手失败 → 200 {"status":"failed","error"}（HTTP 200 恒定，
    /// 语义在 body——session-approve 同口径）+ 审计 action=open result=failed: 前缀。
    /// spawner 恒败（本机 wt/conhost 两分支皆败——降级链收口后的终态）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_open_endpoint_spawn_failure_reports_failed() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam_failing("终端启动失败（模拟）"));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_m"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "失败回执走 200 语义分诊（session-approve 同口径）"
        );
        assert_eq!(
            r.headers()
                .get("cache-control")
                .and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "失败回执同为门禁下私有写路径，禁止中间层缓存"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("终端启动失败（模拟）"),
            "失败回执必须携带 status=failed 与后端错误文案：{body}"
        );
        // 出手了才谈失败：spawner 被调用（wt 在场时降级链两次、不在场一次——只断言非空）
        assert!(!spawner_rec.recorded().is_empty(), "失败回执前提是确有出手");
        // 审计 action=open result=failed: 前缀（出手失败照实落账）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "open");
        assert!(
            audits[0].result.starts_with("failed:"),
            "出手失败审计必须是 failed: 前缀：{}",
            audits[0].result
        );
        assert_eq!(audits[0].session_id, "sess_m");
    }

    /// M2（Mac 验收 D-5 根因①）：macOS osascript TCC 失败形态回执——缝注入假
    /// spawner 直接返回 [`classify_resume_error`] 的产物（分类函数本身跨平台单测
    /// 覆盖；端点在此只验回执契约）：200 failed 携带「自动化授权」指引 + 审计
    /// action=open result=failed: 前缀——账实一致，不再「open ok 但无窗」。
    /// 注：本测试在 Windows 上跑走 Windows 分支（open_session_terminal_with 按
    /// std::env::consts::OS 分派），但端点回执契约跨平台同形，与 OS 分派正交。
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_open_endpoint_macos_tcc_guidance_reports_failed() {
        let spawner_rec = RecordingSpawner::new();
        let state = open_state(spawner_rec.seam_failing(crate::inject::resume::MACOS_TCC_GUIDANCE));
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .clone()
            .oneshot(req(
                "POST",
                "/m/api/v1/session-open",
                Some("mam_device=mm"),
                Some(r#"{"sessionId":"sess_m"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            200,
            "失败回执走 200 语义分诊（session-approve 同口径）"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"status\":\"failed\"") && body.contains("自动化授权"),
            "失败回执必须携带 status=failed 与 TCC 授权指引：{body}"
        );
        assert!(!spawner_rec.recorded().is_empty(), "失败回执前提是确有出手");
        // 审计 action=open result=failed: 前缀且指引在账（出手失败照实落账）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "open");
        assert!(
            audits[0].result.starts_with("failed:") && audits[0].result.contains("自动化授权"),
            "审计必须 failed: 前缀且携带授权指引：{}",
            audits[0].result
        );
    }

    // ==== 移动端附件上传（2026-09-20）：落盘 <会话 cwd>/.mam-attachments/<会话>/ ====

    /// 附件端点测试状态：单会话、project_path 指向 tempdir（零真实目录污染）
    fn attach_state(project_path: std::path::PathBuf, empty_cwd: bool) -> Arc<RemoteState> {
        let mut session = inj_sess(
            "sess_att",
            crate::session::AgentType::Claude,
            21,
            crate::session::SessionStatus::Processing,
        );
        session.project_path = if empty_cwd {
            String::new()
        } else {
            project_path.to_string_lossy().into_owned()
        };
        Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            // 丁T3：本组测试的对话框在场探针缺省「无法判定」（None）——控制类注入
            // 照常投递；「在场即拒」的用例就地建 state 覆盖为假体（见 mode_switch_* 用例）
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| {
                serde_json::json!({
                    "host": { "name": "t", "platform": "macos", "version": "0.0.0-test" },
                    "enabledTools": ["claude"]
                })
            }),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        })
    }

    fn attach_req(uri: &str, cookie: Option<&str>, body: Body) -> axum::http::Request<Body> {
        let mut b = axum::http::Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/octet-stream");
        if let Some(c) = cookie {
            b = b.header("cookie", c);
        }
        b.body(body).unwrap()
    }

    fn attach_tempdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "mam-attach-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join(".git").join("info")).unwrap();
        d
    }

    #[tokio::test]
    async fn attachment_upload_lands_in_project_dir_and_excludes_from_git() {
        let proj = attach_tempdir();
        // 单一 state 实例：设备注册与 router 必须同源（DeviceStore::memory 每个实例独立，
        // 分开构造会让注册的设备在 app 里不存在 → 403 假阴性）
        let state = attach_state(proj.clone(), false);
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=sess_att&name=shot.png",
                Some("mam_device=mm"),
                Body::from(b"\x89PNG fake".as_slice()),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(
            body.contains("\"path\"") && body.contains("\"size\":9"),
            "{body}"
        );
        // 落盘 = <cwd>/.mam-attachments/<session>/，**文件名保真**（5dc540a 用户
        // 裁决：移除纳秒+内容哈希前缀，保留原始名——文件池按名搜索、agent 识名
        // 依赖原名；仅同名才追加 (1)(2) 序号。本断言系 5dc540a 漏改，F4 订正）
        let dir = proj.join(".mam-attachments").join("sess_att");
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let name = entries[0]
            .as_ref()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, "shot.png", "原始文件名保真，无前缀污染");
        // git 本地排除：首份写入即幂等追加；二次上传不重复
        let exclude =
            std::fs::read_to_string(proj.join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(exclude.matches(".mam-attachments/").count(), 1);
        // 二次上传：同一 state（设备注册仍有效），router 可重建
        let app = router(state.clone());
        let r = app
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=sess_att&name=second.txt",
                Some("mam_device=mm"),
                Body::from("two"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let exclude =
            std::fs::read_to_string(proj.join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(exclude.matches(".mam-attachments/").count(), 1, "幂等");
        std::fs::remove_dir_all(&proj).ok();
    }

    #[tokio::test]
    async fn attachment_gate_and_error_contracts() {
        // 403：无设备 cookie（门禁防御）
        let proj = attach_tempdir();
        let state = attach_state(proj.clone(), false);
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=sess_att&name=a.png",
                None,
                Body::from("x"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 403);
        // 404 no_session：未知会话
        let r = router(state.clone())
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=nope&name=a.png",
                Some("mam_device=mm"),
                Body::from("x"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_session"));
        // 404 no_cwd：会话无项目目录（与 resume 同源口径）；empty-cwd state 需另注册设备
        let empty_state = attach_state(proj.clone(), true);
        persist_named_device(&empty_state, "mm", "测试设备");
        let r = router(empty_state)
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=sess_att&name=a.png",
                Some("mam_device=mm"),
                Body::from("x"),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
        assert!(body_string(r).await.contains("no_cwd"));
        // 413 too_large：超 20MB（DefaultBodyLimit 硬兜底）；复用已注册设备的 state
        let big = vec![0u8; crate::remote::attachments::MAX_ATTACHMENT_BYTES + 1];
        let r = router(state.clone())
            .oneshot(attach_req(
                "/m/api/v1/session-attachment?session_id=sess_att&name=a.bin",
                Some("mam_device=mm"),
                Body::from(big),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 413);
        std::fs::remove_dir_all(&proj).ok();
    }

    // ===== 丁T3 接入①：模式切换的对话框在场守卫（§2.7 裁8/9，问题 5）=====
    //
    // 端点守卫的落点是「投递前屏读」——真实屏读需要 conhost 目标（CI 不可观测），
    // 故经 `RemoteState.dialog_probe` 缝注入假体：在场形态用**真机屏幕原文**解析出的
    // 选项表（`inject::dialog` 夹具同源，来自 2026-09-21 探测档案 screen-t5-*），
    // 不在场/不可判定形态返回 None。四个用例把两路（shift+tab / slash）与三种探针
    // 形态都钉住。

    /// codex 计划批准框的**真机屏幕原文**（`screen-t5-codex-implement-before.txt`
    /// 行 24–29）→ 解析成选项表（在场假体的载荷）。解析失败即断言红——夹具不合法
    /// 时用例必须失败而不是静默放行。
    fn real_dialog_fixture() -> Vec<crate::inject::dialog::DialogOption> {
        let lines: Vec<String> = [
            "  Implement this plan?",
            "",
            "› 1. Yes, implement this plan          Switch to Default and start coding.",
            "  2. Yes, clear context and implement  Fresh thread. Context: 2% used.",
            "  3. No, stay in Plan mode             Continue planning with the model.",
            "",
            "  Press enter to confirm or esc to go back",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        crate::inject::dialog::parse_dialog_options(&lines)
            .expect("夹具必须是真机屏幕原文（能解析出编号选项簇）")
    }

    /// **Key 路（shift+tab）在场即拒**：claude 会话 + 假体报在场 → 409
    /// `blocked_by_dialog` + 中文回执；**零注入**（injector 两个记录表全空）+
    /// **零审计**（账只记真发生过的事）。
    #[tokio::test]
    async fn mode_switch_key_path_blocked_when_dialog_present() {
        let fake = FakeInjector::ok();
        let opts = real_dialog_fixture();
        let state = inject_state_with_dialog(
            fake.clone(),
            std::sync::Arc::new(move |_, _| Some(opts.clone())),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                // sess_a = claude Waiting（shift+tab 路）
                Some(r#"{"sessionId":"sess_a","target":"plan"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            409,
            "对话框在场 → 控制类注入必须被拒（409 与 no_mechanism 同用 CONFLICT）"
        );
        let body = body_string(r).await;
        assert!(
            body.contains("\"error\":\"blocked_by_dialog\""),
            "拒绝码须可程序分诊：{body}"
        );
        assert!(
            body.contains("终端有待决对话框，请先处理"),
            "中文回执须直给用户可读语义：{body}"
        );
        assert!(
            fake.recorded_keys().is_empty(),
            "拒绝必须零注入（shift+tab 未投递）：{:?}",
            fake.recorded_keys()
        );
        assert!(fake.recorded().is_empty(), "拒绝路径不得有任何文本注入");
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(
            audits.is_empty(),
            "拒绝必须零审计（校验失败不落账，与 approve/question 同口径）：{audits:?}"
        );
    }

    /// **Text 路（斜杠命令）同受此门**：codex 会话 + 假体报在场 → 同样 409 且零注入。
    /// 这条是「两路都在投递之前」的锁——若守卫被误挪到 Key 分支内，本用例即变红。
    #[tokio::test]
    async fn mode_switch_slash_path_blocked_when_dialog_present() {
        let fake = FakeInjector::ok();
        let opts = real_dialog_fixture();
        let state = inject_state_with_dialog(
            fake.clone(),
            std::sync::Arc::new(move |_, _| Some(opts.clone())),
        );
        persist_named_device(&state, "mm", "测试设备");
        // sess_q = codex Waiting（approve 族夹具已有；本组用 inject_state 的会话清单，
        // 故此处换用 sess_d：zcode → no_mechanism，不行；改走「会话快照里加 codex」）
        // ——为保持夹具单一来源，本用例直接复用 inject_state 的 sess_a 会话但**改工具**
        // 不可行（夹具是不可变的），故显式建造一个 codex 会话的 state。
        let codex_sess = inj_sess(
            "sess_t3b",
            crate::session::AgentType::Codex,
            71,
            crate::session::SessionStatus::Waiting,
        );
        let state2 = Arc::new(RemoteState {
            session_source: {
                let s = codex_sess.clone();
                Box::new(move || crate::session::SessionsResponse {
                    sessions: vec![s.clone()],
                    total_count: 1,
                    waiting_count: 0,
                })
            },
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: fake.clone(),
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            dialog_probe: {
                let opts = real_dialog_fixture();
                std::sync::Arc::new(move |_, _| Some(opts.clone()))
            },
            // 本用例只验控制类注入守卫（对话框在场即拒）——问答阶段机的屏读缝
            // 缺省「读不到屏」（该用例不走问答路径）
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        });
        persist_named_device(&state2, "mm", "测试设备");
        let app = router(state2.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                // codex 的目标档 = 斜杠命令路（/plan）
                Some(r#"{"sessionId":"sess_t3b","target":"plan"}"#),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "斜杠命令路同受对话框在场门");
        let body = body_string(r).await;
        assert!(body.contains("\"error\":\"blocked_by_dialog\""), "{body}");
        assert!(
            fake.recorded().is_empty() && fake.recorded_keys().is_empty(),
            "斜杠命令必须零注入（文本与回车都不发）：{:?}/{:?}",
            fake.recorded(),
            fake.recorded_keys()
        );
        let audits = state2
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "零审计：{audits:?}");
    }

    /// **不在场正常投递**（回归锁：守卫不得把正常路径也拒掉）：假体返回 None →
    /// 既有行为原样（claude shift+tab 入 key_calls + 审计 action=mode result=ok）。
    /// 独占会话 `sess_t3c`（守卫 id 立规：INFLIGHT 按裸 id 全局占用，真投递的用例
    /// 必须各占唯一 id，否则并行跑会互相挤成「投递进行中」假红）。
    #[tokio::test]
    async fn mode_switch_proceeds_when_dialog_absent() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_guard_state(
            fake.clone(),
            "sess_t3c",
            crate::session::AgentType::Claude,
            72,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "无对话框 → 照常投递");
        let body = body_string(r).await;
        assert!(body.contains("\"status\":\"key_sent\""), "{body}");
        // 丁T3：如实标注「本次是否真的检测过」——None ≠ 在场，回执不得暗示已检查
        assert!(
            body.contains("\"dialogChecked\":false"),
            "探针不可用时须如实标注未检测：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(72u32, "shift+tab".to_string())],
            "shift+tab 键照常投递"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "mode", "正常路径审计不变");
        assert_eq!(audits[0].result, "ok");
    }

    /// **在场检测成功 → 回执标注 dialogChecked:true**（与上一用例的 false 成对锁：
    /// 该字段必须真实反映探针是否给出结论，而不是恒 true/false 的装饰）。独占会话
    /// `sess_t3d`（守卫 id 立规，同上）。
    #[tokio::test]
    async fn mode_switch_reports_dialog_checked_when_probe_answered() {
        let fake = FakeInjector::ok();
        // 假体返回**空表**（构造上不可达——解析器下界 ≥2——但恰好用来表达「探针给出了
        // 答案（Some）且判不在场」这一格：blocks_control_injection(Some(&[])) == false）
        let (state, sid) = mode_guard_state(
            fake.clone(),
            "sess_t3d",
            crate::session::AgentType::Claude,
            73,
            std::sync::Arc::new(|_, _| Some(vec![])),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(body.contains("\"dialogChecked\":true"), "{body}");
        assert_eq!(fake.recorded_keys(), vec![(73u32, "shift+tab".to_string())]);
    }

    // ===== 丁T3 F4-2：审批侧 dialog_probe 缝的两格自动化证据 =====
    //
    // 缺口（评审核实）：`read_dialog_options` 改经缝之后，`kimi_plan_approval_card_
    // never_emits_mapping_keys` 的 `v["dialog"]==true` 分支**只在有真实窗口的机器上
    // 才走得到**（CI 恒走 else）——缝的意义正是让屏读可测，故必须用假体把两格都钉住：
    // ① 缝给真机选项表 → GET 下发 `dialog:<n>` + `dialog=true`（T5 主路径）；
    // ② 缝给 None → 降级二元卡 + `degradedHint`（R1-3 防重警示，安全面）。
    // 两格都是真断言（键位不外泄 / 降级文案原文），不是「不 panic」。

    /// F4-2 格①：缝返回**真机屏幕原文**解析出的选项表（`real_dialog_fixture` =
    /// codex `Implement this plan?` 框，`screen-t5-codex-implement-before.txt` 行 24–29）
    /// → GET `/session-approve-options` 必须走 dialog 分支：`dialog=true` + 选项 id
    /// 全为 `dialog:<n>` + label 是屏上原文。
    #[tokio::test]
    async fn approve_dialog_branch_emits_dialog_options_via_probe() {
        let fake = FakeInjector::ok();
        let opts = real_dialog_fixture();
        let expected: Vec<(u32, String)> =
            opts.iter().map(|o| (o.number, o.label.clone())).collect();
        let state = approve_state_with_dialog(
            fake.clone(),
            Some(APPROVE_HIT_MSG),
            std::sync::Arc::new(move |_, _| Some(opts.clone())),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true);
        assert_eq!(v["dialog"], true, "缝给选项表 ⇒ dialog 分支必须成立");
        let options = v["options"].as_array().expect("options 应为数组");
        assert_eq!(
            options.len(),
            expected.len(),
            "选项数必须等于屏读解析出的编号项数"
        );
        for (o, (num, label)) in options.iter().zip(expected.iter()) {
            assert_eq!(
                o["id"].as_str().unwrap(),
                format!("dialog:{num}"),
                "dialog 分支的 id 形态（点按注入该数字）：{o}"
            );
            assert_eq!(
                o["label"].as_str().unwrap(),
                label.as_str(),
                "label 必须是屏上原文（T5 目标：把真实选项文本交给用户）：{o}"
            );
        }
        // 降级警示**不得**在 dialog 分支出现（R1-3 的警示条件是「命中审批但没读到
        // 对话框」——读到就不是降级态）
        assert_eq!(
            v["degradedHint"],
            serde_json::Value::Null,
            "读到选项时不叠加降级警示"
        );
    }

    /// F4-2 格②：缝返回 `None`（CI/非 Windows/对话框未绘制）→ 降级二元卡 +
    /// `degradedHint`（R1-3 防重警示：二元键可能错位命中非预期选项）。
    /// 与格①成对：同一夹具、同一会话，只换探针结论。
    #[tokio::test]
    async fn approve_dialog_branch_degrades_with_hint_via_probe() {
        let fake = FakeInjector::ok();
        let state = approve_state_with_dialog(
            fake.clone(),
            Some(APPROVE_HIT_MSG),
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "GET",
                "/m/api/v1/session-approve-options?session_id=sess_a",
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["available"], true, "命中审批（映射存在且 detect 命中）");
        assert_eq!(v["dialog"], false, "缝给 None ⇒ 降级二元卡");
        let options = v["options"].as_array().unwrap();
        assert_eq!(options.len(), 2, "降级 = 映射表二元项（允许/拒绝）");
        assert!(
            options
                .iter()
                .all(|o| !o["id"].as_str().unwrap().starts_with("dialog:")),
            "降级路径不得夹带 dialog:<n>"
        );
        assert_eq!(
            v["degradedHint"],
            "未读到终端对话框选项——终端可能正显示多选项，二元键可能错位，建议到终端确认",
            "R1-3 防重警示必须下发（前端二元卡脚注）"
        );
    }

    /// 单会话 + 可注入对话框探针的 state（丁T3 模式守卫用例专用建造器：会话 id/工具/
    /// pid 与探针全部参数化——守卫 id 立规要求真投递用例各占唯一会话 id）。
    fn mode_guard_state(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        sid: &str,
        tool: crate::session::AgentType,
        pid: u32,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
    ) -> (Arc<RemoteState>, String) {
        let session = inj_sess(sid, tool, pid, crate::session::SessionStatus::Waiting);
        let sid_out = session.id.clone();
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector,
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            dialog_probe,
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        });
        (state, sid_out)
    }

    /// 丁T4 建造器：单会话 + **指定会话状态**（codex 运行中门用）+ 缺省探针。
    /// 与 [`mode_guard_state`] 同构，多一个 status 参数（其余缝同口径）。
    fn mode_state_with_status(
        sid: &str,
        tool: crate::session::AgentType,
        pid: u32,
        status: crate::session::SessionStatus,
    ) -> (Arc<RemoteState>, String) {
        let session = inj_sess(sid, tool, pid, status);
        let sid_out = session.id.clone();
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector: FakeInjector::ok(),
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            dialog_probe: std::sync::Arc::new(|_, _| None),
            screen_probe: std::sync::Arc::new(|_, _| None),
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        });
        (state, sid_out)
    }

    /// 丁T4 建造器：单会话（指定状态）+ **指定注入器**（两段式投递用例用）。
    fn mode_state_with_injector(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        sid: &str,
        tool: crate::session::AgentType,
        pid: u32,
        status: crate::session::SessionStatus,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
    ) -> (Arc<RemoteState>, String) {
        mode_state_with_screen(
            injector,
            sid,
            tool,
            pid,
            status,
            dialog_probe,
            // 缺省无屏读（CI/非 Windows 与既有用例的既有行为；D20 的屏读轮询窗需要
            // 它的用例走 mode_state_with_screen 显式注入脚本化屏序列）
            std::sync::Arc::new(|_, _| None),
        )
    }

    /// D20 建造器：在 [`mode_state_with_injector`] 之上多一个**屏读能力缝**
    /// （`RemoteState.screen_probe`）——回读轮询的脚本化屏序列从这里进。
    ///
    /// 为什么必须缝上：D20 之后**模式回读是一个轮询循环**（读几拍、命中即停、窗尽取
    /// 最后一次判定），它是本批的新判据；不缝屏读，这条控制流就只有真机能覆盖
    /// （本批已两次栽在这上面）。
    #[allow(clippy::too_many_arguments)] // 7 个（建造器的每个缝都要显式传；不再加）
    fn mode_state_with_screen(
        injector: std::sync::Arc<dyn crate::inject::engine::Injector>,
        sid: &str,
        tool: crate::session::AgentType,
        pid: u32,
        status: crate::session::SessionStatus,
        dialog_probe: std::sync::Arc<crate::remote::server::DialogProbeFn>,
        screen_probe: std::sync::Arc<crate::remote::server::ScreenProbeFn>,
    ) -> (Arc<RemoteState>, String) {
        let session = inj_sess(sid, tool, pid, status);
        let sid_out = session.id.clone();
        let state = Arc::new(RemoteState {
            session_source: Box::new(move || crate::session::SessionsResponse {
                sessions: vec![session.clone()],
                total_count: 1,
                waiting_count: 0,
            }),
            store: crate::remote::pairing::DeviceStore::memory(),
            injector,
            resume_spawner: std::sync::Arc::new(|_: &crate::inject::resume::SpawnSpec| Ok(())),
            confirm_probe: std::sync::Arc::new(|_, _, _| true),
            dialog_probe,
            screen_probe,
            host_source: Box::new(|| serde_json::Value::Null),
            message_source: Box::new(|_, _, _| Err("测试桩：未注入内容源".to_string())),
            path_source: Box::new(|_, _, _| (Vec::new(), false)),
            watcher_tx: tokio::sync::broadcast::channel(64).0,
            sse_registry: Arc::new(SseRegistry::default()),
            max_devices_source: Box::new(|| 3),
            pin_limiter: std::sync::Mutex::new(crate::remote::pin::PinRateLimiter::new()),
            pin_source: Box::new(|| Some("1234".to_string())),
            now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
            tunnel_hosts_source: Box::new(|| Some(Vec::new())),
            via_hosts_source: Box::new(|| None),
            home_source: Box::new(|| None),
        });
        (state, sid_out)
    }

    /// D20：脚本化屏读缝（第 i 次读返回 `screens[i]`；用尽后**重复最后一屏**——
    /// 与 `inject::mode::tests::run_readback_script` 同语义：模拟「屏不再变」）。
    /// `None` 元素 = 那一拍读不到屏（读屏失败/平台无屏读）。
    fn scripted_screen_probe(
        screens: Vec<Option<Vec<String>>>,
    ) -> std::sync::Arc<crate::remote::server::ScreenProbeFn> {
        let pos = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::sync::Arc::new(move |_sid: &str, _pid: u32| {
            let i = pos.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let idx = i.min(screens.len().saturating_sub(1));
            screens.get(idx).and_then(|s| s.clone())
        })
    }

    fn screen_lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ===== 丁T4：模式栏二维结构 / 回读全开 / 组切换 / 运行中门 =====
    //
    // 本组用例的共同前提：**CI/非 Windows 下屏读恒 None** → GET 的 `current` 为 null。
    // 这对本组无碍：T4 的新面是**结构表与组路由**（纯表 + 请求面），屏读词表本身由
    // `inject::mode` 的单测覆盖（夹具 = T6 探测档案的真机屏幕原文逐字快照）。

    /// GET /session-mode 下发**结构表与两组**（裁5 的接口面）：codex 两组、权限组
    /// 无回读源 → current=null；模式组 readback=true。裁7 的退役档也在载荷里
    /// （`legacy`）——**前端不渲染为按钮**，由 `tests/mobile/ModeBar.test.tsx` 锁住。
    #[tokio::test]
    async fn session_mode_reports_two_axis_structure_for_codex() {
        let (state, sid) = mode_state_with_status(
            "sess_t4_get_codex",
            crate::session::AgentType::Codex,
            81,
            crate::session::SessionStatus::Waiting,
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["tool"], "codex");
        assert_eq!(v["structure"], "twoAxis");
        let groups = v["groups"].as_array().expect("groups 必须是数组");
        assert_eq!(groups.len(), 2, "二维家出两组（裁5）");
        assert_eq!(groups[0]["id"], "mode");
        assert_eq!(groups[1]["id"], "permission");
        // 权限组：无底栏回读源 → readback=false 且 current=null（**如实**，不假装知道）
        assert_eq!(groups[1]["readback"], false);
        assert!(groups[1]["current"].is_null());
        assert_eq!(groups[0]["readback"], true);
        // 档位：模式组 [操作(可选), 计划(可选)] = shift+tab toggle；权限组 [只读, 默认, 完全信任]
        assert_eq!(groups[0]["layout"], "toggle", "codex 模式组=单钮 toggle");
        assert_eq!(
            groups[1]["layout"], "picker",
            "codex 权限组=单选面板（2026-09-23 用户方案：读回终端菜单选项供用户点选）"
        );
        let mode_tiers = groups[0]["tiers"].as_array().unwrap();
        assert_eq!(mode_tiers[0]["mode"], "default");
        assert_eq!(mode_tiers[0]["label"], "操作");
        assert_eq!(mode_tiers[0]["selectable"], true);
        assert!(
            mode_tiers[0]["reason"].is_null(),
            "可选档不带 reason（如实回执的反面是不乱贴标签）"
        );
        assert_eq!(mode_tiers[1]["mode"], "plan");
        assert_eq!(mode_tiers[1]["selectable"], true);
        let perm_tiers = groups[1]["tiers"].as_array().unwrap();
        assert_eq!(
            perm_tiers.len(),
            4,
            "2026-09-23 起四档（含自动审批=Approve for me）"
        );
        assert_eq!(
            perm_tiers
                .iter()
                .map(|t| t["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["只读", "默认", "自动审批", "完全信任"]
        );
        // 裁7：退役档在 legacy 里如实登记，**不在 tiers 里**
        let legacy = groups[1]["legacy"]
            .as_array()
            .expect("codex 权限组带 legacy");
        assert_eq!(legacy.len(), 2);
        assert_eq!(legacy[0]["label"], "untrusted");
        assert_eq!(legacy[1]["label"], "on-failure");
        assert!(
            !perm_tiers
                .iter()
                .any(|t| matches!(t["mode"].as_str(), Some("untrusted") | Some("on-failure"))),
            "退役档不得作为可选档出现在 tiers 里（裁7）"
        );
        // 旧前端兼容视图仍在（顶层 current/switchKind）
        assert_eq!(v["switchKind"], "slashCommand");
        assert!(v["current"].is_null());
    }

    /// GET：权限组 current 来自「**上次切换**」记忆（2026-09-23 codex 模式切换改造
    /// 的显示面）——verified 切换写入 `PERMISSION_TIER_MEMORY` 后，GET 把它回放为
    /// 权限组的 current/currentLabel；无记录的会话权限组仍恒 null（模式未知，如实）。
    #[tokio::test]
    async fn session_mode_reports_permission_tier_from_memory() {
        let (state, sid) = mode_state_with_status(
            "sess_mc_get_mem",
            crate::session::AgentType::Codex,
            92,
            crate::session::SessionStatus::Waiting,
        );
        persist_named_device(&state, "mm", "测试设备");
        // 无记录：权限组恒 null（不假装知道）
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert!(v["groups"][1]["current"].is_null(), "无记忆 → 模式未知");

        // 记忆后：GET 回放为 current/currentLabel（模拟一次 verified=true 的切换）
        crate::remote::api::remember_permission_tier(&state.store, &sid, "readOnly");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["groups"][1]["current"], "readOnly");
        assert_eq!(v["groups"][1]["currentLabel"], "只读");
    }

    /// GET：kimi 两组 + **权限组的屏显标签是工具自己的词**（§2.6 kimi 列）
    #[tokio::test]
    async fn session_mode_reports_kimi_permission_labels() {
        let (state, sid) = mode_state_with_status(
            "sess_t4_get_kimi",
            crate::session::AgentType::Kimi,
            82,
            crate::session::SessionStatus::Idle,
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["structure"], "twoAxis");
        let perm = &v["groups"][1];
        assert_eq!(perm["id"], "permission");
        assert_eq!(perm["label"], "权限");
        assert_eq!(
            perm["tiers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["总是询问", "按需询问", "永不询问"],
            "kimi 权限组屏显标签 = 该工具自己的词（不是 MAM 通用名）"
        );
        // 三档全部可选（「总是询问」走两段式；另两档有直达变体）
        assert!(perm["tiers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["selectable"] == true));
        assert!(perm["legacy"].as_array().unwrap().is_empty());
    }

    /// GET：单轴家（opencode）只有一组，且档位标签已按裁6 术语对齐
    #[tokio::test]
    async fn session_mode_reports_single_axis_for_opencode() {
        let (state, sid) = mode_state_with_status(
            "sess_t4_get_oc",
            crate::session::AgentType::OpenCode,
            83,
            crate::session::SessionStatus::Idle,
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["structure"], "singleAxis");
        assert_eq!(v["groups"].as_array().unwrap().len(), 1);
        assert_eq!(v["groups"][0]["step"], true, "shift+tab 一步一档");
        assert_eq!(
            v["groups"][0]["tiers"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["label"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["默认", "计划"],
            "裁6：Build 改显「默认」"
        );
    }

    /// GET：未实测工具 → structure="none" + 空 groups（前端不渲染）
    #[tokio::test]
    async fn session_mode_reports_none_for_untested_tool() {
        let (state, sid) = mode_state_with_status(
            "sess_t4_get_wb",
            crate::session::AgentType::WorkBuddy,
            84,
            crate::session::SessionStatus::Idle,
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "GET",
                &format!("/m/api/v1/session-mode?session_id={sid}"),
                Some("mam_device=mm"),
                None,
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["structure"], "none");
        assert!(v["groups"].as_array().unwrap().is_empty());
        assert_eq!(v["switchKind"], "unsupported");
    }

    /// POST：**codex 模式组 shift+tab toggle**（2026-09-23 用户实测裁决）——显式
    /// `group:"mode"` + `target:"plan"|"default"` → 只投递一次 `shift+tab` 键
    /// （无斜杠命令、无额外回车）；落点由回读轮询核验（CI 无屏读 → Unverifiable，
    /// 不影响投递本身）。
    #[tokio::test]
    async fn session_mode_switch_codex_mode_group_sends_shift_tab() {
        for target in ["plan", "default"] {
            let fake = FakeInjector::ok();
            let (state, sid) = mode_state_with_injector(
                fake.clone(),
                "sess_mc_mode_toggle",
                crate::session::AgentType::Codex,
                91,
                crate::session::SessionStatus::Waiting,
                std::sync::Arc::new(|_, _| None),
            );
            persist_named_device(&state, "mm", "测试设备");
            let app = router(state);
            let r = app
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-mode/switch",
                    Some("mam_device=mm"),
                    Some(&format!(
                        r#"{{"sessionId":"{sid}","target":"{target}","group":"mode"}}"#
                    )),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body = body_string(r).await;
            assert!(
                fake.recorded().is_empty(),
                "模式组 shift+tab 不投递任何文本：{body}"
            );
            assert_eq!(
                fake.recorded_keys(),
                vec![(91u32, "shift+tab".to_string())],
                "目标档 {target} 同样只发一次 shift+tab：{body}"
            );
        }
    }

    /// POST：**显式 group 路由**（丁T4 新增字段）——codex 权限组「完全信任」→ 走
    /// `/permissions` 两段式的**第一段**（文本 + 回车；第二段无真屏读 → 中止并如实回执）
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_mode_switch_routes_explicit_permission_group() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_perm",
            crate::session::AgentType::Codex,
            85,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"bypass","group":"permission"}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        // 第一段已投递：`/permissions` 文本 + enter 键
        assert_eq!(
            fake.recorded(),
            vec![(85u32, "/permissions".to_string())],
            "两段式的第一段必须投递开启命令：{body}"
        );
        assert_eq!(
            fake.recorded_keys(),
            vec![(85u32, "enter".to_string())],
            "斜杠命令需回车提交"
        );
        // 第二段（CI 无屏读）→ 中止 + 如实回执（**不是**「已切换」）
        assert!(
            body.contains("\"status\":\"failed\""),
            "第二段读不到菜单必须如实失败：{body}"
        );
        assert!(
            body.contains("请人工核对终端"),
            "失败文案要讲清「命令已发、档位未切」：{body}"
        );
    }

    /// POST：**旧客户端不带 group** —— codex 的 `default` 由后端推断到模式组
    /// （2026-09-23 起 Default 两组都可选 → 歧义消解与 kimi 同规：取模式组，
    /// shift+tab toggle；见 `resolve_group` 文档与 `group_inference_rules`）
    #[tokio::test]
    async fn session_mode_switch_infers_group_for_legacy_client() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_infer",
            crate::session::AgentType::Codex,
            86,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"default"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        // 推断到模式组 → shift+tab 被投递（Key 路，无斜杠命令）
        assert!(fake.recorded().is_empty(), "{body}");
        assert_eq!(
            fake.recorded_keys(),
            vec![(86u32, "shift+tab".to_string())],
            "{body}"
        );
        // 审计摘要含**组名**（二维工具的组是语义的一部分）
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert_eq!(audits[0].action, "mode");
        assert!(
            audits[0].summary.contains("模式"),
            "二维家审计摘要要带组名（否则分不清是模式组还是权限组的默认）：{:?}",
            audits[0].summary
        );
    }

    /// POST：**退役档不可选**（裁7 的输入面）——POST `untrusted` → 400（零注入零审计）
    #[tokio::test]
    async fn session_mode_switch_rejects_legacy_enum() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_legacy",
            crate::session::AgentType::Codex,
            87,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        for target in ["untrusted", "on-failure"] {
            let r = app
                .clone()
                .oneshot(req(
                    "POST",
                    "/m/api/v1/session-mode/switch",
                    Some("mam_device=mm"),
                    Some(&format!(r#"{{"sessionId":"{sid}","target":"{target}"}}"#)),
                ))
                .await
                .unwrap();
            assert_eq!(r.status(), 400, "退役档不得作为可选档（裁7）：{target}");
        }
        assert!(fake.recorded().is_empty(), "拒绝路径零注入");
        assert!(fake.recorded_keys().is_empty());
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "拒绝路径零审计：{audits:?}");
    }

    /// POST：**组里没有的档** → 409 no_mechanism（kimi 模式组没有只读档）
    #[tokio::test]
    async fn session_mode_switch_rejects_tier_outside_group() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_outgroup",
            crate::session::AgentType::Kimi,
            88,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"readOnly","group":"mode"}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409);
        let body = body_string(r).await;
        assert!(body.contains("\"error\":\"no_mechanism\""), "{body}");
        assert!(fake.recorded().is_empty() && fake.recorded_keys().is_empty());
    }

    /// **codex `/plan` 运行中不可用 → 如实回执**（§2.6 表末）：会话 Processing →
    /// 200 failed + 中文说明，**零注入零审计**（codex 自己也会拒，MAM 提前拦）。
    /// 还原动作：删掉 `codex_plan_busy` 那道门 → 本断言先红（会变成投递 `/plan`）。
    #[tokio::test]
    async fn session_mode_switch_reports_codex_plan_busy() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_busy",
            crate::session::AgentType::Codex,
            89,
            crate::session::SessionStatus::Processing,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let body = body_string(r).await;
        assert!(body.contains("\"status\":\"failed\""), "{body}");
        assert!(
            body.contains("运行中不接受模式切换"),
            "回执要讲清「为什么没切」（如实，不是静默失败）：{body}"
        );
        assert!(
            fake.recorded().is_empty() && fake.recorded_keys().is_empty(),
            "运行中门必须在投递之前：{:?}/{:?}",
            fake.recorded(),
            fake.recorded_keys()
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(
            audits.is_empty(),
            "零投递零审计（与忙让位同口径）：{audits:?}"
        );
    }

    /// codex 运行中门**只管 Plan 档**：同一 Processing 会话切权限组 → 照常出手
    /// （权限菜单的可用性与回合状态无关；未实测有同类限制 → 不扩张）。
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_mode_switch_codex_busy_only_blocks_plan() {
        let fake = FakeInjector::ok();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_busy_perm",
            crate::session::AgentType::Codex,
            90,
            crate::session::SessionStatus::Processing,
            std::sync::Arc::new(|_, _| None),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"bypass","group":"permission"}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            fake.recorded(),
            vec![(90u32, "/permissions".to_string())],
            "运行中门只拦 Plan 档（权限组照常出手）"
        );
    }

    /// **两段式的第二段不重入守卫**（丁T4 的核心设计点）：第一段之前的守卫恰好调用
    /// **一次**，第一段照常投递——若第二段重入守卫，屏上刚打开的菜单会被判成
    /// 「待决对话框」→ 409 blocked_by_dialog（自相矛盾：权限档永远切不了）。
    ///
    /// **本用例能证明什么**：探针只在**第一段之前**被调用一次（守卫位）→ 断「调用
    /// 次数恰好 1」+「第一段照常投递」。若有人在第二段前再插一次探针，计数变 2 → 先红。
    #[tokio::test]
    #[cfg_attr(
        not(any(windows, target_os = "macos")),
        ignore = "注入平台门（inject/routing.rs）：仅 Windows/macOS 可注入，本测走注入链"
    )]
    async fn session_mode_switch_menu_guard_runs_once_before_first_stage() {
        let fake = FakeInjector::ok();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_probe = calls.clone();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_guard_once",
            crate::session::AgentType::Codex,
            91,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(move |_, _| {
                // 第一次（也是唯一一次）报**不在场**：用户点按钮时终端上没有别人的
                // 对话框；第一段之后出现的菜单**不经过探针**（这正是设计点）
                calls_probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                None
            }),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"readOnly","group":"permission"}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "第一段不得被自己的菜单拦下");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "守卫在位恰好调用一次（每请求一道门；第二段不重入）"
        );
        assert_eq!(
            fake.recorded(),
            vec![(91u32, "/permissions".to_string())],
            "第一段照常投递"
        );
        assert_eq!(fake.recorded_keys(), vec![(91u32, "enter".to_string())]);
    }

    /// **第一段之前有真对话框 → 仍被守卫拒**（回归锁：守卫位置不得因为两段式改造
    /// 而漂移）。用真机屏幕原文（codex `Implement this plan?`）作在场假体。
    #[tokio::test]
    async fn session_mode_switch_menu_blocked_when_foreign_dialog_present() {
        let fake = FakeInjector::ok();
        let opts = real_dialog_fixture();
        let (state, sid) = mode_state_with_injector(
            fake.clone(),
            "sess_t4_menu_blocked",
            crate::session::AgentType::Codex,
            92,
            crate::session::SessionStatus::Waiting,
            std::sync::Arc::new(move |_, _| Some(opts.clone())),
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state.clone());
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"bypass","group":"permission"}}"#
                )),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "两段式也要过第一段前的那道守卫");
        let body = body_string(r).await;
        assert!(body.contains("\"error\":\"blocked_by_dialog\""), "{body}");
        assert!(
            fake.recorded().is_empty() && fake.recorded_keys().is_empty(),
            "拒绝 = 零注入（含第一段的开启命令）"
        );
        let audits = state
            .store
            .with(|c| crate::database::dao::write_audit::recent_conn(c, 10));
        assert!(audits.is_empty(), "零审计：{audits:?}");
    }

    // ===== D20：模式回读的**动态轮询**（宪法 §5.(c) 第 8 条 / 计划 §2.9）=====
    //
    // 本组用例把**端点侧**的回读路径（含缝接线与回执合成）钉在门禁里：内核的轮询语义
    // 由 `inject::mode::tests::readback_poll_*` 覆盖，这里补的是「端点用不用它、用对
    // 没有、回执怎么念」。屏序列经 `RemoteState.screen_probe` 缝注入（零 conhost）。

    /// **用户实机观察①的回归锁（端点侧）**：claude 切档后**第一拍读到旧档**、后续拍读到
    /// 目标档 → 回执必须是 `verified=true`（而不是旧实现的「回读与预期不符」）。
    ///
    /// 夹具用 claude 真机底栏原文（T6 探测档案逐字）。会话切档前是**默认档**
    /// （`manual mode on`），目标 = `plan` → 机制是 shift+tab 一步，按实测环序
    /// `[AcceptEdits, Plan, Bypass, Default]` 推算应到档 = **接受编辑**
    /// （`⏵⏵ accept edits on`）——故重绘后的那一拍必须是这一条，才叫「命中」。
    ///
    /// 还原动作（变异①）：把端点改回「单次读」（不调 `poll_mode_readback`，只读一次屏）
    /// → 本测试先红（`verified` 会是 false、hint 会是「与预期不符」）。
    #[tokio::test]
    async fn session_mode_switch_readback_polls_until_target_mode_appears() {
        let fake = FakeInjector::ok();
        let stale = screen_lines(&["  ⏸ manual mode on · ? for shortuts ·←for agents"]);
        let fresh = screen_lines(&["  ⏵⏵ accept edits on (shift+tab to cycle) · ← for agents"]);
        // 屏序列：① 切档前的 before（注入前的单次快照，D20(c) 例外）读到默认档；
        // ② 投递后第一拍仍是旧档（重绘未及——观察① 的形态）；③ 第二拍拍到目标档
        let probe = scripted_screen_probe(vec![Some(stale.clone()), Some(stale), Some(fresh)]);
        let (state, sid) = mode_state_with_screen(
            fake.clone(),
            "sess_d20_readback_poll",
            crate::session::AgentType::Claude,
            93,
            crate::session::SessionStatus::Idle,
            std::sync::Arc::new(|_, _| None),
            probe,
        );
        persist_named_device(&state, "mm", "测试设备");
        let app = router(state);
        let r = app
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(fake.recorded_keys(), vec![(93u32, "shift+tab".to_string())]);
        assert_eq!(
            v["verified"], true,
            "第一拍旧档、第二拍目标档 → 必须确认成功（观察①的修复形态）：{v}"
        );
        assert!(v["hint"].is_null(), "命中态不得带 hint（不假装失败）：{v}");
        assert_eq!(
            v["current"], "acceptEdits",
            "回执的 current 取**最后一拍**读数：{v}"
        );
    }

    /// **窗尽如实区分三态（端点侧文案）**：同一条端点、同样的投递，屏上「一直是旧档」
    /// （不符）与「一直读不到屏」（未及确认）必须给出**两句不同的话**——D20(b) 明令
    /// 不得退化成统一的「失败」。
    ///
    /// 还原动作（变异②）：把 `mode_verify_receipt` 的 `Mismatch` / `Unverifiable` 两支
    /// 合并成同一句 hint → 本测试的 `assert_ne!` 与两处关键词断言先红。
    #[tokio::test]
    async fn session_mode_switch_readback_window_end_keeps_three_states_distinct() {
        let fake = FakeInjector::ok();
        let stale = screen_lines(&["  ⏸ manual mode on · ? for shortuts ·←for agents"]);

        // ① 窗内一直是旧档 → 「不符」：必须报出预期/实际两边（预期 = 环序推算的 AcceptEdits）
        let probe = scripted_screen_probe(vec![Some(stale.clone())]); // 用尽后重复 = 屏不变
        let (state, sid) = mode_state_with_screen(
            fake.clone(),
            "sess_d20_mismatch",
            crate::session::AgentType::Claude,
            94,
            crate::session::SessionStatus::Idle,
            std::sync::Arc::new(|_, _| None),
            probe,
        );
        persist_named_device(&state, "mm", "测试设备");
        let r = router(state)
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["verified"], false);
        let mismatch_hint = v["hint"].as_str().unwrap().to_string();
        assert!(
            mismatch_hint.contains("预期「接受编辑」") && mismatch_hint.contains("实际「默认」"),
            "「不符」要把两边都报出来（预期 = 按实测环序推算的应到档）：{mismatch_hint}"
        );
        assert_eq!(
            v["current"], "default",
            "窗尽时的 current = 最后一拍读到的档（不是 null）：{v}"
        );

        // ② 窗内始终读不到屏 → 「未及确认」：另一句话，且 current 必须是 null
        let probe = scripted_screen_probe(vec![None]);
        let (state2, sid2) = mode_state_with_screen(
            fake.clone(),
            "sess_d20_unverifiable",
            crate::session::AgentType::Claude,
            95,
            crate::session::SessionStatus::Idle,
            std::sync::Arc::new(|_, _| None),
            probe,
        );
        persist_named_device(&state2, "mm", "测试设备");
        let r = router(state2)
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid2}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["verified"], false);
        let unverifiable_hint = v["hint"].as_str().unwrap().to_string();
        assert!(
            unverifiable_hint.contains("无法自动确认"),
            "「未及确认」说的是「读不到」（另一句话）：{unverifiable_hint}"
        );
        assert!(
            !unverifiable_hint.contains("与预期不符"),
            "读不到 **不是**「不符」（D20(b)：两态不得混同）：{unverifiable_hint}"
        );
        assert!(
            v["current"].is_null(),
            "一格都没读到 → current 必须为 null（不给过期值）：{v}"
        );
        assert_ne!(
            mismatch_hint, unverifiable_hint,
            "两种超时的回执文案必须不同（D20(b)）"
        );
    }

    /// **命中即停、窗尽有界**（端点侧的量）：屏序列第 2 拍即读到目标档 → 缝的读数**恰好
    /// 3 次**（① before 快照 1 次 + 回读 2 拍），而不是把 15 拍的窗睡满。
    ///
    /// 这条同时钉住 D20(a) 的「禁止用固定睡眠替代轮询」在**端点侧**也成立：若有人把
    /// 轮询换回「睡满窗再读一次」，读数次数会退化到 1（且本断言先红）。
    #[tokio::test]
    async fn session_mode_switch_readback_stops_at_first_hit_within_window() {
        let fake = FakeInjector::ok();
        let stale = screen_lines(&["  ⏸ manual mode on · ? for shortuts ·←for agents"]);
        let fresh = screen_lines(&["  ⏵⏵ accept edits on (shift+tab to cycle) · ← for agents"]);
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reads_for_probe = reads.clone();
        let pos = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let probe: std::sync::Arc<crate::remote::server::ScreenProbeFn> =
            std::sync::Arc::new(move |_sid: &str, _pid: u32| {
                reads_for_probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let i = pos.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // 0 = before 快照（旧档）；1 = 回读首拍（仍是旧档，重绘未及）；
                // 2 起 = 目标档（命中 → 停止轮询，不再读）
                if i <= 1 {
                    Some(stale.clone())
                } else {
                    Some(fresh.clone())
                }
            });
        let (state, sid) = mode_state_with_screen(
            fake.clone(),
            "sess_d20_readback_stop",
            crate::session::AgentType::Claude,
            96,
            crate::session::SessionStatus::Idle,
            std::sync::Arc::new(|_, _| None),
            probe,
        );
        persist_named_device(&state, "mm", "测试设备");
        let r = router(state)
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(r#"{{"sessionId":"{sid}","target":"plan"}}"#)),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(v["verified"], true, "{v}");
        assert_eq!(
            reads.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "① before 快照 + ② 回读两拍（第二拍命中即停）——窗共 15 拍，不许睡满"
        );
    }

    /// **无判据可读 → 不轮询**（D20(a)：判据不存在就没有轮询可做）：kimi 权限组的
    /// `spec.readback == false`（实测底栏不含权限档文本）→ `expected_mode_after` 给
    /// `None` → 内核只读一拍（`mode::poll_mode_readback` 的「短路径 ②」）。
    ///
    /// 读数断言 = **2**：① 注入前的 `before` 快照 1 次（D20(c) 的瞬时快照，kimi 整体
    /// 支持回读故这一次会读）+ ② 回读一拍。若是 16（= 1 + 15 拍睡满窗）说明有人把
    /// 短路径拆了——那正是 D20(a) 禁止的「固定睡眠」。
    ///
    /// 还原动作：把内核的 `if expected.is_some() { rounds.max(1) } else { 1 }` 改回
    /// `rounds.max(1)` → 本测试先红（读数 16）。
    #[tokio::test]
    async fn session_mode_switch_does_not_spin_when_group_has_no_readback_source() {
        let fake = FakeInjector::ok();
        let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reads_for_probe = reads.clone();
        let probe: std::sync::Arc<crate::remote::server::ScreenProbeFn> =
            std::sync::Arc::new(move |_sid: &str, _pid: u32| {
                reads_for_probe.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // 屏读可用，但该组没有任何东西构成回读判据
                Some(screen_lines(&[" GLM-5.3-Flash thinking: high  C:\\proj"]))
            });
        let (state, sid) = mode_state_with_screen(
            fake.clone(),
            "sess_d20_no_readback",
            crate::session::AgentType::Kimi,
            97,
            crate::session::SessionStatus::Idle,
            std::sync::Arc::new(|_, _| None),
            probe,
        );
        persist_named_device(&state, "mm", "测试设备");
        let r = router(state)
            .oneshot(req(
                "POST",
                "/m/api/v1/session-mode/switch",
                Some("mam_device=mm"),
                Some(&format!(
                    r#"{{"sessionId":"{sid}","target":"bypass","group":"permission"}}"#
                )),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body_string(r).await).unwrap();
        assert_eq!(
            fake.recorded(), // `/auto` 是 kimi 权限组 Bypass 的直达命令
            vec![(97u32, "/auto".to_string())],
            "kimi 权限组 Bypass = /auto 直达：{v}"
        );
        assert_eq!(
            reads.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "① before 快照 1 次 + ② 无判据时只读一拍（不是 15 拍睡满窗）：{v}"
        );
        assert!(
            v["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("无法自动确认"),
            "无回读源 → 回执如实说「无法自动确认」（不假装）：{v}"
        );
    }
}
