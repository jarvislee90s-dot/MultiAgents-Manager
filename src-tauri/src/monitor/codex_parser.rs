// Codex CLI 会话解析 — type + payload 协议（rollout-*.jsonl）
// 公共设施（cwd 归一化、git URL 缓存、JSONL 尾部读取）见 monitor::{cwd,git,jsonl,project}

use super::app_status::{derive_app_status, overlay_mtime_stale, AppEntryKind};
use super::cwd::normalize_cwd_for_match;
use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use super::session_scan::{fresh_within, SessionFileScan};
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, ProcessForm, Session, SessionStatus};
use log::{debug, info};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// L2/L3 会话扫描流水线（monitor::session_scan；codex 是唯一无界历史扫描者，三层全接）
const CODEX_SCAN: SessionFileScan = SessionFileScan::new("codex-digest");

/// rollout 文件命名判据（collect 的 accept 闭包）
fn is_rollout_file(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("rollout") && n.ends_with(".jsonl"))
        .unwrap_or(false)
}

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

/// 扫描 Codex 会话：1. rollout 目录按 cwd 匹配 CLI 进程（CLI 前端仍写 rollout）；
/// 2. 宿主 APP 在场时读 state/thread_history SQLite 出 APP 卡（codex_thread_parser）
///
/// 会话扫描预算三层（monitor::session_scan）：零进程零解析（编排层 + 此处纵深防御）；
/// (mtime,size) 摘要缓存（纯内容 digest）；24h 新鲜窗口 + 活跃进程匹配失败回退全量。
/// SQLite 路线查询即过滤，仅受 L1 约束。
pub fn get_codex_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let sessions_dir = dirs::home_dir()
        .map(|h| h.join(".codex").join("sessions"))
        .unwrap_or_default();
    scan_codex_sessions(&sessions_dir, processes, SystemTime::now())
}

