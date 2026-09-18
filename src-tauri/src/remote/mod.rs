// 远程接入层（M2）：axum 内嵌服务器 + 访问密码配对（M5）+ 移动看板 API
// 范围与红线见 docs/superpowers/plans/2026-09-17-m5-access-pin-and-file-pool.md

pub mod api;
#[cfg(test)]
pub mod attachment_fixtures;
pub mod conn_owner;
pub mod content;
pub mod events;
pub mod files;
pub mod gate;
pub mod pairing;
pub mod pin;
pub mod power;
pub mod server;
pub mod tunnel;
pub mod watcher;

pub const KEY_ENABLED: &str = "remote.enabled";
/// **M5 A5 起废弃**：bind 由 chan_lan 派生（bind_from_channels），生产路径不再读取；
/// 仅一次性迁移（read_channels）读它一次，之后废弃不删（用户回滚旧版本仍可读得其语义）
pub const KEY_BIND: &str = "remote.bind";
pub const KEY_PORT: &str = "remote.port";
pub const KEY_PUBLIC_ACK: &str = "remote.public_ack";
/// 本机展示名（P8b）：设置里可覆盖 sysinfo 探测值；空串视为未设置
pub const KEY_HOST_NAME: &str = "remote.host_name";
/// 默认端口（避开 3080=dsh / 1420=vite / 18789=zcode）
pub const DEFAULT_PORT: u16 = 9420;
/// **M5 A5 起废弃**：通道选择被三通道独立开关取代（KEY_CHAN_LAN/QUICK/NAMED）；
/// 仅一次性迁移（read_channels）读它一次，之后废弃不删（回滚旧版本仍可读）
pub const KEY_CHANNEL: &str = "remote.channel";
/// 三通道独立开关（M5 A5）：值口径 = **"1" 开 / "0" 关**——写入侧（toggle 命令与
/// 迁移）恒写显式两值；读取侧 channel_flag_from 只认 "1"，其余（"0"/缺键/乱串）一律关
pub const KEY_CHAN_LAN: &str = "remote.chan_lan";
pub const KEY_CHAN_QUICK: &str = "remote.chan_quick";
pub const KEY_CHAN_NAMED: &str = "remote.chan_named";
/// 命名隧道 Tunnel Token（M4 T1b；明文本地存储与设备表同库）
pub const KEY_TUNNEL_TOKEN: &str = "remote.tunnel_token";
/// 设备上限键（spec T2c：默认 10 台可配——M5 A5 用户裁决 3 → 10）
pub const KEY_MAX_DEVICES: &str = "remote.max_devices";
/// M5 P2-c：命名隧道地址记忆——last = 最近一次 stderr 解析成功的完整地址
/// （tunnel.rs 摄取点自动写入）；manual = 用户手填的固定地址（设置页，兜底
/// 「域名解析不到」场景）。两者都进豁免/with 的域名名单与状态展示
pub const KEY_NAMED_ADDR_LAST: &str = "remote.named_addr_last";
/// 访问密码键（M5 A2）：4 位数字（validate_pin 唯一口径；A3 端点 / A4 命令消费）
pub const KEY_ACCESS_PIN: &str = "remote.access_pin";

/// 上限解析（纯函数）：None/乱串 → 10（M5 A5 用户裁决：默认 3 → 10；已存值不迁移）；
/// clamp 1..=10 不变
pub fn max_devices_from(v: Option<String>) -> usize {
    v.and_then(|s| s.trim().parse::<usize>().ok())
        .map(|n| n.clamp(1, 10))
        .unwrap_or(10)
}

/// 生产上限源（STATE 构造注入；唯一 KV 读取点）
fn max_devices_from_kv() -> usize {
    max_devices_from(crate::database::dao::settings::get_setting(KEY_MAX_DEVICES))
}

// ============================================================
// 三通道独立开关（M5 A5）：KV + 惰性迁移 + bind 派生
// ============================================================

/// 三通道开关集合：lan = 局域网监听（0.0.0.0，天然包含回环）；quick/named =
/// cloudflared 隧道。本机通道常驻锁死无开关（前端 A6 只渲染）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelFlags {
    pub lan: bool,
    pub quick: bool,
    pub named: bool,
}

/// 通道种类（remote_toggle_channel 的参数值域）；本机常驻无命令
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelKind {
    Lan,
    Quick,
    Named,
}

impl ChannelKind {
    /// 解析（纯函数）：仅认三字面量，其余 Err（错误文案回显原值）
    fn parse(v: &str) -> Result<Self, String> {
        match v {
            "lan" => Ok(Self::Lan),
            "quick" => Ok(Self::Quick),
            "named" => Ok(Self::Named),
            other => Err(format!("通道值非法: {other}（仅 lan/quick/named）")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Lan => "lan",
            Self::Quick => "quick",
            Self::Named => "named",
        }
    }

    fn flag(self, f: ChannelFlags) -> bool {
        match self {
            Self::Lan => f.lan,
            Self::Quick => f.quick,
            Self::Named => f.named,
        }
    }
}

/// 开关解析（纯函数）：仅 "1" 为开——写入侧恒写显式两值，读侧把 "0"/缺键/乱串
/// 一律判关（防御性：不认第二真值）
fn channel_flag_from(v: Option<String>) -> bool {
    v.as_deref() == Some("1")
}

/// 迁移映射纯函数（M5 A5 一次性、幂等）：旧键值组合 → 三开关。
/// 映射口径（用户裁决）：`remote.bind` 仅 "0.0.0.0" → lan 开（127.0.0.1/缺失/乱串
/// → 关）；`remote.channel` quick/named → 对应开关开，off/缺失/乱串（parse_channel
/// 返回 None）→ 双关。旧 channel 是单值三选一，双开组合不可能由迁移产生
fn migrate_channels_from_legacy(bind: Option<&str>, channel: Option<&str>) -> ChannelFlags {
    ChannelFlags {
        lan: bind == Some("0.0.0.0"),
        quick: tunnel::parse_channel(channel) == Some(tunnel::KEY_CHANNEL_VALUE_QUICK),
        named: tunnel::parse_channel(channel) == Some(tunnel::KEY_CHANNEL_VALUE_NAMED),
    }
}

/// 三通道开关读取唯一入口（含一次性惰性迁移，幂等）。
/// **迁移落库点选型：惰性迁移（本函数）而非 database/migration.rs**——schema 迁移层
/// 不应反向依赖 remote 的 KV 语义；惰性迁移把「判定 + 映射 + 落库」收口在唯一读取点，
/// 首次 status / toggle / 启动恢复触达即完成，且天然幂等。
/// 已迁移判定 = 三新键至少其一存在（迁移恒三键全写含 "0"，不留半迁移态）；
/// 旧键（KEY_BIND / KEY_CHANNEL）在全代码库仅剩本函数这一处读取，读后**废弃不删**
/// （用户回滚旧版本仍可读得其语义）
fn read_channels() -> ChannelFlags {
    use crate::database::dao::settings;
    let lan = settings::get_setting(KEY_CHAN_LAN);
    let quick = settings::get_setting(KEY_CHAN_QUICK);
    let named = settings::get_setting(KEY_CHAN_NAMED);
    if lan.is_some() || quick.is_some() || named.is_some() {
        return ChannelFlags {
            lan: channel_flag_from(lan),
            quick: channel_flag_from(quick),
            named: channel_flag_from(named),
        };
    }
    let flags = migrate_channels_from_legacy(
        settings::get_setting(KEY_BIND).as_deref(),
        settings::get_setting(KEY_CHANNEL).as_deref(),
    );
    for (key, on) in [
        (KEY_CHAN_LAN, flags.lan),
        (KEY_CHAN_QUICK, flags.quick),
        (KEY_CHAN_NAMED, flags.named),
    ] {
        settings::set_setting(key, if on { "1" } else { "0" });
    }
    flags
}

/// 三通道 KV 写入（生产注入；值口径恒 "1"/"0" 显式两值）
fn write_chan_flag(kind: ChannelKind, on: bool) {
    let key = match kind {
        ChannelKind::Lan => KEY_CHAN_LAN,
        ChannelKind::Quick => KEY_CHAN_QUICK,
        ChannelKind::Named => KEY_CHAN_NAMED,
    };
    crate::database::dao::settings::set_setting(key, if on { "1" } else { "0" });
}

/// bind 派生（纯函数，M5 A5）：chan_lan 开 → "0.0.0.0"（对外监听，P7 门随
/// remote_toggle_channel("lan", true) 生效）；关 → "127.0.0.1"。
/// KEY_BIND 旧键废弃后 bind 的唯一来源——0.0.0.0 由局域网开关驱动，不再独立成键
fn bind_from_channels(lan_on: bool) -> &'static str {
    if lan_on {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    }
}

/// 在线口径（spec T2d 自审修正）：活跃 SSE 连接 ∨ 30s 内过闸
pub fn is_online(registry_hit: bool, last_seen_at: i64, now: i64) -> bool {
    registry_hit || now - last_seen_at < 30_000
}

/// 16 字节随机 hex（设备 id 生成器，M5 A3 起 /pair/pin 落行消费）：零参随机形态，
/// 不可位置式（生成器消费后复用会让不同设备撞行）
fn random_hex_16() -> String {
    let mut b = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ============================================================
// 生命周期接线（Task 4）：服务器随设置启停 + tauri 命令 + 启动恢复
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
        // P8 数据同源：直调唯一聚合口（R3 单飞护栏保护第三消费者），禁止复制聚合逻辑
        session_source: Box::new(crate::adapter::get_all_sessions),
        store: pairing::DeviceStore::global(),
        // M7 Task 5（方案 A）：注入器生产装配——消费方 flush_one / session-send 直发；
        // Task 6 已接线：api_router 注册 session-send 等路由 + serve() 挂 spawn_flush_loop
        injector: std::sync::Arc::new(crate::inject::engine::RealInjector),
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
        // M4 T2：设备上限源（生产读 KV——max_devices_from_kv 是唯一读取点）
        max_devices_source: Box::new(max_devices_from_kv),
        // M5 A3：/pair/pin 认证端点接线
        pin_limiter: Mutex::new(pin::PinRateLimiter::new()),
        pin_source: Box::new(pin::get_pin),
        now_source: Box::new(|| chrono::Utc::now().timestamp_millis()),
        // gate 回环豁免 / via 判定的隧道域名源（生产 = 双通道快照聚合抽取；M5 A5）
        tunnel_hosts_source: Box::new(tunnel_hosts_from_snapshot),
        via_hosts_source: Box::new(via_hosts_from_snapshot),
        // M5 P2-a 追记：敏感黑名单的主目录基准（真实 home；取不到时 read_file_safe
        // 走全段保守匹配分支）
        home_source: Box::new(real_home_dir),
    })
});

/// 从看板地址抽域名（纯函数）：快照 url 契约恒为含 /m 的完整地址（board_url 归一），
/// 取 scheme 后、首个 / 前的 host 段，过 gate::normalize_host 归一（小写+剥端口）
fn host_of_board_url(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split('/')
        .next()
        .map(gate::normalize_host)
        .filter(|h| !h.is_empty())
}

