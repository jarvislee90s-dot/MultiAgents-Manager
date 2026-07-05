use serde::Deserialize;
// JSONL 解析器 — Claude Code (message.role + content[]) 和 Codex CLI (type + payload)
// Claude 部分移植自 agent-sessions session/parser.rs

use crate::adapter::AgentProcess;
use crate::session::ProcessForm;
use crate::session::{AgentType, Session, SessionStatus};
use crate::session::model::JsonlMessage;
use super::status::*;
use log::{debug, info};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use once_cell::sync::Lazy;

static GIT_URL_CACHE: Lazy<Mutex<HashMap<String, Option<String>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// ===== 路径编码（Claude projects 目录名 <-> 实际路径）=====

/// 将路径转换为 Claude projects 目录名（如 /Users/x/proj -> -Users-x-proj）
pub fn convert_path_to_dir_name(path: &str) -> String {
    let path = path.strip_prefix('/').unwrap_or(path);
    let mut result = String::from("-");
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' => {
                if chars.peek() == Some(&'.') {
                    result.push('-');
                    result.push('-');
                    chars.next();
                } else {
                    result.push('-');
                }
            }
            _ => result.push(c),
        }
    }
    result
}

/// 将 Claude projects 目录名还原为路径
pub fn convert_dir_name_to_path(dir_name: &str) -> String {
    let name = dir_name.strip_prefix('-').unwrap_or(dir_name);
    let parts: Vec<&str> = name.split('-').collect();
    if parts.is_empty() {
        return String::new();
    }
    let projects_idx = parts.iter().position(|&p| p == "Projects" || p == "UnityProjects");
    if let Some(idx) = projects_idx {
        let path_parts = &parts[..=idx];
        let project_parts = &parts[idx + 1..];
        let mut path = String::from("/");
        path.push_str(&path_parts.join("/"));
        if !project_parts.is_empty() {
            path.push('/');
            let mut segments: Vec<String> = Vec::new();
            let mut current = String::new();
            let mut in_hidden = false;
            for part in project_parts {
                if part.is_empty() {
                    if !current.is_empty() {
                        segments.push(current);
                        current = String::new();
                    }
                    in_hidden = true;
                } else if in_hidden {
                    if current.is_empty() {
                        current = format!(".{}", part);
                    } else {
                        segments.push(current);
                        current = part.to_string();
                    }
                } else {
                    if current.is_empty() {
                        current = part.to_string();
                    } else {
                        current.push('-');
                        current.push_str(part);
                    }
                }
            }
            if !current.is_empty() {
                segments.push(current);
            }
            path.push_str(&segments.join("/"));
        }
        path
    } else {
        format!("/{}", name.replace('-', "/"))
    }
}

// ===== Git URL 缓存 =====

fn get_github_url(project_path: &str) -> Option<String> {
    {
        let cache = GIT_URL_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(project_path) {
            return cached.clone();
        }
    }
    let result = (|| {
        let output = Command::new("git").args(["remote", "get-url", "origin"])
            .current_dir(project_path).output().ok()?;
        if !output.status.success() { return None; }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(p) = url.strip_prefix("git@github.com:") {
            let p = p.strip_suffix(".git").unwrap_or(p);
            Some(format!("https://github.com/{}", p))
        } else if url.starts_with("https://github.com/") {
            Some(url.strip_suffix(".git").unwrap_or(&url).to_string())
        } else {
            None
        }
    })();
    GIT_URL_CACHE.lock().unwrap().insert(project_path.to_string(), result.clone());
    result
}

// ===== 通用辅助 =====

fn extract_cwd_from_jsonl(jsonl_path: &Path) -> Option<String> {
    let file = File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(20).flatten() {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(&line) {
            if let Some(cwd) = msg.cwd {
                if cwd.starts_with('/') { return Some(cwd); }
            }
        }
    }
    None
}

fn is_subagent_file(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str())
        .map(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        .unwrap_or(false)
}

fn count_active_subagents(project_dir: &Path, parent_session_id: &str) -> usize {
    use std::time::{Duration, SystemTime};
    let threshold = Duration::from_secs(30);
    let now = SystemTime::now();
    fs::read_dir(project_dir).into_iter().flatten().flatten()
        .filter(|e| is_subagent_file(&e.path()))
        .filter(|e| {
            e.metadata().and_then(|m| m.modified()).ok()
                .and_then(|m| now.duration_since(m).ok())
                .map(|d| d < threshold).unwrap_or(false)
        })
        .filter(|e| {
            let file = File::open(e.path()).ok();
            file.and_then(|f| {
                BufReader::new(f).lines().take(5).flatten()
                    .find_map(|line| serde_json::from_str::<JsonlMessage>(&line).ok())
                    .and_then(|m| m.session_id)
                    .map(|id| id == parent_session_id)
            }).unwrap_or(false)
        })
        .count()
}