/// 可注入核心（目录 / 时钟；测试驱动 L3 窗口与回退路径，不触真实 ~/.codex）
fn scan_codex_sessions(
    sessions_dir: &Path,
    processes: &[AgentProcess],
    now: SystemTime,
) -> Vec<Session> {
    let mut sessions = Vec::new();
    // L1 纵深防御：零进程零解析（编排层已统一短路，直调/测试路径同样不扫）
    if processes.is_empty() {
        info!("Codex: 0 sessions from 0 processes");
        return sessions;
    }

    if !sessions_dir.exists() {
        return sessions;
    }

    // 全量收集（mtime 倒序）；mtimes 同时供 Phase 1 年龄现算与 Phase 2 新鲜过滤复用
    let scanned = CODEX_SCAN.collect(sessions_dir, is_rollout_file);
    let jsonl_files: Vec<PathBuf> = scanned.iter().map(|(f, _)| f.clone()).collect();
    let mtimes: Vec<SystemTime> = scanned.iter().map(|(_, m)| *m).collect();
    debug!("Codex: {} session files scanned", jsonl_files.len());

    // L3 解析集 = 24h 新鲜文件（现解析）∪ 缓存已有的陈旧文件摘要（peek）。
    // 陈旧且未缓存 = 历史 rollout，正常路径不解析（本机 2GB 历史库即靠这层收敛）；
    // None 槽位留给 fill_uncached 回退兜底
    let mut digests: Vec<Arc<Option<CodexFileDigest>>> = scanned
        .iter()
        .map(|(f, m)| {
            if fresh_within(*m, now) {
                CODEX_SCAN.parse(f, read_codex_digest)
            } else {
                CODEX_SCAN
                    .peek::<Option<CodexFileDigest>>(f)
                    .unwrap_or_else(|| Arc::new(None))
            }
        })
        .collect();

    let mut matched_file_indices: HashSet<usize> = HashSet::new();
    let mut matched_processes: HashSet<usize> = HashSet::new();

    // Phase 1: 按 cwd 精确匹配——每个进程一张卡，取目录匹配中 mtime 最新的 rollout。
    // codex CLI 每轮对话写新 rollout（session_id 变），若按文件循环会把同一进程
    // 挂成多张卡（用户实测同一窗口被识别为重复会话）
    let pending = phase1_match(
        processes,
        &mtimes,
        &mut digests,
        &mut sessions,
        &mut matched_file_indices,
        &mut matched_processes,
    );
    // L3 回退：有带 cwd 的进程未匹配——其 rollout 可能空闲超过 24h 窗口（如挂机过夜）。
    // 全量解析一次（结果进缓存，整个应用生命周期只付一次），再补一轮匹配
    if pending > 0 {
        CODEX_SCAN.fill_uncached(&jsonl_files, &mut digests, read_codex_digest);
        phase1_match(
            processes,
            &mtimes,
            &mut digests,
            &mut sessions,
            &mut matched_file_indices,
            &mut matched_processes,
        );
    }

    // Phase 2（W4 每会话一卡，2026-09-10 起改读 app-server SQLite）：新版 Codex APP
    // 已把会话迁入 ~/.codex/state_*.sqlite（rollout 目录不再有新写入），APP 卡由
    // codex_thread_parser 双库产出（threads 元数据实时、items/turns 投影可用则用）。
    // 宿主 App 进程在场才出卡；完成转绿的持久未读由 DB 管线合并
    let app_processes: Vec<AgentProcess> = processes
        .iter()
        .filter(|p| matches!(p.form, ProcessForm::App))
        .cloned()
        .collect();
    // review F2：宿主判定口径与 monitor::host::is_host_process 一致
    // （ChatGPT 主进程/chatgpt.exe 才算宿主；CLI 同名 exe、内嵌框架进程不算）
    if let Some(host) = codex_host_process(&app_processes) {
        // CLI 认领（Phase 1 已占用的文件）对应的会话 id——同一会话不得重复出 APP 卡
        let cli_claimed_ids: HashSet<String> = matched_file_indices
            .iter()
            .filter_map(|&i| {
                digests
                    .get(i)
                    .and_then(|a| a.as_ref().as_ref())
                    .and_then(|d| d.session_id.clone())
            })
            .collect();
        // sessions 目录（~/.codex/sessions）的父目录就是 .codex 数据根
        let roots = sessions_dir
            .parent()
            .map(super::codex_thread_parser::CodexThreadRoots::from_codex_root)
            .unwrap_or_default();
        let now_s = now
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        sessions.extend(super::codex_thread_parser::build_sessions(
            &roots,
            host,
            now_s,
            &cli_claimed_ids,
        ));
    }

    // L2 缓存收敛：清掉已消失文件的孤儿条目
    let live: HashSet<PathBuf> = jsonl_files.iter().cloned().collect();
    CODEX_SCAN.retain_existing(&live);

    info!(
        "Codex: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// Phase 1 匹配一轮：返回「有合法 cwd 但仍未匹配」的进程数（>0 时调用方决定回退）
#[allow(clippy::too_many_arguments)]
fn phase1_match(
    processes: &[AgentProcess],
    mtimes: &[SystemTime],
    digests: &mut [Arc<Option<CodexFileDigest>>],
    sessions: &mut Vec<Session>,
    matched_files: &mut HashSet<usize>,
    matched_processes: &mut HashSet<usize>,
) -> usize {
    let now = SystemTime::now();
    for (pidx, process) in processes.iter().enumerate() {
        if matched_processes.contains(&pidx) {
            continue;
        }
        let Some(cwd) = &process.cwd else { continue };
        let normalized = normalize_cwd_for_match(&cwd.to_string_lossy());
        if normalized.is_empty() {
            continue;
        }
        // scanned 按 mtime 倒序，找第一个目录匹配且未被其他进程占用的文件
        for (idx, mtime) in mtimes.iter().enumerate() {
            if matched_files.contains(&idx) {
                continue;
            }
            let Some(digest) = digests.get(idx).and_then(|a| a.as_ref().as_ref()) else {
                continue; // 陈旧未解析（回退轮会补上）
            };
            if normalize_cwd_for_match(&digest.project_path) != normalized {
                continue;
            }
            // 时间叠加现算（L2 契约）：状态阈值语义与旧实现一致
            let file_age_secs = now.duration_since(*mtime).ok().map(|d| d.as_secs_f32());
            if let Some(mut session) = session_from_digest(digest, process.form, file_age_secs) {
                session.pid = process.pid;
                session.cpu_usage = process.cpu_usage;
                session.form = process.form;
                session.jump_supported = jump_supported_for(process.form);
                session.github_url = get_github_url(&session.project_path);
                sessions.push(session);
                matched_files.insert(idx);
                matched_processes.insert(pidx);
                break; // 每进程只取最新一个
            }
        }
    }
    // 仍有合法 cwd 且未匹配的进程数
    processes
        .iter()
        .enumerate()
        .filter(|(pidx, p)| {
            !matched_processes.contains(pidx)
                && p.cwd
                    .as_ref()
                    .map(|c| !normalize_cwd_for_match(&c.to_string_lossy()).is_empty())
                    .unwrap_or(false)
        })
        .count()
}

// 文件收集已收编进 monitor::session_scan::SessionFileScan::collect（L3 模板见该模块文档）
// Phase 2 旧聚合链（unclaimed_app_parsed / fresh_unclaimed_files / aggregate_app_sessions）
// 已随 APP 存储迁移 SQLite 退役（2026-09-10），由 codex_thread_parser 双库产出 APP 卡

/// 纯内容摘要（L2 缓存产物）：只由文件内容决定，与当前时间 / 进程无关。
/// last_message 的 100 字符截断是纯内容操作，随摘要一并缓存
struct CodexFileDigest {
    session_id: Option<String>,
    project_path: String,
    kinds: Vec<AppEntryKind>,
    last_message: Option<String>,
    last_role: Option<String>,
    last_timestamp: Option<String>,
}

/// 读单个 rollout 的内容摘要（尾读 500 行倒扫；session_meta 缺失或 cwd 为空 → None）
fn read_codex_digest(jsonl_path: &Path) -> Option<CodexFileDigest> {
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

    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 {
            format!("{}...", m.chars().take(100).collect::<String>())
        } else {
            m
        }
    });

    Some(CodexFileDigest {
        session_id: Some(session_id),
        project_path,
        kinds,
        last_message,
        last_role,
        last_timestamp,
    })
}

