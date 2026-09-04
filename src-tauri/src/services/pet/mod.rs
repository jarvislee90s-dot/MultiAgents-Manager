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

/// 重命名宠物 = 目录重命名 + manifest.id 同步（备份旧 manifest，spec §10-1）。
/// 顺序：先 rename 目录再写 manifest——rename 失败零副作用。
/// manifest 写失败为非致命：manifest.id 仅是展示字段，宠物身份以文件夹名为准
/// （校验/匹配/激活均用文件夹名），目录已改名即主操作已完成；返回 Err 会让 UI 把
/// 已成功的改名报成失败，且用户重试会命中"宠物不存在: old_id"的死路。
pub fn rename_pet_in(root: &Path, old_id: &str, new_id: &str) -> Result<(), String> {
    if old_id == new_id {
        return Ok(());
    }
    let old_dir = pet_dir(root, old_id);
    if !old_dir.is_dir() {
        return Err(format!("宠物不存在: {}", old_id));
    }
    import::validate_pet_name(root, new_id)?;
    let new_dir = pet_dir(root, new_id);
    std::fs::rename(&old_dir, &new_dir).map_err(|e| format!("重命名失败: {}", e))?;
    if let Some(mut m) = manifest::load(&new_dir) {
        m.id = new_id.to_string();
        if let Err(e) = manifest::write_with_backup(&new_dir, &m, true) {
            // 目录已改名（主操作完成），仅 id 展示字段未同步：记日志、不判失败（下次激活/修复会兜底）
            log::warn!("manifest.id 同步失败（目录已改名 {} → {}）: {}", old_id, new_id, e);
        }
    }
    Ok(())
}

/// 删除宠物：整目录移入回收站（spec §10；trash crate 已是项目依赖）
pub fn delete_pet_in(root: &Path, id: &str) -> Result<(), String> {
    let dir = pet_dir(root, id);
    if !dir.is_dir() {
        return Err(format!("宠物不存在: {}", id));
    }
    trash::delete(&dir).map_err(|e| format!("删除失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkpet(root: &std::path::Path, id: &str) {
        let dir = pet_dir(root, id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spritesheet.webp"), b"s").unwrap();
    }

    #[test]
    fn rename_updates_dir_and_manifest_id() {
        let root = tempfile::tempdir().unwrap();
        mkpet(root.path(), "old-name");
        let m = manifest::PetManifest {
            schema_version: 1,
            id: "old-name".into(),
            display_name: "D".into(),
            description: String::new(),
            source: "folder".into(),
            sprite_version_number: 1,
            spritesheet_size_bytes: 1,
            has_voice: false,
            has_subtitle: false,
            voices: vec![],
        };
        manifest::write_with_backup(&pet_dir(root.path(), "old-name"), &m, false).unwrap();
        rename_pet_in(root.path(), "old-name", "new-name").unwrap();
        assert!(pet_dir(root.path(), "new-name").is_dir());
        assert!(!pet_dir(root.path(), "old-name").exists());
        let m2 = manifest::load(&pet_dir(root.path(), "new-name")).unwrap();
        assert_eq!(m2.id, "new-name");
        // 备份存在且记录旧 id
        assert!(pet_dir(root.path(), "new-name").join(manifest::BACKUP_FILE).is_file());
    }

    #[test]
    fn rename_conflict_errs() {
        let root = tempfile::tempdir().unwrap();
        mkpet(root.path(), "a");
        mkpet(root.path(), "b");
        assert!(rename_pet_in(root.path(), "a", "b").is_err());
        // 重命名为自身是 no-op
        assert!(rename_pet_in(root.path(), "a", "a").is_ok());
        // FIX-5：rename 失败（目标已存在）时旧目录 manifest 未被改写（零副作用）
        let old_manifest = manifest::load(&pet_dir(root.path(), "a")).unwrap_or_else(|| {
            let m = manifest::PetManifest {
                schema_version: 1,
                id: "a".into(),
                display_name: "A".into(),
                description: String::new(),
                source: "folder".into(),
                sprite_version_number: 1,
                spritesheet_size_bytes: 1,
                has_voice: false,
                has_subtitle: false,
                voices: vec![],
            };
            manifest::write_with_backup(&pet_dir(root.path(), "a"), &m, false).unwrap();
            m
        });
        assert!(rename_pet_in(root.path(), "a", "b").is_err());
        let after = manifest::load(&pet_dir(root.path(), "a")).unwrap();
        assert_eq!(after.id, old_manifest.id, "rename 失败不应改写旧 manifest");
        assert_eq!(after.id, "a");
    }
}