// 远程接入层（M2）：axum 内嵌服务器 + 直通配对 + 移动看板 API
// 范围与红线见 docs/superpowers/plans/2026-09-14-m2-remote-access-board.md

pub mod api;
pub mod approval;
pub mod content;
pub mod events;
pub mod files;
pub mod gate;
pub mod pairing;
pub mod power;
pub mod server;
pub mod tunnel;
pub mod watcher;

pub const KEY_ENABLED: &str = "remote.enabled";
pub const KEY_BIND: &str = "remote.bind";
pub const KEY_PORT: &str = "remote.port";
pub const KEY_PUBLIC_ACK: &str = "remote.public_ack";
/// 本机展示名（P8b）：设置里可覆盖 sysinfo 探测值；空串视为未设置
pub const KEY_HOST_NAME: &str = "remote.host_name";
/// 默认端口（避开 3080=dsh / 1420=vite / 18789=zcode）
pub const DEFAULT_PORT: u16 = 9420;
/// 外部通道（M4 T1）：off / quick / named 三值（tunnel::parse_channel 唯一解析口）
pub const KEY_CHANNEL: &str = "remote.channel";
/// 命名隧道 Tunnel Token（M4 T1b；明文本地存储与设备表同库）
pub const KEY_TUNNEL_TOKEN: &str = "remote.tunnel_token";
/// 设备上限键（spec T2c：默认 3 台可配）
pub const KEY_MAX_DEVICES: &str = "remote.max_devices";

/// 上限解析（纯函数）：None/乱串 → 3；clamp 1..=10
pub fn max_devices_from(v: Option<String>) -> usize {
    v.and_then(|s| s.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 10))
        .unwrap_or(3)
}

/// 生产上限源（STATE 构造注入；唯一 KV 读取点）
fn max_devices_from_kv() -> usize {
    max_devices_from(crate::database::dao::settings::get_setting(KEY_MAX_DEVICES))
}

/// 在线口径（spec T2d 自审修正）：活跃 SSE 连接 ∨ 30s 内过闸
pub fn is_online(registry_hit: bool, last_seen_at: i64, now: i64) -> bool {
    registry_hit || now - last_seen_at < 30_000
}

/// 8 字节随机 hex（审批请求 id 生成器）：fill_bytes 后逐字节两位 hex——
/// 与 STATE 里 pairing token 生成器同写法，抽成复用函数（控制者预裁定 2：
/// 生成器零参随机形态，**不可位置式**——消费后复用会让旧轮询搭上新请求）
fn random_hex_8() -> String {
    let mut b = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// 16 字节随机 hex（审批配对设备 id 生成器）：与直通配对 device_id 同风格
fn random_hex_16() -> String {
    let mut b = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ============================================================
// 生命周期接线（Task 4）：服务器随设置启停 + 四个 tauri 命令 + 启动恢复
// ============================================================

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// 服务器任务句柄单例：Some = 已启动（重复开启幂等）。
/// 存 tauri 的 JoinHandle 而非 tokio 的——控制者裁决（2026-09-14）：
/// 裸 `tokio::spawn` 在没有 runtime 上下文的线程上直接 panic（no reactor running），
/// 而本模块的两个调用点（sync 命令 `remote_toggle`、`lib.rs` 的 `.setup()`）都不在
/// runtime 内；`tauri::async_runtime::spawn` 内部先 `enter()` 再 spawn，任意线程可用。
static SERVER_HANDLE: Lazy<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(None));

/// 共享状态单例：服务器任务与 tauri 命令共用同一份 pairing / 会话源 / 设备存储
static STATE: Lazy<std::sync::Arc<server::RemoteState>> = Lazy::new(|| {
    std::sync::Arc::new(server::RemoteState {
        pairing: Mutex::new(pairing::PairingService::new(
            10 * 60 * 1000, // 配对码有效期 10 分钟
            pairing::PairingClock {
                now: Box::new(|| chrono::Utc::now().timestamp_millis()),
                token: Box::new(|| {
                    let mut b = [0u8; 16];
                    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
                    b.iter().map(|x| format!("{x:02x}")).collect()
                }),
            },
        )),
        // P8 数据同源：直调唯一聚合口（R3 单飞护栏保护第三消费者），禁止复制聚合逻辑
        session_source: Box::new(crate::adapter::get_all_sessions),
        store: pairing::DeviceStore::global(),
        // M3 Task 1：host 载荷同源直调（P8b 读 settings + enabledTools 读 DB，注入缝供测试）
        host_source: Box::new(host_info),
        // M3 Task 7：会话内容同源直调（八工具统一出口 content::read_session_messages，
        // 注入缝供端点测试；生产签名 fn(&str,&str,usize) 与 trait 对象形态一致）
        message_source: Box::new(content::read_session_messages),
        // M3 Task 8：文件路径源同源直调（files::extract_file_paths 内部复用 content
        // 层读取，注入缝供端点测试）
        path_source: Box::new(files::extract_file_paths),
        // M3 Task 5：跃迁事件通道与扫描循环同源（watcher::event_sender 与
        // SessionWatcher::start 共用全进程唯一通道；Task 6 的 SSE 只订阅此 tx）
        watcher_tx: watcher::event_sender(),
        // M4 T0a：SSE 连接注册表（吊销/停止即时断连 + Task 7 在线口径数据源）
        sse_registry: std::sync::Arc::new(server::SseRegistry::default()),
        // M4 T2：审批队列（零参随机 hex 生成器——id/设备 id 不可位置式，防消费后复用）
        approval: Mutex::new(approval::ApprovalService::new(
            5 * 60 * 1000,
            Box::new(random_hex_8), // 请求 id：8 字节随机 hex
            // 4 位码：0000-9999 等概率，前导零补齐
            Box::new(|| {
                format!(
                    "{:04}",
                    rand::Rng::gen_range(&mut rand::thread_rng(), 0..10000)
                )
            }),
            Box::new(random_hex_16), // 设备 id：16 字节随机 hex（与直通配对 device_id 同风格）
        )),
        // M4 T2：设备上限源（生产读 KV——max_devices_from_kv 是唯一读取点）
        max_devices_source: Box::new(max_devices_from_kv),
    })
});

/// 读取设置里的绑定地址与端口（薄壳：DB 读取在此外置，解析内核抽为纯函数便于单测）。
/// M2-R3：设置里存了非法 bind 时返回 Err（由调用方决定拒绝启动或展示回落）
fn bind_and_port() -> Result<(String, u16), String> {
    parse_bind(
        crate::database::dao::settings::get_setting(KEY_BIND).as_deref(),
        crate::database::dao::settings::get_setting(KEY_PORT).as_deref(),
    )
}

/// 绑定/端口解析内核（纯函数，不触 DB）：无设置回落 `127.0.0.1:DEFAULT_PORT`；
/// 端口字符串非法（非数字 / 超 u16 范围）回落默认端口。
/// M2-R3：bind 值域校验——空缺回落 127.0.0.1，`localhost` 与任何合法 IP 地址放行，
/// 其余字符串（乱串/带端口的复合串/URL 等）一律 Err，绝不静默透传给绑定与安全门
fn parse_bind(bind: Option<&str>, port: Option<&str>) -> Result<(String, u16), String> {
    let raw = bind.unwrap_or("127.0.0.1");
    validate_bind(raw).map(|bind| {
        let port: u16 = port.and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_PORT);
        (bind.to_string(), port)
    })
}

/// M2-R3 绑定值域校验（纯函数）：空缺 → 127.0.0.1；`localhost` 与合法 IP 地址放行
/// （返回归一化后的 bind）；其余 Err 并回显原值
fn validate_bind(raw: &str) -> Result<&str, String> {
    // None（键缺失）与 Some("")（配置损坏）语义不同：前者回落默认，后者必须报错——
    // 静默把空串当 127.0.0.1 会掩盖写坏设置的真实故障
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("远程绑定地址为空（设置键存在但值为空串）".to_string());
    }
    if trimmed.eq_ignore_ascii_case("localhost") || trimmed.parse::<std::net::IpAddr>().is_ok() {
        Ok(trimmed)
    } else {
        Err(format!(
            "远程绑定地址非法: {raw:?}（仅支持 127.0.0.1 / localhost / ::1 / 合法 IP 地址）"
        ))
    }
}

