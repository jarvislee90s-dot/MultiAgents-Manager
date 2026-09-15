// 远程接入层（M2）：axum 内嵌服务器 + 直通配对 + 移动看板 API
// 范围与红线见 docs/superpowers/plans/2026-09-14-m2-remote-access-board.md

pub mod api;
pub mod gate;
pub mod pairing;
pub mod server;

pub const KEY_ENABLED: &str = "remote.enabled";
pub const KEY_BIND: &str = "remote.bind";
pub const KEY_PORT: &str = "remote.port";
pub const KEY_PUBLIC_ACK: &str = "remote.public_ack";
/// 默认端口（避开 3080=dsh / 1420=vite / 18789=zcode）
pub const DEFAULT_PORT: u16 = 9420;

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
    start_server_core(
        &mut h,
        bind_and_port,
        || crate::database::dao::settings::get_setting(KEY_PUBLIC_ACK),
        |bind, port| {
            // 见 SERVER_HANDLE 注释：必须用 tauri::async_runtime::spawn（sync 命令 /
            // setup 线程无 tokio runtime 上下文）；其 JoinHandle 同样支持 abort
            // （stop_server 语义不变）
            tauri::async_runtime::spawn(async move {
                if let Err(e) = server::serve(&bind, port, STATE.clone()).await {
                    log::error!("远程服务器退出: {e}");
                }
            })
        },
    )
    .map(|_| ())
}

/// 停止远程服务：abort 服务器任务 + 全吊销已配对设备 + 清空配对码（不变量 5「全吊销」：
/// `stop()` 只清内存 token，DB 侧吊销由 `revoke_all` 完成，二者缺一不可）。
/// 锁序：SERVER_HANDLE（取走即释放）→ store DB 锁 → pairing 锁——两条锁不并发持有，
/// 与 api::pair 的「pairing 释放后再进 store」（Task 3 已收窄）不构成锁序反转
fn stop_server() {
    if let Some(h) = SERVER_HANDLE.lock().unwrap().take() {
        h.abort();
    }
    STATE.store.with(|c| {
        let _ = pairing::revoke_all(c); // 停止 = 全吊销（七不变量）
    });
    STATE.pairing.lock().unwrap().stop();
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
    )
}

/// enabled 展示校准内核（纯函数）：DB 声明开启**且**服务器句柄存活才算启用。
/// 动机（SSOT 与现实校准）：DB enabled=true 只代表用户意图——若启动失败残留或任务
/// 自退后未自愈，实际无人监听，展示必须以现实为准。正常态不受影响：start_server
/// 成功后句柄是长驻 serve 任务（is_finished=false），必然存活
fn status_enabled(db_enabled: bool, handle_alive: bool) -> bool {
    db_enabled && handle_alive
}

/// 设置页状态展示：enabled / bind / port / url / lanUrls（仅 0.0.0.0 给局域网候选）
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
    let lan = lan_urls_for(&bind, local_lan_ips(), port);
    serde_json::json!({ "enabled": enabled, "bind": bind, "port": port,
        "url": format!("http://{bind}:{port}/m"), "lanUrls": lan })
}

/// 刷新配对二维码：发行新 token（单活跃——发行即作废旧 token，不变量 1）并拼出可扫 URL。
/// 0.0.0.0 绑定时 host 取局域网地址候选的第一个（枚举失败回落 loopback）
#[tauri::command]
pub fn remote_issue_token() -> Result<serde_json::Value, String> {
    let token = STATE.pairing.lock().unwrap().issue();
    let (_, port) = bind_and_port().unwrap_or_else(|_| ("127.0.0.1".to_string(), DEFAULT_PORT));
    let host = match crate::database::dao::settings::get_setting(KEY_BIND) {
        Some(b) if b == "0.0.0.0" => local_lan_ips()
            .first()
            .cloned()
            .unwrap_or_else(|| "127.0.0.1".into()),
        Some(b) => b,
        None => "127.0.0.1".into(),
    };
    Ok(
        serde_json::json!({ "token": token, "url": format!("http://{host}:{port}/m#token={token}") }),
    )
}

/// TLS 前置确认（P7 安全门）：用户确认已配置 TLS 反向代理后置位，解锁 0.0.0.0 绑定
#[tauri::command]
pub fn remote_confirm_public() -> Result<(), String> {
    crate::database::dao::settings::set_setting(KEY_PUBLIC_ACK, "true");
    Ok(())
}

/// 局域网地址枚举（0.0.0.0 模式给手机可输入的候选）。
/// 实现用 std::net UDP connect 技巧零依赖：`connect` 只决定默认对端、**不发包**，
/// 失败（无路由 / 离线）返回空表
fn local_lan_ips() -> Vec<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|s| s.connect("8.8.8.8:80").ok().map(|_| s));
    s.and_then(|s| s.local_addr().ok())
        .map(|a| a.ip().to_string())
        .into_iter()
        .collect()
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
}
