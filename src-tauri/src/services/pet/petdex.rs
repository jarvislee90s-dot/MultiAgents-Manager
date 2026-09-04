// petdex 在线导入 — 链接解析、清单匹配、zip 下载（spec §8.3/§13 域名白名单）
use super::error::PetRpcError;
use super::import::{self, StagedPet};
use serde::Deserialize;
use std::path::Path;

pub const MANIFEST_URL: &str = "https://petdex.dev/api/manifest";

/// 仅允许 petdex 域（页面域 + 资产域，spec §13）
pub fn allowed_host(host: &str) -> bool {
    host == "petdex.dev" || host == "www.petdex.dev" || host.ends_with(".petdex.dev")
}

/// 从宠物页链接解析 slug：/pets/<slug>（兼容 /en/pets/<slug>、尾斜杠、query）
pub fn parse_slug(url: &str) -> Option<String> {
    let path = url.split('?').next()?.split('#').next()?;
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let i = segs.iter().position(|s| *s == "pets")?;
    let slug = segs.get(i + 1)?;
    let ok = !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok { Some(slug.to_string()) } else { None }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetdexEntry {
    pub slug: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub zip_url: String,
    #[serde(default)]
    pub sprite_version_number: u8,
}

/// 完整 URL 合法性：强制 https + host 白名单（spec §13；重定向每跳同样适用）
fn url_allowed(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str().map(allowed_host).unwrap_or(false)
}

fn client(secs: u64) -> Result<reqwest::Client, PetRpcError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(secs))
        // 每跳重定向都重新校验白名单：防 http 明文降级与跳转到非 petdex 域
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("重定向次数过多")
            } else if url_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("重定向目标不在 petdex 白名单内")
            }
        }))
        .build()
        .map_err(|e| PetRpcError::internal(e.to_string()))
}

/// 拉全量清单并按 slug 匹配（petdex 文档化的稳定接口，spec §3）
pub async fn fetch_entry(slug: &str) -> Result<PetdexEntry, PetRpcError> {
    let list = client(30)?
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| PetRpcError::new("manifest-request-failed", format!("petdex 清单请求失败: {}", e)).with("err", e.to_string()))?
        .error_for_status()
        .map_err(|e| PetRpcError::new("manifest-status", format!("petdex 清单响应异常: {}", e)).with("err", e.to_string()))?
        .json::<Vec<PetdexEntry>>()
        .await
        .map_err(|e| PetRpcError::new("manifest-parse-failed", format!("petdex 清单解析失败: {}", e)).with("err", e.to_string()))?;
    list.into_iter()
        .find(|e| e.slug == slug)
        .ok_or_else(|| PetRpcError::new("pet-not-on-petdex", format!("petdex 上未找到宠物: {}", slug)).with("slug", slug.to_string()))
}

/// 下载 zip 字节（首跳 https + 域名白名单校验，重定向每跳由 client 策略校验，spec §8.3）
pub async fn download_zip(zip_url: &str) -> Result<Vec<u8>, PetRpcError> {
    let url = reqwest::Url::parse(zip_url).map_err(|e| PetRpcError::new("slug-parse-failed", format!("下载地址非法: {}", e)).with("err", e.to_string()))?;
    if !url_allowed(&url) {
        let host = url.host_str().unwrap_or("").to_string();
        return Err(PetRpcError::new("host-forbidden", format!("拒绝非 petdex 域下载: {}", url.as_str())).with("host", host));
    }
    let bytes = client(120)?
        .get(zip_url)
        .send()
        .await
        .map_err(|e| PetRpcError::new("download-failed", format!("下载失败: {}", e)).with("err", e.to_string()))?
        .error_for_status()
        .map_err(|e| PetRpcError::new("download-status", format!("下载响应异常: {}", e)).with("err", e.to_string()))?
        .bytes()
        .await
        .map_err(|e| PetRpcError::new("download-failed", format!("下载读取失败: {}", e)).with("err", e.to_string()))?;
    Ok(bytes.to_vec())
}