/// M2-R3 对外判定内核（纯函数）：**白名单反转**——只有「内环可达」的绑定不算对外
/// （127.0.0.0/8 整段、::1、localhost 名单）；其余一切合法地址（0.0.0.0 通配、::
/// 未指定地址、任意具体网卡 IP）一律视为对外、必须过 TLS 确认门。
/// 旧实现只认字面量 "0.0.0.0"，写具体网卡 IP 即绕过 ack 门——正是本函数要堵的口。
/// 输入必须是 parse_bind 校验过的值；防御性兜底：解析失败按对外处理（fail-closed）
fn is_external_bind(bind: &str) -> bool {
    if bind.eq_ignore_ascii_case("localhost") {
        return false;
    }
    match bind.parse::<std::net::IpAddr>() {
        Ok(ip) => !ip.is_loopback(),
        Err(_) => true,
    }
}

/// 句柄存活判定内核（纯函数，不触全局/DB/端口）：只有「句柄存在且其任务仍在运行」
/// 才算存活。任务自行退出（典型：端口被占，`serve` 返回 Err 后任务结束）会留下
/// **已完成**的陈旧句柄——自愈判定：已完成 = 不存在，允许重新 spawn。
fn handle_is_live(h: &Option<tauri::async_runtime::JoinHandle<()>>) -> bool {
    h.as_ref()
        .map(|jh| !jh.inner().is_finished())
        .unwrap_or(false)
}

/// 启动内核（可测核心，外部依赖全部注入）：「查重自愈 → 读设置 → 安全门 → spawn」。
/// 返回是否实际 spawn（false = 幂等跳过）。
/// 自愈动机：服务器任务**自行退出**（典型：9420 端口被占，`serve` 返回 Err，任务内仅
/// 打日志）后 `SERVER_HANDLE` 残留的是**已完成**句柄——若按 `is_some` 视为已启动，
/// 之后的 remote_toggle(true) / restore_on_launch 都会静默返回 Ok 而实际无人监听，
/// remote_status 仍报 enabled=true（SSOT 与现实背离），必须"先关再开"才能恢复。
/// 故把已完成视为不存在：取走陈旧句柄、继续正常 spawn 路径（重开即自愈）；
/// **运行中**的句柄仍幂等跳过（不重复 spawn、不重复绑定）。
fn start_server_core(
    h: &mut Option<tauri::async_runtime::JoinHandle<()>>,
    bind_and_port: impl FnOnce() -> Result<(String, u16), String>,
    public_ack: impl FnOnce() -> Option<String>,
    spawn: impl FnOnce(String, u16) -> tauri::async_runtime::JoinHandle<()>,
) -> Result<bool, String> {
    if handle_is_live(h) {
        return Ok(false); // 运行中：幂等
    }
    // 自愈：取走已完成（或本就为空）的旧句柄
    *h = None;
    // M2-R3：bind 非法（设置被写入乱串）在此即拒绝，不走绑定、不碰安全门
    let (bind, port) = bind_and_port()?;
    // P7 安全门（M2-R3 判定反转）：**除内环白名单外一律视为对外**（is_external_bind），
    // 必须先确认 TLS 前置，否则拒绝对外——旧实现只拦字面量 "0.0.0.0"，写具体网卡
    // IP 即绕过。ack 惰性读取：仅对外绑定时才查 DB
    if is_external_bind(&bind) && !public_ack().map(|v| v == "true").unwrap_or(false) {
        return Err("对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）".into());
    }
    *h = Some(spawn(bind, port));
    Ok(true)
}

/// 启动 axum 服务器（幂等：**运行中**的句柄不重复启动）。
/// 薄壳：SERVER_HANDLE 锁贯穿全程（「查重 → 读设置 → 安全门 → spawn」同锁完成，防并发
/// 双开），DB 读取与真实 spawn（绑端口）以闭包注入 start_server_core，使其可零污染单测
fn start_server() -> Result<(), String> {
    let mut h = SERVER_HANDLE.lock().unwrap();
    let spawned = start_server_core(
        &mut h,
        bind_and_port,
        || crate::database::dao::settings::get_setting(KEY_PUBLIC_ACK),
        |bind, port| {
            // 见 SERVER_HANDLE 注释：必须用 tauri::async_runtime::spawn（sync 命令 /
            // setup 线程无 tokio runtime 上下文）；其 JoinHandle 同样支持 abort
            // （stop_server 语义不变）
            tauri::async_runtime::spawn(async move {
                // M4 T1c：隧道随服务器启动（off/未配置内部直返；失败写快照不阻断——
                // 隧道失败不阻断远程主体）
                tunnel::start_if_configured(port);
                if let Err(e) = server::serve(&bind, port, STATE.clone()).await {
                    log::error!("远程服务器退出: {e}");
                    // serve 失败退出任务时隧道留着无意义，一并停掉
                    tunnel::stop();
                }
            })
        },
    )?;
    if spawned {
        // M4 T3：远程真正开启 → 持电源锁（spec T3：保活跟随远程开关，默认开）；
        // 仅真正 spawn 才持锁（幂等跳过时保活已在持，PowerCore 自身幂等，双保险）
        power::acquire();
    }
    Ok(())
}

/// 停止远程服务：abort 服务器任务 + 全吊销已配对设备 + 清空配对码（不变量 5「全吊销」：
/// `stop()` 只清内存 token，DB 侧吊销由 `revoke_all` 完成，二者缺一不可）。
/// 锁序：SERVER_HANDLE（取走即释放）→ store DB 锁 → pairing 锁——两条锁不并发持有，
/// 与 api::pair 的「pairing 释放后再进 store」（Task 3 已收窄）不构成锁序反转
fn stop_server() {
    if let Some(h) = SERVER_HANDLE.lock().unwrap().take() {
        h.abort();
    }
    // M4 T0a：停止 = 已建立 SSE 连接即时断开（旧限制「仅拒新连」的修复前半；
    // 单设备吊销断连在 Task 8 的 remote_revoke_device 接线）
    STATE.sse_registry.disconnect_all();
    STATE.store.with(|c| {
        let _ = pairing::revoke_all(c); // 停止 = 全吊销（七不变量）
    });
    // M4（评审 Important 同源，裁决外扩展——可回退）：审批队列随 STATE 存活、不随服务器
    // 重启清空，停止 = 全吊销后若不清已消费项，重开远程后旧 requestId 重 poll 仍可复活
    // 已吊销设备（同 purge_approved 防复活语义，spec T0a 即时生效）
    let purged = STATE.approval.lock().unwrap().purge_approved();
    // 终审建议并入：purge 计数走审计留痕（与同文件其它审计事件同风格），不再仅打日志
    events::audit("server_stop_purged_queue", &format!("purged={purged}"));
    STATE.pairing.lock().unwrap().stop();
    // M4 T1c：停服务器时隧道进程一并退出（spec T1b「切换/关闭远程时隧道联动」）
    tunnel::stop();
    // M4 T3：电源锁随远程关闭释放（caffeinate kill / 执行状态清除 + 磁盘代设还原）
    power::release();
}

/// 开关内核（可测核心，SSOT 写入与启停以闭包注入）：写 enabled SSOT → 启/停服务器。
/// 开启分支 start 失败时**回滚 SSOT 为 false** 再传 Err：若不回滚，DB 里 enabled 残留
/// true 而服务器实际没起（典型：TLS 安全门拒绝 / 端口被占），重进设置页开关显示 ON
/// 而无人监听——Task 8 验收场景「TLS 门」的状态撒谎。remote_status 的句柄校准只是
/// 展示层兜底，SSOT 本身必须与现实一致
fn toggle_core(
    enabled: bool,
    // FnMut：开启失败时会被调用两次（写 true + 回滚写 false）
    mut set_enabled: impl FnMut(bool),
    start: impl FnOnce() -> Result<(), String>,
    stop: impl FnOnce(),
) -> Result<(), String> {
    if enabled {
        set_enabled(true);
        if let Err(e) = start() {
            // 回滚：启动失败不得让 SSOT 残留 true（见函数注释）
            set_enabled(false);
            return Err(e);
        }
        Ok(())
    } else {
        set_enabled(false);
        stop();
        Ok(())
    }
}

/// 开关远程接入（前端设置页 invoke）：写设置 SSOT 后启停服务器（失败回滚见 toggle_core）
#[tauri::command]
pub fn remote_toggle(enabled: bool) -> Result<(), String> {
    toggle_core(
        enabled,
        |v| {
            crate::database::dao::settings::set_setting(
                KEY_ENABLED,
                if v { "true" } else { "false" },
            )
        },
        start_server,
        stop_server,
    )?;
    // M4 T1c：成功后广播状态变更（Task 8 的 useRemoteEvents 监听 → 状态页即时刷新）
    events::emit_ui("remote-changed", serde_json::json!({ "enabled": enabled }));
    Ok(())
}

