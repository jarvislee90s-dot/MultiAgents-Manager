// gate 中间件：/m/api/* 全量过闸（403）——放行名单仅 /pair/pin（换 cookie 的入口）
// + 本机免密豁免（M5 A3：回环 + 本地 Host 双条件，穿透测试锁定）
// cookie 解析手写（不加 cookie 依赖）：mam_device=<hex>
//
// 路径语义（重要，勿按直觉改）：本中间件由 `server.rs` 作为 **nest("/m/api/v1") 的内层 layer**
// 挂载，axum 的 nest 会**剥掉前缀**——这里 `req.uri().path()` 看到的是 `/pair/pin`、
// `/sessions`、`/nope`，而**不是** `/m/api/v1/pair/pin`。放行名单必须用相对路径，写成
// 绝对路径会导致 pair 被 403（无法换 cookie）或未知路径漏放，两种错法都不报编译错。
// （Task 7 追加静态路由不经过本中间件：那些路由注册在 nest 之外的外层 Router。）

use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::IpAddr;
use std::sync::Arc;

use super::server::RemoteState;

/// Host 头归一（纯函数）：小写 + 剥端口（Host 可带端口 `host:9420`，split(':') 取首段）。
/// 隧道域名恒为裸域名无端口；IPv6 字面量 Host（`[::1]:9420`）剥后失真只会更严格
/// （剥不出域名 → 不匹配隧道名单 → 按"非隧道 Host"判定，方向安全）
pub fn normalize_host(host: &str) -> String {
    host.trim()
        .to_ascii_lowercase()
        .split(':')
        .next()
        .unwrap_or("")
        .to_string()
}

/// 本机访问判定（纯函数，**安全关键**，一处定义两处消费：gate 豁免 + via 判定）：
/// 来源 IP 为回环 **且** Host 非空 **且** 归一化 Host 不在隧道域名名单。
/// 双条件缺一不可：cloudflared 在本机把公网隧道流量从回环转发进来（源 IP=回环），
/// 仅查回环会把全部隧道流量免密放进（穿透）；Host 命中任一隧道域名即视为非本地。
/// 空 Host 头按非本地处理（fail closed——正常 HTTP/1.1 客户端必带 Host，缺失即异常流量）。
pub fn is_local_access(source_ip: IpAddr, host: &str, tunnel_hosts: &[String]) -> bool {
    if host.trim().is_empty() {
        return false; // 空 Host：fail closed
    }
    let normalized = normalize_host(host);
    source_ip.is_loopback() && !tunnel_hosts.iter().any(|t| normalize_host(t) == normalized)
}

/// via 判定（配对时刻，纯函数）：回环+非隧道 Host → "local"；Host==quick 域名 → "quick"；
/// Host==named 域名 → "named"；其余 → "lan"。
/// local 分支复用 is_local_access——「via=="local"」与 gate 豁免是同一判据的同义，
/// 同一请求在两条路径（花名册徽标 / 门禁放行）不会分歧
pub fn classify_via(
    source_ip: IpAddr,
    host: &str,
    quick_hosts: &[String],
    named_hosts: &[String],
) -> &'static str {
    let mut union: Vec<String> = quick_hosts.to_vec();
    union.extend(named_hosts.iter().cloned());
    if is_local_access(source_ip, host, &union) {
        "local"
    } else if quick_hosts
        .iter()
        .any(|t| normalize_host(t) == normalize_host(host))
    {
        "quick"
    } else if named_hosts
        .iter()
        .any(|t| normalize_host(t) == normalize_host(host))
    {
        "named"
    } else {
        "lan"
    }
}

/// 设备 cookie 名（唯一来源：此处解析与 api::pair_pin 下发都引用它，避免两处字面量漂移）
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

