use serde::{Deserialize, Serialize};

/// AI 编程工具类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Claude,
    Codex,
    OpenCode,
    OpenClaw,
    Kimi,
    WorkBuddy,
    ZCode,
}

impl AgentType {
    /// 稳定 tool_id 字符串（与 serde lowercase 形态一致）。此前多处以
    /// `format!("{:?}", variant).to_lowercase()` 推导（PR #46 review M4：adapter/mod.rs
    /// 5 处 + hooks.rs 1 处），依赖「变体名小写恰好等于 tool_id」的隐式约定——
    /// 给枚举加自定义 Debug 或重命名变体会全部同时静默错位，且错的是进程匹配/
    /// 宿主判定这类核心链路。match 穷尽使新增变体在编译期强制补映射
    pub fn tool_id(&self) -> &'static str {
        match self {
            AgentType::Claude => "claude",
            AgentType::Codex => "codex",
            AgentType::OpenCode => "opencode",
            AgentType::OpenClaw => "openclaw",
            AgentType::Kimi => "kimi",
            AgentType::WorkBuddy => "workbuddy",
            AgentType::ZCode => "zcode",
        }
    }
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
/// macOS 下 CLI 走 TTY 链路、App 走 activate application（W2）；其他平台不支持
pub fn jump_supported_for(form: ProcessForm) -> bool {
    let _ = form; // 矩阵只取决于平台：CLI 与 App 形态同可用性（App 激活链路 W2）
    if cfg!(windows) {
        return true;
    }
    cfg!(target_os = "macos")
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
    /// 进程形态（CLI / 桌面 APP）
    pub form: ProcessForm,
    /// 是否支持跳转（CLI=TTY 链路，App=APP 激活）
    pub jump_supported: bool,
    /// 未读标记（W4）：true = 绿色已完成且用户未查看的持久未读卡（APP 类专用）
    pub unread: bool,
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
        // Windows：CLI 与 App 均可窗口级聚焦；macOS：CLI 走 TTY、App 走 APP 激活（W2）；
        // 其他平台：不支持。两平台结果一致（均支持），故合并断言
        let expected = cfg!(windows) || cfg!(target_os = "macos");
        assert_eq!(jump_supported_for(ProcessForm::Cli), expected);
        assert_eq!(jump_supported_for(ProcessForm::App), expected);
    }
}