/// enabled 展示校准内核（纯函数）：DB 声明开启**且**服务器句柄存活才算启用。
/// 动机（SSOT 与现实校准）：DB enabled=true 只代表用户意图——若启动失败残留或任务
/// 自退后未自愈，实际无人监听，展示必须以现实为准。正常态不受影响：start_server
/// 成功后句柄是长驻 serve 任务（is_finished=false），必然存活
fn status_enabled(db_enabled: bool, handle_alive: bool) -> bool {
    db_enabled && handle_alive
}

/// 设置页状态展示：enabled / bind / port / url / lanUrls（仅 0.0.0.0 给局域网候选）
/// M3 Task 1 追加 host（P8a 版本 + P8b 本机名 + P8d enabledTools 数据源）
#[tauri::command]
pub fn remote_status() -> serde_json::Value {
    // 展示层：设置里 bind 非法时回落默认值展示（真实启动会在 start_server 被拒）
    let (bind, port) = bind_and_port().unwrap_or_else(|e| {
        log::warn!("remote_status: 设置里的绑定地址非法，按默认展示: {e}");
        ("127.0.0.1".to_string(), DEFAULT_PORT)
    });
    let db_enabled = crate::database::dao::settings::get_setting(KEY_ENABLED)
        .map(|v| v == "true")
        .unwrap_or(false);
    // 短锁：只取 SERVER_HANDLE 的存活快照立即释放，锁内不碰 DB / pairing（不新增嵌套锁序）
    let handle_alive = handle_is_live(&SERVER_HANDLE.lock().unwrap());
    let enabled = status_enabled(db_enabled, handle_alive);
    // LAN 枚举只做一次：候选同时喂 lanUrls 与 0.0.0.0 时的主显示 url（P7 v6 修正）
    let candidates = local_lan_candidates();
    let ips: Vec<String> = candidates.iter().map(|(ip, _)| ip.clone()).collect();
    let lan = lan_urls_for(&bind, ips.clone(), port);
    // host 载荷薄装配：可测内核 host_payload（见下），此处只注入真实依赖
    // （空串/空白设置由 display_host_name 内部过滤，见其注释）
    let mut st = host_payload(
        || crate::database::dao::settings::get_setting(KEY_HOST_NAME),
        crate::database::dao::agent_tool::enabled_tool_ids,
        boot_id(),
    );
    // 原 status 键并入同一返回值（消费方：设置页 RemoteSection + 移动端 /host 直调）
    st["enabled"] = serde_json::json!(enabled);
    st["bind"] = serde_json::json!(bind);
    st["port"] = serde_json::json!(port);
    st["url"] = serde_json::json!(display_url_for(&bind, port, ips));
    st["lanUrls"] = serde_json::json!(lan);
    // 地址表（2026-09-16 用户裁决）：设置页「访问地址」单区块逐条渲染的数据源。
    // M4 T1a：隧道地址恒首位 primary（隧道出错时回退纯局域网表）
    let tun = tunnel::snapshot();
    st["channel"] = serde_json::json!(crate::database::dao::settings::get_setting(KEY_CHANNEL)
        .as_deref()
        .unwrap_or("off"));
    st["tunnelUrl"] = serde_json::json!(tun.url);
    st["tunnelError"] = serde_json::json!(tun.error);
    st["addresses"] = serde_json::json!(address_entries_with_tunnel(
        tun.url.filter(|_| tun.error.is_none()),
        address_entries(&bind, port, &candidates),
    ));
    st
}

/// 本机展示名（纯函数，注入缝：sysinfo 以闭包注入便于测试）：
/// DB 设置（Some 且**非空白**，配置损坏的空串不得顶替真实主机名）
/// > sysinfo 探测 > "MAM" 品牌兜底——双机双子域辨识（P8b）
fn display_host_name(saved: Option<String>, sysinfo: impl FnOnce() -> Option<String>) -> String {
    saved
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(sysinfo)
        .unwrap_or_else(|| "MAM".into())
}

/// 编译目标平台标识（P8）：固定三值，供移动端按平台给提示/图标
fn platform_id() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// host 载荷内核（纯装配，外部依赖全部注入，零 DB 接触可单测）：
///   host: { name, platform, version } + enabledTools（P8d chips 过滤数据源，Task 3 消费）。
/// 返回 serde_json Value 便于 remote_status 原地并入其余 status 键
/// MAM 进程生命周期标识（每次启动重新生成，进程内恒定）：移动端「随进程
/// 消失」的客户端态（消息书签）持久化时打上此标识——MAM 重启后客户端读到
/// 不同的 bootId 即自行清空，页面刷新（同一进程）则原样恢复
static BOOT_ID: Lazy<String> = Lazy::new(|| {
    let mut b = [0u8; 8];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
});

/// 进程生命周期标识（host 载荷下发；只读借用避免克隆）
fn boot_id() -> &'static str {
    &BOOT_ID
}

fn host_payload(
    saved_name: impl FnOnce() -> Option<String>,
    enabled_tools: impl FnOnce() -> Vec<String>,
    boot_id: &str,
) -> serde_json::Value {
    let name = display_host_name(saved_name(), sysinfo::System::host_name);
    serde_json::json!({
        // P8a+P8b：品牌版本号 + 本机名称（双机双子域辨识）
        "host": {
            "name": name,
            "platform": platform_id(),
            "version": env!("CARGO_PKG_VERSION"),
            // 进程生命周期标识（书签修复）：移动端「随进程消失」的客户端态
            // （消息书签）据此区分同一进程与「MAM 已重启」——重启即清空
            "bootId": boot_id,
        },
        "enabledTools": enabled_tools(),
    })
}

/// /m/api/v1/host 移动端装配（api.rs handler 直调）：host 部分与 remote_status 同源
/// （P8 数据同源红线：禁止复制聚合逻辑），状态键（enabled/bind/…）移动端不需要，不返回
fn host_info() -> serde_json::Value {
    host_payload(
        || crate::database::dao::settings::get_setting(KEY_HOST_NAME),
        crate::database::dao::agent_tool::enabled_tool_ids,
        boot_id(),
    )
}

/// 刷新配对二维码：发行新 token（单活跃——发行即作废旧 token，不变量 1）并拼出可扫 URL。
/// host 选取与 remote_status 的 url 字段同源（display_host_for：0.0.0.0 → 局域网首个
/// 或回落 loopback），仅比主显示 url 多拼 `#token=` 配对载荷
#[tauri::command]
pub fn remote_issue_token() -> Result<serde_json::Value, String> {
    let token = STATE.pairing.lock().unwrap().issue();
    let (_, port) = bind_and_port().unwrap_or_else(|_| ("127.0.0.1".to_string(), DEFAULT_PORT));
    // bind 非法（设置被写坏）时按默认 loopback 展示，与 remote_status 口径一致；
    // token 扫码串必须可拨号，故同样走 display_host_for 的 0.0.0.0 → LAN 修正
    let bind =
        crate::database::dao::settings::get_setting(KEY_BIND).unwrap_or_else(|| "127.0.0.1".into());
    let host = display_host_for(&bind, local_lan_ips());
    // M4 T1c：隧道健康（有地址且无 error）时扫码 URL 跟随隧道基址，否则回落 http 直连
    Ok(serde_json::json!({
        "token": token,
        "url": pair_url_with_tunnel(
            tunnel::snapshot().url.filter(|_| tunnel::snapshot().error.is_none()),
            format!("http://{host}:{port}/m"),
            &token,
        )
    }))
}

// ============================================================
// M4 T2：审批配对 / 设备花名册命令（桌面面板数据源与操作入口）
// ============================================================

/// 待审批队列（设置页面板数据源；name/ip/ua/expiresAt/4位码）
#[tauri::command]
pub fn remote_pending_requests() -> serde_json::Value {
    let now = chrono::Utc::now().timestamp_millis();
    let list = STATE.approval.lock().unwrap().pending(now);
    serde_json::json!(list
        .iter()
        .map(|r| serde_json::json!({
            "id": r.id, "name": r.name, "ip": r.ip, "ua": r.ua,
            "code": r.code, "expiresAt": r.expires_at,
        }))
        .collect::<Vec<_>>())
}

/// 桌面批准（spec T2b 路径一）。cap 先于 approve 锁计算（不持审批锁碰 DB）；
/// 上限门经 max_devices_source 注入缝——与 api::pair / pair_confirm 三入口同门（spec T2c）
#[tauri::command]
pub fn remote_approve_request(id: String) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();
    let max = (STATE.max_devices_source)();
    let cap = STATE.store.with(|c| pairing::device_count(c) >= max);
    match STATE.approval.lock().unwrap().approve(&id, now, || cap) {
        approval::ApproveOutcome::Ok { .. } => {
            events::audit("pair_approved", &id);
            Ok(())
        }
        approval::ApproveOutcome::CapFull => Err("设备已满，请先在花名册吊销腾位".into()),
        approval::ApproveOutcome::NotFound | approval::ApproveOutcome::Expired => {
            Err("请求已过期或不存在".into())
        }
    }
}

