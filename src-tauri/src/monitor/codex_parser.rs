// Codex CLI 会话解析 — type + payload 协议（rollout-*.jsonl）
// 公共设施（cwd 归一化、git URL 缓存、JSONL 尾部读取）见 monitor::{cwd,git,jsonl,project}

use super::app_status::{derive_app_status, overlay_mtime_stale, AppEntryKind};
use super::cwd::normalize_cwd_for_match;
use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, ProcessForm, Session, SessionStatus};
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

/// Codex 条目 → 归一化 APP 条目（issue #6 格式翻译适配器）：
/// 解包 response_item / event_msg 外壳，映射到共享判定核（monitor::app_status）的
/// AppEntryKind。修复点：function_call / function_call_output 等不带 role 的条目
/// 旧实现全被跳过（第二轮从第一条 assistant message 落盘起「最后带 role 的条目」
/// 恒为 assistant 纯文本 → 恒判 Idle），现在工具调用条目参与判定；
/// reasoning / token_count / item_completed 等记账条目 → Other（跳过，不参与判定）
fn codex_entry_kind(entry: &CodexEntry) -> AppEntryKind {
    let Some(payload) = entry.payload.as_ref() else {
        return AppEntryKind::Other;
    };
    match entry.entry_type.as_deref() {
        Some("response_item") => match payload.get("type").and_then(|v| v.as_str()) {
            Some("message") => match payload.get("role").and_then(|v| v.as_str()) {
                Some("user") => AppEntryKind::UserMessage,
                Some("assistant") => AppEntryKind::AssistantMessage,
                _ => AppEntryKind::Other, // developer 等系统角色不参与判定
            },
            Some("function_call") | Some("function_call_output") => AppEntryKind::ToolCall,
            _ => AppEntryKind::Other, // reasoning 等记账条目
        },
        Some("event_msg") => match payload.get("type").and_then(|v| v.as_str()) {
            Some("task_started") => AppEntryKind::TurnStart,
            Some("task_complete") => AppEntryKind::TurnEnd,
            _ => AppEntryKind::Other, // token_count / item_completed / thread_settings_applied 等
        },
        _ => AppEntryKind::Other, // session_meta / token_usage_record / world_state / turn_context
    }
}

/// APP 形态每会话一卡（spec W4 通用规则的 Codex 落地）：
/// 输入已按 mtime 倒序的 (文件, 会话) 与对应 mtime，取未被 CLI 认领的、24h 内有更新
/// 的文件，按 sessionId 聚合（同会话多个 rollout 取最新），宿主 App 进程在场才出卡
/// review F2：聚合宿主 = exe 匹配 monitor::host::is_host_process 的 App 进程
/// （ChatGPT 主进程 / chatgpt.exe）；CLI 同名 exe、内嵌框架进程不算宿主，
/// 防止孤儿框架进程让聚合卡在宿主死后继续出现
fn codex_host_process(app_processes: &[AgentProcess]) -> Option<&AgentProcess> {
    app_processes.iter().find(|p| {
        p.exe
            .as_ref()
            .map(|e| {
                crate::monitor::host::is_host_process(&e.to_string_lossy().to_lowercase(), "codex")
            })
            .unwrap_or(false)
    })
}

