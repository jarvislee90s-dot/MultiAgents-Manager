// 工具检测器 — 检测已安装的 AI 编程工具
// 简化版（无 rayon，3 个工具顺序检测足够快）

use crate::adapter::{
    claude::ClaudeAdapter, codex::CodexAdapter, openclaw::OpenClawAdapter,
    opencode::OpenCodeAdapter, AgentAdapter,
};
use log::debug;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDetection {
    pub tool_id: String,
    pub name: String,
    pub base_dir: String,
    pub dir_exists: bool,
    pub cli_available: bool,
}

/// 检测所有已安装的工具
pub fn detect_all_tools() -> Vec<ToolDetection> {
    let adapters: Vec<Box<dyn AgentAdapter>> = vec![
        Box::new(ClaudeAdapter),
        Box::new(CodexAdapter),
        Box::new(OpenCodeAdapter),
        Box::new(OpenClawAdapter),
    ];

    adapters
        .iter()
        .map(|adapter| {
            let base = adapter.base_dir();
            let dir_exists = base.exists();
            // 检测 CLI 可用性：检查进程名的第一个字符
            let cli_available = which(adapter.process_names()[0]);

            debug!(
                "{}: dir={}, cli={}",
                adapter.name(),
                dir_exists,
                cli_available
            );

            ToolDetection {
                tool_id: format!("{:?}", adapter.agent_type()).to_lowercase(),
                name: adapter.name().to_string(),
                base_dir: base.to_string_lossy().to_string(),
                dir_exists,
                cli_available,
            }
        })
        .collect()
}

/// 检测可执行文件是否在 PATH 中（纯路径扫描，不 spawn 子进程，跨平台）
/// Windows 下额外尝试 .exe 扩展名
fn which(cmd: &str) -> bool {
    let path_env = std::env::var("PATH").unwrap_or_default();
    let candidates: Vec<String> = if cfg!(windows) {
        vec![format!("{}.exe", cmd), cmd.to_string()]
    } else {
        vec![cmd.to_string()]
    };
    std::env::split_paths(&path_env)
        .any(|dir| candidates.iter().any(|name| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_which_finds_git() {
        // CI 与开发机均安装 git
        assert!(which("git"));
    }

    #[test]
    fn test_which_rejects_missing_cmd() {
        assert!(!which("mam-definitely-missing-cmd"));
    }
}
