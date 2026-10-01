# 远程新建会话 · Phase C 实施计划（单批 C0–C12）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 移动端一键「新建会话」：指定目录 + 四家终端工具（claude/codex/kimi/opencode），主机起终端 → 锚点式处置信任/更新弹窗 → 以发起设备身份注入首句 → 会话物化上板并自动跳转。

**Architecture:** 服务端新增创建状态机（`inject/create.rs`，缝驱动可测：spawn/屏读/注入/确认复用 `RemoteState` 既有缝）；弹窗识别走锚点账本新增「新建会话」场景（append-only，逐字取探测取证）；三个新端点挂 `/m/api/v1`；配对不确定信号暴露到会话数据并在投递回执提示。前端看板入口 + 表单 + 进度态。**单批一次做成**，末尾统一评审停点。

**Tech Stack:** Rust（axum 端点、sysinfo 进程锚定、CONOUT$ 屏读缝）+ React/TS（`src/mobile/`）。

**上位文档（执行前必读）：**
- spec：`docs/superpowers/specs/2026-09-27-remote-session-create-design.md`（§4 管线语义与红线为准）
- 探测定案 SSOT：`research/refs/phase2-消息注入/2026-09-27-新建会话四家探测定案.md`（**锚文案逐字、键序、时延、物化判据的唯一来源**——账本条目与键序常量从这里抄，禁止手造）

**接口对账记录（2026-10-02 已核实，执行者勿改判）：**
- `SpawnSpec` 是**枚举**（`resume.rs:53`：`Windows{program,args,cwd,new_console}` / `MacosApplescript{script}`）——env 扩展加在 Windows 变体；macOS 走脚本内联 `env K=V`（Mac 探测 M2 实证形态，归 Mac 后补批）。
- `Injector` 域（`engine.rs:424`）：`locate_and_inject_spec(pid, text, spec)` / `locate_and_send_key_spec(pid, key, spec)`——键域已支持单字符 `'2'`（`engine.rs control_records` 的 is_ascii_alphanumeric 分支）。
- `RemoteState` 既有缝（`server.rs:283`）：`session_source` / `store` / `injector` / `resume_spawner: Arc<dyn Fn(SpawnSpec)->Result<(),String>>` / `confirm_probe: Arc<dyn Fn(&str,&str,&str)->bool>` / `screen_probe: Arc<dyn Fn(&str,u32)->Option<Vec<String>>>` / `archive_source: Box<dyn Fn()->Vec<SessionArchiveRow>>`。
- `monitor::cwd::normalize_cwd_for_match` 为 `pub(crate)`（create.rs 同 crate 直用）。
- `remote/files.rs::is_sensitive_path` 是 **home 域语义**（只拦主目录基准内的敏感段）——create 需要**全局路径**黑名单，故复用 `SENSITIVE_DIRS` **表**（升 pub(crate)）而非该函数。
- `session_archive.last_seen` 为 RFC3339 UTC 字符串（`archive.rs` 写入 `chrono::Utc::now().to_rfc3339()`）——7 天窗用 `DateTime::parse_from_rfc3339`。
- 桌面审计页对 `action` **原样渲染**（`AuditLogSection.tsx:108`，无枚举映射）——新增 `create`/`dialog` action 零前端适配。
- enabledTools 数据源 = `host_source()`（settings DAO 同源）——create 端点工具门读同一源。

---

## §0 总则（红线，违反即返工）

1. **TDD**：每任务先失败测试再实现；纯核与状态机必须 fake 缝单测覆盖，实机验证归 `#[ignore]` E2E。
2. **门禁**：后端任务收尾 `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`；前端任务 `pnpm check && pnpm test && pnpm build:mobile`；C12 六门禁全量终跑。
3. **提交纪律**：每任务至少一个 commit，**全部本地不 push**；C12 后唯一停点：主线评审 → 用户真机验收。
4. **零接触红线**：单测用内存库（`DeviceStore::memory`）/ tempdir，绝不写真实 `~/.mam`（E2E 例外：审计/队列落真实库为验收语义，沿用 m9r_e2e 口径）；E2E 只碰自建临时目录会话，结束 `taskkill /T /F` 清场。
5. **键序红线（探测实证，测试锁死）**：claude 信任框默认 `❯ No, exit` → 必须 `down`+`enter`（直按 enter=退出）；codex 更新框默认 `Update now` → 必须字符 `'2'`（esc 无效、enter 禁用）；codex/kimi 信任框 enter 直通。
6. **锚文案来源红线**：账本 `text` 逐字取探测定案表 §4/§5/§7，`evidence` 指向 `create-probe/20260927-100552` 或 Mac 报告；缺锚先补探测，禁止手造。
7. **路径约束（v1）**：纯 ASCII（`non_ascii_path`）、本地卷（UNC 拒绝 `not_local_volume`）、黑名单（`blacklisted`）、绝对路径、缺失递归创建（失败 `mkdir_failed`）。

## §1 文件结构总览

