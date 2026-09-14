// 预设组应用逻辑 — 独占应用（spec §5.1）+ 基底恢复（spec §5.2）+ 部分成功处理

pub mod snapshot;
pub mod stash;
pub mod sweep;

use crate::database;
use crate::services;
use log::info;

/// 应用预设（独占语义，spec §5.1）：开 = 拍基底（若无）→ 差集清扫 → 启用预设项
#[derive(Default, Debug)]
pub struct ApplyResult {
    pub success: usize,
    pub failures: Vec<String>,
    pub conflicts: Vec<String>,
    /// 本次被暂存的原生技能名
    pub stashed: Vec<String>,
    /// 本次被停用的 MAM 资源 extension_id
    pub disabled: Vec<String>,
    /// 预设原生技能项从暂存区接回的名（ensure-present 语义）
    pub restored_native: Vec<String>,
}

/// 工具私有预设的跨工具应用属硬错误（spec §3.3）
pub fn apply_preset(preset_id: &str, tool_id: &str) -> Result<ApplyResult, String> {
    let preset = database::get_preset(preset_id).ok_or_else(|| format!("预设不存在: {}", preset_id))?;
    if preset.scope == "tool" && preset.bound_tool.as_deref() != Some(tool_id) {
        return Err(format!(
            "预设 {} 绑定 {}，不能应用到 {}",
            preset.name,
            preset.bound_tool.as_deref().unwrap_or("?"),
            tool_id
        ));
    }
    let mut result = ApplyResult {
        success: 0,
        failures: Vec::new(),
        conflicts: Vec::new(),
        stashed: Vec::new(),
        disabled: Vec::new(),
        restored_native: Vec::new(),
    };

    // 1) 专属过滤（spec §6）：不兼容项剔除进 conflicts
    let all_items = database::get_preset_items(preset_id);
    let extensions = database::list_extensions();
    let mut apply_items: Vec<(String, String)> = Vec::new();
    for (ext_id, kind) in &all_items {
        if database::tool_allowed(ext_id, tool_id) {
            apply_items.push((ext_id.clone(), kind.clone()));
        } else {
            let name = extensions
                .iter()
                .find(|e| &e.id == ext_id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| ext_id.clone());
            result
                .conflicts
                .push(format!("{}（{}）：专属绑定不兼容 {}", name, kind, tool_id));
        }
    }

    // 2) 基底快照：无 → 拍（会话开始）；有 → 沿用，仅旧预设历史置 inactive（spec §3.2）
    match database::get_base_snapshot(tool_id) {
        None => snapshot::capture_base_snapshot(tool_id)?,
        Some((Some(old), _)) => {
            if old != preset_id {
                let _ = database::record_preset_application(&old, tool_id, false);
            }
        }
        Some((None, _)) => { /* 快照在而无激活（异常残留）——沿用快照即可 */ }
    }

    // 3) 独占清扫（差集 = 当前 − 预设项 − 常驻）
    let plan = sweep::plan_sweep(tool_id, &apply_items);
    let (disabled, stashed, sweep_failures) = sweep::execute_sweep(tool_id, &plan);
    result.disabled = disabled;
    result.stashed = stashed;
    result.failures.extend(sweep_failures);

    // 4) 启用预设项：MAM 走既有服务；原生技能 = 确保在场（spec §3.3）
    for (ext_id, kind) in &apply_items {
        let name = ext_id
            .strip_prefix(&format!("{}-", kind))
            .unwrap_or(ext_id);
        let is_native_item = kind == "skill"
            && extensions
                .iter()
                .find(|e| &e.id == ext_id)
                .map(|e| e.is_native && e.source_tool.as_deref() == Some(tool_id))
                .unwrap_or(false);
        let outcome = if is_native_item {
            ensure_native_present(tool_id, name, &mut result.restored_native)
        } else {
            match kind.as_str() {
                "skill" => services::enable_skill_for_tool(name, tool_id),
                "mcp" => services::toggle_mcp(name, tool_id, true),
                "plugin" => {
                    let plugin_kind = extensions
                        .iter()
                        .find(|e| &e.id == ext_id)
                        .and_then(|e| e.tags.clone())
                        .unwrap_or_else(|| "file".to_string());
                    crate::services::plugin::toggle_plugin(name, tool_id, true, &plugin_kind)
                }
                _ => Err(format!("未知类型: {}", kind)),
            }
        };
        match outcome {
            Ok(()) => result.success += 1,
            Err(e) => result.failures.push(format!("{}: {}", ext_id, e)),
        }
    }

    // 5) 记激活 + 历史
    database::set_active_preset(tool_id, preset_id)?;
    let _ = database::record_preset_application(preset_id, tool_id, true);
    info!(
        "预设组 {} → {}（独占）— 成功 {} 停用 {} 暂存 {} 失败 {} 冲突 {}",
        preset_id, tool_id, result.success, result.disabled.len(),
        result.stashed.len(), result.failures.len(), result.conflicts.len()
    );
    Ok(result)
}

