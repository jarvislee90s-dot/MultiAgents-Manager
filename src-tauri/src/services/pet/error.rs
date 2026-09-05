// 跨 IPC 结构化错误（第六轮 Commit 2）：code = 稳定错误码（前端 i18n 键 pet.rpc.<code>）；
// params = 插值参数；detail = 开发者可读原文（日志用，前端不展示）。
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetRpcError {
    pub code: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    pub detail: String,
}

impl PetRpcError {
    pub fn new(code: &str, detail: impl Into<String>) -> Self {
        Self { code: code.into(), params: BTreeMap::new(), detail: detail.into() }
    }
    pub fn with(mut self, key: &str, val: impl Into<String>) -> Self {
        self.params.insert(key.into(), val.into());
        self
    }
    /// 未映射的底层 IO/网络错误统一收敛（接受 String：统一调用点形态 |e| internal(e.to_string())，
    /// 保持类型简单）
    pub fn internal(detail: String) -> Self {
        Self::new("internal", detail)
    }
}

/// 全量错误码单一清单（测试基准，46 码）：新增错误码须同步登记到此处与前端
/// locales 的 pet.rpc.* 键（zh/en 两份）；rpc_codes_have_i18n_keys 测试负责锁住两侧不漂移。
/// 登记依据：42 个单行构造站点 + internal 兜底 + redirect-too-many/redirect-forbidden
/// + download-too-large/manifest-too-large（P1-1 下载封顶）。
#[cfg(test)]
pub const ALL_RPC_CODES: &[&str] = &[
    "audio-format-unsupported",
    "audio-not-found",
    "audio-relpath-invalid",
    "copy-failed",
    "delete-failed",
    "download-failed",
    "download-status",
    "download-too-large",
    "download-url-invalid",
    "finalize-move-failed",
    "finalize-scan-failed",
    "group-invalid",
    "host-forbidden",
    "internal",
    "manifest-backup-failed",
    "manifest-parse-failed",
    "manifest-request-failed",
    "manifest-status",
    "manifest-too-large",
    "manifest-write-failed",
    "pet-dir-missing",
    "pet-exists",
    "pet-name-dot-prefix",
    "pet-name-empty",
    "pet-name-illegal",
    "pet-name-reserved",
    "pet-name-reserved-device",
    "pet-name-too-long",
    "pet-not-found",
    "pet-not-on-petdex",
    "petdex-no-zip",
    "redirect-forbidden",
    "redirect-too-many",
    "rename-failed",
    "reveal-failed",
    "sheet-not-found",
    "slug-invalid",
    "slug-parse-failed",
    "source-not-folder",
    "staging-create-failed",
    "staging-id-invalid",
    "staging-missing-sheet",
    "staging-not-found",
    "tmp-write-failed",
    "zip-entry-illegal-path",
    "zip-open-failed",
    "zip-read-failed",
    "zip-too-many-entries",
    "zip-total-over-limit",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_shape_camel_case() {
        let e = PetRpcError::new("pet-exists", "宠物已存在: abc").with("name", "abc");
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"code\":\"pet-exists\""));
        assert!(json.contains("\"params\":{\"name\":\"abc\"}"));
        assert!(json.contains("\"detail\"")); // camelCase 下 detail 本就小写
        assert!(!json.contains("\"code_\""));
    }

    #[test]
    fn internal_converges_unmapped() {
        let e = PetRpcError::internal("io error boom".to_string());
        assert_eq!(e.code, "internal");
        assert_eq!(e.detail, "io error boom");
        assert!(e.params.is_empty());
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"code\":\"internal\""));
        // params 空表也要序列化为 {}（前端 e.params ?? {} 分支依赖形状稳定）
        assert!(json.contains("\"params\":{}"));
    }

    /// 码表闭环（第八轮）：ALL_RPC_CODES 每个码都必须在前端 zh.json 有 pet.rpc.<code> 键。
    /// 文件读不到时 panic 带明确信息（不静默跳过）。
    #[test]
    fn rpc_codes_have_i18n_keys() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../src/i18n/locales/zh.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("无法读取 zh.json（{}）: {}——码表一致性测试要求该文件存在", path.display(), e));
        let root: serde_json::Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("zh.json 不是合法 JSON: {}", e));
        let rpc = root
            .get("pet")
            .and_then(|p| p.get("rpc"))
            .and_then(|r| r.as_object())
            .unwrap_or_else(|| panic!("zh.json 缺少 pet.rpc 对象——i18n 键结构与码表约定不符"));
        for code in ALL_RPC_CODES {
            assert!(
                rpc.contains_key(*code),
                "错误码 `{}` 在 zh.json 的 pet.rpc.* 中没有对应 i18n 键（新增错误码须同步登记 locales）",
                code
            );
        }
    }
}