pub fn aggregate_app_sessions(
    parsed: &[(PathBuf, Option<Session>)],
    mtimes: &[std::time::SystemTime],
    host: &AgentProcess,
) -> Vec<Session> {
    use std::collections::HashMap;

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
    // review F2：宿主判定口径与 monitor::host::is_host_process 一致
    // （ChatGPT 主进程/chatgpt.exe 才算宿主；CLI 同名 exe、内嵌框架进程不算）
    if let Some(host) = codex_host_process(&app_processes) {
        // CLI 认领 = Phase 1 已占用；剩余文件进入聚合。
        // review F6：24h 新鲜过滤前移到二次 parse 之前——历史 rollout（通常占绝大多数）
        // 不再重复尾读，只有窗口内的文件付出 App 形态重解析（300s 阈值语义）的 IO
        let now = std::time::SystemTime::now();
        let mtimes: Vec<std::time::SystemTime> = jsonl_files
            .iter()
            .map(|f| {
                f.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            })
            .collect();
        let fresh = fresh_unclaimed_files(&jsonl_files, &mtimes, &matched_file_indices, now);
        let unclaimed = unclaimed_app_parsed(&fresh);
        let unclaimed_mtimes: Vec<std::time::SystemTime> = fresh.iter().map(|(_, m)| *m).collect();
        sessions.extend(aggregate_app_sessions(&unclaimed, &unclaimed_mtimes, host));
    }

    info!(
        "Codex: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// Phase 2 聚合输入构建：未被 CLI 认领且 24h 窗口内（已由 fresh_within_24h 前移过滤）
/// 的 rollout 文件，按 APP 形态重新解析。旧实现（Task 10 重构前）会按实际进程形态
/// 重解析，重构时丢失导致 APP 卡沿用 CLI 的 60s mtime 阈值——工具调用尾部停更
/// 60-300s 被误判 Waiting（应为 Processing）。仅窗口内文件付出有界尾读（500 行）
fn unclaimed_app_parsed(
    fresh_files: &[(PathBuf, std::time::SystemTime)],
) -> Vec<(PathBuf, Option<Session>)> {
    fresh_files
        .iter()
        .map(|(f, _)| (f.clone(), parse_codex_jsonl(f, ProcessForm::App)))
        .collect()
}

/// 未认领候选中 24h 窗口内的文件（review F6：纯函数，过滤前移到二次 parse 之前，
/// 历史 rollout 不再重复尾读；mtimes 经参数注入，测试无需真实文件）
fn fresh_unclaimed_files(
    jsonl_files: &[PathBuf],
    mtimes: &[std::time::SystemTime],
    matched: &HashSet<usize>,
    now: std::time::SystemTime,
) -> Vec<(PathBuf, std::time::SystemTime)> {
    let window = std::time::Duration::from_secs(24 * 3600);
    jsonl_files
        .iter()
        .enumerate()
        .filter(|(idx, _)| !matched.contains(idx))
        .filter_map(|(idx, f)| mtimes.get(idx).copied().map(|m| (f.clone(), m)))
        .filter(|(_, mtime)| {
            now.duration_since(*mtime)
                .map(|age| age < window)
                .unwrap_or(false)
        })
        .collect()
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

    // 文件年龄（秒）供共享核的 300s mtime 叠加使用；mtime 不可知 → None（按未过期处理）
    let file_age_secs = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32());

    let recent = read_recent_lines(jsonl_path, RECENT_LINES);

    let mut session_id = None;
    let mut project_path = String::new();
    let mut last_message = None;
    let mut last_role = None;
    let mut last_timestamp: Option<String> = None;
    // 归一化条目序列（文件顺序），供共享判定核尾部倒扫
    let mut kinds: Vec<AppEntryKind> = Vec::new();
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
                    // 找最后一条文本消息（含 role 记录，展示用 last_message_role；
                    // 与旧实现一致：仅在有内容的消息上记录角色）
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
                                last_role = payload
                                    .and_then(|p| p.get("role"))
                                    .and_then(|v| v.as_str())
                                    .map(String::from);
                            }
                        }
                    }
                    // 记录归一化条目（含 role 的 message 与不带 role 的 function_call 等）
                    kinds.push(codex_entry_kind(&entry));
                }
                Some("event_msg") => {
                    // task_started / task_complete 参与判定；token_count / item_completed 等记账条目 → Other
                    kinds.push(codex_entry_kind(&entry));
                }
                _ => {}
            }
        }
    }
    // 倒扫时按文件顺序 push，此处反转回文件顺序（旧 → 新）供共享核尾部倒扫
    kinds.reverse();

    let session_id = session_id?;
    if project_path.is_empty() {
        return None;
    }

    // 状态判定（issue #6）：共享核尾部倒扫取第一条有语义条目——
    // user → Thinking；assistant 纯文本 → Idle；function_call/function_call_output →
    // Processing；task_started → Processing；task_complete → Idle；记账条目跳过。
    // 无任何语义条目（如仅 session_meta 的 rollout）→ None，兜底按形态：
    // APP 形态文件新鲜（<300s）→ Processing，停更 → Waiting；CLI 保持 60s 阈值
    let status = match derive_app_status(&kinds) {
        Some(status) => status,
        None => {
            let fresh = match process_form {
                ProcessForm::App => file_age_secs.map(|a| a < 300.0).unwrap_or(false),
                ProcessForm::Cli => file_age_secs.map(|a| a < 60.0).unwrap_or(false),
            };
            if fresh {
                SessionStatus::Processing
            } else {
                SessionStatus::Waiting
            }
        }
    };
    // 叠加 300s 规则：Processing 且 JSONL mtime 停更 >= 300s → Waiting（与 WorkBuddy 一致）
    let status = overlay_mtime_stale(status, file_age_secs.map_or(0, |a| (a * 1000.0) as u64));

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
        let host = AgentProcess {
            pid: 100,
            cpu_usage: 0.0,
            cwd: None,
            exe: Some("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".into()),
            form: ProcessForm::App,
        };
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
        let host = AgentProcess {
            pid: 100,
            cpu_usage: 0.0,
            cwd: None,
            exe: Some("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".into()),
            form: ProcessForm::App,
        };
        // 24h 窗口外 → 不出卡（宿主缺席不出卡的口径由 codex_host_criterion 测试锁定）
        assert!(aggregate_app_sessions(&parsed, &[old], &host).is_empty());
        let _ = hour_ago;
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