/// 门禁：/pair/pin 放行（PIN 即凭据）+ 本机免密豁免；其余请求必须带有效设备 cookie，
/// 否则 403。路径为 nest 剥前缀后的相对路径（见文件头注释），放行名单是 `"/pair/pin"`
pub async fn gate(State(state): State<Arc<RemoteState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // M5 A3：放行名单收口为 /pair/pin **精确相等**（旧 /pair 直通与 /pair/* 审批名单随
    // 密码制下线删除；/pair/pin/extra 等多余段不匹配，仍须过闸）
    if path == "/pair/pin" {
        return next.run(req).await; // PIN 即凭据的换 cookie 入口放行
    }
    // M5 A3 本机免密（**双条件**，穿透防线，is_local_access 一处定义）：
    // 来源 IP 为回环 **且** Host 头非隧道域名——cloudflared 在本机把公网隧道流量从
    // 回环转发进来（源 IP=回环），仅查回环会把全部隧道流量免密放进；Host 命中快照
    // 隧道域名即按外部流量对待。命中 → 直接放行（本机免密访问全看板）。
    // 源 IP 取 ConnectInfo（serve 以 into_make_service_with_connect_info 注入）；
    // 提取不到（异常装配）按非本地处理；空 Host 头同样 fail closed（纯函数内判定）。
    // **快照错误 = 不豁免（评审 Important 1，fail-closed）**：豁免的 Host 条件依赖
    // 隧道域名名单，快照处于错误终态时无从判定 Host 是否隧道域名——tunnel_hosts_source
    // 返回 None 哨兵，此处必须**完全跳过豁免**（回环 + 任意 Host 都不免费），代价仅
    // 隧道错误态下本机也需配对一次。依赖说明：tunnel.rs 现状 error ⇒ 无存活 cloudflared
    // （穿透面本应消失），但豁免判定**不押注**该不变量——快照错误一律收紧。
    // 廉价前置（评审 Minor 5）：先判 `is_loopback()`，非回环流量不付快照锁 + 名单分配开销
    if let Some(ci) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    {
        let source_ip = ci.0.ip();
        if source_ip.is_loopback() {
            // PID 实锤优先（2026-09-18）：来连套接字归属 MAM 账本的 cloudflared
            // 通道 → 必为隧道转发，**跳过豁免**落到下方设备 cookie 校验（域名名单
            // 未知/漂移都不影响）；查不到（本机浏览器直连 / 表不可得 / macOS）→
            // 回落域名名单判定
            let pid_is_tunnel = super::conn_owner::tunnel_channel_for_conn(ci.0.port()).is_some();
            if !pid_is_tunnel {
                if let Some(tunnel_hosts) = (state.tunnel_hosts_source)() {
                    let host = req
                        .headers()
                        .get(header::HOST)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if is_local_access(source_ip, host, &tunnel_hosts) {
                        return next.run(req).await; // 本机免密直达全看板
                    }
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Host 归一：小写 + 剥端口；空白 trim
    #[test]
    fn normalize_host_lowercases_and_strips_port() {
        assert_eq!(normalize_host("Mam.Example.COM:9420"), "mam.example.com");
        assert_eq!(normalize_host("mam.example.com"), "mam.example.com");
        assert_eq!(normalize_host("  LocalHost:80 "), "localhost");
        assert_eq!(normalize_host(""), "");
    }

    /// 本机判定真值表（安全关键，逐分支锁定）：
    /// 回环+本地 Host → 真；回环+隧道 Host → 假（穿透防线）；非回环 → 假；空 Host → 假
    #[test]
    fn is_local_access_truth_table() {
        let tunnels = vec!["mam-test.trycloudflare.com".to_string()];
        let lb: IpAddr = "127.0.0.1".parse().unwrap();
        let ext: IpAddr = "10.0.0.5".parse().unwrap();
        // 回环 + 非隧道 Host（带端口也归一匹配）
        assert!(is_local_access(lb, "localhost:9420", &tunnels));
        assert!(is_local_access(
            "::1".parse().unwrap(),
            "192.168.1.5",
            &tunnels
        ));
        // 回环 + 隧道 Host → 假：cloudflared 本机回环转发的穿透防线（变异锚点：
        // 去掉 is_local_access 的 Host 条件，本断言退化真，穿透测试必红）
        assert!(!is_local_access(lb, "mam-test.trycloudflare.com", &tunnels));
        // 非回环（真实局域网/公网设备）一律假，即使 Host 是本地的
        assert!(!is_local_access(ext, "localhost:9420", &tunnels));
        // 空 Host 头 fail closed
        assert!(!is_local_access(lb, "", &tunnels));
        // 隧道名单为空时回环 + 本地 Host 仍真（纯本机场景无隧道）
        assert!(is_local_access(lb, "localhost", &[]));
    }

    /// via 四分支：回环+非隧道 → local；quick 域名 → quick；named 域名 → named；其余 → lan
    #[test]
    fn classify_via_four_branches() {
        let quick = vec!["q-test.trycloudflare.com".to_string()];
        let named = vec!["mam.example.com".to_string()];
        let lb: IpAddr = "127.0.0.1".parse().unwrap();
        let ext: IpAddr = "10.0.0.5".parse().unwrap();
        assert_eq!(classify_via(lb, "localhost:9420", &quick, &named), "local");
        assert_eq!(
            classify_via(ext, "q-test.trycloudflare.com", &quick, &named),
            "quick",
            "Host==quick 域名 → quick（IP 是否回环不影响分支序）"
        );
        assert_eq!(
            classify_via(ext, "mam.example.com:443", &quick, &named),
            "named",
            "Host 带端口归一后匹配 named"
        );
        assert_eq!(classify_via(ext, "192.168.1.9:9420", &quick, &named), "lan");
        assert_eq!(
            classify_via(ext, "", &quick, &named),
            "lan",
            "空 Host → lan"
        );
        // 回环 + Host 命中 quick 名单 → quick 而非 local（穿透语义在 via 上同样成立：
        // 名单内的隧道域名永不落 local 分支；名单外的域名对判定器而言即本地域）
        assert_eq!(
            classify_via(lb, "q-test.trycloudflare.com:443", &quick, &named),
            "quick"
        );
    }
}