fn get_recent_jsonl_files(project_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(project_dir).into_iter().flatten().flatten()
        .filter(|e| {
            let p = e.path();
            p.extension().map(|ext| ext == "jsonl").unwrap_or(false) && !is_subagent_file(&p)
        })
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((path, modified))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.into_iter().map(|(p, _)| p).collect()
}

// ===== Claude Code 会话解析 =====

/// 扫描 ~/.claude/projects，匹配运行中的 Claude 进程
pub fn get_claude_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();

    // cwd -> processes 映射
    let mut cwd_to_processes: HashMap<String, Vec<&AgentProcess>> = HashMap::new();
    let mut expected_dir_names: HashSet<String> = HashSet::new();
    for process in processes {
        if let Some(cwd) = &process.cwd {
            let cwd_str = cwd.to_string_lossy().to_string();
            expected_dir_names.insert(convert_path_to_dir_name(&cwd_str));
            cwd_to_processes.entry(cwd_str).or_default().push(process);
        }
    }

    let claude_dir = dirs::home_dir().map(|h| h.join(".claude").join("projects")).unwrap_or_default();
    if !claude_dir.exists() {
        return sessions;
    }

    if let Ok(entries) = fs::read_dir(&claude_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !expected_dir_names.contains(dir_name) { continue; }

            let jsonl_files = get_recent_jsonl_files(&path);
            if jsonl_files.is_empty() { continue; }

            let mut cwd_to_files: HashMap<String, Vec<PathBuf>> = HashMap::new();
            for f in &jsonl_files {
                let file_cwd = extract_cwd_from_jsonl(f)
                    .unwrap_or_else(|| convert_dir_name_to_path(dir_name));
                cwd_to_files.entry(file_cwd).or_default().push(f.clone());
            }

            for (project_path, files) in &cwd_to_files {
                if let Some(procs) = cwd_to_processes.get(project_path) {
                    for (idx, proc) in procs.iter().enumerate() {
                        if let Some(f) = files.get(idx) {
                            if let Some(mut session) = parse_claude_jsonl(f, project_path, proc) {
                                session.active_subagent_count = count_active_subagents(&path, &session.id);
                                sessions.push(session);
                            }
                        }
                    }
                }
            }
        }
    }

    info!("Claude: {} sessions from {} processes", sessions.len(), processes.len());
    sessions
}

/// 解析单个 Claude JSONL 文件
fn parse_claude_jsonl(jsonl_path: &Path, project_path: &str, process: &AgentProcess) -> Option<Session> {
    use std::time::SystemTime;

    let file_age_secs = jsonl_path.metadata().and_then(|m| m.modified()).ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32());
    let file_recently_modified = file_age_secs.map(|a| a < 3.0).unwrap_or(false);

    let file = File::open(jsonl_path).ok()?;
    let file_size = file.metadata().ok()?.len();
    let mut reader = BufReader::new(file);

    const TAIL_BYTES: u64 = 512 * 1024;
    if file_size > TAIL_BYTES {
        let _ = reader.seek(SeekFrom::End(-(TAIL_BYTES as i64)));
        let mut _partial = String::new();
        let _ = reader.read_line(&mut _partial);
    }

    let lines: Vec<_> = reader.lines().flatten().collect();
    let recent: Vec<_> = lines.iter().rev().take(500).collect();

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

    for line in &recent {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(line) {
            if session_id.is_none() { session_id = msg.session_id; }
            if git_branch.is_none() { git_branch = msg.git_branch; }
            if last_timestamp.is_none() { last_timestamp = msg.timestamp; }

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

            if session_id.is_some() && found_status { break; }
        }
    }

    // 找最后一条有文本的消息作为预览
    for line in &recent {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(&line) {
            if let Some(content) = &msg.message {
                if let Some(c) = &content.content {
                    let text = match c {
                        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                        serde_json::Value::Array(arr) => arr.iter().find_map(|v| {
                            v.get("text").and_then(|t| t.as_str()).filter(|s| !s.is_empty()).map(String::from)
                        }),
                        _ => None,
                    };
                    if text.is_some() { last_message = text; break; }
                }
            }
        }
    }

    let session_id = session_id?;
    let session_title = session_id[..session_id.len().min(12)].to_string();
    let status = if is_compacting {
        SessionStatus::Compacting
    } else {
        determine_status(last_msg_type.as_deref(), last_has_tool_use, last_has_tool_result,
                         last_is_local, last_is_interrupted, last_is_user_input, file_recently_modified)
    };

    let project_name = project_path.split('/').filter(|s| !s.is_empty()).last().unwrap_or("Unknown").to_string();
    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 { format!("{}...", m.chars().take(100).collect::<String>()) } else { m }
    });

    Some(Session {
        id: session_id,
        agent_type: AgentType::Claude,
        project_name,
        project_path: project_path.to_string(),
        git_branch,
        github_url: get_github_url(project_path),
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at: last_timestamp.unwrap_or_else(|| "Unknown".to_string()),
        pid: process.pid,
        cpu_usage: process.cpu_usage,
        active_subagent_count: 0,
        form: process.form,
        jump_supported: matches!(process.form, ProcessForm::Cli),
        title: Some(session_title),
    })
}