/// 设备花名册（在线口径 = SSE 注册表 ∨ 30s 过闸；name 直出——直通/审批落库均带设备名）
#[tauri::command]
pub fn remote_devices() -> serde_json::Value {
    let now = chrono::Utc::now().timestamp_millis();
    let rows: Vec<(String, String, i64, i64, i64)> = STATE.store.with(|c| {
        c.prepare("SELECT id, name, first_paired_at, last_seen_at, revoked FROM remote_devices ORDER BY first_paired_at")
            // Rows 借用局部 Statement——必须在闭包内收集为 owned 值（E0515）
            .and_then(|mut s| {
                let rows: Vec<(String, String, i64, i64, i64)> = s
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })?
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            })
            .unwrap_or_default()
    });
    serde_json::json!(rows
        .iter()
        .filter(|(_, _, _, _, revoked)| *revoked == 0)
        .map(|(id, name, paired, seen, _)| {
            serde_json::json!({
                "id": id, "name": name, "firstPairedAt": paired,
                "lastSeenAt": seen, "online": is_online(STATE.sse_registry.has(id), *seen, now),
            })
        })
        .collect::<Vec<_>>())
}

/// 单设备吊销：DB 置位 + 审批队列已消费项清理 + SSE 即时断连（Task 1 注册表接线）+ 审计
#[tauri::command]
pub fn remote_revoke_device(id: String) -> Result<(), String> {
    STATE.store.with(|c| pairing::revoke_device(c, &id))?;
    // M4（评审 Important，spec T0a 即时生效）：清掉审批队列中该设备的已消费项——
    // 否则残留项 TTL 内被旧 requestId 重 poll/confirm 经 INSERT OR REPLACE 复活
    let purged = STATE.approval.lock().unwrap().purge_by_device(&id);
    let n = STATE.sse_registry.disconnect_device(&id);
    events::audit(
        "device_revoked",
        &format!("id={id} closed_sse={n} purged={purged}"),
    );
    events::emit_ui("remote-roster-changed", serde_json::json!({"id": id}));
    Ok(())
}

/// 全部吊销（不停止服务器——与「停止远程」的差别只在服务存续）
#[tauri::command]
pub fn remote_revoke_all_devices() -> Result<usize, String> {
    let n = STATE
        .store
        .with(pairing::revoke_all)
        .map_err(|e| e.to_string())?;
    // M4（评审 Important）：清掉一切已产生设备的消费项（未批准 pending 不动，防复活同上）
    let purged = STATE.approval.lock().unwrap().purge_approved();
    let closed = STATE.sse_registry.disconnect_all();
    events::audit(
        "devices_revoked_all",
        &format!("count={n} closed_sse={closed} purged={purged}"),
    );
    events::emit_ui("remote-roster-changed", serde_json::json!({}));
    Ok(n)
}

/// TLS 前置确认（P7 安全门）：用户确认已配置 TLS 反向代理后置位，解锁 0.0.0.0 绑定
#[tauri::command]
pub fn remote_confirm_public() -> Result<(), String> {
    crate::database::dao::settings::set_setting(KEY_PUBLIC_ACK, "true");
    Ok(())
}

/// 通道设置（M4 T1a）：写 KV；显式停旧启新（E2E S11 修正：off→quick/named 时
/// 隧道从未运行，restart_if_running 的「运行中才重启」会直接空转，隧道永不拉起）；
/// 成功后广播状态变更。校验先行（named 必须已有 Token——先写后验会把非法通道落库）
#[tauri::command]
pub fn remote_set_channel(channel: String, token: Option<String>) -> Result<(), String> {
    let mode = tunnel::parse_channel(Some(&channel))
        .ok_or_else(|| format!("通道值非法: {channel}（仅 off/quick/named）"))?;
    if let Some(t) = token {
        crate::database::dao::settings::set_setting(KEY_TUNNEL_TOKEN, t.trim());
    }
    if mode == tunnel::KEY_CHANNEL_VALUE_NAMED
        && crate::database::dao::settings::get_setting(KEY_TUNNEL_TOKEN)
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
    {
        return Err("命名隧道需要填写 Tunnel Token".into());
    }
    crate::database::dao::settings::set_setting(KEY_CHANNEL, mode);
    let (_, port) = bind_and_port().unwrap_or(("127.0.0.1".into(), DEFAULT_PORT));
    tunnel::stop();
    // 终审 Important：加服务器句柄存活门——远程关闭（enabled=false）时隧道无意义且浪费
    // 资源（会触发 cloudflared 下载并 spawn 指向死端口的公网 URL）；而开启远程的路径
    // 本就会再调 start_if_configured，此门无功能损失。锁短临界区：存活快照取完即放
    if mode != tunnel::KEY_CHANNEL_VALUE_OFF && handle_is_live(&SERVER_HANDLE.lock().unwrap()) {
        tunnel::start_if_configured(port);
    }
    events::emit_ui("remote-changed", serde_json::json!({ "channel": mode }));
    events::audit("channel_set", &format!("channel={mode}"));
    Ok(())
}

// ============================================================
// M4 T4：托盘远程入口（system_tray 消费的展示快照）
// ============================================================

/// 托盘展示纯核（M4 T4）：enabled=false 不给地址（无服务可连）；
/// 隧道开地址优先（T1a 同口径）
pub fn tray_display_from(
    enabled: bool,
    tunnel_url: Option<String>,
    bind_url: String,
) -> (bool, String) {
    if !enabled {
        return (false, String::new());
    }
    (true, tunnel_url.unwrap_or(bind_url))
}

/// 托盘快照（system_tray 消费；与 remote_status 同源不复制聚合——直调各单点）
pub fn tray_display() -> (bool, String) {
    let (bind, port) = bind_and_port().unwrap_or(("127.0.0.1".to_string(), DEFAULT_PORT));
    let enabled = status_enabled(
        crate::database::dao::settings::get_setting(KEY_ENABLED)
            .map(|v| v == "true")
            .unwrap_or(false),
        handle_is_live(&SERVER_HANDLE.lock().unwrap()),
    );
    let tun = tunnel::snapshot();
    tray_display_from(
        enabled,
        tun.url.filter(|_| tun.error.is_none()),
        display_url_for(&bind, port, local_lan_ips()),
    )
}

/// 局域网地址枚举（0.0.0.0 模式给手机可输入的候选）。
/// Bug 7（M3 验收）：旧实现只做 UDP connect 技巧（`connect` 只决定默认对端、
/// **不发包**）——运行时快照最多 1 个 IP（仅默认路由网卡）、无外网路由瞬间返回
/// 空表 → display_host_for 回落 127.0.0.1（= 验收第 2 条偶发不过的根因），多网卡
/// 机器候选缺失。新实现：UDP 默认路由 IP 优先 + sysinfo 全量 UP 网卡枚举追加，
/// 去重保序（UDP 恒首位——display_host_for 取 first 的语义不变）
fn local_lan_candidates() -> Vec<(String, String)> {
    let udp = std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|s| s.connect("8.8.8.8:80").ok().map(|_| s))
        .and_then(|s| s.local_addr().ok())
        .map(|a| a.ip().to_string());
    merge_lan_candidates(udp, enumerated_lan_candidates())
}

/// 候选 IP 表（仅地址，display_host_for / lan_urls_for 的输入形态）
fn local_lan_ips() -> Vec<String> {
    local_lan_candidates()
        .into_iter()
        .map(|(ip, _)| ip)
        .collect()
}

/// sysinfo 全量网卡枚举（Bug 7；2026-09-16 起带网卡名）：仅收非回环 / 非链路
/// 本地 / 非未指定 IPv4。虚拟网卡（WSL / Hyper-V vEthernet）**不排除**——多候选
/// 无害（设置页逐条展示并标注网卡名），主显示 url 仍由 UDP 默认路由首位决定
fn enumerated_lan_candidates() -> Vec<(String, String)> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut out = Vec::new();
    for (name, data) in networks.list() {
        for net in data.ip_networks() {
            if let std::net::IpAddr::V4(v4) = net.addr {
                if !(v4.is_loopback() || v4.is_link_local() || v4.is_unspecified()) {
                    out.push((v4.to_string(), name.clone()));
                }
            }
        }
    }
    out
}

