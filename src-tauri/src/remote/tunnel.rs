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

/// cloudflared 固定版本（用户实测版本）。**固定版本是 sha256 校验与镜像回退不引入
/// 供应链风险的前提**——latest 会随上游漂移，校验表与镜像缓存都跟不上；升级 =
/// 改本常量 + 逐项更新 expected_sha256 表（登记于 docs/superpowers/specs/2026-09-16-m4-external-fullchain-design.md §12.5）
pub const CLOUDFLARED_VERSION: &str = "2026.9.1";

/// 平台资产名（纯函数）：download_url_for 的 URL 末段、expected_sha256 查表键与
/// 失败文案共用同一标识——三处必须恒一致，抽出单一来源
pub fn asset_name() -> &'static str {
    if cfg!(target_os = "macos") {
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
    }
}

/// 官方 SHA256 表（纯函数）：逐字取自 cloudflared {CLOUDFLARED_VERSION} release 页
/// 的 SHA256 Checksums 清单；macOS 的 .tgz 校验对象是**下载的压缩包本身**（解压前）。
/// 未知资产直接 Err（防御性：资产名写错立刻暴露，绝不静默免检）。
/// 升级流程：改 CLOUDFLARED_VERSION → 从新 release 页重取五值 → 逐项替换
fn expected_sha256(asset: &str) -> Result<&'static str, String> {
    match asset {
        "cloudflared-darwin-amd64.tgz" => {
            Ok("1ea07ae775b03236bd6be18ca1848d6bdc4af2f4f3bce398823b5a36e5761b75")
        }
        "cloudflared-darwin-arm64.tgz" => {
            Ok("9a0b19f67dc7a3011bc6b972c7ce06a5fcea8784ac6bd599ffa382ea4aeb5a6e")
        }
        "cloudflared-windows-amd64.exe" => {
            Ok("2837888cc0f5d58f15b6dc478376de90b4d3ba5241c7947455d1e0a0df429712")
        }
        "cloudflared-linux-amd64" => {
            Ok("03f1f25d1cc93b9ad6c60569d44060bc4f17ed97075760ed8cfca4b12dcd68cc")
        }
        "cloudflared-linux-arm64" => {
            Ok("3d97437c71848bd8df68041e12436b484a661d95073ea1937f01a845ce88faa3")
        }
        other => Err(format!(
            "未知 cloudflared 资产名: {other:?}（sha256 表未登记，拒绝下载）"
        )),
    }
}

/// 候选下载源（纯函数）：官方在首位，其后镜像（前缀 + 完整官方 URL 拼接）。
/// 镜像入表前置条件：开发期 `curl -sI` 实测存活（302/200），死镜像剔除——
/// 2026-09-17 实测 ghfast.top=200、gh-proxy.com=200，均在表
fn download_candidates(official: &str) -> Vec<String> {
    const MIRROR_PREFIXES: [&str; 2] = ["https://ghfast.top/", "https://gh-proxy.com/"];
    let mut out = vec![official.to_string()];
    out.extend(MIRROR_PREFIXES.map(|p| format!("{p}{official}")));
    out
}

/// sha256 完整性校验（纯函数）：hex 小写比对——匹配 Ok(())，不符 Err（该候选弃用）
fn verify_sha256(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hex == expected.to_ascii_lowercase() {
        Ok(())
    } else {
        Err(format!("sha256 校验不符（期望 {expected}，实际 {hex}）"))
    }
}

/// 下载编排内核（纯编排，fetch_one 注入——生产实现才碰网络）：按候选序尝试
/// 下载 → sha256 校验 → 匹配才落盘；失败/不符的候选弃用换下一；全部失败聚合原因。
/// 本地写盘失败不换候选（磁盘问题换镜像无解）直接 Err
fn try_download(
    candidates: &[String],
    expected: &str,
    mut fetch_one: impl FnMut(&str) -> Result<Vec<u8>, String>,
    dest: &Path,
) -> Result<(), String> {
    let mut errs: Vec<String> = Vec::new();
    for url in candidates {
        match fetch_one(url) {
            Ok(bytes) => match verify_sha256(&bytes, expected) {
                Ok(()) => {
                    if let Some(dir) = dest.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败: {e}"))?;
                    }
                    return std::fs::write(dest, &bytes).map_err(|e| format!("写文件失败: {e}"));
                }
                Err(e) => errs.push(format!("{url} → {e}")),
            },
            Err(e) => errs.push(format!("{url} → {e}")),
        }
    }
    Err(format!("全部候选源失败: {}", errs.join("；")))
}