| 文件 | 动作 | 职责 | 任务 |
|---|---|---|---|
| `docs/MASTER-PLAN.md` | 改 | D21 + F2.8 | C0 |
| `src-tauri/src/inject/anchor_ledger.rs` | 改 | 四场景常量 + 账本条目 | C1 |
| `src-tauri/src/inject/create_path.rs` | 建 | 路径校验纯核 | C2 |
| `src-tauri/src/remote/files.rs` | 改 | `SENSITIVE_DIRS` 升 pub(crate) | C2 |
| `src-tauri/src/inject/resume.rs` | 改 | SpawnSpec::Windows 增 env + create 规格 | C3 |
| `src-tauri/src/inject/create.rs` | 建 | 进程锚定 + 状态机内核 | C4/C5 |
| `src-tauri/src/remote/server.rs`、`api.rs`、`mod.rs` | 改 | 任务注册缝 + 三端点 + 审计 + 配对信号 | C6/C7 |
| `src-tauri/src/commands/session.rs` | 改 | `running_projects_from_processes` 升 pub(crate) | C7 |
| `src-tauri/tests/create_e2e.rs` | 建 | 四家实机 E2E + HTTP 端到端总测试 | C8/C9 |
| `src/mobile/api.ts` | 改 | 三方法 + 类型 | C10 |
| `src/mobile/CreateSessionSheet.tsx`、`Board.tsx` | 建/改 | 表单+进度+跳转 | C11 |
| `docs/release-notes/create-acceptance-checklist.md` | 建 | 手工验收清单 | C12 |

---

### Task C0: 宪法 D21 裁决落正文

**Files:** Modify `docs/MASTER-PLAN.md`

- [ ] **Step 1** §1.2 二期功能表末尾（F2.7 之后）追加：

```markdown
| F2.8 | 远程新建会话 | 手机端指定目录+工具（四家终端 CLI），主机起终端开新会话：锚点式处置信任/更新弹窗 → 发起设备身份注入首句 → 会话物化上板自动跳转（2026-10-02 D21 提前纳入二期收尾） |
```

- [ ] **Step 2** §2.2 二期输入输出表末尾追加：

```markdown
| F2.8 新建会话 | `/m/api/session-create`（tool、projectPath、firstMessage?） | 起终端（裸命令+DISABLE_AUTOUPDATER=1）→ 屏读锚点处置弹窗 → 首句注入（发起设备签名）→ 新会话文件物化；回执 taskId → phase 流转 → sessionId；审计 create/dialog/send 三类行 |
```

- [ ] **Step 3** 决策记录表（D20 之后）追加：

```markdown
| D21 | **远程新建会话提前为二期收尾功能**：原属三期「会话管理操作」域，因与二期注入链强耦合提前；边界=四家固定 CLI 裸命令、无任意参数，目录受黑名单+ASCII+本地卷约束，不构成「不暴露任意 shell」红线突破 | 2026-09-26 |
```

- [ ] **Step 4** Commit：`docs(charter): D21 远程新建会话提前为二期收尾（F2.8）`

### Task C1: 锚点账本「新建会话」四场景

**Files:** Modify `src-tauri/src/inject/anchor_ledger.rs`；锚文案逐字取探测定案表 §4/§5/§7

- [ ] **Step 1: 失败测试**（tests 模块追加）：

```rust
#[test]
fn create_scenarios_anchors_hit_verbatim() {
    use super::detect;
    let hit = |tool: &str, sc: &str, sl: &str, line: &str| {
        detect(&[line.to_lowercase()], tool, sc, sl).is_some()
    };
    // 信任框：claude（标题 + 双选项行都在 CONFIRM 槽）
    assert!(hit("claude", scenario::CREATE_TRUST, slot::TITLE,
        "Quick safety check: Is this a project you created or one you trust?"));
    assert!(hit("claude", scenario::CREATE_TRUST, slot::CONFIRM, "❯ No, exit"));
    assert!(hit("claude", scenario::CREATE_TRUST, slot::CONFIRM, "Yes, I trust this folder"));
    // codex 双形态：Windows 0.156.1（Folder access）与 Mac 0.155.1（长问句）同槽并存
    assert!(hit("codex", scenario::CREATE_TRUST, slot::TITLE, "Folder access"));
    assert!(hit("codex", scenario::CREATE_TRUST, slot::TITLE,
        "Do you trust the contents of this directory?"));
    assert!(hit("codex", scenario::CREATE_TRUST, slot::CONFIRM, "1. Trust and continue"));
    assert!(hit("codex", scenario::CREATE_TRUST, slot::CONFIRM, "› 1. Yes, continue"));
    // kimi（per-folder）
    assert!(hit("kimi", scenario::CREATE_TRUST, slot::TITLE, "Trust this folder?"));
    // 更新框：codex 实机构造 + claude 非阻塞横幅
    assert!(hit("codex", scenario::CREATE_UPDATE, slot::TITLE, "Update available"));
    assert!(hit("codex", scenario::CREATE_UPDATE, slot::CONFIRM, "1. Update now"));
    assert!(hit("claude", scenario::CREATE_UPDATE, slot::PRESENT, "✔ Update installed · Restart to apply"));
    // idle：claude 双版本同槽多文案（2026-09-23 裁决：版本不参与筛选）
    assert!(hit("claude", scenario::CREATE_IDLE, slot::PRESENT, "? for shortcuts"));
    assert!(hit("claude", scenario::CREATE_IDLE, slot::PRESENT, "auto mode on (shift+tab to cycle)"));
    assert!(hit("codex", scenario::CREATE_IDLE, slot::PRESENT, "Ask Codex to do anything"));
    assert!(hit("kimi", scenario::CREATE_IDLE, slot::PRESENT, "No session yet"));
    assert!(hit("opencode", scenario::CREATE_IDLE, slot::PRESENT, "Ask anything"));
    // 首装不可处置态（识别 → 专属失败码）
    assert!(hit("claude", scenario::CREATE_ONBOARD, slot::TITLE, "Unable to connect to Anthropic services"));
    assert!(hit("codex", scenario::CREATE_ONBOARD, slot::TITLE, "Sign in with ChatGPT"));
    // 既有场景零影响（append-only 基线）
    assert!(!super::candidates("codex", scenario::PERMISSION_MENU, slot::TITLE).is_empty());
}
```