/// LAN 候选合并内核（纯函数，Bug 7 可测核心）：UDP 默认路由恒首位（网卡名从
/// 枚举表反查，查不到给**空串**——前端按语言本地化为「本机」，后端不硬编码文案）
/// → 枚举候选去重保序追加 → 回环 / 非法 / 非 IPv4 剔除。UDP 探测失败或只探到
/// 回环时由枚举结果兜底——消灭「无外网路由瞬间回落 127.0.0.1」的主场景
fn merge_lan_candidates(
    udp: Option<String>,
    enumerated: Vec<(String, String)>,
) -> Vec<(String, String)> {
    fn valid(ip: &str) -> bool {
        ip.parse::<std::net::Ipv4Addr>()
            .map(|v| !(v.is_loopback() || v.is_link_local() || v.is_unspecified()))
            .unwrap_or(false)
    }
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(ip) = udp {
        if valid(&ip) {
            // 网卡名反查失败 → 空串（前端本地化兜底），不因缺名丢候选
            let iface = enumerated
                .iter()
                .find(|(e, _)| *e == ip)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            out.push((ip, iface));
        }
    }
    for (ip, iface) in enumerated {
        if !valid(&ip) || out.iter().any(|(e, _)| *e == ip) {
            continue;
        }
        out.push((ip, iface));
    }
    out
}

/// 主显示 host 选取内核（纯函数，不触网络）：0.0.0.0 通配绑定时取局域网候选首个
/// （枚举失败回落 127.0.0.1——M2 用户实测 0.0.0.0 地址本身不可拨号），其余绑定原样。
/// remote_status 的 `url` 字段与 remote_issue_token 的扫码 URL 同源共用（P7 v6 修正）；
/// 与 lan_urls_for 同为「绑定形态 → 可达地址」口径，风格对齐
fn display_host_for(bind: &str, ips: Vec<String>) -> String {
    if bind == "0.0.0.0" {
        ips.into_iter().next().unwrap_or_else(|| "127.0.0.1".into())
    } else {
        bind.to_string()
    }
}

/// 主显示地址内核（纯函数）：完整可直达 URL（`http://{host}:{port}/m`），
/// host 由 display_host_for 选取（0.0.0.0 → 局域网首个 或 127.0.0.1 兜底）
fn display_url_for(bind: &str, port: u16, ips: Vec<String>) -> String {
    format!("http://{}:{port}/m", display_host_for(bind, ips))
}

/// 局域网候选门控内核（纯函数，不触网络）：仅对外绑定（0.0.0.0）给出候选，
/// loopback / 具体地址绑定恒空——与 start_server 的安全门同一判据。
/// M2-R2：条目为**完整可直达 URL**（`http://{ip}:{port}/m`）而非裸主机名——
/// 设置页展示与复制按钮原样输出该串，裸 IP 手机没法直接用
fn lan_urls_for(bind: &str, ips: Vec<String>, port: u16) -> Vec<String> {
    if bind == "0.0.0.0" {
        ips.into_iter()
            .map(|ip| format!("http://{ip}:{port}/m"))
            .collect()
    } else {
        vec![]
    }
}

/// 地址表内核（纯函数，2026-09-16 用户裁决）：设置页「访问地址」合并为**一个区块**
/// 逐条展示——多网卡机器有两个不同网段的地址（如实测 WLAN 192.168.66.x 与
/// 以太网 192.168.42.x），旧版「访问地址 + 局域网地址」两块并列会被读成重复。
/// 条目：`{url, iface, primary}`，0.0.0.0 时逐候选出（首位 primary=推荐），
/// 空候选回落 loopback；具体地址/loopback 绑定为单条目
fn address_entries(
    bind: &str,
    port: u16,
    candidates: &[(String, String)],
) -> Vec<serde_json::Value> {
    if bind == "0.0.0.0" && !candidates.is_empty() {
        return candidates
            .iter()
            .enumerate()
            .map(|(i, (ip, iface))| {
                serde_json::json!({
                    "url": format!("http://{ip}:{port}/m"),
                    // 网卡名（探测不到为空串——前端本地化兜底，后端不硬编码文案）
                    "iface": iface,
                    "primary": i == 0,
                })
            })
            .collect();
    }
    // 非通配绑定，或通配但无候选（离线）→ 单条目：host 取 display_host_for 同值；
    // iface 空串 = 前端本地化「本机」（非通配）语义
    let host = display_host_for(bind, candidates.iter().map(|(ip, _)| ip.clone()).collect());
    vec![serde_json::json!({
        "url": format!("http://{host}:{port}/m"),
        "iface": "",
        "primary": true,
    })]
}

/// 地址表隧道前插（纯函数，M4 T1a）：隧道地址恒首位 primary；既有条目补 kind="lan"
pub fn address_entries_with_tunnel(
    tunnel_url: Option<String>,
    mut base: Vec<serde_json::Value>,
) -> Vec<serde_json::Value> {
    for e in &mut base {
        if e.get("kind").is_none() {
            e["kind"] = serde_json::json!("lan");
        }
        e["primary"] = serde_json::json!(false); // 隧道在时局域网不再推荐位
    }
    match tunnel_url {
        Some(u) => {
            let mut out = vec![serde_json::json!({
                "url": u, "iface": "", "primary": true, "kind": "tunnel",
            })];
            out.append(&mut base);
            out
        }
        None => {
            if let Some(first) = base.first_mut() {
                first["primary"] = serde_json::json!(true);
            }
            base
        }
    }
}

/// 配对 URL 隧道跟随（纯函数）：隧道开 → 隧道基址/m#token=
pub fn pair_url_with_tunnel(tunnel: Option<String>, base_url: String, token: &str) -> String {
    match tunnel {
        Some(t) => format!("{}/m#token={token}", t.trim_end_matches('/')),
        None => format!("{base_url}#token={token}"),
    }
}