/// 时间叠加 + Session 构建（每次现算，L2 契约的"外层"）；pid / cpu / form 由调用方盖章。
/// 状态判定（issue #6）：共享核尾部倒扫取第一条有语义条目——user → Thinking；
/// assistant 纯文本 → Idle；function_call/function_call_output → Processing；
/// task_started → Processing；task_complete → Idle；记账条目跳过。
/// 无任何语义条目（如仅 session_meta 的 rollout）→ 兜底按形态：APP 文件新鲜
/// （<300s）→ Processing，停更 → Idle（绿灯）；CLI 保持 60s 阈值。
/// codex 全线不落兜底红（2026-09-10 用户决策）：停更后无法区分"等用户输入"与
/// "对话已结束"，红灯「等待操作」会对每次聊完的会话误报；落绿灯接入
/// 「完成转绿 → 未读徽标 → 已读后绿卡剔除」既有管线（与 codex_thread_parser 同语义）
fn session_from_digest(
    digest: &CodexFileDigest,
    process_form: ProcessForm,
    file_age_secs: Option<f32>,
) -> Option<Session> {
    let status = match derive_app_status(&digest.kinds) {
        Some(status) => status,
        None => {
            let fresh = match process_form {
                ProcessForm::App => file_age_secs.map(|a| a < 300.0).unwrap_or(false),
                ProcessForm::Cli => file_age_secs.map(|a| a < 60.0).unwrap_or(false),
            };
            if fresh {
                SessionStatus::Processing
            } else {
                SessionStatus::Idle
            }
        }
    };
    // 叠加 300s 规则（共享核）：Processing 且 JSONL mtime 停更 >= 300s → Waiting；
    // derive_app_status 不产出 Waiting，此处 Waiting 只能来自时间兜底路径 →
    // 就地转 Idle，内容推导的状态（Thinking/Idle/Processing）不受影响
    let status = overlay_mtime_stale(status, file_age_secs.map_or(0, |a| (a * 1000.0) as u64));
    let status = if status == SessionStatus::Waiting {
        SessionStatus::Idle
    } else {
        status
    };

    let project_name = project_name_from_path(&digest.project_path);
    // 卡片前缀统一 12 位 hex（按字符截取，多字节 id 不 panic）：UUIDv7 前 8 hex 只编码
    // 65.5s 粒度，同分钟启动的会话撞车（实测 01a08083-5ca0 与 01a08083-2260）；
    // 前 12 hex 是完整 48 位毫秒时间戳（毫秒级粒度），不同会话几乎必然错开，
    // 再叠 4 位随机位兜底，实际不撞。先剥连字符再截取——直接 take(12)
    // 会让连字符占去一格只剩 11 位 hex。注意与 hook marker（MAM:<id 前 8 位>）口径
    // 解耦：marker 通道尚未启用，未来启用时应同步改为 12 位（issue 见 marker 复活提案）
    let codex_title = digest
        .session_id
        .as_deref()?
        .chars()
        .filter(|c| *c != '-')
        .take(12)
        .collect::<String>();
    Some(Session {
        id: digest.session_id.clone()?,
        agent_type: AgentType::Codex,
        project_name,
        project_path: digest.project_path.clone(),
        git_branch: None,
        github_url: None, // 延迟到进程匹配后填充（见 get_codex_sessions），避免批量解析时风暴式 spawn git
        status,
        last_message: digest.last_message.clone(),
        last_message_role: digest.last_role.clone(),
        last_activity_at: digest
            .last_timestamp
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
        pid: 0, // 由调用方设置
        cpu_usage: 0.0,
        active_subagent_count: 0,
        form: ProcessForm::Cli,                               // 由调用方设置
        jump_supported: jump_supported_for(ProcessForm::Cli), // 由调用方按进程形态覆盖
        unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        title: Some(codex_title),
    })
}