/// 终态获取失败文案（纯函数）：原因 + 官方下载页 + **完整绝对路径**指引——
/// 旧文案只给 `~/.mam/bin/` 相对写法，用户无法直接定位放置点
fn download_failure_message(reason: &str, asset: &str, dest: &Path) -> String {
    format!(
        "cloudflared 自动获取失败: {reason}；可从 \
         https://github.com/cloudflare/cloudflared/releases 手动下载 {asset}，\
         放到 {} 后重试",
        dest.display()
    )
}

/// 下载源（纯函数）：官方 GitHub Releases **固定版本** + 固定资产名（latest 已弃用——
/// 固定版本是 sha256 校验与镜像回退不引入供应链风险的前提，见 CLOUDFLARED_VERSION）
pub fn download_url_for() -> Result<String, String> {
    Ok(format!(
        "https://github.com/cloudflare/cloudflared/releases/download/{CLOUDFLARED_VERSION}/{}",
        asset_name()
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

/// 下载器注入缝形态（生产 download_to / 测试闭包共用；只收落盘 dest——候选循环 +
/// sha 校验的编排已收口进生产实现，测试闭包直接写假文件）。clippy type_complexity
/// 对函数参数的阈值更严，按其建议抽 type 定义——具体类型不变
pub type Downloader = Box<dyn FnOnce(&Path) -> Result<(), String>>;

/// 获取内核（可测）：已就绪 → 直接返回；否则调注入下载器（生产实现见 download_to，
/// 内部完成候选回退 + sha256 校验 + 落盘到 dest），下载完成后 macOS 从 .tgz 解压
/// （系统 tar）、赋可执行位；Windows 直接落位。
/// 平台分支用编译期 cfg 属性而非运行期 cfg!：分支体内 use 了 std::os::unix 专属
/// 条目，运行期分支在 Windows 目标上无法编译（对蓝本的最小编译适配，各平台行为不变）
pub fn ensure_with(bin: &Path, dl: Downloader) -> Result<PathBuf, String> {
    if binary_ready(bin) {
        return Ok(bin.to_path_buf());
    }
    let tmp = bin.with_extension("download");
    dl(&tmp)?;
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

/// 生产下载器（supervise 在 spawn_blocking 线程调用——同步外壳，单次 HTTP 才
/// block_on，阻塞线程无 runtime 限制）。reqwest rustls + system-proxy（读 Windows/
/// macOS 系统代理——此前 default-features=false 把该默认特性关掉导致直连
/// github.com 失败的根因修复）+ connect 15s / 总 600s 超时；候选序尝试
/// （官方 + 实测存活镜像）逐个 sha256 校验，通过才落盘 dest。
/// 仅生产路径调用，测试零网络红线不触碰本函数
pub fn download_to(dest: &Path) -> Result<(), String> {
    let official = download_url_for()?;
    let expected = expected_sha256(asset_name())?;
    let candidates = download_candidates(&official);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600)) // 60MB 级二进制容忍慢网、杜绝永久挂起
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    try_download(
        &candidates,
        expected,
        |url| {
            tauri::async_runtime::block_on(async {
                let bytes = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| format!("下载失败: {e}"))?
                    .error_for_status()
                    .map_err(|e| format!("下载失败: {e}"))?
                    .bytes()
                    .await
                    .map_err(|e| format!("下载中断: {e}"))?;
                Ok(bytes.to_vec())
            })
        },
        dest,
    )
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
    pub url: Option<String>,   // 看板完整地址（含 /m——摄取点经 board_url 归一）/ 未知时 None
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
/// pub(crate) 供 remote/mod.rs 的「快照 → 域名适配器」单测注入通道/错误态
/// （评审 Minor 6；全局态仅该测试触碰，用后还原默认值）
pub(crate) fn set_snapshot(f: impl FnOnce(&mut TunnelStatus)) {
    f(&mut SNAPSHOT.lock().unwrap());
}

/// 运行句柄：stop 信号 + supervisor 任务句柄（子进程由 supervisor 全权持有，
/// stop 经标志位传导——supervisor 内 kill_on_drop 保证 abort 时子进程必死）
struct TunnelHandle {
    stop: Arc<AtomicBool>,
    supervisor: tauri::async_runtime::JoinHandle<()>,
}
static TUNNEL: Lazy<Mutex<Option<TunnelHandle>>> = Lazy::new(|| Mutex::new(None));

/// 隧道地址归一为看板地址（纯函数，平台无关的纯字符串处理）：去尾部 `/`（可多个）
/// → 追加 `/m`；已以 `/m` 结尾则幂等（不重复追加，不产生 `/m/m`）。
/// 快照 url 契约：摄取点（supervise 的 stderr 解析）统一过本函数，url 恒为
/// 看板完整地址——消费方（地址表 / issue_token / 托盘复制 / toast）直接使用，
/// 不再各自拼接（根路径 / 会 404，服务端只伺服 /m 前缀）。
/// 输入假定为裸域名（无路径）；若未来解析器返回带路径 URL 需重新评估
pub fn board_url(base: &str) -> String {
    let t = base.trim_end_matches('/');
    if t.ends_with("/m") {
        t.to_string()
    } else {
        format!("{t}/m")
    }
}

/// stderr 摄取点解析+归一（纯函数）：quick=trycloudflare 行 / named=自有子域行
/// → 裸域名 → board_url 归一为看板完整地址。原「parse_quick_url/parse_named_url
/// → board_url」两步收口为单点，锁定「快照 url 恒为看板地址」契约的执行点——
/// stderr 任务只调本函数，不得绕开归一直取裸域名
pub fn parsed_board_url(line: &str, quick: bool) -> Option<String> {
    let parsed = if quick {
        parse_quick_url(line)
    } else {
        parse_named_url(line)
    };
    parsed.map(|u| board_url(&u))
}

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
    // 生产实现 download_to 内部对单次 HTTP 做 block_on：必须在 spawn_blocking 线程里
    // 调用——supervise 本身是 async 任务，async 上下文内禁止 block_on（会死锁/panic；
    // blocking 线程池无此限制）
    let bin_path = cloudflared_path();
    // bin_path 被 move 进 spawn_blocking 闭包——显示串先另存（错误文案需完整绝对路径）
    let bin_display = bin_path.display().to_string();
    let bin =
        match tokio::task::spawn_blocking(move || ensure_with(&bin_path, Box::new(download_to)))
            .await
            .unwrap_or_else(|e| Err(format!("获取任务异常: {e}")))
        {
            Ok(p) => p,
            Err(e) => {
                set_snapshot(|s| {
                    s.error = Some(download_failure_message(
                        &e,
                        asset_name(),
                        Path::new(&bin_display),
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
                    // 归一在摄取点（parsed_board_url 内部完成解析 + board_url 两步）：
                    // 快照 url 恒为看板完整地址——url_sink / set_snapshot / emit_ui 消费的
                    // 全是归一后的值，下游（地址表 / issue_token / 托盘 / toast）不再各自拼 /m
                    if let Some(u) = parsed_board_url(&line, quick) {
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
        // 固定版本段（pinned）：latest 是 sha 校验与镜像回退的供应链风险源，禁用
        assert!(url.contains(&format!("/releases/download/{CLOUDFLARED_VERSION}/")));
        assert!(!url.contains("latest"));
        if cfg!(target_os = "macos") {
            assert!(url.contains("cloudflared-darwin-"));
            assert!(url.ends_with(".tgz"));
        } else if cfg!(windows) {
            assert!(url.ends_with("cloudflared-windows-amd64.exe"));
        }
        assert!(url.starts_with("https://github.com/cloudflare/cloudflared/releases/"));
        // URL 末段恒为资产名——expected_sha256 查表与失败文案共用同一标识
        assert_eq!(url.rsplit('/').next(), Some(asset_name()));
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
            Box::new(move |_dest| {
                c2.store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }),
        );
        assert!(r.is_ok());
        assert!(!called.load(std::sync::atomic::Ordering::Relaxed));
    }

    // ==== Task 6：下载器加固（固定版本 + sha256 + 镜像回退 + 失败指引）====
    // 零网络红线：fetch 闭包全部为内存假实现，不触真实 HTTP

    /// 公认 SHA256 测试向量：sha256("hello")
    const HELLO_SHA: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    #[test]
    fn download_candidates_official_first_then_alive_mirrors() {
        let official =
            "https://github.com/cloudflare/cloudflared/releases/download/2026.9.1/cloudflared-windows-amd64.exe";
        let c = download_candidates(official);
        assert_eq!(c[0], official, "官方源恒首位");
        // 镜像 = 前缀 + 完整官方 URL；两者 2026-09-17 开发期 curl -sI 实测均 200 存活
        assert_eq!(c[1], format!("https://ghfast.top/{official}"));
        assert_eq!(c[2], format!("https://gh-proxy.com/{official}"));
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn expected_sha256_table_matches_official_manifest() {
        // 值逐字对齐 2026.9.1 release 页 SHA256 Checksums 清单（升级 = 改常量 + 更新本表）
        assert_eq!(
            expected_sha256("cloudflared-darwin-amd64.tgz").unwrap(),
            "1ea07ae775b03236bd6be18ca1848d6bdc4af2f4f3bce398823b5a36e5761b75"
        );
        assert_eq!(
            expected_sha256("cloudflared-darwin-arm64.tgz").unwrap(),
            "9a0b19f67dc7a3011bc6b972c7ce06a5fcea8784ac6bd599ffa382ea4aeb5a6e"
        );
        assert_eq!(
            expected_sha256("cloudflared-windows-amd64.exe").unwrap(),
            "2837888cc0f5d58f15b6dc478376de90b4d3ba5241c7947455d1e0a0df429712"
        );
        assert_eq!(
            expected_sha256("cloudflared-linux-amd64").unwrap(),
            "03f1f25d1cc93b9ad6c60569d44060bc4f17ed97075760ed8cfca4b12dcd68cc"
        );
        assert_eq!(
            expected_sha256("cloudflared-linux-arm64").unwrap(),
            "3d97437c71848bd8df68041e12436b484a661d95073ea1937f01a845ce88faa3"
        );
    }

    #[test]
    fn expected_sha256_rejects_unknown_asset() {
        // 防御性：资产名写错立刻暴露，绝不静默免检
        assert!(expected_sha256("cloudflared-freebsd-amd64").is_err());
        assert!(expected_sha256("").is_err());
    }

    #[test]
    fn verify_sha256_accepts_correct_and_rejects_wrong_hex() {
        // 正确 hex 通过
        assert_eq!(verify_sha256(b"hello", HELLO_SHA), Ok(()));
        // 错误 hex（另一公认向量的值）拒绝
        assert!(verify_sha256(
            b"hello",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        )
        .is_err());
        // 乱串期望值拒绝
        assert!(verify_sha256(b"hello", "not-a-hash").is_err());
    }

    #[test]
    fn try_download_stops_at_first_valid_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("bin").join("cloudflared");
        let mut called: Vec<String> = vec![];
        let r = try_download(
            &["https://a/1".to_string(), "https://b/2".to_string()],
            HELLO_SHA,
            |url| {
                called.push(url.to_string());
                Ok(b"hello".to_vec())
            },
            &dest,
        );
        assert!(r.is_ok());
        // 首候选成功即停：后续 fetch 不被调
        assert_eq!(called, vec!["https://a/1".to_string()]);
        // 校验通过才落盘（含父目录自动创建）
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[test]
    fn try_download_falls_back_on_fetch_error_and_sha_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("cloudflared");
        let mut called: Vec<String> = vec![];
        let r = try_download(
            &[
                "https://a/1".to_string(),
                "https://b/2".to_string(),
                "https://c/3".to_string(),
            ],
            HELLO_SHA,
            |url| {
                called.push(url.to_string());
                match url {
                    "https://a/1" => Err("连接超时".into()), // 网络失败 → 换下一
                    "https://b/2" => Ok(b"corrupted".to_vec()), // sha 不符 → 弃用换下一
                    _ => Ok(b"hello".to_vec()),              // 首个有效候选
                }
            },
            &dest,
        );
        assert!(r.is_ok());
        assert_eq!(
            called,
            vec![
                "https://a/1".to_string(),
                "https://b/2".to_string(),
                "https://c/3".to_string()
            ],
            "失败候选依序弃用回落，不得提前终止"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
    }

    #[test]
    fn try_download_errs_and_writes_nothing_when_all_fail() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("cloudflared");
        let r = try_download(
            &["https://a/1".to_string(), "https://b/2".to_string()],
            HELLO_SHA,
            |url| Err(format!("源不可达: {url}")),
            &dest,
        );
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(
            msg.contains("https://a/1") && msg.contains("https://b/2"),
            "全部失败须聚合各候选原因: {msg}"
        );
        assert!(!dest.exists(), "全失败不得落盘");
    }

    #[test]
    fn download_failure_message_has_absolute_path_and_official_page() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join("bin").join(bin_name()); // 真实绝对路径
        let msg = download_failure_message("全部候选源失败", asset_name(), &abs);
        assert!(msg.contains("https://github.com/cloudflare/cloudflared/releases"));
        assert!(
            msg.contains(&abs.display().to_string()),
            "必须含完整绝对路径（不得只有 ~/.mam/bin/ 相对写法）: {msg}"
        );
        assert!(msg.contains(asset_name()));
    }

    #[test]
    fn parsed_board_url_normalizes_quick_and_named_lines() {
        // quick 形态 → 看板完整地址（「快照 url 恒为看板地址」契约的执行点）
        assert_eq!(
            parsed_board_url(
                r#"2026-09-16T00:00:00Z INF |  https://example-words-here.trycloudflare.com"#,
                true
            ),
            Some("https://example-words-here.trycloudflare.com/m".to_string())
        );
        // named 形态（自有子域行）→ 同样归一
        assert_eq!(
            parsed_board_url("INF hostname=https://mam.example.asia", false),
            Some("https://mam.example.asia/m".to_string())
        );
        // 已含 /m 的输入幂等（不产生 /m/m）
        assert_eq!(
            parsed_board_url("INF hostname=https://mam.example.asia/m", false),
            Some("https://mam.example.asia/m".to_string())
        );
        // 解析不到的行 → None
        assert_eq!(
            parsed_board_url("INF Registered tunnel connection", true),
            None
        );
        // quick 模式下 cloudflare.com 域行不误报
        assert_eq!(
            parsed_board_url("see https://docs.cloudflare.com/", true),
            None
        );
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

    // ==== 裸域名 404 修复：board_url 归一（纯字符串，平台无关） ====

    #[test]
    fn board_url_appends_m_idempotent_and_trims_slashes() {
        // 裸域名（quick/named stderr 解析形态）→ 补 /m
        assert_eq!(
            board_url("https://example-words-here.trycloudflare.com"),
            "https://example-words-here.trycloudflare.com/m"
        );
        assert_eq!(
            board_url("https://mam.example.asia"),
            "https://mam.example.asia/m"
        );
        // 已以 /m 结尾 → 幂等（不得产生 /m/m）
        assert_eq!(
            board_url("https://mam.example.asia/m"),
            "https://mam.example.asia/m"
        );
        // 尾部 / 去重（可多个），含 /m/ 形态
        assert_eq!(
            board_url("https://mam.example.asia/"),
            "https://mam.example.asia/m"
        );
        assert_eq!(
            board_url("https://mam.example.asia///"),
            "https://mam.example.asia/m"
        );
        assert_eq!(
            board_url("https://mam.example.asia/m/"),
            "https://mam.example.asia/m"
        );
        // 退化输入不 panic：空串 → 仍返回含 /m 的形态
        assert_eq!(board_url(""), "/m");
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
            // 生产输入恒为 board_url 归一后的 /m 形态（快照 url 契约），夹具镜像之
            Some("https://mam.example.asia/m".into()), // 签名 Option<String>（brief 同款 .into() 形态）
            base.clone(),
        );
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0]["url"],
            serde_json::json!("https://mam.example.asia/m"),
            "隧道条目 url 原样透传（归一已在摄取点完成，此处不再补 /m）"
        );
        assert_eq!(out[0]["kind"], serde_json::json!("tunnel"));
        assert_eq!(out[1]["kind"], serde_json::json!("lan"));
        // 无隧道 → 原样（既有条目补 kind="lan"）
        let out2 = super::super::address_entries_with_tunnel(None, base);
        assert_eq!(out2.len(), 1);
        assert_eq!(out2[0]["kind"], serde_json::json!("lan"));
    }
}