/// 应用启动恢复（lib.rs setup 调用）：开机自启（若启用）。失败仅告警不阻断启动
pub fn restore_on_launch() {
    // M4 T3：电源保活崩溃恢复（Windows 磁盘代设原值未还原时写回并清键；其余平台无值即空操作）
    power::restore_on_launch();
    if crate::database::dao::settings::get_setting(KEY_ENABLED)
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        if let Err(e) = start_server() {
            log::warn!("远程服务自启失败: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_devices_table_idempotent_and_shaped() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 真机调用序：schema::init 建全部表 → migration::migrate 增量迁移（见 database/mod.rs）
        crate::database::schema::init(&conn);
        // 迁移两遍（幂等）
        crate::database::migration::migrate(&conn).unwrap();
        crate::database::migration::migrate(&conn).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(remote_devices)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for c in [
            "id",
            "name",
            "ua",
            "origin_ip",
            "first_paired_at",
            "last_seen_at",
            "revoked",
        ] {
            assert!(cols.contains(&c.to_string()), "缺列 {c}: {cols:?}");
        }
    }

    /// 设置键取值固定（外部依赖：前端 / 移动端约定，不得随实现漂移）
    #[test]
    fn setting_keys_are_stable() {
        assert_eq!(KEY_ENABLED, "remote.enabled");
        assert_eq!(KEY_BIND, "remote.bind");
        assert_eq!(KEY_PORT, "remote.port");
        assert_eq!(KEY_PUBLIC_ACK, "remote.public_ack");
        assert_eq!(KEY_HOST_NAME, "remote.host_name");
        assert_eq!(DEFAULT_PORT, 9420);
    }

    // ==== Task 4 生命周期接线：纯逻辑测试 ====
    // 零污染约束：不触真实 ~/.mam/mam.db、不绑端口、不真正 start_server()。
    // (a)(b) 测的是从 bind_and_port / remote_status 抽出的纯函数内核（DB 读取留在薄壳里）。

    /// (a) bind_and_port 的解析内核：无设置 / 坏端口字符串的回落行为。
    /// M2-R3：bind 值域校验——非法地址字符串一律 Err（端口回落行为保持不变）
    #[test]
    fn parse_bind_falls_back_on_missing_or_bad_port() {
        // 无任何设置 → loopback + 默认端口
        assert_eq!(
            parse_bind(None, None).unwrap(),
            ("127.0.0.1".to_string(), DEFAULT_PORT)
        );
        // 端口非数字 → 回落默认端口
        assert_eq!(
            parse_bind(Some("0.0.0.0"), Some("not-a-port")).unwrap(),
            ("0.0.0.0".to_string(), DEFAULT_PORT)
        );
        // 端口越界（> u16::MAX）→ 同样回落
        assert_eq!(
            parse_bind(Some("0.0.0.0"), Some("99999")).unwrap(),
            ("0.0.0.0".to_string(), DEFAULT_PORT)
        );
        // 合法端口被采用
        assert_eq!(
            parse_bind(Some("127.0.0.1"), Some("9421")).unwrap(),
            ("127.0.0.1".to_string(), 9421)
        );
        // bind 缺失但端口合法：bind 回落、端口采用
        assert_eq!(
            parse_bind(None, Some("8080")).unwrap(),
            ("127.0.0.1".to_string(), 8080)
        );
    }

    /// (a-2) M2-R3：bind 值域校验——解析失败的地址串必须 Err（不得静默透传给绑定/安全门）
    #[test]
    fn parse_bind_rejects_invalid_bind_address() {
        for bad in [
            "not-an-address",
            "999.1.1.1",
            "http://evil",
            "192.168.1.5:9420",
            "",
        ] {
            let r = parse_bind(Some(bad), Some("9420"));
            assert!(r.is_err(), "非法 bind {bad:?} 必须被拒绝");
            assert!(
                r.unwrap_err().contains(bad),
                "错误信息必须回显非法值便于用户定位"
            );
        }
        // 合法形态不误拒：IPv4 / IPv6 / localhost
        for good in [
            "127.0.0.1",
            "0.0.0.0",
            "192.168.1.5",
            "::",
            "::1",
            "fe80::1",
            "localhost",
        ] {
            assert!(
                parse_bind(Some(good), Some("9420")).is_ok(),
                "合法 bind {good:?} 不得被拒绝"
            );
        }
    }

    /// (a-3) M2-R3：对外判定内核——四分支（:: / 具体网卡 IP / 0.0.0.0 / 白名单）
    /// 白名单 = 仅内环可达（127.0.0.1 / localhost / ::1 / 127.0.0.0-8）；其余合法地址
    /// （含未指定地址 :: 与 0.0.0.0、任意网卡 IP）一律视为对外、必须过 ack 门
    #[test]
    fn is_external_bind_four_branches() {
        // 对外三支
        assert!(
            is_external_bind("::"),
            "未指定 IPv6 地址监听全部 v6 接口，属对外"
        );
        assert!(
            is_external_bind("192.168.1.5"),
            "具体网卡 IP 对局域网可达，属对外（旧实现只认 0.0.0.0 字面量，是绕过口）"
        );
        assert!(is_external_bind("0.0.0.0"), "通配绑定属对外");
        // 白名单支：仅内环
        assert!(!is_external_bind("127.0.0.1"));
        assert!(!is_external_bind("localhost"));
        assert!(
            !is_external_bind("::1"),
            "::1 是 IPv6 内环，不可从局域网到达"
        );
        assert!(!is_external_bind("127.0.0.2"), "127.0.0.0/8 整段都是内环");
    }

    /// (b) 局域网候选的门控内核：仅 0.0.0.0（对外）模式给出候选，loopback / 具体地址绑定恒空。
    /// M2-R2：条目是**完整可直达 URL**（http://{ip}:{port}/m），与前端 fixture
    /// （tests/remote/api.test.ts 的 lanUrls 形态）及设置页「展示 + 复制即用」对齐
    #[test]
    fn lan_urls_only_for_wildcard_bind_and_full_url_shape() {
        let ips = vec!["192.168.1.5".to_string(), "10.0.0.2".to_string()];
        assert_eq!(
            lan_urls_for("0.0.0.0", ips.clone(), 9420),
            vec![
                "http://192.168.1.5:9420/m".to_string(),
                "http://10.0.0.2:9420/m".to_string()
            ],
            "0.0.0.0 模式应给出完整可直达 URL（手机复制即用）"
        );
        assert!(
            lan_urls_for("127.0.0.1", ips.clone(), 9420).is_empty(),
            "loopback 模式必须返回空 Vec"
        );
        assert!(
            lan_urls_for("192.168.1.5", ips, 9420).is_empty(),
            "具体地址绑定不属于对外候选场景"
        );
    }

    /// (c-1) toggle 内核的失败回滚（终审修复轮）：开启分支 start 失败（TLS 门拒绝 /
    /// 端口被占）必须把 SSOT 回滚为 false 再原样传出 Err——否则 DB 残留 enabled=true
    /// 而服务器没起，设置页重进显示 ON（状态撒谎）。零污染：SSOT 写入与启停均为
    /// 记录调用的假闭包，不触真实 ~/.mam/mam.db、不绑端口、不碰 SERVER_HANDLE
    #[test]
    fn toggle_core_rolls_back_ssot_when_start_fails() {
        // (1) 开启失败：先写 true，start 报错后回滚 false，Err 原样传出
        let mut log: Vec<&str> = vec![];
        let r = toggle_core(
            true,
            |v| log.push(if v { "set=true" } else { "set=false" }),
            || Err("对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）".into()),
            || panic!("开启分支不得触发 stop"),
        );
        assert!(r.is_err(), "start 失败必须把 Err 传出去");
        assert_eq!(
            log,
            vec!["set=true", "set=false"],
            "启动失败必须把 SSOT 回滚为 false"
        );

        // (2) 开启成功：SSOT 保持 true，不回滚
        let mut log: Vec<&str> = vec![];
        let r = toggle_core(
            true,
            |v| log.push(if v { "set=true" } else { "set=false" }),
            || Ok(()),
            || panic!("开启分支不得触发 stop"),
        );
        assert!(r.is_ok());
        assert_eq!(log, vec!["set=true"], "成功路径不得回滚 SSOT");

        // (3) 关闭：SSOT 写 false + 触发 stop，不触发 start
        let mut log: Vec<&str> = vec![];
        let mut stopped = false;
        let r = toggle_core(
            false,
            |v| log.push(if v { "set=true" } else { "set=false" }),
            || panic!("关闭分支不得触发 start"),
            || stopped = true,
        );
        assert!(r.is_ok());
        assert!(stopped, "关闭必须触发 stop");
        assert_eq!(log, vec!["set=false"], "关闭路径 SSOT 直接写 false");
    }

    /// (c-2) remote_status 的 enabled 校准内核（终审修复轮）：DB 声明与句柄存活
    /// 两者缺一不可——DB=true 但句柄死（启动失败残留 / 任务自退）按未启用展示，
    /// SSOT 与现实背离时以现实为准
    #[test]
    fn status_enabled_requires_db_and_live_handle() {
        assert!(status_enabled(true, true), "DB 开 + 句柄活 → 正常 ON");
        assert!(
            !status_enabled(true, false),
            "DB 开但句柄死 → 按未启用展示（SSOT 与现实校准）"
        );
        assert!(!status_enabled(false, true), "DB 关 → OFF，句柄活也不算");
        assert!(!status_enabled(false, false), "DB 关 + 句柄死 → OFF");
    }

    /// (a-佐证) spawn 用法裁决的运行时证据：本测试线程**不进入任何 tokio runtime 上下文**，
    /// 与 sync tauri 命令线程、lib.rs `.setup()` 主线程同境——裸 `tokio::spawn` 在此会
    /// panic（no reactor running），而 `tauri::async_runtime::spawn`（内部先 enter 再
    /// spawn，tauri async_runtime.rs `Runtime::spawn`）必须正常工作。同时锁定句柄
    /// `abort()` 能力（stop_server 依赖 SERVER_HANDLE 存此类型）
    #[test]
    fn async_runtime_spawn_works_without_runtime_context() {
        let h = tauri::async_runtime::spawn(async { 41 + 1 });
        assert_eq!(
            tauri::async_runtime::block_on(h).unwrap(),
            42,
            "无 runtime 上下文的线程上 spawn 的任务应在 tauri 全局 runtime 上完成"
        );
        // 对已完成任务 abort 是文档化 no-op——无论 abort 与任务完成谁先到，此段都不 panic
        let h2 = tauri::async_runtime::spawn(async {});
        h2.abort();
        let _ = tauri::async_runtime::block_on(h2);
    }

    // ==== M3 Task 2：P7 修正（0.0.0.0 主显示地址改局域网 IP）====
    // 计划里的测试名 url_uses_lan_ip_when_bound_to_all_interfaces 保留，断言落在纯函数
    // display_url_for 上——零污染裁决（延续 Task 1）：brief 原稿直调 remote_status 前
    // set_setting(KEY_BIND, ...) 会写真实 ~/.mam/mam.db，禁止；display_url_for 是
    // remote_status 装配 url 字段的可测内核（DB/网络读取留在薄壳里）

    /// 计划测试名保留：0.0.0.0 通配绑定时主显示 url 必须落到可拨号的局域网 IP
    /// （M2 用户实测 0.0.0.0 地址本身不可连），四分支全覆盖：
    ///   1. 0.0.0.0 + 有局域网 IP → 取第一个；
    ///   2. 0.0.0.0 + 无局域网 IP（离线/无路由）→ 回落 127.0.0.1；
    ///   3. 具体网卡 IP 绑定 → 原样（本身可达）；
    ///   4. loopback 绑定 → 原样
    #[test]
    fn url_uses_lan_ip_when_bound_to_all_interfaces() {
        // 1) 通配绑定 + 局域网候选命中 → 首个 IP
        assert_eq!(
            display_url_for(
                "0.0.0.0",
                9420,
                vec!["192.168.1.5".into(), "10.0.0.2".into()]
            ),
            "http://192.168.1.5:9420/m",
            "0.0.0.0 主显示地址必须为可拨号的局域网 IP（取首个）"
        );
        // 2) 通配绑定 + 枚举失败 → 回落 loopback（不把 0.0.0.0 拼进 url）
        assert_eq!(
            display_url_for("0.0.0.0", 9420, vec![]),
            "http://127.0.0.1:9420/m",
            "无局域网候选时回落 127.0.0.1"
        );
        // 3) 具体网卡 IP 绑定 → 原样透传
        assert_eq!(
            display_url_for("192.168.1.5", 9420, vec![]),
            "http://192.168.1.5:9420/m"
        );
        // 4) loopback 绑定 → 原样透传
        assert_eq!(
            display_url_for("127.0.0.1", 9420, vec![]),
            "http://127.0.0.1:9420/m"
        );
    }

    /// Bug 7（M3 验收）：LAN 候选合并内核——UDP 默认路由恒首位、去重保序、
    /// 回环剔除、UDP 空时枚举兜底（消灭回落 127.0.0.1 的主场景）。
    /// 2026-09-16 起候选带网卡名（设置页逐条标注）
    #[test]
    fn merge_lan_candidates_orders_dedups_and_falls_back() {
        fn cands(list: &[(&str, &str)]) -> Vec<(String, String)> {
            list.iter()
                .map(|(ip, iface)| (ip.to_string(), iface.to_string()))
                .collect()
        }
        // UDP 优先 + 去重保序（枚举中的同 IP 不重复入列）
        assert_eq!(
            merge_lan_candidates(
                Some("192.168.1.5".into()),
                cands(&[
                    ("192.168.1.5", "WLAN"),
                    ("10.0.0.2", "以太网"),
                    ("172.16.0.3", "虚拟网卡"),
                ])
            ),
            cands(&[
                ("192.168.1.5", "WLAN"),
                ("10.0.0.2", "以太网"),
                ("172.16.0.3", "虚拟网卡"),
            ])
        );
        // 回环 / 非法串 / 非 IPv4 剔除
        assert_eq!(
            merge_lan_candidates(
                Some("127.0.0.1".into()),
                cands(&[
                    ("127.0.0.1", "Loopback"),
                    ("10.0.0.2", "以太网"),
                    ("not-an-ip", "x"),
                    ("::1", "v6"),
                ])
            ),
            cands(&[("10.0.0.2", "以太网")])
        );
        // UDP 探测失败（None）→ 枚举兜底（旧实现此场景恒空表 → 回落 127.0.0.1）
        assert_eq!(
            merge_lan_candidates(None, cands(&[("192.168.1.5", "WLAN")])),
            cands(&[("192.168.1.5", "WLAN")])
        );
        // 双双为空 → 空表（display_host_for 仍回落 127.0.0.1 作最后兜底）
        assert!(merge_lan_candidates(None, vec![]).is_empty());
        // UDP 只探到回环（无外网路由的隔离网段）→ 剔除后由枚举兜底
        assert_eq!(
            merge_lan_candidates(Some("127.0.0.1".into()), cands(&[("10.0.0.9", "以太网")])),
            cands(&[("10.0.0.9", "以太网")])
        );
        // UDP 命中的 IP 不在枚举里（罕见：枚举与探测时序不一致）→ 网卡名留空
        // （前端本地化兜底），不得因缺名丢候选
        assert_eq!(
            merge_lan_candidates(Some("192.168.9.9".into()), cands(&[("10.0.0.2", "以太网")])),
            cands(&[("192.168.9.9", ""), ("10.0.0.2", "以太网")])
        );
    }

    /// 2026-09-16 用户裁决：设置页「访问地址」合并为一个区块，逐条标注网卡名——
    /// 地址表内核：0.0.0.0 时逐候选出条目（首位 primary），空候选回落 loopback；
    /// 具体绑定/loopback 单条目；网卡名缺失留空串（文案本地化在前端）
    #[test]
    fn address_entries_labels_iface_and_marks_primary() {
        let cands = vec![
            ("192.168.66.202".to_string(), "WLAN".to_string()),
            ("192.168.42.216".to_string(), "以太网".to_string()),
        ];
        // 0.0.0.0：全部候选逐条出，首位标推荐并带网卡名
        let e = address_entries("0.0.0.0", 9420, &cands);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0]["url"], "http://192.168.66.202:9420/m");
        assert_eq!(e[0]["iface"], "WLAN");
        assert_eq!(e[0]["primary"], true);
        assert_eq!(e[1]["url"], "http://192.168.42.216:9420/m");
        assert_eq!(e[1]["iface"], "以太网");
        assert_eq!(e[1]["primary"], false);
        // 0.0.0.0 且候选为空（离线）→ 单条 loopback 兜底（与 display_url_for 同值）
        let e = address_entries("0.0.0.0", 9420, &[]);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0]["url"], "http://127.0.0.1:9420/m");
        assert_eq!(e[0]["primary"], true);
        // loopback / 具体地址绑定 → 单条目（iface 空串 = 前端「本机」本地化）
        let e = address_entries("127.0.0.1", 9420, &cands);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0]["url"], "http://127.0.0.1:9420/m");
        assert_eq!(e[0]["iface"], "");
        assert_eq!(e[0]["primary"], true);
        // 网卡名缺失（探测不到）→ 空串透传，不硬编码中文文案
        let e = address_entries("0.0.0.0", 9420, &[("10.0.0.5".into(), String::new())]);
        assert_eq!(e[0]["iface"], "");
    }

    /// (c) 陈旧句柄自愈（评审 Important 修复的行为锁定）：经 `start_server_core` 的
    /// **真实分支**驱动（而非只测谓词——那样把查重还原成 is_some 的变异测不出来）。
    /// 零污染：SERVER_HANDLE / STATE 全局不被触碰——句柄槽是局部变量，设置读取与
    /// spawn 均为注入的假闭包（不读真实 ~/.mam/mam.db、不绑任何端口）；pending 任务
    /// 用 abort 清理。锁定的行为矩阵（变异锚点：把核心查重还原为 `is_some` 时
    /// case 2 必红）：
    ///   1. 运行中句柄 → 幂等跳过（不 spawn、不读设置）；
    ///   2. 已完成句柄（服务器自退残留）→ 视为不存在：取走旧句柄并重新 spawn（自愈）；
    ///   3. None → 正常 spawn；
    ///   4. 安全门未因抽核移位：0.0.0.0 且未确认 ack → Err 且不 spawn。
    #[test]
    fn start_server_core_self_heals_finished_handle_and_skips_live_one() {
        // ---- case 1: 运行中句柄 → 幂等跳过（假闭包 panic 证明确实未被调用）----
        let live = tauri::async_runtime::spawn(std::future::pending::<()>());
        let mut slot = Some(live);
        let r = start_server_core(
            &mut slot,
            || panic!("幂等跳过不得读取设置"),
            || panic!("幂等跳过不得读取 ack"),
            |_, _| panic!("幂等跳过不得 spawn"),
        );
        assert!(!r.unwrap(), "运行中句柄必须幂等跳过");
        slot.as_ref().unwrap().abort(); // 清理 pending 任务

        // ---- case 2: 已完成句柄 → 自愈（本轮修复的核心分支）----
        // spawn 一个立即返回的空任务，自旋等它真正结束（不能用 block_on——那会消费句柄）
        let done = tauri::async_runtime::spawn(async {});
        let mut waited = 0u32;
        while !done.inner().is_finished() {
            assert!(waited < 5000, "空任务 5s 内未结束，测试环境异常");
            std::thread::sleep(std::time::Duration::from_millis(1));
            waited += 1;
        }
        let mut slot = Some(done);
        let mut spawned = 0usize;
        let r = start_server_core(
            &mut slot,
            || Ok(("127.0.0.1".to_string(), 12345)),
            || None,
            |b, p| {
                spawned += 1;
                assert_eq!((b.as_str(), p), ("127.0.0.1", 12345));
                tauri::async_runtime::spawn(async {}) // 假 spawn：不绑任何端口
            },
        );
        assert!(
            r.unwrap(),
            "已完成句柄必须视为不存在并重新 spawn（自愈），\
             否则服务器自退后重开会静默失效"
        );
        assert_eq!(spawned, 1, "自愈后必须恰好 spawn 一次");
        assert!(slot.is_some(), "新句柄应写回槽位");
        slot.as_ref().unwrap().abort(); // 清理假任务

        // ---- case 3: None → 正常 spawn ----
        let mut slot: Option<tauri::async_runtime::JoinHandle<()>> = None;
        let mut spawned = 0usize;
        let r = start_server_core(
            &mut slot,
            || Ok(("127.0.0.1".to_string(), DEFAULT_PORT)),
            || panic!("loopback 绑定不应读取 ack"),
            |_, _| {
                spawned += 1;
                tauri::async_runtime::spawn(async {})
            },
        );
        assert!(r.unwrap(), "无句柄应正常 spawn");
        assert_eq!(spawned, 1);
        assert!(slot.is_some(), "新句柄应写回槽位");
        slot.as_ref().unwrap().abort();

        // ---- case 4: 安全门未移位（M2-R3 判定反转）：一切对外绑定（含具体网卡 IP
        //      与 ::）未确认 ack → Err 且不 spawn；内环白名单不误拦 ----
        for external in ["0.0.0.0", "192.168.1.5", "::", "fe80::1"] {
            let mut slot: Option<tauri::async_runtime::JoinHandle<()>> = None;
            let bind = external.to_string();
            let r = start_server_core(
                &mut slot,
                || Ok((bind.clone(), DEFAULT_PORT)),
                || None,
                |_, _| panic!("未确认对外（{external}）时不得 spawn"),
            );
            assert!(r.is_err(), "对外绑定 {external} 未确认 TLS 前置必须拒绝");
            assert!(slot.is_none(), "被安全门拒绝时不得留下句柄（{external}）");
        }
    }

    /// (d) 配对 URL 隧道跟随（Task 5 纯核）：隧道开（无 error）→ 隧道基址/m#token=；
    /// 隧道关/异常 → 既有 http 直连逻辑原样。issue_token 的隧道门控在此锁定
    #[test]
    fn pair_url_prefers_tunnel_base() {
        // 隧道开 → https://host/m#token=…；隧道关 → 既有 http 逻辑
        assert_eq!(
            pair_url_with_tunnel(
                Some("https://mam.example.asia".into()),
                "http://192.168.1.5:9420/m".into(),
                "tok" // clippy useless_conversion 适配（蓝本 "tok".into() 对 &str 参数多余）
            ),
            "https://mam.example.asia/m#token=tok"
        );
        assert_eq!(
            pair_url_with_tunnel(None, "http://192.168.1.5:9420/m".into(), "tok"),
            "http://192.168.1.5:9420/m#token=tok"
        );
    }

    // ==== M4 T2 纯函数：在线口径 + 设备上限解析 ====

    #[test]
    fn online_threshold_30s() {
        assert!(is_online(true, 0, 60_000)); // 活跃 SSE 连接：不看 last_seen
        assert!(is_online(false, 40_000, 60_000)); // 20s 前过闸 → 在线
        assert!(!is_online(false, 20_000, 60_000)); // 40s 前过闸 → 离线
    }

    #[test]
    fn max_devices_default_and_clamp() {
        assert_eq!(max_devices_from(None), 3);
        assert_eq!(max_devices_from(Some("1".into())), 1);
        assert_eq!(max_devices_from(Some("99".into())), 10); // clamp 上限
        assert_eq!(max_devices_from(Some("x".into())), 3); // 乱串回落默认
    }

    // ==== M4 T4 托盘展示纯核 ====

    /// 托盘展示纯核：隧道开（地址有效）→ (true, 隧道地址)；无隧道 → (true,
    /// 绑定口径地址)；enabled=false → (false, 空串)——无服务可连时不给地址
    /// （托盘地址项据此禁用并展示占位「—」）
    #[test]
    fn tray_display_prefers_tunnel_url() {
        // 隧道开 → 隧道地址优先（T1a 同口径）
        assert_eq!(
            tray_display_from(
                true,
                Some("https://mam.example.asia".into()),
                "http://192.168.1.5:9420/m".into()
            ),
            (true, "https://mam.example.asia".into())
        );
        // 开且无隧道 → 绑定口径地址
        assert_eq!(
            tray_display_from(true, None, "http://192.168.1.5:9420/m".into()),
            (true, "http://192.168.1.5:9420/m".into())
        );
        // 关 → 不给地址（enabled=false 时隧道/绑定地址一并丢弃）
        assert_eq!(
            tray_display_from(false, None, "http://127.0.0.1:9420/m".into()),
            (false, String::new())
        );
    }
}