#[cfg(test)]
mod phase2_app_form_tests {
    use super::*;
    use crate::session::SessionStatus;

    /// 夹具：仅 session_meta 的 rollout（无任何语义条目 → 共享核返回 None，走形态兜底），
    /// mtime 拨到 120s 前——落在 CLI 60s 与 APP 300s 阈值之间，可区分两种形态语义
    fn write_meta_only_rollout(dir: &Path) -> PathBuf {
        let path = dir.join("rollout-2026-01-01t00-00-00.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{"id":"meta-only-s1","cwd":"/work/demo"}}"#,
        )
        .unwrap();
        let stale = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(120))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stale)
            .unwrap();
        path
    }

    /// 夹具区分度自检：同一文件，Cli 形态（60s）判 Waiting，App 形态（300s）判 Processing
    #[test]
    fn fixture_discriminates_cli_vs_app_mtime_thresholds() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_meta_only_rollout(tmp.path());
        let cli = parse_codex_jsonl(&f, ProcessForm::Cli).unwrap();
        assert_eq!(cli.status, SessionStatus::Waiting);
        let app = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(app.status, SessionStatus::Processing);
    }

    /// Phase 2 聚合输入必须按 App 形态解析（spec §4/§7）：未认领文件 60-300s 停更
    /// 的工具调用尾部在 APP 卡上应为 Processing（红）而非 Waiting（黄）
    #[test]
    fn unclaimed_aggregate_input_is_parsed_with_app_form() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_meta_only_rollout(tmp.path());
        let parsed = unclaimed_app_parsed(&[(f, std::time::SystemTime::now())]);
        assert_eq!(parsed.len(), 1);
        let session = parsed[0].1.as_ref().unwrap();
        assert_eq!(
            session.status,
            SessionStatus::Processing,
            "聚合输入按 Cli 60s 阈值解析会把 APP 卡误判为 Waiting"
        );
    }

    /// review F6：24h 新鲜过滤必须前移到二次 parse 之前——
    /// 历史 rollout（mtime 超窗）不得进入重解析，消除重复尾读 IO
    #[test]
    fn fresh_filter_drops_stale_rollouts_before_reparse() {
        let now = std::time::SystemTime::now();
        let fresh = now - std::time::Duration::from_secs(3600);
        let stale = now - std::time::Duration::from_secs(25 * 3600);
        let files = vec![
            PathBuf::from("/a-rollout-fresh"),
            PathBuf::from("/b-rollout-stale"),
        ];
        let mtimes = vec![fresh, stale];
        let out = fresh_unclaimed_files(&files, &mtimes, &HashSet::new(), now);
        assert_eq!(
            out,
            vec![(PathBuf::from("/a-rollout-fresh"), fresh)],
            "超窗文件必须在二次 parse 前被过滤"
        );
    }

    /// review F2：聚合宿主判定口径与 monitor::host::is_host_process 一致——
    /// ChatGPT 主进程可作宿主；CLI 同名 exe / 内嵌框架进程不得算宿主
    #[test]
    fn codex_host_criterion_matches_is_host_process() {
        let chatgpt_main = AgentProcess {
            pid: 1,
            cpu_usage: 0.0,
            cwd: None,
            exe: Some("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT".into()),
            form: ProcessForm::App,
        };
        let cli_like = AgentProcess {
            pid: 2,
            cpu_usage: 0.0,
            cwd: None,
            exe: Some("/usr/local/bin/codex".into()),
            form: ProcessForm::App,
        };
        let framework = AgentProcess {
            pid: 3,
            cpu_usage: 0.0,
            cwd: None,
            exe: Some(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex.framework/Versions/A/Codex"
                    .into(),
            ),
            form: ProcessForm::App,
        };
        assert_eq!(
            codex_host_process(&[cli_like.clone(), chatgpt_main]).map(|p| p.pid),
            Some(1),
            "宿主 = exe 匹配 is_host_process 的 App 进程"
        );
        assert!(
            codex_host_process(&[cli_like, framework]).is_none(),
            "CLI 同名 exe 与内嵌框架进程都不是宿主"
        );
    }

    /// 已被 CLI 认领的文件不得进入聚合输入（Phase 1 / Phase 2 不重复出卡）
    #[test]
    fn claimed_files_are_excluded_from_aggregate_input() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_meta_only_rollout(tmp.path());
        let claimed: HashSet<usize> = HashSet::from([0]);
        let files = vec![f, PathBuf::from("/other-fresh")];
        let now = std::time::SystemTime::now();
        let mtimes = vec![now, now];
        assert_eq!(
            fresh_unclaimed_files(&files, &mtimes, &claimed, now),
            vec![(PathBuf::from("/other-fresh"), now)],
            "被认领索引排除、未认领新鲜文件存活"
        );
    }
}

