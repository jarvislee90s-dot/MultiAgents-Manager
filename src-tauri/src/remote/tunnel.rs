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
}
