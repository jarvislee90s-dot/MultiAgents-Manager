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

/// 卸载结果：needs_confirmation=true 表示 SSOT 技能仍被 ~/.agents/skills 直链引用，
/// 已在破坏性步骤前中断、未做任何变更，等待前端二级确认后以 force=true 重试
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallOutcome {
    pub needs_confirmation: bool,
}

/// `.agents_dir/<name>` 是否为指向 ssot_path 的直链（spec §4.4；情景 B「保留为共享」的形态：
/// `.agents/skills/<name>` → `~/.mam/skills/<name>`）。
/// 判据：路径本体是链接（symlink/junction，复用 linker::link_marker_is_present），
/// 且 read_link 的单跳字面 target 与 ssot_path 经同规则归一化（相对绝对化 + 词法消解 +
/// Windows 前缀剥离，共用 linker::normalize_link_target）后相等。
/// 真目录 / 普通文件 / 不存在 / 断链 / 指向其他位置 → false。
fn agents_link_points_to(
    agents_dir: &std::path::Path,
    name: &str,
    ssot_path: &std::path::Path,
) -> bool {
    let link_path = agents_dir.join(name);
    // 必须是链接本体：真目录、普通文件、不存在均非直链
    if !crate::linker::link_marker_is_present(&link_path) {
        return false;
    }
    // read_link 读单跳字面 target（断链同样可读出 target，交由字面比较判定）
    let Ok(raw_target) = std::fs::read_link(&link_path) else {
        return false;
    };
    crate::linker::normalize_link_target(&raw_target, agents_dir)
        == crate::linker::normalize_link_target(ssot_path, agents_dir)
}

/// `.agents/<name>` 是否为 MAM 管理的 Layer 2 投影（review N-1）：链接本体存在，
/// 单跳字面 target 归一化后位于 active_root（`~/.mam/active`）之下即命中，
/// 无论链路是否可达（断链残链同样需要用户知情确认）。判定与迁移谓词同源
/// （linker::normalize_link_target 统一归一化 + 组件级前缀比较）
fn agents_link_targets_layer2(
    agents_dir: &std::path::Path,
    name: &str,
    active_root: &std::path::Path,
) -> bool {
    let link_path = agents_dir.join(name);
    if !crate::linker::link_marker_is_present(&link_path) {
        return false;
    }
    let Ok(raw) = std::fs::read_link(&link_path) else {
        return false;
    };
    crate::linker::normalize_link_target(&raw, agents_dir).starts_with(active_root)
}

