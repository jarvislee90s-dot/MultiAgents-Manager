// WorkBuddy 会话解析：心跳文件（~/.workbuddy/sessions/<PID>.json）关联进程与会话，
// 会话历史在 ~/.workbuddy/projects/<路径编码>/<sessionId>.jsonl（OpenAI 风格 type/role/content）
// 所有文件均为未文档化私有格式：解析失败一律跳过/降级，禁止 panic（spec W3 防御性要求）

use super::git::get_github_url;
use super::jsonl::read_recent_lines;
use super::project::project_name_from_path;
use crate::adapter::AgentProcess;
use crate::session::{jump_supported_for, AgentType, ProcessForm, Session, SessionStatus};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 心跳新鲜阈值：取 MAM 轮询周期（约 30s）的 3 倍，防止轮询间隙卡片闪烁
pub const HEARTBEAT_FRESH_MS: u64 = 90_000;

/// App 形态状态叠加阈值（spec §4「叠加 mtime 阈值（App 形态 300s，与 Codex APP 一致）」）：
/// JSONL mtime 停更超过该时长时，函数调用类尾部（Processing）降级为 Waiting
pub const APP_STATUS_STALE_MS: u64 = 300_000;

/// 每轮观测到的 pid → sessionId（Task 10 心跳消失补偿的依据）
pub static LAST_SEEN_SESSIONS: Lazy<Mutex<HashMap<u32, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct Heartbeat {
    pub pid: u32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub cwd: String,
    #[serde(rename = "lastHeartbeat")]
    pub last_heartbeat_ms: u64,
    /// 会话类型（serve/prewarm/interactive 等）；字段缺失视为通过（防御私有格式演进）
    #[serde(default)]
    pub kind: Option<String>,
}

pub fn parse_heartbeat(json: &str) -> Option<Heartbeat> {
    serde_json::from_str(json).ok()
}

/// ASCII hex 字符判断（大小写均可）
fn is_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || (b'A'..=b'F').contains(&b)
}

/// sessionId 严格 UUID 形态判定：8-4-4-4-12 五段、每段均为 ASCII hex。
/// 纯字节实现（不引入 regex 依赖）。prewarm 池的 `prewarm-wb-pool-<13位ms>-<6位hex>`
/// 恰为 36 字符 4 连字符，仅凭「长度 36 + 连字符 4」判定会被骗过——必须逐段校验 hex 字符集
pub fn heartbeat_session_id_is_uuid(hb: &Heartbeat) -> bool {
    let id = hb.session_id.as_bytes();
    if id.len() != 36 {
        return false;
    }
    // 五段长度：8-4-4-4-12（合计 32 个 hex + 4 个连字符）
    let segs = [8usize, 4, 4, 4, 12];
    let mut pos = 0usize;
    for (i, len) in segs.iter().enumerate() {
        let end = pos + len;
        if !id[pos..end].iter().all(|&b| is_hex(b)) {
            return false;
        }
        pos = end;
        if i < segs.len() - 1 {
            if id.get(pos) != Some(&b'-') {
                return false;
            }
            pos += 1;
        }
    }
    true
}

pub fn heartbeat_is_alive(hb: &Heartbeat, now_ms: u64) -> bool {
    now_ms.saturating_sub(hb.last_heartbeat_ms) < HEARTBEAT_FRESH_MS
}

/// 项目路径编码：去首分隔符后 / 与 \ 统一替换为 -
pub fn mangle_project_path(cwd: &str) -> String {
    let trimmed = cwd.trim_start_matches('/');
    trimmed.replace(['/', '\\'], "-")
}

pub fn session_jsonl_path(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    home.join(".workbuddy")
        .join("projects")
        .join(mangle_project_path(cwd))
        .join(format!("{}.jsonl", session_id))
}

/// mtime 阈值叠加（spec §4）：函数调用类尾部停更 >= 300s 降级 Waiting；
/// assistant 文本（Idle）等其余状态不受影响。mtime 年龄不可知时按未过期处理（防御）
pub fn overlay_mtime_stale(status: SessionStatus, mtime_age_ms: u64) -> SessionStatus {
    match status {
        SessionStatus::Processing if mtime_age_ms >= APP_STATUS_STALE_MS => SessionStatus::Waiting,
        other => other,
    }
}

