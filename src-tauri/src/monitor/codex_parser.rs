// Codex CLI 会话解析 — type + payload 协议（rollout-*.jsonl）
// 公共设施（cwd 归一化、git URL 缓存、JSONL 尾部读取）见 monitor::{cwd,git,jsonl,project}

use super::cwd::normalize_cwd_for_match;
use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use super::status::*;
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, ProcessForm, Session};
use log::{debug, info};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const RECENT_LINES: usize = 500;

/// Codex JSONL 条目
#[derive(Deserialize)]
struct CodexEntry {
    pub timestamp: Option<String>,
    #[serde(rename = "type")]
    entry_type: Option<String>,
    payload: Option<serde_json::Value>,
}

/// APP 形态每会话一卡（spec W4 通用规则的 Codex 落地）：
/// 输入已按 mtime 倒序的 (文件, 会话) 与对应 mtime，取未被 CLI 认领的、24h 内有更新
/// 的文件，按 sessionId 聚合（同会话多个 rollout 取最新），宿主 App 进程在场才出卡
pub fn aggregate_app_sessions(
    parsed: &[(PathBuf, Option<Session>)],
    mtimes: &[std::time::SystemTime],
    app_processes: &[AgentProcess],
) -> Vec<Session> {
    use std::collections::HashMap;

    let Some(host) = app_processes.first() else {
        return Vec::new();
    };
    let now = std::time::SystemTime::now();
    let window = std::time::Duration::from_secs(24 * 3600);

    // sessionId → (mtime, session)，保留同会话 mtime 最新者
    let mut by_session: HashMap<String, (std::time::SystemTime, Session)> = HashMap::new();
    for ((_, session_opt), mtime) in parsed.iter().zip(mtimes.iter()) {
        let Some(session) = session_opt else { continue };
        // 该文件已被 CLI 进程认领的判定由调用方通过 parsed 子集传入（见 get_codex_sessions）
        let fresh = now
            .duration_since(*mtime)
            .map(|d| d < window)
            .unwrap_or(false);
        if !fresh {
            continue;
        }
        by_session
            .entry(session.id.clone())
            .and_modify(|e| {
                if *mtime > e.0 {
                    *e = (*mtime, session.clone());
                }
            })
            .or_insert_with(|| (*mtime, session.clone()));
    }

    by_session
        .into_values()
        .map(|(_, mut s)| {
            s.pid = host.pid;
            s.cpu_usage = host.cpu_usage;
            s.form = ProcessForm::App;
            s.jump_supported = jump_supported_for(ProcessForm::App);
            s.github_url = get_github_url(&s.project_path);
            s
        })
        .collect()
}