/// 卸载资源：.agents 直链守卫（spec §4.4）→ 清理所有工具的分配与配置 → 删 SSOT → 删 DB 行 → 删 store 索引
///
/// kind == "skill" 时，若 `~/.agents/skills/<name>` 是指向 SSOT 的直链且 force != true，
/// 返回 needs_confirmation=true 且不做任何变更（在任何破坏性步骤之前中断）；
/// 前端二级确认后以 force=true 重试走原流程。确认后强制删除时**不碰 .agents 直链**
/// （spec §4.2：MAM 永不写共享目录，§4.3 迁移对话框是唯一例外）——删除 SSOT 后直链将悬空，
/// 这是用户确认后的预期结果，不做清理。
///
/// review N-1：迁移窗口期内的遗留链指向 Layer 2（`~/.mam/active/…`）而非 Layer 1，
/// 同样触发确认——无提示卸载会连带删掉 Layer 1/Layer 2，经 `.agents` 消费该技能的
/// 外部工具（zcode 等）静默失效，且事后对话框的「保留为共享」因 Layer 1 缺失只能
/// skip，用户失去知情选择权。
#[tauri::command]
pub fn uninstall_resource(
    kind: String,
    name: String,
    force: Option<bool>,
) -> Result<UninstallOutcome, String> {
    if !["skill", "mcp", "plugin"].contains(&kind.as_str()) {
        return Err(format!("未知资源类型: {}", kind));
    }
    let ext_id = format!("{}-{}", kind, name);
    let record = crate::database::list_extensions()
        .into_iter()
        .find(|e| e.kind == kind && e.name == name);

    // 0) SSOT 删除保护（spec §4.4 + review N-1）：在任何破坏性步骤（逐工具清理）之前，
    //    检查 ~/.agents/skills/<name> 是否为指向 SSOT 的直链（情景 B「保留为共享」形态）
    //    或指向 Layer 2 的 MAM 遗留投影（迁移窗口期形态）；命中且未强制 → 零改动要求确认。
    //    SSOT 候选选取与下方 SSOT 删除循环同规则（第一个 is_file/is_dir 者）。
    if kind == "skill" && force != Some(true) {
        let home = dirs::home_dir().unwrap_or_default();
        let agents_dir = home.join(".agents").join("skills");
        let ssot_hit = resolve_ssot_paths("skill", &name, record.as_ref().map(|r| r.id.as_str()))
            .into_iter()
            .find(|p| p.is_file() || p.is_dir())
            .is_some_and(|ssot_path| agents_link_points_to(&agents_dir, &name, &ssot_path));
        let layer2_hit =
            agents_link_targets_layer2(&agents_dir, &name, &home.join(".mam").join("active"));
        if ssot_hit || layer2_hit {
            log::info!(
                "SSOT 技能 {} 仍被 ~/.agents/skills 引用（{}），需用户确认后卸载",
                name,
                if ssot_hit {
                    "直链"
                } else {
                    "Layer 2 遗留投影"
                }
            );
            return Ok(UninstallOutcome {
                needs_confirmation: true,
            });
        }
    }

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
        let mam_mcp = dirs::home_dir()
            .unwrap_or_default()
            .join(".mam")
            .join("mcp");
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
    Ok(UninstallOutcome {
        needs_confirmation: false,
    })
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

    // —— agents_link_points_to 直链判定（spec §7.8）——

    /// tempdir 内构造 `<tmp>/mam/skills/<tag>`（SSOT 真目录）+ `<tmp>/agents/skills`，
    /// 零真实家目录访问；返回 (TempDir 保活, agents_dir, ssot_path)
    fn setup_layout(tag: &str) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("mam").join("skills").join(tag);
        std::fs::create_dir_all(&ssot).unwrap();
        let agents_dir = tmp.path().join("agents").join("skills");
        std::fs::create_dir_all(&agents_dir).unwrap();
        (tmp, agents_dir, ssot)
    }

    /// 跨平台建立目录直链（Unix symlink / Windows junction）
    fn make_dir_link(target: &std::path::Path, link: &std::path::Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        junction::create(target, link).unwrap();
    }

    #[test]
    fn agents_direct_link_to_ssot_detected() {
        let (_tmp, agents_dir, ssot) = setup_layout("foo");
        make_dir_link(&ssot, &agents_dir.join("foo"));
        assert!(agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    #[test]
    fn agents_native_dir_is_not_direct_link() {
        let (_tmp, agents_dir, ssot) = setup_layout("foo");
        // 手装真目录（非链接）→ false
        std::fs::create_dir_all(agents_dir.join("foo")).unwrap();
        assert!(!agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    #[test]
    fn agents_unrelated_link_rejected() {
        let (tmp, agents_dir, ssot) = setup_layout("foo");
        // 指向 SSOT 之外位置的外部链接 → false
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        make_dir_link(&elsewhere, &agents_dir.join("foo"));
        assert!(!agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    #[test]
    fn agents_missing_entry_rejected() {
        let (_tmp, agents_dir, ssot) = setup_layout("foo");
        // `.agents/skills/<name>` 不存在 → false
        assert!(!agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    #[test]
    fn agents_dangling_link_rejected() {
        let (tmp, agents_dir, ssot) = setup_layout("foo");
        // 断链（target 不存在）→ read_link 可读出字面 target，但与 ssot_path 不等 → false
        let gone = tmp.path().join("mam").join("skills").join("gone");
        make_dir_link(&gone, &agents_dir.join("foo"));
        assert!(!agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    #[test]
    #[cfg(unix)]
    fn agents_relative_link_target_resolved() {
        let (_tmp, agents_dir, ssot) = setup_layout("foo");
        // 相对 target（../../mam/skills/foo）按链接所在目录绝对化后应命中同一直链语义
        let rel = std::path::Path::new("../../mam/skills/foo");
        std::os::unix::fs::symlink(rel, agents_dir.join("foo")).unwrap();
        assert!(agents_link_points_to(&agents_dir, "foo", &ssot));
    }

    // —— agents_link_targets_layer2（review N-1：卸载守卫的 Layer 2 遗留投影分支）——

    /// tempdir 内构造 `<tmp>/mam/active`（Layer 2 根）供投影判定用
    fn active_root_of(tmp: &tempfile::TempDir) -> std::path::PathBuf {
        tmp.path().join("mam").join("active")
    }

    #[test]
    #[cfg(unix)]
    fn layer2_projection_links_are_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents").join("skills");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let active = active_root_of(&tmp);

        // 可达投影：agents/foo → active/codex/foo（Layer 1 真目录在 SSOT 侧）
        let layer1 = tmp.path().join("mam").join("skills").join("foo");
        std::fs::create_dir_all(&layer1).unwrap();
        std::fs::create_dir_all(active.join("codex")).unwrap();
        make_dir_link(&layer1, &active.join("codex").join("foo"));
        make_dir_link(&active.join("codex").join("foo"), &agents_dir.join("foo"));
        assert!(agents_link_targets_layer2(&agents_dir, "foo", &active));

        // 断链投影：Layer 2 被删（disable/uninstall 先行）→ 残链同样需要知情确认
        let dangling = agents_dir.join("stale");
        make_dir_link(&active.join("codex").join("gone"), &dangling);
        assert!(agents_link_targets_layer2(&agents_dir, "stale", &active));
    }

    #[test]
    #[cfg(unix)]
    fn layer2_projection_excludes_external_and_direct_layer1_links() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().join("agents").join("skills");
        std::fs::create_dir_all(&agents_dir).unwrap();
        let active = active_root_of(&tmp);

        // 外部无关链接（target 不在 ~/.mam/active 下）→ 不命中（直链分支另行处理）
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        make_dir_link(&elsewhere, &agents_dir.join("ext"));
        assert!(!agents_link_targets_layer2(&agents_dir, "ext", &active));

        // Layer 1 直链（情景 B「保留为共享」产物）→ 不在本分支命中（走直链守卫）
        let layer1 = tmp.path().join("mam").join("skills").join("kept");
        std::fs::create_dir_all(&layer1).unwrap();
        make_dir_link(&layer1, &agents_dir.join("kept"));
        assert!(!agents_link_targets_layer2(&agents_dir, "kept", &active));

        // 真目录 / 不存在 → 不命中
        std::fs::create_dir_all(agents_dir.join("native")).unwrap();
        assert!(!agents_link_targets_layer2(&agents_dir, "native", &active));
        assert!(!agents_link_targets_layer2(&agents_dir, "missing", &active));
    }
}