/// 单通道域名归集（纯函数）：错误通道不宣称（error ⇒ 无存活 cloudflared，快照里的
/// 旧地址不可信）；url → host_of_board_url 归一。via 分通道与豁免并集共用
fn channel_hosts(c: &tunnel::ChannelStatus) -> Vec<String> {
    if c.error.is_some() {
        return Vec::new();
    }
    c.url
        .as_deref()
        .and_then(host_of_board_url)
        .into_iter()
        .collect()
}

/// 豁免名单内核（纯函数，2026-09-17 穿透修复）：
/// - 任一通道快照**错误终态** → None（既有哨兵，A3 评审 Important 1：错误通道域名
///   不可信，整体 None 让 gate 完全跳过本机豁免，绝不可当空名单处理——那会让 Host
///   条件恒满足 → 回环流量全豁免 fail-open。代价仅错误态下本机也需配对一次。
///   依赖说明：tunnel.rs 现状 error ⇒ 无存活 cloudflared，但豁免判定不押注该不变量）；
/// - 任一通道**在运行而域名缺失** → None（本次新增的 fail-closed，同样关键）：
///   token 模式命名隧道的域名可能解析不到（cloudflared 不打印 https:// 横幅时），
///   空名单会让「Host 不在名单」恒真——cloudflared 从回环转发的**全部**公网流量
///   免密直进（实测穿透：第二台电脑无 cookie 直登）。原则：**豁免永不依赖
///   「名单恰好非空」**，域名未知时收口豁免（本机访问也需配对一次）。
///   变异锚点：删掉 running-without-url 分支 → mod 测试
///   `tunnel_hosts_fail_closed_when_running_without_url` 必红
fn tunnel_hosts_from_status(
    s: &tunnel::TunnelStatus,
    extra_named: &[String],
) -> Option<Vec<String>> {
    if s.quick.error.is_some() || s.named.error.is_some() {
        return None;
    }
    let missing_url_while_running = |c: &tunnel::ChannelStatus| c.running && c.url.is_none();
    if missing_url_while_running(&s.quick) {
        return None;
    }
    // named 域名未知时：无手填/记忆兜底 → 收口（安全侧）；有 → 视为域名已知
    if missing_url_while_running(&s.named) && extra_named.is_empty() {
        return None;
    }
    Some(
        channel_hosts(&s.quick)
            .into_iter()
            .chain(channel_hosts(&s.named))
            .chain(extra_named.iter().cloned())
            .collect(),
    )
}

fn tunnel_hosts_from_snapshot() -> Option<Vec<String>> {
    tunnel_hosts_from_status(&tunnel::snapshot(), &named_extra_hosts())
}

/// 敏感黑名单主目录基准的生产源（提取为具名函数以便接线探针测试）：
/// 端点经 `RemoteState.home_source` 消费它；取不到 home（极端环境）返回 None，
/// `read_file_safe` 随之走全段保守匹配分支（fail-closed）。
fn real_home_dir() -> Option<String> {
    dirs::home_dir().and_then(|h| h.to_str().map(str::to_string))
}

/// 最近一次解析成功的命名地址（P2-c 自动记忆，tunnel.rs 摄取点写入）
fn named_last_addr() -> Option<String> {
    crate::database::dao::settings::get_setting(KEY_NAMED_ADDR_LAST)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 手填/记忆地址的 host 归集（host_of_board_url 归一 + 去重；供豁免与 via 名单）
fn named_extra_hosts() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in [named_last_addr()].into_iter().flatten() {
        if let Some(h) = host_of_board_url(&u) {
            if !out.contains(&h) {
                out.push(h);
            }
        }
    }
    out
}

/// via 判定的分通道域名（生产源）：双通道各自归集（错误通道不宣称——错误通道的
/// 旧域名不得再给新配对设备打通道标签）。**None 哨兵与豁免同源**：名单不可信
/// （错误终态 / 运行中而域名缺失）时返回 None——via 判定侧收到 None 保守标
/// 「局域网」，绝不判「本机」（2026-09-18 实测：名单为空曾把隧道设备标成本机）
fn via_hosts_from_snapshot() -> Option<(Vec<String>, Vec<String>)> {
    via_hosts_from_status(&tunnel::snapshot(), &named_extra_hosts())
}

/// via 判定内核（纯函数，extras 注入）：与豁免名单同判据（哨兵 None → via 保守）
fn via_hosts_from_status(
    s: &tunnel::TunnelStatus,
    extra_named: &[String],
) -> Option<(Vec<String>, Vec<String>)> {
    tunnel_hosts_from_status(s, extra_named)
        .map(|_| (channel_hosts(&s.quick), channel_hosts(&s.named)))
}

