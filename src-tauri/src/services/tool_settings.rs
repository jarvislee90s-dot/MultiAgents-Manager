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
    /// 未能还原的项（SSOT 缺失或暂存失败，链接保持不变）——保存结果中逐项报告（spec W5 清理语义 1 + §9）
    pub skipped: Vec<String>,
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
/// 连接参数变体（review F4：内存库可测）
pub fn ensure_tool_enabled_conn(conn: &rusqlite::Connection, tool_id: &str) -> Result<(), String> {
    if agent_tool::get_tool_enabled_conn(conn, tool_id) {
        Ok(())
    } else {
        Err(format!(
            "工具 {} 未启用，请先在设置-工具管理中开启",
            tool_id
        ))
    }
}

pub fn ensure_tool_enabled(tool_id: &str) -> Result<(), String> {
    let conn = crate::database::connection::DB.lock().unwrap();
    ensure_tool_enabled_conn(&conn, tool_id)
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
                    report_restore(
                        restore_mam_link(&ssot, &dir.join(&ext.name), &ext.name),
                        &ext.name,
                        result,
                    );
                }
            }
            "plugin" => {
                if let Some(adapter) = crate::adapter::adapter_by_id(tool_id) {
                    if let Some(dir) = adapter.plugin_dirs().first() {
                        // 文件型插件 SSOT：~/.mam/plugins/<name>
                        let ssot = home.join(".mam").join("plugins").join(&ext.name);
                        report_restore(
                            restore_mam_link(&ssot, &dir.join(&ext.name), &ext.name),
                            &ext.name,
                            result,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    // 未读卡一并清除（取消勾选立即彻底隐藏）
    crate::database::dao::unread::clear_tool(tool_id);
    // 同步清空 W4 心跳消失补偿的观测表：只清未读表不够——停用后任务随即完成、prewarm
    // 回池删除心跳文件时，下一轮补偿会凭残留的 pid→session 记录为已停用工具重新
    // upsert 未读行，让刚清掉的未读卡「复活」。按工具隔离清理（P2-3）：只移除属于
    // 该工具的条目，保留其他工具（未来心跳驱动工具）的观测记录
    crate::monitor::workbuddy_parser::LAST_SEEN_SESSIONS
        .lock()
        .unwrap()
        .retain(|_, (tool, _)| tool != tool_id);
}

/// 还原单项结果（spec W5 清理语义 1 + §9：SSOT 缺失跳过并在保存结果中逐项报告）
#[derive(Debug, Clone, PartialEq, Eq)]
enum RestoreOutcome {
    /// 已完整还原为真实内容
    Restored,
    /// 未能还原且链接保持不变（SSOT 缺失 / 暂存失败 / 落位失败），由调用方计入 skipped 逐项报告
    Skipped,
    /// 无需处理：目标非 MAM 链接态（原生目录或不存在），不报告也不计数
    NotApplicable,
}

/// 按还原结果归账：完全还原计入 restored；跳过/失败计入 skipped 供前端逐项提示
fn report_restore(outcome: RestoreOutcome, name: &str, result: &mut ApplyResult) {
    match outcome {
        RestoreOutcome::Restored => result.restored.push(name.to_string()),
        RestoreOutcome::Skipped => result.skipped.push(name.to_string()),
        RestoreOutcome::NotApplicable => {}
    }
}

/// 还原单个「MAM 建的链接」为真实内容：仅链接态（Valid/Dangling）处理，
/// 原生目录（NotLink）与不存在（Missing）不动（NotApplicable）；
/// SSOT 缺失或还原中途失败 → Skipped（链接保持不变），由调用方逐项报告给用户。
/// 先把 SSOT 内容暂存到目标旁的临时路径（目录走 copy_dir_recursive，
/// 单文件如配置型插件的 .json 走 fs::copy），暂存成功才移除链接并原子落位；
/// 任一步失败保持现场并 log::warn。
fn restore_mam_link(
    ssot: &std::path::Path,
    target: &std::path::Path,
    name: &str,
) -> RestoreOutcome {
    let health = crate::linker::check_link_health(target);
    if !matches!(
        health,
        crate::linker::LinkHealth::Valid | crate::linker::LinkHealth::Dangling
    ) {
        return RestoreOutcome::NotApplicable;
    }
    if !ssot.exists() {
        log::warn!(
            "还原 {} 跳过：SSOT 缺失（{}），链接保持不变",
            name,
            ssot.display()
        );
        return RestoreOutcome::Skipped;
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
        log::warn!("还原 {} 跳过：SSOT 暂存出错（{}），链接保持不变", name, e);
        return RestoreOutcome::Skipped;
    }
    if let Err(e) = crate::linker::remove_link(target) {
        let _ = crate::linker::remove_link(&tmp);
        log::warn!("还原 {} 跳过：移除旧链接出错（{}），链接保持不变", name, e);
        return RestoreOutcome::Skipped;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = crate::linker::remove_link(&tmp);
        log::warn!(
            "还原 {} 跳过：临时内容落位出错（{}），需重新勾选后重建",
            name,
            e
        );
        return RestoreOutcome::Skipped;
    }
    RestoreOutcome::Restored
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

#[cfg(test)]
mod restore_tests {
    use super::*;

    /// 目录型链接的平台无关建链：Unix 走 symlink、Windows 走 junction（P1-3 A：create_link 抽象）。
    /// 注意 create_link 对「单文件」在 Windows 上是 fs::copy 而非链接——文件型还原测试因此
    /// 只断言「还原为真实文件」的结果（不要求先存在链接，见 restores_single_file_link_to_real_file）
    fn create_dir_link(source: &std::path::Path, target: &std::path::Path) {
        crate::linker::create_link(source, target).unwrap();
    }

    /// 目录型 SSOT：链接还原为真实目录（含内容），返回 Restored，临时目录不残留
    #[test]
    fn restores_dir_link_to_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("skill-a");
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "hello").unwrap();
        let target = tmp.path().join("tools").join("skill-a");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        create_dir_link(&ssot, &target);

        let outcome = restore_mam_link(&ssot, &target, "skill-a");

        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(target.is_dir());
        assert!(
            !crate::linker::link_marker_is_present(&target),
            "还原后应不再是链接"
        );
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

    /// 单文件型 SSOT（配置型插件 .json）：走 fs::copy 暂存后还原为真实文件。
    /// 说明：Windows 上 create_link 对文件目标即 fs::copy（无文件链接），因此本测试
    /// 在双平台都以「还原为真实文件且内容正确」为断言，不依赖先存在链接
    #[test]
    fn restores_single_file_link_to_real_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("my-plugin.json");
        std::fs::create_dir_all(ssot.parent().unwrap()).unwrap();
        std::fs::write(&ssot, r#"{"k":"v"}"#).unwrap();
        let target = tmp.path().join("tools").join("my-plugin.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        // Windows 上 create_link 对文件是 copy（不是链接），restore 视为 NotApplicable——
        // 直接测「文件目标由 copy 建链后 restore 为真实文件」的等价行为：Unix 建 symlink，
        // Windows 建 copy 目标，restore 后都是真实文件
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ssot, &target).unwrap();
        #[cfg(windows)]
        std::fs::copy(&ssot, &target).unwrap();

        // Unix：链接 → 还原为真实文件；Windows：已是真实副本 → NotApplicable（不报告）
        #[cfg(unix)]
        {
            let outcome = restore_mam_link(&ssot, &target, "my-plugin.json");
            assert_eq!(outcome, RestoreOutcome::Restored);
        }
        #[cfg(windows)]
        {
            let outcome = restore_mam_link(&ssot, &target, "my-plugin.json");
            assert_eq!(outcome, RestoreOutcome::NotApplicable);
        }
        assert!(target.is_file());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), r#"{"k":"v"}"#);
    }

    /// 暂存失败（SSOT 目录含悬空链接）：链接保持原样，返回 Skipped（调用方计入 skipped 逐项报告）
    #[test]
    fn keeps_link_when_staging_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("broken");
        std::fs::create_dir_all(&ssot).unwrap();
        // 目录内含悬空链接 → copy_dir_recursive 的 fs::copy 必然失败
        let dangling_target = tmp.path().join("no-such-file");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&dangling_target, ssot.join("bad")).unwrap();
        #[cfg(windows)]
        {
            // Windows 无 dangling symlink 直接构造；junction 须指向真实存在的源——
            // 用「先建后删」制造悬空 junction 目标
            std::fs::create_dir_all(&dangling_target).unwrap();
            crate::linker::create_link(&dangling_target, &ssot.join("bad")).unwrap();
            std::fs::remove_dir_all(&dangling_target).unwrap();
        }
        let target = tmp.path().join("tools").join("broken");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        create_dir_link(&ssot, &target);

        let outcome = restore_mam_link(&ssot, &target, "broken");

        assert_eq!(outcome, RestoreOutcome::Skipped);
        assert!(
            crate::linker::link_marker_is_present(&target),
            "暂存失败时链接不应被移除"
        );
    }

    /// SSOT 缺失（spec W5 清理语义 1 + §9）：链接保持原样，返回 Skipped 供调用方逐项报告
    #[test]
    fn skips_and_keeps_link_when_ssot_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // SSOT 未创建（~/.mam/skills/<name> 被用户删除的场景），链接悬空
        let ssot = tmp.path().join("repo").join("gone");
        let target = tmp.path().join("tools").join("gone");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&ssot, &target).unwrap();
        #[cfg(windows)]
        {
            // junction 须指向存在的源：先建目录再建 junction 后删除源 → 悬空
            std::fs::create_dir_all(&ssot).unwrap();
            create_dir_link(&ssot, &target);
            std::fs::remove_dir_all(&ssot).unwrap();
        }

        let outcome = restore_mam_link(&ssot, &target, "gone");

        assert_eq!(outcome, RestoreOutcome::Skipped);
        assert!(
            crate::linker::link_marker_is_present(&target),
            "SSOT 缺失时链接不应被动"
        );
    }

    /// 目标为原生目录（非 MAM 链接态）：无需还原，返回 NotApplicable（不报告也不计入 restored）
    #[test]
    fn not_applicable_for_native_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("native");
        std::fs::create_dir_all(&ssot).unwrap();
        let target = tmp.path().join("tools").join("native");
        std::fs::create_dir_all(&target).unwrap(); // 原生目录，非链接

        let outcome = restore_mam_link(&ssot, &target, "native");

        assert_eq!(outcome, RestoreOutcome::NotApplicable);
        assert!(target.is_dir());
        assert!(!crate::linker::link_marker_is_present(&target));
    }

    // ---- P1-3 B：Windows junction 专项（#[cfg(windows)]） ----

    /// junction 的 Valid/Dangling 健康判定 + remove_link 只删链接不删 SSOT 内容
    #[cfg(windows)]
    #[test]
    fn windows_junction_health_and_remove_only_link() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("skill");
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "ssot content").unwrap();
        let target = tmp.path().join("tools").join("skill");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        create_dir_link(&ssot, &target);

        // Valid：junction 目标可达
        assert_eq!(
            crate::linker::check_link_health(&target),
            crate::linker::LinkHealth::Valid
        );
        // 删除 SSOT → Dangling
        std::fs::remove_dir_all(&ssot).unwrap();
        assert_eq!(
            crate::linker::check_link_health(&target),
            crate::linker::LinkHealth::Dangling
        );
        // remove_link 只删链接本身，不碰 SSOT（SSOT 已被删，验证不误删）
        crate::linker::remove_link(&target).unwrap();
        assert!(!target.exists() && !target.is_symlink());
    }

    /// 跨 junction 的 restore_mam_link：暂存 + rename 落位为真实内容
    #[cfg(windows)]
    #[test]
    fn windows_restore_across_junction_materializes_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("skill-a");
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), "hello").unwrap();
        let target = tmp.path().join("tools").join("skill-a");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        create_dir_link(&ssot, &target); // junction

        let outcome = restore_mam_link(&ssot, &target, "skill-a");

        assert_eq!(outcome, RestoreOutcome::Restored);
        assert!(target.is_dir());
        assert!(!crate::linker::link_marker_is_present(&target));
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            "hello"
        );
    }

    /// SSOT 缺失时 Windows junction 还原报告 Skipped 且链接不动
    #[cfg(windows)]
    #[test]
    fn windows_ssot_missing_reports_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let ssot = tmp.path().join("repo").join("gone");
        let target = tmp.path().join("tools").join("gone");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        // 先建源再建 junction，随后删源 → 悬空 junction（Windows 不能直接建悬空 junction）
        std::fs::create_dir_all(&ssot).unwrap();
        create_dir_link(&ssot, &target);
        std::fs::remove_dir_all(&ssot).unwrap();

        let outcome = restore_mam_link(&ssot, &target, "gone");

        assert_eq!(outcome, RestoreOutcome::Skipped);
        assert!(crate::linker::link_marker_is_present(&target));
    }
}

#[cfg(test)]
mod ensure_guard_tests {
    use super::*;

    /// review F4：守卫的连接参数变体——停用工具 → 明确错误；启用/缺行 → 放行
    #[test]
    fn ensure_tool_enabled_conn_blocks_disabled_tool() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        agent_tool::ensure_tool_rows_conn(&conn);
        agent_tool::set_tool_enabled_conn(&conn, "opencode", false);
        let err = ensure_tool_enabled_conn(&conn, "opencode").unwrap_err();
        assert!(err.contains("opencode"), "错误信息须包含工具 id：{err}");
        assert!(err.contains("工具管理"), "错误信息须给出恢复路径：{err}");
        // 缺行防御视为启用
        assert!(ensure_tool_enabled_conn(&conn, "nonexistent").is_ok());
        // 重新启用 → 放行
        agent_tool::set_tool_enabled_conn(&conn, "opencode", true);
        assert!(ensure_tool_enabled_conn(&conn, "opencode").is_ok());
    }
}
