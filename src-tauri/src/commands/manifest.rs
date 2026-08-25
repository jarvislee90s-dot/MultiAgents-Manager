// Manifest 相关 IPC 命令

use crate::services::manifest::{ManifestValidator, ValidationError};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateResult {
    pub valid: bool,
    pub manifest: Option<crate::services::manifest::Manifest>,
    pub errors: Option<Vec<ValidationError>>,
}

#[tauri::command]
pub fn validate_manifest(path: String) -> ValidateResult {
    match ManifestValidator::validate_file(std::path::Path::new(&path)) {
        Ok(manifest) => ValidateResult {
            valid: true,
            manifest: Some(manifest),
            errors: None,
        },
        Err(errors) => ValidateResult {
            valid: false,
            manifest: None,
            errors: Some(errors),
        },
    }
}

#[tauri::command]
pub fn install_resource_from_manifest(path: String) -> Result<(), String> {
    let manifest =
        ManifestValidator::validate_file(std::path::Path::new(&path)).map_err(|errors| {
            errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ")
        })?;

    let mam_dir = dirs::home_dir().unwrap_or_default().join(".mam");
    let dest_dir = match manifest.common.kind {
        crate::services::manifest::Kind::Skill => mam_dir.join("skills").join(&manifest.common.id),
        crate::services::manifest::Kind::Mcp => mam_dir.join("mcp").join(&manifest.common.id),
        crate::services::manifest::Kind::Plugin => {
            mam_dir.join("plugins").join(&manifest.common.id)
        }
    };

    let source = std::path::Path::new(&path)
        .parent()
        .ok_or("无法获取资源目录")?;
    crate::linker::copy_dir_recursive(source, &dest_dir)?;

    let manifest_dest = dest_dir.join("mam.json");
    std::fs::copy(&path, &manifest_dest).map_err(|e| e.to_string())?;

    crate::services::manifest::store::add_entry(&manifest)?;

    let ext = crate::database::ExtensionRecord {
        id: manifest.common.id.clone(),
        kind: format!("{:?}", manifest.common.kind).to_lowercase(),
        name: manifest.common.name.clone(),
        description: manifest.common.description.clone(),
        source_path: dest_dir.to_string_lossy().to_string(),
        source_url: manifest.common.homepage.clone(),
        version: None,
        // 插件按约定（见 preset/mod.rs:40 注释）在 tags 存 "file"/"config" 子类型，
        // 供 toggle_plugin 识别；其余 kind 存 manifest 元数据标签
        tags: match manifest.common.kind {
            crate::services::manifest::Kind::Plugin => {
                manifest.plugin.as_ref().map(|p| p.plugin_type.clone())
            }
            _ => manifest.common.tags.as_ref().map(|t| t.join(",")),
        },
        suite: None,
        source_tool: None,
        is_native: false,
    };
    crate::database::insert_extension(&ext)?;
    Ok(())
}

/// SSOT 路径候选：目录类资源 [<name>, <id>]（普通安装用 name，manifest 安装用 id），
/// MCP 为 <name>.json / <id>.json 文件。取第一个存在者删除。
fn resolve_ssot_paths(kind: &str, name: &str, record_id: Option<&str>) -> Vec<std::path::PathBuf> {
    let mam = dirs::home_dir().unwrap_or_default().join(".mam");
    let mut candidates = Vec::new();
    match kind {
        "skill" | "plugin" => {
            let dir = if kind == "skill" { "skills" } else { "plugins" };
            candidates.push(mam.join(dir).join(name));
            if let Some(id) = record_id {
                candidates.push(mam.join(dir).join(id));
            }
        }
        "mcp" => {
            candidates.push(mam.join("mcp").join(format!("{}.json", name)));
            if let Some(id) = record_id {
                candidates.push(mam.join("mcp").join(format!("{}.json", id)));
            }
        }
        _ => {}
    }
    candidates
}

