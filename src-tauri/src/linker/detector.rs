// 工具检测器 — 检测已安装的 AI 编程工具
// 简化版（无 rayon，3 个工具顺序检测足够快）

use crate::adapter::{AgentAdapter, claude::ClaudeAdapter, codex::CodexAdapter, opencode::OpenCodeAdapter};
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
    ];

    adapters.iter().map(|adapter| {
        let base = adapter.base_dir();
        let dir_exists = base.exists();
        // 检测 CLI 可用性：检查进程名的第一个字符
        let cli_available = which(&adapter.process_names()[0]);

        debug!("{}: dir={}, cli={}", adapter.name(), dir_exists, cli_available);

        ToolDetection {
            tool_id: format!("{:?}", adapter.agent_type()).to_lowercase(),
            name: adapter.name().to_string(),
            base_dir: base.to_string_lossy().to_string(),
            dir_exists,
            cli_available,
        }
    }).collect()
}

/// 简易 which 命令 — 检测可执行文件是否在 PATH 中
fn which(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
