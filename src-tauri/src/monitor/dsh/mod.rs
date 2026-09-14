// dsh（DeepSeek harness）监控解析 — M1 只读底座
// 会话存储/事件语义全部依据 M0 探测报告（research/dsh-probe-2026-09-13-report.md）
// 双数据源：storages/session_projcache（首选）+ sessions/<项目>/<会话>/session.vN.jsonl.zstd（兜底）

pub mod decode;
pub mod log;
pub mod preview;
pub mod projcache;
pub mod status;

use crate::adapter::AgentProcess;
use crate::monitor::session_scan::SessionFileScan;
use crate::session::{AgentType, ProcessForm, Session, SessionStatus};

pub use status::LockState;

// L2 内容缓存（monitor::session_scan 预算层，AGENTS.md 新工具接入模板同款）：
// 代际日志的解析产物按 (mtime, size) 缓存——前端 3 秒轮询下未变化的历史会话
// 零解码零解析。M1 实测真机 77 会话/解压 155MB 每轮全量重扫 12.5s（debug），
// 远超 3s 轮询间隔且跑在 IPC 线程上，是 dev 模式整机卡顿的根因
const DSH_LOG_SCAN: SessionFileScan = SessionFileScan::new("dsh-log");

/// 单个会话代际日志的纯内容解析产物（不含 lock / 时间叠加，可安全跨轮询缓存）
#[derive(Debug, Clone)]
pub(crate) struct DshSessionDigest {
    pub(crate) header: log::DshHeader,
    pub(crate) is_subagent: bool,
    pub(crate) facts: status::TurnFacts,
    /// (role, text) 预览（真人输入/助手回复取 seq 更新者，注入已过滤）
    pub(crate) preview: (Option<String>, Option<String>),
    /// 日志侧标题（session/title 最新事件；projcache 优先的叠加在外层按轮现算）
    pub(crate) log_title: Option<String>,
    /// 事件流最大 time（未读锚点；None = 无 time 字段时外层兜底 mtime）
    pub(crate) max_event_time_ms: Option<i64>,
    /// 所选代际文件 mtime（静默兜底 / last_activity 兜底输入）
    pub(crate) log_mtime_ms: i64,
}

/// 读取（带 L2 缓存）代际日志解析产物：内容未变 → 同一 Arc 秒回；损坏/不可读
/// → 缓存 None（下次 mtime/size 变化时自动重试，与无缓存版"读不到→跳过"同语义）
fn load_digest(gen_path: &std::path::Path) -> std::sync::Arc<Option<DshSessionDigest>> {
    DSH_LOG_SCAN.parse(gen_path, build_digest)
}

