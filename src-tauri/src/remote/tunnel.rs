// 外部通道（M4 T1）：cloudflared 获取层 + 隧道进程管理（Task 5）。
// 设计出处：docs/superpowers/specs/2026-09-16-m4-external-fullchain-design.md T1b/T1c/T1d

use std::path::{Path, PathBuf};

/// 通道三值（remote.channel 的值域；parse_channel 是唯一解析口）
pub const KEY_CHANNEL_VALUE_OFF: &str = "off";
pub const KEY_CHANNEL_VALUE_QUICK: &str = "quick";
pub const KEY_CHANNEL_VALUE_NAMED: &str = "named";

/// 通道设置解析（纯函数）：None/空串 → off（未配置视为关闭，老用户升级兼容）；
/// 仅认三个字面量，其余 None（写入侧 UI 限三选一，这里防御乱串）
pub fn parse_channel(v: Option<&str>) -> Option<&'static str> {
    match v.unwrap_or("") {
        "" | KEY_CHANNEL_VALUE_OFF => Some(KEY_CHANNEL_VALUE_OFF),
        KEY_CHANNEL_VALUE_QUICK => Some(KEY_CHANNEL_VALUE_QUICK),
        KEY_CHANNEL_VALUE_NAMED => Some(KEY_CHANNEL_VALUE_NAMED),
        _ => None,
    }
}

/// 平台二进制名（Windows 带 .exe 后缀）
pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

/// cloudflared 固定放置路径（spec T1d：~/.mam/bin/，手动放置旁路即放这里）。
/// 数据目录构造沿代码库先例（manifest.rs：dirs::home_dir().unwrap_or_default().join(".mam")，
/// 无统一 helper——不新造，保持散用先例）
pub fn cloudflared_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("bin")
        .join(bin_name())
}

/// 下载源（纯函数）：官方 GitHub Releases latest 固定资产名（2026-09-16 spec T1d 自决默认）
pub fn download_url_for() -> Result<String, String> {
    let asset = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "cloudflared-darwin-arm64.tgz"
        } else {
            "cloudflared-darwin-amd64.tgz"
        }
    } else if cfg!(windows) {
        "cloudflared-windows-amd64.exe"
    } else if cfg!(target_arch = "aarch64") {
        "cloudflared-linux-arm64"
    } else {
        "cloudflared-linux-amd64"
    };
    Ok(format!(
        "https://github.com/cloudflare/cloudflared/releases/latest/download/{asset}"
    ))
}

/// 二进制就绪判定（macOS 需可执行位；Windows 只查存在）
fn binary_ready(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    true
}

/// 下载器注入缝形态（生产 download_to / 测试闭包共用；clippy type_complexity
/// 对函数参数的阈值更严，按其建议抽 type 定义——具体类型不变）
pub type Downloader = Box<dyn FnOnce(&str, &Path) -> Result<(), String>>;

/// 获取内核（可测）：已就绪 → 直接返回；否则调注入下载器（生产实现见 download_to），
/// 下载完成后 macOS 从 .tgz 解压（系统 tar）、赋可执行位；Windows 直接落位。
/// 平台分支用编译期 cfg 属性而非运行期 cfg!：分支体内 use 了 std::os::unix 专属
/// 条目，运行期分支在 Windows 目标上无法编译（对蓝本的最小编译适配，各平台行为不变）
pub fn ensure_with(bin: &Path, dl: Downloader) -> Result<PathBuf, String> {
    if binary_ready(bin) {
        return Ok(bin.to_path_buf());
    }
    let url = download_url_for()?;
    let tmp = bin.with_extension("download");
    dl(&url, &tmp)?;
    #[cfg(target_os = "macos")]
    {
        // .tgz 内含裸二进制 cloudflared：解到同目录再改名（tar 存在于 macOS/全部 CI）
        let dir = bin.parent().ok_or("cloudflared 路径无父目录")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("建 bin 目录失败: {e}"))?;
        let st = std::process::Command::new("tar")
            .args([
                "-xzf",
                tmp.to_str().unwrap_or_default(),
                "-C",
                dir.to_str().unwrap_or_default(),
            ])
            .status()
            .map_err(|e| format!("tar 启动失败: {e}"))?;
        if !st.success() {
            return Err("cloudflared 解压失败".into());
        }
        let _ = std::fs::remove_file(&tmp);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("赋执行位失败: {e}"))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::fs::rename(&tmp, bin).map_err(|e| format!("落位失败: {e}"))?;
    }
    if binary_ready(bin) {
        Ok(bin.to_path_buf())
    } else {
        Err("cloudflared 落位后校验失败".into())
    }
}

