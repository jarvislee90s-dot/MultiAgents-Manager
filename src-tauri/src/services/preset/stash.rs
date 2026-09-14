// 暂存区引擎（spec §3.1/§5.3）：原生技能目录整体移动（同盘 rename，零拷贝），
// 非压缩包；stash_journal 是唯一账本，崩溃后凭账本恢复
use std::path::Path;

use crate::database::{self, StashEntryRecord};

/// 暂存区：~/.mam/stash/<tool>/skills
pub fn stash_dir(tool_id: &str) -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".mam")
        .join("stash")
        .join(tool_id)
        .join("skills")
}

/// 把原生技能目录移入暂存区。先移动后记账；记账失败回滚移动（不留无账暂存）
pub fn stash_native_skill(
    tool_id: &str,
    skill_name: &str,
    original_path: &Path,
) -> Result<(), String> {
    if !original_path.is_dir() {
        return Err(format!("原生技能目录不存在: {}", original_path.display()));
    }
    let dir = stash_dir(tool_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stashed = dir.join(skill_name);
    if stashed.exists() {
        return Err(format!("暂存区已有同名项: {}", stashed.display()));
    }
    std::fs::rename(original_path, &stashed).map_err(|e| e.to_string())?;
    if let Err(e) = database::record_stash(
        tool_id,
        skill_name,
        &stashed.to_string_lossy(),
        &original_path.to_string_lossy(),
    ) {
        // 回滚移动，宁可不暂存也不留无账目录
        let _ = std::fs::rename(&stashed, original_path);
        return Err(format!("暂存记账失败已回滚: {}", e));
    }
    log::info!("原生技能 {} 已暂存: {}", skill_name, stashed.display());
    Ok(())
}

/// 回移单条。原位被占（同名且存在）→ Err（不覆盖，调用方留待人工处理）
pub fn restore_stashed_skill(entry: &StashEntryRecord) -> Result<(), String> {
    let stashed = Path::new(&entry.stashed_path);
    let original = Path::new(&entry.original_path);
    if !stashed.exists() {
        return Err(format!("暂存区已无此项: {}", stashed.display()));
    }
    if original.exists() {
        return Err(format!("原位已被占用: {}", original.display()));
    }
    if let Some(parent) = original.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(stashed, original).map_err(|e| e.to_string())?;
    database::mark_stash_restored(entry.id)?;
    log::info!(
        "暂存技能 {} 已回移 {}",
        entry.skill_name,
        original.display()
    );
    Ok(())
}

/// 回移该工具全部未恢复项；冲突项不覆盖、留在暂存区并报 "name: 原因"
pub fn restore_all_for_tool(tool_id: &str) -> (Vec<String>, Vec<String>) {
    let mut restored = Vec::new();
    let mut conflicts = Vec::new();
    for entry in database::unrestored_stash(Some(tool_id)) {
        match restore_stashed_skill(&entry) {
            Ok(()) => restored.push(entry.skill_name),
            Err(e) => conflicts.push(format!("{}: {}", entry.skill_name, e)),
        }
    }
    (restored, conflicts)
}

/// 启动孤儿恢复（spec §5.3）：全工具扫未恢复账目，暂存文件在且原位空 → 回移；
/// 返回成功恢复条数。冲突项保持暂存并 log（M2 UI 列出人工处理）
pub fn recover_orphans() -> usize {
    let mut n = 0;
    for entry in database::unrestored_stash(None) {
        match restore_stashed_skill(&entry) {
            Ok(()) => n += 1,
            Err(e) => log::warn!("孤儿暂存 {} 未恢复: {}", entry.skill_name, e),
        }
    }
    n
}