/// 纯内容解析（session_scan.rs 要求：时间叠加绝不在此函数内）
fn build_digest(gen_path: &std::path::Path) -> Option<DshSessionDigest> {
    let bytes = std::fs::read(gen_path).ok()?;
    let text = if gen_path.extension().map(|e| e == "zstd").unwrap_or(false) {
        decode::decode_zstd_frames(&bytes).ok()?.text
    } else {
        String::from_utf8_lossy(&bytes).to_string()
    };
    let header = log::parse_header(&text)?;
    let events = log::parse_events(&text);
    let preview = preview::extract(&events);
    let log_title = preview::title(&events, None);
    let max_event_time_ms = events.iter().filter_map(|e| e.time).max();
    let log_mtime_ms = std::fs::metadata(gen_path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some(DshSessionDigest {
        is_subagent: log::is_subagent(&header),
        header,
        facts: status::scan_facts(&events),
        preview,
        log_title,
        max_event_time_ms,
        log_mtime_ms,
    })
}

/// dsh 数据根：$DSH_HOME 覆盖（M0 F14 优先级：env > ~/.dsh），测试注入用
pub fn dsh_home() -> std::path::PathBuf {
    std::env::var("DSH_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".dsh"))
}

/// dsh 宿主 cmdline 双令牌门（单源，M0 §5）：cmdline 含 "dsh"（精确令牌，
/// 或 /dsh、\dsh 路径结尾）与 "web"（精确令牌）即视为 dsh 宿主。dsh 宿主是
/// 前台终端启动的 node 进程，exe 判据不可用——find_dsh_processes（进程发现）
/// 与 host::tool_host_alive_in（宿主存活判定）共用本口径，防两处判定漂移
pub fn cmdline_is_dsh_host(cmd: &[std::ffi::OsString]) -> bool {
    let tokens: Vec<String> = cmd
        .iter()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    let has_dsh = tokens
        .iter()
        .any(|t| t == "dsh" || t.ends_with("/dsh") || t.ends_with("\\dsh"));
    let has_web = tokens.iter().any(|t| t == "web");
    has_dsh && has_web
}

/// 进程发现：node 进程且 cmdline 含 "dsh" 与 "web" 令牌（M0 §5：进程名是 node，
/// 必须按 cmdline 判定；取命中的第一个作为宿主——esbuild 等子进程 cmdline 无此二令牌）
pub fn find_dsh_processes(system: &sysinfo::System) -> Vec<AgentProcess> {
    let mut out = Vec::new();
    for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if cmd.is_empty() {
            continue;
        }
        // 判定口径单源委托 cmdline_is_dsh_host（host.rs 存活判定同款）
        if cmdline_is_dsh_host(cmd) {
            out.push(AgentProcess {
                pid: pid.as_u32(),
                cpu_usage: process.cpu_usage(),
                cwd: process.cwd().map(|p| p.to_path_buf()),
                exe: process.exe().map(|p| p.to_path_buf()),
                form: ProcessForm::App,
            });
            // 只需一个宿主（卡片 pid 用）——命中即取第一个，后续同名进程不再收集
            break;
        }
    }
    out
}

/// 会话锁持有探测（零干扰）：lock 文件不存在 → Unknown；有文件则问 lsof 是否有进程开着它
fn probe_lock_state(session_dir: &std::path::Path) -> LockState {
    let lock = session_dir.join("session.lock");
    if !lock.exists() {
        return LockState::Unknown; // v0 旧目录无锁文件（M0 F3）
    }
    let out = std::process::Command::new("lsof")
        .arg("-t")
        .arg(&lock)
        .output();
    match out {
        Ok(o)
            if o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty() =>
        {
            LockState::Held
        }
        Ok(_) => LockState::Free,    // 文件在但无人持有 → 写入者已死
        Err(_) => LockState::Unknown, // lsof 不可用（如 Windows）→ 静默兜底
    }
}

/// 会话聚合：宿主在位时扫描全部会话目录（含历史会话）出卡；未运行 → 无卡
pub fn get_dsh_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let Some(host) = processes.first() else {
        return Vec::new();
    };
    scan_sessions(&dsh_home(), host)
}

