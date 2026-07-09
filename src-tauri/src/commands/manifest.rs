// Manifest 相关 IPC 命令

use crate::services::manifest::{ManifestValidator, ValidationError};
use serde::Serialize;
use tauri::{Builder, Runtime};

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
        Ok(manifest) => ValidateResult { valid: true, manifest: Some(manifest), errors: None },
        Err(errors) => ValidateResult { valid: false, manifest: None, errors: Some(errors) },
    }
}

#[tauri::command]
pub fn install_resource_from_manifest(path: String) -> Result<(), String> {
    let manifest = ManifestValidator::validate_file(std::path::Path::new(&path))
        .map_err(|errors| errors.iter().map(|e| format!("{}: {}", e.field, e.message)).collect::<Vec<_>>().join("; "))?;

    let mam_dir = dirs::home_dir().unwrap_or_default().join(".mam");
    let dest_dir = match manifest.common.kind {
        crate::services::manifest::Kind::Skill => mam_dir.join("skills").join(&manifest.common.id),
        crate::services::manifest::Kind::Mcp => mam_dir.join("mcp").join(&manifest.common.id),
        crate::services::manifest::Kind::Plugin => mam_dir.join("plugins").join(&manifest.common.id),
    };

    let source = std::path::Path::new(&path).parent().ok_or("无法获取资源目录")?;
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
        tags: manifest.common.tags.as_ref().map(|t| t.join(",")),
        suite: None,
        source_tool: None,
        is_native: false,
    };
    crate::database::insert_extension(&ext)?;
    Ok(())
}

#[tauri::command]
pub fn uninstall_resource(ext_id: String, kind: String) -> Result<(), String> {
    let mam_dir = dirs::home_dir().unwrap_or_default().join(".mam");
    let kind_dir = match kind.as_str() {
        "skill" => "skills",
        "mcp" => "mcp",
        "plugin" => "plugins",
        _ => return Err(format!("未知资源类型: {}", kind)),
    };
    let resource_dir = mam_dir.join(kind_dir).join(&ext_id);

    // 移除所有工具的分配
    let assignments = crate::database::list_all_assignments();
    for assignment in assignments.iter().filter(|a| a.extension_id == ext_id) {
        match kind.as_str() {
            "skill" => { let _ = crate::services::skill::disable_skill_for_tool(&ext_id, &assignment.agent_tool_id); }
            "mcp" => { let _ = crate::services::mcp::remove_mcp(&assignment.agent_tool_id, &ext_id); }
            "plugin" => { let _ = crate::services::plugin::toggle_plugin(&ext_id, &assignment.agent_tool_id, false, "file"); }
            _ => {}
        }
    }

    if resource_dir.exists() {
        std::fs::remove_dir_all(&resource_dir).map_err(|e| format!("删除目录失败: {}", e))?;
    }

    crate::services::manifest::store::remove_entry(&ext_id)?;
    Ok(())
}

#[tauri::command]
pub fn get_store_index() -> Result<serde_json::Value, String> {
    crate::services::manifest::store::read_index()
}

pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        validate_manifest, install_resource_from_manifest, uninstall_resource, get_store_index
    ])
}
