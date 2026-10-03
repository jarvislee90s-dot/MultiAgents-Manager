//! 物化发现（C6，spec §4 第 5 步）：首句注入后轮询各工具的会话落盘，产出**候选
//! session id 列表**交调用方用确认戳（`confirm::stamp_of` + confirm_probe 缝）终判。
//! 只做「目录 stat/mtime 预过滤 + 最小身份读取」——扫描预算契约精神：不打开未过
//! 滤的文件（codex 首行读取是 mtime 预过滤后的单行读）。
//!
//! 入参一律**显式 base 路径**（tempdir 可测，零接触真实家目录）；真实家目录的
//! per-tool root 推导收在 [`create_store_root`]（生产装配闭包消费）。
//!
//! 各工具口径（均取 monitor 层现成解析 conventions，勿另立第二份）：
//! - claude：`<root>/projects/<项目 slug>/<sid>.jsonl`，slug = `path_codec::convert_path_to_dir_name`
//!   （claude_parser 同款，目录名大小写不敏感比较），sid = 文件名 stem；
//! - codex：`<root>/sessions/<Y>/<M>/<D>/rollout-*.jsonl` **今日目录**内 mtime>since，
//!   首行 JSON 的 `payload.cwd` 与目标目录归一匹配才取（rollout 在信任处置时即创建，
//!   只看 mtime 会假阳——cwd 匹配是第二道；confirm stamp 是最终判据），sid = `payload.id`；
//! - kimi：`session_index.jsonl` 条目 workDir 归一匹配 + wire.jsonl mtime>since
//!   （kimi_parser 现成口径。计划原文「`<root>/sessions/` 下 `wd_*` 目录」实况：
//!   `wd_*` 会话目录**实存**但并非索引正源——正源是 `session_index.jsonl`（workDir
//!   指向 wd_* 目录，sessionDir 越界信任边界复刻 resolve_session_dir）；本函数照
//!   索引正源定位，超窗/无索引条目自然漏配 → materialize_timeout 兜底，彼时会话
//!   实际可能已上板、仅 done 回执缺失——用户在看板可见，不丢数据）；
//! - opencode（P4 红线）：**拷 `opencode.db`+`-wal`+`-shm` 三件套到临时目录再查副本**
//!   （写入进 WAL，主库 mtime 不动——探测首轮 120s 全 miss 的教训）；session 表
//!   `directory` 归一匹配 + `time_updated` > since。
//!
//! 返回候选**最新在前**（调用方逐个 confirm，命中即收口）。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::monitor::cwd::{cwd_equivalent, normalize_cwd_for_match};

/// 每工具数据根（由用户家目录推导；与 monitor 各解析器的 home 拼接同款）：
/// - claude → `~/.claude`（discover 内再拼 `projects`）；
/// - codex → `~/.codex`（再拼 `sessions`）；
/// - kimi → kimi 数据根 home（`KIMI_CODE_HOME` 显式设置时以其为准，sessions 缺失
///   即整体不可用不回落——`kimi_parser::resolve_data_root` 同款规则）；
/// - opencode → `~/.local/share/opencode`（再拼 `opencode.db`）。
///
/// 白名单外工具 → None。
pub fn create_store_root(tool: &str, home: &Path) -> Option<PathBuf> {
    match tool {
        "claude" => Some(home.join(".claude")),
        "codex" => Some(home.join(".codex")),
        "kimi" => crate::monitor::kimi_parser::resolve_data_root(
            std::env::var("KIMI_CODE_HOME").ok().as_deref(),
            home,
        )
        .map(|r| r.home),
        "opencode" => Some(home.join(".local").join("share").join("opencode")),
        _ => None,
    }
}

/// 物化发现主入口：返回「> since 且落在 dir 下」的候选 session id（最新在前）。
/// `store_root` = [`create_store_root`] 的产物（该工具的数据根，非家目录本身）。
/// 未知工具 / 根不存在 → 空表（调用方继续轮询至预算耗尽，不报错）。
pub fn discover_new_session(
    tool: &str,
    store_root: &Path,
    since: SystemTime,
    dir: &str,
) -> Vec<String> {
    match tool {
        "claude" => discover_claude(store_root, since, dir),
        "codex" => discover_codex(store_root, since, dir),
        "kimi" => discover_kimi(store_root, since, dir),
        "opencode" => discover_opencode(store_root, since, dir),
        _ => Vec::new(),
    }
}