#[cfg(test)]
mod app_status_fixture_tests {
    use super::*;

    /// 真实 rollout 脱敏片段（issue #6 实测样本
    /// `~/.codex/sessions/2026/09/06/rollout-2026-09-06T13-40-16-01a0753b-*.jsonl`）：
    /// 同一会话文件内含两轮（05:40:24-53 与 05:41:19-43），session_id 不变，
    /// 尾部序列 user → reasoning → assistant message → function_call* → … → task_complete。
    /// 字段按真实结构保留（type/payload.type/role/content/name），内容脱敏
    const SESSION_ID: &str = "01a0753b-4fa9-7ab1-a245-7972b9ef2e41";
    // 解析后的 cwd 值（单反斜杠）；嵌入 JSON 时经 json_escape 转义（真实 rollout 中
    // 原始文本为 "E:\\LLMproject\\..." 形态）
    const CWD: &str = "E:\\LLMproject\\demo";

    /// JSON 字符串值转义（反斜杠 → \\），供 format! 拼 JSON 文本使用
    fn json_escape(s: &str) -> String {
        s.replace('\\', "\\\\")
    }

    fn meta() -> String {
        format!(
            r#"{{"timestamp":"2026-09-06T05:40:24.228Z","ordinal":0,"type":"session_meta","payload":{{"id":"{SESSION_ID}","cwd":"{}"}}}}"#,
            json_escape(CWD)
        )
    }

