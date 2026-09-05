// manifest.json — 结构、读取与备份写入（spec §4.2）。写入前自动备份 manifest.json.bak（仅保留最近一份）
use super::error::PetRpcError;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MANIFEST_FILE: &str = "manifest.json";
pub const BACKUP_FILE: &str = "manifest.json.bak";
pub const SCHEMA_VERSION: u32 = 1;

/// 四个固定语音分组（与 foxbell 语音系统一致，spec §5.1）
pub const VOICE_GROUPS: [&str; 4] = ["general", "approval", "done", "error"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEntry {
    pub group: String,
    /// 字幕文本 = 音频文件名去扩展名（EP8）
    pub name: String,
    /// 相对宠物目录：voice/<group>/<文件名>
    pub file: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {
    pub schema_version: u32,
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// codex | petdex | folder | zip
    #[serde(default)]
    pub source: String,
    /// 1=v1（9 行） 2=v2（11 行）；0=未知（尚未探测）
    pub sprite_version_number: u8,
    #[serde(default)]
    pub spritesheet_size_bytes: u64,
    pub has_voice: bool,
    pub has_subtitle: bool,
    #[serde(default)]
    pub voices: Vec<VoiceEntry>,
}

/// 读取 manifest.json；任何失败（缺失/损坏）返回 None，由调用方决定生成或修复（spec §6）
pub fn load(dir: &Path) -> Option<PetManifest> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 写 manifest.json；backup=true 且旧文件存在时先复制为 manifest.json.bak（spec §4.1）
pub fn write_with_backup(dir: &Path, m: &PetManifest, backup: bool) -> Result<(), PetRpcError> {
    let path = dir.join(MANIFEST_FILE);
    if backup && path.exists() {
        std::fs::copy(&path, dir.join(BACKUP_FILE))
            .map_err(|e| PetRpcError::new("manifest-backup-failed", format!("备份 manifest 失败: {}", e)).with("err", e.to_string()))?;
    }
    let text = serde_json::to_string_pretty(m).map_err(|e| PetRpcError::internal(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| PetRpcError::new("manifest-write-failed", format!("写入 manifest 失败: {}", e)).with("err", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PetManifest {
        PetManifest {
            schema_version: SCHEMA_VERSION,
            id: "starry-dew".into(),
            display_name: "Starry Dew".into(),
            description: "test".into(),
            source: "folder".into(),
            sprite_version_number: 1,
            spritesheet_size_bytes: 1652314,
            has_voice: true,
            has_subtitle: true,
            voices: vec![VoiceEntry {
                group: "general".into(),
                name: "休息一下吧".into(),
                file: "voice/general/休息一下吧.m4a".into(),
                size_bytes: 123456,
                duration_ms: 3200,
            }],
        }
    }

    #[test]
    fn serde_roundtrip_camel_case() {
        let json = serde_json::to_string(&sample()).unwrap();
        // camelCase 字段名（前端契约）
        assert!(json.contains("\"schemaVersion\""));
        assert!(json.contains("\"displayName\""));
        assert!(json.contains("\"spriteVersionNumber\""));
        assert!(json.contains("\"sizeBytes\""));
        let back: PetManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sample());
    }

    #[test]
    fn default_fields_tolerate_missing() {
        // description/source/spritesheetSizeBytes/voices 均可缺省
        let m: PetManifest = serde_json::from_str(
            r#"{"schemaVersion":1,"id":"a","displayName":"A","spriteVersionNumber":1,"hasVoice":false,"hasSubtitle":false}"#,
        )
        .unwrap();
        assert_eq!(m.voices.len(), 0);
        assert_eq!(m.source, "");
    }

    #[test]
    fn load_missing_or_corrupt_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(load(tmp.path()).is_none());
        std::fs::write(tmp.path().join(MANIFEST_FILE), "{broken").unwrap();
        assert!(load(tmp.path()).is_none());
    }

    #[test]
    fn write_with_backup_keeps_one_bak() {
        let tmp = tempfile::tempdir().unwrap();
        let mut m = sample();
        write_with_backup(tmp.path(), &m, false).unwrap();
        m.display_name = "v2".into();
        write_with_backup(tmp.path(), &m, true).unwrap();
        let bak = std::fs::read_to_string(tmp.path().join(BACKUP_FILE)).unwrap();
        assert!(bak.contains("Starry Dew")); // bak 是旧内容
        assert_eq!(load(tmp.path()).unwrap().display_name, "v2");
    }
}