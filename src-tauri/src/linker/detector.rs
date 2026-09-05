// 工具检测器 — 检测已安装的 AI 编程工具
// 简化版（无 rayon，3 个工具顺序检测足够快）

use crate::adapter::{all_adapters, AgentAdapter};
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
    let adapters: Vec<Box<dyn AgentAdapter>> = all_adapters();

    adapters
        .iter()
        .map(|adapter| {
            let base = adapter.base_dir();
            let dir_exists = base.exists();
            // 防御空切片：心跳驱动工具（workbuddy）无进程名 → cli_available=false
            let cli_available = cli_available(adapter.as_ref());

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

/// CLI 可用性（dir OR CLI 口径的 CLI 半边，detect_all_tools 与 is_tool_installed
/// 共用，防口径漂移）：检查首个进程名是否在 PATH。防御空切片：心跳驱动工具
/// （workbuddy）无进程名 → false
fn cli_available(adapter: &dyn AgentAdapter) -> bool {
    adapter
        .process_names()
        .first()
        .map(|name| which(name))
        .unwrap_or(false)
}

/// 工具已安装判定（issue #36-7：dir OR CLI，与 detect_all_tools 同一口径）：
/// base 目录存在，或首个进程名在 PATH 可达。心跳驱动工具（workbuddy）无进程名
/// → 仅看 base_dir。供工具管理页 installed 徽标复用，消除两页口径不一致
pub fn is_tool_installed(adapter: &dyn AgentAdapter) -> bool {
    adapter.base_dir().exists() || cli_available(adapter)
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