/// 解析单个 Codex JSONL 文件（缓存摘要 + 时间叠加现算）。
/// 生产路径已改走 digest + session_from_digest 分离管线（Phase 1）与 SQLite
/// （Phase 2，codex_thread_parser），本函数保留为测试入口（形态阈值 / 标题等单测）
#[cfg(test)]
fn parse_codex_jsonl(jsonl_path: &Path, process_form: ProcessForm) -> Option<Session> {
    let digest = CODEX_SCAN.parse(jsonl_path, read_codex_digest);
    // 时间叠加每次现算（L2 契约）：mtime 变化必然伴随 (mtime,size) 失效重读
    let file_age_secs = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32());
    let d = digest.as_ref().as_ref()?;
    session_from_digest(d, process_form, file_age_secs)
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

    /// 同分钟启动的两个 UUIDv7 会话（前 8 位相同）必须靠更长前缀区分
    #[test]
    fn same_minute_uuid7_sessions_get_distinct_titles() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = |name: &str, id: &str| {
            let p = tmp.path().join(name);
            std::fs::write(
                &p,
                format!(
                    r#"{{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{{"id":"{}","cwd":"/work/demo"}}}}"#,
                    id
                ),
            )
            .unwrap();
            p
        };
        let a = mk("a.jsonl", "01a08083-5ca0-74c2-97bf-6dfdd149fac5");
        let b = mk("b.jsonl", "01a08083-2260-7462-beff-2212cffec36d");
        let sa = parse_codex_jsonl(&a, ProcessForm::Cli).unwrap();
        let sb = parse_codex_jsonl(&b, ProcessForm::Cli).unwrap();
        assert_ne!(sa.title, sb.title, "同分钟双开的 codex 卡片标题不得相同");
        assert_eq!(sa.title.as_deref(), Some("01a080835ca0"));
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

    /// 夹具区分度自检：同一文件，Cli 形态（60s）停更 → Idle（绿灯），App 形态（300s）→ Processing
    #[test]
    fn fixture_discriminates_cli_vs_app_mtime_thresholds() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_meta_only_rollout(tmp.path());
        let cli = parse_codex_jsonl(&f, ProcessForm::Cli).unwrap();
        assert_eq!(cli.status, SessionStatus::Idle);
        let app = parse_codex_jsonl(&f, ProcessForm::App).unwrap();
        assert_eq!(app.status, SessionStatus::Processing);
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

    /// 300s 叠加 + 兜底红消除：Processing 且文件停更 >= 300s → Idle（绿灯完成待看，
    /// 2026-09-10 用户决策——停更后无法区分"等输入"与"已结束"，不落红灯「等待操作」）
    #[test]
    fn processing_stale_downgrades_to_idle() {
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
        assert_eq!(session.status, SessionStatus::Idle);
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

/// 扫描预算三层（monitor::session_scan）行为锁定：零进程零解析 / 24h 窗口 / 回退
#[cfg(test)]
mod scan_budget_tests {
    use super::*;
    use std::time::Duration;

    fn write_rollout_at(dir: &Path, name: &str, cwd: &str, id: &str) -> PathBuf {
        let line = format!(
            r#"{{"timestamp":"2026-09-10T00:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}"}}}}"#
        );
        let f = dir.join(name);
        std::fs::write(&f, line).unwrap();
        f
    }

    fn cli_process(cwd: &str) -> AgentProcess {
        AgentProcess {
            pid: 4321,
            cpu_usage: 0.0,
            cwd: Some(PathBuf::from(cwd)),
            exe: None,
            form: ProcessForm::Cli,
        }
    }

    /// L1：零进程时不得解析任何文件（缓存中无摘要即为"从未解析"的可观测证据）
    #[test]
    fn zero_processes_never_parses_files() {
        let tmp = tempfile::tempdir().unwrap();
        let f = write_rollout_at(tmp.path(), "rollout-a.jsonl", "/tmp/proj-zero", "id-zero");
        let out = scan_codex_sessions(tmp.path(), &[], SystemTime::now());
        assert!(out.is_empty());
        assert!(
            CODEX_SCAN.peek::<Option<CodexFileDigest>>(&f).is_none(),
            "零进程不得触发任何文件解析"
        );
    }

    /// L3 回退：进程的 rollout 空闲超过 24h 窗口（注入未来时钟模拟）时，
    /// 全量兜底解析仍应匹配出卡——空闲超窗的活跃会话卡不丢
    #[test]
    fn stale_file_still_matches_via_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        write_rollout_at(tmp.path(), "rollout-old.jsonl", "/tmp/proj-fb", "id-fb");
        // 未来时钟：文件真实 mtime（刚写入）全部落在 24h 窗口之外
        let future = SystemTime::now()
            + Duration::from_secs(super::super::session_scan::SCAN_FRESH_SECS + 3600);
        let out = scan_codex_sessions(tmp.path(), &[cli_process("/tmp/proj-fb")], future);
        assert_eq!(out.len(), 1, "窗口外历史文件经回退解析后仍应出卡");
        assert_eq!(out[0].project_path, "/tmp/proj-fb");
        assert_eq!(out[0].pid, 4321, "回退出卡仍要盖进程章");
    }

    /// L3：窗口外文件 + cwd 不匹配的进程 → 回退解析后依然无卡（宁缺勿错）
    #[test]
    fn stale_unmatched_file_yields_no_card() {
        let tmp = tempfile::tempdir().unwrap();
        write_rollout_at(tmp.path(), "rollout-other.jsonl", "/tmp/proj-x", "id-x");
        let future = SystemTime::now()
            + Duration::from_secs(super::super::session_scan::SCAN_FRESH_SECS + 3600);
        let out = scan_codex_sessions(tmp.path(), &[cli_process("/tmp/proj-y")], future);
        assert!(out.is_empty(), "cwd 不匹配不得出卡");
    }

    /// L2：同一文件两轮扫描（未修改）摘要命中缓存——第二轮与首轮结果一致
    /// （Phase 2 需宿主 App 进程在场，此处用 CLI 匹配路径验证）
    #[test]
    fn second_scan_reuses_cached_digest() {
        let tmp = tempfile::tempdir().unwrap();
        write_rollout_at(tmp.path(), "rollout-c.jsonl", "/tmp/proj-c", "id-c");
        let now = SystemTime::now();
        let first = scan_codex_sessions(tmp.path(), &[cli_process("/tmp/proj-c")], now);
        let second = scan_codex_sessions(tmp.path(), &[cli_process("/tmp/proj-c")], now);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[0].project_path, second[0].project_path);
    }
}