/// 内部扫描（home 注入，测试直调——避免 DSH_HOME 环境变量在并行测试中互踩）
fn scan_sessions(home: &std::path::Path, host: &AgentProcess) -> Vec<Session> {
    let sessions_root = home.join("sessions");
    let Ok(entries) = std::fs::read_dir(&sessions_root) else {
        return Vec::new();
    };
    let now_ms = chrono::Utc::now().timestamp_millis();

    let mut sessions = Vec::new();
    // 本轮扫描命中的代际日志全集（缓存收敛用，防已删会话的孤儿条目常驻）
    let mut live_logs: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for project_dir in entries.flatten() {
        let ppath = project_dir.path();
        if !ppath.is_dir() {
            continue;
        }
        let name = project_dir.file_name().to_string_lossy().to_string();
        if name.starts_with("session_projcache") || name == "workspace.json" {
            continue;
        }
        let Ok(session_dirs) = std::fs::read_dir(&ppath) else {
            continue;
        };
        for sdir in session_dirs.flatten() {
            let spath = sdir.path();
            if !spath.is_dir() {
                continue;
            }
            // 代际选择（取代际最大者）+ L2 缓存解析（读不到/解不开 → 跳过该会话不影响其他）
            let Some((version, gen_path)) = log::generation_logs(&spath).pop() else {
                continue;
            };
            let Some(digest) = load_digest(&gen_path).as_ref().clone() else {
                continue;
            };
            live_logs.insert(gen_path);
            let header = &digest.header;
            if digest.is_subagent {
                continue; // 子 Agent 不出卡（M0 F7）
            }
            // 版本门（设计 P5 + 备忘 A8）：header.version 超出已知集（0..=3）→
            // 降级卡"格式待适配"（未知语义不猜——探测红线），不影响其他会话
            if let Some(v) = header.version {
                if !(0..=3).contains(&v) {
                    ::log::warn!("dsh: 会话 {} 为未知代际 v{}，出降级卡", header.id, v);
                    sessions.push(Session {
                        id: header.id.clone(),
                        agent_type: AgentType::Dsh,
                        project_name: header
                            .cwd
                            .as_deref()
                            .map(crate::monitor::project::project_name_from_path)
                            .unwrap_or_else(|| name.clone()),
                        project_path: header.cwd.clone().unwrap_or_default(),
                        title: Some(format!("dsh 格式待适配（v{}）", v)),
                        git_branch: None,
                        github_url: None,
                        status: SessionStatus::Idle,
                        last_message: None,
                        last_message_role: None,
                        last_activity_at: chrono::DateTime::from_timestamp_millis(
                            digest.log_mtime_ms,
                        )
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_default(),
                        pid: host.pid,
                        cpu_usage: host.cpu_usage,
                        active_subagent_count: 0,
                        form: ProcessForm::App,
                        jump_supported: crate::session::jump_supported_for(ProcessForm::App),
                        unread: false,
                    });
                    continue;
                }
            }

            // 双源：projcache（identity 过校验才可用；小文件每轮直读——量级远低于日志）
            let cache = projcache::load(home, &header.id)
                .filter(|_| projcache_identity_ok(home, header, version));

            // 状态：lock 交叉判定 + 静默兜底。lock 仅开裔回合才探测（lsof 子进程，
            // 终审 I2）——闭合回合的 derive 不消费 lock，探测是纯浪费
            let lock = if digest.facts.has_open_turn() {
                probe_lock_state(&spath)
            } else {
                LockState::Unknown
            };
            let silence_ms = now_ms - digest.log_mtime_ms;
            let outcome = status::derive_facts(&digest.facts, lock, Some(silence_ms));

            let (role, text) = digest.preview.clone();
            let title = cache
                .as_ref()
                .and_then(|c| c.title.clone())
                .or_else(|| digest.log_title.clone());
            let last_activity_ms = cache
                .as_ref()
                .and_then(|c| c.last_prompt_at)
                .or(digest.max_event_time_ms)
                .unwrap_or(digest.log_mtime_ms);

            // 未读（设计 P2 定死：等批准/出错/刚完成/回合被阻塞 → 未读）。
            // 锚点用事件流最大 time——勿用 lastPromptAt（完成晚于提问，
            // 用提问时刻会漏掉"读后完成"的刚完成未读）
            let unread_anchor_ms = digest.max_event_time_ms.unwrap_or(digest.log_mtime_ms);
            // MAM 自有库读已读水位（绝不写 ~/.dsh）；按会话短锁即取即放，
            // 避免长扫描（zstd 解码 + lsof 子进程）期间独占全局连接
            let last_read = {
                let conn = crate::database::connection::DB.lock().unwrap();
                crate::database::dao::dsh_read::last_read_at(&conn, &header.id).unwrap_or(0)
            };
            let unread = (matches!(
                outcome.status,
                SessionStatus::Waiting | SessionStatus::Finished
            ) || outcome.end_kind.as_deref() == Some("blocked"))
                && unread_anchor_ms > last_read;

            sessions.push(Session {
                id: header.id.clone(),
                agent_type: AgentType::Dsh,
                project_name: header
                    .cwd
                    .as_deref()
                    .map(crate::monitor::project::project_name_from_path)
                    .unwrap_or_else(|| name.clone()),
                project_path: header.cwd.clone().unwrap_or_default(),
                title,
                git_branch: None,
                github_url: None,
                status: outcome.status,
                last_message: text,
                last_message_role: role,
                last_activity_at: chrono::DateTime::from_timestamp_millis(last_activity_ms)
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default(),
                pid: host.pid,
                cpu_usage: host.cpu_usage,
                active_subagent_count: 0,
                form: ProcessForm::App,
                jump_supported: crate::session::jump_supported_for(ProcessForm::App),
                unread,
            });
        }
    }
    // 收敛缓存：清掉本轮未命中的代际日志条目（会话目录已被删除等）
    DSH_LOG_SCAN.retain_existing(&live_logs);
    sessions
}

