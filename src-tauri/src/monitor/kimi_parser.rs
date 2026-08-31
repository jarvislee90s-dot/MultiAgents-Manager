// Kimi Code 会话解析 — session_index.jsonl 定位 + wire.jsonl 状态判定
//
// 数据布局（官方文档 configuration/data-locations + 本机实测）：
//   $KIMI_CODE_HOME（默认 ~/.kimi-code，可用环境变量重定向；早期版本为 ~/.kimi）
//   ├── session_index.jsonl      每行一个 {sessionId, sessionDir, workDir}
//   └── sessions/<workDirKey>/<sessionId>/
//       ├── state.json           {title, isCustomTitle, createdAt, updatedAt, ...}
//       └── agents/main/wire.jsonl  事件流（按 "type" 字段判别，time 为 epoch 毫秒）
//
// 会话定位：以 session_index.jsonl 的 workDir 与进程 cwd 归一化匹配（workDirKey 的
// slug+sha256 不可逆算，索引是唯一稳定映射）；每个进程取 mtime 最新的会话（一进程一卡）。
//
// 状态判定：扫 wire.jsonl 尾部取最新一条有效信号（见 entry_status），
// 兜底按文件 60s 内是否有改动（与既有工具 file_recently_modified 同语义）。

use super::cwd::normalize_cwd_for_match;
use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, Session, SessionStatus};
use log::{debug, info};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const RECENT_LINES: usize = 500;
/// 兜底"文件仍在写入"的时间窗（秒），与既有解析器同量级
const FILE_RECENT_SECS: f32 = 60.0;

