// 导入 — 暂存区、来源落地（文件夹/zip/codex）、音频暂存、finalize 原子落地（spec §8/§13）
use super::{manifest, pet_dir, scan, staging_root};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const SHEET_FILE: &str = "spritesheet.webp";
pub const MAX_ZIP_TOTAL_BYTES: u64 = 100 * 1024 * 1024;
pub const MAX_ZIP_FILES: usize = 200;
/// 允许的音频扩展名（spec §5.1）
pub const AUDIO_EXTS: [&str; 7] = ["m4a", "mp3", "wav", "ogg", "opus", "flac", "aac"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedVoiceFile {
    pub group: String,
    pub name: String,
    pub file: String, // voice/<group>/<文件名>
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedPet {
    pub staging_id: String,
    pub dir: String,
    pub suggested_name: String,
    pub suggested_display_name: String,
    /// codex pet.json 透传；0=未知（前端图集探测后回填 manifest，spec §4.2）
    pub sprite_version_number: u8,
    pub spritesheet_size: u64,
    pub voice_files: Vec<StagedVoiceFile>,
}

/// 简易唯一 id：时间戳 + 进程内计数（避免引入 uuid 依赖）
fn uid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn ext_lower(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

fn is_audio(p: &Path) -> bool {
    AUDIO_EXTS.contains(&ext_lower(p).as_str())
}

fn valid_group(g: &str) -> bool {
    manifest::VOICE_GROUPS.contains(&g)
}

fn new_staging(root: &Path) -> Result<PathBuf, String> {
    let dir = staging_root(root).join(uid());
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建暂存区失败: {}", e))?;
    Ok(dir)
}

/// 在根目录或一层子目录内定位 spritesheet.webp（根优先，spec §8.2）
pub fn locate_sheet(src: &Path) -> Option<PathBuf> {
    let direct = src.join(SHEET_FILE);
    if direct.is_file() {
        return Some(direct);
    }
    let Ok(rd) = std::fs::read_dir(src) else { return None };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && p.join(SHEET_FILE).is_file() {
            return Some(p.join(SHEET_FILE));
        }
    }
    None
}

/// 复制 voice/ 子树：仅四分组目录下的合法音频（spec §8.2 自动带入）
fn copy_voice_tree(base: &Path, dir: &Path, dest_base: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let p = entry.path();
        if p.is_dir() {
            copy_voice_tree(base, &p, dest_base)?;
            continue;
        }
        if !is_audio(&p) {
            continue;
        }
        let Ok(rel) = p.strip_prefix(base) else { continue };
        let dest = dest_base.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&p, &dest).map_err(|e| format!("复制音频失败: {}", e))?;
    }
    Ok(())
}

