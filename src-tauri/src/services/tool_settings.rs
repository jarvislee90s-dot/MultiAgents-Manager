// 工具勾选管理（spec W5）：查询（含 managed 标志）与保存时的清理/重建。
// 取消勾选 = skill/文件型插件的「MAM 链接」还原为真实文件 + MAM 管理的 MCP 条目移除 + 未读卡清除；
// SSOT 仓库与 DB 分配关系全部保留（禁止删除）；重新勾选按原分配幂等重建。

use crate::database::dao::{agent_tool, extension};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSetting {
    pub tool_id: String,
    pub name: String,
    pub enabled: bool,
    pub installed: bool,
    pub managed: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSettingChange {
    pub tool_id: String,
    pub enabled: bool,
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub restored: Vec<String>,
    pub restored_mcps: Vec<String>,
    pub rebuild_failed: Vec<String>,
}

pub fn get_tool_settings() -> Vec<ToolSetting> {
    agent_tool::ensure_tool_rows();
    crate::adapter::TOOL_IDS
        .iter()
        .filter_map(|id| {
            let adapter = crate::adapter::adapter_by_id(id)?;
            Some(ToolSetting {
                tool_id: id.to_string(),
                name: adapter.name().to_string(),
                enabled: agent_tool::get_tool_enabled(id),
                installed: adapter.base_dir().exists(),
                managed: tool_has_managed_content(id),
            })
        })
        .collect()
}

/// managed = 该工具存在启用的分配（skill/文件型插件链接或 MCP 条目）
fn tool_has_managed_content(tool_id: &str) -> bool {
    extension::list_all_assignments()
        .iter()
        .any(|a| a.agent_tool_id == tool_id && a.enabled)
}

/// toggle 类命令守卫：未勾选工具的资源管理操作直接拒绝
/// （spec W5「enable/disable 类命令返回明确错误」；仅守卫命令入口，
/// get_tool_settings / apply_tool_changes 本身不走此守卫，保证工具可重新开启）
pub fn ensure_tool_enabled(tool_id: &str) -> Result<(), String> {
    if agent_tool::get_tool_enabled(tool_id) {
        Ok(())
    } else {
        Err(format!(
            "工具 {} 未启用，请先在设置-工具管理中开启",
            tool_id
        ))
    }
}

pub fn apply_tool_changes(changes: Vec<ToolSettingChange>) -> ApplyResult {
    let mut result = ApplyResult::default();
    for c in &changes {
        let was = agent_tool::get_tool_enabled(&c.tool_id);
        // 状态未变跳过
        if was == c.enabled {
            continue;
        }
        agent_tool::set_tool_enabled(&c.tool_id, c.enabled);
        if !c.enabled {
            disable_tool_cleanup(&c.tool_id, &mut result);
        } else {
            rebuild_tool_links(&c.tool_id, &mut result);
        }
    }
    result
}

/// 取消勾选：链接还原 + MCP 条目移除 + 未读卡清除（spec W5 清理语义）
fn disable_tool_cleanup(tool_id: &str, result: &mut ApplyResult) {
    let home = dirs::home_dir().unwrap_or_default();
    let assignments: Vec<_> = extension::list_all_assignments()
        .into_iter()
        .filter(|a| a.agent_tool_id == tool_id && a.enabled)
        .collect();
    let extensions = extension::list_extensions();

    for a in &assignments {
        // MCP 分配没有 extensions 行（仅 assignment，id 形如 mcp-<name>），按前缀识别
        if let Some(mcp_name) = a.extension_id.strip_prefix("mcp-") {
            if crate::services::mcp::remove_mcp(tool_id, mcp_name).is_ok() {
                result.restored_mcps.push(mcp_name.to_string());
            }
            continue;
        }
        let Some(ext) = extensions.iter().find(|e| e.id == a.extension_id) else {
            continue;
        };
        match ext.kind.as_str() {
            "skill" => {
                if let Some(dir) = crate::adapter::skill_dir_for_tool(tool_id, &home) {
                    // SSOT skill 仓库即 ensure_repo_dir()（~/.mam/skills/<name>）
                    let ssot = crate::linker::ensure_repo_dir().join(&ext.name);
                    restore_mam_link(&ssot, &dir.join(&ext.name), &ext.name, result);
                }
            }
            "plugin" => {
                if let Some(adapter) = crate::adapter::adapter_by_id(tool_id) {
                    if let Some(dir) = adapter.plugin_dirs().first() {
                        // 文件型插件 SSOT：~/.mam/plugins/<name>
                        let ssot = home.join(".mam").join("plugins").join(&ext.name);
                        restore_mam_link(&ssot, &dir.join(&ext.name), &ext.name, result);
                    }
                }
            }
            _ => {}
        }
    }
    // 未读卡一并清除（取消勾选立即彻底隐藏）
    crate::database::dao::unread::clear_tool(tool_id);
}

/// 还原单个「MAM 建的链接」为真实内容：仅链接态（Valid/Dangling）处理，
/// 原生目录（NotLink）与不存在（Missing）不动；SSOT 缺失跳过并记录日志。
/// 先把 SSOT 内容暂存到目标旁的临时路径（目录走 copy_dir_recursive，
/// 单文件如配置型插件的 .json 走 fs::copy），暂存成功才移除链接并原子落位；
/// 任一步失败保持现场并 log::warn，不计入 restored。
fn restore_mam_link(
    ssot: &std::path::Path,
    target: &std::path::Path,
    name: &str,
    result: &mut ApplyResult,
) {
    let health = crate::linker::check_link_health(target);
    if !matches!(
        health,
        crate::linker::LinkHealth::Valid | crate::linker::LinkHealth::Dangling
    ) {
        return;
    }
    if !ssot.exists() {
        log::warn!("还原 {} 跳过：SSOT 缺失（{}）", name, ssot.display());
        return;
    }
    let tmp = target.with_extension("mam_restore_tmp");
    // 清理可能的历史残留，保证后续 rename 可落位
    let _ = crate::linker::remove_link(&tmp);
    // 先暂存（copy FIRST）：目录复制目录，单文件直接复制文件
    let staged = if ssot.is_dir() {
        crate::linker::copy_dir_recursive(ssot, &tmp)
    } else {
        std::fs::copy(ssot, &tmp)
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    if let Err(e) = staged {
        // 暂存失败：不动链接，工具内容保持原样
        let _ = crate::linker::remove_link(&tmp);
        log::warn!("还原 {} 失败：SSOT 暂存出错（{}），链接保持原样", name, e);
        return;
    }
    if let Err(e) = crate::linker::remove_link(target) {
        let _ = crate::linker::remove_link(&tmp);
        log::warn!("还原 {} 失败：移除旧链接出错（{}）", name, e);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = crate::linker::remove_link(&tmp);
        log::warn!("还原 {} 失败：临时内容落位出错（{}）", name, e);
        return;
    }
    result.restored.push(name.to_string());
}

/// 重新勾选：按原分配重建（幂等；失败项记录 rebuild_failed 不中断）
fn rebuild_tool_links(tool_id: &str, result: &mut ApplyResult) {
    let assignments: Vec<_> = extension::list_all_assignments()
        .into_iter()
        .filter(|a| a.agent_tool_id == tool_id && a.enabled)
        .collect();
    let extensions = extension::list_extensions();

    for a in &assignments {
        if let Some(mcp_name) = a.extension_id.strip_prefix("mcp-") {
            // SSOT MCP 配置：~/.mam/mcp/<name>.json（与 save_mcp_config / import_mcp_to_ssot 一致）
            let path = dirs::home_dir()
                .unwrap_or_default()
                .join(".mam")
                .join("mcp")
                .join(format!("{}.json", mcp_name));
            let ok = match std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<crate::services::mcp::McpConfig>(&s).ok())
            {
                Some(config) => crate::services::mcp::write_mcp(tool_id, mcp_name, &config).is_ok(),
                None => false,
            };
            if !ok {
                result.rebuild_failed.push(mcp_name.to_string());
            }
            continue;
        }
        let Some(ext) = extensions.iter().find(|e| e.id == a.extension_id) else {
            continue;
        };
        let ok = match ext.kind.as_str() {
            "skill" => crate::services::skill::enable_skill_for_tool(&ext.name, tool_id).is_ok(),
            // 插件重建按 extensions.tags 区分子类型（"file" | "config"），缺失/歧义回退 file
            // （与 preset/mod.rs 的 plugin_kind 读取一致）
            "plugin" => {
                let kind = if ext.tags.as_deref().map(str::trim) == Some("config") {
                    "config"
                } else {
                    "file"
                };
                crate::services::toggle_plugin(&ext.name, tool_id, true, kind).is_ok()
            }
            _ => true,
        };
        if !ok {
            result.rebuild_failed.push(ext.name.clone());
        }
    }
}

#[cfg(all(test, unix))]
mod restore_tests {
    use super::*;

    /// 目录型 SSOT：链接还原为真实目录（含内容），计入 restored，临时目录不残留
    #[test]
    fn restores_dir_link_to_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("skill-a");
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "hello").unwrap();
        let target = tmp.path().join("tools").join("skill-a");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&ssot, &target).unwrap();

        let mut result = ApplyResult::default();
        restore_mam_link(&ssot, &target, "skill-a", &mut result);

        assert_eq!(result.restored, vec!["skill-a".to_string()]);
        assert!(!target.is_symlink());
        assert!(target.is_dir());
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "hello"
        );
        assert!(!tmp
            .path()
            .join("tools")
            .join("skill-a.mam_restore_tmp")
            .exists());
    }

    /// 单文件型 SSOT（配置型插件 .json）：走 fs::copy 暂存后还原为真实文件
    #[test]
    fn restores_single_file_link_to_real_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("my-plugin.json");
        std::fs::create_dir_all(ssot.parent().unwrap()).unwrap();
        std::fs::write(&ssot, r#"{"k":"v"}"#).unwrap();
        let target = tmp.path().join("tools").join("my-plugin.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&ssot, &target).unwrap();

        let mut result = ApplyResult::default();
        restore_mam_link(&ssot, &target, "my-plugin.json", &mut result);

        assert_eq!(result.restored, vec!["my-plugin.json".to_string()]);
        assert!(!target.is_symlink());
        assert!(target.is_file());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), r#"{"k":"v"}"#);
    }

    /// 暂存失败（SSOT 目录含悬空 symlink）：链接保持原样，不计入 restored
    #[test]
    fn keeps_link_when_staging_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("broken");
        std::fs::create_dir_all(&ssot).unwrap();
        // 目录内含悬空 symlink → copy_dir_recursive 的 fs::copy 必然失败
        std::os::unix::fs::symlink(tmp.path().join("no-such-file"), ssot.join("bad")).unwrap();
        let target = tmp.path().join("tools").join("broken");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&ssot, &target).unwrap();

        let mut result = ApplyResult::default();
        restore_mam_link(&ssot, &target, "broken", &mut result);

        assert!(result.restored.is_empty());
        assert!(target.is_symlink(), "暂存失败时链接不应被移除");
    }
}
