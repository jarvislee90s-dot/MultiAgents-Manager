use serde::{Deserialize, Serialize};

/// AI 编程工具类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Claude,
    Codex,
    OpenCode,
    OpenClaw,
}

/// 会话状态（红绿灯五态 + Finished）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Waiting,
    Processing,
    Thinking,
    Compacting,
    Idle,
    Finished,
}

/// 进程形态：CLI 或桌面 APP
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProcessForm {
    Cli,
    App,
}

/// 跳转终端是否可用：Windows 下 CLI 与 App 均可窗口级聚焦（见 window/win32.rs）；
/// macOS 仅 CLI（TTY 链路，window/ 模块的 iTerm2/Terminal.app/tmux）；其他平台不支持
pub fn jump_supported_for(form: ProcessForm) -> bool {
    if cfg!(windows) {
        // Windows：CLI 与 App 均可窗口级聚焦（见 window/win32.rs）
        return true;
    }
    matches!(form, ProcessForm::Cli) && cfg!(target_os = "macos")
}

/// 一次 AI 编程工具的运行实例
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub agent_type: AgentType,
    pub project_name: String,
    pub project_path: String,
    pub title: Option<String>,
    pub git_branch: Option<String>,
    pub github_url: Option<String>,
    pub status: SessionStatus,
    pub last_message: Option<String>,
    pub last_message_role: Option<String>,
    pub last_activity_at: String,
    pub pid: u32,
    pub cpu_usage: f32,
    pub active_subagent_count: usize,
    /// 进程形态（CLI / 桌面 APP），APP 形态不可跳转
    pub form: ProcessForm,
    /// 是否支持终端跳转（CLI=true, App=false）
    pub jump_supported: bool,
}

/// 全部会话的聚合响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsResponse {
    pub sessions: Vec<Session>,
    pub total_count: usize,
    pub waiting_count: usize,
}

/// JSONL 消息解析结构（Claude / Codex 共用）
#[derive(Debug, Deserialize)]
pub(crate) struct JsonlMessage {
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    pub cwd: Option<String>,
    pub timestamp: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    pub subtype: Option<String>,
    #[serde(rename = "isCompactSummary")]
    pub is_compact_summary: Option<bool>,
    pub message: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MessageContent {
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
}

#[cfg(test)]
mod jump_tests {
    use super::*;

    #[test]
    fn jump_supported_matches_platform_matrix() {
        // Windows：CLI 与 App 均可窗口级聚焦；macOS：仅 CLI（TTY 链路）；其他平台：不支持
        if cfg!(windows) {
            assert!(jump_supported_for(ProcessForm::Cli));
            assert!(jump_supported_for(ProcessForm::App));
        } else if cfg!(target_os = "macos") {
            assert!(jump_supported_for(ProcessForm::Cli));
            assert!(!jump_supported_for(ProcessForm::App));
        } else {
            assert!(!jump_supported_for(ProcessForm::Cli));
            assert!(!jump_supported_for(ProcessForm::App));
        }
    }
}
