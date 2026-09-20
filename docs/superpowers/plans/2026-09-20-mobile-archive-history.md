# 移动端历史会话区（归档+重新激活闭环）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 死会话进入移动端独立历史页（登记制归档、懒加载、工具×项目×天数筛选），详情页「在桌面端打开」走既有 resume 链完成重新激活闭环；活会话详情页移除该按钮。

**Architecture:** SQLite 新表 `session_archive` 由活板扫描每轮旁路 upsert（写放大守卫）；远端新增 `GET/DELETE /sessions-archived`（懒加载，`RemoteState` 注入缝供测试）+ `session-open` 归档回退；移动端新增历史页与归档详情页两个组件，App 状态路由扩展。扫描管道、注入引擎、通知/unread 管道零改动。

**Tech Stack:** Tauri 2 + Rust（rusqlite/axum/chrono）+ React 19 + TypeScript + Vitest（jsdom）+ cargo test

**Spec:** `docs/superpowers/specs/2026-09-20-mobile-archive-history-design.md`（八项用户裁决见 spec §3，实现不得偏离）

## Global Constraints

- 分支：`feat/mobile-archive-history`（基于 fba50c9）。提交信息 conventional 前缀 + 中文描述（如 `feat(archive): …`）
- 语言：代码标识符英文，代码注释中文；移动端 UI 文案中文内联（移动端无 i18n 契约）
- 扫描预算契约（AGENTS.md）：**零新增文件系统扫描**——归档数据只来自扫描旁路 upsert；`get_all_sessions` 的会话产物在登记前后必须逐字节一致（登记挂在产物定序之后、返回之前，纯读消费）
- W5 审计词表不扩展：归档浏览/删除不写 `write_audit`；`session-open` 审计行为不变
- 时间戳口径：库内一律 `chrono::Utc::now().to_rfc3339()`；窗口过滤在端点内 `parse_from_rfc3339` 解析后比较（不裸串比较）
- RemoteState 新增字段需补齐**全部**构造点（锚点：grep `resume_spawner:` 共约 8 处——remote/mod.rs 生产装配 1 处 + remote/server.rs 测试构造若干处，同位补新字段两行）
- 测试命令：Rust 用 `cd src-tauri && cargo test <过滤名>`；前端用 `pnpm test <文件路径>`（vitest run）；任务收尾跑 `cd src-tauri && cargo clippy` 与 `pnpm lint`（零新告警）
- 移动端测试目录 `tests/mobile/`（jsdom）；fetch mock 用 `vi.stubGlobal` 自包含桩（参照 `tests/mobile/SessionDetail.test.tsx:62-140` 的分路模式）

---

### Task 1: 数据层——`session_archive` 表 + DAO（登记/查询/删除/反查）

**Files:**
- Modify: `src-tauri/src/database/schema.rs`（init 的 execute_batch 末尾追加建表）
- Create: `src-tauri/src/database/dao/archive.rs`
- Modify: `src-tauri/src/database/dao/mod.rs`（`pub mod archive;`）
- Modify: `src-tauri/src/database/mod.rs`（re-export）

**Interfaces:**
- Consumes: `crate::session::Session`（`session/model.rs:70`，字段 id/agent_type/project_name/project_path/title/status）；`crate::database::connection::DB`
- Produces（后续任务依赖的精确签名）:
  - `pub struct SessionArchiveRow { pub session_id: String, pub agent_type: String, pub project_path: String, pub project_name: String, pub title: Option<String>, pub last_status: String, pub first_seen: String, pub last_seen: String, pub updated_at: String }`
  - `pub fn register_sessions(sessions: &[Session])`（全局 DB 包装）/ `pub fn register_sessions_conn(conn: &rusqlite::Connection, sessions: &[Session]) -> usize`（返回实际写入行数）
  - `pub fn query_archive_all() -> Vec<SessionArchiveRow>` / `pub fn query_archive_all_conn(conn: &rusqlite::Connection) -> Vec<SessionArchiveRow>`（全量，last_seen DESC）
  - `pub fn delete_archive(session_id: Option<&str>) -> usize`（None=清空）/ `pub fn delete_archive_conn(conn: &rusqlite::Connection, session_id: Option<&str>) -> usize`
  - `pub fn agent_type_from_tool_id(tool: &str) -> Option<crate::session::AgentType>`（八工具反查）

- [ ] **Step 1: schema 建表（schema.rs init 的 execute_batch 内、最后一张表之后追加）**

```sql
CREATE TABLE IF NOT EXISTS session_archive (
    session_id   TEXT PRIMARY KEY,
    agent_type   TEXT NOT NULL,
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL,
    title        TEXT,
    last_status  TEXT NOT NULL,
    first_seen   TEXT NOT NULL,
    last_seen    TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
```

- [ ] **Step 2: 写失败测试（dao/archive.rs 文件先建好结构 + `#[cfg(test)] mod tests`；实现函数先写空壳 `todo!()` 或直接先只写测试，模块能编译）**

dao/archive.rs 测试（用内存库，模式对齐 dao/session.rs 的 `_conn` 注入风格）：

```rust
use rusqlite::Connection;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AgentType, Session, SessionStatus};

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::database::schema::init(&conn);
        conn
    }

    fn sess(id: &str, cwd: &str, title: Option<&str>) -> Session {
        Session {
            id: id.into(),
            agent_type: AgentType::Codex,
            project_name: "proj-x".into(),
            project_path: cwd.into(),
            title: title.map(Into::into),
            git_branch: None,
            github_url: None,
            status: SessionStatus::Waiting,
            last_message: None,
            last_message_role: None,
            last_activity_at: "2026-09-20T00:00:00Z".into(),
            pid: 7,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: crate::session::ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        }
    }

    #[test]
    fn register_then_query_roundtrip() {
        let conn = mem_conn();
        let n = register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", Some("标题"))]);
        assert_eq!(n, 1);
        let rows = query_archive_all_conn(&conn);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.session_id, "s1");
        assert_eq!(r.agent_type, "codex");
        assert_eq!(r.project_path, "/tmp/a");
        assert_eq!(r.project_name, "proj-x");
        assert_eq!(r.title.as_deref(), Some("标题"));
        assert_eq!(r.last_status, "waiting");
        assert_eq!(r.first_seen, r.last_seen); // 首次登记两者同刻
    }

    #[test]
    fn register_skips_empty_cwd() {
        let conn = mem_conn();
        let n = register_sessions_conn(&conn, &[&sess("s1", "", None)]);
        assert_eq!(n, 0);
        assert!(query_archive_all_conn(&conn).is_empty());
    }

    #[test]
    fn guard_no_rewrite_when_unchanged() {
        let conn = mem_conn();
        register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", Some("t"))]);
        let before = conn.total_changes();
        let n = register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", Some("t"))]);
        assert_eq!(n, 0); // 字段无变化且 last_seen 新鲜（<60s）→ 零写事务
        assert_eq!(conn.total_changes(), before);
    }

    #[test]
    fn field_change_forces_rewrite_and_first_seen_survives() {
        let conn = mem_conn();
        register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", Some("t"))]);
        let first = query_archive_all_conn(&conn)[0].first_seen.clone();
        let n = register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", Some("新标题"))]);
        assert_eq!(n, 1);
        let rows = query_archive_all_conn(&conn);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title.as_deref(), Some("新标题"));
        assert_eq!(rows[0].first_seen, first); // first_seen 不随 upsert 重置
    }

    #[test]
    fn delete_single_and_all() {
        let conn = mem_conn();
        register_sessions_conn(&conn, &[&sess("s1", "/tmp/a", None), &sess("s2", "/tmp/b", None)]);
        assert_eq!(delete_archive_conn(&conn, Some("s1")), 1);
        assert_eq!(query_archive_all_conn(&conn).len(), 1);
        assert_eq!(delete_archive_conn(&conn, None), 1);
        assert!(query_archive_all_conn(&conn).is_empty());
    }

    #[test]
    fn agent_type_reverse_mapping() {
        assert!(matches!(agent_type_from_tool_id("claude"), Some(AgentType::Claude)));
        assert!(matches!(agent_type_from_tool_id("dsh"), Some(AgentType::Dsh)));
        assert!(agent_type_from_tool_id("unknown-tool").is_none());
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cd src-tauri && cargo test dao::archive`
Expected: 编译失败（函数未定义）——先写空实现使编译通过后再跑，断言 FAIL

- [ ] **Step 4: 实现 dao/archive.rs（完整代码）**