/// Kimi Code 数据根目录：KIMI_CODE_HOME 环境变量优先，否则 ~/.kimi-code
pub(crate) fn kimi_home() -> PathBuf {
    if let Ok(h) = std::env::var("KIMI_CODE_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join(".kimi-code")
}

/// 会话根目录：$KIMI_CODE_HOME/sessions；主目录不存在时回退早期版本 ~/.kimi/sessions
fn kimi_sessions_root() -> Option<PathBuf> {
    let primary = kimi_home().join("sessions");
    if primary.exists() {
        return Some(primary);
    }
    let legacy = dirs::home_dir()
        .unwrap_or_default()
        .join(".kimi")
        .join("sessions");
    legacy.exists().then_some(legacy)
}

/// session_index.jsonl 条目（字段名以官方文档为准；alias 容忍 snake_case 变体）
#[derive(Deserialize)]
struct KimiIndexEntry {
    #[serde(rename = "sessionId", alias = "session_id")]
    session_id: String,
    #[serde(rename = "sessionDir", alias = "session_dir")]
    session_dir: String,
    #[serde(rename = "workDir", alias = "work_dir")]
    work_dir: String,
}

/// 索引解析后的会话（wire.jsonl 按 mtime 倒序排列，匹配时取最新）
struct IndexedSession {
    session_id: String,
    work_dir: String,
    session_dir: PathBuf,
    wire_mtime: SystemTime,
}

/// state.json 元数据（字段缺失容忍）
#[derive(Deserialize)]
struct KimiState {
    title: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
}

/// wire.jsonl 条目（按 type 判别，各形态字段缺失容忍）
#[derive(Deserialize)]
struct KimiWireEntry {
    #[serde(rename = "type")]
    etype: Option<String>,
    time: Option<i64>,
    /// turn.prompt / turn.steer：用户输入 [{type:"text", text}]
    input: Option<Vec<KimiTextPart>>,
    /// context.append_message：{role, content, toolCalls}
    message: Option<KimiMessage>,
    /// context.append_loop_event：{type, part, finishReason, ...}（结构随事件变化，用 Value 挖取）
    event: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct KimiTextPart {
    text: Option<String>,
}

#[derive(Deserialize)]
struct KimiMessage {
    role: Option<String>,
    content: Option<Vec<KimiTextPart>>,
    #[serde(rename = "toolCalls")]
    tool_calls: Option<Vec<serde_json::Value>>,
}

/// 扫描 session_index.jsonl 匹配运行中的 Kimi 进程
pub fn get_kimi_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();
    if processes.is_empty() {
        return sessions;
    }
    let Some(sessions_root) = kimi_sessions_root() else {
        debug!("Kimi: sessions root not found, skipping");
        return sessions;
    };

    let index = read_session_index(&sessions_root, &kimi_home());
    if index.is_empty() {
        debug!("Kimi: session_index.jsonl missing or empty");
        return sessions;
    }

    let mut matched: HashSet<usize> = HashSet::new();

    // Phase 1: 按 cwd 精确匹配——每个进程一张卡，取该 workDir 下 mtime 最新的会话
    for process in processes {
        let Some(cwd) = &process.cwd else { continue };
        let normalized = normalize_cwd_for_match(&cwd.to_string_lossy());
        if normalized.is_empty() {
            continue;
        }
        for (idx, entry) in index.iter().enumerate() {
            if matched.contains(&idx) {
                continue;
            }
            if normalize_cwd_for_match(&entry.work_dir) != normalized {
                continue;
            }
            if let Some(session) = parse_kimi_session(entry, process) {
                sessions.push(session);
                matched.insert(idx);
                break; // 每进程只取最新一个
            }
        }
    }

    // Phase 2: 无有效 cwd 的进程回退到最新未匹配会话（与 codex 解析器同策略）
    for process in processes {
        let has_cwd = process
            .cwd
            .as_ref()
            .map(|c| !normalize_cwd_for_match(&c.to_string_lossy()).is_empty())
            .unwrap_or(false);
        if has_cwd {
            continue;
        }
        for (idx, entry) in index.iter().enumerate() {
            if matched.contains(&idx) {
                continue;
            }
            if let Some(session) = parse_kimi_session(entry, process) {
                sessions.push(session);
                matched.insert(idx);
                break;
            }
        }
    }

    info!(
        "Kimi: {} sessions from {} processes",
        sessions.len(),
        processes.len()
    );
    sessions
}

/// 解析 session_index.jsonl（按 wire.jsonl mtime 倒序；无 wire 的条目丢弃）
fn read_session_index(sessions_root: &Path, home: &Path) -> Vec<IndexedSession> {
    let index_path = home.join("session_index.jsonl");
    let content = match fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut index: Vec<IndexedSession> = content
        .lines()
        .filter_map(|line| {
            let entry: KimiIndexEntry = serde_json::from_str(line).ok()?;
            let session_dir = resolve_session_dir(home, sessions_root, &entry.session_dir);
            let wire = session_dir.join("agents").join("main").join("wire.jsonl");
            let wire_mtime = fs::metadata(&wire).and_then(|m| m.modified()).ok()?;
            Some(IndexedSession {
                session_id: entry.session_id,
                work_dir: entry.work_dir,
                session_dir,
                wire_mtime,
            })
        })
        .collect();
    index.sort_by_key(|s| std::cmp::Reverse(s.wire_mtime));
    index
}

/// sessionDir 可能是绝对路径，也可能相对 sessions/ 或数据根目录
fn resolve_session_dir(home: &Path, sessions_root: &Path, session_dir: &str) -> PathBuf {
    let p = PathBuf::from(session_dir);
    if p.is_absolute() {
        return p;
    }
    let in_sessions = sessions_root.join(&p);
    if in_sessions.exists() {
        return in_sessions;
    }
    home.join(&p)
}

/// 解析单个 Kimi 会话：state.json 元数据 + wire.jsonl 尾部状态
fn parse_kimi_session(entry: &IndexedSession, process: &AgentProcess) -> Option<Session> {
    let wire = entry
        .session_dir
        .join("agents")
        .join("main")
        .join("wire.jsonl");
    if !wire.exists() {
        return None;
    }

    let file_recently_modified = wire
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| SystemTime::now().duration_since(m).ok())
        .map(|d| d.as_secs_f32() < FILE_RECENT_SECS)
        .unwrap_or(false);

    let recent = read_recent_lines(&wire, RECENT_LINES);

    let state: Option<KimiState> = fs::read_to_string(entry.session_dir.join("state.json"))
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok());

    let mut status: Option<SessionStatus> = None;
    let mut last_message: Option<String> = None;
    let mut last_role: Option<String> = None;
    let mut last_ts: Option<i64> = None;

    for line in recent.iter().rev() {
        let Ok(wire_entry) = serde_json::from_str::<KimiWireEntry>(line) else {
            continue;
        };
        if last_ts.is_none() {
            last_ts = wire_entry.time;
        }
        if last_message.is_none() {
            if let Some((text, role)) = entry_text(&wire_entry) {
                last_message = Some(text);
                last_role = Some(role);
            }
        }
        if status.is_none() {
            status = entry_status(&wire_entry);
        }
        if status.is_some() && last_message.is_some() && last_ts.is_some() {
            break;
        }
    }

    let status = status.unwrap_or(if file_recently_modified {
        SessionStatus::Processing
    } else {
        SessionStatus::Waiting
    });

    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 {
            format!("{}...", m.chars().take(100).collect::<String>())
        } else {
            m
        }
    });

    // 标题优先取 state.json（用户自定义/会话摘要），回退 8 位 id 前缀（与其他工具卡片一致）
    let title = state
        .as_ref()
        .and_then(|s| s.title.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| entry.session_id[..entry.session_id.len().min(8)].to_string());
    let last_activity_at = last_ts
        .map(ms_to_iso)
        .or_else(|| state.and_then(|s| s.updated_at))
        .unwrap_or_else(|| "Unknown".to_string());

    Some(Session {
        id: entry.session_id.clone(),
        agent_type: AgentType::Kimi,
        project_name: project_name_from_path(&entry.work_dir),
        project_path: entry.work_dir.clone(),
        git_branch: None,
        github_url: get_github_url(&entry.work_dir),
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at,
        pid: process.pid,
        cpu_usage: process.cpu_usage,
        active_subagent_count: 0,
        form: process.form,
        jump_supported: jump_supported_for(process.form),
        title: Some(title),
    })
}

