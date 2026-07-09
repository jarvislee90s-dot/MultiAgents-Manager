use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use super::types::Manifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub github_repo: Option<String>,
    pub installed: bool,
    pub featured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreIndex {
    pub version: String,
    pub updated: String,
    pub entries: Vec<StoreEntry>,
}

fn store_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".mam/store/index.json")
}

pub fn read_index() -> Result<serde_json::Value, String> {
    let path = store_path();
    if !path.exists() {
        let empty = StoreIndex { version: "1".into(), updated: chrono::Utc::now().to_rfc3339(), entries: vec![] };
        return serde_json::to_value(&empty).map_err(|e| e.to_string());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str::<serde_json::Value>(&content).map_err(|e| e.to_string())
}

pub fn add_entry(manifest: &Manifest) -> Result<(), String> {
    let path = store_path();
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let mut index = if path.exists() {
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        serde_json::from_str::<StoreIndex>(&content).unwrap_or(StoreIndex { version: "1".into(), updated: String::new(), entries: vec![] })
    } else {
        StoreIndex { version: "1".into(), updated: String::new(), entries: vec![] }
    };
    let entry = StoreEntry {
        id: manifest.common.id.clone(), name: manifest.common.name.clone(),
        kind: format!("{:?}", manifest.common.kind).to_lowercase(), version: manifest.common.version.clone(),
        description: manifest.common.description.clone(), tags: manifest.common.tags.clone(),
        github_repo: manifest.common.github_repo.clone(), installed: true, featured: false,
    };
    index.entries.retain(|e| e.id != entry.id);
    index.entries.push(entry);
    index.updated = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_entry(id: &str) -> Result<(), String> {
    let path = store_path();
    if !path.exists() { return Ok(()); }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut index: StoreIndex = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    if let Some(entry) = index.entries.iter_mut().find(|e| e.id == id) { entry.installed = false; }
    index.updated = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}