```rust
//! 历史会话归档 DAO（spec：2026-09-20-mobile-archive-history §5）。
//! 登记制：只收 MAM 运行期间上过板的会话（活板扫描旁路 upsert），不做任何
//! 死会话文件扫描（扫描预算契约 L1/L3 零触碰）。读取侧懒加载（历史页请求时），
//! 写放大守卫：字段无变化且 last_seen 距上次写入 <60s → 零写事务（fsync 风暴
//! 防护，口径同 dao::session::update_session_status 的 LAST_SEEN_REFRESH 模式）。
use rusqlite::{params, Connection};

use crate::database::connection::DB;
use crate::session::{AgentType, Session};

/// last_seen 刷新间隔（秒）：静止活会话至多每 60s 落一次盘
const ARCHIVE_REFRESH_SECS: i64 = 60;

/// 归档行（查询投影；first_seen/last_seen/updated_at 均 RFC3339 UTC）
#[derive(Debug, Clone)]
pub struct SessionArchiveRow {
    pub session_id: String,
    pub agent_type: String,
    pub project_path: String,
    pub project_name: String,
    pub title: Option<String>,
    pub last_status: String,
    pub first_seen: String,
    pub last_seen: String,
    pub updated_at: String,
}

/// tool_id 反查 AgentType（登记存 tool_id 字符串，激活回退构造 Session 时用）
pub fn agent_type_from_tool_id(tool: &str) -> Option<AgentType> {
    match tool {
        "claude" => Some(AgentType::Claude),
        "codex" => Some(AgentType::Codex),
        "opencode" => Some(AgentType::OpenCode),
        "openclaw" => Some(AgentType::OpenClaw),
        "kimi" => Some(AgentType::Kimi),
        "workbuddy" => Some(AgentType::WorkBuddy),
        "zcode" => Some(AgentType::ZCode),
        "dsh" => Some(AgentType::Dsh),
        _ => None,
    }
}

fn last_seen_stale(last_seen: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(last_seen) {
        Ok(t) => chrono::Utc::now()
            .signed_duration_since(t.with_timezone(&chrono::Utc))
            .num_seconds()
            > ARCHIVE_REFRESH_SECS,
        Err(_) => true, // 畸形值按需刷新处理（刷新写库即自愈）
    }
}

/// 批量登记（全局 DB；挂点：adapter::get_all_sessions_inner 尾部）。
/// 返回实际写入行数（测试与调试观测用）。**纯读消费入参，不改扫描产物**。
pub fn register_sessions(sessions: &[Session]) {
    let conn = DB.lock().unwrap();
    register_sessions_conn(&conn, sessions);
}

/// 同上（连接注入版）。无 project_path 会话跳过（无法 resume，归档无价值——spec §5）
pub fn register_sessions_conn(conn: &Connection, sessions: &[Session]) -> usize {
    let mut written = 0usize;
    for s in sessions {
        if s.project_path.trim().is_empty() {
            continue;
        }
        let existing: Option<(Option<String>, String, String, String)> = conn
            .query_row(
                "SELECT title, last_status, project_path, last_seen FROM session_archive WHERE session_id = ?",
                [&s.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .ok();
        let status = format!("{:?}", s.status).to_lowercase();
        let (changed, stale) = match &existing {
            None => (true, true),
            Some((title, last_status, path, seen)) => {
                (*title != s.title || *last_status != status || *path != s.project_path
                    , last_seen_stale(seen))
            }
        };
        if !changed && !stale {
            continue; // 守卫：零写事务
        }
        let now = chrono::Utc::now().to_rfc3339();
        let first_seen = existing
            .as_ref()
            .map(|(_, _, _, _)| now.clone()) // 占位，下一行修正：first_seen 保留原值
            ;
        let first_seen = match existing.is_some() {
            _ => now.clone(), // 见下——INSERT OR REPLACE 无法保留原列，改为显式两分支
        };
        let _ = first_seen; // （实现时直接按下方 SQL 分支，删除本占位链）
        if existing.is_some() {
            conn.execute(
                "UPDATE session_archive SET title=?1, last_status=?2, project_path=?3,
                 project_name=?4, agent_type=?5, last_seen=?6, updated_at=?6 WHERE session_id=?7",
                params![
                    s.title, status, s.project_path, s.project_name,
                    s.agent_type.tool_id(), now, s.id
                ],
            )
            .map(|_| written += 1)
            .ok();
        } else {
            conn.execute(
                "INSERT INTO session_archive
                 (session_id, agent_type, project_path, project_name, title, last_status,
                  first_seen, last_seen, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?7)",
                params![
                    s.id, s.agent_type.tool_id(), s.project_path, s.project_name,
                    s.title, status, now
                ],
            )
            .map(|_| written += 1)
            .ok();
        }
    }
    written
}

/// 全量归档行（last_seen DESC）。数据量为 MAM 运行期会话量级（小），端点在内存
/// 做窗口过滤与活板排除（spec §6.1：缝保持「无参取数」最简形态）
pub fn query_archive_all() -> Vec<SessionArchiveRow> {
    let conn = DB.lock().unwrap();
    query_archive_all_conn(&conn)
}

/// 同上（连接注入版）
pub fn query_archive_all_conn(conn: &Connection) -> Vec<SessionArchiveRow> {
    let mut stmt = match conn.prepare(
        "SELECT session_id, agent_type, project_path, project_name, title, last_status,
                first_seen, last_seen, updated_at
         FROM session_archive ORDER BY last_seen DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(), // 表未建（老库未迁移）防御：空集
    };
    stmt.query_map([], |row| {
        Ok(SessionArchiveRow {
            session_id: row.get(0)?,
            agent_type: row.get(1)?,
            project_path: row.get(2)?,
            project_name: row.get(3)?,
            title: row.get(4)?,
            last_status: row.get(5)?,
            first_seen: row.get(6)?,
            last_seen: row.get(7)?,
            updated_at: row.get(8)?,
        })
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// 删除归档（None=清空全部）。返回删除行数
pub fn delete_archive(session_id: Option<&str>) -> usize {
    let conn = DB.lock().unwrap();
    delete_archive_conn(&conn, session_id)
}

/// 同上（连接注入版）
pub fn delete_archive_conn(conn: &Connection, session_id: Option<&str>) -> usize {
    match session_id {
        Some(id) => conn
            .execute("DELETE FROM session_archive WHERE session_id = ?", [id])
            .unwrap_or(0),
        None => conn
            .execute("DELETE FROM session_archive", [])
            .unwrap_or(0),
    }
}
```

**实现注意（把上面对照清干净的最终形态）**：`register_sessions_conn` 内 UPDATE/INSERT 两分支如上；写实现时删除 `first_seen` 占位链那几行伪码（那是推导痕迹）——UPDATE 分支不触碰 `first_seen` 列即天然保留原值，INSERT 分支三时间列同刻。

- [ ] **Step 5: mod/re-export 接线**

`dao/mod.rs` 加 `pub mod archive;`；`database/mod.rs` 在 `pub use dao::session::{...}` 之后追加：

```rust
pub use dao::archive::{
    agent_type_from_tool_id, delete_archive, query_archive_all, register_sessions,
    SessionArchiveRow,
};
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cd src-tauri && cargo test dao::archive`
Expected: 6 个测试全 PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/database/
git commit -m "feat(archive): session_archive 表 + 登记/查询/删除 DAO——写放大守卫(60s/字段变化)与空 cwd 跳过，登记制归档数据层"
```

---

### Task 2: 登记挂点——扫描旁路消费

**Files:**
- Modify: `src-tauri/src/adapter/mod.rs`（`get_all_sessions_inner` 尾部，`update_session_status` 循环与 `cleanup_stale_sessions` 之间）

**Interfaces:**
- Consumes: Task 1 的 `crate::database::register_sessions(&[Session])`
- Produces: 无新接口——行为：每轮扫描后登记表增量更新；扫描产物不变

- [ ] **Step 1: 加挂点（在 `cleanup_stale_sessions` 调用之前插入）**

定位锚点（adapter/mod.rs `get_all_sessions_inner` 尾部既有代码）：

```rust
    // 更新会话状态缓存（通知去重用）
    for session in &all_sessions {
        let _ = crate::database::update_session_status(
            &session.id,
            &format!("{:?}", session.agent_type),
            &format!("{:?}", session.status),
        );
    }
```

在其后插入：

```rust
    // 历史会话登记（spec 2026-09-20-mobile-archive-history §5）：扫描的旁路消费者，
    // 挂在产物定序/过滤/排序全部完成之后——all_sessions 已定型，本调用纯读消费，
    // 扫描产物（SessionsResponse）在登记前后逐字节一致（预算契约零触碰）。
    // 无 cwd 会话与写放大守卫在 DAO 内处理
    crate::database::register_sessions(&all_sessions);
```

- [ ] **Step 2: 编译 + 既有测试回归**

Run: `cd src-tauri && cargo check && cargo test adapter`
Expected: 编译通过；adapter 既有测试全绿（登记不改变任何扫描行为——纯追加调用，无返回值消费）

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/adapter/mod.rs
git commit -m "feat(archive): 活板扫描尾部挂登记 upsert——旁路消费不改扫描产物，多客户端轮询共享登记"
```

