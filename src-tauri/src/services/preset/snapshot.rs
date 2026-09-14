// 状态扫描与基底拍快照（spec §3.2/§5.1 步骤3）：拍快照先于任何清扫动作
use crate::database::{self, BaseSnapshotItemRecord};

/// 扫描工具当前完整资源状态：MAM 启用项（assignment）+ 原生真目录（skill 目录里的非链接目录）
pub fn scan_tool_state(tool_id: &str) -> Vec<BaseSnapshotItemRecord> {
    let mut items = Vec::new();

    // 1) MAM 管理项：enabled assignment（id 前缀即 kind，全库约定）。
    //    只记工具级行——list_assignments 返回含子 Agent 行（同一 ext_id 两行：
    //    工具级 + sub_agent 级，见 dao/extension.rs:71-91），不过滤会对快照
    //    PK (tool_id, extension_id) 二次插入报 UNIQUE 冲突；子 Agent 链接由
    //    工具级启停级联清理，恢复时单独重建（restore_tool 的子 Agent 重建段）
    for a in database::list_assignments(tool_id) {
        if !a.enabled || a.sub_agent_id.is_some() {
            continue;
        }
        let kind = if a.extension_id.starts_with("skill-") {
            "skill"
        } else if a.extension_id.starts_with("mcp-") {
            "mcp"
        } else if a.extension_id.starts_with("plugin-") {
            "plugin"
        } else {
            continue;
        };
        items.push(BaseSnapshotItemRecord {
            extension_id: a.extension_id,
            kind: kind.to_string(),
            origin: "mam".to_string(),
        });
    }

    // 2) 原生技能：主 skill 目录下的真目录（非符号链接）。MAM 启用项在目录里是链接，
    //    与真目录天然不重叠；链接穿透套件（父目录是链接）不在此层出现
    if let Some(dir) = crate::adapter::primary_skill_dir(tool_id) {
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    // 只认真目录：符号链接是 MAM（或用户外链），不暂存
                    if path.is_dir() && !path.is_symlink() {
                        if let Some(name) = e.file_name().to_str() {
                            // 子 Agent 布局目录（subagents/）不是技能，跳过
                            if name == "subagents" {
                                continue;
                            }
                            items.push(BaseSnapshotItemRecord {
                                extension_id: format!("skill-{}", name),
                                kind: "skill".to_string(),
                                origin: "native".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    items
}

/// 拍基底快照（active=None；设激活由应用流程负责）
pub fn capture_base_snapshot(tool_id: &str) -> Result<(), String> {
    let items = scan_tool_state(tool_id);
    database::save_base_snapshot(tool_id, None, &items)
}