// ============================================================
// M3 Task 1：Host 信息（P8a 品牌版本号 + P8b 本机名 + P8d enabledTools 数据源）
// 测试策略（控制者裁决，零污染最高优先）：可测逻辑抽成纯函数 host_payload /
// display_host_name / platform_id，DB 读取（remote.host_name / enabled_tool_ids）
// 以闭包注入——测试不触全局 DB Lazy，remote_status / host_info 只做薄装配不测。
// 计划里的测试名 remote_status_includes_host_info 保留，断言落在纯函数上。
// ============================================================

#[cfg(test)]
mod host_tests {
    use super::*;

    /// 计划测试名保留：remote_status 的 host 载荷断言落在纯函数 host_payload 上
    /// （零 DB 接触——remote_status 本体是读 settings DAO 的薄装配，集成路径不测）
    #[test]
    fn remote_status_includes_host_info() {
        let st = host_payload(
            || Some("JARVIS-Win".to_string()),
            || vec!["claude".to_string(), "codex".to_string()],
            "boot-test-1",
        );
        let host = st.get("host").expect("remote_status 应含 host 字段");
        assert!(host.get("name").is_some());
        assert!(host.get("version").is_some());
        assert!(
            host.get("platform").is_some(),
            "platform 字段为移动端约定的固定三值之一"
        );
        assert_eq!(host.get("name").unwrap(), "JARVIS-Win");
        assert_eq!(
            host.get("version").unwrap(),
            env!("CARGO_PKG_VERSION"),
            "版本号必须与 crate 版本一致（P8a）"
        );
        // enabledTools（P8d 数据源）随 host 载荷一并返回，Task 3 chips 过滤直接消费
        assert_eq!(
            st.get("enabledTools").unwrap(),
            &serde_json::json!(["claude", "codex"]),
            "enabledTools 应透传 enabled_tool_ids 的结果（按种子顺序）"
        );
        // bootId（书签修复）：随 host 载荷下发，移动端据此守卫「随进程消失」的
        // 客户端态——非空即可（值随机，不锁内容）
        let boot = host.get("bootId").and_then(|b| b.as_str()).unwrap_or("");
        assert!(!boot.is_empty(), "host 载荷必须携带 bootId，实际 {st}");
    }

