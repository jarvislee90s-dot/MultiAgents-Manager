use std::path::Path;
use serde::Serialize;
use super::types::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    pub code: String,
}

pub struct ManifestValidator;

impl ManifestValidator {
    pub fn validate_file(path: &Path) -> Result<Manifest, Vec<ValidationError>> {
        if !path.exists() {
            return Err(vec![ValidationError {
                field: "file".into(), message: format!("文件不存在: {}", path.display()), code: "FILE_NOT_FOUND".into(),
            }]);
        }
        let content = std::fs::read_to_string(path).map_err(|e| vec![ValidationError {
            field: "file".into(), message: format!("读取文件失败: {}", e), code: "READ_ERROR".into(),
        }])?;
        Self::validate_json(&content)
    }

    pub fn validate_json(json: &str) -> Result<Manifest, Vec<ValidationError>> {
        let manifest: Manifest = serde_json::from_str(json).map_err(|e| vec![ValidationError {
            field: "root".into(), message: format!("JSON 解析失败: {}", e), code: "PARSE_ERROR".into(),
        }])?;
        Self::validate_manifest(&manifest)?;
        Ok(manifest)
    }

    pub fn validate_manifest(manifest: &Manifest) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if manifest.common.id.is_empty() {
            errors.push(ValidationError { field: "id".into(), message: "id 不能为空".into(), code: "REQUIRED".into() });
        }
        if !manifest.common.id.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_' || c == '-') {
            errors.push(ValidationError { field: "id".into(), message: "id 仅允许字母、数字、.、_、-".into(), code: "INVALID_FORMAT".into() });
        }
        if !is_valid_semver(&manifest.common.version) {
            errors.push(ValidationError { field: "version".into(), message: "version 必须为有效 semver".into(), code: "INVALID_SEMVER".into() });
        }
        if let Some(repo) = &manifest.common.github_repo {
            if !repo.contains('/') || repo.split('/').count() != 2 {
                errors.push(ValidationError { field: "githubRepo".into(), message: "githubRepo 格式应为 owner/repo".into(), code: "INVALID_FORMAT".into() });
            }
        }
        match manifest.common.kind {
            Kind::Skill => {
                if manifest.skill.is_none() {
                    errors.push(ValidationError { field: "skill".into(), message: "kind 为 skill 时 skill.entry 必填".into(), code: "REQUIRED".into() });
                } else if let Some(skill) = &manifest.skill {
                    if skill.entry.contains("..") {
                        errors.push(ValidationError { field: "skill.entry".into(), message: "不允许路径穿越（../）".into(), code: "PATH_TRAVERSAL".into() });
                    }
                }
            }
            Kind::Mcp => {
                if manifest.mcp.is_none() {
                    errors.push(ValidationError { field: "mcp".into(), message: "kind 为 mcp 时 mcp.command 必填".into(), code: "REQUIRED".into() });
                }
                // Task 10: MCP format compatibility check
                if let Some(mcp_fields) = &manifest.mcp {
                    if let Some(formats) = &mcp_fields.formats {
                        if let Some(compat) = &manifest.common.compatibility {
                            for entry in compat {
                                if let Some(ref mcp_format) = entry.mcp_format {
                                    if !formats.iter().any(|f| f == mcp_format) {
                                        errors.push(ValidationError {
                                            field: format!("compatibility[{}].mcpFormat", entry.tool),
                                            message: format!("工具 {} 要求 MCP 格式 {}，但 mcp.formats 未包含此格式", entry.tool, mcp_format),
                                            code: "FORMAT_MISMATCH".into(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Kind::Plugin => {
                if manifest.plugin.is_none() {
                    errors.push(ValidationError { field: "plugin".into(), message: "kind 为 plugin 时 plugin.entry 和 plugin.type 必填".into(), code: "REQUIRED".into() });
                }
            }
        }
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}

fn is_valid_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() >= 3 && parts.iter().take(3).all(|p| p.parse::<u32>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_skill_manifest() {
        let json = r#"{"id":"com.example.test","name":"Test","version":"1.0.0","kind":"skill","skill":{"entry":"SKILL.md"}}"#;
        assert!(ManifestValidator::validate_json(json).is_ok());
    }

    #[test]
    fn test_missing_id() {
        let json = r#"{"name":"T","version":"1.0.0","kind":"skill","skill":{"entry":"SKILL.md"}}"#;
        let errors = ManifestValidator::validate_json(json).unwrap_err();
        assert!(errors.iter().any(|e| e.code == "PARSE_ERROR"));
    }

    #[test]
    fn test_invalid_semver() {
        let json = r#"{"id":"t","name":"T","version":"abc","kind":"skill","skill":{"entry":"SKILL.md"}}"#;
        let errors = ManifestValidator::validate_json(json).unwrap_err();
        assert!(errors.iter().any(|e| e.code == "INVALID_SEMVER"));
    }

    #[test]
    fn test_path_traversal() {
        let json = r#"{"id":"t","name":"T","version":"1.0.0","kind":"skill","skill":{"entry":"../../../etc/passwd"}}"#;
        let errors = ManifestValidator::validate_json(json).unwrap_err();
        assert!(errors.iter().any(|e| e.code == "PATH_TRAVERSAL"));
    }

    #[test]
    fn test_mcp_format_mismatch() {
        let json = r#"{"id":"t","name":"T","version":"1.0.0","kind":"mcp","mcp":{"command":"npx","formats":["json"]},"compatibility":[{"tool":"codex","mcpFormat":"toml"}]}"#;
        let errors = ManifestValidator::validate_json(json).unwrap_err();
        assert!(errors.iter().any(|e| e.code == "FORMAT_MISMATCH"));
    }
}