/// 原生技能项的「确保在场」：目录在 → Ok；在暂存区 → 接回；都没有 → 失败
fn ensure_native_present(
    tool_id: &str,
    name: &str,
    restored_native: &mut Vec<String>,
) -> Result<(), String> {
    let dir = crate::adapter::primary_skill_dir(tool_id)
        .ok_or_else(|| format!("工具 {} 无 skill 目录", tool_id))?;
    if dir.join(name).exists() {
        return Ok(());
    }
    let entry = database::unrestored_stash(Some(tool_id))
        .into_iter()
        .find(|e| e.skill_name == name)
        .ok_or_else(|| format!("原生技能 {} 既不在工具目录也不在暂存区", name))?;
    stash::restore_stashed_skill(&entry)?;
    restored_native.push(name.to_string());
    Ok(())
}

/// 恢复默认（关，spec §5.2）：对齐基底 → 销毁快照。幂等：无快照返回空结果
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restored_mam: Vec<String>,
    pub restored_native: Vec<String>,
    pub conflicts: Vec<String>,
}

pub fn restore_tool(tool_id: &str) -> Result<RestoreResult, String> {
    let mut out = RestoreResult::default();
    let Some((active_preset, base_items)) = database::get_base_snapshot(tool_id) else {
        return Ok(out); // 无激活预设：幂等空操作
    };

    // 1) 暂存回移（冲突不覆盖，留待人工处理）
    let (restored_native, conflicts) = stash::restore_all_for_tool(tool_id);
    out.restored_native = restored_native;
    out.conflicts = conflicts;

    // 2) MAM 资源精确重建到基底集合（spec §5.2 步骤3）：
    //    快照 mam 集 = 目标；当前 enabled 集 = 现状；差异双向处理
    let target: Vec<(String, String)> = base_items
        .iter()
        .filter(|i| i.origin == "mam")
        .map(|i| (i.extension_id.clone(), i.kind.clone()))
        .collect();
    let current: Vec<(String, String)> = database::list_assignments(tool_id)
        .into_iter()
        // 只看工具级行：子 Agent 行由下面的重建段处理（与 scan_tool_state 同口径）
        .filter(|a| a.enabled && a.sub_agent_id.is_none())
        .map(|a| {
            let kind = if a.extension_id.starts_with("skill-") {
                "skill"
            } else if a.extension_id.starts_with("mcp-") {
                "mcp"
            } else {
                "plugin"
            };
            (a.extension_id, kind.to_string())
        })
        .collect();

    let extensions = database::list_extensions();
    let enable_one = |ext_id: &str, kind: &str| -> Result<(), String> {
        let name = ext_id.strip_prefix(&format!("{}-", kind)).unwrap_or(ext_id);
        match kind {
            "skill" => services::enable_skill_for_tool(name, tool_id),
            "mcp" => services::toggle_mcp(name, tool_id, true),
            "plugin" => {
                let plugin_kind = extensions
                    .iter()
                    .find(|e| e.id == ext_id)
                    .and_then(|e| e.tags.clone())
                    .unwrap_or_else(|| "file".to_string());
                crate::services::plugin::toggle_plugin(name, tool_id, true, &plugin_kind)
            }
            _ => Ok(()),
        }
    };
    let disable_one = |ext_id: &str, kind: &str| -> Result<(), String> {
        let name = ext_id.strip_prefix(&format!("{}-", kind)).unwrap_or(ext_id);
        match kind {
            "skill" => services::disable_skill_for_tool(name, tool_id),
            "mcp" => services::toggle_mcp(name, tool_id, false),
            "plugin" => {
                let plugin_kind = extensions
                    .iter()
                    .find(|e| e.id == ext_id)
                    .and_then(|e| e.tags.clone())
                    .unwrap_or_else(|| "file".to_string());
                crate::services::plugin::toggle_plugin(name, tool_id, false, &plugin_kind)
            }
            _ => Ok(()),
        }
    };

    for (id, kind) in &target {
        if !current.iter().any(|(cid, _)| cid == id) {
            match enable_one(id, kind) {
                Ok(()) => out.restored_mam.push(id.clone()),
                Err(e) => out.conflicts.push(format!("{}: 重建失败 {}", id, e)),
            }
        }
    }
    for (id, kind) in &current {
        if !target.iter().any(|(tid, _)| tid == id) {
            if let Err(e) = disable_one(id, kind) {
                out.conflicts.push(format!("{}: 停用失败 {}", id, e));
            }
        }
    }

    // 2.5) 子 Agent 链接重建：清扫的工具级禁用会级联断 Layer3（cleanup_layer3_on_tool_disable），
    //      工具级恢复后对「基底内且仍 enabled」的子 Agent 分配行重建链接——与 W5
    //      rebuild_tool_links 同思路（先工具级后子 Agent）；幂等，链在则替换。
    //      基底外技能的子 Agent 行不重建（其工具级已被本流程停用）
    for a in database::list_assignments(tool_id) {
        if !a.enabled || a.sub_agent_id.is_none() {
            continue;
        }
        if !target.iter().any(|(tid, _)| tid == &a.extension_id) {
            continue;
        }
        if let Some(name) = a.extension_id.strip_prefix("skill-") {
            let sub = a.sub_agent_id.clone().unwrap_or_default();
            if let Err(e) = services::assign_skill_to_subagent(name, tool_id, &sub) {
                out.conflicts.push(format!("{}#{}: 子 Agent 链接重建失败 {}", a.extension_id, sub, e));
            }
        }
    }

    // 3) 销毁快照（会话结束）+ 历史置 inactive
    database::destroy_base_snapshot(tool_id)?;
    if let Some(p) = active_preset {
        let _ = database::record_preset_application(&p, tool_id, false);
    }
    info!("工具 {} 已恢复默认（基底对齐完成）", tool_id);
    Ok(out)
}