/// 读取绑定地址与端口（薄壳：DB 读取在此外置，解析内核抽为纯函数便于单测）。
/// M5 A5：bind 不再读 KEY_BIND（废弃）——由三通道开关派生（read_channels 含惰性迁移
/// → bind_from_channels）。M2-R3：parse_bind 校验保留作纵深防御（派生值恒合法）
fn bind_and_port() -> Result<(String, u16), String> {
    parse_bind(
        Some(bind_from_channels(read_channels().lan)),
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

/// P7 门特征错误文案（M2-R3 起的既有口径，单点常量化）：前端 TLS Dialog（A6 局域网
/// 开关 Dialog 同判据）据此识别「需先确认 TLS 反代」——文案含 remote_confirm_public
/// 指引。start_server_core 内联门与 ensure_public_ack_if_external 共用，防两处漂移
const PUBLIC_ACK_REQUIRED_MSG: &str =
    "对外绑定需先确认已配置 TLS 反向代理（remote_confirm_public）";

/// P7 门检查内核（纯函数）：对外绑定 + 未确认 TLS 前置 → Err 特征文案（沿用
/// start_server_core 口径）；内环绑定或已确认 → Ok。remote_toggle_channel("lan", true)
/// 在热重启前先行检查，让错误在改绑动作发生前暴露
fn ensure_public_ack_if_external(bind: &str, ack: Option<String>) -> Result<(), String> {
    if is_external_bind(bind) && !ack.map(|v| v == "true").unwrap_or(false) {
        return Err(PUBLIC_ACK_REQUIRED_MSG.to_string());
    }
    Ok(())
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
        return Err(PUBLIC_ACK_REQUIRED_MSG.to_string());
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
                // M5 A5：隧道随监听 spawn 逐通道恢复（原 start_if_configured 单通道位；
                // 幂等——restart_listener 路径隧道未被停，已存活句柄在此自然短路）
                restore_enabled_tunnels(port);
                if let Err(e) = server::serve(&bind, port, STATE.clone()).await {
                    log::error!("远程服务器退出: {e}");
                    // serve 失败退出任务时隧道留着无意义（代理目标已无人监听），
                    // 双通道一并停掉
                    tunnel::stop_all();
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

/// 停止内核（可测核心，外部依赖全部注入）：abort 服务器任务 → SSE 全断连 →
/// [仅 revoke] 全吊销设备 → [注入的]隧道停法 → 释放电源锁。
/// **M5 A4 吊销收窄矩阵（调用点 → revoke / 隧道停法取值，全量清单，专测锁定语义）**：
///   - `remote_toggle(false)`（显性关闭远程）→ revoke=`false`（`stop_server_explicit_close`）
///     + 隧道 `stop_all`（全停，重开按各卡状态恢复）。
///
///     **2026-09-18 用户裁决修订**：显性关闭 = 停止对外服务，不再吊销设备——吊销
///     仅剩「重置设备」与「修改访问密码」两个入口（凭当前 PIN 重配对即恢复，
///     设备名保留）；
///   - 热重启（`restart_listener` → `stop_server_hot_restart`，M5 A4 遗留命名化）→
///     revoke=`false` + **隧道不动**（隧道句柄独立于监听进程，随通道开关与总开关启停；
///     监听热重启弹掉隧道会让 lan 开关把 quick 临时隧道换址）；
///   - 重置密码（`remote_set_pin` 改值）/ 重置设备（`remote_reset_devices`）→ 不经停机，
///     走 `revoke_all_and_disconnect`（服务器不停）；
///   - MAM 应用退出/重启（lib.rs `RunEvent::Exit`）→ 本就不调 stop_server（只停隧道
///     `stop_all` + 放电源锁），维持「重启不吊销」现状。
///
/// `revoke=false` = 只停监听，设备记录与 cookie 全部保留——热重启后 cookie 仍过闸。
fn stop_server_core(
    handle: Option<tauri::async_runtime::JoinHandle<()>>,
    revoke: bool,
    disconnect_all: impl FnOnce(),
    revoke_all: impl FnOnce(),
    stop_tunnel: impl FnOnce(),
    release_power: impl FnOnce(),
) {
    if let Some(h) = handle {
        h.abort();
    }
    // M4 T0a：停止 = 已建立 SSE 连接即时断开。热重启路径同样断——监听没了连接必死，
    // 显式断开让注册表即刻一致，不依赖任务 abort 的 Drop 时序
    disconnect_all();
    if revoke {
        revoke_all(); // 显性关闭 = 全吊销（收窄后仅此停机分支吊销）
    }
    stop_tunnel();
    release_power();
}

/// 停止远程服务（吊销与隧道停法参数化）：revoke 语义矩阵见 stop_server_core 注释。
/// M5 A3：旧直通码/审批制的内存态（PairingService token / ApprovalService 队列）已随
/// 模块删除——吊销 = `revoke_all` 一条即足够，已配对设备下次过闸即 403。
/// 锁纪律：**两把锁从不嵌套持有，各自独立短临界区**——SERVER_HANDLE 短锁取走句柄
/// 即释放；store/registry 经 Arc 克隆在锁外的闭包里触达（M5 A5 修正表述：
/// 原注释「registry 永远最后进最先出」与实际顺序不符）
fn stop_server_with(revoke: bool, stop_tunnels: impl FnOnce()) {
    let handle = SERVER_HANDLE.lock().unwrap().take();
    let st_reg = STATE.clone();
    let st_store = STATE.clone();
    stop_server_core(
        handle,
        revoke,
        move || {
            st_reg.sse_registry.disconnect_all();
        },
        move || {
            let _ = st_store.store.with(pairing::revoke_all); // 吊销失败仅忽略，不阻断停机
        },
        stop_tunnels,
        // M4 T3：电源锁随远程关闭释放（caffeinate kill / 执行状态清除 + 磁盘代设还原）
        power::release,
    );
}

/// 停止远程服务（通用停机）：revoke 按调用方语义 + 隧道全停（M5 A5 双通道 stop_all）
fn stop_server(revoke: bool) {
    stop_server_with(revoke, tunnel::stop_all);
}

/// 显性关闭远程的停止路径（remote_toggle(false) 专用）：停止监听 + 隧道全停 +
/// 放电源——**不吊销设备**（2026-09-18 用户裁决修订：吊销触发器收敛为「重置设备」
/// 与「修改访问密码」两个显式动作；开关切换只是停/起服务，重开后已配对设备凭
/// cookie 自动恢复，无需重输密码）。命名函数而非内联闭包：吊销矩阵的每个调用点
/// 在代码里可点名审阅（历史锚点：旧实现误接 revoke=true 曾是裁决内行为，裁决
/// 修订后误接回 true 会让「开关切换掉线」复活——评审时对照本矩阵逐行核对）
fn stop_server_explicit_close() {
    stop_server(false);
}

/// 热重启的停止路径（restart_listener 专用；M5 A4 遗留命名化——原为 restart_listener
/// 内联闭包 `|| stop_server(false)`）：只停监听——不吊销（设备与 cookie 全保留）
/// 且**不停隧道**（见 stop_server_core 矩阵「热重启」行）。
/// **变异锚点**：误接成 `stop_server(true)` → server.rs
/// hot_restart_without_revoke_keeps_device_cookie_valid 必红；误塞隧道停止（如
/// `tunnel::stop_all`）→ lan 开关弹掉 quick 隧道换址（通道开关独立语义破坏）
fn stop_server_hot_restart() {
    stop_server_with(false, || {});
}

/// 热重启内核（可测核心）：仅运行中才有「重启」语义——未运行空转（下次
/// remote_toggle(true) / restore_on_launch 自然按新设置启动）；运行中先停
/// （不吊销）再按当前设置重启。start 失败时旧监听已停：保持停机 + Err 原样上抛
/// （口径裁决见 restart_listener 注释）
fn restart_listener_core(
    live: bool,
    stop: impl FnOnce(),
    start: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if !live {
        return Ok(());
    }
    stop();
    start()
}

/// 改绑定/改端口自动热重启（M5 A4，不吊销）：远程开启状态下，停监听（设备记录与
/// cookie 全保留、**隧道不受扰**）→ 按当前设置重新监听，用户无感、不掉线。
/// 复用既有 stop/start 监听代码路径（stop_server_hot_restart + start_server），
/// 无并行实现。**通用原语**：A5 的 remote_toggle_channel（chan_lan 派生 bind）在其上
/// 接线；不拦通用 set_setting——旧 remote.bind 键已废弃（生产不读），改绑定 = 局域网
/// 开关（前端 A6 收口）。
/// **失败口径（二选一已裁决：保持停机 + 错误上抛，不回落旧 bind/port）**：
/// 回落会让「实际监听地址」与「设置页展示」背离——显示已收窄（如 127.0.0.1）实际
/// 仍对外（或反之）属状态撒谎；且 enabled SSOT 与现实的背离正是 status_enabled
/// 句柄校准 + start 自愈既有机制覆盖的场景，用户解除端口占用后重开一次即恢复。
pub fn restart_listener() -> Result<(), String> {
    // 短锁：只取存活快照立即释放（与 remote_status / remote_toggle_channel 同型）
    let live = handle_is_live(&SERVER_HANDLE.lock().unwrap());
    restart_listener_core(live, stop_server_hot_restart, start_server)
}

/// 逐通道恢复内核（可测核心，M5 A5）：只对开着的隧道通道 ensure（lan 的监听恢复由
/// start_server 承担不在此列；本机通道常驻无动作）
fn restore_tunnels_core(flags: ChannelFlags, mut ensure: impl FnMut(ChannelKind)) {
    if flags.quick {
        ensure(ChannelKind::Quick);
    }
    if flags.named {
        ensure(ChannelKind::Named);
    }
}

/// 恢复开着的隧道通道（总开关 spawn 路径 / remote_toggle(true) 的单点；通道开关
/// remote_toggle_channel 不经此——它直连 tunnel::start_channel 单通道）。
/// 读三通道 KV（含惰性迁移），逐通道 ensure（幂等：已存活句柄短路）
fn restore_enabled_tunnels(port: u16) {
    let flags = read_channels();
    restore_tunnels_core(flags, |k| tunnel::start_channel(k.as_str(), port));
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

/// 开关远程接入（前端设置页 invoke）：写设置 SSOT 后启停服务器（失败回滚见 toggle_core）。
/// M5 A4 吊销收窄：关闭走 stop_server_explicit_close（停服务 + 隧道全停，不吊销——2026-09-18 裁决）；开启分支
/// 不 stop。M5 A5 总开关恢复逻辑：开启成功 → 按各通道 KV 恢复隧道 + PIN 自动生成
/// （关闭 = 全停，重开按各卡状态恢复——用户裁决）
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
        stop_server_explicit_close,
    )?;
    if enabled {
        // 恢复开着的隧道（幂等：start_server spawn 闭包已恢复过一轮，重入切换时
        // 此处兜底——已存活句柄短路）。lan 的监听已由 start_server 按派生 bind 起，
        // 无需额外动作
        let (_, port) = bind_and_port().unwrap_or(("127.0.0.1".into(), DEFAULT_PORT));
        restore_enabled_tunnels(port);
        // PIN 自动生成：未设置才生成写入；已有 PIN 不覆盖（随 remote_status.pin 返回）
        if ensure_pin_auto() {
            events::audit("pin_autogenerated", "master=true");
        }
    }
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

/// 设置页状态展示：enabled / maxDevices / channels（四通道状态）/ pin / host 载荷。
/// M3 Task 1 追加 host（P8a 版本 + P8b 本机名 + P8d enabledTools 数据源）
/// M5 A5 追加 channels（四通道状态）+ pin（当前访问密码）；
/// M5 A8 回归裁决：M3 起的 legacy 键（bind/port/url/lanUrls/channel/tunnelUrl/
/// tunnelError/addresses）经逐键 grep 确认前端零消费（设置页 A6 起唯一数据源是
/// channels），随本任务删除——地址载荷只保留 channels 一条通路，不再双轨漂移
#[tauri::command]
pub fn remote_status() -> serde_json::Value {
    // 展示层：派生 bind + 设置端口（派生值恒合法，unwrap_or 仅端口解析的兜底口径）
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
    // LAN 枚举：喂 channels.lan.addresses（完整可直达 URL 列表）
    let candidates = local_lan_candidates();
    let ips: Vec<String> = candidates.iter().map(|(ip, _)| ip.clone()).collect();
    let lan = lan_urls_for(&bind, ips, port);
    // host 载荷薄装配：可测内核 host_payload（见下），此处只注入真实依赖
    // （空串/空白设置由 display_host_name 内部过滤，见其注释）
    let mut st = host_payload(
        || crate::database::dao::settings::get_setting(KEY_HOST_NAME),
        crate::database::dao::agent_tool::enabled_tool_ids,
        boot_id(),
    );
    // 原 status 键并入同一返回值（消费方：设置页 RemoteSection + 移动端 /host 直调）
    st["enabled"] = serde_json::json!(enabled);
    // 设备上限（线稿「已接入设备 N / 上限」徽标；KV 可改，未设置默认 10——决策 #17）
    st["maxDevices"] = serde_json::json!(max_devices_from_kv());
    // M5 A5：三通道开关（read_channels 含惰性迁移）+ 双通道隧道快照
    let chans = read_channels();
    let tun = tunnel::snapshot();
    // M5 A5：四通道状态（形状契约见 channels_payload 注释）+ 当前访问密码
    // （gate 已保证本载荷只被本机/已过闸前端读到——pin 展示给设置页与看板持有者）
    let mut ch = channels_payload(enabled, chans, port, lan, &tun);
    // M5 P2-c：命名地址记忆/手填透出（named 卡片显示优先级：解析地址 > 手填 > 上次）
    ch["named"]["lastAddr"] = serde_json::json!(named_last_addr());
    st["channels"] = ch;
    st["pin"] = serde_json::json!(pin::get_pin());
    st
}

/// 快照聚合的「首个可用隧道看板地址」（纯函数，quick 优先）：错误通道不宣称——
/// 旧单通道口径 `url.filter(error.is_none())` 的双通道推广。legacy 单地址消费方
/// （托盘 / addresses 表 / tunnelUrl 键）共用；新消费方请直接读 channels 载荷
fn first_available_tunnel_url(tun: &tunnel::TunnelStatus) -> Option<String> {
    tun.quick
        .url
        .clone()
        .filter(|_| tun.quick.error.is_none())
        .or_else(|| tun.named.url.clone().filter(|_| tun.named.error.is_none()))
}

/// 四通道状态载荷内核（纯函数，M5 A5；A6 设置页卡片与 A7 移动端消费**同一形状**——
/// 本注释即契约）：
/// ```json
/// {
///   "local": { "running": bool, "address": "http://127.0.0.1:{port}/m" },
///   "lan":   { "enabled": bool, "running": bool, "addresses": ["http://{ip}:{port}/m", ...] },
///   "quick": { "enabled": bool, "running": bool, "address": str|null, "error": str|null },
///   "named": { "enabled": bool, "running": bool, "address": str|null, "error": str|null }
/// }
/// ```
/// enabled = 三通道 KV 开关；running = 运行态（local/lan 随监听存活，隧道随句柄存活）；
/// 隧道 address = 看板完整地址（**错误态或已停不宣称 → null**；已停门 = 评审 Minor 3
/// 收口：stderr 在途行可落在 stop_channel 快照复位之后，留下 url=Some/running=false
/// 的毫秒级陈旧快照，payload 侧以 running 为门保证已停通道绝不宣称地址——窗口取舍
/// 见 tunnel.rs stderr 摄取点注释）；error = 该通道终态错误。
/// lan.addresses 沿 lan_urls_for 口径（仅 0.0.0.0 非空，完整可直达 URL）
fn channels_payload(
    listener_alive: bool,
    flags: ChannelFlags,
    port: u16,
    lan_addresses: Vec<String>,
    tun: &tunnel::TunnelStatus,
) -> serde_json::Value {
    let chan = |c: &tunnel::ChannelStatus, enabled: bool| {
        serde_json::json!({
            "enabled": enabled,
            "running": c.running,
            // 地址双门（error ∧ running）：A6 消费契约——已停通道绝不宣称地址
            "address": c.url.clone().filter(|_| c.error.is_none() && c.running),
            "error": c.error.clone(),
        })
    };
    serde_json::json!({
        "local": {
            "running": listener_alive,
            "address": format!("http://127.0.0.1:{port}/m"),
        },
        "lan": {
            "enabled": flags.lan,
            "running": listener_alive && flags.lan,
            "addresses": lan_addresses,
        },
        "quick": chan(&tun.quick, flags.quick),
        "named": chan(&tun.named, flags.named),
    })
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

// ============================================================
// 设备花名册命令（桌面面板数据源与操作入口；M5 A3 起配对仅 /pair/pin，
// 审批/直通命令已随密码制下线）
// ============================================================

/// 设备花名册（在线口径 = SSE 注册表 ∨ 30s 过闸；name 直出——配对落库即带设备名）。
/// M5 A8：载荷带出 via（配对时刻接入通道，A1 落库列）——桌面花名册 via 徽标数据源
#[tauri::command]
pub fn remote_devices() -> serde_json::Value {
    let now = chrono::Utc::now().timestamp_millis();
    let rows: Vec<(String, String, String, i64, i64, i64)> = STATE.store.with(|c| {
        c.prepare("SELECT id, name, via, first_paired_at, last_seen_at, revoked FROM remote_devices ORDER BY first_paired_at")
            // Rows 借用局部 Statement——必须在闭包内收集为 owned 值（E0515）
            .and_then(|mut s| {
                let rows: Vec<(String, String, String, i64, i64, i64)> = s
                    .query_map([], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                    })?
                    .filter_map(Result::ok)
                    .collect();
                Ok(rows)
            })
            .unwrap_or_default()
    });
    serde_json::json!(rows
        .iter()
        .filter(|(_, _, _, _, _, revoked)| *revoked == 0)
        .map(|(id, name, via, paired, seen, _)| {
            serde_json::json!({
                "id": id, "name": name, "via": via, "firstPairedAt": paired,
                "lastSeenAt": seen, "online": is_online(STATE.sse_registry.has(id), *seen, now),
            })
        })
        .collect::<Vec<_>>())
}

/// 单设备吊销：DB 置位 + SSE 即时断连（Task 1 注册表接线）+ 审计。
/// M5 A3：审批队列清理随审批制下线——旧审批路径已不存在，无「重 poll 复活」面
#[tauri::command]
pub fn remote_revoke_device(id: String) -> Result<(), String> {
    STATE.store.with(|c| pairing::revoke_device(c, &id))?;
    let n = STATE.sse_registry.disconnect_device(&id);
    events::audit("device_revoked", &format!("id={id} closed_sse={n}"));
    events::emit_ui("remote-roster-changed", serde_json::json!({"id": id}));
    Ok(())
}

/// 吊销全部设备 + SSE 即时断连（M5 A4 单一实现）：`remote_revoke_all_devices` /
/// `remote_reset_devices` / `remote_set_pin`（改值）三入口共用，杜绝「吊销+断连」
/// 逻辑两份漂移。返回 (吊销数, 断连数) 供各入口按自身语义审计
fn revoke_all_and_disconnect() -> Result<(usize, usize), String> {
    let n = STATE
        .store
        .with(pairing::revoke_all)
        .map_err(|e| e.to_string())?;
    let closed = STATE.sse_registry.disconnect_all();
    Ok((n, closed))
}

/// 全部吊销（不停止服务器——与「停止远程」的差别只在服务存续）。
/// M5 A4：吊销+断连机制与 remote_reset_devices / remote_set_pin（改值）共用
/// revoke_all_and_disconnect 单一实现，本命令只保留既有审计语义
#[tauri::command]
pub fn remote_revoke_all_devices() -> Result<usize, String> {
    let (n, closed) = revoke_all_and_disconnect()?;
    events::audit(
        "devices_revoked_all",
        &format!("count={n} closed_sse={closed}"),
    );
    events::emit_ui("remote-roster-changed", serde_json::json!({}));
    Ok(n)
}

/// 重置设备（M5 A4，线稿「重置设备」按钮的新口径命令）：吊销全部设备 + SSE 断连，
/// **不改 PIN**。与 remote_revoke_all_devices 行为同一实现（见 revoke_all_and_disconnect），
/// 差异只在审计语义走 m5 口径（devices_reset）；A6 落 UI 后旧命令的去留由 A8 回归裁决
#[tauri::command]
pub fn remote_reset_devices() -> Result<(), String> {
    let (n, closed) = revoke_all_and_disconnect()?;
    events::audit("devices_reset", &format!("count={n} closed_sse={closed}"));
    events::emit_ui("remote-roster-changed", serde_json::json!({}));
    Ok(())
}

/// remote_set_pin 的结果（审计与事件由命令薄壳按此分派；内核只管判定与落库/吊销）
#[derive(Debug, PartialEq, Eq)]
enum PinSetOutcome {
    /// 首次设置（此前 pin_not_set）。该状态下 /pair/pin 恒 401（pair_pin ②），
    /// 不可能有已配对设备——首次设置无需也无可吊销
    FirstSet,
    /// 改值：全部设备已吊销 + SSE 已断连（裁决「修改后所有设备需重新输入」）
    Changed { reset: usize, closed: usize },
    /// 与旧值一致（trim 后比较）：幂等，不落库、不吊销
    Unchanged,
}

/// 设置访问密码内核（可测核心，KV 写入与吊销重置全部注入）：
/// validate_pin 校验（A2 口径）→ 与旧值（trim 后）比对分派三态。
/// Changed 分支**先重置后写值**：KV 写入（set_setting）无失败形态而吊销可 Err，
/// 先重置保证任一失败路径状态自洽——失败 = 什么都没发生；成功 = 新值生效且
/// 全设备下线，不存在「新值已生效而旧设备 cookie 仍活」的中间态。
/// 非法 PIN：Err 且 write/reset 均不触（不落 KV）。
/// 存储值统一 trim：与 pair_pin 比对口径（两侧 trim）对齐，杜绝空白噪声值入库
fn set_pin_core(
    new_pin: &str,
    old: Option<String>,
    mut write: impl FnMut(&str),
    reset_devices: impl FnOnce() -> Result<(usize, usize), String>,
) -> Result<PinSetOutcome, String> {
    let new = new_pin.trim();
    if !pin::validate_pin(new) {
        return Err("访问密码必须为 4 位数字".to_string());
    }
    match old.as_deref().map(str::trim) {
        None => {
            write(new);
            Ok(PinSetOutcome::FirstSet)
        }
        Some(old_trimmed) if old_trimmed == new => Ok(PinSetOutcome::Unchanged),
        Some(_) => {
            let (reset, closed) = reset_devices()?;
            write(new);
            Ok(PinSetOutcome::Changed { reset, closed })
        }
    }
}

/// 设置访问密码（M5 A4）：首次设置只落库；改值 = 全部设备吊销 + SSE 断连
/// （重置密码 = 全部设备下线）；同值幂等不吊销；非法 PIN Err 且不落 KV
#[tauri::command]
pub fn remote_set_pin(pin: String) -> Result<(), String> {
    let old = pin::get_pin();
    let outcome = set_pin_core(&pin, old, pin::set_pin, revoke_all_and_disconnect)?;
    match outcome {
        PinSetOutcome::FirstSet => events::audit("pin_set", "first=true"),
        PinSetOutcome::Changed { reset, closed } => {
            events::audit(
                "pin_changed",
                &format!("reset_devices={reset} closed_sse={closed}"),
            );
            // 花名册即时刷新（与吊销同一既有事件，不新增事件类型；3s 轮询兜底）
            events::emit_ui("remote-roster-changed", serde_json::json!({}));
        }
        PinSetOutcome::Unchanged => {}
    }
    Ok(())
}

/// 设备重命名内核（可测核心，DAO 注入）：trim 后空 → Err（不触 DAO）；DAO 未命中
/// （返回 false）→ Err 404 语义；命中 → Ok。40 字截断收敛在 DAO（A1 自守，与
/// /pair/pin 设备自报名同一口径），本层不重复截断
fn rename_device_core(
    device_id: &str,
    name_raw: &str,
    rename: impl FnOnce(&str) -> Result<bool, String>,
) -> Result<(), String> {
    let name = name_raw.trim();
    if name.is_empty() {
        return Err("设备名称不能为空".to_string());
    }
    if rename(name)? {
        Ok(())
    } else {
        Err(format!("设备不存在: {device_id}"))
    }
}

/// 设备重命名（M5 A4，桌面端保存）：空名拒绝；未命中 404 语义；超 40 字由 DAO 截断
#[tauri::command]
pub fn remote_rename_device(id: String, name: String) -> Result<(), String> {
    let st = STATE.clone();
    let id_in_store = id.clone();
    rename_device_core(&id, &name, move |n| {
        st.store
            .with(|c| pairing::rename_device(c, &id_in_store, n))
    })?;
    events::audit("device_renamed", &format!("id={id}"));
    // 花名册即时刷新（与单设备吊销同一既有事件，不新增事件类型）
    events::emit_ui("remote-roster-changed", serde_json::json!({ "id": id }));
    Ok(())
}

/// TLS 前置确认（P7 安全门）：用户确认已配置 TLS 反向代理后置位，解锁 0.0.0.0 绑定
#[tauri::command]
pub fn remote_confirm_public() -> Result<(), String> {
    crate::database::dao::settings::set_setting(KEY_PUBLIC_ACK, "true");
    Ok(())
}

// ============================================================
// 三通道独立开关命令（M5 A5）：remote_toggle_channel
// ============================================================

/// PIN 自动生成内核（纯函数）：未设置（None）→ 生成写入返回 true；已有 → 不覆盖
/// 返回 false（用户自设/既有 PIN 恒优先——用户裁决「已有 PIN 不覆盖」）
fn ensure_pin_core(
    existing: Option<String>,
    gen: impl FnOnce() -> String,
    write: impl FnOnce(&str),
) -> bool {
    match existing {
        None => {
            write(&gen());
            true
        }
        Some(_) => false,
    }
}

/// PIN 自动生成（总开关开启 / 任一对外通道开启时调用）：返回是否新生成
fn ensure_pin_auto() -> bool {
    ensure_pin_core(pin::get_pin(), pin::generate_pin, pin::set_pin)
}

/// 通道开关内核的注入依赖束（clippy too_many_arguments 规避 + 语义分组）
struct ChannelToggleDeps<P, W, R, T> {
    /// P7 ack 读取（生产 = KEY_PUBLIC_ACK KV；测试注入 None / Some("true")）
    public_ack: P,
    /// 三通道 KV 写入（生产 = write_chan_flag；测试记录调用序供回滚断言）
    write_flag: W,
    /// 监听热重启（生产 = restart_listener——不吊销不扰隧道；测试记录调用）
    restart: R,
    /// 隧道启停 (kind, on)：on = start_channel ensure / off = stop_channel
    /// （生产直连 tunnel 单点；测试记录调用）
    tunnel: T,
}

/// 通道开关内核（可测核心，M5 A5）：幂等短路 → 写 KV → 总开关开着才联动运行态
/// （lan = P7 门 + 热重启改绑；quick/named = 隧道启停——**隧道启停仅由本命令与
/// 总开关驱动**）。
/// **失败回滚纪律（toggle_core 同款「失败不撒谎」）**：lan on 在门失败 / 重启失败时
/// 回滚 KV 为关再传 Err——不回滚有二患：开关显示开而监听仍 127.0.0.1（状态撒谎）；
/// 用户确认 TLS 后重开会撞上「同向幂等短路」而永不生效。lan off / 隧道路径无失败
/// 形态，不回滚（off 即目标态）。总开关关着时只写 KV（运行态由下次 remote_toggle(true)
/// 按 restore_tunnels_core / 派生 bind 恢复）
fn toggle_channel_core(
    kind: ChannelKind,
    on: bool,
    current: ChannelFlags,
    master_live: bool,
    deps: ChannelToggleDeps<
        impl FnOnce() -> Option<String>,
        impl FnMut(ChannelKind, bool),
        impl FnOnce() -> Result<(), String>,
        impl FnMut(ChannelKind, bool),
    >,
) -> Result<(), String> {
    let ChannelToggleDeps {
        public_ack,
        mut write_flag,
        restart,
        mut tunnel,
    } = deps;
    if kind.flag(current) == on {
        return Ok(()); // 幂等：重复同向调用 no-op（不写 KV、不触运行态）
    }
    write_flag(kind, on);
    if !master_live {
        return Ok(()); // 总开关关着：只落开关位，恢复交给 remote_toggle(true)
    }
    match (kind, on) {
        (ChannelKind::Lan, true) => {
            // P7 门先行（特征文案与 start_server_core 内联门同源单点）——过门才热重启
            if let Err(e) = ensure_public_ack_if_external(bind_from_channels(true), public_ack()) {
                write_flag(ChannelKind::Lan, false);
                return Err(e);
            }
            restart().inspect_err(|_| {
                write_flag(ChannelKind::Lan, false);
            })?;
            Ok(())
        }
        // lan off：bind 回 127.0.0.1 需热重启；重启失败不回滚（off 即目标态，
        // 「监听已停」由 restart_listener 保持停机口径承担）
        (ChannelKind::Lan, false) => restart(),
        // 隧道启停无失败形态（隧道失败写快照不阻断——spec T1d）
        (k, o) => {
            tunnel(k, o);
            Ok(())
        }
    }
}

/// 三通道独立开关（M5 A5）：channel ∈ "lan" | "quick" | "named"（本机常驻锁死无命令，
/// 前端 A6 只渲染）。幂等（重复同向调用 no-op）。
/// **隧道启停唯一入口 = 本命令 + 总开关 remote_toggle**（旧 remote_set_channel 已删）。
/// lan on：P7 门（未确认 TLS → Err 特征文案，前端弹 Dialog）→ restart_listener 改绑
/// 不吊销。通道开启且 PIN 未设置 → 自动生成（随 remote_status.pin 返回，UI 展示可改）。
/// 失败回滚口径见 toggle_channel_core
#[tauri::command]
pub fn remote_toggle_channel(channel: String, on: bool) -> Result<(), String> {
    let kind = ChannelKind::parse(&channel)?;
    let current = read_channels();
    // 短锁：只取监听存活快照立即释放（「总开关开着」的运行态判据——DB enabled=true
    // 但句柄已死时按未运行处理，自愈交给下次 remote_toggle / restore_on_launch）
    let master_live = handle_is_live(&SERVER_HANDLE.lock().unwrap());
    toggle_channel_core(
        kind,
        on,
        current,
        master_live,
        ChannelToggleDeps {
            public_ack: || crate::database::dao::settings::get_setting(KEY_PUBLIC_ACK),
            write_flag: write_chan_flag,
            restart: restart_listener,
            tunnel: |k: ChannelKind, o: bool| {
                let (_, port) = bind_and_port().unwrap_or(("127.0.0.1".into(), DEFAULT_PORT));
                if o {
                    tunnel::start_channel(k.as_str(), port);
                } else {
                    tunnel::stop_channel(k.as_str());
                }
            },
        },
    )?;
    if on && ensure_pin_auto() {
        events::audit("pin_autogenerated", &format!("channel={}", kind.as_str()));
    }
    events::emit_ui(
        "remote-changed",
        serde_json::json!({ "channel": kind.as_str(), "on": on }),
    );
    events::audit(
        "channel_toggled",
        &format!("channel={} on={on}", kind.as_str()),
    );
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
        first_available_tunnel_url(&tun),
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
/// remote_status 的 `url` 字段与托盘地址同源共用（P7 v6 修正；旧 remote_issue_token 已删）；
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

    /// 敏感黑名单基准的生产接线探针（M5 P2-a 追记，独立复核暴露的盲区）：
    /// 端点侧有注入缝探针，但生产 `STATE` 的接线本身此前无任何测试压住——
    /// 把本文件的 `home_source` 改回 `|| None` 时全量 807 测试曾零告警全绿
    /// （端点测试自注入真值，无从暴露生产接线错误）。本探针直接读 `STATE`
    /// 的注入缝，堵住该形态。
    ///
    /// 零污染：`STATE` 构造只存函数指针与内存态（`DeviceStore::Global` 是 ZST，
    /// 不打开 DB——DB 仅在 `.with()` 时锁取）；此处只调 `home_source` 一次
    /// （读 `dirs::home_dir()`），不触真实 ~/.mam、不绑端口。
    #[test]
    fn production_state_home_source_is_wired_to_real_home() {
        let home = (STATE.home_source)();
        let home = home.expect("生产接线必须给出真实 home（黑名单基准，不可为 None）");
        assert!(
            std::path::Path::new(&home).is_absolute(),
            "黑名单基准必须是绝对路径，实际 {home}"
        );
    }

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
            // M5 A1：设备指纹（upsert 去重键）与接入通道（ASCII 枚举）
            "fingerprint",
            "via",
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

    // ==== M4 T2 纯函数：在线口径 + 设备上限解析 ====

    #[test]
    fn online_threshold_30s() {
        assert!(is_online(true, 0, 60_000)); // 活跃 SSE 连接：不看 last_seen
        assert!(is_online(false, 40_000, 60_000)); // 20s 前过闸 → 在线
        assert!(!is_online(false, 20_000, 60_000)); // 40s 前过闸 → 离线
    }

    #[test]
    fn max_devices_default_and_clamp() {
        assert_eq!(max_devices_from(None), 10); // M5 A5 用户裁决：默认 3 → 10
        assert_eq!(max_devices_from(Some("1".into())), 1);
        assert_eq!(max_devices_from(Some("10".into())), 10); // 已存值原样（clamp 内）
        assert_eq!(max_devices_from(Some("99".into())), 10); // clamp 上限
        assert_eq!(max_devices_from(Some("x".into())), 10); // 乱串回落默认（A5 起 = 10）
    }

    // ==== M5 A3 评审修复（Minor 6）+ M5 A5 双通道化：隧道快照 → 域名适配器单测 ====
    // 全局态纪律：via_hosts/tunnel_hosts 适配器读 tunnel::SNAPSHOT 全局——触碰该全局
    // 的用例一律持 tunnel::test_sync::TUNNEL_GLOBALS 互斥锁（与 tunnel.rs 快照测试
    // 共享，默认多线程运行下两文件并发触碰会互踩），且用后还原默认值

    /// host_of_board_url 纯函数边界：scheme + 路径、无 scheme、大写+端口归一、空 host
    #[test]
    fn host_of_board_url_extracts_and_normalizes() {
        assert_eq!(
            host_of_board_url("https://q-test.trycloudflare.com/m"),
            Some("q-test.trycloudflare.com".to_string()),
            "常规形态：scheme + 看板路径 → 域名"
        );
        assert_eq!(
            host_of_board_url("mam.example.com/m"),
            Some("mam.example.com".to_string()),
            "无 scheme（防御：快照契约恒含 scheme，解析不炸即可）"
        );
        assert_eq!(
            host_of_board_url("https://Mam.Example.COM:8443/m"),
            Some("mam.example.com".to_string()),
            "大写 + 带端口 → normalize_host 归一（小写 + 剥端口）"
        );
        assert_eq!(host_of_board_url("https:///m"), None, "空 host → None");
        assert_eq!(host_of_board_url(""), None, "空串 → None");
    }

    /// M5 A5 双通道聚合 + M5 P2-c extras 参数化：quick/named 各自归集（域名归一）；
    /// 错误通道不宣称，豁免/via **同源 None 哨兵**（fail-closed）。走内核
    /// via_hosts_from_status 注入 extras（空表），不读真实 KV
    #[test]
    fn via_hosts_from_snapshot_aggregates_both_channels_and_fails_closed_on_error() {
        use tunnel::TunnelStatus;
        let _g = tunnel::test_sync::TUNNEL_GLOBALS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // 默认态（双通道皆空）：名单为空集但**可信**（无隧道运行，Some 非哨兵）
        tunnel::set_snapshot(|s| *s = TunnelStatus::default());
        assert_eq!(
            via_hosts_from_status(&tunnel::snapshot(), &[]),
            Some((Vec::new(), Vec::new()))
        );
        // 双通道同开：各自归集（quick url 故意大写——归一后入表）
        tunnel::set_snapshot(|s| {
            s.quick = tunnel::ChannelStatus {
                running: true,
                url: Some("https://Q-Test.trycloudflare.com/m".into()),
                error: None,
            };
            s.named = tunnel::ChannelStatus {
                running: true,
                url: Some("https://mam.example.com/m".into()),
                error: None,
            };
        });
        assert_eq!(
            via_hosts_from_status(&tunnel::snapshot(), &[]),
            Some((
                vec!["q-test.trycloudflare.com".to_string()],
                vec!["mam.example.com".to_string()]
            ))
        );
        // 豁免并集 = 双通道域名链式聚合 + 本机命名附加主机（named_extra_hosts 读真实
        // settings，测试进程无法零接触隔离——按机器相关项做相对断言，通道聚合语义不变）
        let mut expected_union = vec![
            "q-test.trycloudflare.com".to_string(),
            "mam.example.com".to_string(),
        ];
        let extras: Vec<String> = named_extra_hosts()
            .into_iter()
            .filter(|h| !expected_union.contains(&h.to_string()))
            .collect();
        expected_union.extend(extras);
        assert_eq!(
            tunnel_hosts_from_snapshot(),
            Some(expected_union),
            "豁免并集 = 双通道域名链式聚合"
        );
        // 错误通道不宣称 + **哨兵同源**：任一通道错误 → via 判定与豁免一起收 None
        // （错误/未知态绝不给 via 提供「本机」判定依据，2026-09-18 实测误标根因）
        tunnel::set_snapshot(|s| {
            s.quick.error = Some("cloudflared 启动失败".into());
        });
        assert_eq!(
            via_hosts_from_snapshot(),
            None,
            "名单不可信 → None 哨兵（via 判定侧保守标 lan，绝不判本机）"
        );
        assert_eq!(
            tunnel_hosts_from_snapshot(),
            None,
            "任一通道快照错误必须返回 None 哨兵（gate 据此跳过豁免，fail-closed）"
        );
        // 还原默认快照（用后即还，不污染其它测试）
        tunnel::set_snapshot(|s| *s = TunnelStatus::default());
    }

    /// 穿透修复专测（2026-09-17 实测事故）：任一通道「运行中而域名缺失」→ 豁免名单
    /// 整体 None（fail-closed）。空名单会让「Host 不在名单」恒真——cloudflared 从
    /// 回环转发的全部公网流量免密直进（第二台电脑无 cookie 直登的根因）。
    /// 变异锚点：删掉 tunnel_hosts_from_status 的 running-without-url 分支本测试必红
    #[test]
    fn tunnel_hosts_fail_closed_when_running_without_url() {
        use tunnel::{ChannelStatus, TunnelStatus};
        // named 在跑（token 模式域名解析不到）→ 名单不可信 → None
        let s = TunnelStatus {
            quick: ChannelStatus {
                running: false,
                url: None,
                error: None,
            },
            named: ChannelStatus {
                running: true,
                url: None,
                error: None,
            },
        };
        assert_eq!(
            tunnel_hosts_from_status(&s, &[]),
            None,
            "运行中而域名缺失 → 豁免整体收口（不可当空名单放行）"
        );
        // 反例边界：通道关着且无 url（正常关闭态）→ 名单为空集而非 None——
        // 此时不存在隧道流量，豁免语义照旧（回环 + 非隧道 Host = 本机免密）
        let off = TunnelStatus {
            quick: ChannelStatus {
                running: false,
                url: None,
                error: None,
            },
            named: ChannelStatus {
                running: false,
                url: None,
                error: None,
            },
        };
        assert_eq!(tunnel_hosts_from_status(&off, &[]), Some(Vec::new()));
    }

    /// P2-c 记忆地址兜底：named 运行而解析不到 url，但 KV 有记忆地址 →
    /// 名单不收口（记忆 host 入名单），豁免 Host 判定恢复精确
    #[test]
    fn tunnel_hosts_remembered_addr_rescues_missing_named_url() {
        use tunnel::{ChannelStatus, TunnelStatus};
        let s = TunnelStatus {
            quick: ChannelStatus {
                running: false,
                url: None,
                error: None,
            },
            named: ChannelStatus {
                running: true,
                url: None,
                error: None,
            },
        };
        assert_eq!(
            tunnel_hosts_from_status(&s, &[]),
            None,
            "无记忆地址 → 收口（域名未知，安全侧）"
        );
        assert_eq!(
            tunnel_hosts_from_status(&s, &["mam-win.bondtoolbox.asia".to_string()]),
            Some(vec!["mam-win.bondtoolbox.asia".to_string()]),
            "有记忆地址 → 名单含其 host（豁免恢复精确，不再 fail-closed）"
        );
    }

    /// 豁免名单正常态：运行且有 url → 名单含其域名（纯函数内核直测——
    /// snapshot 的 running 现算自真实句柄槽，set_snapshot 无法伪造 running=true）
    #[test]
    fn tunnel_hosts_normal_state_lists_running_channel_domain() {
        use tunnel::{ChannelStatus, TunnelStatus};
        let s = TunnelStatus {
            quick: ChannelStatus {
                running: true,
                url: Some("https://demo.trycloudflare.com/m".into()),
                error: None,
            },
            named: ChannelStatus {
                running: false,
                url: None,
                error: None,
            },
        };
        assert_eq!(
            tunnel_hosts_from_status(&s, &[]),
            Some(vec!["demo.trycloudflare.com".to_string()])
        );
    }

    /// first_available_tunnel_url（legacy 单地址聚合口径）：quick 优先；错误通道不宣称；
    /// 双皆空 → None
    #[test]
    fn first_available_tunnel_url_prefers_quick_and_skips_errored() {
        use tunnel::{ChannelStatus, TunnelStatus};
        let mk = |u: Option<&str>, e: Option<&str>| ChannelStatus {
            running: false,
            url: u.map(str::to_string),
            error: e.map(str::to_string),
        };
        // 双开 → quick 优先
        let t = TunnelStatus {
            quick: mk(Some("https://q.trycloudflare.com/m"), None),
            named: mk(Some("https://mam.example.com/m"), None),
        };
        assert_eq!(
            first_available_tunnel_url(&t).as_deref(),
            Some("https://q.trycloudflare.com/m")
        );
        // quick 错误 → 回落 named（错误通道不宣称）
        let t = TunnelStatus {
            quick: mk(Some("https://q.trycloudflare.com/m"), Some("挂了")),
            named: mk(Some("https://mam.example.com/m"), None),
        };
        assert_eq!(
            first_available_tunnel_url(&t).as_deref(),
            Some("https://mam.example.com/m")
        );
        // 双皆空 / 双皆错 → None
        assert_eq!(first_available_tunnel_url(&TunnelStatus::default()), None);
        let t = TunnelStatus {
            quick: mk(None, Some("挂了")),
            named: mk(None, Some("也挂了")),
        };
        assert_eq!(first_available_tunnel_url(&t), None);
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

    // ==== M5 A4：吊销收窄（stop_server(revoke) / 热重启 / 设置密码 / 重命名） ====
    // 零污染约束：真实 stop_server / restart_listener 触碰全局 DB、电源锁（Windows
    // 注册表代设）与隧道进程——测试只驱动注入式内核（stop_server_core /
    // restart_listener_core / set_pin_core / rename_device_core）+ 内存库；
    // gate 级 cookie 回归见 server.rs 的 hot_restart_without_revoke_keeps_device_cookie_valid

    /// 内存库连接（真机调用序：schema::init → migration::migrate，见 DeviceStore::memory 注释）
    fn memory_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        crate::database::migration::migrate(&conn).unwrap();
        conn
    }

    /// 合成测试设备（ua/ip 按 id 派生——upsert 按指纹去重，空值会撞键合并）
    fn synth_device(id: &str, paired_at: i64) -> pairing::NewDevice {
        pairing::NewDevice {
            id: id.into(),
            name: format!("设备-{id}"),
            ua: format!("ua-{id}"),
            origin_ip: format!("ip-{id}"),
            via: "lan".into(),
            paired_at,
        }
    }

    /// 显性关闭语义专测（吊销矩阵 revoke=true 行）：吊销设备 + 断连 + 停隧道 +
    /// 放电源锁，调用序 abort 后依序；abort 分支真实生效（pending 任务随 future
    /// drop 可观测终结——tx 随被取消的任务 drop，rx 端 Disconnected）
    #[test]
    fn stop_server_core_explicit_close_revokes_and_teardowns_in_order() {
        let arc = std::sync::Arc::new(std::sync::Mutex::new(memory_conn()));
        let store = pairing::DeviceStore::Owned(arc.clone());
        let now = chrono::Utc::now().timestamp_millis();
        store.with(|c| pairing::persist_device(c, &synth_device("rv", now)).unwrap());
        assert!(store.with(|c| pairing::device_valid(c, "rv", now)));

        let log: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let ld = log.clone();
        let lt = log.clone();
        let lp = log.clone();
        let store_revoke = store;
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let live = tauri::async_runtime::spawn(async move {
            let _tx = tx; // 任务被 abort 时随 future 一起 drop → rx 端可观测
            std::future::pending::<()>().await;
        });
        stop_server_core(
            Some(live),
            true,
            move || ld.borrow_mut().push("disconnect"),
            move || {
                let _ = store_revoke.with(pairing::revoke_all);
            },
            move || lt.borrow_mut().push("tunnel"),
            move || lp.borrow_mut().push("power"),
        );
        assert_eq!(
            &*log.borrow(),
            &["disconnect", "tunnel", "power"],
            "停止序：abort → 断连 → 吊销 → 停隧道 → 放电源锁（吊销走真实内存库不留日志）"
        );
        // abort 后任务被取消：tx drop → Disconnected（自旋等待调度，上限 5s）
        let mut waited = 0u32;
        loop {
            match rx.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    assert!(waited < 5000, "abort 后 pending 任务 5s 内未终结");
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    waited += 1;
                }
                Ok(v) => panic!("pending 任务不应产出值，实际 {v:?}"),
            }
        }
        // 吊销已在闭包内真实执行：同一内存库上设备已失效（显性关闭 = 全设备下线）
        assert!(
            !pairing::DeviceStore::Owned(arc).with(|c| pairing::device_valid(c, "rv", now)),
            "revoke=true（显性关闭）必须吊销设备"
        );
    }

    /// 吊销矩阵 revoke=false 行：只停机不吊销——设备仍有效（热重启后 cookie 仍过闸
    /// 的核心等价物）；吊销闭包以 panic 证明确实未被调用。真实 store/registry 闭包
    /// 形态见 server.rs 的 gate 级回归
    #[test]
    fn stop_server_core_hot_restart_keeps_devices_valid() {
        let arc = std::sync::Arc::new(std::sync::Mutex::new(memory_conn()));
        let store = pairing::DeviceStore::Owned(arc.clone());
        let now = chrono::Utc::now().timestamp_millis();
        store.with(|c| pairing::persist_device(c, &synth_device("keep", now)).unwrap());
        stop_server_core(
            None,
            false,
            || {},
            || panic!("revoke=false（热重启）不得吊销设备"),
            || {},
            || {},
        );
        assert!(
            store.with(|c| pairing::device_valid(c, "keep", now)),
            "revoke=false 后设备必须仍有效（cookie 不掉线）"
        );
    }

    /// 热重启内核：未运行空转（stop/start 均不触，panic 闭包证明）；运行中先停后启
    /// （共享日志锁顺序）
    #[test]
    fn restart_listener_core_gates_on_live_and_stops_before_start() {
        // 未运行：无「重启」语义——下次开启自然按新设置启动
        let r = restart_listener_core(
            false,
            || panic!("未运行不得 stop"),
            || panic!("未运行不得 start"),
        );
        assert!(r.is_ok(), "未运行时热重启空转返回 Ok");

        // 运行中：先 stop（revoke=false 路径）后 start，顺序锁定
        let log: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let ls = log.clone();
        let lg = log.clone();
        let r = restart_listener_core(
            true,
            move || ls.borrow_mut().push("stop"),
            move || {
                lg.borrow_mut().push("start");
                Ok(())
            },
        );
        assert!(r.is_ok());
        assert_eq!(&*log.borrow(), &["stop", "start"], "先停（不吊销）后启");
    }

    /// 热重启失败口径（二选一裁决：保持停机 + 错误上抛，不回落旧 bind/port）：
    /// start Err 原样传出，stop 恰好执行一次——不静默重试、不回落重启
    #[test]
    fn restart_listener_core_propagates_start_failure_after_single_stop() {
        let stops = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let s = stops.clone();
        let r = restart_listener_core(
            true,
            move || s.set(s.get() + 1),
            || Err("绑定 127.0.0.1:9420 失败: 端口被占".into()),
        );
        assert_eq!(
            r.unwrap_err(),
            "绑定 127.0.0.1:9420 失败: 端口被占",
            "启动失败原样上抛（端口占用等错误透传给调用方展示）"
        );
        assert_eq!(stops.get(), 1, "失败路径 stop 恰好一次（保持停机语义）");
    }

    /// set_pin 内核：非法 PIN → Err 且写值/吊销均不触（不落 KV；panic 闭包证明）
    #[test]
    fn set_pin_core_invalid_pin_touches_nothing() {
        for bad in ["12", "12345", "12a4", "", "   ", "１２３４", " 12 "] {
            let r = set_pin_core(
                bad,
                None,
                |_| panic!("非法 PIN {bad:?} 不得写 KV"),
                || panic!("非法 PIN {bad:?} 不得吊销"),
            );
            assert!(r.is_err(), "非法 PIN {bad:?} 必须拒绝");
        }
    }

    /// set_pin 内核：首次设置（旧值 None）只写值、不吊销——pin_not_set 时 /pair/pin
    /// 恒 401，不可能有已配对设备，无可吊销；写入 trim 后的规范值
    #[test]
    fn set_pin_core_first_set_writes_without_reset() {
        let mut written: Vec<String> = vec![];
        let r = set_pin_core(
            " 5678 ",
            None,
            |p| written.push(p.to_string()),
            || panic!("首次设置不得吊销（无可吊销设备）"),
        );
        assert_eq!(r.unwrap(), PinSetOutcome::FirstSet);
        assert_eq!(
            written,
            vec!["5678"],
            "写入 trim 后的规范值（比对口径无空白噪声）"
        );
    }

    /// set_pin 内核：同值幂等（trim 后比较）——不重写 KV、不吊销设备
    #[test]
    fn set_pin_core_same_value_is_idempotent_noop() {
        let r = set_pin_core(
            "1234",
            Some("1234".into()),
            |_| panic!("同值不得重写 KV"),
            || panic!("同值不得吊销设备"),
        );
        assert_eq!(r.unwrap(), PinSetOutcome::Unchanged);
        // 带空白的同值同样幂等（比对与存储统一 trim）
        let r = set_pin_core(
            " 1234 ",
            Some("1234".into()),
            |_| panic!("同值不得重写 KV"),
            || panic!("同值不得吊销设备"),
        );
        assert_eq!(r.unwrap(), PinSetOutcome::Unchanged);
    }

    /// set_pin 内核：改值 = 先吊销断连（重置密码 = 全部设备下线）后写新值，
    /// 顺序锁定（先重置后写：KV 写无失败形态，不存在「新值已生效而旧 cookie 仍活」
    /// 的中间态）
    #[test]
    fn set_pin_core_changed_resets_all_then_writes() {
        let order: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let ow = order.clone();
        let or = order.clone();
        let r = set_pin_core(
            "9999",
            Some("1234".into()),
            move |p| {
                ow.borrow_mut()
                    .push(if p == "9999" { "write" } else { "write-bad" })
            },
            move || {
                or.borrow_mut().push("reset");
                Ok((2, 3))
            },
        );
        assert_eq!(
            r.unwrap(),
            PinSetOutcome::Changed {
                reset: 2,
                closed: 3
            },
            "改值回传吊销/断连计数供审计"
        );
        assert_eq!(
            &*order.borrow(),
            &["reset", "write"],
            "先重置后写值（失败路径状态自洽）"
        );
    }

    /// set_pin 内核：改值时重置失败 → Err 上传且不写值（先重置后写的自洽性：
    /// 失败 = 什么都没发生，旧 PIN 与旧设备俱在）
    #[test]
    fn set_pin_core_reset_failure_propagates_without_write() {
        let r = set_pin_core(
            "9999",
            Some("1234".into()),
            |_| panic!("重置失败不得写值"),
            || Err("吊销失败".into()),
        );
        assert_eq!(r.unwrap_err(), "吊销失败");
    }

    /// 重命名内核：空名/纯空白 → Err 且不触 DAO（panic 闭包证明）
    #[test]
    fn rename_device_core_blank_name_rejected_without_dao_call() {
        for bad in ["", "   ", "\t\n"] {
            let r = rename_device_core("d1", bad, |_| panic!("空名 {bad:?} 不得触 DAO"));
            assert!(r.is_err(), "空名 {bad:?} 必须拒绝");
        }
    }

    /// 重命名内核：未命中（DAO false）→ Err 404 语义，文案含设备 id 便于定位
    #[test]
    fn rename_device_core_miss_maps_to_not_found() {
        let r = rename_device_core("no-such", "新名字", |_| Ok(false));
        let e = r.unwrap_err();
        assert!(
            e.contains("设备不存在") && e.contains("no-such"),
            "404 语义文案应含设备 id: {e}"
        );
    }

    /// 重命名内核：命中 → Ok；经真实 DAO（内存库）端到端——50 字截断为 40
    /// （A1 DAO 自守贯穿命令内核，命令层不重复截断）；trim 生效
    #[test]
    fn rename_device_core_success_and_dao_truncation_end_to_end() {
        let conn = memory_conn();
        let now = chrono::Utc::now().timestamp_millis();
        let dev = synth_device("rn", now);
        pairing::persist_device(&conn, &dev).unwrap();
        rename_device_core("rn", &format!("  {}  ", "甲".repeat(50)), |n| {
            pairing::rename_device(&conn, "rn", n)
        })
        .unwrap();
        let stored: String = conn
            .query_row("SELECT name FROM remote_devices WHERE id = 'rn'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, "甲".repeat(40), "DAO 40 字截断贯穿命令内核");
    }

    // ==== M5 A5：三通道独立开关（迁移映射 / bind 派生 / toggle 内核 / 恢复 / PIN / 载荷） ====
    // 零污染约束：read_channels / write_chan_flag / 真实命令触全局 DB——测试只驱动
    // 纯函数与注入式内核；tunnel 启停/快照全局的用例持 tunnel::test_sync 锁

    /// 迁移映射纯函数全矩阵：旧 bind/channel 值组合 → 三开关（含缺失/乱值回落）。
    /// 口径：仅 "0.0.0.0" → lan 开；仅 quick/named 字面量 → 对应隧道开；
    /// off/缺失/乱串（parse_channel None）→ 双关
    #[test]
    fn migrate_channels_from_legacy_full_matrix() {
        let all_off = ChannelFlags::default();
        // bind 维度：0.0.0.0 → lan on；127.0.0.1 / localhost / 乱串 / 缺失 → lan off
        assert_eq!(
            migrate_channels_from_legacy(Some("0.0.0.0"), None),
            ChannelFlags {
                lan: true,
                quick: false,
                named: false
            }
        );
        for b in [
            Some("127.0.0.1"),
            Some("localhost"),
            Some("192.168.1.5"),
            Some(""),
            Some("999.1.1.1"),
            None,
        ] {
            assert_eq!(
                migrate_channels_from_legacy(b, None),
                all_off,
                "bind {b:?} 不得映射出任何开关"
            );
        }
        // channel 维度：quick / named / off / 缺失 / 乱串（大小写敏感拒绝 → None → off）
        assert_eq!(
            migrate_channels_from_legacy(None, Some("quick")),
            ChannelFlags {
                lan: false,
                quick: true,
                named: false
            }
        );
        assert_eq!(
            migrate_channels_from_legacy(None, Some("named")),
            ChannelFlags {
                lan: false,
                quick: false,
                named: true
            }
        );
        for c in [Some("off"), Some(""), Some("QUICK"), Some("tls"), None] {
            assert_eq!(
                migrate_channels_from_legacy(None, c),
                all_off,
                "channel {c:?} 不得映射出任何开关"
            );
        }
        // 组合：双通道同开不可能由单值旧键迁移产生（0.0.0.0 + named 各归各位）
        assert_eq!(
            migrate_channels_from_legacy(Some("0.0.0.0"), Some("named")),
            ChannelFlags {
                lan: true,
                quick: false,
                named: true
            }
        );
    }

    /// 开关解析口径：仅 "1" 为开；"0" / 空串 / 乱串 / 缺键一律关
    #[test]
    fn channel_flag_from_only_one_means_on() {
        assert!(channel_flag_from(Some("1".into())));
        for v in [
            Some("0".to_string()),
            Some("".to_string()),
            Some("true".to_string()),
            None,
        ] {
            assert!(!channel_flag_from(v.clone()), "{v:?} 必须判关");
        }
    }

    /// bind 派生：lan 开 → 0.0.0.0（对外，P7 门在 toggle_channel 生效）；关 → 127.0.0.1
    #[test]
    fn bind_from_channels_derives_wildcard_or_loopback() {
        assert_eq!(bind_from_channels(true), "0.0.0.0");
        assert_eq!(bind_from_channels(false), "127.0.0.1");
    }

    /// 通道命令参数值域：仅认三字面量，其余 Err 且回显原值
    #[test]
    fn channel_kind_parse_rejects_unknown_values() {
        assert_eq!(ChannelKind::parse("lan"), Ok(ChannelKind::Lan));
        assert_eq!(ChannelKind::parse("quick"), Ok(ChannelKind::Quick));
        assert_eq!(ChannelKind::parse("named"), Ok(ChannelKind::Named));
        for bad in ["off", "local", "QUICK", ""] {
            let e = ChannelKind::parse(bad).unwrap_err();
            assert!(e.contains(bad), "错误信息必须回显非法值便于定位: {e}");
        }
    }

    /// 通道开关内核：重复同向调用幂等 no-op——写 KV / 重启 / 隧道启停全不触
    /// （panic 闭包证明），总开关开着也一样
    #[test]
    fn toggle_channel_core_same_direction_is_noop() {
        // lan 已开再开（总开关开着）：全不触
        let r = toggle_channel_core(
            ChannelKind::Lan,
            true,
            ChannelFlags {
                lan: true,
                ..Default::default()
            },
            true,
            ChannelToggleDeps {
                public_ack: || panic!("幂等 no-op 不得读 ack"),
                write_flag: |_, _| panic!("幂等 no-op 不得写 KV"),
                restart: || panic!("幂等 no-op 不得重启监听"),
                tunnel: |_, _| panic!("幂等 no-op 不得触隧道"),
            },
        );
        assert!(r.is_ok());
        // quick 已关再关（总开关关着）：同样全不触
        let r = toggle_channel_core(
            ChannelKind::Quick,
            false,
            ChannelFlags::default(),
            false,
            ChannelToggleDeps {
                public_ack: || panic!("幂等 no-op 不得读 ack"),
                write_flag: |_, _| panic!("幂等 no-op 不得写 KV"),
                restart: || panic!("幂等 no-op 不得重启监听"),
                tunnel: |_, _| panic!("幂等 no-op 不得触隧道"),
            },
        );
        assert!(r.is_ok());
    }

    /// 通道开关内核：lan on 且总开关开着时 P7 门拦停——未确认 TLS → Err 特征文案
    /// （与 start_server_core 同源常量），KV 回滚为关（先写 true 后回滚 false），
    /// 重启不触。ack=Some("false") 同拦（与 start 门同判据）
    #[test]
    fn toggle_channel_core_lan_on_requires_public_ack_and_rolls_back() {
        for ack in [None, Some("false".to_string())] {
            let writes: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> =
                Default::default();
            let lw = writes.clone();
            let r = toggle_channel_core(
                ChannelKind::Lan,
                true,
                ChannelFlags::default(),
                true,
                ChannelToggleDeps {
                    public_ack: move || ack.clone(),
                    write_flag: move |k: ChannelKind, v: bool| {
                        lw.borrow_mut().push((k.as_str(), v))
                    },
                    restart: || panic!("未过 P7 门不得重启监听"),
                    tunnel: |_, _| panic!("lan 开关不得触隧道"),
                },
            );
            let err = r.unwrap_err();
            assert_eq!(
                err, PUBLIC_ACK_REQUIRED_MSG,
                "特征错误码（前端 TLS Dialog 判据）必须逐字一致"
            );
            assert_eq!(
                &*writes.borrow(),
                &[("lan", true), ("lan", false)],
                "门失败必须回滚 KV（否则确认后重开被同向幂等短路吞掉）"
            );
        }
    }

    /// 通道开关内核：lan on 过 P7 门 → 热重启恰好一次（不吊销语义由
    /// restart_listener → stop_server_hot_restart 承担，gate 级回归见 server.rs
    /// hot_restart_without_revoke_keeps_device_cookie_valid）；重启失败 → KV 回滚 +
    /// Err 原样上抛（保持停机不撒谎）
    #[test]
    fn toggle_channel_core_lan_on_restarts_then_rolls_back_on_failure() {
        // 过门成功：写 KV 一次 + 重启一次，Ok
        let writes: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lw = writes.clone();
        let restarts = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let lr = restarts.clone();
        let r = toggle_channel_core(
            ChannelKind::Lan,
            true,
            ChannelFlags::default(),
            true,
            ChannelToggleDeps {
                public_ack: || Some("true".into()),
                write_flag: move |k: ChannelKind, v: bool| lw.borrow_mut().push((k.as_str(), v)),
                restart: move || {
                    lr.set(lr.get() + 1);
                    Ok(())
                },
                tunnel: |_, _| panic!("lan 开关不得触隧道"),
            },
        );
        assert!(r.is_ok());
        assert_eq!(&*writes.borrow(), &[("lan", true)]);
        assert_eq!(restarts.get(), 1, "过门后必须恰好热重启一次");

        // 重启失败（如端口被占）：KV 回滚，Err 原样传出
        let writes: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lw = writes.clone();
        let r = toggle_channel_core(
            ChannelKind::Lan,
            true,
            ChannelFlags::default(),
            true,
            ChannelToggleDeps {
                public_ack: || Some("true".into()),
                write_flag: move |k: ChannelKind, v: bool| lw.borrow_mut().push((k.as_str(), v)),
                restart: || Err("绑定 0.0.0.0:9420 失败: 端口被占".into()),
                tunnel: |_, _| panic!("lan 开关不得触隧道"),
            },
        );
        assert_eq!(r.unwrap_err(), "绑定 0.0.0.0:9420 失败: 端口被占");
        assert_eq!(
            &*writes.borrow(),
            &[("lan", true), ("lan", false)],
            "重启失败必须回滚 KV（失败不撒谎）"
        );
    }

    /// 通道开关内核：lan off 走重启改绑回 127.0.0.1；重启失败**不回滚**（off 即目标态，
    /// 监听已停由 restart_listener 保持停机口径承担），Err 上抛
    #[test]
    fn toggle_channel_core_lan_off_restarts_without_rollback_on_failure() {
        let writes: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lw = writes.clone();
        let r = toggle_channel_core(
            ChannelKind::Lan,
            false,
            ChannelFlags {
                lan: true,
                ..Default::default()
            },
            true,
            ChannelToggleDeps {
                public_ack: || panic!("lan off 不读 ack 门"),
                write_flag: move |k: ChannelKind, v: bool| lw.borrow_mut().push((k.as_str(), v)),
                restart: || Err("绑定 127.0.0.1:9420 失败: 端口被占".into()),
                tunnel: |_, _| panic!("lan 开关不得触隧道"),
            },
        );
        assert!(r.is_err(), "重启失败原样上抛");
        assert_eq!(
            &*writes.borrow(),
            &[("lan", false)],
            "off 是目标态：失败也不回滚成 true"
        );
    }

    /// 通道开关内核：quick/named 启停只触隧道（kind, on 透传），不触重启与 ack 门；
    /// 总开关关着时只写 KV，运行态不触（恢复交给 remote_toggle(true)）
    #[test]
    fn toggle_channel_core_tunnel_lifecycle_gated_on_master() {
        // quick on（总开关开着）→ tunnel(Quick, true)
        let calls: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lc = calls.clone();
        let r = toggle_channel_core(
            ChannelKind::Quick,
            true,
            ChannelFlags::default(),
            true,
            ChannelToggleDeps {
                public_ack: || panic!("隧道开关不读 ack 门"),
                write_flag: |_, _| {},
                restart: || panic!("隧道开关不得重启监听"),
                tunnel: move |k: ChannelKind, o: bool| {
                    lc.borrow_mut().push((k.as_str(), o));
                },
            },
        );
        assert!(r.is_ok());
        assert_eq!(&*calls.borrow(), &[("quick", true)]);

        // named off（总开关开着）→ tunnel(Named, false)
        let calls: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lc = calls.clone();
        let r = toggle_channel_core(
            ChannelKind::Named,
            false,
            ChannelFlags {
                named: true,
                ..Default::default()
            },
            true,
            ChannelToggleDeps {
                public_ack: || panic!("隧道开关不读 ack 门"),
                write_flag: |_, _| {},
                restart: || panic!("隧道开关不得重启监听"),
                tunnel: move |k: ChannelKind, o: bool| {
                    lc.borrow_mut().push((k.as_str(), o));
                },
            },
        );
        assert!(r.is_ok());
        assert_eq!(&*calls.borrow(), &[("named", false)]);

        // 总开关关着：只写 KV，隧道/重启/ack 全不触
        let writes: std::rc::Rc<std::cell::RefCell<Vec<(&'static str, bool)>>> = Default::default();
        let lw = writes.clone();
        let r = toggle_channel_core(
            ChannelKind::Quick,
            true,
            ChannelFlags::default(),
            false,
            ChannelToggleDeps {
                public_ack: || panic!("总开关关着不读 ack"),
                write_flag: move |k: ChannelKind, v: bool| lw.borrow_mut().push((k.as_str(), v)),
                restart: || panic!("总开关关着不得重启监听"),
                tunnel: |_, _| panic!("总开关关着不得触隧道"),
            },
        );
        assert!(r.is_ok());
        assert_eq!(&*writes.borrow(), &[("quick", true)]);
    }

    /// 逐通道恢复内核：只对开着的隧道通道 ensure（lan 不进隧道恢复——监听由
    /// start_server 按派生 bind 承担）；全关 → 不触任何 ensure
    #[test]
    fn restore_tunnels_core_only_ensures_enabled_channels() {
        let calls: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let lc = calls.clone();
        restore_tunnels_core(
            ChannelFlags {
                lan: true,
                quick: true,
                named: false,
            },
            move |k| lc.borrow_mut().push(k.as_str()),
        );
        assert_eq!(
            &*calls.borrow(),
            &["quick"],
            "lan 开不得触发隧道 ensure；named 关不得 ensure"
        );
        // 双开：依序 quick → named
        let calls: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>> = Default::default();
        let lc = calls.clone();
        restore_tunnels_core(
            ChannelFlags {
                lan: false,
                quick: true,
                named: true,
            },
            move |k| lc.borrow_mut().push(k.as_str()),
        );
        assert_eq!(&*calls.borrow(), &["quick", "named"]);
        // 全关：panic 闭包证明确实不触
        restore_tunnels_core(ChannelFlags::default(), |_| {
            panic!("全关时不得触发任何隧道 ensure")
        });
    }

    /// PIN 自动生成内核：未设置 → 生成写入返回 true；已有 → 不覆盖（写闭包 panic
    /// 证明未被调）返回 false
    #[test]
    fn ensure_pin_core_writes_only_when_missing() {
        // None → 写入生成值
        let mut written: Vec<String> = vec![];
        let generated =
            ensure_pin_core(None, || "4321".to_string(), |p| written.push(p.to_string()));
        assert!(generated, "None 必须生成写入");
        assert_eq!(written, vec!["4321"]);
        // 已有 → 不覆盖
        let generated = ensure_pin_core(
            Some("1234".into()),
            || panic!("已有 PIN 不得覆盖"),
            |_| panic!("已有 PIN 不得写 KV"),
        );
        assert!(!generated);
    }

    /// 四通道状态载荷形状（A6/A7 消费契约，见 channels_payload 注释）：
    /// 四键齐全、local 常驻地址、lan 门控地址表、隧道错误态不宣称地址只给 error
    #[test]
    fn channels_payload_shape_contract() {
        use tunnel::{ChannelStatus, TunnelStatus};
        let tun = TunnelStatus {
            quick: ChannelStatus {
                running: true,
                url: Some("https://q.trycloudflare.com/m".into()),
                error: None,
            },
            named: ChannelStatus {
                running: false,
                url: Some("https://stale.example.com/m".into()),
                error: Some("cloudflared 启动失败".into()),
            },
        };
        let p = channels_payload(
            true,
            ChannelFlags {
                lan: true,
                quick: true,
                named: true,
            },
            9420,
            vec![
                "http://192.168.1.5:9420/m".to_string(),
                "http://10.0.0.2:9420/m".to_string(),
            ],
            &tun,
        );
        // 四键形状
        for k in ["local", "lan", "quick", "named"] {
            assert!(p.get(k).is_some(), "channels 载荷缺 {k} 键");
        }
        // 本机常驻：running = 监听存活，address = 回环直达
        assert_eq!(p["local"]["running"], true);
        assert_eq!(p["local"]["address"], "http://127.0.0.1:9420/m");
        // 局域网：enabled = 开关，running = 开关 ∧ 监听，addresses = 完整可直达 URL 表
        assert_eq!(p["lan"]["enabled"], true);
        assert_eq!(p["lan"]["running"], true);
        assert_eq!(
            p["lan"]["addresses"],
            serde_json::json!(["http://192.168.1.5:9420/m", "http://10.0.0.2:9420/m"])
        );
        // quick：运行中 + 地址宣称；无 error
        assert_eq!(p["quick"]["enabled"], true);
        assert_eq!(p["quick"]["running"], true);
        assert_eq!(p["quick"]["address"], "https://q.trycloudflare.com/m");
        assert_eq!(p["quick"]["error"], serde_json::Value::Null);
        // named：错误态不宣称地址（stale url 不得出表），只给 error
        assert_eq!(p["named"]["address"], serde_json::Value::Null);
        assert_eq!(p["named"]["error"], "cloudflared 启动失败");
        assert_eq!(p["named"]["running"], false);

        // 监听关着：local/lan running 全 false（本机/局域网随总开关停）
        let p = channels_payload(
            false,
            ChannelFlags {
                lan: true,
                ..Default::default()
            },
            9420,
            vec!["http://192.168.1.5:9420/m".to_string()],
            &TunnelStatus::default(),
        );
        assert_eq!(p["local"]["running"], false);
        assert_eq!(
            p["lan"]["running"], false,
            "lan.running = 监听存活 ∧ 开关，缺一不可"
        );
        assert_eq!(p["lan"]["enabled"], true, "enabled 只看开关位");
    }

    /// 评审 Minor 3 专测（A6 消费契约防线）：stop_channel 的快照复位与 stderr
    /// 在途行存在毫秒级竞态窗口——产物是 url=Some/running=false/error=None 的陈旧
    /// 快照，channels_payload 必须以 running 为门收口（已停通道绝不宣称地址）。
    /// 变异锚点：去掉 chan 闭包 address 过滤里的 `&& c.running`，本测试
    /// 「无错误已停通道」分支退化出 address=Some 而红
    #[test]
    fn channels_payload_gates_address_on_running_for_stale_snapshot_window() {
        use tunnel::{ChannelStatus, TunnelStatus};
        let tun = TunnelStatus {
            quick: ChannelStatus {
                running: false,
                url: Some("https://stale.trycloudflare.com/m".into()),
                error: None, // 无错误、无句柄——典型「在途行落在复位之后」的窗口产物
            },
            named: ChannelStatus::default(),
        };
        let p = channels_payload(
            true,
            ChannelFlags {
                quick: true,
                ..Default::default()
            },
            9420,
            vec![],
            &tun,
        );
        assert_eq!(p["quick"]["running"], false);
        assert_eq!(
            p["quick"]["address"],
            serde_json::Value::Null,
            "running=false 的通道不得宣称地址（陈旧快照窗口收口）"
        );
        assert_eq!(p["quick"]["error"], serde_json::Value::Null);
        // 对照：同一 url 在 running=true 时正常宣称（门不误伤活通道）
        let tun = TunnelStatus {
            quick: ChannelStatus {
                running: true,
                url: Some("https://fresh.trycloudflare.com/m".into()),
                error: None,
            },
            named: ChannelStatus::default(),
        };
        let p = channels_payload(
            true,
            ChannelFlags {
                quick: true,
                ..Default::default()
            },
            9420,
            vec![],
            &tun,
        );
        assert_eq!(
            p["quick"]["address"], "https://fresh.trycloudflare.com/m",
            "running=true 的活通道地址宣称不受门影响"
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