/// 卸载资源：清理所有工具的分配与配置 → 删 SSOT → 删 DB 行 → 删 store 索引
#[tauri::command]
pub fn uninstall_resource(kind: String, name: String) -> Result<(), String> {
    if !["skill", "mcp", "plugin"].contains(&kind.as_str()) {
        return Err(format!("未知资源类型: {}", kind));
    }
    let ext_id = format!("{}-{}", kind, name);
    let record = crate::database::list_extensions()
        .into_iter()
        .find(|e| e.kind == kind && e.name == name);

    // 1) 按工具清理（一律用 name，assignment 键约定为 kind-name）
    // 同一工具可能有多条 assignment（含子 Agent 维度），用 BTreeSet 去重避免重复清理
    let tools: std::collections::BTreeSet<String> = crate::database::list_all_assignments()
        .iter()
        .filter(|a| a.extension_id == ext_id)
        .map(|a| a.agent_tool_id.clone())
        .collect();
    for tool_id in tools {
        let result = match kind.as_str() {
            "skill" => crate::services::skill::disable_skill_for_tool(&name, &tool_id),
            "mcp" => crate::services::mcp::remove_mcp(&tool_id, &name),
            "plugin" => {
                let plugin_kind = record
                    .as_ref()
                    .and_then(|r| r.tags.clone())
                    .unwrap_or_else(|| "file".to_string());
                crate::services::plugin::toggle_plugin(&name, &tool_id, false, &plugin_kind)
            }
            _ => unreachable!(),
        };
        if let Err(e) = result {
            log::warn!("卸载清理 {} ({}) 失败: {}", name, tool_id, e);
        }
    }

    // 2) 删除 SSOT 文件/目录（取第一个存在的候选）
    for path in resolve_ssot_paths(&kind, &name, record.as_ref().map(|r| r.id.as_str())) {
        if path.is_file() {
            if let Err(e) = std::fs::remove_file(&path) {
                log::warn!("删除 SSOT 路径失败 {}: {}", path.display(), e);
            }
            break;
        }
        if path.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                log::warn!("删除 SSOT 路径失败 {}: {}", path.display(), e);
            }
            break;
        }
    }

    // 2.1) manifest 安装的 MCP 以目录形式存放于 ~/.mam/mcp/<id>/（而非 <name>.json 文件），
    //      上面文件/目录候选循环不会命中，这里按 manifest 安装布局补充目录清理
    if kind == "mcp" {
        let mam_mcp = dirs::home_dir().unwrap_or_default().join(".mam").join("mcp");
        let mut dir_candidates = vec![mam_mcp.join(&name)];
        if let Some(r) = record.as_ref() {
            dir_candidates.push(mam_mcp.join(&r.id));
        }
        for dir in dir_candidates {
            if dir.is_dir() {
                if let Err(e) = std::fs::remove_dir_all(&dir) {
                    log::warn!("删除 SSOT 路径失败 {}: {}", dir.display(), e);
                }
                break;
            }
        }
    }

    // 3) 删除 DB 行（约定 id 与 manifest 安装 id 两种）
    let _ = crate::database::delete_assignments_for(&ext_id);
    let _ = crate::database::delete_extension(&ext_id);
    if let Some(ref r) = record {
        if r.id != ext_id {
            let _ = crate::database::delete_assignments_for(&r.id);
            let _ = crate::database::delete_extension(&r.id);
        }
    }

    // 4) store 索引（manifest 安装才有；无条目时忽略）
    let store_id = record.as_ref().map(|r| r.id.clone()).unwrap_or(ext_id);
    if let Err(e) = crate::services::manifest::store::remove_entry(&store_id) {
        log::debug!("store 索引无 {} 条目，跳过: {}", name, e);
    }
    log::info!("资源已卸载: {} ({})", name, kind);
    Ok(())
}

#[tauri::command]
pub fn get_store_index() -> Result<serde_json::Value, String> {
    crate::services::manifest::store::read_index()
}

#[cfg(test)]
mod uninstall_tests {
    use super::*;

    fn norm(p: &std::path::Path) -> String {
        p.to_string_lossy().replace('\\', "/")
    }

    #[test]
    fn resolves_dir_candidates_in_order() {
        let ps = resolve_ssot_paths("skill", "foo", Some("foo-1.0"));
        assert!(norm(&ps[0]).ends_with(".mam/skills/foo"));
        assert!(norm(&ps[1]).ends_with(".mam/skills/foo-1.0"));
    }

    #[test]
    fn resolves_mcp_json_file_only() {
        let ps = resolve_ssot_paths("mcp", "firecrawl", None);
        assert_eq!(ps.len(), 1);
        assert!(norm(&ps[0]).ends_with(".mam/mcp/firecrawl.json"));
    }

    #[test]
    fn unknown_kind_yields_no_candidates() {
        assert!(resolve_ssot_paths("widget", "x", None).is_empty());
    }
}