/// projcache identity 校验（5 字段；坏记录路径在此收敛）
fn projcache_identity_ok(home: &std::path::Path, header: &log::DshHeader, version: i64) -> bool {
    let path = home
        .join("storages/session_projcache/sessions")
        .join(format!("{}.json", header.id));
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    v.get("record")
        .and_then(|r| r.get("identity"))
        .map(|ident| projcache::identity_matches(ident, header, version))
        .unwrap_or(false)
}

// ===== 单元测试：宿主 cmdline 双令牌门（C1 终审：host.rs 存活判定同源口径的防漂移锁）=====
#[cfg(test)]
mod cmdline_gate_tests {
    use super::cmdline_is_dsh_host;
    use std::ffi::OsString;

    fn cmd(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn dual_tokens_qualify_as_host() {
        // 精确双令牌（node + dsh + web）
        assert!(cmdline_is_dsh_host(&cmd(&["node", "/usr/local/bin/dsh", "web"])));
        assert!(cmdline_is_dsh_host(&cmd(&["node", "dsh", "web"])));
        // 令牌顺序无关（extra 参数不影响；但脚本路径须以 /dsh、\dsh 结尾或恰为
        // "dsh"——/opt/dsh/cli.js 这类路径中段形态不算，见 substring 用例）
        assert!(cmdline_is_dsh_host(&cmd(&["/usr/local/bin/node", "web", "dsh"])));
        assert!(cmdline_is_dsh_host(&cmd(&["node", "/opt/dsh", "web", "--port=4173"])));
    }

    #[test]
    fn path_suffix_dsh_qualifies() {
        // 路径结尾 /dsh（POSIX）与 \dsh（Windows）均算 dsh 令牌
        assert!(cmdline_is_dsh_host(&cmd(&["node", "/opt/dsh/bin/dsh", "web"])));
        assert!(cmdline_is_dsh_host(&cmd(&["node", "C:\\tools\\dsh\\bin\\dsh", "web"])));
    }

    #[test]
    fn single_token_is_not_host() {
        // 只含其一 → 非宿主（esbuild 等子进程形态）
        assert!(!cmdline_is_dsh_host(&cmd(&["node", "/opt/dsh/bin/dsh"])));
        assert!(!cmdline_is_dsh_host(&cmd(&["node", "web"])));
        assert!(!cmdline_is_dsh_host(&cmd(&[])));
    }

    #[test]
    fn substring_tokens_do_not_qualify() {
        // "dshweb" 子串不构成 dsh 令牌、"webview" 子串不构成 web 令牌（防误匹配）
        assert!(!cmdline_is_dsh_host(&cmd(&["node", "dshweb", "web"])));
        assert!(!cmdline_is_dsh_host(&cmd(&["node", "dsh", "webview"])));
        // 子串出现在路径中间同样不算（仅路径结尾 /dsh|\dsh 认可）
        assert!(!cmdline_is_dsh_host(&cmd(&["node", "/opt/dsh-web/cli.js", "web"])));
    }
}

// ===== 集成测试（Task 8）：会话编排流水线（home 注入直调，不触真机 ~/.dsh）=====
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::session::{ProcessForm, SessionStatus};

    /// 造一个隔离 dsh home：一个项目 + 一个会话（zstd 单帧事件）
    fn make_home(events: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let sess = dir.path().join("sessions/--tmp-proj--/session-abc");
        std::fs::create_dir_all(&sess).unwrap();
        // JSON 花括号不能进 format! 格式串——header 行用普通字面量变量拼接
        let header_line = "{\"type\":\"session\",\"version\":3,\"id\":\"session-abc\",\"cwd\":\"/tmp/proj\",\"createdAt\":1000,\"isSeeded\":false}";
        let frame = zstd::stream::encode_all(format!("{header_line}\n{events}").as_bytes(), 3)
            .unwrap();
        std::fs::write(sess.join("session.v3.jsonl.zstd"), &frame).unwrap();
        dir
    }

    fn fake_host() -> AgentProcess {
        AgentProcess {
            pid: 42,
            cpu_usage: 0.5,
            cwd: None,
            exe: None,
            form: ProcessForm::App,
        }
    }

