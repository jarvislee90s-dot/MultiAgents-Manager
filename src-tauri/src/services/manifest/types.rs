use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Skill,
    Mcp,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Permission {
    #[serde(rename = "filesystem.read")]
    FilesystemRead,
    #[serde(rename = "filesystem.write")]
    FilesystemWrite,
    #[serde(rename = "network")]
    Network,
    #[serde(rename = "shell")]
    Shell,
    #[serde(rename = "env.read")]
    EnvRead,
    #[serde(rename = "settings.write")]
    SettingsWrite,
    #[serde(rename = "symlink.create")]
    SymlinkCreate,
}

impl Permission {
    pub fn risk_level(&self) -> &'static str {
        match self {
            Permission::FilesystemRead | Permission::SymlinkCreate => "low",
            Permission::FilesystemWrite | Permission::Network | Permission::EnvRead => "medium",
            Permission::Shell | Permission::SettingsWrite => "high",
        }
    }
    pub fn description(&self) -> &'static str {
        match self {
            Permission::FilesystemRead => "读取文件",
            Permission::FilesystemWrite => "写入文件",
            Permission::Network => "发起网络请求",
            Permission::Shell => "执行 shell 命令（高风险）",
            Permission::EnvRead => "读取环境变量",
            Permission::SettingsWrite => "写入工具配置文件（高风险）",
            Permission::SymlinkCreate => "创建符号链接",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityEntry {
    pub tool: String,
    pub min_version: Option<String>,
    pub mcp_format: Option<String>,
    pub sub_agent_support: Option<bool>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub url: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCommon {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: Kind,
    pub description: Option<String>,
    pub author: Option<Author>,
    pub homepage: Option<String>,
    pub icon_url: Option<String>,
    pub tags: Option<Vec<String>>,
    pub min_runtime: Option<String>,
    pub github_repo: Option<String>,
    pub permissions: Option<Vec<Permission>>,
    pub compatibility: Option<Vec<CompatibilityEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFields {
    pub entry: String,
    pub includes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpFields {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub env: Option<std::collections::BTreeMap<String, String>>,
    pub formats: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginFields {
    pub entry: String,
    #[serde(rename = "type")]
    pub plugin_type: String,
    pub config_template: Option<String>,
}

/// 完整 Manifest（使用 #[serde(flatten)] 展开公共字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(flatten)]
    pub common: ManifestCommon,
    pub skill: Option<SkillFields>,
    pub mcp: Option<McpFields>,
    pub plugin: Option<PluginFields>,
}