- [ ] **Step 2** `cargo test create_scenarios_anchors --lib` → 编译错（常量不存在）。
- [ ] **Step 3: 实现**——`scenario` 模块追加：

```rust
/// 新建会话·信任框（spec §4.4；探测 create-probe/20260927-100552 P3a + Mac §③）
pub const CREATE_TRUST: &str = "create_trust";
/// 新建会话·更新提示（codex 实机构造 P3b + claude 横幅 Mac §③）
pub const CREATE_UPDATE: &str = "create_update";
/// 新建会话·空闲输入框（首句注入前置判据，P2b）
pub const CREATE_IDLE: &str = "create_idle";
/// 新建会话·首装/登录不可处置态（识别后专属报错，P3c）
pub const CREATE_ONBOARD: &str = "create_onboard";
```

`ANCHOR_LEDGER` 末尾追加（`text` 小写逐字；`evidence` 指向探测段；**claude 选项行放 CONFIRM 槽**——选项行语义，危险默认与信任项同槽并存）：

```rust
// ===== 新建会话·信任框（create_trust）=====
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "quick safety check", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p3a-claude（Mac 2.1.283 同文案）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "no, exit", observed_version: "2.1.278",
    evidence: "同上——危险默认项，处置必须 down+enter（键序红线）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "yes, i trust this folder", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p3a-claude" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "folder access", observed_version: "0.156.1",
    evidence: "create-probe/20260927-100552 p3a-codex-r2（Windows 形态）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "do you trust the contents of this directory", observed_version: "0.155.1",
    evidence: "Mac 报告 §③（Mac 形态；同槽多文案 append-only）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "1. trust and continue", observed_version: "0.156.1",
    evidence: "create-probe/20260927-100552 p3a-codex-r2（Windows）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "yes, continue", observed_version: "0.155.1",
    evidence: "Mac 报告 §③（`› 1. Yes, continue`）" },
AnchorRow { tool: "kimi", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "trust this folder?", observed_version: "2.0.2",
    evidence: "create-probe/20260927-100552 p3a-kimi（per-folder，与 HOME 无关）" },
// ===== 新建会话·更新提示（create_update）=====
AnchorRow { tool: "codex", scenario: scenario::CREATE_UPDATE, slot: slot::TITLE,
    text: "update available", observed_version: "0.156.1",
    evidence: "create-probe/20260927-100552 p3b（真实版本落差构造）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_UPDATE, slot: slot::CONFIRM,
    text: "1. update now", observed_version: "0.156.1",
    evidence: "同上——危险默认项，处置必须字符 '2'（esc 无效、enter 禁用）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_UPDATE, slot: slot::PRESENT,
    text: "update installed · restart to apply", observed_version: "2.1.283",
    evidence: "Mac 报告 §③ 自然构造（非阻塞横幅，不处置不阻断 idle 判定）" },
// ===== 新建会话·空闲输入框（create_idle；claude 双版本同槽）=====
AnchorRow { tool: "claude", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "? for shortcuts", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p2b（Windows 双宿主一致）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "auto mode on (shift+tab to cycle)", observed_version: "2.1.283",
    evidence: "Mac 报告 §③（同槽多文案，版本仅备注——2026-09-23 账本裁决）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "ask codex to do anything", observed_version: "0.156.1",
    evidence: "create-probe/20260927-100552 p2b" },
AnchorRow { tool: "kimi", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "no session yet", observed_version: "2.0.2",
    evidence: "同上——新建会话特有锚，兼验确为新会话" },
AnchorRow { tool: "opencode", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "ask anything", observed_version: "1.18.32",
    evidence: "create-probe/20260927-100552 p2b（首帧即达，无信任框）" },
// ===== 新建会话·首装/登录不可处置（create_onboard）=====
AnchorRow { tool: "claude", scenario: scenario::CREATE_ONBOARD, slot: slot::TITLE,
    text: "unable to connect to anthropic services", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p3c（403 网络墙）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_ONBOARD, slot: slot::TITLE,
    text: "sign in with chatgpt", observed_version: "0.156.1",
    evidence: "同上（登录三选屏，需真人）" },
```

- [ ] **Step 4** 测试过 → 门禁 → Commit `feat(create): 锚点账本新建会话四场景（探测逐字取证，codex 双形态同槽）`

### Task C2: 路径校验纯核

**Files:** Create `src-tauri/src/inject/create_path.rs`；Modify `src/inject/mod.rs`（挂模块）、`src/remote/files.rs`（`const SENSITIVE_DIRS` → `pub(crate) const`，调用点零改动）