    #[test]
    fn emits_card_with_status_and_preview() {
        let home = make_home(
            "{\"type\":\"turn/start\",\"seq\":4,\"data\":{}}\n\
             {\"type\":\"user/message\",\"seq\":8,\"data\":{\"content\":[{\"type\":\"text\",\"text\":\"hi\"}],\"source\":{\"kind\":\"user\"}}}\n\
             {\"type\":\"assistant/message\",\"seq\":9,\"data\":{\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}}\n\
             {\"type\":\"turn/end\",\"seq\":10,\"data\":{\"reason\":{\"kind\":\"completed\"}}}\n",
        );
        let sessions = scan_sessions(home.path(), &fake_host());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.id, "session-abc");
        assert_eq!(s.agent_type, crate::session::AgentType::Dsh);
        assert_eq!(s.status, SessionStatus::Finished);
        assert_eq!(s.last_message.as_deref(), Some("done"));
        assert_eq!(s.last_message_role.as_deref(), Some("assistant"));
        assert_eq!(s.form, ProcessForm::App);
        assert_eq!(s.pid, 42);
        assert!(s.unread, "刚完成且从未读过 → 未读");
    }

    #[test]
    fn skips_subagent_and_missing_host() {
        // 子 Agent 会话不出卡
        let dir = tempfile::tempdir().unwrap();
        let sess = dir.path().join("sessions/--tmp-proj--/3b8a0933-0000-0000-0000-000000000000");
        std::fs::create_dir_all(&sess).unwrap();
        let frame = zstd::stream::encode_all(
            b"{\"type\":\"session\",\"version\":0,\"id\":\"sub-1\",\"cwd\":\"/tmp\",\"origin\":\"subagent\",\"delegationDepth\":1}\n".as_slice(),
            3).unwrap();
        std::fs::write(sess.join("session.jsonl.zstd"), &frame).unwrap();
        let with_host = scan_sessions(dir.path(), &fake_host());
        assert!(with_host.is_empty(), "子 Agent 过滤");
        // 宿主不在 → 无卡（与其他工具一致；不入扫描，无需 home）
        assert!(get_dsh_sessions(&[]).is_empty());
    }

    #[test]
    fn digest_cache_reuses_unchanged_log() {
        // L2 内容缓存（session_scan.rs 预算层）：(mtime,size) 未变 ⇒ 复用同一份
        // 解析产物 Arc——3 秒轮询下 77 个历史会话不再每轮全量解码
        let home = make_home(
            "{\"type\":\"turn/start\",\"seq\":4,\"data\":{}}\n\
             {\"type\":\"turn/end\",\"seq\":6,\"data\":{\"reason\":{\"kind\":\"completed\"}}}\n",
        );
        let dir = home.path().join("sessions/--tmp-proj--/session-abc");
        let (_, gen) = log::generation_logs(&dir).pop().unwrap();
        let first = load_digest(&gen);
        let second = load_digest(&gen);
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "文件未变应命中缓存返回同一 Arc"
        );
    }

    #[test]
    fn digest_invalidates_when_log_appends() {
        // 追加一帧（size 变化）→ 缓存必须失效：completed → error，Finished → Waiting
        let home = make_home(
            "{\"type\":\"turn/start\",\"seq\":4,\"data\":{}}\n\
             {\"type\":\"turn/end\",\"seq\":6,\"data\":{\"reason\":{\"kind\":\"completed\"}}}\n",
        );
        let host = fake_host();
        assert_eq!(
            scan_sessions(home.path(), &host)[0].status,
            SessionStatus::Finished
        );
        let dir = home.path().join("sessions/--tmp-proj--/session-abc");
        let (_, gen) = log::generation_logs(&dir).pop().unwrap();
        let mut bytes = std::fs::read(&gen).unwrap();
        bytes.extend_from_slice(
            &zstd::stream::encode_all(
                b"{\"type\":\"turn/end\",\"seq\":9,\"data\":{\"reason\":{\"kind\":\"error\"}}}\n"
                    .as_slice(),
                3,
            )
            .unwrap(),
        );
        std::fs::write(&gen, &bytes).unwrap();
        let cards = scan_sessions(home.path(), &host);
        assert_eq!(cards[0].status, SessionStatus::Waiting, "追加 error 帧后应重扫");
    }
}

