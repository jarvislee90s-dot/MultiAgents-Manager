// 外部宠物服务 — 仓库路径与子模块入口（spec §4/§17）
pub mod import;
pub mod manifest;
pub mod petdex;
pub mod scan;

use std::path::{Path, PathBuf};

/// 宠物仓库根目录 ~/.mam/pets
pub fn pets_root() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mam").join("pets")
}

/// 指定宠物的目录
pub fn pet_dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

/// 导入暂存区根目录 ~/.mam/pets/.import-staging（隐藏目录，清单扫描自动跳过）
pub fn staging_root(root: &Path) -> PathBuf {
    root.join(".import-staging")
}

/// 重命名宠物 = 目录重命名 + manifest.id 同步（备份旧 manifest，spec §10-1）
pub fn rename_pet_in(root: &Path, old_id: &str, new_id: &str) -> Result<(), String> {
    if old_id == new_id {
        return Ok(());
    }
    let old_dir = pet_dir(root, old_id);
    if !old_dir.is_dir() {
        return Err(format!("宠物不存在: {}", old_id));
    }
    import::validate_pet_name(root, new_id)?;
    if let Some(mut m) = manifest::load(&old_dir) {
        m.id = new_id.to_string();
        manifest::write_with_backup(&old_dir, &m, true)?;
    }
    std::fs::rename(&old_dir, pet_dir(root, new_id)).map_err(|e| format!("重命名失败: {}", e))
}

/// 删除宠物：整目录移入回收站（spec §10；trash crate 已是项目依赖）
pub fn delete_pet_in(root: &Path, id: &str) -> Result<(), String> {
    let dir = pet_dir(root, id);
    if !dir.is_dir() {
        return Err(format!("宠物不存在: {}", id));
    }
    trash::delete(&dir).map_err(|e| format!("删除失败: {}", e))
}