/// 扫描 ~/.codex/sessions，匹配运行中的 Codex 进程
/// 1. 按 cwd 匹配 CLI 进程 2. 未被认领的近期 rollout 按 sessionId 聚合为 APP 卡
pub fn get_codex_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();

    let sessions_dir = dirs::home_dir()
        .map(|h| h.join(".codex").join("sessions"))
        .unwrap_or_default();
    if !sessions_dir.exists() {
        return sessions;
    }

    let jsonl_files = collect_codex_session_files(&sessions_dir);
    debug!("Codex: found {} session files", jsonl_files.len());

    // 解析所有会话文件，提取 cwd（用 Cli 默认值获取 project_path 用于匹配）
    let parsed: Vec<(PathBuf, Option<Session>)> = jsonl_files
        .iter()
        .map(|f| (f.clone(), parse_codex_jsonl(f, ProcessForm::Cli)))
        .collect();

    let mut matched_file_indices: HashSet<usize> = HashSet::new();

    // Phase 1: 按 cwd 精确匹配——每个进程一张卡，取目录匹配中 mtime 最新的 rollout。
    // codex CLI 每轮对话写新 rollout（session_id 变），若按文件循环会把同一进程
    // 挂成多张卡（用户实测同一窗口被识别为重复会话）
    for process in processes {
        let Some(cwd) = &process.cwd else { continue };
        let normalized = normalize_cwd_for_match(&cwd.to_string_lossy());
        if normalized.is_empty() {
            continue;
        }
        // parsed 按 mtime 倒序，找第一个目录匹配且未被其他进程占用的文件
        for (idx, (file_path, session_opt)) in parsed.iter().enumerate() {
            if matched_file_indices.contains(&idx) {
                continue;
            }
            let Some(session) = session_opt else { continue };
            if normalize_cwd_for_match(&session.project_path) != normalized {
                continue;
            }
            // 用实际 process_form 重新解析以获取正确状态
            let mut session =
                parse_codex_jsonl(file_path, process.form).unwrap_or_else(|| session.clone());
            session.pid = process.pid;
            session.cpu_usage = process.cpu_usage;
            session.form = process.form;
            session.jump_supported = jump_supported_for(process.form);
            session.github_url = get_github_url(&session.project_path);
            sessions.push(session);
            matched_file_indices.insert(idx);
            break; // 每进程只取最新一个
        }
    }

    // Phase 2（W4 每会话一卡）：未被 CLI 认领的近期 rollout 按 sessionId 聚合，
    // 每会话一张卡（宿主 App 进程在场才出活跃卡；完成转绿的持久未读由 DB 管线合并）
    let app_processes: Vec<AgentProcess> = processes
        .iter()
        .filter(|p| matches!(p.form, ProcessForm::App))
        .cloned()
        .collect();
    if !app_processes.is_empty() {
        // CLI 认领 = Phase 1 已占用；剩余文件进入聚合
        let mtimes: Vec<std::time::SystemTime> = jsonl_files
            .iter()
            .map(|f| {
                f.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
            .collect();
        let unclaimed: Vec<(PathBuf, Option<Session>)> = parsed
            .iter()
            .enumerate()
            .filter(|(idx, _)| !matched_file_indices.contains(idx))
            .map(|(_, pair)| pair.clone())
            .collect();
        let unclaimed_mtimes: Vec<std::time::SystemTime> = parsed
            .iter()
            .enumerate()
            .filter(|(idx, _)| !matched_file_indices.contains(idx))
            .filter_map(|(idx, _)| mtimes.get(idx).copied())
            .collect();
        sessions.extend(aggregate_app_sessions(
            &unclaimed,
            &unclaimed_mtimes,
            &app_processes,
        ));
    }

    info!(
        "Codex: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// 递归收集 ~/.codex/sessions 下的 rollout-*.jsonl 文件（按修改时间倒序）
fn collect_codex_session_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    collect_codex_files_inner(dir, &mut files);
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files.into_iter().map(|(p, _)| p).collect()
}

fn collect_codex_files_inner(dir: &Path, files: &mut Vec<(PathBuf, std::time::SystemTime)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_codex_files_inner(&path, files);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("rollout") && n.ends_with(".jsonl"))
                .unwrap_or(false)
            {
                if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                    files.push((path, modified));
                }
            }
        }
    }
}