---

### Task 3: 后端读路径——GET/DELETE `/sessions-archived`

**Files:**
- Modify: `src-tauri/src/remote/server.rs`（RemoteState 加 2 缝字段 + 全部构造点补齐 + 路由注册）
- Modify: `src-tauri/src/remote/mod.rs`（生产装配）
- Modify: `src-tauri/src/remote/api.rs`（两个 handler + DTO）
- Test: `src-tauri/src/remote/server.rs` 既有 `#[cfg(test)]` 区追加（`test_state` 基建复用）

**Interfaces:**
- Consumes: Task 1 `query_archive_all`/`delete_archive`/`SessionArchiveRow`；既有 `st.session_source`、PIN gate 路由器
- Produces:
  - `GET /m/api/v1/sessions-archived?days=1|3|7` → `200 {"archived":[ArchivedSessionDto],"projects":[String]}`（days 非法夹取 1；活板同 id 查期排除；last_seen 降序）
  - `DELETE /m/api/v1/sessions-archived?session_id=…` 或 `?all=1` → `200 {"ok":true,"deleted":n}`；两参皆无 → 400
  - RemoteState 新字段（后续任务与测试构造依赖）:
    - `pub archive_source: Box<dyn Fn() -> Vec<crate::database::SessionArchiveRow> + Send + Sync>`
    - `pub archive_delete: std::sync::Arc<dyn Fn(Option<&str>) -> usize + Send + Sync>`

- [ ] **Step 1: RemoteState 缝字段 + 构造点**

server.rs 结构体（`resume_spawner` 字段后）追加：

```rust
    /// 归档读源注入缝（spec §6.1）：生产 = database::query_archive_all（全量行，
    /// 窗口/排除在端点内做）；测试注入固定行集（零真实 ~/.mam 接触）
    pub archive_source: Box<dyn Fn() -> Vec<crate::database::SessionArchiveRow> + Send + Sync>,
    /// 归档删除缝：生产 = database::delete_archive；测试记录型假体返回计数
    pub archive_delete: std::sync::Arc<dyn Fn(Option<&str>) -> usize + Send + Sync>,
```

生产装配 remote/mod.rs（`resume_spawner` 行后）：

```rust
        archive_source: Box::new(crate::database::query_archive_all),
        archive_delete: std::sync::Arc::new(crate::database::delete_archive),
```

全部测试构造点（锚点 grep `resume_spawner:`，server.rs 内每处）同位补：

```rust
            archive_source: Box::new(|_| Vec::new()),   // 注意：无参闭包 → Box::new(Vec::new) 形态见下
            archive_delete: std::sync::Arc::new(|_| 0),
```

（闭包无参：写 `Box::new(|| Vec::new())` 与 `Arc::new(|_: Option<&str>| 0usize)`——按编译器推断微调，语义=空归档/零删除。）

- [ ] **Step 2: 写失败测试（server.rs 既有测试区追加）**

```rust
    // ==== 历史会话区（spec 2026-09-20-mobile-archive-history §6.1）====
    mod archive_api_tests {
        use super::*;
        use crate::database::SessionArchiveRow;

        fn arch_row(id: &str, tool: &str, proj: &str, seen_secs_ago: i64) -> SessionArchiveRow {
            let seen = (chrono::Utc::now() - chrono::Duration::seconds(seen_secs_ago)).to_rfc3339();
            SessionArchiveRow {
                session_id: id.into(),
                agent_type: tool.into(),
                project_path: format!("/tmp/{proj}"),
                project_name: proj.into(),
                title: Some("标题".into()),
                last_status: "idle".into(),
                first_seen: seen.clone(),
                last_seen: seen,
                updated_at: String::new(),
            }
        }

        fn archive_state(rows: Vec<SessionArchiveRow>) -> axum::Router {
            let mut st = test_state();
            st.archive_source = Box::new(move || rows.clone());
            st.archive_delete = std::sync::Arc::new(|_| 0);
            crate::remote::server::router(st)
        }

        #[tokio::test]
        async fn days_window_and_order() {
            let app = archive_state(vec![
                arch_row("fresh", "codex", "a", 3600),        // 1h 前 → 1 天窗内
                arch_row("old2d", "kimi", "b", 2 * 86400),    // 2 天前 → 仅 3/7 天窗
            ]);
            let r = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/m/api/v1/sessions-archived?days=1")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let arr = v["archived"].as_array().unwrap();
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["sessionId"], "fresh");
            let projects = v["projects"].as_array().unwrap();
            assert_eq!(projects, &["a".into()]); // 窗口内项目聚合
        }

        #[tokio::test]
        async fn days_invalid_clamped_to_one() {
            let app = archive_state(vec![
                arch_row("old2d", "kimi", "b", 2 * 86400),
            ]);
            let r = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/m/api/v1/sessions-archived?days=999")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["archived"].as_array().unwrap().len(), 0); // 夹到 1 天 → 排除
        }

        #[tokio::test]
        async fn live_session_excluded_from_archive() {
            // test_state 的 session_source 需带一条活会话（复用该文件既有夹具形态；
            // 若 test_state 默认快照为空，就地改 st.session_source 返回含 "live-1" 的快照）
            let rows = vec![
                arch_row("live-1", "codex", "a", 60),
                arch_row("dead-1", "kimi", "b", 120),
            ];
            let mut st = test_state();
            st.archive_source = Box::new(move || rows.clone());
            st.archive_delete = std::sync::Arc::new(|_| 0);
            // 活板快照注入（形态对齐本文件既有 session_source 假体写法）
            st.session_source = Box::new(|| crate::remote::server::SessionsSnapshot {
                sessions: vec![crate::remote::server::inj_sess("live-1")],
            });
            let app = crate::remote::server::router(st);
            let r = app
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/m/api/v1/sessions-archived")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let arr = v["archived"].as_array().unwrap();
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["sessionId"], "dead-1");
        }

        #[tokio::test]
        async fn delete_requires_param() {
            let app = archive_state(vec![]);
            let r = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("DELETE")
                        .uri("/m/api/v1/sessions-archived")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), 400);
        }
    }
```

**注**：`inj_sess`/`SessionsSnapshot` 为 server.rs 既有测试夹具（resume 端点测试同款）；若实际名字不同，以本文件既有写法为准对齐——测试意图不变（活板含 `live-1`）。

- [ ] **Step 3: 跑测试确认失败**

Run: `cd src-tauri && cargo test archive_api_tests`
Expected: 编译失败（字段/handler 未定义）

- [ ] **Step 4: 实现 api.rs 两个 handler + DTO**

api.rs（session_open handler 之前，注入端点区）：

```rust
// ==== 历史会话区端点（spec 2026-09-20-mobile-archive-history §6.1）====

/// GET /sessions-archived 响应条目（camelCase）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivedSessionDto {
    pub session_id: String,
    pub agent_type: String,
    pub project_path: String,
    pub project_name: String,
    pub title: Option<String>,
    pub last_status: String,
    pub last_seen_at: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsArchivedResp {
    pub archived: Vec<ArchivedSessionDto>,
    pub projects: Vec<String>,
}

/// GET /m/api/v1/sessions-archived?days=1|3|7：懒加载归档列表。
/// days 非法夹取 1；活板同 id 查期排除（不删行）；last_seen 降序；
/// projects = 结果集内 distinct 项目名按该项目最大 last_seen 降序（下拉选项源）。
/// 时间比较一律 parse_from_rfc3339 解析后比（不裸串比较）；畸形时间行防御性排除。
pub async fn sessions_archived(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let days = match params.get("days").and_then(|s| s.parse::<i64>().ok()) {
        Some(3) => 3,
        Some(7) => 7,
        _ => 1,
    };
    let st2 = st.clone();
    let resp = tokio::task::spawn_blocking(move || {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
        let live_ids: std::collections::HashSet<String> = (st2.session_source)()
            .sessions
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let mut items: Vec<(chrono::DateTime<chrono::Utc>, ArchivedSessionDto)> =
            (st2.archive_source)()
                .into_iter()
                .filter_map(|row| {
                    if live_ids.contains(&row.session_id) {
                        return None;
                    }
                    let seen = chrono::DateTime::parse_from_rfc3339(&row.last_seen)
                        .ok()?
                        .with_timezone(&chrono::Utc);
                    if seen < cutoff {
                        return None;
                    }
                    Some((
                        seen,
                        ArchivedSessionDto {
                            session_id: row.session_id,
                            agent_type: row.agent_type,
                            project_path: row.project_path,
                            project_name: row.project_name,
                            title: row.title,
                            last_status: row.last_status,
                            last_seen_at: row.last_seen,
                        },
                    ))
                })
                .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0));
        // 项目聚合：distinct 项目名，按该项目条目最大 last_seen 降序
        let mut best: Vec<(String, chrono::DateTime<chrono::Utc>)> = Vec::new();
        for (seen, dto) in &items {
            match best.iter_mut().find(|(n, _)| *n == dto.project_name) {
                Some(e) => {
                    if *e.1 < *seen {
                        e.1 = *seen;
                    }
                }
                None => best.push((dto.project_name.clone(), *seen)),
            }
        }
        best.sort_by(|a, b| b.1.cmp(&a.1));
        SessionsArchivedResp {
            archived: items.into_iter().map(|(_, dto)| dto).collect(),
            projects: best.into_iter().map(|(n, _)| n).collect(),
        }
    })
    .await;
    match resp {
        Ok(r) => (
            StatusCode::OK,
            [(axum::http::header::CACHE_CONTROL, "no-store")],
            axum::Json(r),
        )
            .into_response(),
        Err(e) => {
            log::error!("sessions-archived 任务异常: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(axum::http::header::CACHE_CONTROL, "no-store")],
                Json(serde_json::json!({ "error": "internal" })),
            )
                .into_response()
        }
    }
}

/// DELETE /m/api/v1/sessions-archived?session_id=… 或 ?all=1：手动管理
/// （spec 裁决 8：只增不删+手动）。归档管理不写 write_audit（W5 词表为注入动作域）。
pub async fn sessions_archived_delete(
    State(st): State<Arc<RemoteState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let all = params.get("all").map(|v| v == "1").unwrap_or(false);
    let sid = params
        .get("session_id")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if !all && sid.is_none() {
        return bad_request();
    }
    let target: Option<String> = if all { None } else { sid };
    let deleted = (st.archive_delete)(target.as_deref());
    (
        StatusCode::OK,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({ "ok": true, "deleted": deleted })),
    )
        .into_response()
}
```