/// 兼容委托（M2 移除）：旧命令入口校验激活中再走 restore_tool
pub fn deactivate_preset(preset_id: &str, tool_id: &str) -> Result<(), String> {
    match database::get_base_snapshot(tool_id) {
        Some((Some(active), _)) if active != preset_id => {
            return Err(format!("工具 {} 当前激活的是其他预设", tool_id));
        }
        _ => {}
    }
    restore_tool(tool_id).map(|_| ())
}

/// 应用预设组到子 Agent
pub fn apply_preset_to_subagent(preset_id: &str, tool_id: &str, sub_agent_id: &str) -> ApplyResult {
    let items = database::get_preset_items(preset_id);
    let mut success = 0;
    let mut failures = Vec::new();
    let mut conflicts = Vec::new();

    for (ext_id, kind) in &items {
        // 子 Agent 级只支持 skill（MCP 和 Plugin 是工具级配置）
        if kind != "skill" {
            conflicts.push(format!("{} 类型 {} 不支持子 Agent 级分配", ext_id, kind));
            continue;
        }

        // 检查是否在工具级范围内
        let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
        if !crate::services::is_skill_in_tool_range(name, tool_id) {
            failures.push(format!("{}: 该 skill 未在工具级启用", ext_id));
            continue;
        }

        let result = crate::services::assign_skill_to_subagent(name, tool_id, sub_agent_id);
        match result {
            Ok(()) => success += 1,
            Err(e) => failures.push(format!("{}: {}", ext_id, e)),
        }
    }

    let _ = database::record_preset_application_subagent(preset_id, tool_id, sub_agent_id, true);
    info!(
        "预设组 {} -> {}:{} -- 成功 {} 失败 {} 冲突 {}",
        preset_id,
        tool_id,
        sub_agent_id,
        success,
        failures.len(),
        conflicts.len()
    );
    ApplyResult {
        success,
        failures,
        conflicts,
        ..Default::default()
    }
}

/// 兼容性检查结果
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub compatible: Vec<CompatibleItem>,
    pub incompatible: Vec<IncompatibleItem>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibleItem {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncompatibleItem {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub reason: String,
}

/// 检查预设组与工具的兼容性
pub fn check_compatibility(preset_id: &str, tool_id: &str) -> CompatibilityReport {
    let items = database::get_preset_items(preset_id);
    let mut compatible = Vec::new();
    let mut incompatible = Vec::new();

    for (ext_id, kind) in items {
        // 获取资源信息
        let ext = database::list_extensions()
            .into_iter()
            .find(|e| e.id == ext_id);
        let name = ext
            .as_ref()
            .map(|e| e.name.clone())
            .unwrap_or_else(|| ext_id.clone());

        // 兼容判定（spec §6）：真值源是 resource_bindings（手动标记），
        // extensions.tags 不再参与（其语义是来源工具/插件子类型，历史误用）
        let is_compatible = crate::database::tool_allowed(&ext_id, tool_id);

        if is_compatible {
            compatible.push(CompatibleItem {
                id: ext_id.clone(),
                name,
                kind,
            });
        } else {
            let bound = crate::database::get_resource_binding(&ext_id)
                .map(|b| b.exclusive_tools)
                .unwrap_or_default();
            incompatible.push(IncompatibleItem {
                id: ext_id,
                name,
                kind,
                reason: format!("专属 {}，不支持 {}", bound, tool_id),
            });
        }
    }

    CompatibilityReport {
        compatible,
        incompatible,
    }
}
pub fn deactivate_preset_from_subagent(
    preset_id: &str,
    tool_id: &str,
    sub_agent_id: &str,
) -> Result<(), String> {
    let items = database::get_preset_items(preset_id);
    let mut errors = Vec::new();
    for (ext_id, kind) in &items {
        if kind != "skill" {
            continue;
        }
        let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
        let result = crate::linker::layer3::unlink_skill_from_layer3(name, tool_id, sub_agent_id);
        if let Err(e) = result {
            errors.push(format!("{}: {}", ext_id, e));
        }
        // 更新数据库记录
        let _ = crate::database::disable_subagent_assignment(ext_id, tool_id, sub_agent_id);
    }
    database::record_preset_application_subagent(preset_id, tool_id, sub_agent_id, false)?;
    if !errors.is_empty() {
        log::warn!("deactivate_preset_from_subagent 部分失败: {:?}", errors);
    }
    info!(
        "预设组 {} 从 {}:{} 取消激活",
        preset_id, tool_id, sub_agent_id
    );
    Ok(())
}