/// 解析单个 Codex JSONL 文件
fn parse_codex_jsonl(jsonl_path: &Path, process_form: ProcessForm) -> Option<Session> {
    use std::time::SystemTime;

    let file_age = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32());
    // Codex APP 单步工具调用之间可能 10-30s 无文件改动；60s 对 APP 太短
    // APP 形态使用更大的阈值（300s = 5分钟），CLI 保持 60s
    let file_recently_modified = match process_form {
        ProcessForm::App => file_age.map(|a| a < 300.0).unwrap_or(false),
        ProcessForm::Cli => file_age.map(|a| a < 60.0).unwrap_or(false),
    };

    let recent = read_recent_lines(jsonl_path, RECENT_LINES);

    let mut session_id = None;
    let mut project_path = String::new();
    let mut last_message = None;
    let mut last_role = None;
    let mut last_entry_type: Option<String> = None;
    let mut last_has_tool_use = false;
    let mut last_timestamp: Option<String> = None;
    let mut found_status = false;

    for line in recent.iter().rev() {
        if let Ok(entry) = serde_json::from_str::<CodexEntry>(line) {
            // 顶层 timestamp 作为最后活动时间（最近一条 entry）
            if last_timestamp.is_none() {
                if let Some(ts) = &entry.timestamp {
                    last_timestamp = Some(ts.clone());
                }
            }
            match entry.entry_type.as_deref() {
                Some("session_meta") => {
                    if session_id.is_none() {
                        session_id = entry
                            .payload
                            .as_ref()
                            .and_then(|p| p.get("id"))
                            .and_then(|v| v.as_str())
                            .map(String::from);
                    }
                    if project_path.is_empty() {
                        project_path = entry
                            .payload
                            .as_ref()
                            .and_then(|p| p.get("cwd"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
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
                                    let item_type = payload
                                        .and_then(|p| p.get("type"))
                                        .and_then(|v| v.as_str());
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
                                    v.get("text")
                                        .and_then(|t| t.as_str())
                                        .filter(|s| !s.is_empty())
                                        .map(String::from)
                                }),
                                _ => None,
                            };
                            if text.is_some() {
                                last_message = text;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let session_id = session_id?;
    if project_path.is_empty() {
        return None;
    }

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
        msg_type,
        last_has_tool_use,
        false,
        false,
        false,
        false,
        file_recently_modified,
    );

    let project_name = project_name_from_path(&project_path);
    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 {
            format!("{}...", m.chars().take(100).collect::<String>())
        } else {
            m
        }
    });

    // 卡片前缀统一 8 位（按字符截取，多字节 id 不 panic），与 hook marker（MAM:<id 前 8 位>）保持一致
    let codex_title = session_id.chars().take(8).collect::<String>();
    Some(Session {
        id: session_id,
        agent_type: AgentType::Codex,
        project_name,
        project_path: project_path.clone(),
        git_branch: None,
        github_url: None, // 延迟到进程匹配后填充（见 get_codex_sessions），避免批量解析时风暴式 spawn git
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at: last_timestamp.unwrap_or_else(|| "Unknown".to_string()),
        pid: 0, // 由调用方设置
        cpu_usage: 0.0,
        active_subagent_count: 0,
        form: ProcessForm::Cli,                               // 由调用方设置
        jump_supported: jump_supported_for(ProcessForm::Cli), // 由调用方按进程形态覆盖
        unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        title: Some(codex_title),
    })
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;
    use crate::session::SessionStatus;

    fn mk(id: &str, proj: &str, title: Option<String>) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::Codex,
            project_name: proj.into(),
            project_path: format!("/tmp/{}", proj),
            title,
            git_branch: None,
            github_url: None,
            status: SessionStatus::Idle,
            last_message: None,
            last_message_role: None,
            last_activity_at: String::new(),
            pid: 0,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: true,
            unread: false,
        }
    }

    #[test]
    fn aggregate_groups_by_session_id_and_picks_latest() {
        // mtime 全部取过去 1h / 1min（实现按 now 起 24h 新鲜窗口过滤，UNIX_EPOCH 会被判过期）
        let now = std::time::SystemTime::now();
        let hour_ago = now
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or(now);
        let min_ago = now
            .checked_sub(std::time::Duration::from_secs(60))
            .unwrap_or(now);
        // s1 两个 rollout，用 title 区分：最新文件（1min 前）应胜出
        let parsed = vec![
            (
                PathBuf::from("/a-rollout-s1-old"),
                Some(mk("s1", "P1", Some("old".into()))),
            ),
            (
                PathBuf::from("/b-rollout-s1-new"),
                Some(mk("s1", "P1", Some("new".into()))),
            ),
            (PathBuf::from("/c-rollout-s2"), Some(mk("s2", "P2", None))),
        ];
        // s1 最新文件是第 2 个（mtime 更大）
        let mtimes = vec![hour_ago, min_ago, hour_ago];
        let host = vec![AgentProcess {
            pid: 100,
            cpu_usage: 0.0,
            cwd: None,
            form: ProcessForm::App,
        }];
        let out = aggregate_app_sessions(&parsed, &mtimes, &host);
        // 按 sessionId 聚合：s1 + s2 各一张，无重复
        assert_eq!(out.len(), 2);
        assert!(out.iter().any(|s| s.id == "s1"));
        assert!(out.iter().any(|s| s.id == "s2"));
        // 同会话多 rollout 取 mtime 最新者
        assert_eq!(
            out.iter().find(|s| s.id == "s1").unwrap().title.as_deref(),
            Some("new")
        );
        // 宿主在场时卡归 App 形态、pid/cpu 取宿主进程
        assert!(out.iter().all(|s| matches!(s.form, ProcessForm::App)));
        assert!(out.iter().all(|s| s.pid == 100));
    }

    #[test]
    fn aggregate_skips_matched_files_and_requires_host() {
        let parsed = vec![(PathBuf::from("/x"), Some(mk("s1", "P", None)))];
        let base = std::time::SystemTime::UNIX_EPOCH;
        // 24h 窗口外 → 不出卡
        let old = base + std::time::Duration::from_secs(1);
        let now = std::time::SystemTime::now();
        let hour_ago = now
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or(now);
        assert!(aggregate_app_sessions(&parsed, &[old], &[]).is_empty());
        // 宿主进程不存在 → 不出卡（活跃卡需宿主存活；持久未读由 DB 合并管线负责）
        assert!(aggregate_app_sessions(&parsed, &[hour_ago], &[]).is_empty());
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;

    /// sessionId 含多字节字符时，标题回退按字符取前 8 位，不得 panic
    #[test]
    fn multibyte_session_id_title_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let jsonl = tmp.path().join("rollout-2026-01-01.jsonl");
        std::fs::write(
            &jsonl,
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"会话🔥x","cwd":"/work/demo"}}"#,
        )
        .unwrap();
        let session = parse_codex_jsonl(&jsonl, ProcessForm::Cli).expect("应解析出会话");
        assert_eq!(session.title.as_deref(), Some("会话🔥x"));
    }
}
