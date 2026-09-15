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