/// 生产下载器（reqwest rustls，进度不回调——设置页显示「下载中」文案即可）。
/// 仅生产路径调用（Task 5 接线），测试零网络红线不触碰本函数
pub async fn download_to(url: &str, dest: &Path) -> Result<(), String> {
    let bytes = reqwest::get(url)
        .await
        .map_err(|e| format!("下载失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("下载失败: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("下载中断: {e}"))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败: {e}"))?;
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("写文件失败: {e}"))
}

// ============================================================
// 隧道进程管理（T1b/T1c）：spawn → stderr 解析(quick) → 守护重启 → 快照
// ============================================================

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 对外快照（remote_status 消费；ui 线程与 supervisor 任务共写——Mutex 单字段写短临界区）
#[derive(Clone, Default, serde::Serialize)]
pub struct TunnelStatus {
    pub mode: String,          // off/quick/named（当前实际运行模式）
    pub url: Option<String>,   // quick=trycloudflare 地址 / named=子域（未知时 None）
    pub error: Option<String>, // 起不来/连续失败的终态错误（界面明示）
}
static SNAPSHOT: Lazy<Mutex<TunnelStatus>> = Lazy::new(|| {
    Mutex::new(TunnelStatus {
        mode: KEY_CHANNEL_VALUE_OFF.into(),
        url: None,
        error: None,
    })
});

pub fn snapshot() -> TunnelStatus {
    SNAPSHOT.lock().unwrap().clone()
}
fn set_snapshot(f: impl FnOnce(&mut TunnelStatus)) {
    f(&mut SNAPSHOT.lock().unwrap());
}

/// 运行句柄：stop 信号 + supervisor 任务句柄（子进程由 supervisor 全权持有，
/// stop 经标志位传导——supervisor 内 kill_on_drop 保证 abort 时子进程必死）
struct TunnelHandle {
    stop: Arc<AtomicBool>,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}
static TUNNEL: Lazy<Mutex<Option<TunnelHandle>>> = Lazy::new(|| Mutex::new(None));

/// quick stderr 地址解析（纯函数）：行内 https://*.trycloudflare.com 才算
pub fn parse_quick_url(line: &str) -> Option<String> {
    let (s, e) = (line.find("https://")?, line.find(".trycloudflare.com")?);
    if e <= s {
        return None;
    }
    Some(line[s..].split_whitespace().next()?.to_string())
}

/// 守护退避（纯函数）：连续失败 n 次后的下次重启等待；None = 放弃
pub fn backoff_ms(consecutive_failures: u32) -> Option<u64> {
    match consecutive_failures {
        1 => Some(1_000),
        2 => Some(2_000),
        3 => Some(4_000),
        _ => None,
    }
}

/// 隧道地址条目（纯函数）：恒 primary，iface 空串（前端按 kind 渲染「外部通道」徽标）。
/// 地址表首位由 mod.rs 的 address_entries_with_tunnel 前插保证
pub fn tunnel_address_entry(url: &str) -> serde_json::Value {
    serde_json::json!({
        "url": url,
        "iface": "",
        "primary": true,
        "kind": "tunnel",
    })
}