/// 链接 → 暂存：解析 slug → 清单匹配 → 下载 zip → 统一 zip 管线（spec §8.3 仅下载压缩包）
pub async fn stage_from_url(root: &Path, url: &str) -> Result<StagedPet, PetRpcError> {
    let slug = parse_slug(url)
        .ok_or_else(|| PetRpcError::new("slug-parse-failed", "无法从链接解析宠物标识（期望 https://petdex.dev/pets/<slug>）"))?;
    let entry = fetch_entry(&slug).await?;
    if entry.zip_url.is_empty() {
        return Err(PetRpcError::new("petdex-no-zip", "该宠物没有可下载的压缩包"));
    }
    let bytes = download_zip(&entry.zip_url).await?;
    let tmp = std::env::temp_dir().join(format!("mam-petdex-{}.zip", slug));
    std::fs::write(&tmp, &bytes).map_err(|e| PetRpcError::new("tmp-write-failed", format!("临时文件写入失败: {}", e)).with("err", e.to_string()))?;
    let staged = import::stage_from_zip_in(root, &tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut staged = staged?;
    staged.suggested_name = slug.clone();
    if !entry.display_name.is_empty() {
        staged.suggested_display_name = entry.display_name.clone();
    }
    staged.sprite_version_number = entry.sprite_version_number;
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slug_variants() {
        assert_eq!(parse_slug("https://petdex.dev/pets/capvolt").as_deref(), Some("capvolt"));
        assert_eq!(parse_slug("https://petdex.dev/en/pets/capvolt/").as_deref(), Some("capvolt"));
        assert_eq!(parse_slug("https://petdex.dev/pets/capvolt?x=1").as_deref(), Some("capvolt"));
        assert_eq!(parse_slug("https://petdex.dev/collections"), None);
        assert_eq!(parse_slug("https://petdex.dev/pets/"), None);
        assert_eq!(parse_slug("https://evil.com/pets/abc").as_deref(), Some("abc")); // 域名不限（只解析 slug），下载域在 download_zip 校验
    }

    #[test]
    fn allowed_host_whitelist() {
        assert!(allowed_host("petdex.dev"));
        assert!(allowed_host("assets.petdex.dev"));
        assert!(!allowed_host("evil.dev"));
        assert!(!allowed_host("petdex.dev.evil.com"));
    }

    /// 结构化错误代表性断言：download_zip 白名单拒绝 → code=host-forbidden + params.host（第六轮）
    #[tokio::test]
    async fn download_zip_host_forbidden_code_and_params() {
        let e = download_zip("https://evil.com/x.zip").await.unwrap_err();
        assert_eq!(e.code, "host-forbidden");
        assert_eq!(e.params.get("host").map(String::as_str), Some("evil.com"));
    }

    #[test]
    fn url_allowed_requires_https_and_whitelist() {
        let mk = |u: &str| reqwest::Url::parse(u).unwrap();
        // http 明文拒绝（spec §13 强制 https）
        assert!(!url_allowed(&mk("http://petdex.dev/x.zip")));
        // https 白名单域通过
        assert!(url_allowed(&mk("https://petdex.dev/x.zip")));
        // 域名相似但非白名单
        assert!(!url_allowed(&mk("https://evil-petdex.dev/x.zip")));
        assert!(!url_allowed(&mk("https://petdex.dev.evil.com/x.zip")));
        // 多级子域仍在白名单内（allowed_host 前缀匹配 .petdex.dev）
        assert!(url_allowed(&mk("https://sub.assets.petdex.dev/x.zip")));
    }

    #[test]
    fn entry_deserializes_manifest_shape() {
        // 与 petdex.dev/api/manifest 实测字段一致（spec §3）
        let e: PetdexEntry = serde_json::from_str(
            r#"{"slug":"capvolt","displayName":"Pikachu","spritesheetUrl":"https://assets.petdex.dev/pets/capvolt-x/sprite.webp","petJsonUrl":"...","zipUrl":"https://assets.petdex.dev/pets/capvolt-x/zip.zip","spriteVersionNumber":1}"#,
        )
        .unwrap();
        assert_eq!(e.slug, "capvolt");
        assert!(e.zip_url.contains("assets.petdex.dev"));
    }
}