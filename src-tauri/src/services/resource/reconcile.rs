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
/// 只看 skill（mcp/plugin 后续版本）；L4 外链只报告不接管。
///
/// 派发拍平对账口径（用户裁决 2026-09-17）：磁盘派发链接名一律是拍平名
/// （套件-技能名），账本侧比较时**正向映射**到拍平名再比（磁盘侧不做歧义
/// 反解）；报告的 extension_id 尽量保持账本嵌套规范名——L1/L2 由账本侧驱动
/// 天然保持原名；L3（磁盘有账本无）的拍平名无法无歧义反解回嵌套规范名
/// （多个嵌套名可拍平到同名，且账本无行即无映射源），extension_id 按磁盘
/// 拍平名报告，path 保留磁盘实况——此为该裁决的已知不可反解边界
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
        // 账本正向映射：嵌套规范名 → 拍平派发名（平铺名恒等）；
        // flat_to_ledger 供 L2 命中时回填账本嵌套规范名（报告身份保持嵌套名）
        let flat_of = |ext_id: &str| -> String {
            let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
            format!("skill-{}", crate::linker::dispatch_name(name))
        };
        let ledger_flat: std::collections::HashSet<String> =
            ledger.iter().map(|id| flat_of(id)).collect();
        let flat_to_ledger: std::collections::HashMap<String, String> =
            ledger.iter().map(|id| (flat_of(id), id.clone())).collect();
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
                    } else if path.is_dir() && ledger_flat.contains(&ext_id) {
                        // 磁盘真目录名与账本拍平名吻合 → L2；报告身份回填账本嵌套规范名
                        out.push(DriftItem {
                            tool_id: tool.to_string(),
                            kind: "L2".into(),
                            extension_id: flat_to_ledger.get(&ext_id).cloned().unwrap_or(ext_id),
                            path: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
        for ext_id in &ledger {
            // 磁盘 MAM 链接集合按拍平名比较（正向映射，不反解磁盘名）
            if !disk_mam_links.contains(&flat_of(ext_id)) {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                // 判定与 path 均按拍平名（磁盘派发落点实况）；extension_id 保持账本嵌套规范名
                if !crate::linker::dispatch_target(&dir, name).exists() {
                    out.push(DriftItem {
                        tool_id: tool.to_string(),
                        kind: "L1".into(),
                        extension_id: ext_id.clone(),
                        path: crate::linker::dispatch_target(&dir, name)
                            .to_string_lossy()
                            .to_string(),
                    });
                }
            }
        }
        for ext_id in &disk_mam_links {
            if !ledger_flat.contains(ext_id) {
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
    /// 漂移条目定位（终审 Minor #4 结构化契约）：前端批量行键映射用，
    /// 与 message 文本解耦——message 保留 "ext @ tool: detail" 仅供人读
    pub extension_id: String,
    pub tool_id: String,
    pub message: String,
}

/// 处置成功
fn fixed_do(item: &DriftItem, message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: true,
        needs_manual: false,
        extension_id: item.extension_id.clone(),
        tool_id: item.tool_id.clone(),
        message,
    }
}

/// 处置失败（普通失败，不升级人工）
fn failed(item: &DriftItem, message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: false,
        needs_manual: false,
        extension_id: item.extension_id.clone(),
        tool_id: item.tool_id.clone(),
        message,
    }
}

/// 升级人工（MAM 不动手 / 绝不删除现场）
fn needs_manual(item: &DriftItem, message: String) -> ReconcileOutcome {
    ReconcileOutcome {
        fixed: false,
        needs_manual: true,
        extension_id: item.extension_id.clone(),
        tool_id: item.tool_id.clone(),
        message,
    }
}

fn from_result(res: Result<(), String>, item: &DriftItem, ok_message: String) -> ReconcileOutcome {
    match res {
        Ok(()) => fixed_do(item, ok_message),
        Err(e) => failed(item, format!("处置失败: {}", e)),
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
            item,
            format!("{}: 已按账本重建链接", where_at),
        ),
        ("L1", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, false, "missing"),
            item,
            format!("{}: 已按磁盘回写账本 disabled/missing", where_at),
        ),
        ("L2", "a") => match crate::services::skill::enable_skill_for_tool(name, &item.tool_id) {
            Ok(()) => fixed_do(item, format!("{}: 真目录与共享仓库一致，已替换为链接", where_at)),
            // M1 真目录守卫（「不一致」稳定标记）：内容不一致 → 需人工处理，绝不删除现场
            Err(e) if e.contains("不一致") => needs_manual(
                item,
                format!(
                    "{}: 磁盘真目录与共享仓库内容不一致，需人工处理；MAM 未做任何改动，现场已保留（{}）",
                    where_at, e
                ),
            ),
            Err(e) => failed(item, format!("处置失败: {}", e)),
        },
        ("L2", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, false, "missing"),
            item,
            format!(
                "{}: 已按磁盘回写账本 disabled/missing（真目录保留原生态）",
                where_at
            ),
        ),
        ("L3", "a") => from_result(
            crate::services::skill::disable_skill_for_tool(name, &item.tool_id),
            item,
            format!("{}: 已按账本清除链接（含 Layer 2 级联）", where_at),
        ),
        ("L3", "b") => from_result(
            crate::database::upsert_assignment(&item.extension_id, &item.tool_id, true, "valid"),
            item,
            format!("{}: 已按磁盘回写账本 enabled/valid", where_at),
        ),
        ("L4", _) => needs_manual(
            item,
            format!(
                "{}: 外链不归 MAM 接管，需人工确认归属或手动处理",
                where_at
            ),
        ),
        _ => failed(
            item,
            format!("未知组合: kind={}, mode={}", item.kind, mode),
        ),
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
