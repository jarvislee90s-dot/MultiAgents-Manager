use super::types::Manifest;

pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
}

/// 检查 GitHub Release 是否有新版本（Phase 2 实现 HTTP 请求）
pub fn check_for_updates(_manifest: &Manifest) -> Option<UpdateInfo> {
    None
}