/// 条目的状态信号：None 表示该条目不构成状态信号，继续向前扫
fn entry_status(e: &KimiWireEntry) -> Option<SessionStatus> {
    use SessionStatus::*;
    match e.etype.as_deref()? {
        // 压缩进行中（未见 complete/cancel 即停在 begin）
        "full_compaction.begin" => Some(Compacting),
        // 压缩已完成/取消：压缩点本身不是状态信号，继续前扫
        "full_compaction.complete" | "full_compaction.cancel" | "context.apply_compaction"
        | "micro_compaction.apply" => None,
        "context.append_loop_event" => {
            let event = e.event.as_ref()?;
            match event.get("type")?.as_str()? {
                // 工具执行中 / LLM 步进中 → 处理中
                "tool.call" | "step.begin" => Some(Processing),
                "content.part" => {
                    match event.pointer("/part/type").and_then(|t| t.as_str()) {
                        Some("think") => Some(Thinking),
                        Some("text") => Some(Processing), // 回复流式输出中
                        _ => None,
                    }
                }
                "step.end" => {
                    // finishReason=tool_use：本步以工具调用结束，后续还有步骤 → Processing
                    // end_turn：轮次边界，继续前扫找轮次级信号
                    (event.get("finishReason").and_then(|r| r.as_str()) == Some("tool_use"))
                        .then_some(Processing)
                }
                // 工具刚返回、等下一步决策：非终态信号，继续前扫
                "tool.result" => None,
                _ => None,
            }
        }
        // 用户刚提交输入，Agent 开始工作
        "turn.prompt" | "turn.steer" => Some(Thinking),
        // 用户打断，回到输入态
        "turn.cancel" => Some(Waiting),
        // 一轮结束（usage.record 是每轮最后一个事件）：等待用户输入
        "usage.record" => Some(Waiting),
        // LLM 请求在飞
        "llm.request" => Some(Processing),
        // 权限放行后继续执行
        "permission.record_approval_result" => Some(Processing),
        // 目标模式轮次进行中
        "goal.create" | "goal.update" => Some(Processing),
        "context.append_message" => {
            let msg = e.message.as_ref()?;
            match msg.role.as_deref() {
                Some("user") => Some(Thinking),
                Some("assistant") => {
                    if msg.tool_calls.as_ref().is_some_and(|t| !t.is_empty()) {
                        Some(Processing) // 带工具调用的 assistant 消息 → 工具将执行
                    } else {
                        Some(Waiting) // 纯文本回复 → 轮次结束，等待输入
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 提取条目中的可展示文本（用于 last_message 预览），返回 (文本, 角色)
fn entry_text(e: &KimiWireEntry) -> Option<(String, String)> {
    match e.etype.as_deref()? {
        "context.append_loop_event" => {
            let event = e.event.as_ref()?;
            if event.get("type")?.as_str()? == "content.part" {
                let part = event.get("part")?;
                if part.get("type")?.as_str()? == "text" {
                    let text = part.get("text")?.as_str()?;
                    if !text.is_empty() {
                        return Some((text.to_string(), "assistant".to_string()));
                    }
                }
            }
            None
        }
        "turn.prompt" | "turn.steer" => {
            let text = e.input.as_ref()?.iter().find_map(|p| p.text.clone())?;
            if !text.is_empty() {
                Some((text, "user".to_string()))
            } else {
                None
            }
        }
        "context.append_message" => {
            let msg = e.message.as_ref()?;
            let role = msg.role.clone().unwrap_or_else(|| "user".to_string());
            let text = msg.content.as_ref()?.iter().find_map(|p| p.text.clone())?;
            if !text.is_empty() {
                Some((text, role))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 毫秒时间戳 → ISO 8601 字符串（与 opencode 解析器同格式）
fn ms_to_iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "Unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::ProcessForm;

    fn fake_process(pid: u32, cwd: &str) -> AgentProcess {
        AgentProcess {
            pid,
            cpu_usage: 0.0,
            cwd: Some(PathBuf::from(cwd)),
            form: ProcessForm::Cli,
        }
    }

    /// 在临时目录构造一个 Kimi 会话（session_index.jsonl + wire.jsonl），返回 (home, 期望 workDir)
    fn fixture_session(
        home: &Path,
        work_dir: &str,
        wire_lines: &[&str],
    ) -> (PathBuf, String) {
        let sessions = home.join("sessions");
        let session_dir = sessions
            .join("wd_demo_0123456789ab")
            .join("session_11111111-1111-1111-1111-111111111111");
        fs::create_dir_all(session_dir.join("agents").join("main")).unwrap();
        fs::write(
            session_dir.join("state.json"),
            r#"{"title":"Demo Session","createdAt":"2026-08-01T00:00:00.000Z","updatedAt":"2026-08-01T00:01:00.000Z"}"#,
        )
        .unwrap();
        fs::write(
            session_dir.join("agents").join("main").join("wire.jsonl"),
            wire_lines.join("\n"),
        )
        .unwrap();
        fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{{\"sessionId\":\"11111111-1111-1111-1111-111111111111\",\"sessionDir\":\"{}\",\"workDir\":\"{}\"}}\n",
                session_dir.to_string_lossy(),
                work_dir
            ),
        )
        .unwrap();
        (session_dir, work_dir.to_string())
    }

    /// 串行化 KIMI_CODE_HOME 环境变量切换：进程级 env 在并行测试间共享，必须互斥
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn run_with_home<T>(home: &Path, f: impl FnOnce() -> T) -> T {
        let _guard = HOME_LOCK.lock().unwrap();
        std::env::set_var("KIMI_CODE_HOME", home);
        let result = f();
        std::env::remove_var("KIMI_CODE_HOME");
        result
    }

    #[test]
    fn no_kimi_home_means_no_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("missing-kimi-home");
        let sessions = run_with_home(&home, || get_kimi_sessions(&[fake_process(1, "/work")]));
        assert!(sessions.is_empty());
    }

    #[test]
    fn no_processes_means_no_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions = run_with_home(tmp.path(), || get_kimi_sessions(&[]));
        assert!(sessions.is_empty());
    }

    #[test]
    fn turn_prompt_last_means_thinking() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"metadata","protocol_version":"1.4","created_at":1782300834489}"#,
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"帮我写个函数"}],"origin":{"kind":"user"},"time":1782300900000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(4242, &work_dir)])
        });
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.agent_type, AgentType::Kimi);
        assert_eq!(s.pid, 4242);
        assert_eq!(s.status, SessionStatus::Thinking);
        assert_eq!(s.project_name, "demo");
        assert_eq!(s.title.as_deref(), Some("Demo Session"));
        assert_eq!(s.last_message.as_deref(), Some("帮我写个函数"));
        assert_eq!(s.last_message_role.as_deref(), Some("user"));
        assert!(s.jump_supported);
    }

    #[test]
    fn usage_record_last_means_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi"}],"time":1782300900000}"#,
                r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"text","text":"done!"}},"time":1782300905000}"#,
                r#"{"type":"usage.record","model":"m","usage":{"output":1},"time":1782300906000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Waiting);
        assert_eq!(sessions[0].last_message.as_deref(), Some("done!"));
        assert_eq!(sessions[0].last_message_role.as_deref(), Some("assistant"));
    }

    #[test]
    fn tool_call_in_flight_means_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"run tests"}],"time":1782300900000}"#,
                r#"{"type":"context.append_loop_event","event":{"type":"tool.call","toolCallId":"Bash_1","name":"Bash"},"time":1782300901000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions[0].status, SessionStatus::Processing);
    }

    #[test]
    fn step_end_with_tool_use_means_processing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[r#"{"type":"context.append_loop_event","event":{"type":"step.end","finishReason":"tool_use"},"time":1782300901000}"#],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions[0].status, SessionStatus::Processing);
    }

    #[test]
    fn think_part_means_thinking() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[r#"{"type":"context.append_loop_event","event":{"type":"content.part","part":{"type":"think","think":"reasoning"}},"time":1782300901000}"#],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions[0].status, SessionStatus::Thinking);
    }

    #[test]
    fn turn_cancel_means_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"go"}],"time":1782300900000}"#,
                r#"{"type":"turn.cancel","time":1782300901000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions[0].status, SessionStatus::Waiting);
    }

    #[test]
    fn compaction_begin_means_compacting() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"turn.prompt","input":[{"type":"text","text":"go"}],"time":1782300900000}"#,
                r#"{"type":"full_compaction.begin","source":"manual","time":1782300901000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions[0].status, SessionStatus::Compacting);
    }

    #[test]
    fn compaction_complete_falls_through_to_waiting() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[
                r#"{"type":"usage.record","model":"m","usage":{"output":1},"time":1782300900000}"#,
                r#"{"type":"full_compaction.complete","time":1782300901000}"#,
            ],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        // full_compaction.complete 不是状态信号，继续前扫到 usage.record → Waiting
        assert_eq!(sessions[0].status, SessionStatus::Waiting);
    }

    #[test]
    fn non_matching_cwd_produces_no_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, _) = fixture_session(
            &home,
            "/work/demo",
            &[r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi"}],"time":1782300900000}"#],
        );
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, "/other/project")])
        });
        assert!(sessions.is_empty());
    }

    #[test]
    fn malformed_index_lines_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(
            &home,
            "/work/demo",
            &[r#"{"type":"turn.prompt","input":[{"type":"text","text":"hi"}],"time":1782300900000}"#],
        );
        // 追加一行损坏数据，解析应容忍
        fs::write(home.join("session_index.jsonl"), format!("not json\n{{\"sessionId\":\"11111111-1111-1111-1111-111111111111\",\"sessionDir\":\"{}\",\"workDir\":\"{}\"}}\n", home.join("sessions/wd_demo_0123456789ab/session_11111111-1111-1111-1111-111111111111").to_string_lossy(), work_dir)).unwrap();
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn empty_wire_falls_back_to_file_age() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("kimi-home");
        fs::create_dir_all(&home).unwrap();
        let (_, work_dir) = fixture_session(&home, "/work/demo", &[]);
        let sessions = run_with_home(&home, || {
            get_kimi_sessions(&[fake_process(1, &work_dir)])
        });
        // 空 wire 无信号：文件刚写入（<60s）→ Processing
        assert_eq!(sessions[0].status, SessionStatus::Processing);
        assert_eq!(sessions[0].last_message, None);
        assert_eq!(
            sessions[0].title.as_deref(),
            Some("Demo Session"),
            "state.json 标题可用"
        );
    }
}