// ===== Codex CLI 会话解析 =====

/// Codex JSONL 条目
#[derive(Deserialize)]
struct CodexEntry {
    pub timestamp: Option<String>,
    #[serde(rename = "type")]
    entry_type: Option<String>,
    payload: Option<serde_json::Value>,
}

/// 扫描 ~/.codex/sessions，匹配运行中的 Codex 进程
/// 1. 按 cwd 匹配 CLI 进程 2. 未匹配的 APP 进程回退到最近会话文件
pub fn get_codex_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();

    let sessions_dir = dirs::home_dir().map(|h| h.join(".codex").join("sessions")).unwrap_or_default();
    if !sessions_dir.exists() {
        return sessions;
    }

    let jsonl_files = collect_codex_session_files(&sessions_dir);
    debug!("Codex: found {} session files", jsonl_files.len());

    // 解析所有会话文件，提取 cwd
    let parsed: Vec<(PathBuf, Option<Session>)> = jsonl_files.iter()
        .map(|f| (f.clone(), parse_codex_jsonl(f)))
        .collect();

    // cwd -> processes 映射（用于精确匹配）
    let mut cwd_to_processes: HashMap<String, Vec<&AgentProcess>> = HashMap::new();
    let mut unmatched_processes: Vec<&AgentProcess> = Vec::new();
    for process in processes {
        match &process.cwd {
            Some(cwd) => {
                let cwd_str = cwd.to_string_lossy().to_string();
                if cwd_str == "/" || cwd_str.is_empty() {
                    unmatched_processes.push(process);
                } else {
                    cwd_to_processes.entry(cwd_str).or_default().push(process);
                }
            }
            None => unmatched_processes.push(process),
        }
    }

    let mut matched_file_indices: HashSet<usize> = HashSet::new();

    // Phase 1: 按 cwd 精确匹配
    for (idx, (_, session_opt)) in parsed.iter().enumerate() {
        if let Some(session) = session_opt {
            if let Some(procs) = cwd_to_processes.get(&session.project_path) {
                if let Some(proc) = procs.first() {
                    let mut session = session.clone();
                    session.pid = proc.pid;
                    session.cpu_usage = proc.cpu_usage;
                    session.form = proc.form;
                    session.jump_supported = matches!(proc.form, ProcessForm::Cli);
                    sessions.push(session);
                    matched_file_indices.insert(idx);
                }
            }
        }
    }

    // Phase 2: 未匹配的进程（如 APP 形态 cwd="/")回退到最近的未匹配会话文件
    for process in &unmatched_processes {
        for (idx, (_, session_opt)) in parsed.iter().enumerate() {
            if matched_file_indices.contains(&idx) {
                continue;
            }
            if let Some(session) = session_opt {
                let mut session = session.clone();
                session.pid = process.pid;
                session.cpu_usage = process.cpu_usage;
                session.form = process.form;
                session.jump_supported = matches!(process.form, ProcessForm::Cli);
                sessions.push(session);
                matched_file_indices.insert(idx);
                break; // 每个未匹配进程只取一个会话
            }
        }
    }

    info!("Codex: {} sessions from {} processes", sessions.len(), processes.len());
    sessions
}

/// 递归收集 ~/.codex/sessions 下的 rollout-*.jsonl 文件（按修改时间倒序）
fn collect_codex_session_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    collect_codex_files_inner(dir, &mut files);
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.into_iter().map(|(p, _)| p).collect()
}

fn collect_codex_files_inner(dir: &Path, files: &mut Vec<(PathBuf, std::time::SystemTime)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_codex_files_inner(&path, files);
            } else if path.file_name().and_then(|n| n.to_str())
                .map(|n| n.starts_with("rollout") && n.ends_with(".jsonl")).unwrap_or(false)
            {
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    files.push((path, modified));
                }
            }
        }
    }
}