/// JSONL 尾部状态推导：最后一条有效条目决定状态（spec W3 映射）
pub fn derive_status_from_tail(lines: &[String]) -> SessionStatus {
    let mut last: Option<&String> = None;
    for line in lines {
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) if v.get("type").is_some() => last = Some(line),
            _ => continue,
        }
    }
    let Some(line) = last else {
        return SessionStatus::Waiting;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return SessionStatus::Waiting;
    };
    match v["type"].as_str().unwrap_or_default() {
        "message" => match v["role"].as_str().unwrap_or_default() {
            "user" => SessionStatus::Thinking,
            _ => SessionStatus::Idle, // assistant 完成
        },
        "function_call" | "function_call_result" => SessionStatus::Processing,
        // reasoning/file-history-snapshot 等中间条目按运行中处理
        _ => SessionStatus::Processing,
    }
}

/// 会话标题：只读打开 workbuddy.db 读 sessions.title；失败降级 None（调用方再降级首条 user 消息）
pub fn title_from_db(home: &Path, session_id: &str) -> Option<String> {
    use rusqlite::OpenFlags;
    let db = home.join(".workbuddy").join("workbuddy.db");
    let conn = rusqlite::Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT title FROM sessions WHERE id = ?1 AND deleted_at IS NULL",
        [session_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .filter(|t| !t.trim().is_empty())
}

fn heartbeat_path(home: &Path, pid: u32) -> PathBuf {
    home.join(".workbuddy")
        .join("sessions")
        .join(format!("{}.json", pid))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 主入口：活跃心跳的 WorkBuddy 进程 → 每会话一张卡
pub fn get_workbuddy_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    let mut sessions = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return sessions;
    };
    let now = now_ms();

    for process in processes {
        // 防御：心跳文件缺失/损坏 → 跳过该进程（含独立 CLI、空闲 prewarm）
        let Some(hb) = std::fs::read_to_string(heartbeat_path(&home, process.pid))
            .ok()
            .and_then(|s| parse_heartbeat(&s))
        else {
            continue;
        };
        // 过滤：严格 UUID 形态（真实任务会话）+ 心跳新鲜 + kind 非 prewarm（双保险，字段缺失视为通过）
        if !heartbeat_session_id_is_uuid(&hb)
            || hb.kind.as_deref() == Some("prewarm")
            || !heartbeat_is_alive(&hb, now)
        {
            continue;
        }

        let jsonl = session_jsonl_path(&home, &hb.cwd, &hb.session_id);
        if !jsonl.exists() {
            continue; // 会话文件未落盘（防御）
        }

        // 尾部解析（复用通用 JSONL 尾读设施；行数与 codex 一致 500）
        let lines = read_recent_lines(&jsonl, 500);
        // JSONL mtime（epoch 毫秒）只取一次，供状态叠加与 last_activity_at 复用
        let jsonl_mtime_ms = jsonl
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);
        // 叠加 App 形态 mtime 阈值（spec §4：App 形态 300s，与 Codex APP 一致）——
        // 函数调用尾部停更 >= 300s 视为等待而非运行中；mtime 缺失按未过期处理（防御）
        let mtime_age_ms = jsonl_mtime_ms.map_or(0, |m| now.saturating_sub(m));
        let status = overlay_mtime_stale(derive_status_from_tail(&lines), mtime_age_ms);
        let last_message = lines
            .iter()
            .rev()
            .find_map(|l| extract_message_text(l))
            .unwrap_or_default();

        let title = title_from_db(&home, &hb.session_id)
            .or_else(|| first_user_text(&lines))
            .map(|t| t.chars().take(60).collect::<String>());

        sessions.push(Session {
            id: hb.session_id.clone(),
            agent_type: AgentType::WorkBuddy,
            project_name: project_name_from_path(&hb.cwd),
            project_path: hb.cwd.clone(),
            title,
            git_branch: None,
            github_url: get_github_url(&hb.cwd),
            status,
            last_message: if last_message.is_empty() {
                None
            } else {
                Some(last_message)
            },
            last_message_role: None,
            last_activity_at: jsonl_mtime_ms
                .map(|ms| {
                    chrono::DateTime::from_timestamp((ms / 1000) as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
                .unwrap_or_default(),
            pid: process.pid,
            cpu_usage: process.cpu_usage,
            active_subagent_count: 0,
            form: ProcessForm::App,
            jump_supported: jump_supported_for(ProcessForm::App),
            unread: false, // 扫描出的活跃卡默认非未读；未读卡由 adapter 层合并
        });

        // 记录本轮 pid→session（心跳消失补偿依据）
        LAST_SEEN_SESSIONS
            .lock()
            .unwrap()
            .insert(process.pid, hb.session_id.clone());
    }
    sessions
}

/// 提取 message 条目 content 数组中首个非空 text 片段
fn extract_message_text(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["type"].as_str()? != "message" {
        return None;
    }
    v["content"]
        .as_array()?
        .iter()
        .find_map(|c| {
            c.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.trim().is_empty())
}

/// 降级标题：首条 user 消息文本（DB 查询失败时使用）
fn first_user_text(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            if v["type"].as_str() == Some("message") && v["role"].as_str() == Some("user") {
                extract_message_text(l)
            } else {
                None
            }
        })
        .next()
}

/// 补偿核心（spec W4 / §8 可测试）：判定 last_seen 中心跳已消失的 pid（文件缺失或过期），
/// 读其 JSONL 终态——完成 → 产出待插入的未读记录；运行中被杀 → 不产出。
/// 返回补偿产物并由调用方落库（DAO 注入点，测试可断言产物而不触库）；
/// 同时从 last_seen 移除已消失条目（未消失条目保留供下轮参考）
pub fn compensate_vanished_heartbeats_in(
    home: &Path,
    now_ms: u64,
    last_seen: &Mutex<HashMap<u32, String>>,
    status_of: &dyn Fn(&str) -> Option<String>,
) -> Vec<crate::database::dao::unread::UnreadSessionRecord> {
    let mut compensated = Vec::new();

    // 锁内只做纯内存快照（锁 hygiene：绝不持锁跨文件 I/O），判定消失在锁外进行
    let candidates: Vec<(u32, String)> = {
        let last_seen = last_seen.lock().unwrap();
        last_seen
            .iter()
            .map(|(pid, sid)| (*pid, sid.clone()))
            .collect()
    };
    let vanished: Vec<(u32, String)> = candidates
        .into_iter()
        .filter(|(pid, _)| {
            // 心跳文件没了 = 回池/退出；过期同样视为消失
            match std::fs::read_to_string(heartbeat_path(home, *pid))
                .ok()
                .and_then(|s| parse_heartbeat(&s))
            {
                Some(hb) => !heartbeat_is_alive(&hb, now_ms),
                None => true,
            }
        })
        .collect();

    for (pid, session_id) in vanished {
        // 逐个短暂重锁移除（不做额外清理，未消失条目保留供下轮参考）
        last_seen.lock().unwrap().remove(&pid);
        // 找该会话的 JSONL：遍历 projects 下所有 <sessionId>.jsonl（会话可能换过项目目录）
        let projects_dir = home.join(".workbuddy").join("projects");
        let Ok(entries) = std::fs::read_dir(&projects_dir) else {
            continue; // 防御：目录读取失败只跳过该 pid，不中断其余补偿
        };
        let target = entries.filter_map(|e| e.ok()).find_map(|dir| {
            let p = dir.path().join(format!("{}.jsonl", session_id));
            p.exists().then_some(p)
        });
        let Some(jsonl) = target else { continue };
        let lines = read_recent_lines(&jsonl, 500);
        if derive_status_from_tail(&lines) != SessionStatus::Idle {
            continue; // 非完成态（运行中被杀等）→ 不补
        }
        // review M1：状态缓存已记录「绿已被 sync 观测」（Idle/Finished）时，行缺席
        // 是因为用户已读删行——补偿不得复活（否则一次性复活未读卡）
        if matches!(
            status_of(&session_id).as_deref(),
            Some("Idle") | Some("Finished")
        ) {
            continue;
        }
        // cwd 直接用 WorkBuddy DB 的 cwd 字段反查
        let cwd = workbuddy_cwd_from_db(home, &session_id).unwrap_or_default();
        let last_message = lines.iter().rev().find_map(|l| extract_message_text(l));
        compensated.push(crate::database::dao::unread::UnreadSessionRecord {
            tool_id: "workbuddy".into(),
            session_id: session_id.clone(),
            project_name: if cwd.is_empty() {
                "WorkBuddy".into()
            } else {
                project_name_from_path(&cwd)
            },
            title: title_from_db(home, &session_id),
            last_message,
            // 以补偿时刻为转绿时间：转绿从未被观测，此刻即首绿
            turned_green_at_ms: now_ms as i64,
            expires_at_ms: now_ms as i64 + 24 * 3600 * 1000,
        });
    }
    compensated
}

/// 主入口：真实 home / 真实时钟 / 全局 LAST_SEEN_SESSIONS 的薄包装（补偿行在此落库）。
/// 注：DAO upsert 冲突时仅刷新展示字段、保留原 turned_green_at/expires_at（见
/// `upsert_unread`）——对已存在的行此处只起补展示快照作用；仅当行不存在（转绿从未
/// 被观测）时插入值生效，符合 spec §5「转绿时间」语义
pub fn compensate_vanished_heartbeats() {
    // W5 门禁：工具已停用则不做补偿（enabled 是 W5 单一事实源）。否则停用后任务随即完成、
    // prewarm 回池删除心跳文件时，本函数会为已停用工具 upsert 未读行，「复活」未读卡并
    // 触发完成通知，违反 spec W5「彻底隐藏/通知静音」。读 DB 真实启用态；集成级路径
    // （GUI 阶段验证），纯函数层 compensate_vanished_heartbeats_in 保持不触库
    if !crate::database::dao::agent_tool::get_tool_enabled("workbuddy") {
        return;
    }
    let Some(home) = dirs::home_dir() else { return };
    let now = now_ms();
    for record in compensate_vanished_heartbeats_in(&home, now, &LAST_SEEN_SESSIONS, &|sid| {
        crate::database::find_status(sid)
    }) {
        crate::database::dao::unread::upsert(&record);
    }
}

/// 补偿用：只读打开 workbuddy.db 反查会话 cwd；列缺失/打开失败一律 None（防御）
fn workbuddy_cwd_from_db(home: &Path, session_id: &str) -> Option<String> {
    use rusqlite::OpenFlags;
    let db = home.join(".workbuddy").join("workbuddy.db");
    let conn = rusqlite::Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.query_row(
        "SELECT cwd FROM sessions WHERE id = ?1 AND deleted_at IS NULL",
        [session_id],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEARTBEAT_ACTIVE: &str = r#"{
      "pid": 11952,
      "lastHeartbeat": 1788444900119,
      "sessionId": "7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c",
      "cwd": "/Users/jarvis/Documents/MultiAgents-Manager",
      "startedAt": 1788444900112,
      "kind": "interactive",
      "updatedAt": 1788444900347
    }"#;

    const HEARTBEAT_SERVE: &str = r#"{
      "pid": 8979,
      "lastHeartbeat": 1788445813951,
      "sessionId": "interactive-8979",
      "cwd": "/private/var/folders/xx/T/workbuddy-host-cli/xxx",
      "kind": "interactive",
      "url": "http://127.0.0.1:50027"
    }"#;

    // Windows 实测 prewarm 池样本（附录 A）：sessionId 恰为 36 字符 4 连字符，
    // 仅凭「长度+连字符计数」会被误判为 UUID；须逐段 hex 校验拒绝 + kind=prewarm 双保险
    const HEARTBEAT_PREWARM: &str = r#"{
      "pid": 17692,
      "lastHeartbeat": 1788496419201,
      "sessionId": "prewarm-wb-pool-1788496419201-bb1050",
      "cwd": "C:\\Users\\bunny\\WorkBuddy",
      "kind": "prewarm",
      "meta": {"status": "idle"}
    }"#;

    #[test]
    fn mangle_strips_leading_slash_and_replaces_separators() {
        assert_eq!(
            mangle_project_path("/Users/jarvis/Documents/MultiAgents-Manager"),
            "Users-jarvis-Documents-MultiAgents-Manager"
        );
        // Windows 形态容错：反斜杠路径同样编码
        assert_eq!(
            mangle_project_path("C:\\Users\\jarvis\\proj"),
            "C:-Users-jarvis-proj"
        );
    }

    #[test]
    fn heartbeat_uuid_session_is_real_task() {
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert_eq!(hb.pid, 11952);
        assert!(heartbeat_session_id_is_uuid(&hb));
        let serve = parse_heartbeat(HEARTBEAT_SERVE).unwrap();
        assert!(!heartbeat_session_id_is_uuid(&serve)); // --serve 排除
    }

    // ---- 严格 UUID 形态判定（P0-3）：prewarm 池 36 字符/4 连字符骗不过逐段 hex 校验 ----

    #[test]
    fn uuid_accepts_real_and_uppercase_hex() {
        // 真实任务会话样本（Windows 实测，附录 A）
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert!(heartbeat_session_id_is_uuid(&hb));
        // 全大写 hex 同样合法（UUID 不区分大小写）
        let upper = Heartbeat {
            pid: 1,
            session_id: "ECBF3D35-76E9-42DF-B71D-89409EC156EA".into(),
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: None,
        };
        assert!(heartbeat_session_id_is_uuid(&upper));
    }

    #[test]
    fn uuid_rejects_prewarm_pool_pseudo_uuid() {
        // Windows 实测样本：`prewarm-wb-pool-<13位ms>-<6位hex>` 恰为 36 字符 4 连字符，
        // 旧「长度 36 + 连字符 4」判定会误放行——逐段 hex 校验必须拒绝
        let hb = parse_heartbeat(HEARTBEAT_PREWARM).unwrap();
        assert_eq!(hb.session_id.len(), 36);
        assert_eq!(hb.session_id.bytes().filter(|c| *c == b'-').count(), 4);
        assert!(!heartbeat_session_id_is_uuid(&hb));
    }

    #[test]
    fn uuid_rejects_interactive_serve_id() {
        let serve = parse_heartbeat(HEARTBEAT_SERVE).unwrap();
        assert!(!heartbeat_session_id_is_uuid(&serve)); // interactive-<pid> 排除
    }

    #[test]
    fn uuid_rejects_non_hex_segment() {
        // 8-4-4-4-12 形态但含非 hex 字符（如 g/h 等超出 a-f 的字母）→ 拒绝
        let bad = Heartbeat {
            pid: 1,
            session_id: "ecbf3d35-76e9-42df-b71d-89409ec156ea".into(),
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: None,
        };
        assert!(heartbeat_session_id_is_uuid(&bad));
        let g8hh = Heartbeat {
            pid: 1,
            session_id: "g8hh3d35-76e9-42df-b71d-89409ec156ea".into(),
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: None,
        };
        assert!(!heartbeat_session_id_is_uuid(&g8hh)); // 首段含 g（非 hex）
        // 连字符位置错误：8-4-4-4-12 的分段长度不对 → 拒绝
        let wrong_segs = Heartbeat {
            pid: 1,
            session_id: "ecbf3d35-76e9-42df-b71d-89409ec156e".into(), // 末段 11 字符
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: None,
        };
        assert!(!heartbeat_session_id_is_uuid(&wrong_segs));
    }

    // ---- kind 防御（P0-3 双保险）：kind=prewarm 拒绝，缺失视为通过 ----

    #[test]
    fn kind_prewarm_is_filtered_out() {
        // 即使 sessionId 真为 UUID 形态，kind=prewarm 也必须排除（双保险防线独立生效）：
        // 私有格式演进后 prewarm 若改用 UUID 命名，严格 UUID 判定会放行，kind 仍能拦截
        let prewarm_uuid_shaped = Heartbeat {
            pid: 1,
            session_id: "ecbf3d35-76e9-42df-b71d-89409ec156ea".into(),
            cwd: "C:\\Users\\bunny\\WorkBuddy".into(),
            last_heartbeat_ms: 0,
            kind: Some("prewarm".into()),
        };
        assert!(heartbeat_session_id_is_uuid(&prewarm_uuid_shaped));
        assert!(prewarm_uuid_shaped.kind.as_deref() == Some("prewarm"));
        // 真实 prewarm 样本本身也不满足严格 UUID（段长 7-2-4-13-6）
        let hb = parse_heartbeat(HEARTBEAT_PREWARM).unwrap();
        assert!(!heartbeat_session_id_is_uuid(&hb));
        assert!(hb.kind.as_deref() == Some("prewarm"));
    }

    #[test]
    fn kind_missing_is_allowed() {
        // 字段缺失（旧格式/演进防御）视为通过
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert!(hb.kind.is_some()); // 现行格式带 kind
        let no_kind = Heartbeat {
            pid: 1,
            session_id: "ecbf3d35-76e9-42df-b71d-89409ec156ea".into(),
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: None,
        };
        assert!(no_kind.kind.is_none());
        assert!(heartbeat_session_id_is_uuid(&no_kind));
        // 非 prewarm 的 kind（interactive）放行
        let interactive = Heartbeat {
            pid: 1,
            session_id: "ecbf3d35-76e9-42df-b71d-89409ec156ea".into(),
            cwd: "/tmp".into(),
            last_heartbeat_ms: 0,
            kind: Some("interactive".into()),
        };
        assert!(interactive.kind.as_deref() != Some("prewarm"));
    }

    #[test]
    fn heartbeat_parse_rejects_garbage() {
        assert!(parse_heartbeat("not json").is_none());
        assert!(parse_heartbeat("{}").is_none()); // 缺 sessionId
    }

    #[test]
    fn heartbeat_freshness() {
        let hb = parse_heartbeat(HEARTBEAT_ACTIVE).unwrap();
        assert!(heartbeat_is_alive(&hb, hb.last_heartbeat_ms + 1));
        assert!(!heartbeat_is_alive(
            &hb,
            hb.last_heartbeat_ms + HEARTBEAT_FRESH_MS + 1
        ));
    }

    #[test]
    fn session_jsonl_path_layout() {
        let p = session_jsonl_path(
            std::path::Path::new("/home/u"),
            "/Users/jarvis/Documents/MultiAgents-Manager",
            "7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c",
        );
        assert_eq!(
            p,
            std::path::PathBuf::from(
                "/home/u/.workbuddy/projects/Users-jarvis-Documents-MultiAgents-Manager/7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c.jsonl"
            )
        );
    }

    #[test]
    fn tail_user_message_is_thinking() {
        let lines = vec![
            r#"{"type":"message","role":"user","content":[{"type":"input_text","text":"跑测试"}]}"#
                .into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Thinking);
    }

    #[test]
    fn tail_function_call_is_processing() {
        let lines = vec![r#"{"type":"function_call","name":"shell"}"#.into()];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Processing);
    }

    #[test]
    fn tail_assistant_text_is_idle() {
        let lines = vec![
            r#"{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"完成"}]}"#.into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Idle);
    }

    #[test]
    fn tail_last_entry_wins() {
        let lines = vec![
            r#"{"type":"function_call","name":"shell"}"#.into(),
            r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"好"}]}"#.into(),
        ];
        assert_eq!(derive_status_from_tail(&lines), SessionStatus::Idle);
    }

    #[test]
    fn tail_empty_is_waiting() {
        assert_eq!(derive_status_from_tail(&[]), SessionStatus::Waiting);
    }

    // ---- App 形态 mtime 阈值叠加（spec §4，与 Codex APP 语义一致）----

    #[test]
    fn processing_stale_downgrades_to_waiting() {
        // 函数调用类尾部 + JSONL 停更 >= 300s → Processing 降级 Waiting
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS),
            SessionStatus::Waiting
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS + 1),
            SessionStatus::Waiting
        );
    }

    #[test]
    fn processing_fresh_stays_processing() {
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, APP_STATUS_STALE_MS - 1),
            SessionStatus::Processing
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Processing, 0),
            SessionStatus::Processing
        );
    }

    #[test]
    fn idle_stays_idle_regardless_of_mtime() {
        // assistant 纯文本尾部是明确完成信号：文件过旧也不拉回 Waiting（与 determine_status 语义一致）
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Idle, APP_STATUS_STALE_MS * 10),
            SessionStatus::Idle
        );
    }

    #[test]
    fn waiting_passes_through_unaffected() {
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Waiting, APP_STATUS_STALE_MS * 10),
            SessionStatus::Waiting
        );
        assert_eq!(
            overlay_mtime_stale(SessionStatus::Waiting, 0),
            SessionStatus::Waiting
        );
    }

    // ---- 心跳消失竞态补偿（spec §8 测试策略：tempdir 驱动，注入 home/时钟/观测表）----

    mod compensation_tests {
        use super::*;

        const SID: &str = "7005f4cd-ef8b-4b7c-bcc5-b0f914c8a58c";
        const CWD: &str = "/Users/jarvis/proj";
        const ASSISTANT_TAIL: &str = r#"{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}"#;
        const RUNNING_TAIL: &str = r#"{"type":"function_call","name":"shell"}"#;

        fn write_jsonl(home: &Path, sid: &str, tail: &str) {
            let dir = home
                .join(".workbuddy/projects")
                .join(mangle_project_path(CWD));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{sid}.jsonl")), tail).unwrap();
        }

        fn write_heartbeat(home: &Path, pid: u32, last_heartbeat_ms: u64) {
            let hb = format!(
                r#"{{"pid":{pid},"sessionId":"{SID}","cwd":"{CWD}","lastHeartbeat":{last_heartbeat_ms}}}"#
            );
            let dir = home.join(".workbuddy/sessions");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{pid}.json")), hb).unwrap();
        }

        #[test]
        fn vanished_and_completed_session_is_compensated() {
            let home = tempfile::tempdir().unwrap();
            write_jsonl(home.path(), SID, ASSISTANT_TAIL); // 终态 = assistant 完成
                                                           // 心跳文件缺席（prewarm 回池/退出）+ 上一轮观测表记录过该 pid
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let out = compensate_vanished_heartbeats_in(home.path(), 10_000, &last_seen, &|_| None);
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].session_id, SID);
            assert_eq!(out[0].tool_id, "workbuddy");
            assert_eq!(out[0].turned_green_at_ms, 10_000); // 以补偿时刻为转绿时间
                                                           // 已消失条目从观测表移除，下轮不重复补偿
            assert!(last_seen.lock().unwrap().is_empty());
        }

        #[test]
        fn vanished_but_killed_mid_run_is_not_compensated() {
            let home = tempfile::tempdir().unwrap();
            write_jsonl(home.path(), SID, RUNNING_TAIL); // 终态 = 运行中被杀
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let out = compensate_vanished_heartbeats_in(home.path(), 10_000, &last_seen, &|_| None);
            assert!(out.is_empty());
            // 消失即移除观测表条目（即便不补），防止陈旧 pid 长期滞留
            assert!(last_seen.lock().unwrap().is_empty());
        }

        #[test]
        fn fresh_heartbeat_is_skipped() {
            let home = tempfile::tempdir().unwrap();
            write_jsonl(home.path(), SID, ASSISTANT_TAIL);
            write_heartbeat(home.path(), 11952, 9_999); // 心跳存在且新鲜（10000-9999 < 90s）
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let out = compensate_vanished_heartbeats_in(home.path(), 10_000, &last_seen, &|_| None);
            assert!(out.is_empty());
            // 未消失条目保留，供下轮补偿参考
            assert_eq!(
                last_seen.lock().unwrap().get(&11952).map(String::as_str),
                Some(SID)
            );
        }

        #[test]
        fn stale_heartbeat_counts_as_vanished() {
            let home = tempfile::tempdir().unwrap();
            write_jsonl(home.path(), SID, ASSISTANT_TAIL);
            write_heartbeat(home.path(), 11952, 0); // 心跳文件在但早已过期（now-0 >= 90s）
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let out =
                compensate_vanished_heartbeats_in(home.path(), 100_000, &last_seen, &|_| None);
            assert_eq!(out.len(), 1); // 过期 = 视为消失，终态完成 → 补
        }

        /// review M1 回归锁：用户已读删行后 prewarm 回池（心跳消失），
        /// 状态缓存记录「绿已被观测」（Idle/Finished）→ 补偿不得复活未读行
        #[test]
        fn read_dismissed_green_session_is_not_resurrected() {
            let home = tempfile::tempdir().unwrap();
            write_jsonl(home.path(), SID, ASSISTANT_TAIL);
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let status_of = |sid: &str| (sid == SID).then(|| "Idle".to_string());
            let out =
                compensate_vanished_heartbeats_in(home.path(), 10_000, &last_seen, &status_of);
            assert!(out.is_empty(), "已观测绿的会话不得经补偿复活未读行");
            // 消失条目照常移除，不滞留
            assert!(last_seen.lock().unwrap().is_empty());
        }

        #[test]
        fn vanished_without_jsonl_is_ignored() {
            // 观测表有记录但会话文件不存在（防御）→ 不产出、不 panic
            let home = tempfile::tempdir().unwrap();
            let last_seen = Mutex::new(HashMap::from([(11952u32, SID.to_string())]));
            let out = compensate_vanished_heartbeats_in(home.path(), 10_000, &last_seen, &|_| None);
            assert!(out.is_empty());
        }
    }
}