/// mtime 读取（不存在/读不到 → None）
fn mtime_of(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

// ---- claude：projects/<slug>/<sid>.jsonl ----

fn discover_claude(root: &Path, since: SystemTime, dir: &str) -> Vec<String> {
    let projects = root.join("projects");
    // claude_parser 同款：归一 cwd → 目录名 → 小写比较（Claude 保留用户敲入的盘符大小写，
    // 盘上可能同时存在 E--xxx 与 e--xxx 两种目录形态）
    let want = normalize_cwd_for_match(dir);
    if want.is_empty() {
        return Vec::new();
    }
    let slug = crate::monitor::path_codec::convert_path_to_dir_name(&want).to_lowercase();
    let Ok(entries) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    let mut out: Vec<(SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.to_lowercase() != slug {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&p) else {
            continue;
        };
        for f in files.flatten() {
            let fp = f.path();
            let is_session_jsonl = fp.extension().map(|e| e == "jsonl").unwrap_or(false)
                && !crate::monitor::jsonl::is_subagent_file(&fp);
            if !is_session_jsonl {
                continue;
            }
            let Some(mt) = mtime_of(&fp) else { continue };
            if mt <= since {
                continue;
            }
            if let Some(stem) = fp.file_stem().and_then(|s| s.to_str()) {
                out.push((mt, stem.to_string()));
            }
        }
    }
    // 最新在前（同刻并列按名字稳定序，HashMap 时代无此问题——read_dir 序不定必须排）
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, id)| id).collect()
}

// ---- codex：sessions/<Y>/<M>/<D>/rollout-*.jsonl 今日目录，首行 cwd 匹配 ----

fn discover_codex(root: &Path, since: SystemTime, dir: &str) -> Vec<String> {
    // 今日目录（codex 按本地日期分桶；%m/%d 两位补零与实机形态一致）
    let today = chrono::Local::now().format("%Y/%m/%d").to_string();
    let day_dir = root.join("sessions").join(&today);
    let Ok(entries) = std::fs::read_dir(&day_dir) else {
        return Vec::new();
    };
    let mut out: Vec<(SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        // rollout 命名判据（codex_parser::is_rollout_file 同款）
        let is_rollout = p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("rollout") && n.ends_with(".jsonl"))
            .unwrap_or(false);
        if !is_rollout {
            continue;
        }
        let Some(mt) = mtime_of(&p) else { continue };
        if mt <= since {
            continue;
        }
        // 候选已被 mtime 预过滤（个位数），首行单行读可接受（扫描预算契约口径）
        if let Some(id) = rollout_identity(&p, dir) {
            out.push((mt, id));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, id)| id).collect()
}

/// rollout 首行身份读取：`{"type":"session_meta","payload":{"id":…,"cwd":…}}`。
/// cwd 与目标目录归一匹配才返回 id（codex_parser::session_meta_identity 同款字段名）；
/// 首行读不到 / 无 id / cwd 不匹配 → None。
fn rollout_identity(path: &Path, dir: &str) -> Option<String> {
    let first = crate::monitor::jsonl::read_first_lines(path, 1);
    let line = first.first()?;
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let payload = v.get("payload")?;
    let id = payload.get("id").and_then(|x| x.as_str())?;
    let cwd = payload.get("cwd").and_then(|x| x.as_str()).unwrap_or("");
    if cwd_equivalent(cwd, dir) {
        Some(id.to_string())
    } else {
        None
    }
}

// ---- kimi：session_index.jsonl workDir 匹配 + wire.jsonl mtime ----

