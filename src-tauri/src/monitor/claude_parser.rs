// Claude Code 会话解析 — message.role + content[] 协议
// 移植自 agent-sessions session/parser.rs；公共设施（cwd 归一化、路径编解码、
// git URL 缓存、JSONL 读取）见 monitor::{cwd,path_codec,git,project,jsonl}

use super::cwd::normalize_cwd_for_match;
use super::git::get_github_url;
use super::jsonl::{
    count_active_subagents, extract_cwd_from_jsonl, get_recent_jsonl_files, read_recent_lines,
};
use super::path_codec::{convert_dir_name_to_path, convert_path_to_dir_name};
use super::project::project_name_from_path;
use super::session_scan::SessionFileScan;
use super::status::*;
use crate::adapter::AgentProcess;
use crate::session::model::JsonlMessage;
use crate::session::{jump_supported_for, AgentType, Session, SessionStatus};
use log::info;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const RECENT_LINES: usize = 500;

/// L2 摘要缓存（monitor::session_scan）：claude 为进程界定有界扫描（目录名预过滤），
/// L1+L2 已足够——窗口化反而会丢空闲超窗的活跃卡（见 session_scan 模块文档）
const CLAUDE_CWD_SCAN: SessionFileScan = SessionFileScan::new("claude-cwd");
const CLAUDE_DIGEST_SCAN: SessionFileScan = SessionFileScan::new("claude-digest");

/// 扫描 ~/.claude/projects，匹配运行中的 Claude 进程
pub fn get_claude_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();

    // cwd -> processes 映射
    let mut cwd_to_processes: HashMap<String, Vec<&AgentProcess>> = HashMap::new();
    let mut expected_dir_names: HashSet<String> = HashSet::new();
    for process in processes {
        if let Some(cwd) = &process.cwd {
            let normalized = normalize_cwd_for_match(&cwd.to_string_lossy());
            // 目录名比较大小写不敏感：Claude 保留用户 cd 时敲入的盘符/路径大小写
            // （实机同时存在 E--xxx 与 e--xxx 两种目录），sysinfo 返回的可能是另一种
            expected_dir_names.insert(convert_path_to_dir_name(&normalized).to_lowercase());
            cwd_to_processes
                .entry(normalized)
                .or_default()
                .push(process);
        }
    }

    let claude_dir = dirs::home_dir()
        .map(|h| h.join(".claude").join("projects"))
        .unwrap_or_default();
    if !claude_dir.exists() {
        return sessions;
    }

    if let Ok(entries) = fs::read_dir(&claude_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !expected_dir_names.contains(&dir_name.to_lowercase()) {
                continue;
            }

            let jsonl_files = get_recent_jsonl_files(&path);
            if jsonl_files.is_empty() {
                continue;
            }

            let mut cwd_to_files: HashMap<String, Vec<std::path::PathBuf>> = HashMap::new();
            for f in &jsonl_files {
                // L2：cwd 提取走 (mtime,size) 缓存（纯内容函数）；未解析出 cwd 才用目录名反推
                let file_cwd = CLAUDE_CWD_SCAN
                    .parse(f, extract_cwd_from_jsonl)
                    .as_ref()
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| convert_dir_name_to_path(dir_name));
                // 与进程 cwd 同一归一化域（Windows 下小写、无尾部分隔符），两侧才可比
                cwd_to_files
                    .entry(normalize_cwd_for_match(&file_cwd))
                    .or_default()
                    .push(f.clone());
            }

            for (project_path, files) in &cwd_to_files {
                if let Some(procs) = cwd_to_processes.get(project_path) {
                    for (idx, proc) in procs.iter().enumerate() {
                        if let Some(f) = files.get(idx) {
                            if let Some(mut session) = parse_claude_jsonl(f, project_path, proc) {
                                session.active_subagent_count =
                                    count_active_subagents(&path, &session.id);
                                sessions.push(session);
                            }
                        }
                    }
                }
            }
        }
    }

    info!(
        "Claude: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// 纯内容摘要（L2 缓存产物）：只由文件内容决定；100 字符截断随摘要一并缓存
struct ClaudeFileDigest {
    session_id: Option<String>,
    git_branch: Option<String>,
    last_timestamp: Option<String>,
    last_msg_type: Option<String>,
    last_has_tool_use: bool,
    last_has_tool_result: bool,
    last_is_local: bool,
    last_is_interrupted: bool,
    last_is_user_input: bool,
    is_compacting: bool,
    last_message: Option<String>,
    last_role: Option<String>,
}

fn read_claude_digest(jsonl_path: &Path) -> ClaudeFileDigest {
    let recent = read_recent_lines(jsonl_path, RECENT_LINES);

    let mut session_id = None;
    let mut git_branch = None;
    let mut last_timestamp = None;
    let mut last_message = None;
    let mut last_role = None;
    let mut last_msg_type = None;
    let mut last_has_tool_use = false;
    let mut last_has_tool_result = false;
    let mut last_is_local = false;
    let mut last_is_interrupted = false;
    let mut last_is_user_input = false;
    let mut found_status = false;
    let mut is_compacting = false;

    for line in recent.iter().rev() {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(line) {
            if session_id.is_none() {
                session_id = msg.session_id;
            }
            if git_branch.is_none() {
                git_branch = msg.git_branch;
            }
            if last_timestamp.is_none() {
                last_timestamp = msg.timestamp;
            }

            if !found_status && !is_compacting {
                if msg.is_compact_summary == Some(true) {
                    // compaction 已完成
                } else if msg.subtype.as_deref() == Some("compact_boundary") {
                    is_compacting = true;
                }
            }

            if !found_status {
                if let Some(content) = &msg.message {
                    if let Some(c) = &content.content {
                        let has_content = match c {
                            serde_json::Value::String(s) => !s.is_empty(),
                            serde_json::Value::Array(arr) => !arr.is_empty(),
                            _ => false,
                        };
                        if has_content {
                            last_msg_type = msg.msg_type.clone();
                            last_role = content.role.clone();
                            last_has_tool_use = has_tool_use(c);
                            last_has_tool_result = has_tool_result(c);
                            last_is_local = is_local_slash_command(c);
                            last_is_interrupted = is_interrupted_request(c);
                            last_is_user_input = is_waiting_for_user_input(c);
                            found_status = true;
                        }
                    }
                }
            }

            if session_id.is_some() && found_status {
                break;
            }
        }
    }

    // 找最后一条有文本的消息作为预览
    for line in recent.iter().rev() {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(line) {
            if let Some(content) = &msg.message {
                if let Some(c) = &content.content {
                    let text = match c {
                        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                        serde_json::Value::Array(arr) => arr.iter().find_map(|v| {
                            v.get("text")
                                .and_then(|t| t.as_str())
                                .filter(|s| !s.is_empty())
                                .map(String::from)
                        }),
                        _ => None,
                    };
                    if text.is_some() {
                        last_message = text;
                        break;
                    }
                }
            }
        }
    }

    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 {
            format!("{}...", m.chars().take(100).collect::<String>())
        } else {
            m
        }
    });

    ClaudeFileDigest {
        session_id,
        git_branch,
        last_timestamp,
        last_msg_type,
        last_has_tool_use,
        last_has_tool_result,
        last_is_local,
        last_is_interrupted,
        last_is_user_input,
        is_compacting,
        last_message,
        last_role,
    }
}