/// 按当前设置启动隧道（幂等：运行中直接返回；off 直接返回）。
/// 失败不返回 Err（隧道失败不阻断远程主体——spec T1d「不阻塞其他功能」），
/// 错误写快照 error 供设置页展示
pub fn start_if_configured(port: u16) {
    let Some(mode) = parse_channel(
        crate::database::dao::settings::get_setting(crate::remote::KEY_CHANNEL).as_deref(),
    ) else {
        set_snapshot(|s| s.error = Some("通道设置非法".into()));
        return;
    };
    if mode == KEY_CHANNEL_VALUE_OFF {
        return; // 关闭态：快照保持 off
    }
    if mode == KEY_CHANNEL_VALUE_NAMED
        && crate::database::dao::settings::get_setting(crate::remote::KEY_TUNNEL_TOKEN)
            .map(|t| t.trim().is_empty())
            .unwrap_or(true)
    {
        set_snapshot(|s| {
            s.mode = mode.into();
            s.error = Some("命名隧道缺少 Tunnel Token".into());
        });
        return;
    }
    let mut h = TUNNEL.lock().unwrap();
    if h.is_some() {
        return; // 运行中幂等（通道切换走 restart 路径）
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let supervisor = tauri::async_runtime::spawn(async move {
        supervise(mode.to_string(), port, stop2).await;
    });
    *h = Some(TunnelHandle { stop, supervisor });
}

/// 守护主循环：确保二进制 → spawn → 监听 stderr(quick 解析地址) → wait → 退避重启
async fn supervise(mode: String, port: u16, stop: Arc<AtomicBool>) {
    set_snapshot(|s| {
        s.mode = mode.clone();
        s.url = None;
        s.error = None;
    });
    // 获取二进制（可能触发下载——秒到分钟级）。ensure_with 的下载器是**同步闭包**，
    // 内部需要 async 的 download_to——必须在 spawn_blocking 线程里 block_on：
    // supervise 本身是 async 任务，直接 tauri::async_runtime::block_on 会死锁/panic
    // （async 上下文内禁止 block_on；blocking 线程池无此限制）
    let bin_path = cloudflared_path();
    let bin = match tokio::task::spawn_blocking(move || {
        let dl = |url: &str, dest: &Path| tauri::async_runtime::block_on(download_to(url, dest));
        ensure_with(&bin_path, Box::new(dl))
    })
    .await
    .unwrap_or_else(|e| Err(format!("获取任务异常: {e}")))
    {
        Ok(p) => p,
        Err(e) => {
            set_snapshot(|s| {
                s.error = Some(format!(
                    "cloudflared 获取失败: {e}；可手动放置到 ~/.mam/bin/{}",
                    bin_name()
                ))
            });
            return;
        }
    };
    let mut failures: u32 = 0;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let started_at = std::time::Instant::now();
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.arg("tunnel").arg("--no-autoupdate").kill_on_drop(true); // supervisor 被 abort 时子进程必死（无孤儿）
        if mode == KEY_CHANNEL_VALUE_QUICK {
            cmd.arg("--url").arg(format!("http://127.0.0.1:{port}"));
        } else {
            let token =
                crate::database::dao::settings::get_setting(crate::remote::KEY_TUNNEL_TOKEN)
                    .unwrap_or_default();
            cmd.arg("run").arg("--token").arg(token);
        }
        cmd.stderr(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_snapshot(|s| s.error = Some(format!("cloudflared 启动失败: {e}")));
                return;
            }
        };
        // stderr 读取任务（地址解析：quick=trycloudflare 行；named=自有子域行）
        let stderr = child.stderr.take();
        let mode_for_stderr = mode.clone();
        let url_sink = Arc::new(Mutex::new(Option::<String>::None));
        let url_sink2 = url_sink.clone();
        if let Some(err) = stderr {
            tauri::async_runtime::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let quick = mode_for_stderr == KEY_CHANNEL_VALUE_QUICK;
                let mut lines = BufReader::new(err).lines();
                // clippy whilelet_loop 适配（蓝本 loop/match 同语义：Ok(None)/Err 均终止）
                while let Ok(Some(line)) = lines.next_line().await {
                    let u = if quick {
                        parse_quick_url(&line)
                    } else {
                        parse_named_url(&line)
                    };
                    if let Some(u) = u {
                        let fresh = {
                            let mut g = url_sink2.lock().unwrap();
                            let fresh = g.as_ref() != Some(&u);
                            *g = Some(u.clone());
                            fresh
                        };
                        if fresh {
                            set_snapshot(|s| s.url = Some(u.clone()));
                            // spec T1c：地址变化桌面通知（含首次拿到地址）
                            crate::remote::events::emit_ui(
                                "remote-tunnel-address",
                                serde_json::json!({ "url": u }),
                            );
                        }
                    }
                }
            });
        }
        // 等子进程退出（或被 stop 方经 kill_on_drop/标志位终止）
        let status = child.wait().await;
        if stop.load(Ordering::Relaxed) {
            break; // 主动停止不算失败
        }
        // 稳定运行 ≥60s 后的退出视为「新失败」重置计数——否则数周内三次偶发闪断
        // 就会累计到永久放弃（backoff 给的 3 次是**连续**失败语义，spec T1b）
        if started_at.elapsed() >= std::time::Duration::from_secs(60) {
            failures = 0;
        }
        failures += 1;
        match backoff_ms(failures) {
            Some(ms) => {
                log::warn!("cloudflared 退出({status:?})，{ms}ms 后第 {failures} 次重启");
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }
            None => {
                set_snapshot(|s| s.error = Some("cloudflared 连续失败 3 次，已停止守护".into()));
                // spec T1b：放弃时桌面通知报错（Task 8 的 useRemoteEvents 监听）
                crate::remote::events::emit_ui(
                    "remote-tunnel-error",
                    serde_json::json!({"error": "cloudflared 连续失败 3 次，已停止守护"}),
                );
                return;
            }
        }
    }
}

