// 纯消息状态判断 — 从消息内容推导会话状态，不依赖 CPU 或文件年龄启发式
// 移植自 agent-sessions session/status.rs

use crate::session::SessionStatus;

/// 检查 content 是否包含 tool_use 块
pub fn has_tool_use(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "tool_use")
                .unwrap_or(false)
        })
    } else {
        false
    }
}

/// 检查是否所有 tool_use 都是用户输入类工具（如 AskUserQuestion）— 这些应算 Waiting
pub fn is_waiting_for_user_input(content: &serde_json::Value) -> bool {
    let user_input_tools = ["AskUserQuestion"];
    if let serde_json::Value::Array(arr) = content {
        let tool_use_blocks: Vec<_> = arr.iter()
            .filter(|item| {
                item.get("type").and_then(|t| t.as_str()).map(|t| t == "tool_use").unwrap_or(false)
            })
            .collect();
        !tool_use_blocks.is_empty() && tool_use_blocks.iter().all(|item| {
            item.get("name").and_then(|n| n.as_str())
                .map(|name| user_input_tools.contains(&name)).unwrap_or(false)
        })
    } else {
        false
    }
}

/// 检查 content 是否包含 tool_result 块
pub fn has_tool_result(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()).map(|t| t == "tool_result").unwrap_or(false)
        })
    } else {
        false
    }
}

fn extract_text_content(content: &serde_json::Value) -> &str {
    match content {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Array(arr) => {
            arr.iter().find_map(|v| v.get("text").and_then(|t| t.as_str())).unwrap_or("")
        }
        _ => "",
    }
}

/// 用户按 Escape 中断了请求
pub fn is_interrupted_request(content: &serde_json::Value) -> bool {
    extract_text_content(content).contains("[Request interrupted by user]")
}

/// 本地斜杠命令（不触发 Claude 回复）
pub fn is_local_slash_command(content: &serde_json::Value) -> bool {
    let text = extract_text_content(content);
    let trimmed = text.trim();
    let local_commands = [
        "/clear", "/compact", "/help", "/config", "/cost", "/doctor",
        "/init", "/login", "/logout", "/memory", "/model", "/permissions",
        "/pr-comments", "/review", "/status", "/terminal-setup", "/vim",
    ];
    if local_commands.iter().any(|cmd| trimmed == *cmd || trimmed.starts_with(&format!("{} ", cmd))) {
        return true;
    }
    if let Some(start) = trimmed.find("<command-name>") {
        let after = &trimmed[start + "<command-name>".len()..];
        if let Some(end) = after.find("</command-name>") {
            let cmd_name = after[..end].trim();
            return local_commands.iter().any(|cmd| cmd_name == *cmd || cmd_name.starts_with(&format!("{} ", cmd)));
        }
    }
    false
}



/// 根据最后一条消息推导会话状态
pub fn determine_status(
    last_msg_type: Option<&str>,
    has_tool_use: bool,
    _has_tool_result: bool,
    is_local_command: bool,
    is_interrupted: bool,
    is_user_input_tool: bool,
    file_recently_modified: bool,
) -> SessionStatus {
    match last_msg_type {
        Some("assistant") => {
            if has_tool_use && is_user_input_tool {
                SessionStatus::Waiting
            } else if has_tool_use {
                SessionStatus::Processing
            } else if file_recently_modified {
                SessionStatus::Processing
            } else {
                // Assistant finished responding, no pending tool calls → idle
                SessionStatus::Idle
            }
        }
        Some("user") => {
            if is_local_command || is_interrupted {
                SessionStatus::Waiting
            } else {
                SessionStatus::Thinking
            }
        }
        _ => {
            if file_recently_modified {
                SessionStatus::Processing
            } else {
                SessionStatus::Waiting
            }
        }
    }
}