fn discover_kimi(root: &Path, since: SystemTime, dir: &str) -> Vec<String> {
    // store_root 即 kimi 数据根 home（create_store_root 的产物）；sessions 必同源
    let data = crate::monitor::kimi_parser::KimiDataRoot {
        home: root.to_path_buf(),
        sessions: root.join("sessions"),
    };
    let want = normalize_cwd_for_match(dir);
    let mut out: Vec<(SystemTime, String)> = Vec::new();
    for e in crate::monitor::kimi_parser::parse_session_index(&data) {
        if !cwd_equivalent(&e.work_dir, dir) || want.is_empty() {
            continue;
        }
        // 索引行 → wire.jsonl mtime（stat_index_entry 的信任边界口径：sessionDir
        // 须落在 sessions 根之下——resolve_session_dir + starts_with 复刻）
        let session_dir = crate::monitor::kimi_parser::resolve_session_dir(&data, &e.session_dir);
        if !session_dir.starts_with(&data.sessions) {
            continue;
        }
        let wire = session_dir.join("agents").join("main").join("wire.jsonl");
        let Some(mt) = mtime_of(&wire) else { continue };
        if mt <= since {
            continue;
        }
        out.push((mt, e.session_id));
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, id)| id).collect()
}

// ---- opencode：三件套拷贝 → 副本查询（P4 红线） ----

/// 副本临时目录前缀（std::env::temp_dir 下；进程退出后残留由系统临时目录清理兜底，
/// 正常路径末尾 best-effort 删除）
const OC_COPY_PREFIX: &str = "mam-oc-probe-copy-";

fn discover_opencode(root: &Path, since: SystemTime, dir: &str) -> Vec<String> {
    let db = root.join("opencode.db");
    if !db.exists() {
        return Vec::new();
    }
    // P4 红线：主库直接连会读不到 WAL 里未 checkpoint 的新写入（探测首轮 120s 全
    // miss 的教训）——拷 db/-wal/-shm 三件套到临时目录，对副本查询。tempfile 是
    // dev-dep，运行时目录用 std::env::temp_dir + 唯一名（C6 报告登记）。
    let unique = format!(
        "{OC_COPY_PREFIX}{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let copy_dir = std::env::temp_dir().join(unique);
    if std::fs::create_dir_all(&copy_dir).is_err() {
        return Vec::new();
    }
    let copy_db = copy_dir.join("opencode.db");
    for ext in ["", "-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{ext}", db.display()));
        if src.exists() {
            let _ = std::fs::copy(&src, PathBuf::from(format!("{}{ext}", copy_db.display())));
        }
    }
    let out = query_opencode_copy(&copy_db, since, dir);
    // best-effort 清理（失败不碍事——只读副本，系统临时目录最终回收）
    let _ = std::fs::remove_dir_all(&copy_dir);
    out
}