    fn task_started(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"event_msg","payload":{{"type":"task_started","turn_id":"t1"}}}}"#
        )
    }

    fn user_msg(ts: &str, ordinal: u32, text: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{text}"}}]}}}}"#
        )
    }

    fn assistant_msg(ts: &str, ordinal: u32, text: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"{text}"}}]}}}}"#
        )
    }

    fn function_call(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"response_item","payload":{{"type":"function_call","id":"fc_1","name":"exec_command","arguments":"{{\"cmd\":\"git status\"}}"}}}}"#
        )
    }

    fn function_call_output(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"response_item","payload":{{"type":"function_call_output","call_id":"call_1","output":"ok"}}}}"#
        )
    }

    fn reasoning(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"response_item","payload":{{"type":"reasoning","summary":[]}}}}"#
        )
    }

    fn token_count(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"event_msg","payload":{{"type":"token_count","info":{{}}}}}}"#
        )
    }

    fn item_completed(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"event_msg","payload":{{"type":"item_completed","thread_id":"{SESSION_ID}"}}}}"#
        )
    }

    fn task_complete(ts: &str, ordinal: u32) -> String {
        format!(
            r#"{{"timestamp":"{ts}","ordinal":{ordinal},"type":"event_msg","payload":{{"type":"task_complete","turn_id":"t1"}}}}"#
        )
    }

    fn write_rollout(dir: &Path, lines: &[String]) -> PathBuf {
        let path =
            dir.join("rollout-2026-09-06T13-40-16-01a0753b-4fa9-7ab1-a245-7972b9ef2e41.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    /// 第一轮完整序列（user → assistant → function_call → output → task_complete）
    fn round_one() -> Vec<String> {
        vec![
            meta(),
            task_started("2026-09-06T05:40:24.228Z", 1),
            user_msg("2026-09-06T05:40:30.320Z", 2, "检查仓库状态"),
            reasoning("2026-09-06T05:40:34.579Z", 3),
            assistant_msg("2026-09-06T05:40:34.952Z", 4, "我先看一下"),
            function_call("2026-09-06T05:40:34.953Z", 5),
            function_call_output("2026-09-06T05:40:35.831Z", 6),
            token_count("2026-09-06T05:40:35.887Z", 7),
            task_complete("2026-09-06T05:40:53.976Z", 8),
        ]
    }

    /// 第二轮运行期序列（追加写入同一文件：user → reasoning → assistant → function_call*）
    fn round_two_running() -> Vec<String> {
        vec![
            meta(),
            user_msg("2026-09-06T05:41:25.003Z", 9, "同步到本地"),
            reasoning("2026-09-06T05:41:27.626Z", 10),
            assistant_msg("2026-09-06T05:41:28.121Z", 11, "开始同步"),
            function_call("2026-09-06T05:41:28.122Z", 12),
            function_call("2026-09-06T05:41:28.123Z", 13),
            function_call_output("2026-09-06T05:41:28.631Z", 14),
            token_count("2026-09-06T05:41:28.679Z", 15),
        ]
    }

    /// 第二轮完成序列（追加 task_complete 收尾）
    fn round_two_complete() -> Vec<String> {
        let mut lines = round_two_running();
        lines.push(assistant_msg("2026-09-06T05:41:43.138Z", 16, "同步完成"));
        lines.push(token_count("2026-09-06T05:41:43.140Z", 17));
        lines.push(task_complete("2026-09-06T05:41:43.151Z", 18));
        lines
    }

    /// 夹具自检：真实结构片段能解析出会话（session_id / cwd 正确）
    #[test]
    fn fixture_parses_session_meta() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout(tmp.path(), &round_one());
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.id, SESSION_ID);
        assert_eq!(session.project_path, CWD);
    }

    /// 关键判定 1：function_call 在尾 → Processing（第二轮运行期可见，issue #6 修复点）
    #[test]
    fn function_call_tail_is_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout(tmp.path(), &round_two_running());
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(
            session.status,
            SessionStatus::Processing,
            "第二轮工具执行期必须显示运行中（旧实现恒判 Idle）"
        );
    }

    /// 关键判定 2：task_complete / assistant message 在尾 → Idle
    #[test]
    fn task_complete_tail_is_idle() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout(tmp.path(), &round_one());
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Idle);
    }

    #[test]
    fn assistant_message_tail_is_idle() {
        let tmp = tempfile::tempdir().unwrap();
        // 无 task_complete 的尾部（assistant 纯文本收尾）同样判 Idle
        let mut lines = round_two_running();
        lines.push(assistant_msg("2026-09-06T05:41:43.138Z", 16, "同步完成"));
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Idle);
    }

    /// 关键判定 3：user message 在尾 → Thinking
    #[test]
    fn user_message_tail_is_thinking() {
        let tmp = tempfile::tempdir().unwrap();
        let mut lines = round_one();
        lines.push(user_msg("2026-09-06T05:41:25.003Z", 9, "同步到本地"));
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Thinking);
    }

    /// 关键判定 4：记账条目（token_count / item_completed / reasoning）在尾不改变判定
    #[test]
    fn bookkeeping_tail_does_not_change_status() {
        let tmp = tempfile::tempdir().unwrap();
        // 工具调用尾部 + 纯记账条目 → 仍为 Processing
        let mut lines = round_two_running();
        lines.push(token_count("2026-09-06T05:41:41.367Z", 16));
        lines.push(item_completed("2026-09-06T05:41:43.136Z", 17));
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Processing);
        // 回合结束尾部 + 纯记账条目 → 仍为 Idle（顺带改善：旧实现误显运行中）
        let mut lines = round_two_complete();
        lines.push(item_completed("2026-09-06T05:41:43.200Z", 19));
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Idle);
    }

    /// 两轮同文件场景（用户实测）：第二轮运行期为 Processing、完成后为 Idle
    #[test]
    fn two_rounds_same_file_second_round_running_then_idle() {
        let tmp = tempfile::tempdir().unwrap();
        // 第二轮运行中（追加写入同一 rollout 文件、session_id 不变）
        let mut lines = round_one();
        lines.extend(round_two_running());
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(
            session.status,
            SessionStatus::Processing,
            "第二轮运行期必须显示运行中（issue #6 主修复）"
        );
        // 第二轮完成（task_complete 收尾）→ Idle
        let mut lines = round_one();
        lines.extend(round_two_complete());
        let f = write_rollout(tmp.path(), &lines);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Idle);
    }

    /// 300s 叠加：Processing 且文件停更 >= 300s → Waiting（与 WorkBuddy 语义一致）
    #[test]
    fn processing_stale_downgrades_to_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout(tmp.path(), &round_two_running());
        let stale = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(301))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(stale)
            .unwrap();
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Waiting);
    }

    /// 兜底分侧保持：无语义条目（仅 session_meta）时，APP 形态文件新鲜 → Processing
    /// （与 phase2_app_form_tests::fixture_discriminates_cli_vs_app_mtime_thresholds 互补）
    #[test]
    fn meta_only_fresh_app_form_is_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout(tmp.path(), &[meta()]);
        let session = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(session.status, SessionStatus::Processing);
    }
}