/// 解析单个 Codex JSONL 文件
fn parse_codex_jsonl(jsonl_path: &Path) -> Option<Session> {
    use std::time::SystemTime;

    let file_age = jsonl_path.metadata().and_then(|m| m.modified()).ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32());
    // Codex APP 单步工具调用之间可能 10-30s 无文件改动；3s 太短会误判为 Idle
    let file_recently_modified = file_age.map(|a| a < 60.0).unwrap_or(false);

    let file = File::open(jsonl_path).ok()?;
    let file_size = file.metadata().ok()?.len();
    let mut reader = BufReader::new(file);

    const TAIL_BYTES: u64 = 512 * 1024;
    if file_size > TAIL_BYTES {
        let _ = reader.seek(SeekFrom::End(-(TAIL_BYTES as i64)));
        let mut _partial = String::new();
        let _ = reader.read_line(&mut _partial);
    }

    let lines: Vec<_> = reader.lines().flatten().collect();
    let recent: Vec<_> = lines.iter().rev().take(500).collect();

    let mut session_id = None;
    let mut project_path = String::new();
    let mut last_message = None;
    let mut last_role = None;
    let mut last_entry_type: Option<String> = None;
    let mut last_has_tool_use = false;
    let mut last_timestamp: Option<String> = None;
    let mut found_status = false;

    for line in &recent {
        if let Ok(entry) = serde_json::from_str::<CodexEntry>(&line) {
            // 顶层 timestamp 作为最后活动时间（最近一条 entry）
            if last_timestamp.is_none() {
                if let Some(ts) = &entry.timestamp {
                    last_timestamp = Some(ts.clone());
                }
            }
            match entry.entry_type.as_deref() {
                Some("session_meta") => {
                    if session_id.is_none() {
                        session_id = entry.payload.as_ref()
                            .and_then(|p| p.get("id")).and_then(|v| v.as_str()).map(String::from);
                    }
                    if project_path.is_empty() {
                        project_path = entry.payload.as_ref()
                            .and_then(|p| p.get("cwd")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    }
                }
                Some("response_item") => {
                    if !found_status {
                        let payload = entry.payload.as_ref();
                        let role = payload.and_then(|p| p.get("role")).and_then(|v| v.as_str());
                        let content = payload.and_then(|p| p.get("content"));
                        if let Some(role) = role {
                            last_entry_type = Some("assistant".to_string()); // Codex 用 role 而非 type
                            last_role = Some(role.to_string());
                            if let Some(c) = content {
                                let has_content = match c {
                                    serde_json::Value::String(s) => !s.is_empty(),
                                    serde_json::Value::Array(arr) => !arr.is_empty(),
                                    _ => false,
                                };
                                if has_content {
                                    // Codex 的 type 字段: response_item 中的 payload 有 type
                                    let item_type = payload.and_then(|p| p.get("type")).and_then(|v| v.as_str());
                                    last_entry_type = Some(item_type.unwrap_or(role).to_string());
                                    last_has_tool_use = has_tool_use(c);
                                    found_status = true;
                                }
                            }
                        }
                    }
                    // 找最后一条文本消息
                    if last_message.is_none() {
                        let payload = entry.payload.as_ref();
                        let content = payload.and_then(|p| p.get("content"));
                        if let Some(c) = content {
                            let text = match c {
                                serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                                serde_json::Value::Array(arr) => arr.iter().find_map(|v| {
                                    v.get("text").and_then(|t| t.as_str()).filter(|s| !s.is_empty()).map(String::from)
                                }),
                                _ => None,
                            };
                            if text.is_some() { last_message = text; }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let session_id = session_id?;
    if project_path.is_empty() { return None; }

    // Codex 状态判断：复用 determine_status 的逻辑
    // response_item with role=assistant + tool_use -> Processing
    // response_item with role=assistant + text -> Waiting
    // response_item with role=user -> Thinking
    let msg_type: Option<&str> = match last_role.as_deref() {
        Some("assistant") => Some("assistant"),
        Some("user") => Some("user"),
        _ => last_entry_type.as_deref(),
    };
    let status = determine_status(
        msg_type, last_has_tool_use, false, false, false, false, file_recently_modified
    );

    let project_name = project_path.split('/').filter(|s| !s.is_empty()).last().unwrap_or("Unknown").to_string();
    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 { format!("{}...", m.chars().take(100).collect::<String>()) } else { m }
    });

    let codex_title = session_id[..session_id.len().min(12)].to_string();
    Some(Session {
        id: session_id,
        agent_type: AgentType::Codex,
        project_name,
        project_path: project_path.clone(),
        git_branch: None,
        github_url: get_github_url(&project_path),
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at: last_timestamp.unwrap_or_else(|| "Unknown".to_string()),
        pid: 0, // 由调用方设置
        cpu_usage: 0.0,
        active_subagent_count: 0,
        form: ProcessForm::Cli, // 由调用方设置
        jump_supported: true,
        title: Some(codex_title),
    })
}