/// 暂存区内收集 voice 文件清单
fn list_staged_voice(staging: &Path) -> Vec<StagedVoiceFile> {
    let mut out = Vec::new();
    let mut stack = vec![staging.join("voice")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Ok(rel) = p.strip_prefix(staging) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let mut segs = rel.split('/');
            let (_v, group, file) = (segs.next(), segs.next().unwrap_or(""), segs.next().unwrap_or(""));
            if !valid_group(group) {
                continue;
            }
            out.push(StagedVoiceFile {
                group: group.to_string(),
                name: Path::new(file).file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string(),
                file: rel,
                size_bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexPetJson {
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    sprite_version_number: u8,
}

/// 读来源目录的 pet.json（codex 元数据透传；仅用于向导预填，spec §8.1）
fn codex_meta(dir: &Path) -> (String, u8) {
    let Ok(text) = std::fs::read_to_string(dir.join("pet.json")) else {
        return (String::new(), 0);
    };
    match serde_json::from_str::<CodexPetJson>(&text) {
        Ok(j) => (j.display_name, j.sprite_version_number),
        Err(_) => (String::new(), 0),
    }
}

/// 建议名合法化：非法字符折叠为 '-'（最终名称在 finalize 时严格校验）
fn sanitize_name(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

fn finish_staged(
    staging: &Path,
    suggested_name: String,
    suggested_display_name: String,
    sprite_version_number: u8,
) -> Result<StagedPet, String> {
    Ok(StagedPet {
        staging_id: staging.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string(),
        dir: staging.to_string_lossy().to_string(),
        suggested_name,
        suggested_display_name,
        sprite_version_number,
        spritesheet_size: std::fs::metadata(staging.join(SHEET_FILE)).map(|m| m.len()).unwrap_or(0),
        voice_files: list_staged_voice(staging),
    })
}

/// 文件夹来源暂存：定位图集 → 复制图集 + voice/ → 返回暂存描述（spec §8.2）
pub fn stage_from_folder_in(root: &Path, src: &Path) -> Result<StagedPet, String> {
    if !src.is_dir() {
        return Err("来源不是文件夹".into());
    }
    let sheet = locate_sheet(src).ok_or_else(|| "未找到 spritesheet.webp（根目录或一层子目录）".to_string())?;
    let sheet_root = sheet.parent().unwrap_or(src).to_path_buf();
    let staging = new_staging(root)?;
    let copy = (|| -> Result<(), String> {
        std::fs::copy(&sheet, staging.join(SHEET_FILE)).map_err(|e| format!("复制图集失败: {}", e))?;
        let voice_root = sheet_root.join("voice");
        if voice_root.is_dir() {
            copy_voice_tree(&voice_root, &voice_root, &staging.join("voice"))?;
        }
        Ok(())
    })();
    if let Err(e) = copy {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let (disp, ver) = codex_meta(&sheet_root);
    let suggested_name = sanitize_name(
        sheet_root.file_name().and_then(|n| n.to_str()).unwrap_or("pet"),
    );
    finish_staged(&staging, suggested_name, disp, ver)
}

/// 安全解压：enclosed_name 防 zip-slip + 文件数/总大小上限（spec §13）
pub fn safe_unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    safe_unzip_with_limit(zip_path, dest, MAX_ZIP_TOTAL_BYTES)
}

/// 上限的人类可读格式：整除 MiB → "NMB"，否则 "NB"（FIX-9）
fn fmt_limit(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes.is_multiple_of(MIB) {
        format!("{}MB", bytes / MIB)
    } else {
        format!("{}B", bytes)
    }
}

/// 解压内核：总大小上限参数化（便于小上限单测），按 io::copy 的实际字节数累计（FIX-5，
/// 原实现累加 entry.size() 是压缩前声明值，可被 zip 头谎报绕过）；单条目读取用 take
/// 限制在剩余额度内，防止谎报小尺寸的大文件把磁盘写爆
fn safe_unzip_with_limit(zip_path: &Path, dest: &Path, max_total: u64) -> Result<(), String> {
    let f = std::fs::File::open(zip_path).map_err(|e| format!("打开压缩包失败: {}", e))?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("读取压缩包失败: {}", e))?;
    if zip.len() > MAX_ZIP_FILES {
        return Err(format!("压缩包文件数超限（>{}）", MAX_ZIP_FILES));
    }
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name 已拒绝绝对路径与 .. 穿越；None 即非法条目
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("压缩包含非法路径条目: {}", entry.name()));
        };
        if entry.is_dir() {
            std::fs::create_dir_all(dest.join(rel)).map_err(|e| e.to_string())?;
            continue;
        }
        let out_path = dest.join(rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
        // take(剩余额度+1)：多读 1 字节用于区分"恰好填满"与"超限"
        let mut limited = std::io::Read::take(&mut entry, max_total - total + 1);
        let written = std::io::copy(&mut limited, &mut out).map_err(|e| e.to_string())?;
        total += written;
        if total > max_total {
            let _ = std::fs::remove_file(&out_path); // 超限即拒绝：不留半截文件
            return Err(format!("压缩包解压总量超限（>{}）", fmt_limit(max_total)));
        }
    }
    Ok(())
}

/// zip 来源暂存：解压到 staging 下的 extract 目录 → 复用文件夹管线 → 清理（spec §8.2）
pub fn stage_from_zip_in(root: &Path, zip_path: &Path) -> Result<StagedPet, String> {
    if !zip_path.is_file() {
        return Err("压缩包不存在".into());
    }
    let extract = staging_root(root).join(format!("extract-{}", uid()));
    if let Err(e) = safe_unzip(zip_path, &extract) {
        let _ = std::fs::remove_dir_all(&extract);
        return Err(e);
    }
    let staged = stage_from_folder_in(root, &extract);
    let _ = std::fs::remove_dir_all(&extract);
    staged
}

/// codex 来源暂存（spec §8.1）：仅取 spritesheet.webp（+ 自动带入 voice/ 若存在）
pub fn stage_from_codex_in(root: &Path, codex_root: &Path, codex_id: &str) -> Result<StagedPet, String> {
    let src = codex_root.join(codex_id);
    if !src.is_dir() {
        return Err(format!("codex 宠物不存在: {}", codex_id));
    }
    stage_from_folder_in(root, &src)
}

/// 单个音频复制进目标 voice/<group>/（暂存与正式目录共用）
fn copy_audio_into(dest_voice: &Path, src: &Path, group: &str) -> Result<StagedVoiceFile, String> {
    if !valid_group(group) {
        return Err(format!("非法分组: {}", group));
    }
    if !src.is_file() {
        return Err(format!("音频文件不存在: {}", src.display()));
    }
    if !is_audio(src) {
        return Err(format!("不支持的音频格式: {}", src.display()));
    }
    let name = src.file_stem().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
    let file_name = src.file_name().and_then(|n| n.to_str()).unwrap_or("audio").to_string();
    let dest = dest_voice.join(group).join(&file_name);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    std::fs::copy(src, &dest).map_err(|e| format!("复制音频失败: {}", e))?;
    Ok(StagedVoiceFile {
        group: group.to_string(),
        name,
        file: format!("voice/{}/{}", group, file_name),
        size_bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
    })
}

fn staging_dir(root: &Path, staging_id: &str) -> Result<PathBuf, String> {
    if staging_id.contains("..") || staging_id.contains('/') || staging_id.contains('\\') {
        return Err("非法暂存区 id".into());
    }
    let d = staging_root(root).join(staging_id);
    if !d.is_dir() {
        return Err("暂存区不存在".into());
    }
    Ok(d)
}

/// 向导音频暂存（spec §8.4-3）
pub fn stage_audio_in(root: &Path, staging_id: &str, src_paths: &[String], group: &str) -> Result<Vec<StagedVoiceFile>, String> {
    if !valid_group(group) {
        return Err(format!("非法分组: {}", group));
    }
    let staging = staging_dir(root, staging_id)?;
    let mut out = Vec::new();
    for p in src_paths {
        out.push(copy_audio_into(&staging.join("voice"), Path::new(p), group)?);
    }
    Ok(out)
}

/// 修改面板直接向正式目录添加音频（spec §10-3）
pub fn add_voice_files_in(root: &Path, pet_id: &str, src_paths: &[String], group: &str) -> Result<Vec<StagedVoiceFile>, String> {
    if !valid_group(group) {
        return Err(format!("非法分组: {}", group));
    }
    let dir = pet_dir(root, pet_id);
    if !dir.is_dir() {
        return Err(format!("宠物不存在: {}", pet_id));
    }
    let mut out = Vec::new();
    for p in src_paths {
        out.push(copy_audio_into(&dir.join("voice"), Path::new(p), group)?);
    }
    Ok(out)
}

/// 音频路径安全校验：必须形如 voice/<group>/<file> 且无穿越（staged=true 为暂存区）
pub fn remove_audio_in(root: &Path, base_id: &str, rel: &str, staged: bool) -> Result<(), String> {
    if !rel.starts_with("voice/") || rel.contains("..") || rel.contains('\\') {
        return Err("非法音频路径".into());
    }
    if Path::new(rel).components().count() != 3 {
        return Err("非法音频路径".into());
    }
    let base = if staged { staging_dir(root, base_id)? } else { pet_dir(root, base_id) };
    if !base.is_dir() {
        return Err("目录不存在".into());
    }
    let p = base.join(rel);
    if p.is_file() {
        std::fs::remove_file(&p).map_err(|e| format!("删除音频失败: {}", e))?;
    }
    Ok(())
}

/// 宠物名（= 文件夹名）严格校验（spec §8.4-1）
pub fn validate_pet_name(root: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("宠物名不能为空".into());
    }
    if name.starts_with('.') {
        return Err("宠物名不能以点开头".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("宠物名仅支持字母/数字/连字符/下划线".into());
    }
    if name.eq_ignore_ascii_case("foxbell") {
        return Err("foxbell 为内置宠物保留名".into());
    }
    if pet_dir(root, name).exists() {
        return Err(format!("宠物已存在: {}", name));
    }
    Ok(())
}

/// finalize：写 manifest（前端已探测 voices）→ 同盘 rename 原子落地（spec §8.4-5）
pub fn finalize_in(root: &Path, staging_id: &str, name: &str, mut m: manifest::PetManifest) -> Result<scan::PetSummary, String> {
    validate_pet_name(root, name)?;
    let staging = staging_dir(root, staging_id)?;
    if !staging.join(SHEET_FILE).is_file() {
        return Err("暂存区缺少 spritesheet.webp".into());
    }
    m.schema_version = manifest::SCHEMA_VERSION;
    m.id = name.to_string();
    manifest::write_with_backup(&staging, &m, false)?;
    let dest = pet_dir(root, name);
    std::fs::rename(&staging, &dest).map_err(|e| format!("落地失败: {}", e))?;
    scan::list_pets_in(root)
        .into_iter()
        .find(|s| s.id == name)
        .ok_or_else(|| "落地后读取宠物信息失败".to_string())
}

/// 取消导入：清理暂存区（spec §8.4-6）
pub fn cancel_in(root: &Path, staging_id: &str) -> Result<(), String> {
    let Ok(staging) = staging_dir(root, staging_id) else { return Ok(()) };
    std::fs::remove_dir_all(&staging).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkpet(root: &Path, id: &str) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("voice/general")).unwrap();
        std::fs::write(dir.join(SHEET_FILE), b"sheet-bytes").unwrap();
        std::fs::write(dir.join("voice/general/休息一下吧.m4a"), b"a").unwrap();
        dir
    }

    #[test]
    fn stage_from_folder_copies_sheet_and_voice() {
        let root = tempfile::tempdir().unwrap();
        let src = mkpet(root.path(), "src-pet");
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        assert_eq!(s.suggested_name, "src-pet");
        assert_eq!(s.spritesheet_size, 11);
        assert_eq!(s.voice_files.len(), 1);
        assert_eq!(s.voice_files[0].group, "general");
        assert_eq!(s.voice_files[0].name, "休息一下吧");
        assert!(staging_root(root.path()).join(&s.staging_id).join(SHEET_FILE).is_file());
    }

    #[test]
    fn stage_locates_sheet_one_level_deep() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("wrapper");
        mkpet(root.path(), "wrapper/inner-pet"); // src/wrapper/inner-pet/...
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        assert_eq!(s.suggested_name, "inner-pet"); // 用图集所在目录名
    }

    #[test]
    fn stage_without_sheet_errs_and_cleans() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("empty");
        std::fs::create_dir_all(&src).unwrap();
        assert!(stage_from_folder_in(root.path(), &src).is_err());
        assert!(staging_root(root.path()).read_dir().map(|mut d| d.next().is_none()).unwrap_or(true));
    }

    #[test]
    fn codex_meta_prefills_display_and_version() {
        let root = tempfile::tempdir().unwrap();
        let src = mkpet(root.path(), "linabell");
        std::fs::write(
            src.join("pet.json"),
            r#"{"displayName":"玲娜贝儿","spriteVersionNumber":2}"#,
        )
        .unwrap();
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        assert_eq!(s.suggested_display_name, "玲娜贝儿");
        assert_eq!(s.sprite_version_number, 2);
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            std::io::Write::write_all(&mut w, data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn zip_stage_unwraps_one_level() {
        let root = tempfile::tempdir().unwrap();
        let zp = root.path().join("p.zip");
        write_zip(&zp, &[("inner/spritesheet.webp", "sheet"), ("inner/pet.json", "{}")]);
        let s = stage_from_zip_in(root.path(), &zp).unwrap();
        assert_eq!(s.suggested_name, "inner");
        assert_eq!(s.spritesheet_size, 5);
    }

    #[test]
    fn zip_slip_rejected() {
        let root = tempfile::tempdir().unwrap();
        let zp = root.path().join("evil.zip");
        // zip crate 的 writer 会规范化 ".."，手工构造恶意条目名直接写仍可能被拒；
        // 因此本测试断言 safe_unzip 对该条目返回 Err（enclosed_name 拒绝穿越）
        write_zip(&zp, &[("../evil.txt", "x")]);
        let dest = root.path().join("dest");
        let r = safe_unzip(&zp, &dest);
        // 无论 zip writer 是否已规范化：要么安全拒绝，要么落点必须在 dest 内（无 dest 外文件）
        match r {
            Err(_) => {}
            Ok(()) => assert!(!root.path().join("evil.txt").exists()),
        }
    }

    #[test]
    fn zip_size_cap_on_actual_bytes() {
        let root = tempfile::tempdir().unwrap();
        let zp = root.path().join("big.zip");
        // 压缩后条目实际解出 20 字节，上限 10 → 必须按解压实际字节数拒绝（FIX-5）
        write_zip(&zp, &[("a.bin", "01234567890123456789")]);
        let dest = root.path().join("dest2");
        let r = safe_unzip_with_limit(&zp, &dest, 10);
        assert!(r.is_err(), "20 字节条目应超 10 字节上限");
        // 失败时 dest 不留残留文件
        let leftover = std::fs::read_dir(&dest)
            .map(|rd| rd.flatten().any(|e| e.path().is_file()))
            .unwrap_or(false);
        assert!(!leftover, "上限拒绝后 dest 不应有残留文件");
        // 同一包 20 字节上限正常通过
        let dest_ok = root.path().join("dest3");
        assert!(safe_unzip_with_limit(&zp, &dest_ok, 20).is_ok());
        // 错误信息人类可读（FIX-9）：小上限显示 ">10B"，公开上限显示 ">100MB"
        assert!(r.unwrap_err().contains(">10B"));
    }

    #[test]
    fn fmt_limit_human_readable() {
        // 整除 MiB → MB；否则字节（FIX-9）
        assert_eq!(fmt_limit(100 * 1024 * 1024), "100MB");
        assert_eq!(fmt_limit(1024 * 1024), "1MB");
        assert_eq!(fmt_limit(10), "10B");
        assert_eq!(fmt_limit(1024 * 1024 + 1), "1048577B"); // 非整除走 B
        // 公开路径错误信息由 fmt_limit(MAX_ZIP_TOTAL_BYTES) 生成 → 恢复 ">100MB"
        assert_eq!(fmt_limit(MAX_ZIP_TOTAL_BYTES), "100MB");
    }

    #[test]
    fn finalize_moves_and_names_manifest() {
        let root = tempfile::tempdir().unwrap();
        let src = mkpet(root.path(), "src");
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        let m = manifest::PetManifest {
            schema_version: 1,
            id: String::new(), // 由 finalize 回填
            display_name: "Starry Dew".into(),
            description: String::new(),
            source: "folder".into(),
            sprite_version_number: 1,
            spritesheet_size_bytes: s.spritesheet_size,
            has_voice: false,
            has_subtitle: false,
            voices: vec![],
        };
        let sum = finalize_in(root.path(), &s.staging_id, "starry-dew", m).unwrap();
        assert_eq!(sum.id, "starry-dew");
        assert!(pet_dir(root.path(), "starry-dew").join("manifest.json").is_file());
        // 暂存区已腾空
        assert!(!staging_root(root.path()).join(&s.staging_id).exists());
    }

    #[test]
    fn validate_pet_name_rules() {
        let root = tempfile::tempdir().unwrap();
        assert!(validate_pet_name(root.path(), "abc-123_X").is_ok());
        assert!(validate_pet_name(root.path(), "").is_err());
        assert!(validate_pet_name(root.path(), "中文").is_err());
        assert!(validate_pet_name(root.path(), "../hack").is_err());
        assert!(validate_pet_name(root.path(), "foxbell").is_err());
        assert!(validate_pet_name(root.path(), "FoxBell").is_err());
        std::fs::create_dir_all(root.path().join("dup")).unwrap();
        assert!(validate_pet_name(root.path(), "dup").is_err());
    }

    #[test]
    fn stage_audio_copies_and_remove_deletes() {
        let root = tempfile::tempdir().unwrap();
        let src = mkpet(root.path(), "src");
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        let audio_src = root.path().join("hi.mp3");
        std::fs::write(&audio_src, b"mp3-bytes").unwrap();
        let added = stage_audio_in(root.path(), &s.staging_id, &[audio_src.to_string_lossy().to_string()], "done").unwrap();
        assert_eq!(added[0].file, "voice/done/hi.mp3");
        assert_eq!(added[0].name, "hi");
        assert!(stage_audio_in(root.path(), &s.staging_id, &[], "bad-group").is_err());
        remove_audio_in(root.path(), &s.staging_id, "voice/done/hi.mp3", true).unwrap();
        assert!(remove_audio_in(root.path(), &s.staging_id, "../evil", true).is_err());
    }

    #[test]
    fn cancel_cleans_staging() {
        let root = tempfile::tempdir().unwrap();
        let src = mkpet(root.path(), "src");
        let s = stage_from_folder_in(root.path(), &src).unwrap();
        cancel_in(root.path(), &s.staging_id).unwrap();
        assert!(!staging_root(root.path()).join(&s.staging_id).exists());
    }
}