/// 解析单个 Claude JSONL 文件（L2 缓存摘要 + 时间叠加/进程盖章现算，保留原签名）
fn parse_claude_jsonl(
    jsonl_path: &Path,
    project_path: &str,
    process: &AgentProcess,
) -> Option<Session> {
    use std::time::SystemTime;

    let d = CLAUDE_DIGEST_SCAN.parse(jsonl_path, read_claude_digest);
    let digest = d.as_ref();
    // 时间叠加每次现算（L2 契约）：recently-modified 参与状态判定，不能进缓存
    let file_recently_modified = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|age| age.as_secs_f32() < 3.0)
        .unwrap_or(false);

    let session_id = digest.session_id.clone()?;
    // 卡片前缀统一 8 位（按字符截取，多字节 id 不 panic），与 hook marker（MAM:<id 前 8 位>）保持一致
    let session_title = session_id.chars().take(8).collect::<String>();
    let status = if digest.is_compacting {
        SessionStatus::Compacting
    } else {
        determine_status(
            digest.last_msg_type.as_deref(),
            digest.last_has_tool_use,
            digest.last_has_tool_result,
            digest.last_is_local,
            digest.last_is_interrupted,
            digest.last_is_user_input,
            file_recently_modified,
        )
    };

    let project_name = project_name_from_path(project_path);
    Some(Session {
        id: session_id,
        agent_type: AgentType::Claude,
        project_name,
        project_path: project_path.to_string(),
        git_branch: digest.git_branch.clone(),
        github_url: get_github_url(project_path),
        status,
        last_message: digest.last_message.clone(),
        last_message_role: digest.last_role.clone(),
        last_activity_at: digest
            .last_timestamp
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
        pid: process.pid,
        cpu_usage: process.cpu_usage,
        active_subagent_count: 0,
        form: process.form,
        jump_supported: jump_supported_for(process.form),
        unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        title: Some(session_title),
    })
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use crate::session::ProcessForm;

    fn fake_process(pid: u32) -> AgentProcess {
        AgentProcess {
            pid,
            cpu_usage: 0.0,
            cwd: Some(std::path::PathBuf::from("/work/demo")),
            form: ProcessForm::Cli,
            exe: None,
        }
    }

    /// sessionId 含多字节字符时，标题回退按字符取前 8 位，不得 panic
    #[test]
    fn multibyte_session_id_title_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let jsonl = tmp.path().join("session.jsonl");
        std::fs::write(
            &jsonl,
            concat!(
                r#"{"sessionId":"会话🔥x","cwd":"/work/demo","timestamp":"2026-01-01T00:00:00Z","#,
                r#""type":"user","message":{"role":"user","content":"hi"}}"#
            ),
        )
        .unwrap();
        let session =
            parse_claude_jsonl(&jsonl, "/work/demo", &fake_process(1)).expect("应解析出会话");
        // "会话🔥x" 共 4 个字符，不足 8 位时整串即标题
        assert_eq!(session.title.as_deref(), Some("会话🔥x"));
    }
}