/// 副本查询：会话表 directory 归一匹配 + time_updated > since（毫秒 int）。
///
/// **双 schema 分派（D1 同口径）**：2.x 会话表改名 `session_v2`（`session` 冻结，
/// 新会话只写 `session_v2`——若仍查 `session`，2.x 下 create 物化永远 0 命中）；
/// 表不存在则回落 1.x 旧表名。探测判据与 `monitor::opencode_parser::Schema::detect` 一致。
fn query_opencode_copy(copy_db: &Path, since: SystemTime, dir: &str) -> Vec<String> {
    let Some(conn) = crate::monitor::sqlite::open_readonly_with_timeout(copy_db) else {
        return Vec::new();
    };
    let since_ms = since
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let table = if conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_v2'",
            [],
            |_| Ok(()),
        )
        .is_ok()
    {
        "session_v2"
    } else {
        "session"
    };
    let Ok(mut stmt) = conn.prepare(&format!(
        "SELECT id, directory, time_updated FROM {table} ORDER BY time_updated DESC"
    )) else {
        return Vec::new();
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    let mut out: Vec<(i64, String)> = rows
        .ok()
        .map(|it| {
            it.flatten()
                .filter(|(_, directory, updated)| {
                    *updated > since_ms && cwd_equivalent(directory, dir)
                })
                .map(|(id, _, updated)| (updated, id))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().map(|(_, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// since 采样与文件落盘之间留出文件系统时间戳分辨率余量（FAT/NTFS 100ns–2s；
    /// 本机 tempdir 分辨率足够，5ms 实测量级安全）
    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(30));
    }

    // ---- claude ----

    #[test]
    fn claude_finds_new_jsonl_by_slug_and_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".claude");
        let proj =
            root.join("projects")
                .join(crate::monitor::path_codec::convert_path_to_dir_name(
                    &normalize_cwd_for_match(r"E:\work\proj"),
                ));
        std::fs::create_dir_all(&proj).unwrap();
        let old = proj.join("old-session.jsonl");
        std::fs::write(&old, "x\n").unwrap();
        settle();
        let since = SystemTime::now();
        settle();
        let new = proj.join("abc123def456.jsonl");
        std::fs::write(&new, "x\n").unwrap();
        // 子 agent 文件（agent-*.jsonl，jsonl::is_subagent_file 同款命名）与别的项目
        // 目录不入选
        std::fs::write(proj.join("agent-abc123def456.jsonl"), "x\n").unwrap();
        let other =
            root.join("projects")
                .join(crate::monitor::path_codec::convert_path_to_dir_name(
                    &normalize_cwd_for_match(r"E:\other"),
                ));
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("zzz.jsonl"), "x\n").unwrap();

        let out = discover_claude(&root, since, r"E:\work\proj");
        assert_eq!(
            out,
            vec!["abc123def456".to_string()],
            "只收新落盘的主会话文件"
        );
        // 全量（since=UNIX 纪元）= 新旧都收，最新在前
        let all = discover_claude(&root, SystemTime::UNIX_EPOCH, r"E:\work\proj");
        assert_eq!(
            all,
            vec!["abc123def456".to_string(), "old-session".to_string()]
        );
    }

    #[test]
    fn claude_slug_match_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".claude");
        // 盘上目录小写盘符形态（claude_parser 实测两种形态并存）
        let proj =
            root.join("projects")
                .join(crate::monitor::path_codec::convert_path_to_dir_name(
                    &normalize_cwd_for_match(r"e:\work\proj"),
                ));
        std::fs::create_dir_all(&proj).unwrap();
        settle();
        let since = SystemTime::now();
        settle();
        std::fs::write(proj.join("sid-1.jsonl"), "x\n").unwrap();
        let out = discover_claude(&root, since, r"E:\WORK\PROJ");
        assert_eq!(out, vec!["sid-1".to_string()]);
    }

    // ---- codex ----

    /// rollout 夹具：今日目录 + 首行 session_meta
    fn codex_rollout(root: &Path, name: &str, id: &str, cwd: &str) -> PathBuf {
        let day = root
            .join("sessions")
            .join(chrono::Local::now().format("%Y/%m/%d").to_string());
        std::fs::create_dir_all(&day).unwrap();
        let p = day.join(name);
        let meta = serde_json::json!({
            "timestamp": "2026-10-02T00:00:00.000Z",
            "type": "session_meta",
            "payload": { "id": id, "cwd": cwd }
        });
        std::fs::write(&p, format!("{meta}\n")).unwrap();
        p
    }

    #[test]
    fn codex_requires_mtime_and_cwd_match() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".codex");
        let old = codex_rollout(
            &root,
            "rollout-2026-10-01-old.jsonl",
            "sid-old",
            r"E:\work\p",
        );
        settle();
        let since = SystemTime::now();
        settle();
        let hit = codex_rollout(&root, "rollout-2026-10-02-a.jsonl", "sid-new", r"E:\work\p");
        let _ = hit;
        // cwd 不匹配的新 rollout 不入选（第二道判据）
        let _miss = codex_rollout(
            &root,
            "rollout-2026-10-02-b.jsonl",
            "sid-other-dir",
            r"E:\elsewhere",
        );

        let out = discover_codex(&root, since, r"E:\work\p");
        assert_eq!(out, vec!["sid-new".to_string()], "mtime+cwd 双过才收");
        // since 早于全部：old 也收（cwd 匹配），最新在前
        let all = discover_codex(&root, SystemTime::UNIX_EPOCH, r"E:\work\p");
        assert_eq!(all.first().map(String::as_str), Some("sid-new"));
        assert!(
            all.contains(&"sid-old".to_string()),
            "旧 rollout cwd 匹配也收: {all:?}"
        );
        let _ = old;
    }

    // ---- kimi ----

    /// kimi 夹具：session_index.jsonl 一行 + sessions/<dir>/agents/main/wire.jsonl
    fn kimi_fixture(root: &Path, sid: &str, work_dir: &str) {
        let data = crate::monitor::kimi_parser::KimiDataRoot {
            home: root.to_path_buf(),
            sessions: root.join("sessions"),
        };
        let session_dir = data.sessions.join("wd_test").join(sid);
        std::fs::create_dir_all(session_dir.join("agents").join("main")).unwrap();
        std::fs::write(
            session_dir.join("agents").join("main").join("wire.jsonl"),
            "{}\n",
        )
        .unwrap();
        let entry = serde_json::json!({
            "sessionId": sid,
            "sessionDir": format!("wd_test/{sid}"),
            "workDir": work_dir
        });
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(data.home.join("session_index.jsonl"))
            .unwrap();
        writeln!(f, "{entry}").unwrap();
    }

    #[test]
    fn kimi_finds_by_index_workdir_and_wire_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".kimi-code");
        kimi_fixture(&root, "ks-old", r"E:\work\p");
        settle();
        let since = SystemTime::now();
        settle();
        kimi_fixture(&root, "ks-new", r"E:\work\p");
        kimi_fixture(&root, "ks-other", r"E:\elsewhere");

        let out = discover_kimi(&root, since, r"E:\work\p");
        assert_eq!(out, vec!["ks-new".to_string()]);
        let all = discover_kimi(&root, SystemTime::UNIX_EPOCH, r"E:\work\p");
        assert_eq!(all, vec!["ks-new".to_string(), "ks-old".to_string()]);
    }

    // ---- opencode ----

    /// opencode 夹具：主库（含 WAL 模拟不必——直接建主库+一行；拷贝逻辑吃同一套）。
    /// `v2=false` 建 1.x 旧 `session` 表；`v2=true` 建 2.x `session_v2` 表（D1 定案口径）
    fn opencode_fixture(root: &Path, sid: &str, directory: &str, updated_ms: i64) {
        opencode_fixture_schema(root, sid, directory, updated_ms, false);
    }

    fn opencode_fixture_schema(root: &Path, sid: &str, directory: &str, updated_ms: i64, v2: bool) {
        std::fs::create_dir_all(root).unwrap();
        let db = root.join("opencode.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        let table = if v2 { "session_v2" } else { "session" };
        conn.execute(
            &format!("CREATE TABLE IF NOT EXISTS {table} (id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, title TEXT, time_updated INTEGER)"),
            [],
        )
        .unwrap();
        conn.execute(
            &format!("INSERT INTO {table} (id, project_id, directory, title, time_updated) VALUES (?1,'p1',?2,'t',?3)"),
            rusqlite::params![sid, directory, updated_ms],
        )
        .unwrap();
    }

    #[test]
    fn opencode_copy_probe_finds_by_directory_and_time() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local/share/opencode");
        let base = chrono::Utc::now().timestamp_millis();
        opencode_fixture(&root, "oc-old", r"E:\work\p", base - 60_000);
        opencode_fixture(&root, "oc-new", r"E:\work\p", base);
        opencode_fixture(&root, "oc-dir-miss", r"E:\elsewhere", base);
        let since = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis((base - 30_000) as u64);

        let out = discover_opencode(&root, since, r"E:\work\p");
        assert_eq!(out, vec!["oc-new".to_string()], "时间窗 + directory 双过");
        // 全量：同目录两行都收，最新在前
        let all = discover_opencode(&root, SystemTime::UNIX_EPOCH, r"E:\work\p");
        assert_eq!(all, vec!["oc-new".to_string(), "oc-old".to_string()]);
        // 归一匹配：Windows 走大小写+分隔符变体；Linux 大小写敏感（normalize 的
        // 平台真源语义），用分隔符/尾斜杠变体——两平台都验「归一后同目录」
        let norm_query = if cfg!(windows) {
            r"e:\WORK\P/"
        } else {
            r"E:/work/p/"
        };
        let norm = discover_opencode(&root, SystemTime::UNIX_EPOCH, norm_query);
        assert_eq!(norm.len(), 2);
    }

    /// 2.x 库（`session_v2`）：仍能按 directory 命中——若查冻结的 `session` 表则 0 命中，
    /// create 物化在 2.x 下会永远超时（D2 修复点）
    #[test]
    fn opencode2_copy_probe_dispatches_to_session_v2() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local/share/opencode");
        let base = chrono::Utc::now().timestamp_millis();
        opencode_fixture_schema(&root, "oc2-new", r"E:\work\p", base, true);
        opencode_fixture_schema(&root, "oc2-miss", r"E:\elsewhere", base, true);
        let since = std::time::SystemTime::UNIX_EPOCH;

        let out = discover_opencode(&root, since, r"E:\work\p");
        assert_eq!(
            out,
            vec!["oc2-new".to_string()],
            "2.x 库须走 session_v2（旧表名已冻结 → 0 命中）"
        );
        // 归一匹配同样成立（平台变体选择同上：Linux 大小写敏感）
        let norm_query = if cfg!(windows) {
            r"e:\WORK\P/"
        } else {
            r"E:/work/p/"
        };
        let norm = discover_opencode(&root, since, norm_query);
        assert_eq!(norm.len(), 1);
    }

    /// 混合库（v1 冻结表 + v2 表并存）：走 v2（D1 分派定案，prefer session_v2）
    #[test]
    fn opencode_mixed_db_prefers_v2() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local/share/opencode");
        let base = chrono::Utc::now().timestamp_millis();
        // 先建 v1 冻结表并插一条（不该被采）
        opencode_fixture_schema(&root, "legacy", r"E:\work\p", base, false);
        // 再建 v2 表并插新会话
        opencode_fixture_schema(&root, "oc2", r"E:\work\p", base, true);
        let out = discover_opencode(&root, std::time::SystemTime::UNIX_EPOCH, r"E:\work\p");
        assert!(
            out.contains(&"oc2".to_string()),
            "v2 表在场时须走 v2（含新会话）"
        );
    }

    /// 1.x 库（仅旧表）：仍走旧表名，零回归
    #[test]
    fn opencode1_copy_probe_uses_legacy_table() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local/share/opencode");
        let base = chrono::Utc::now().timestamp_millis();
        opencode_fixture(&root, "oc1", r"E:\work\p", base);
        let out = discover_opencode(&root, std::time::SystemTime::UNIX_EPOCH, r"E:\work\p");
        assert_eq!(out, vec!["oc1".to_string()], "纯 1.x 库走旧表名");
    }

    // ---- root 推导与白名单 ----

    #[test]
    fn create_store_root_maps_whitelist() {
        let home = Path::new("/home/u");
        assert_eq!(
            create_store_root("claude", home).as_deref(),
            Some(Path::new("/home/u/.claude"))
        );
        assert_eq!(
            create_store_root("codex", home).as_deref(),
            Some(Path::new("/home/u/.codex"))
        );
        assert_eq!(
            create_store_root("opencode", home).as_deref(),
            Some(Path::new("/home/u/.local/share/opencode"))
        );
        // kimi：resolve_data_root 规则同源——sessions 目录**存在**才选定（无 env 注入
        // 时优先 ~/.kimi-code/sessions）；kimi 分支用 tempdir home 驱动（固定假家目录
        // 无 sessions 目录 → 按规则整体不可用返回 None，正是要锁的语义）。
        // env 守卫（C6 评审 M2）：KIMI_CODE_HOME 在场时 create_store_root 走 env
        // 解析——home 参数不参与，两条 home 驱动断言都不成立，并行假红面消除
        let tmp = tempfile::tempdir().unwrap();
        if std::env::var_os("KIMI_CODE_HOME").is_none() {
            assert_eq!(
                create_store_root("kimi", tmp.path()),
                None,
                "无 sessions 目录 → kimi 数据根不可用（resolve_data_root 同规）"
            );
            std::fs::create_dir_all(tmp.path().join(".kimi-code").join("sessions")).unwrap();
            assert_eq!(
                create_store_root("kimi", tmp.path()).as_deref(),
                Some(tmp.path().join(".kimi-code").as_path())
            );
        }
        // 白名单外 → None
        assert_eq!(create_store_root("workbuddy", home), None);
    }

    #[test]
    fn unknown_tool_yields_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            discover_new_session("workbuddy", tmp.path(), SystemTime::now(), "/tmp").is_empty()
        );
    }
}
