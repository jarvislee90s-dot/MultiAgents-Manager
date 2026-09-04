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
    /// 未映射的底层 IO/网络错误统一收敛（接受 String 以便直接作 map_err 函数指针：
    /// io::Error/serde 错误经 ToString 转换）
    pub fn internal(detail: String) -> Self {
        Self::new("internal", detail)
    }
}

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
}