- [ ] **Step 1: 失败测试**（create_path.rs 内嵌）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_relative_nonascii_unc() {
        assert_eq!(validate("", "windows").err().unwrap().code, "empty");
        assert_eq!(validate(r"relative\path", "windows").err().unwrap().code, "not_absolute");
        assert_eq!(validate(r"E:\项目\demo", "windows").err().unwrap().code, "non_ascii_path");
        assert_eq!(validate("/项目/demo", "macos").err().unwrap().code, "non_ascii_path");
        assert_eq!(validate(r"\\server\share\p", "windows").err().unwrap().code, "not_local_volume");
        assert_eq!(validate("//server/share", "macos").err().unwrap().code, "not_local_volume");
    }
    #[test]
    fn windows_requires_drive_form_macos_requires_slash() {
        assert_eq!(validate(r"/tmp/x", "windows").err().unwrap().code, "bad_windows_form");
        assert!(validate(r"C:\Users\u\proj", "windows").is_ok());
        assert!(validate("/Users/u/proj", "macos").is_ok());
    }
    #[test]
    fn blacklist_hits_and_passes() {
        assert_eq!(validate(r"C:\Windows\System32\x", "windows").err().unwrap().code, "blacklisted");
        assert_eq!(validate("/Users/u/.ssh/keys", "macos").err().unwrap().code, "blacklisted");
        assert!(validate(r"D:\work\proj", "windows").is_ok()); // 普通路径不误伤
    }
}
```

- [ ] **Step 2** 确认失败（模块不存在）。
- [ ] **Step 3: 实现**：

```rust
//! 新建会话路径校验纯核（spec §2：纯 ASCII / 本地卷 / 绝对路径 / 黑名单）。
//! 只判不建；创建（create_dir_all）在状态机校验段执行并如实回执。
//! 黑名单复用 remote::files::SENSITIVE_DIRS 表（非其 is_sensitive_path 函数——
//! 那是 home 域语义，create 需要全局路径拦截）。
pub struct PathReject { pub code: &'static str, pub message: String }

pub fn validate(path: &str, platform: &str) -> Result<(), PathReject> {
    let p = path.trim();
    if p.is_empty() { return Err(rej("empty", "路径为空")); }
    if !p.is_ascii() { return Err(rej("non_ascii_path", "v1 路径限纯 ASCII（CJK 路径留后批）")); }
    if p.starts_with(r"\\") || p.starts_with("//") {
        return Err(rej("not_local_volume", "v1 仅本地卷，UNC/网络路径不入本批"));
    }
    if platform == "windows" {
        let b = p.as_bytes();
        let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
        if !drive { return Err(rej("bad_windows_form", "Windows 路径须为 X:\\ 绝对形态")); }
    } else if !p.starts_with('/') {
        return Err(rej("not_absolute", "macOS 路径须为 / 开头绝对路径"));
    }
    if hits_blacklist(p) { return Err(rej("blacklisted", "路径命中系统危险目录黑名单")); }
    Ok(())
}
/// 全局路径黑名单命中（分隔符双态 + 段边界，与 files.rs 的表同源不同域）
fn hits_blacklist(p: &str) -> bool {
    let low = p.to_ascii_lowercase().replace('/', "\\");
    crate::remote::files::SENSITIVE_DIRS.iter().any(|d| {
        let d = d.to_ascii_lowercase().replace('/', "\\");
        low.contains(&format!("\\{d}\\")) || low.ends_with(&format!("\\{d}"))
    })
}
fn rej(code: &'static str, msg: &str) -> PathReject { PathReject { code, message: msg.to_string() } }
```

（`/Users/u/.ssh/keys` 例依赖 `SENSITIVE_DIRS` 含 `.ssh`——执行时以表实况为准调整测试断言，表内容不改。）
- [ ] **Step 4** 测试过 → 门禁 → Commit `feat(create): 路径校验纯核（ASCII/本地卷/黑名单/绝对路径）`

### Task C3: 起窗规格（SpawnSpec env 扩展）

**Files:** Modify `src-tauri/src/inject/resume.rs`

- [ ] **Step 1: 失败测试**：

```rust
#[test]
fn create_spawn_spec_carries_env_and_bare_command() {
    // conhost 分支：枚举事实形态（resume.rs:53 SpawnSpec::Windows）
    let s = build_create_spawn_spec(None, r"C:\proj", "claude");
    match s {
        SpawnSpec::Windows { program, args, env, .. } => {
            assert_eq!(program, "conhost.exe");
            assert_eq!(args, vec!["cmd", "/k", "claude"]);
            assert!(env.iter().any(|(k, v)| k == "DISABLE_AUTOUPDATER" && v == "1"));
        }
        _ => panic!("Windows 平台必须是 Windows 变体"),
    }
    // WT 分支：命令同为裸工具名、env 同带
    let w = build_create_spawn_spec(Some(r"C:\wt\wt.exe"), r"C:\proj", "codex");
    match w {
        SpawnSpec::Windows { args, env, .. } => {
            assert_eq!(args.last().unwrap(), "codex");
            assert!(!env.is_empty());
        }
        _ => panic!(),
    }
}
```

- [ ] **Step 2** 确认失败（`env` 字段与函数均不存在）。
- [ ] **Step 3: 实现**——① `SpawnSpec::Windows` 变体增 `env: Vec<(String, String)>` 字段（`build_spawn_command_windows` 两处构造补 `env: Vec::new()`，行为零变化）；② `spawn_terminal` Windows 分支 `Command` 追加 `.envs(spec.env.iter().map(|(k, v)| (k, v)))`；③ 新增：

```rust
/// 新建会话起窗规格：裸工具命令 + DISABLE_AUTOUPDATER=1（spec §4.2 环境红线；
/// S2 实测 conhost/WT 双宿主 4/4 透传，形态A=spawn 显式设 env）。
/// macOS 侧（MacosApplescript 变体）归 Mac 后补批：脚本内联 `env K=V ` 前缀（Mac 探测 M2 实证）。
#[cfg(windows)]
pub fn build_create_spawn_spec(wt: Option<&str>, cwd: &str, tool: &str) -> SpawnSpec {
    let mut spec = build_spawn_command_windows(wt, cwd, tool);
    if let SpawnSpec::Windows { env, .. } = &mut spec {
        *env = vec![("DISABLE_AUTOUPDATER".to_string(), "1".to_string())];
    }
    spec
}
```

- [ ] **Step 4** 测试过 → 门禁 → Commit `feat(create): 起窗规格携带 DISABLE_AUTOUPDATER（SpawnSpec env 扩展）`

### Task C4: TUI 进程锚定

**Files:** Create `src-tauri/src/inject/create.rs`（本任务建文件，先只放本函数 + mod.rs 挂模块）

- [ ] **Step 1: 失败测试**：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::cwd::normalize_cwd_for_match as norm;
    #[test]
    fn tui_candidate_matches_cwd_and_name_case_insensitive() {
        assert!(is_tui_candidate("claude", &norm(r"E:\proj\demo"), "claude.exe", &norm("e:/proj/demo")));
        assert!(is_tui_candidate("codex", &norm(r"E:\p"), "node.exe", &norm(r"E:\p")));
        assert!(!is_tui_candidate("claude", &norm(r"E:\other"), "claude.exe", &norm(r"E:\p")));
        assert!(!is_tui_candidate("kimi", &norm(r"E:\p"), "cmd.exe", &norm(r"E:\p"))); // 中间壳不算
        assert!(!is_tui_candidate("kimi", &norm(r"E:\p"), "powershell.exe", &norm(r"E:\p")));
    }
}
```

- [ ] **Step 2** 确认失败 → **Step 3: 实现**：

```rust
/// TUI 进程名集合（m9r_e2e launch_cli 同口径：原生 exe / node / bun；kimi 独立 exe）
fn tui_names(tool: &str) -> Vec<String> {
    let mut v = vec![format!("{tool}.exe"), "node.exe".to_string(), "bun.exe".to_string()];
    if tool == "kimi" { v.insert(0, "kimi.exe".to_string()); }
    v
}
fn is_tui_candidate(tool: &str, proc_cwd: &str, proc_name: &str, want_cwd: &str) -> bool {
    proc_cwd == want_cwd && tui_names(tool).iter().any(|n| n.eq_ignore_ascii_case(proc_name))
}
/// 起窗后轮询发现 TUI pid（sysinfo cwd+进程名；超时 Err 中文回执）。
/// 刷新口径与 resume 效果回查同源（RefreshKind with_cmd+with_cwd Always）。
pub fn find_tui_pid(tool: &str, dir: &std::path::Path, timeout: std::time::Duration) -> Result<u32, String> {
    let want = crate::monitor::cwd::normalize_cwd_for_match(&dir.to_string_lossy());
    let t0 = std::time::Instant::now();
    loop {
        let system = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::new().with_processes(
                sysinfo::ProcessRefreshKind::new()
                    .with_cmd(sysinfo::UpdateKind::Always)
                    .with_cwd(sysinfo::UpdateKind::Always),
            ),
        );
        for (pid, process) in system.processes() {
            if let Some(cwd) = process.cwd() {
                if is_tui_candidate(tool, &crate::monitor::cwd::normalize_cwd_for_match(&cwd.to_string_lossy()),
                    &process.name().to_string_lossy(), &want) {
                    return Ok(pid.as_u32());
                }
            }
        }
        if t0.elapsed() >= timeout {
            return Err(format!("起窗后 {timeout:?} 内未发现 {tool} 进程（目录 {}）", dir.display()));
        }
        std::thread::sleep(std::time::Duration::from_millis(1200));
    }
}
```

- [ ] **Step 4** 测试过 → 门禁 → Commit `feat(create): TUI 进程锚定（cwd+进程名，中间壳排除）`

### Task C5: 创建状态机内核

**Files:** Modify `src-tauri/src/inject/create.rs`

- [ ] **Step 1: 失败测试**：

```rust
#[cfg(test)]
mod pipeline_tests {
    use super::*;
    /// 缝集合（测试 fake；生产由 C6 从 RemoteState 装配）
    struct CreateDeps<'a> {
        screen: &'a dyn Fn(u32) -> Option<Vec<String>>,
        send_key: &'a dyn Fn(u32, &str) -> Result<(), String>,
        send_text: &'a dyn Fn(u32, &str) -> Result<(), String>,
    }
    fn deps_recording(screens: Vec<Vec<String>>, keys: &mut Vec<String>) -> CreateDeps<'static> {
        // 屏读按调用序弹 screens；键注入记录进 keys；文本注入恒 Ok
        let mut it = screens.into_iter();
        let screen: &'static dyn Fn(u32) -> Option<Vec<String>> = Box::leak(Box::new(move |_pid| it.next()));
        let key_sink: &'static std::sync::Mutex<Vec<String>> = Box::leak(Box::new(std::sync::Mutex::new(Vec::new())));
        let recorder: &'static dyn Fn(u32, &str) -> Result<(), String> = Box::leak(Box::new(move |_p, k| {
            key_sink.lock().unwrap().push(k.to_string()); Ok(())
        }));
        // 把外部 keys 桥接到 sink（测试结束取回）
        *key_sink.lock().unwrap() = keys.clone();
        let text: &'static dyn Fn(u32, &str) -> Result<(), String> = Box::leak(Box::new(|_, _| Ok(())));
        CreateDeps { screen, send_key: recorder, send_text: text }
    }

    fn params(tool: &str) -> Params { Params { tool: tool.into(), dir: "C:\\t".into(),
        first_message: "hi".into(), composed: "hi [mobile test-dev]".into() } }

    #[test]
    fn claude_trust_requires_down_then_enter() {
        let screens = vec![
            vec!["Quick safety check: Is this a project you created or one you trust?".into(), "❯ No, exit".into()],
            vec!["Quick safety check: ...".into(), "❯ No, exit".into()],
            vec!["manual mode on · ? for shortcuts".into()],
        ];
        let mut keys = Vec::new();
        let deps = deps_recording(screens, &mut keys);
        let out = run_pipeline(&deps, &params("claude"));
        assert!(matches!(out.status, CreateStatus::WaitingMaterialize));
        assert_eq!(out.keys_sent, vec!["down", "enter"]); // 红线：直按 enter=退出
    }
    #[test]
    fn codex_update_then_trust_with_char2() {
        let screens = vec![
            vec!["Update available".into(), "1. Update now (runs npm install -g @openai/codex)".into(), "2. Skip".into()],
            vec!["Update available".into(), "1. Update now".into()],
            vec!["Folder access".into(), "Trust this folder?".into(), "1. Trust and continue".into()],
            vec!["Ask Codex to do anything".into()],
        ];
        let mut keys = Vec::new();
        let deps = deps_recording(screens, &mut keys);
        let out = run_pipeline(&deps, &params("codex"));
        assert!(matches!(out.status, CreateStatus::WaitingMaterialize));
        assert_eq!(out.keys_sent, vec!["2", "enter"]); // '2'=Skip（esc 无效、enter 禁用）
    }
    #[test]
    fn unrecognized_screen_fails_without_any_key() {
        let screens = (0..16).map(|_| vec!["某种未识别界面".to_string()]).collect();
        let mut keys = Vec::new();
        let deps = deps_recording(screens, &mut keys);
        let out = run_pipeline(&deps, &params("claude"));
        assert!(matches!(out.status, CreateStatus::Failed { code, .. } if code == "unrecognized_screen"));
        assert!(keys.is_empty()); // 不盲打
    }
    #[test]
    fn onboard_screen_fails_with_specific_code() {
        let screens = (0..2).map(|_| vec!["Unable to connect to Anthropic services".into()]).collect();
        let mut keys = Vec::new();
        let deps = deps_recording(screens, &mut keys);
        let out = run_pipeline(&deps, &params("claude"));
        assert!(matches!(out.status, CreateStatus::Failed { code, .. } if code == "network_wall"));
        let mut k2 = Vec::new();
        let d2 = deps_recording((0..2).map(|_| vec!["Sign in with ChatGPT".into()]).collect(), &mut k2);
        let o2 = run_pipeline(&d2, &params("codex"));
        assert!(matches!(o2.status, CreateStatus::Failed { code, .. } if code == "login_wall"));
    }
    #[test]
    fn first_message_blocked_until_idle_anchor() {
        // 屏幕停在信任框（未到 idle）→ 阶段窗耗尽 failed，send_text 从未被调
        let screens = (0..16).map(|_| vec!["Quick safety check".to_string()]).collect();
        let text_called = std::sync::atomic::AtomicBool::new(false);
        let screen_vec = screens; let mut it = screen_vec.into_iter();
        let deps = CreateDeps {
            screen: &|_p| it.next(),
            send_key: &|_, _| Ok(()),
            send_text: &|_, _| { text_called.store(true, SeqCst); Ok(()) },
        };
        let out = run_pipeline(&deps, &params("kimi"));
        assert!(matches!(out.status, CreateStatus::Failed { .. }));
        assert!(!text_called.load(SeqCst));
    }
}
```

- [ ] **Step 2** 确认失败 → **Step 3: 实现**：

```rust
pub enum CreateStatus {
    OpeningTerminal, DialogHandling, InjectingFirst, WaitingMaterialize,
    Done { session_id: String },
    Failed { phase: &'static str, code: String, message: String },
}
pub struct Params { pub tool: String, pub dir: PathBuf, pub first_message: String, pub composed: String }
pub struct CreateOutcome { pub status: CreateStatus, pub keys_sent: Vec<String>,
    pub dialog_log: Vec<(String, Vec<String>)> } // (场景, 键序) 供审计逐条留痕

/// 弹窗处置键序（探测定案 §4/§5）：账本管「认屏」，键序是引擎域常量（与 approve 映射同构）
fn disposal_keys(tool: &str, scenario: &str) -> Vec<&'static str> {
    match (tool, scenario) {
        ("claude", s) if s == scenario::CREATE_TRUST => vec!["down", "enter"],
        ("codex", s) if s == scenario::CREATE_UPDATE => vec!["2"],
        ("codex", _) | ("kimi", _) => vec!["enter"],
        _ => vec!["enter"],
    }
}
/// 状态机主循环（纯内核 + 缝）：2s 步距屏读 → anchor_ledger 逐场景 detect →
/// CREATE_ONBOARD 命中即 Failed（专属码）→ CREATE_UPDATE 的 PRESENT（claude 横幅）
/// 不处置不阻断 → 处置前重读确认场景在场 → CREATE_IDLE/PRESENT 命中才 send_text
/// （composed 已含签名）→ WaitingMaterialize。连续 15 轮（30s）无任何锚 →
/// Failed(unrecognized_screen)。物化轮询不在本层（归 C6 调用层）。
pub fn run_pipeline(deps: &CreateDeps, p: &Params) -> CreateOutcome { /* 按上述语义实现 */ }
```

  实现注释要点：① 屏读 `None` 计一次「未识别」；② 发键间隔 1.5s、发后 2.5s 沉降（探测 P3a 实测处置→idle 1–4.5s）；③ `keys_sent` 记录实际所发键供断言与审计。
- [ ] **Step 4** 测试过 → 门禁 → Commit `feat(create): 创建状态机内核（锚点驱动处置+idle 门+首句注入）`

### Task C6: API 三端点 + 任务注册表 + 审计

**Files:** Modify `src-tauri/src/remote/server.rs`（RemoteState + 路由）、`api.rs`（端点）、`mod.rs`（STATE 装配）；Modify `src-tauri/src/inject/create.rs`（物化发现函数）

- [ ] **Step 1: 失败端点测试**（server.rs 既有内存库 + fake 缝模式）：

```rust
#[test]
fn session_create_contract() {
    // ① 非法路径 → 400 {error:"bad_request", reasonCode:"non_ascii_path"}
    // ② 未装/未启用工具 → 400 reasonCode:"tool_unavailable"
    // ③ 合法 → 200 {taskId}；任务在飞第二个请求 → 409 {error:"conflict"}
    // ④ status：未知 taskId → 404；在飞 → {phase, detail?}
    // ⑤ 审计三类行：action=create（成功/failed:<code>）、action=dialog（summary=场景+键序）、
    //    action=send（首句，设备=发起设备 cookie）
    // ⑥ create-projects：archive fake 给 7 天内/8 天前/非 ASCII 三条 + session fake 一条活跃
    //    → 响应只含 7 天内与活跃项（去重、按 lastActiveAt 排序、字段
    //    {path, lastActiveAt, tools[], activeTools[]}）
}
```

- [ ] **Step 2** 确认失败 → **Step 3: 实现**：
  - `RemoteState` 增 `pub create_tasks: Arc<Mutex<HashMap<u64, CreateTaskShared>>>`（自增 id；`CreateTaskShared { phase, detail, session_id, spawned_pid }`，短临界区快照读写）；STATE 装配空表。
  - **工具门**：`tool_available(tool)` = enabledTools（`host_source()` 同源 settings）∩ 安装探测（PATH 目录扫描 `tool.exe`，OnceLock 缓存——探测 P0 已证 `where` 形态）。
  - **POST /session-create**：校验（工具门 → `create_path::validate` → `create_dir_all` 失败 400 `mkdir_failed`）→ 单飞 409 → `spawn_blocking` 管线：`build_create_spawn_spec`（WT 探测复用 `resume::windows_terminal_path` 口径）→ `st.resume_spawner` → `find_tui_pid`（30s 窗）→ `run_pipeline`（缝装配：`screen` = `st.screen_probe("", pid)`、`send_key`/`send_text` = `st.injector.locate_and_send_key_spec / locate_and_inject_spec` + `families::family_for(tool)`；`composed` = `normalize::compose_injection(&device_name, first_message)`）→ **物化**：`discover_new_session(tool, dir, since)`（claude=`~/.claude/projects/<slug>/` mtime>since 最新 jsonl 取 sid；codex=今日 rollout 且首行 cwd 匹配；kimi=sessions 下 wd_* 新目录取 session id；opencode=**拷 db+wal+shm 三件套**查 session 表 by directory——探测 P4 红线）→ `st.confirm_probe(tool, sid, stamp)`（stamp=`confirm::stamp_of(&composed)` 剥签名后）命中 → Done；全程审计三类行（物化前 session_id 传 `""`，失败行 result=`failed:<code>`）。
  - **GET /session-create/status**：注册表快照 → `{phase, detail?, sessionId?}`；未知 404。
  - **GET /create-projects?days=**：`(st.session_source)()` ∪ `(st.archive_source)()`（RFC3339 解析过 N 天），双方过 `create_path::validate` 的 ASCII 检查，去重，`activeTools` 由快照同目录活跃会话聚合。
- [ ] **Step 4** 端点测试过 → fmt/clippy/test → Commit `feat(create): session-create 三端点+内存任务态+审计三类行`

### Task C7: 配对不确定信号暴露

**Files:** Modify `src-tauri/src/commands/session.rs`（`running_projects_from_processes` 升 `pub(crate)`，桌面门逻辑零改动）、`src-tauri/src/remote/api.rs`

- [ ] **Step 1: 失败测试**：sessions 响应含 `pairingAmbiguous: true`（fake 快照两条同工具同项目 + 进程计数 ≥2）；session-send 成功回执追加 `pairingHint: true`（同态，**不拦截**，delivered 照常）；单会话时两字段均 false/缺席。
- [ ] **Step 2: 实现**：api.rs 增 `fn pairing_ambiguous_set() -> HashSet<(String, String)>`（sysinfo 快照 → `running_projects_from_processes` 计数 ≥2 → (tool, project_name) 小写归一——与桌面门 require_evidence 同判据同口径）；sessions 序列化前打标；session_send 成功回执 JSON 追加 `pairingHint`。桌面门不动。
- [ ] **Step 3** 测试过 → 门禁 → Commit `feat(create): 配对不确定信号暴露（sessions 标记+投递回执提示，不拦截）`

### Task C8: 四家实机 E2E

**Files:** Create `src-tauri/tests/create_e2e.rs`（全 `#[ignore]`，复用 m9r_e2e 的 Ev/清场纪律；不依赖 MAM dev——直调生产内核 + 真缝）

- [ ] **Step 1: 写 E2E**：

```rust
/// 四家矩阵：全新 temp 目录 → build_create_spawn_spec 起窗 → find_tui_pid →
/// run_pipeline（真屏读/真注入缝）→ discover_new_session → confirm stamp →
/// 断言会话文件首条用户消息 == "hi [mobile <名>]"（compose 签名逐字）→ taskkill 清场。
/// 附加断言：claude 轮 keys_sent == [down, enter]；codex 双框轮 == [2, enter]（键序红线实机复核）。
#[test] #[ignore = "实机显式跑"] fn e2e_create_matrix_four_tools() { /* … */ }
```

- [ ] **Step 2** 实机跑 `cargo test --test create_e2e -- --ignored --nocapture --test-threads=1` 四家全过；实跑台账写文件头注释（m9r_e2e 同款）。
- [ ] **Step 3** Commit `test(create): 四家新建全链实机 E2E + 台账`

### Task C9: HTTP 端到端总测试（产品路径全链）

**Files:** Modify `src-tauri/tests/create_e2e.rs`

- [ ] **Step 1: 写用例**（`#[ignore]`，模式 = m9r_e2e 的 http-full-chain：内存库起 axum 服务 + 真注入器/真 spawner）：

```rust
/// 端到端总测试（移动端等效路径）：临时目录 → POST /m/api/v1/session-create
/// （带设备 cookie）→ 轮询 /session-create/status 至 done{sessionId} →
/// GET /sessions 含新会话卡 → POST /session-send 第二条消息 delivered →
/// 审计页五类行可查（create + dialog + send×2 + flush）→ taskkill 清场。
#[test] #[ignore = "实机显式跑"] fn e2e_create_http_full_chain() { /* … */ }
```

- [ ] **Step 2** 实机跑通过 → Commit `test(create): HTTP 端到端总测试（创建→物化→上板→二次投递→审计全链）`

### Task C10: 移动端 API 客户端

**Files:** Modify `src/mobile/api.ts`
- [ ] 失败测试（vitest，mock fetch）→ 实现 `fetchCreateProjects(days)` / `createSession(body)` / `fetchCreateStatus(taskId)`（409/404/400 reasonCode 分支类型化）→ `pnpm test` 过 → Commit `feat(mobile): 新建会话 API 客户端三方法`

### Task C11: 新建会话 UI（表单 + 进度 + 跳转）

**Files:** Create `src/mobile/CreateSessionSheet.tsx`；Modify `src/mobile/Board.tsx`（头部操作区「+」入口，样式对齐既有 chips/BookmarkBar）、`src/mobile/mobile.css`（如需）
- [ ] 失败测试（vitest）→ 实现三段：
  1. **表单**：工具四选（host.enabledTools ∩ 安装探测结果置灰带原因）；目录候选下拉（create-projects，显示 lastActiveAt+tools）+ 手填切换（前端形态提示，服务端权威）；首句输入默认占位 `hi`；**黄字**：选中 tool ∈ 该项目 `activeTools` 时显示「该项目已有该工具的活跃会话，多实例下后续消息路由可能混淆」（spec §2 语义：≥1 活跃会话；与 pairingAmbiguous 投递提示分层）。
  2. **进度态**：phase → 中文（校验/开终端/处置弹窗/注入首句/等待上板），2s 轮询 status；`done.sessionId` → 等看板快照见新卡后跳 SessionDetail（复用既有导航）；`failed` → 分阶段原因 + 重试；404 → 「任务已失效（主机可能重启），请重试」。
  3. 409 → 「已有创建任务进行中」。
- [ ] `pnpm check && pnpm test && pnpm build:mobile` → Commit `feat(mobile): 新建会话入口+表单+进度态+自动跳转`

### Task C12: 验收清单 + 六门禁终跑 + 交付停点

**Files:** Create `docs/release-notes/create-acceptance-checklist.md`
- [ ] 清单覆盖 spec §7 全部人工项：四家全链（含信任框/更新提示路径）、未识别界面报错、多实例黄字、路径边界（黑名单/盘不存在/深层创建/非 ASCII/UNC）、审计三类行逐条、用户真机手机走查（新建→进度→跳转→收发）。
- [ ] **六门禁终跑**（cargo fmt/clippy/test 全套 + pnpm check/test/build:mobile）零回归；E2E 两例复跑记录。
- [ ] Commit → **唯一停点**：向主线交付简报（任务×commit×门禁表 + E2E 实跑数字 + 自裁决清单），评审通过 → 用户真机验收 → 统一 push。

---

## Self-Review（2026-10-02 二轮审计后记录）

- **Spec 覆盖**：§2→C6/C10/C11（黄字 activeTools 分层已对齐 spec §5 修订）；§3→C6（phase 列表已同步去掉 validating——同步 400 不产生任务）；§4.1–4.8→C2/C3/C4/C5/C6（4.7 失败不清场=只回执不杀进程；4.8 codex hooks 提示挂 C6 Done detail）；§5→C7；§6→C0；§7→C8/C9/C12。
- **接口对账**：SpawnSpec 枚举形态/Injector 域/四缝可见性/键域单字符/RFC3339/审计页原样渲染/enabledTools 源——全部与代码实况核对（见文件头「接口对账记录」）。
- **占位扫描**：`run_pipeline`/`find_tui_pid` 循环体/E2E 体为「语义+断言齐备」的实现填写型骨架，非 TBD；其余步骤均含完整代码或逐字内容。
- **类型一致性**：`CreateStatus`/`CreateOutcome{status, keys_sent, dialog_log}`/`Params{composed}`/`PathReject.code` 谓词在 C2/C5/C6/C11 间一致；原因码与 spec §2 逐一对应。