- [ ] **Step 5: 路由注册（server.rs，session-open 路由后）**

```rust
        // 历史会话区（spec 2026-09-20-mobile-archive-history §6.1）：懒加载列表 + 手动管理
        .route(
            "/sessions-archived",
            get(api::sessions_archived).delete(api::sessions_archived_delete),
        )
```

- [ ] **Step 6: 跑测试确认通过 + gate 复核**

Run: `cd src-tauri && cargo test archive_api_tests && cargo test gate_403`
Expected: 全 PASS（gate 403 矩阵对新路由结构性生效——注册在 nest 内即被 PIN gate 包裹）

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/remote/
git commit -m "feat(archive): GET/DELETE /sessions-archived 端点——days 夹取/活板查期排除/项目聚合，archive_source+archive_delete 注入缝零污染测试"
```

---

### Task 4: 后端激活路径——session-open 归档回退

**Files:**
- Modify: `src-tauri/src/remote/api.rs`（session_open 的会话查找闭包，约 :1617）
- Test: `src-tauri/src/remote/server.rs` 测试区追加

**Interfaces:**
- Consumes: Task 1 `agent_type_from_tool_id`；Task 3 `st.archive_source`；既有 `st.resume_spawner`（假体记录，零真开窗）
- Produces: `POST /session-open` 对「活快照未命中但归档表命中」的 session_id 走既有 resume 链；两者皆未命中仍 404 `no_session`（既有契约不破）

- [ ] **Step 1: 写失败测试（server.rs resume 端点测试区旁追加）**

```rust
    mod session_open_archive_fallback_tests {
        use super::*;
        use crate::database::SessionArchiveRow;

        #[tokio::test]
        async fn dead_session_opens_from_archive() {
            let row = SessionArchiveRow {
                session_id: "dead-9".into(),
                agent_type: "codex".into(),
                project_path: "/tmp/proj-dead".into(),
                project_name: "proj-dead".into(),
                title: None,
                last_status: "idle".into(),
                first_seen: String::new(),
                last_seen: chrono::Utc::now().to_rfc3339(),
                updated_at: String::new(),
            };
            let mut st = test_state();
            // 活快照为空（复用 test_state 默认或就地注入空快照——形态对齐既有写法）
            st.archive_source = Box::new(move || vec![row.clone()]);
            let fired = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let f2 = fired.clone();
            st.resume_spawner = std::sync::Arc::new(move |spec| {
                f2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // 断言 cwd 落在归档行 project_path（构造的 SpawnSpec 携带脚本全文）
                let crate::inject::resume::SpawnSpec::MacosApplescript { script } = spec
                else {
                    return Err("非 macOS spec".into());
                };
                if script.contains("/tmp/proj-dead") && script.contains("codex resume dead-9") {
                    Ok(())
                } else {
                    Err(format!("payload 不含归档 cwd/resume 命令: {script}"))
                }
            });
            let app = crate::remote::server::router(st);
            let r = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/m/api/v1/session-open")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(
                            r#"{"sessionId":"dead-9"}"#,
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), 200);
            let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["status"], "opening");
            assert_eq!(fired.load(std::sync::atomic::Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn neither_live_nor_archive_still_404() {
            let mut st = test_state();
            st.archive_source = Box::new(|| Vec::new());
            let app = crate::remote::server::router(st);
            let r = app
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/m/api/v1/session-open")
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(r#"{"sessionId":"ghost"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), 404);
        }
    }
```

**注**：Windows 跑该测试走 Windows 分支 spec——`SpawnSpec` 匹配与脚本断言需按 `cfg` 分诊（既有 resume 端点测试 :4614 有同款「Windows 上跑走 Windows 分支」注释先例，形态对齐之；macOS 断言脚本包含 cwd 与 resume 命令，Windows 断言 `SpawnSpec::Windows` 的 `cwd` 字段）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd src-tauri && cargo test session_open_archive_fallback`
Expected: `dead_session_opens_from_archive` FAIL（现路径 404 no_session）

- [ ] **Step 3: 实现回退（api.rs session_open 的 spawn_blocking 闭包内）**

将现有：

```rust
            let Some(session) = (probe_st.session_source)()
                .sessions
                .into_iter()
                .find(|s| s.id == probe_sid)
            else {
                return Err("no_session".to_string());
            };
```

改为：

```rust
            let session = match (probe_st.session_source)()
                .sessions
                .into_iter()
                .find(|s| s.id == probe_sid)
            {
                Some(s) => s,
                // 归档回退（spec §6.2）：死会话经登记表复活。id 仍取自本机数据
                // （登记行），不回显远端输入——resume.rs 安全口径不变；构造的
                // Session 仅 resume 链消费的三字段有效（id/agent_type/project_path），
                // 其余字段为中性缺省（不上面板）
                None => {
                    let Some(row) = (probe_st.archive_source)()
                        .into_iter()
                        .find(|r| r.session_id == probe_sid)
                    else {
                        return Err("no_session".to_string());
                    };
                    let Some(agent_type) =
                        crate::database::agent_type_from_tool_id(&row.agent_type)
                    else {
                        // 未知 tool_id：resume_command 必返 None，提前以既有哨兵回退
                        return Err("no_resume_command".to_string());
                    };
                    crate::session::Session {
                        id: row.session_id,
                        agent_type,
                        project_name: row.project_name,
                        project_path: row.project_path,
                        title: row.title,
                        git_branch: None,
                        github_url: None,
                        status: crate::session::SessionStatus::Waiting,
                        last_message: None,
                        last_message_role: None,
                        last_activity_at: row.last_seen.clone(),
                        pid: 0,
                        cpu_usage: 0.0,
                        active_subagent_count: 0,
                        form: crate::session::ProcessForm::Cli,
                        jump_supported: false,
                        unread: false,
                    }
                }
            };
```

- [ ] **Step 4: 跑测试确认通过 + resume 既有回归**

Run: `cd src-tauri && cargo test session_open && cargo test resume`
Expected: 全 PASS（既有 resume 端点测试不受影响——活快照命中路径未动）

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/remote/
git commit -m "feat(archive): session-open 归档回退——活快照未命中转登记表构造 Session 走既有 resume 链，双未命中仍 404"
```

---

### Task 5: 移动端纯层——api 扩展 + resume 门迁移 + 归档过滤纯函数

**Files:**
- Create: `src/mobile/resume-gate.ts`（从 SessionDetail.tsx 迁出）
- Create: `src/mobile/archive-logic.ts`
- Modify: `src/mobile/api.ts`（新增归档拉取/删除）
- Modify: `src/mobile/SessionDetail.tsx`（删本地定义，Task 6 一并删按钮 JSX；本任务只做「定义迁出」——SessionDetail 暂改 import 自 resume-gate）
- Test: `tests/mobile/archive-logic.test.ts`（新）、`tests/mobile/api.test.ts`（追加）

**Interfaces:**
- Produces:
  - `resume-gate.ts`: `export const RESUME_SUPPORTED_TOOLS: ReadonlySet<string>`（claude/codex/kimi/opencode）；`export function resumeUnavailableReason(session: { projectPath?: string | null; agentType: string }): string | null`
  - `archive-logic.ts`: `export type ArchiveToolFilter = string`（"all" 或工具 id）；`export function filterArchivedByTool<T extends { agentType: string }>(rows: T[], filter: ArchiveToolFilter): T[]`；`export function filterArchivedByProject<T extends { projectName: string }>(rows: T[], project: string): T[]`（"all" 直通）；`export function relativeEndLabel(lastSeenAt: string, now?: number): string`（刚刚/N 分钟前/N 小时前/昨天/N 天前；畸形输入返回 "—"）
  - `api.ts`: `export interface ArchivedSession { sessionId: string; agentType: string; projectPath: string; projectName: string; title: string | null; lastStatus: string; lastSeenAt: string }`；`export interface ArchivedPayload { archived: ArchivedSession[]; projects: string[] }`；`export async function fetchArchivedSessions(days: 1 | 3 | 7): Promise<ArchivedPayload | null>`（403 → null 回配对页，与 fetchSessions 同口径）；`export async function deleteArchivedSession(sessionId?: string): Promise<number>`（缺省=清空全部）

- [ ] **Step 1: 写失败测试 `tests/mobile/archive-logic.test.ts`**

```typescript
import { describe, expect, it } from "vitest";
import {
  filterArchivedByProject,
  filterArchivedByTool,
  relativeEndLabel,
} from "@/mobile/archive-logic";

const rows = [
  { agentType: "codex", projectName: "proj-a" },
  { agentType: "kimi", projectName: "proj-b" },
  { agentType: "codex", projectName: "proj-b" },
];

describe("archive-logic：工具×项目双维过滤", () => {
  it("all 直通；单工具过滤", () => {
    expect(filterArchivedByTool(rows, "all")).toHaveLength(3);
    expect(filterArchivedByTool(rows, "codex")).toHaveLength(2);
  });
  it("项目过滤 all 直通；单项目过滤；两维可组合（调用侧串联）", () => {
    expect(filterArchivedByProject(rows, "all")).toHaveLength(3);
    expect(filterArchivedByProject(rows, "proj-b")).toHaveLength(2);
    expect(filterArchivedByProject(filterArchivedByTool(rows, "codex"), "proj-b")).toHaveLength(1);
  });
});

describe("relativeEndLabel：相对结束时间", () => {
  const now = Date.parse("2026-09-20T12:00:00Z");
  const at = (minusMin: number) => new Date(now - minusMin * 60_000).toISOString();
  it("分档：刚刚 / 分钟 / 小时 / 昨天 / 天", () => {
    expect(relativeEndLabel(at(2), now)).toBe("刚刚");
    expect(relativeEndLabel(at(30), now)).toBe("30 分钟前");
    expect(relativeEndLabel(at(90), now)).toBe("1 小时前");
    expect(relativeEndLabel(at(26 * 60), now)).toBe("昨天");
    expect(relativeEndLabel(at(3 * 24 * 60), now)).toBe("3 天前");
  });
  it("畸形输入返回 —（防御）", () => {
    expect(relativeEndLabel("not-a-date", now)).toBe("—");
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test tests/mobile/archive-logic.test.ts`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现 `src/mobile/archive-logic.ts`**

```typescript
// 历史会话过滤纯函数（spec §7.2）：工具 × 项目双维 + 相对时间换算。
// 与 board-logic.ts 同风格：零时钟依赖（now 注入）、不改入参。
export type ArchiveToolFilter = string; // "all" 或工具 id

export function filterArchivedByTool<T extends { agentType: string }>(
  rows: T[],
  filter: ArchiveToolFilter,
): T[] {
  if (filter === "all") return rows;
  return rows.filter((r) => r.agentType === filter);
}

export function filterArchivedByProject<T extends { projectName: string }>(
  rows: T[],
  project: string,
): T[] {
  if (project === "all") return rows;
  return rows.filter((r) => r.projectName === project);
}

/** 相对结束时间标签：刚刚(<5min)/N 分钟前(<60)/N 小时前(<24h)/昨天(24–48h)/N 天前。
 *  畸形输入返回 "—"（防御——坏数据不冒泡成 NaN） */
export function relativeEndLabel(lastSeenAt: string, now: number = Date.now()): string {
  const t = Date.parse(lastSeenAt);
  if (Number.isNaN(t)) return "—";
  const min = Math.max(0, Math.floor((now - t) / 60_000));
  if (min < 5) return "刚刚";
  if (min < 60) return `${min} 分钟前`;
  const hours = Math.floor(min / 60);
  if (hours < 24) return `${hours} 小时前`;
  if (hours < 48) return "昨天";
  return `${Math.floor(hours / 24)} 天前`;
}
```

- [ ] **Step 4: 迁出 resume 门 `src/mobile/resume-gate.ts`**

```typescript
// 「在桌面端打开」可用性门（R5 Task 11 原生于 SessionDetail，2026-09-20 归档区
// 复用迁出为独立模块）。SSOT = src-tauri/src/inject/resume.rs 的 RESUME_TABLE——
// 后端查证新工具回填后**两处必须同步**（本镜像仅驱动按钮禁用态）。
export const RESUME_SUPPORTED_TOOLS: ReadonlySet<string> = new Set([
  "claude",
  "codex",
  "kimi",
  "opencode",
]);

/** 禁用原因（中文内联，移动端无 i18n 契约）：null = 可用 */
export function resumeUnavailableReason(session: {
  projectPath?: string | null;
  agentType: string;
}): string | null {
  if (!session.projectPath || !session.projectPath.trim()) {
    return "该会话没有项目目录信息，无法在电脑上打开";
  }
  if (!RESUME_SUPPORTED_TOOLS.has(session.agentType)) {
    return "该工具 resume 命令待查证，暂不支持一键打开";
  }
  return null;
}
```

SessionDetail.tsx：删除本地 `RESUME_SUPPORTED_TOOLS` 与 `resumeUnavailableReason` 定义（:74–:90 一带，注释块整体），文件内消费点暂改为 `import { resumeUnavailableReason } from "./resume-gate";`（Task 6 删按钮后此 import 一并移除）。

- [ ] **Step 5: api.ts 扩展（sessionOpen 函数之后追加）**

```typescript
// ==== 历史会话区（spec 2026-09-20-mobile-archive-history §6.1）====
export interface ArchivedSession {
  sessionId: string;
  agentType: string;
  projectPath: string;
  projectName: string;
  title: string | null;
  lastStatus: string;
  lastSeenAt: string;
}

export interface ArchivedPayload {
  archived: ArchivedSession[];
  projects: string[];
}

/** 懒加载归档列表（进入历史页/切换天数时调用；403 → null 回配对页） */
export async function fetchArchivedSessions(days: 1 | 3 | 7): Promise<ArchivedPayload | null> {
  const r = await fetch(`/m/api/v1/sessions-archived?days=${days}`);
  if (r.status === 403) return null;
  if (!r.ok) {
    throw new ApiError(r.status, `sessions-archived ${r.status}`);
  }
  return r.json() as Promise<ArchivedPayload>;
}

/** 归档手动管理（spec 裁决 8）：带 id = 单条移除；缺省 = 清空全部 */
export async function deleteArchivedSession(sessionId?: string): Promise<number> {
  const qs = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : "?all=1";
  const r = await fetch(`/m/api/v1/sessions-archived${qs}`, { method: "DELETE" });
  if (!r.ok) throw new ApiError(r.status, `sessions-archived DELETE ${r.status}`);
  const data = (await r.json()) as { deleted: number };
  return data.deleted;
}
```

api.test.ts 追加两个用例（形态对齐该文件既有 fetch 桩写法）：`fetchArchivedSessions(7)` 断言 URL 带 `days=7` 且载荷解析；403 返回 null。

- [ ] **Step 6: 跑测试确认通过 + 既有回归**

Run: `pnpm test tests/mobile/archive-logic.test.ts tests/mobile/api.test.ts tests/mobile/SessionDetail.test.tsx`
Expected: 全 PASS（SessionDetail 按钮仍在——本任务只迁定义）

- [ ] **Step 7: Commit**

```bash
git add src/mobile/resume-gate.ts src/mobile/archive-logic.ts src/mobile/api.ts src/mobile/SessionDetail.tsx tests/mobile/
git commit -m "feat(archive): 移动端纯层——fetchArchived/deleteArchived API、resume 门迁出 resume-gate、工具×项目过滤与相对时间纯函数"
```

---

### Task 6: 移动端 UI——历史页 + 归档详情页 + 活详情按钮移除

**Files:**
- Create: `src/mobile/ArchiveBoard.tsx`
- Create: `src/mobile/ArchiveDetail.tsx`
- Modify: `src/mobile/App.tsx`（路由扩展）
- Modify: `src/mobile/Board.tsx`（header 加历史入口 + 新 prop）
- Modify: `src/mobile/SessionDetail.tsx`（删「在电脑上打开」按钮整块 + opening 态 + handleSessionOpen + sessionOpen import + resume-gate import）
- Test: `tests/mobile/ArchiveBoard.test.tsx`（新）、`tests/mobile/ArchiveDetail.test.tsx`（新，承接 SessionDetail 迁出的回执分诊用例）、`tests/mobile/SessionDetail.test.tsx`（删 :1220 起的 describe 块）、`tests/mobile/AppRouting.test.tsx`（追加历史路由用例）

**Interfaces:**
- Consumes: Task 5 全部导出；`api.ts` 既有 `sessionOpen`（:529）与 `fetchSessionMessages(agentType, sessionId, limit)`（:101）；Board 既有 props 模式
- Produces:
  - `ArchiveBoard`：`{ onBack: () => void; onOpenCard: (s: ArchivedSession) => void }`
  - `ArchiveDetail`：`{ session: ArchivedSession; onBack: () => void; onActivated: () => void }`（onActivated = opening 回执后回看板）

- [ ] **Step 1: 写失败测试 `tests/mobile/ArchiveBoard.test.tsx`**

```typescript
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ArchiveBoard from "@/mobile/ArchiveBoard";
import type { ArchivedPayload } from "@/mobile/api";

const payload: ArchivedPayload = {
  archived: [
    {
      sessionId: "a1", agentType: "codex", projectPath: "/tmp/p1", projectName: "proj-1",
      title: "修复登录", lastStatus: "idle", lastSeenAt: new Date(Date.now() - 3600_000).toISOString(),
    },
  ],
  projects: ["proj-1"],
};

function installFetch(ok = true) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/sessions-archived") && !url.includes("DELETE")) {
        if (!ok) return new Response("boom", { status: 500 });
        return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
      }
      return new Response("{}", { status: 404 });
    }),
  );
}

describe("ArchiveBoard：历史页", () => {
  beforeEach(() => installFetch());
  afterEach(() => { vi.unstubAllGlobals(); cleanup(); });

  it("默认 1 天拉取并渲染卡片；卡片上无任何打开按钮", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (i: RequestInfo | URL) => {
      calls.push(String(i));
      return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
    }));
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    expect(await screen.findByText("proj-1")).toBeTruthy();
    expect(calls[0]).toContain("days=1");
    expect(screen.queryByTestId("session-open")).toBeNull(); // 裁决 6：按钮不浮在卡片上
  });

  it("切 7 天 → 重新拉取（URL 带 days=7）", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (i: RequestInfo | URL) => {
      calls.push(String(i));
      return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
    }));
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    await screen.findByText("proj-1");
    fireEvent.click(screen.getByTestId("archive-days-7"));
    await waitFor(() => expect(calls.some((c) => c.includes("days=7"))).toBe(true));
  });

  it("失败态显示重试按钮；点击重发", async () => {
    installFetch(false);
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    expect(await screen.findByTestId("archive-retry")).toBeTruthy();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `pnpm test tests/mobile/ArchiveBoard.test.tsx`
Expected: FAIL（组件不存在）

- [ ] **Step 3: 实现 `src/mobile/ArchiveBoard.tsx`**

```tsx
import { useCallback, useEffect, useState } from "react";
import {
  deleteArchivedSession,
  fetchArchivedSessions,
  type ArchivedPayload,
  type ArchivedSession,
} from "./api";
import { filterArchivedByProject, filterArchivedByTool, relativeEndLabel } from "./archive-logic";
import { AGENT_TYPES } from "./board-logic"; // 注：若 board-logic 未导出该常量，用下方内联 chips 集（实现时以 board-logic 实际导出为准）

type Days = 1 | 3 | 7;

/** 历史会话页（spec §7.2）：懒加载（进页 days=1，切天数重拉，页内不轮询——
 *  死数据静态）；双维筛选（工具 chips × 项目下拉）独立于活板选择；卡片无按钮
 *  （裁决 6——打开动作只在详情页）。 */
export default function ArchiveBoard({
  onBack,
  onOpenCard,
}: {
  onBack: () => void;
  onOpenCard: (s: ArchivedSession) => void;
}) {
  const [days, setDays] = useState<Days>(1);
  const [data, setData] = useState<ArchivedPayload | null>(null);
  const [error, setError] = useState(false);
  const [tool, setTool] = useState<string>("all");
  const [project, setProject] = useState<string>("all");

  const load = useCallback(async (d: Days) => {
    setError(false);
    try {
      const p = await fetchArchivedSessions(d);
      if (p === null) return; // 403：App 层配对态处理，此处静默
      setData(p);
    } catch {
      setError(true);
    }
  }, []);

  useEffect(() => { void load(days); }, [days, load]);

  const rows = data
    ? filterArchivedByProject(filterArchivedByTool(data.archived, tool), project)
    : [];
  // chips = 「全部」+ 当前窗口出现过的工具（动态——与活板 chips 的受管∩有卡不同，
  // 归档以结果集为准）
  const tools = ["all", ...new Set((data?.archived ?? []).map((s) => s.agentType))];

  return (
    <div className="mx-auto max-w-3xl px-4">
      <header className="mb-3 flex items-center justify-between pt-4">
        <div className="flex items-center gap-2">
          <button type="button" onClick={onBack} className="text-sm text-slate-500" aria-label="返回看板">‹ 返回</button>
          <h1 className="text-lg font-semibold">历史会话</h1>
        </div>
        <button
          type="button"
          data-testid="archive-clear"
          className="text-xs text-slate-400"
          onClick={() => {
            if (!window.confirm("清空全部归档记录？")) return;
            void deleteArchivedSession().then(() => void load(days));
          }}
        >
          清空归档
        </button>
      </header>

      {/* 第一行：工具 chips（复用 board-logic 的品牌色约定可选，首版中性样式） */}
      <div className="mb-2 flex gap-2 overflow-x-auto">
        {tools.map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTool(t)}
            className={`rounded-full px-3 py-1 text-xs ${t === tool ? "bg-blue-600 text-white" : "bg-slate-200 text-slate-700"}`}
          >
            {t === "all" ? "全部" : t}
          </button>
        ))}
      </div>

      {/* 第二行：项目下拉 + 天数分段 */}
      <div className="mb-3 flex gap-2">
        <select
          value={project}
          onChange={(e) => setProject(e.target.value)}
          className="min-w-0 flex-1 rounded-lg border border-slate-200 px-2 py-1.5 text-sm"
          aria-label="按项目筛选"
        >
          <option value="all">项目：全部</option>
          {(data?.projects ?? []).map((p) => (
            <option key={p} value={p}>{p}</option>
          ))}
        </select>
        <span className="flex gap-1">
          {([1, 3, 7] as Days[]).map((d) => (
            <button
              key={d}
              type="button"
              data-testid={`archive-days-${d}`}
              onClick={() => setDays(d)}
              className={`rounded-lg px-2.5 py-1.5 text-xs ${d === days ? "bg-blue-600 text-white" : "bg-slate-200 text-slate-700"}`}
            >
              {d}天
            </button>
          ))}
        </span>
      </div>

      {error && (
        <div className="py-8 text-center text-sm text-slate-500">
          加载失败
          <button type="button" data-testid="archive-retry" className="ml-2 underline" onClick={() => void load(days)}>
            重试
          </button>
        </div>
      )}
      {!error && data && rows.length === 0 && (
        <p className="py-8 text-center text-sm text-slate-400">
          {days === 7 ? "暂无归档记录" : `最近 ${days} 天没有非活跃会话，可试 3 天 / 7 天`}
        </p>
      )}
      <div className="flex flex-col gap-2 pb-8">
        {rows.map((s) => (
          <button
            key={s.sessionId}
            type="button"
            onClick={() => onOpenCard(s)}
            className="rounded-xl border border-slate-200 px-3 py-2.5 text-left enabled:hover:bg-slate-50 dark:border-slate-800"
          >
            <div className="flex items-baseline justify-between">
              <span className="text-sm font-medium">{s.projectName}</span>
              <span className="text-xs text-slate-400">{relativeEndLabel(s.lastSeenAt)}结束</span>
            </div>
            <div className="mt-0.5 text-xs text-slate-500">
              {s.agentType} · {s.title ?? "（无标题）"}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
```

**实现注意**：`AGENT_TYPES` 若 board-logic 未导出则删除该 import（chips 由结果集动态生成，本就不需要静态枚举）。

- [ ] **Step 4: 写失败测试 `tests/mobile/ArchiveDetail.test.tsx`（承接 SessionDetail :1220 迁出的回执分诊 + 门 + 乐观回调）**

```typescript
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ArchiveDetail from "@/mobile/ArchiveDetail";
import type { ArchivedSession } from "@/mobile/api";

const card: ArchivedSession = {
  sessionId: "dead-1", agentType: "codex", projectPath: "/tmp/p1", projectName: "proj-1",
  title: "标题", lastStatus: "idle", lastSeenAt: new Date().toISOString(),
};

function installFetch(routes: Record<string, unknown>) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/session-open")) {
        const body = (routes["sessionOpen"]) as Record<string, string> | undefined;
        const status = routes["sessionOpenStatus"] as number | undefined;
        return new Response(JSON.stringify(body ?? { status: "opening" }), {
          status: status ?? 200,
          headers: { "content-type": "application/json" },
        });
      }
      if (url.includes("/session-messages")) {
        return new Response(
          JSON.stringify({ messages: [{ seq: 1, role: "user", content: "旧消息", kind: "text", ts: 1 }] }),
          { headers: { "content-type": "application/json" } },
        );
      }
      if (url.includes("/sessions-archived")) {
        return new Response(JSON.stringify({ deleted: 1 }), { headers: { "content-type": "application/json" } });
      }
      return new Response("{}", { status: 404 });
    }),
  );
}

describe("ArchiveDetail：归档详情与激活", () => {
  beforeEach(() => installFetch({}));
  afterEach(() => { vi.unstubAllGlobals(); cleanup(); });

  it("渲染历史消息内容 + 在桌面端打开按钮存在", async () => {
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    expect(await screen.findByText("旧消息")).toBeTruthy();
    expect(screen.getByTestId("session-open").textContent).toBe("在桌面端打开");
  });

  it("opening 回执 → onActivated 回调（乐观回看板，裁决 6 闭环）", async () => {
    const activated = vi.fn();
    installFetch({ sessionOpen: { status: "opening" } });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={activated} />);
    fireEvent.click(screen.getByTestId("session-open"));
    await screen.findByText("正在电脑上打开终端…");
    expect(activated).toHaveBeenCalled();
  });

  it("failed 回执 → 错误文案上屏、不回调（可重试）", async () => {
    const activated = vi.fn();
    installFetch({ sessionOpen: { status: "failed", error: "终端启动失败（模拟）" } });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={activated} />);
    fireEvent.click(screen.getByTestId("session-open"));
    expect(await screen.findByTestId("session-open-error")).toBeTruthy();
    expect(activated).not.toHaveBeenCalled();
  });

  it("不支持 resume 的工具 → 按钮禁用 + 原因（门迁移回归锁）", () => {
    render(
      <ArchiveDetail
        session={{ ...card, agentType: "workbuddy" }}
        onBack={() => {}}
        onActivated={() => {}}
      />,
    );
    const btn = screen.getByTestId("session-open") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(screen.getByText("该工具 resume 命令待查证，暂不支持一键打开")).toBeTruthy();
  });

  it("从归档移除 → DELETE 后返回", async () => {
    const back = vi.fn();
    render(<ArchiveDetail session={card} onBack={back} onActivated={() => {}} />);
    fireEvent.click(screen.getByTestId("archive-remove"));
    fireEvent.click(screen.getByTestId("archive-remove-confirm"));
    await new Promise((r) => setTimeout(r, 0));
    expect(back).toHaveBeenCalled();
  });
});
```

- [ ] **Step 5: 实现 `src/mobile/ArchiveDetail.tsx`**

```tsx
import { useEffect, useState } from "react";
import {
  deleteArchivedSession,
  fetchSessionMessages,
  sessionOpen,
  type ArchivedSession,
  type SessionMessage,
} from "./api";
import { relativeEndLabel } from "./archive-logic";
import { resumeUnavailableReason } from "./resume-gate";

/** 归档详情页（spec §7.3）：只读消息（/session-messages 按 id 直读文件，零后端改动）
 *  + 唯一动作「在桌面端打开」（复用 sessionOpen）+ 次要动作「从归档移除」。 */
export default function ArchiveDetail({
  session,
  onBack,
  onActivated,
}: {
  session: ArchivedSession;
  onBack: () => void;
  onActivated: () => void;
}) {
  const [messages, setMessages] = useState<SessionMessage[] | null>(null);
  const [contentError, setContentError] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const reason = resumeUnavailableReason(session);

  useEffect(() => {
    let alive = true;
    fetchSessionMessages(session.agentType, session.sessionId, 200)
      .then((p) => { if (alive) setMessages(p.messages); })
      .catch(() => { if (alive) setContentError(true); });
    return () => { alive = false; };
  }, [session.agentType, session.sessionId]);

  const handleOpen = async () => {
    setOpening(true);
    setOpenError(null);
    try {
      const r = await sessionOpen(session.sessionId);
      if (r.status === "opening") {
        onActivated(); // 乐观回看板：活板 3s 内出卡，归档条目下次拉取自然消失
        return;
      }
      setOpenError(`打开失败：${r.error ?? "未知错误"}`);
    } catch (e) {
      setOpenError(`打开失败：${String(e)}`);
    } finally {
      setOpening(false);
    }
  };

  return (
    <div className="mx-auto flex max-w-3xl flex-col px-4">
      <header className="flex items-center gap-2 pt-4">
        <button type="button" onClick={onBack} className="text-sm text-slate-500" aria-label="返回历史页">‹ 返回</button>
        <h1 className="text-lg font-semibold">{session.projectName}</h1>
      </header>
      <p className="mt-1 text-xs text-slate-500">
        {session.agentType} · {session.projectPath} · {relativeEndLabel(session.lastSeenAt)}结束
      </p>

      <div className="shrink-0 px-1 pt-3">
        <button
          type="button"
          data-testid="session-open"
          disabled={reason !== null || opening}
          title={reason ?? undefined}
          onClick={handleOpen}
          className="w-full rounded-lg bg-green-700 px-3 py-2 text-sm text-white enabled:hover:bg-green-800 disabled:opacity-50"
        >
          {opening ? "正在电脑上打开终端…" : "在桌面端打开"}
        </button>
        {reason && <p className="mt-1 text-center text-xs text-slate-400">{reason}</p>}
        {openError && (
          <p data-testid="session-open-error" className="mt-1 text-center text-xs text-red-600">
            {openError}
          </p>
        )}
      </div>

      <div className="mt-3 min-h-0 flex-1 flex-col gap-2 pb-8">
        {contentError && <p className="py-6 text-center text-sm text-slate-400">内容暂不可读（会话文件可能已被工具清理）</p>}
        {messages === null && !contentError && <p className="py-6 text-center text-sm text-slate-400">加载中…</p>}
        {messages?.length === 0 && <p className="py-6 text-center text-sm text-slate-400">（无历史消息）</p>}
        {messages?.map((m) => (
          <div key={m.seq} className="rounded-lg bg-slate-100 px-3 py-2 text-sm dark:bg-slate-900">
            <span className="mr-2 text-xs text-slate-400">{m.role}</span>
            {m.content}
          </div>
        ))}
      </div>

      <div className="pb-6">
        {confirmRemove ? (
          <span className="flex gap-2">
            <button type="button" data-testid="archive-remove-confirm" className="flex-1 rounded-lg border border-red-300 px-3 py-1.5 text-xs text-red-600"
              onClick={() => { void deleteArchivedSession(session.sessionId).then(onBack); }}>
              确认移除
            </button>
            <button type="button" className="flex-1 rounded-lg border border-slate-200 px-3 py-1.5 text-xs" onClick={() => setConfirmRemove(false)}>
              取消
            </button>
          </span>
        ) : (
          <button type="button" data-testid="archive-remove" className="w-full rounded-lg px-3 py-1.5 text-xs text-slate-400" onClick={() => setConfirmRemove(true)}>
            从归档移除
          </button>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 6: App.tsx 路由扩展 + Board 入口 + SessionDetail 按钮删除**

App.tsx（`selected` 状态旁追加两个状态；渲染区追加两块；Board 隐藏条件扩展）：

```tsx
import ArchiveBoard from "./ArchiveBoard";
import ArchiveDetail from "./ArchiveDetail";
import type { ArchivedSession } from "./api";
// …
  const [historyOpen, setHistoryOpen] = useState(false);
  const [archiveSelected, setArchiveSelected] = useState<ArchivedSession | null>(null);
```

渲染区（Board hidden 条件 `!selected` 改 `!selected && !historyOpen`；Board 增传 `onOpenHistory={() => setHistoryOpen(true)}`）：

```tsx
      {paired !== false && (
        <div className={paired === true && !selected && !historyOpen ? "contents" : "hidden"}>
          <Board onPaired={onPaired} onUnpaired={onUnpaired} onOpenSession={setSelected} onOpenHistory={() => setHistoryOpen(true)} />
        </div>
      )}
      {paired === true && historyOpen && !archiveSelected && (
        <ArchiveBoard
          onBack={() => setHistoryOpen(false)}
          onOpenCard={setArchiveSelected}
        />
      )}
      {paired === true && historyOpen && archiveSelected && (
        <ArchiveDetail
          session={archiveSelected}
          onBack={() => setArchiveSelected(null)}
          onActivated={() => { setArchiveSelected(null); setHistoryOpen(false); }}
        />
      )}
```

Board.tsx：props 加 `onOpenHistory: () => void`；标题行（:341 `<header className="mb-3 flex items-baseline justify-between">` 内、会话计数 span 之前）加入口：

```tsx
          <button
            type="button"
            onClick={onOpenHistory}
            className="rounded-lg border border-slate-200 px-2 py-1 text-xs text-slate-600 enabled:hover:bg-slate-100 dark:border-slate-800 dark:text-slate-300"
            aria-label="历史会话"
          >
            🕘 历史
          </button>
```

SessionDetail.tsx 删除（整块）：
- JSX：`{/* R5 一键 resume（在电脑上打开，Task 11）… */}` 注释起的整个 `<div className="shrink-0 px-3 pt-2">` 块（:1215–:1235，含按钮 + resumeReason 段）
- 逻辑：`opening` state、`handleSessionOpen`（:622「R5 一键 resume」注释区）、`sessionOpen` import、`resumeUnavailableReason`/`resume-gate` import
- 保留：其余全部（消息渲染/文件面板/分屏/ApproveCard 不动）

tests/mobile/SessionDetail.test.tsx：删除 `describe("SessionDetail：一键 resume 回执分诊（评审 C1）"…)` 整块（:1220–:1265 一带）及 `routes.sessionOpen*` mock 分路中仅被该块消费的断言（mock 分路本身保留无害可留）。

- [ ] **Step 7: AppRouting 追加历史路由用例（复用该文件 installSse/installFetch 基建；installFetch 分路补 `/sessions-archived`）**

```typescript
// ==== 历史会话路由（spec §7.1）：看板 ↔ 历史页 ↔ 归档详情 ====
it("历史入口进入历史页（归档卡可见）；返回回到看板", async () => {
  installSse(okSessions([sessionFixture()]));
  // installFetch 内追加分路：
  //   url.includes("/sessions-archived") → Response(JSON.stringify({
  //     archived: [{ sessionId: "dead-1", agentType: "claude", projectPath: "/tmp/d",
  //       projectName: "hist-proj", title: null, lastStatus: "idle",
  //       lastSeenAt: new Date().toISOString() }],
  //     projects: ["hist-proj"] }))
  render(<App />);
  await screen.findByText("demo-proj"); // 看板就绪
  fireEvent.click(screen.getByRole("button", { name: "历史会话" }));
  expect(await screen.findByText("hist-proj")).toBeTruthy();
  expect(screen.queryByText("demo-proj")).toBeNull(); // Board hidden（数据保持）
  fireEvent.click(screen.getByRole("button", { name: "返回看板" }));
  expect(await screen.findByText("demo-proj")).toBeTruthy();
});
```

- [ ] **Step 8: 跑全量前端测试**

Run: `pnpm test`
Expected: 全绿（含新增 3 个测试文件 + AppRouting 扩展；SessionDetail 删块后无残留引用失败）

- [ ] **Step 9: lint + build**

Run: `pnpm lint && pnpm build`
Expected: 零新告警、构建通过

- [ ] **Step 10: Commit**

```bash
git add src/mobile/ tests/mobile/
git commit -m "feat(archive): 移动端历史页+归档详情页——懒加载/双维筛选/天数分段，激活闭环乐观回板；活会话详情页移除在电脑上打开按钮（裁决 2/6）"
```

---

### Task 7: 实机回传清单 + 文档收口

**Files:**
- Modify: `docs/release-notes/m6r-m9r-acceptance-checklist.md`（追加「E 段 · 历史会话区」）

**Interfaces:**
- Consumes: 本计划 Task 1–6 全部产出
- Produces: Mac 实机验证条目（供下轮验收执行）

- [ ] **Step 1: 清单追加（文件末尾）**

```markdown
---

## E 段 · 历史会话区（归档+重新激活闭环，spec 2026-09-20-mobile-archive-history）

> 基准：feat/mobile-archive-history。前置：MAM dev 运行 ≥2 分钟（登记表已积累至少一条已死会话——先开一个测试会话再关掉终端）。

| # | 场景 | 步骤 | 预期观察 |
|---|---|---|---|
| E-1 | 登记与懒加载 | 关掉一个测试会话终端 → 移动端进历史页 | 默认 1 天窗内出现该会话卡片（工具/项目/相对时间）；活会话不出现在历史页 |
| E-2 | 双维筛选 | chips 选工具 × 下拉选项目 | 组合过滤正确；项目下拉选项=窗口内项目按最近活跃降序 |
| E-3 | 天数分段 | 切 3 天 / 7 天 | 重拉且 URL days 参数正确（服务端日志/审计无对应写——读端点不写审计） |
| E-4 | 归档详情内容 | 点卡片 | 历史消息只读可见（按 id 直读文件）；「在桌面端打开」按钮在（支持工具）/禁用+原因（不支持工具） |
| E-5 | 激活闭环（核心） | 支持工具点「在桌面端打开」 | 回执 opening；Terminal 开窗 + cd 归档行 project_path + resume 原 id（进程可查）；审计 open ok；3s 内活板出卡；历史页该条消失（下次拉取排除） |
| E-6 | 活会话无按钮回归 | 活会话详情页 | 「在电脑上打开」按钮不存在（裁决 2） |
| E-7 | 手动管理 | 详情页从归档移除 / 历史页清空归档 | 单条消失 / 列表清空；再次拉取不回弹 |
| E-8 | 双未命中 404 契约 | 对不存在的 id POST session-open | 404 no_session（回归锁） |
```

- [ ] **Step 2: 全量回归（Rust + 前端 + clippy/lint）**

Run: `cd src-tauri && cargo test && cargo clippy` ；`pnpm test && pnpm lint && pnpm build`
Expected: 全绿、零新告警

- [ ] **Step 3: Commit**

```bash
git add docs/release-notes/m6r-m9r-acceptance-checklist.md
git commit -m "docs(archive): 实机回传清单追加 E 段——历史会话区八项（登记/筛选/激活闭环/回归锁）"
```

---

## 计划自审记录（写完即查，问题就地修）

1. **Spec 覆盖**：§3 裁决 1→Task 1/2（登记制）；2→Task 6 Step 6（按钮删除）；3→Task 6（仅移动端）；4→Task 3 Step 4（days 夹取）+ Task 6 Step 3（进页拉取/切天数重拉/无轮询）；5→Task 6 Step 6（独立页路由）；6→Task 6 Step 3/5（卡片无按钮、详情页唯一动作）；7→Task 5/6（项目下拉仅历史页，活板 Board 零改动）；8→Task 3 Step 4（DELETE 端点）+ Task 6（两个移除入口）。§5 表/守卫/不登记项→Task 1；§6.1→Task 3；§6.2→Task 4；§6.3 零改动→Task 6 Step 5 复用 fetchSessionMessages（畸形 404→「内容暂不可读」降级，ArchiveDetail contentError 分支）；§6.4 扫描零改动→Task 2 注释锚定；§7 四小节→Task 6；§8 边界 1/2/3/5/8 由数据流天然覆盖、4 明确非目标、6/7 由 Task 6 错误态覆盖；§9→各任务测试 + Task 7 E 段；§10 切分→Task 1–7 一一对应。**无缺口**。
2. **占位符扫描**：Task 1 Step 4 内含一段推导痕迹标注（「实现注意」指明删除占位链）——已在注释中显式标出，最终形态明确；Task 3 Step 1 构造点补齐以 grep 锚点给出（数量由既有代码决定，属机械重复）；Task 6 Step 3 的 `AGENT_TYPES` import 标注了删除路径。无 TBD/TODO。
3. **类型一致性**：`SessionArchiveRow` 九字段在 Task 1/3/4 三处使用一致；`ArchivedSession` 七字段 Task 5 定义与 Task 6 测试/组件一致；`archive_source`/`archive_delete` 签名 Task 3 定义、Task 4 消费一致；`relativeEndLabel`/`filterArchived*` Task 5 定义与 Task 6 消费一致；`onActivated`/`onOpenCard`/`onBack` props 两组件与 App 接线一致。