/// named 地址解析（纯函数）：stderr 中自有子域的 https:// 行——cloudflared run 启动
/// 横幅打印其 hostname；cloudflare.com 域（docs/trycloudflare）一律排除。
/// 解析不到留 None——设置页提示「地址以 Cloudflare 面板为准」
pub fn parse_named_url(line: &str) -> Option<String> {
    let s = line.find("https://")?;
    let url = line[s..].split_whitespace().next()?.to_string();
    let host = url.trim_start_matches("https://");
    if host.contains("cloudflare.com") || !host.contains('.') {
        return None;
    }
    Some(url)
}

/// 停止隧道（幂等）：置 stop → kill 子进程由 supervisor abort 经 kill_on_drop 传播
/// （spawn 时已设 .kill_on_drop(true)，兜底标志位 + abort 双保险）→ 快照复位
pub fn stop() {
    if let Some(h) = TUNNEL.lock().unwrap().take() {
        h.stop.store(true, Ordering::Relaxed);
        h.supervisor.abort(); // kill 由 abort 传播（子进程 kill_on_drop 已设——无孤儿）
    }
    set_snapshot(|s| {
        s.mode = KEY_CHANNEL_VALUE_OFF.into();
        s.url = None;
        s.error = None;
    });
}

/// 通道切换：运行中则重启（spec T1b「切换通道进程一并退出」）
pub fn restart_if_running(port: u16) {
    let running = TUNNEL.lock().unwrap().is_some();
    if running {
        stop();
        start_if_configured(port);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_parse_only_three_literals() {
        assert_eq!(parse_channel(None), Some("off"));
        assert_eq!(parse_channel(Some("")), Some("off"));
        assert_eq!(parse_channel(Some("quick")), Some("quick"));
        assert_eq!(parse_channel(Some("named")), Some("named"));
        assert_eq!(parse_channel(Some("QUICK")), None); // 大小写敏感拒绝
        assert_eq!(parse_channel(Some("tls")), None);
    }

    #[test]
    fn download_url_matches_platform() {
        // 纯函数按 cfg 分支断言本机平台（跨平台 CI 各自命中各自分支）
        let url = download_url_for().unwrap();
        if cfg!(target_os = "macos") {
            assert!(url.contains("cloudflared-darwin-"));
            assert!(url.ends_with(".tgz"));
        } else if cfg!(windows) {
            assert!(url.ends_with("cloudflared-windows-amd64.exe"));
        }
        assert!(url.starts_with("https://github.com/cloudflare/cloudflared/releases/"));
    }

    #[test]
    fn ensure_skips_download_when_binary_exists() {
        // 已存在（含可执行位）→ 不调下载器直接 Ok
        let dir = tempfile::tempdir().unwrap();
        // cloudflared_path 读 HOME——本测试改为直接驱动 ensure_cloudflared 的
        // exists 判定内核：ensure_with(bin_path, downloader)
        let bin = dir.path().join(bin_name());
        #[cfg(unix)]
        std::fs::write(&bin, b"fake").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        std::fs::write(&bin, b"fake").unwrap();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c2 = called.clone();
        let r = ensure_with(
            &bin,
            Box::new(move |_url, _dest| {
                c2.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
        );
        assert!(r.is_ok());
        assert!(!called.load(std::sync::atomic::Ordering::Relaxed));
    }

    // ==== Task 5：隧道进程管理（T1b/T1c）纯函数测试 ====
    // 零网络、零 ~/.mam 红线：以下全部为纯函数断言，不触 DAO / 进程 / 文件系统

    #[test]
    fn quick_url_parses_only_trycloudflare_host() {
        let line = r#"2026-09-16T00:00:00Z INF |  https://example-words-here.trycloudflare.com"#;
        assert_eq!(
            parse_quick_url(line).as_deref(),
            Some("https://example-words-here.trycloudflare.com")
        );
        // 普通日志行/其他域名不误报
        assert_eq!(parse_quick_url("INF Registered tunnel connection"), None);
        assert_eq!(parse_quick_url("see https://docs.cloudflare.com/"), None);
    }

    #[test]
    fn backoff_is_1s_2s_4s_then_give_up() {
        assert_eq!(backoff_ms(1), Some(1000));
        assert_eq!(backoff_ms(2), Some(2000));
        assert_eq!(backoff_ms(3), Some(4000));
        assert_eq!(backoff_ms(4), None); // 连续失败 3 次后停止（spec T1b）
    }

    #[test]
    fn tunnel_address_entry_is_primary_tunnel_kind() {
        let e = tunnel_address_entry("https://mam.example.asia");
        assert_eq!(e["url"], serde_json::json!("https://mam.example.asia"));
        assert_eq!(e["kind"], serde_json::json!("tunnel"));
        assert_eq!(e["primary"], serde_json::json!(true));
        assert_eq!(e["iface"], serde_json::json!("")); // 前端按 kind 渲染「外部通道」徽标
    }

    #[test]
    fn address_entries_prepend_tunnel() {
        // mod.rs 的 address_entries_with_tunnel：隧道地址恒插到表首位
        let base = vec![
            serde_json::json!({"url": "http://192.168.1.5:9420/m", "iface": "en0", "primary": true, "kind": "lan"}),
        ];
        let out = super::super::address_entries_with_tunnel(
            Some("https://mam.example.asia".into()), // 签名 Option<String>（brief 同款 .into() 形态）
            base.clone(),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["kind"], serde_json::json!("tunnel"));
        assert_eq!(out[1]["kind"], serde_json::json!("lan"));
        // 无隧道 → 原样（既有条目补 kind="lan"）
        let out2 = super::super::address_entries_with_tunnel(None, base);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0]["kind"], serde_json::json!("lan"));
    }
}