    /// boot_id 进程内恒定（书签守卫的语义前提：同进程两次读取必须一致）
    #[test]
    fn boot_id_is_stable_within_process() {
        assert_eq!(boot_id(), boot_id());
        assert!(!boot_id().is_empty());
    }

    /// 本机名取值顺序：DB 设置（Some 且非空）> sysinfo > "MAM"（P8b 优先级）
    #[test]
    fn display_host_name_prefers_saved_then_sysinfo_then_fallback() {
        // 1) DB 设置非空 → 直接采用
        assert_eq!(
            display_host_name(Some("JARVIS-Win".into()), || panic!(
                "设置命中时不得回落 sysinfo"
            )),
            "JARVIS-Win"
        );
        // 2) DB 未设置 → sysinfo 命中
        assert_eq!(
            display_host_name(None, || Some("mac-studio".into())),
            "mac-studio"
        );
        // 3) 双双未命中 → "MAM" 品牌兜底
        assert_eq!(display_host_name(None, || None), "MAM");
    }

    /// 空串设置视为未设置（配置损坏不得顶替 sysinfo 真实主机名）
    #[test]
    fn display_host_name_treats_blank_setting_as_unset() {
        assert_eq!(
            display_host_name(Some("".into()), || Some("real-host".into())),
            "real-host",
            "空串设置必须回落 sysinfo（filter 非 empty）"
        );
        assert_eq!(display_host_name(Some("   ".into()), || None), "MAM");
    }

    /// platform 判定（P8）：固定三值之一；本机编译目标 darwin → macos
    #[test]
    fn platform_id_is_one_of_three_values() {
        let p = platform_id();
        assert!(
            ["macos", "windows", "linux"].contains(&p),
            "platform 必须是三值之一，实际 {p}"
        );
        // 编译期判定与运行期取值一致性（darwin/arm64 CI 与本机环境）
        if cfg!(target_os = "macos") {
            assert_eq!(p, "macos");
        } else if cfg!(windows) {
            assert_eq!(p, "windows");
        } else {
            assert_eq!(p, "linux");
        }
    }
}
