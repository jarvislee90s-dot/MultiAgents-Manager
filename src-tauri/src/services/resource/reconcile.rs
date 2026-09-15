// 账本-磁盘对账扫描引擎（spec §13 第一块）

/// 单条账本-磁盘漂移
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftItem {
    pub tool_id: String,
    pub kind: String, // "L1"|"L2"|"L3"|"L4"
    pub extension_id: String,
    pub path: String,
}

/// 扫描 enabled 工具的账本-磁盘漂移（§13）：disabled 工具排除（W5 名册语义）；
/// 只看 skill（mcp/plugin 后续版本）；L4 外链只报告不接管
pub fn scan_drift() -> Vec<DriftItem> {
    let home = dirs::home_dir().unwrap_or_default();
    let mam_root = home.join(".mam");
    let mut out = Vec::new();
    for tool in crate::adapter::TOOL_IDS {
        if !crate::database::get_tool_enabled(tool) {
            continue; // W5 停用 = 名册保留语义，不算漂移
        }
        let ledger: Vec<String> = crate::database::list_assignments(tool)
            .into_iter()
            .filter(|a| {
                a.enabled && a.sub_agent_id.is_none() && a.extension_id.starts_with("skill-")
            })
            .map(|a| a.extension_id)
            .collect();
        let Some(dir) = crate::adapter::primary_skill_dir(tool) else {
            continue;
        };
        let mut disk_mam_links: Vec<String> = Vec::new();
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    // 编译器强制调整：file_name() 返回 owned OsString，须先绑定延长生命周期
                    let file_name = e.file_name();
                    let Some(name) = file_name.to_str() else {
                        continue;
                    };
                    if name == "subagents" {
                        continue;
                    }
                    let ext_id = format!("skill-{}", name);
                    if path.is_symlink() {
                        let target = std::fs::read_link(&path).unwrap_or_default();
                        if target.starts_with(&mam_root) {
                            disk_mam_links.push(ext_id);
                        } else {
                            out.push(DriftItem {
                                tool_id: tool.to_string(),
                                kind: "L4".into(),
                                extension_id: ext_id,
                                path: path.to_string_lossy().to_string(),
                            });
                        }
                    } else if path.is_dir() && ledger.iter().any(|l| l == &ext_id) {
                        out.push(DriftItem {
                            tool_id: tool.to_string(),
                            kind: "L2".into(),
                            extension_id: ext_id,
                            path: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
        for ext_id in &ledger {
            if !disk_mam_links.contains(ext_id) {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                if !dir.join(name).exists() {
                    out.push(DriftItem {
                        tool_id: tool.to_string(),
                        kind: "L1".into(),
                        extension_id: ext_id.clone(),
                        path: dir.join(name).to_string_lossy().to_string(),
                    });
                }
            }
        }
        for ext_id in &disk_mam_links {
            if !ledger.contains(ext_id) {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                out.push(DriftItem {
                    tool_id: tool.to_string(),
                    kind: "L3".into(),
                    extension_id: ext_id.clone(),
                    path: dir.join(name).to_string_lossy().to_string(),
                });
            }
        }
    }
    out
}

/// 单条对账处置结果（spec §13）
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileOutcome {
    /// 是否已修复（账本-磁盘已一致；L1-b/L2-b/L3-b 回写账本也算 fixed）
    pub fixed: bool,
    /// 是否升级人工（L4 外链、L2-a 内容不一致守卫）
    pub needs_manual: bool,
    pub message: String,
}

/// 处置成功
fn fixed_do(message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: true,
        needs_manual: false,
        message,
    }
}

/// 处置失败（普通失败，不升级人工）
fn failed(message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: false,
        needs_manual: false,
        message,
    }
}

/// 升级人工（MAM 不动手 / 绝不删除现场）
fn needs_manual(message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: false,
        needs_manual: true,
        message,
    }
}

fn from_result(res: Result<(), String>, ok_message: String) -> ReconcileOutcome {
    match res {
        Ok(()) => fixed_do(ok_message),
        Err(e) => failed(format!("处置失败: {}", e)),
    }
}

/// 单条对账处置（spec §13）。mode: "a" 账本为准修磁盘 | "b" 磁盘为准回写账本
///
/// - L1-a 重建链接 / L1-b 回写 disabled("missing")
/// - L2-a enable（真目录内容与 SSOT 一致才替换；Err 含「不一致」→ needs_manual 且绝不删除现场）
///   / L2-b 回写 disabled("missing")（真目录保留原生态）
/// - L3-a disable（清链含 Layer2 级联）/ L3-b 回写 enabled("valid")
/// - L4 任意 mode → needs_manual（外链 MAM 不接管，现场不动）
pub fn reconcile_one(item: &DriftItem, mode: &str) -> ReconcileOutcome {
    let name = item
        .extension_id
        .strip_prefix("skill-")
        .unwrap_or(&item.extension_id);
    let where_at = format!("{} @ {}", item.extension_id, item.tool_id);
    match (item.kind.as_str(), mode) {
        ("L1", "a") => from_result(
            crate::services::skill::enable_skill_for_tool(name, &item.tool_id),
            format!("{}: 已按账本重建链接", where_at),
        ),
        ("L1", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, false, "missing"),
            format!("{}: 已按磁盘回写账本 disabled/missing", where_at),
        ),
        ("L2", "a") => match crate::services::skill::enable_skill_for_tool(name, &item.tool_id) {
            Ok(()) => fixed_do(format!("{}: 真目录与共享仓库一致，已替换为链接", where_at)),
            // M1 真目录守卫（「不一致」稳定标记）：内容不一致 → 需人工处理，绝不删除现场
            Err(e) if e.contains("不一致") => needs_manual(format!(
                "{}: 磁盘真目录与共享仓库内容不一致，需人工处理；MAM 未做任何改动，现场已保留（{}）",
                where_at, e
            )),
            Err(e) => failed(format!("处置失败: {}", e)),
        },
        ("L2", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, false, "missing"),
            format!(
                "{}: 已按磁盘回写账本 disabled/missing（真目录保留原生态）",
                where_at
            ),
        ),
        ("L3", "a") => from_result(
            crate::services::skill::disable_skill_for_tool(name, &item.tool_id),
            format!("{}: 已按账本清除链接（含 Layer 2 级联）", where_at),
        ),
        ("L3", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, true, "valid"),
            format!("{}: 已按磁盘回写账本 enabled/valid", where_at),
        ),
        ("L4", _) => needs_manual(format!(
            "{}: 外链不归 MAM 接管，需人工确认归属或手动处理",
            where_at
        )),
        _ => failed(format!("未知组合: kind={}, mode={}", item.kind, mode)),
    }
}

/// 批量对账处置：该工具全部漂移逐条按同一 mode 处置（spec §13）。
/// 逐条独立，单条失败不中断——collect 全部 outcome 返回
pub fn reconcile_tool_batch(tool_id: &str, mode: &str) -> Vec<ReconcileOutcome> {
    scan_drift()
        .into_iter()
        .filter(|item| item.tool_id == tool_id)
        .map(|item| reconcile_one(&item, mode))
        .collect()
}
