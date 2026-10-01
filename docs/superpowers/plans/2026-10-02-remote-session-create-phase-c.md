# 远程新建会话 · Phase C 实施计划（批次一后端 / 批次二前端）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 移动端一键「新建会话」：指定目录 + 四家终端工具（claude/codex/kimi/opencode），主机起终端 → 锚点式处置信任/更新弹窗 → 以发起设备身份注入首句 → 会话物化上板并自动跳转。

**Architecture:** 服务端新增创建状态机（`inject/create.rs`，缝驱动可测：spawn/屏读/注入/确认四缝复用 `RemoteState` 既有注入器与探针缝）；弹窗识别走锚点账本新增「新建会话」场景（append-only）；三个新端点挂 `/m/api/v1`；配对不确定信号暴露到会话数据并在投递回执提示。前端在看板加入口 + 表单 + 进度态。

**Tech Stack:** Rust（axum 端点、sysinfo 进程锚定、CONOUT$ 屏读缝）+ React/TS（mobile 入口 `src/mobile/`）。

**上位文档（执行前必读）：**
- spec：`docs/superpowers/specs/2026-09-27-remote-session-create-design.md`（§4 管线语义与红线为准）
- 探测定案 SSOT：`research/refs/phase2-消息注入/2026-09-27-新建会话四家探测定案.md`（**锚文案逐字、键序、时延、物化判据的唯一来源**——账本条目与键序常量从这里抄，禁止手造）

---

## §0 总则（红线，违反即返工）

1. **TDD**：每个任务先写失败测试再实现；纯核与状态机必须单测覆盖（fake 缝驱动），实机验证归 `#[ignore]` E2E。
2. **门禁**：每任务收尾 `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`（后端任务）或 `pnpm check && pnpm test && pnpm build:mobile`（前端任务）；批次一/二末各跑一次**六门禁全量**。
3. **提交纪律**：每任务至少一个 commit（英文 message 可用），**全部留本地不 push**——批次一完成停下等主线评审，评审通过后放行批次二；批次二完成再次评审。文档类改动（C0 宪法修订）单独 commit。
4. **零接触红线**：测试用内存库（`DeviceStore::memory`）/ tempdir，绝不写真实 `~/.mam`（E2E 例外：审计与队列落真实库为验收语义，沿用 m9r_e2e 口径）；E2E 只碰自建临时目录会话，结束 `taskkill /T /F` 清场。
5. **键序红线（探测实证，测试必须锁定）**：claude 信任框默认 `❯ No, exit` → 必须 `down`+`enter`，直按 enter=退出；codex 更新框默认 `Update now` → 必须字符 `'2'`，esc 无效、enter 禁用；codex/kimi 信任框 enter 直通。
6. **锚文案来源红线**：账本条目的 `text` 逐字取自探测定案表 §4/§5/§7，`evidence` 字段指向探测 run（`create-probe/20260927-100552` 或 Mac 报告）；发现需要新锚时先补探测取证，禁止手造。
7. **路径约束（v1）**：纯 ASCII、本地卷（UNC 拒绝）、黑名单拦截（复用 `remote/files.rs::SENSITIVE_DIRS`）、缺失递归创建。

## §1 文件结构总览

| 文件 | 动作 | 职责 |
|---|---|---|
| `docs/MASTER-PLAN.md` | 改 | D21 裁决 + F2.8 条目（C0） |
| `src-tauri/src/inject/anchor_ledger.rs` | 改 | 新场景常量 + 账本条目（C1） |
| `src-tauri/src/inject/create_path.rs` | 建 | 路径校验纯核（C2） |
| `src-tauri/src/inject/resume.rs` | 改 | `SpawnSpec` 增 env 字段 + spawn 应用（C3） |
| `src-tauri/src/inject/create.rs` | 建 | 进程锚定 + 创建状态机内核（C4/C5） |
| `src-tauri/src/remote/server.rs` | 改 | `RemoteState` 增任务注册表缝（C6） |
| `src-tauri/src/remote/api.rs` | 改 | 三端点 + 审计 + 配对信号（C6/C7） |
| `src-tauri/src/remote/mod.rs` | 改 | STATE 装配（C6） |
| `src-tauri/src/commands/session.rs` | 改 | `running_projects_from_processes` 升 pub(crate)（C7） |
| `src-tauri/src/remote/files.rs` | 改 | `SENSITIVE_DIRS` 升 pub(crate)（C2） |
| `src-tauri/tests/create_e2e.rs` | 建 | 四家实机 E2E（C8） |
| `src/mobile/api.ts`、`src/mobile/CreateSessionSheet.tsx`、`src/mobile/Board.tsx` | 改/建 | 前端（C9-C11） |
| `docs/release-notes/`（验收清单） | 建 | 手工验收清单（C12） |

---

# 批次一（后端全链，C0–C8）

### Task C0: 宪法 D21 裁决落正文

**Files:** Modify `docs/MASTER-PLAN.md`

- [ ] **Step 1** §1.2 二期功能表末尾（F2.7 之后）追加：

```markdown
| F2.8 | 远程新建会话 | 手机端指定目录+工具（四家终端 CLI），主机起终端开新会话：锚点式处置信任/更新弹窗 → 发起设备身份注入首句 → 会话物化上板自动跳转（2026-10-02 D21 提前纳入二期收尾） |
```

- [ ] **Step 2** §2.2 二期输入输出表末尾追加：

```markdown
| F2.8 新建会话 | `/m/api/session-create`（tool、projectPath、firstMessage?） | 起终端（裸命令+DISABLE_AUTOUPDATER=1）→ 屏读锚点处置弹窗 → 首句注入（发起设备签名）→ 新会话文件物化；回执 taskId → phase 流转 → sessionId；审计 create/弹窗处置/send 三类行 |
```

- [ ] **Step 3** 决策记录表（D20 之后）追加：

```markdown
| D21 | **远程新建会话提前为二期收尾功能**：原属三期「会话管理操作」域，因与二期注入链（首句注入/队列/确认层）强耦合提前；边界=四家固定 CLI 裸命令、无任意参数，目录受黑名单+ASCII+本地卷约束，不构成「不暴露任意 shell」红线的突破 | 2026-09-26 |
```

- [ ] **Step 4** Commit: `git add docs/MASTER-PLAN.md && git commit -m "docs(charter): D21 远程新建会话提前为二期收尾（F2.8 条目）"`

### Task C1: 锚点账本「新建会话」场景

**Files:** Modify `src-tauri/src/inject/anchor_ledger.rs`；锚文案来源 = 探测定案表 §4/§5/§7 逐字

- [ ] **Step 1: 写失败测试**（文件尾部 tests 模块内追加）

```rust
#[test]
fn create_scenarios_anchors_hit_verbatim() {
    use super::{candidates, detect};
    let hit = |tool: &str, sc: &str, sl: &str, line: &str| {
        detect(&[line.to_lowercase()], tool, sc, sl).is_some()
    };
    // 信任框标题（逐字取探测定案 §4）
    assert!(hit("claude", scenario::CREATE_TRUST, slot::TITLE,
        "Quick safety check: Is this a project you created or one you trust?"));
    assert!(hit("claude", scenario::CREATE_TRUST, slot::TITLE, "❯ No, exit"));
    assert!(hit("codex", scenario::CREATE_TRUST, slot::TITLE,
        "Do you trust the contents of this directory?"));
    assert!(hit("kimi", scenario::CREATE_TRUST, slot::TITLE, "Trust this folder?"));
    // 危险默认项锚（claude）与安全默认项（codex/kimi）
    assert!(hit("claude", scenario::CREATE_TRUST, slot::CONFIRM, "Yes, I trust this folder"));
    assert!(hit("codex", scenario::CREATE_TRUST, slot::CONFIRM, "1. Trust and continue"));
    // 更新框（codex 实机构造 + claude 横幅）
    assert!(hit("codex", scenario::CREATE_UPDATE, slot::TITLE, "Update available"));
    assert!(hit("codex", scenario::CREATE_UPDATE, slot::CONFIRM, "1. Update now"));
    assert!(hit("claude", scenario::CREATE_UPDATE, slot::PRESENT, "Update installed · Restart to apply"));
    // idle 锚（含 claude 双版本同槽）
    assert!(hit("claude", scenario::CREATE_IDLE, slot::PRESENT, "? for shortcuts"));
    assert!(hit("claude", scenario::CREATE_IDLE, slot::PRESENT, "auto mode on (shift+tab to cycle)"));
    assert!(hit("codex", scenario::CREATE_IDLE, slot::PRESENT, "Ask Codex to do anything"));
    assert!(hit("kimi", scenario::CREATE_IDLE, slot::PRESENT, "No session yet"));
    assert!(hit("opencode", scenario::CREATE_IDLE, slot::PRESENT, "Ask anything"));
    // 首装不可处置态（登记识别 → 专属报错）
    assert!(hit("claude", scenario::CREATE_ONBOARD, slot::TITLE, "Unable to connect to Anthropic services"));
    assert!(hit("codex", scenario::CREATE_ONBOARD, slot::TITLE, "Sign in with ChatGPT"));
    // append-only 基线：既有场景不受影响
    assert!(!candidates("claude", scenario::CREATE_TRUST, slot::TITLE).is_empty());
    assert!(!candidates("codex", scenario::PERMISSION_MENU, slot::TITLE).is_empty());
}
```

- [ ] **Step 2: 跑测试确认失败**（`cargo test create_scenarios_anchors --lib`，期望编译错：`CREATE_TRUST` 不存在）
- [ ] **Step 3: 实现**——`scenario` 模块追加四个常量：

```rust
/// 新建会话·信任框（spec 2026-09-27 §4.4；探测 create-probe/20260927-100552 P3a）
pub const CREATE_TRUST: &str = "create_trust";
/// 新建会话·更新提示（codex 实机构造 + claude 横幅；探测 P3b / Mac 报告 M3）
pub const CREATE_UPDATE: &str = "create_update";
/// 新建会话·空闲输入框（首句注入前置判据；探测 P2b）
pub const CREATE_IDLE: &str = "create_idle";
/// 新建会话·首装/登录不可处置态（识别后给专属报错而非泛化未识别）
pub const CREATE_ONBOARD: &str = "create_onboard";
```

`ANCHOR_LEDGER` 末尾追加条目（`text` 全小写、逐字抄探测定案表；每条 `evidence` 指向 `create-probe/20260927-100552` 对应段或 Mac 报告 §③；claude 信任框附 Mac 2.1.283 同文案第二行同槽追加）：

```rust
// ===== 新建会话·信任框（create_trust）=====
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "quick safety check", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p3a-claude 屏读逐字（Mac 2.1.283 同文案）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "no, exit", observed_version: "2.1.278",
    evidence: "同上——危险默认项，处置必须 down+enter（键序红线）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "yes, i trust this folder", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p3a-claude" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::TITLE,
    text: "do you trust the contents of this directory", observed_version: "0.156.1",
    evidence: "create-probe/20260927-100552 p3a-codex-r2（Mac 0.155.1 同文案）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "1. trust and continue", observed_version: "0.156.1",
    evidence: "同上（Mac 形态 `› 1. Yes, continue` 追加为第二行）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_TRUST, slot: slot::CONFIRM,
    text: "yes, continue", observed_version: "0.155.1",
    evidence: "Mac 报告 §③ codex 信任框逐字" },
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
    evidence: "Mac 报告 §③ 自然构造（非阻塞横幅，无需处置）" },
// ===== 新建会话·空闲输入框（create_idle；claude 双版本同槽多文案）=====
AnchorRow { tool: "claude", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "? for shortcuts", observed_version: "2.1.278",
    evidence: "create-probe/20260927-100552 p2b（Windows 双宿主一致）" },
AnchorRow { tool: "claude", scenario: scenario::CREATE_IDLE, slot: slot::PRESENT,
    text: "auto mode on (shift+tab to cycle)", observed_version: "2.1.283",
    evidence: "Mac 报告 §③（版本槽位→同槽多文案，版本仅备注）" },
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
    evidence: "create-probe/20260927-100552 p3c（403 网络墙，识别即失败回执）" },
AnchorRow { tool: "codex", scenario: scenario::CREATE_ONBOARD, slot: slot::TITLE,
    text: "sign in with chatgpt", observed_version: "0.156.1",
    evidence: "同上（登录三选屏，需真人）" },
```

- [ ] **Step 4: 跑测试通过**（`cargo test create_scenarios_anchors --lib` → PASS）
- [ ] **Step 5: 门禁 + Commit** `feat(create): 锚点账本新建会话四场景（探测逐字取证入账）`

### Task C2: 路径校验纯核

**Files:** Create `src-tauri/src/inject/create_path.rs`；Modify `src-tauri/src/inject/mod.rs`（挂模块）、`src-tauri/src/remote/files.rs`（`SENSITIVE_DIRS` 升 `pub(crate)`，一处 `const`→`pub(crate) const`，调用点零改动）

- [ ] **Step 1: 写失败测试**（create_path.rs 内嵌 tests）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_absolute_empty_and_non_ascii() {
        assert_eq!(validate("".into(), "windows").err().unwrap().code, "empty");
        assert_eq!(validate(r"relative\path".into(), "windows").err().unwrap().code, "not_absolute");
        assert_eq!(validate(r"E:\项目\demo".into(), "windows").err().unwrap().code, "non_ascii_path");
        assert_eq!(validate("/项目/demo".into(), "macos").err().unwrap().code, "non_ascii_path");
    }
    #[test]
    fn rejects_unc_and_windows_drive_rule() {
        // UNC/网络路径：首版一律拒绝（本地卷约束）
        assert_eq!(validate(r"\\server\share\p".into(), "windows").err().unwrap().code, "not_local_volume");
        // Windows 必须盘符形态；裸 /path 拒绝
        assert_eq!(validate(r"/tmp/x".into(), "windows").err().unwrap().code, "bad_windows_form");
    }
    #[test]
    fn accepts_valid_and_rejects_blacklist() {
        assert!(validate(r"C:\Users\u\proj".into(), "windows").is_ok());
        assert!(validate("/Users/u/proj".into(), "macos").is_ok());
        let code = validate(r"C:\Windows\System32\x".into(), "windows").err().unwrap().code;
        assert_eq!(code, "blacklisted");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**（模块不存在，编译错）
- [ ] **Step 3: 实现**

```rust
//! 新建会话路径校验纯核（spec §2：纯 ASCII / 本地卷 / 黑名单 / 绝对路径）。
//! 只判不建；创建动作（create_dir_all）在状态机校验段执行并如实回执失败。

pub struct PathReject { pub code: &'static str, pub message: String }

/// 校验（纯函数，跨平台可测）。`platform` = `std::env::consts::OS` 原值。
pub fn validate(path: &str, platform: &str) -> Result<(), PathReject> {
    let p = path.trim();
    if p.is_empty() { return Err(rej("empty", "路径为空")); }
    if !p.is_ascii() { return Err(rej("non_ascii_path", "v1 路径限纯 ASCII（含中文路径留后批）")); }
    if p.starts_with(r"\\") || p.starts_with("//") {
        return Err(rej("not_local_volume", "v1 仅支持本地卷，UNC/网络路径不入本批"));
    }
    if platform == "windows" {
        let b = p.as_bytes();
        let drive_form = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
        if !drive_form { return Err(rej("bad_windows_form", "Windows 路径须为 X:\\ 形态的绝对路径")); }
    } else if !p.starts_with('/') {
        return Err(rej("not_absolute", "macOS 路径须为 / 开头的绝对路径"));
    }
    if crate::remote::files::SENSITIVE_DIRS.iter().any(|d| {
        let low = p.to_ascii_lowercase();
        low.contains(&format!("\\{}\\", d.to_ascii_lowercase())) || low.contains(&format!("/{}//", d.to_ascii_lowercase())) || low.ends_with(&format!("\\{}", d)) || low.ends_with(&format!("/{}", d))
    }) {
        return Err(rej("blacklisted", "路径命中系统危险目录黑名单"));
    }
    Ok(())
}
fn rej(code: &'static str, msg: &str) -> PathReject { PathReject { code, message: msg.to_string() } }
```

（黑名单判定与 `remote/files.rs` 既有 `SENSITIVE_DIRS` 同表；若 files.rs 已有可复用的命中函数，改为调用它而不是重写匹配逻辑——执行时二选一，DRY 优先。）

- [ ] **Step 4: 跑测试通过** → 门禁 → Commit `feat(create): 路径校验纯核（ASCII/本地卷/黑名单/绝对路径）`

### Task C3: SpawnSpec 环境变量扩展

**Files:** Modify `src-tauri/src/inject/resume.rs`

- [ ] **Step 1: 写失败测试**（resume.rs tests 模块）：

```rust
#[test]
fn create_spawn_spec_carries_env_and_bare_command() {
    let spec = build_create_spawn_spec(None, r"C:\proj", "claude");
    assert_eq!(spec.command, "cmd.exe /k claude"); // 形态与 build_spawn_command_windows 对齐
    assert!(spec.env.iter().any(|(k, v)| k == "DISABLE_AUTOUPDATER" && v == "1"));
}
```

- [ ] **Step 2: 确认失败**（函数不存在）
- [ ] **Step 3: 实现**——① `SpawnSpec` 增 `pub env: Vec<(String, String)>`（`build_spawn_command_windows` 等构造点补 `env: Vec::new()`，行为零变化）；② `spawn_terminal` 的 Windows 分支对 `Command` 追加 `.envs(spec.env.iter().map(|(k,v)|(k,v)))`，macOS 分支在 AppleScript 命令前缀 `env K=V `（Mac 探测 M2 实证形态）；③ 新增：

```rust
/// 新建会话起窗规格：裸工具命令 + DISABLE_AUTOUPDATER=1（spec §4.2 环境红线，
/// S2 实测 conhost/WT 双宿主透传）。宿主决策与 resume 同源（WT 在场优先）。
pub fn build_create_spawn_spec(wt: Option<&str>, cwd: &str, tool: &str) -> SpawnSpec {
    let mut spec = build_spawn_command_windows(wt, cwd, tool);
    spec.env = vec![("DISABLE_AUTOUPDATER".to_string(), "1".to_string())];
    spec
}
```

（`cwd` 预检：resume 的既有 cwd 校验保留；本函数不再额外建目录——建目录归状态机校验段。）
- [ ] **Step 4: 测试过 → 门禁 → Commit** `feat(create): SpawnSpec 环境扩展 + 新建会话起窗规格（DISABLE_AUTOUPDATER）`

### Task C4: 进程锚定

**Files:** Modify `src-tauri/src/inject/create.rs`（本任务建文件，先只放本函数）

- [ ] **Step 1: 写失败测试**（纯函数部分：候选过滤）

```rust
#[test]
fn tui_candidate_filter_matches_cwd_and_name() {
    // normalize_cwd_for_match 同源归一（monitor::cwd），大小写/分隔符双态
    assert!(is_tui_candidate("claude", &normalize("E:\\proj\\demo"), "claude.exe", &normalize("e:/proj/demo")));
    assert!(is_tui_candidate("codex", &normalize("E:\\p"), "node.exe", &normalize("E:\\p")));
    assert!(!is_tui_candidate("claude", &normalize("E:\\other"), "claude.exe", &normalize("E:\\p")));
    assert!(!is_tui_candidate("kimi", &normalize("E:\\p"), "cmd.exe", &normalize("E:\\p"))); // 中间壳不算
}
```

- [ ] **Step 2: 确认失败 → 实现**：

```rust
/// TUI 进程名集合（与 m9r_e2e launch_cli 同口径：原生 exe / node / bun；
/// kimi 独立 exe）。中间壳（cmd/conhost/powershell）不算。
fn tui_names(tool: &str) -> Vec<String> {
    let mut v = vec![format!("{tool}.exe"), "node.exe".into(), "bun.exe".into()];
    if tool == "kimi" { v.insert(0, "kimi.exe".into()); }
    v
}
fn is_tui_candidate(tool: &str, proc_cwd: &str, proc_name: &str, want_cwd: &str) -> bool {
    proc_cwd == want_cwd && tui_names(tool).iter().any(|n| n.eq_ignore_ascii_case(proc_name))
}
/// 起窗后轮询发现 TUI pid（sysinfo cwd+进程名匹配；超时 Err 带中文回执）。
/// 生产扫描口径与 resume::macos_effect_probe 同源（RefreshKind with_cmd+with_cwd Always）。
pub fn find_tui_pid(tool: &str, dir: &Path, timeout: Duration) -> Result<u32, String> { /* sysinfo 轮询 1.2s 步距，is_tui_candidate 命中即返 */ }
```

（`find_tui_pid` 体内为直白的 sysinfo 轮询循环，参照 `resume.rs` 既有刷新口径写；纯函数 `is_tui_candidate` 承载判据，测试锁它。）
- [ ] **Step 3: 测试过 → 门禁 → Commit** `feat(create): TUI 进程锚定（cwd+进程名，中间壳排除）`

### Task C5: 创建状态机内核

**Files:** Modify `src-tauri/src/inject/create.rs`；Modify `src-tauri/src/inject/mod.rs`（如需 re-export）

- [ ] **Step 1: 写失败测试**（fake 缝驱动；核心场景全覆盖）：

```rust
/// 缝集合（测试注入 fake；生产由 C6 从 RemoteState 装配）
struct CreateDeps<'a> {
    screen: &'a dyn Fn(u32) -> Option<Vec<String>>,          // 屏读（已小写化由内核做）
    send_key: &'a dyn Fn(u32, &str) -> Result<(), String>,    // 键注入
    send_text: &'a dyn Fn(u32, &str) -> Result<(), String>,   // 首句注入（含提交回车）
    now_ms: &'a dyn Fn() -> u64,
}

#[test]
fn claude_trust_requires_down_then_enter() {
    // 屏读序列：先信任框（含危险默认行）→ 处置后 idle
    let screens = vec![
        vec!["Quick safety check: Is this a project you created or one you trust?".into(), "❯ No, exit".into()],
        vec!["(same)".into()],
        vec!["manual mode on · ? for shortcuts".into()],
    ];
    let mut keys = Vec::new();
    let deps = deps_recording(&screens, &mut keys);
    let out = run_pipeline(&deps, &Params { tool: "claude", /* … */ ..Default::default() });
    assert_eq!(out.status, CreateStatus::WaitingMaterialize);
    assert_eq!(keys, vec!["down", "enter"]); // 红线：直按 enter=退出，测试锁死
}
#[test]
fn codex_update_first_then_trust_with_char2() {
    let screens = vec![
        vec!["Update available", "1. Update now (runs npm install -g @openai/codex)", "2. Skip"].map(String::from).to_vec(),
        vec!["Do you trust the contents of this directory?", "1. Trust and continue"],
        vec!["Ask Codex to do anything".into()],
    ];
    let mut keys = Vec::new();
    let deps = deps_recording(&screens, &mut keys);
    let out = run_pipeline(&deps, &Params { tool: "codex", ..Default::default() });
    assert_eq!(keys, vec!["2", "enter"]); // '2'=Skip（esc 无效、enter 禁用，锁死）
    assert_eq!(out.status, CreateStatus::WaitingMaterialize);
}
#[test]
fn unrecognized_screen_fails_with_evidence() { /* 连续 N 轮无锚 → failed(unrecognized_screen)，不发任何键 */ }
#[test]
fn onboard_screen_fails_with_specific_message() { /* claude 403 墙 → failed(needs_human_signin/login_wall) 专属文案 */ }
#[test]
fn idle_gate_blocks_first_message_until_anchor() { /* 未见 idle 锚前 send_text 不被调用 */ }
```

- [ ] **Step 2: 确认失败 → 实现内核**：

```rust
pub enum CreateStatus { OpeningTerminal, DialogHandling, InjectingFirst, WaitingMaterialize,
    Done { session_id: String }, Failed { phase: &'static str, code: String, message: String } }
pub struct Params { pub tool: String, pub dir: PathBuf, pub first_message: String,
    pub device_id: String, pub device_name: String }

/// 弹窗处置键序（探测定案表 §4/§5——账本识别场景，键序是代码常量：
/// 账本管「认屏」，键序属引擎域，两者分离与 approve 映射同构）
fn disposal_keys(tool: &str, scenario: &str) -> Vec<&'static str> {
    match (tool, scenario) {
        ("claude", s) if s == scenario::CREATE_TRUST => vec!["down", "enter"],      // 危险默认
        ("codex", s) if s == s_CREATE_UPDATE => vec!["2"],                           // 字符键，域内已支持
        ("codex", _) | ("kimi", _) => vec!["enter"],                                 // 默认即信任
        _ => vec!["enter"],
    }
}
/// 状态机主循环（纯内核 + 缝）：屏读 → anchor_ledger::detect 逐场景 →
/// idle 锚（CREATE_IDLE/PRESENT）命中才 send_text（compose_injection 已在
/// 调用层完成签名）→ 物化交 C6 的轮询层。单阶段 30s 窗、2s 步距（spec §4.4）。
pub fn run_pipeline(deps: &CreateDeps, p: &Params) -> CreateOutcome { /* 按上述语义实现，
   每次处置动作返回 (scenario, keys) 供调用层逐条审计 */ }
```

  实现要点（写进代码注释）：① 屏读 `None`（读失败）计一次「未识别」，连续 15 轮（30s）才 failed；② `CREATE_ONBOARD` 命中即立即 failed（专属码 `login_wall` / `network_wall`，按 tool 分）；③ claude 更新横幅（CREATE_UPDATE/PRESENT）**不处置**、不阻断 idle 判定（探测定案：非阻塞）；④ 每次发键前重读一次屏确认场景仍在场（防处置竞态，与 question.rs 阶段机同款纪律）。
- [ ] **Step 3: 测试过 → 门禁 → Commit** `feat(create): 创建状态机内核（锚点驱动弹窗处置+idle门+首句）`

### Task C6: API 三端点 + 任务注册表 + 审计

**Files:** Modify `src-tauri/src/remote/server.rs`（RemoteState 增缝）、`src-tauri/src/remote/api.rs`（端点）、`src-tauri/src/remote/mod.rs`（STATE 装配）、`src-tauri/src/remote/server.rs` 路由表

- [ ] **Step 1: 写失败端点测试**（沿用 server.rs 既有内存库 + fake 注入器测试模式）：

```rust
#[test]
fn session_create_validates_and_409_singleflight() {
    // ① 路径非法 → 400 + reasonCode=non_ascii_path
    // ② 合法请求 → 200 {taskId}；任务在飞期间第二个请求 → 409
    // ③ status 查询 → phase 流转 JSON；未知 taskId → 404
    // ④ 审计：create 行落库（action=create）；弹窗处置行（action=dialog，
    //    summary=场景+键序）；首句行（action=send，设备=发起设备）
    // ⑤ create-projects：archive_source fake 返回一条 7 天内 + 一条 8 天前 +
    //    一条非 ASCII → 响应只含前者（7 天窗 + ASCII 过滤 + 快照 ∪ 去重）
}
```

- [ ] **Step 2: 确认失败 → 实现**：
  - `RemoteState` 增 `pub create_tasks: Arc<Mutex<HashMap<u64, CreateTaskShared>>>`（id 自增；`CreateTaskShared { phase, detail, session_id, spawned_pid }`，Mutex 短临界区只读写快照）；装配处初始化空表。
  - `POST /session-create`：① 参数校验（tool ∈ 四家且在 enabledTools ∩ 安装探测；`create_path::validate`；多实例信号计算）；② `create_tasks` 单飞判定 → 409；③ `create_dir_all`（失败 400 `mkdir_failed`）；④ `tokio::spawn_blocking` 跑管线（spawn 用 `st.resume_spawner` + `build_create_spawn_spec`；`find_tui_pid`；`run_pipeline` 的缝从 `st.screen_probe`/`st.injector` 装配；首句文本 = `normalize::compose_injection(&device_name, first_message)`，spec = `families::family_for(tool)`，经 `injector.locate_and_inject_spec`）；⑤ 物化轮询：`discover_new_session(tool, dir, since)` 找 sid（claude=项目 slug 目录 mtime>since 的最新 jsonl / codex=今日 rollout 且 cwd 匹配 / kimi=sessions 下 wd_* 新目录 / opencode=拷三件套查 session 表 by directory）→ `st.confirm_probe(tool, sid, stamp)` 命中 → Done；⑥ 全程逐段审计（`inject::audit_write`，action：`create`/`dialog`/`send`，session_id 用 ""（物化前）→ sid（物化后），失败行 result=`failed:<code>`）。
  - `GET /session-create/status`：读注册表快照 → JSON；404 未知/已重启。
  - `GET /create-projects?days=`：`(st.session_source)()` 的 project_path ∪ `(st.archive_source)()` 按 last_seen 过 N 天（时间解析按 archive.rs 实际写入格式），双方都过 `create_path::validate` 的 ASCII 检查，去重排序。
- [ ] **Step 3: 测试过 → 六门禁局部（fmt/clippy/test）→ Commit** `feat(create): session-create 三端点+内存任务态+审计三类行`

### Task C7: 配对不确定信号暴露

**Files:** Modify `src-tauri/src/commands/session.rs`（`running_projects_from_processes` 升 `pub(crate)`）、`src-tauri/src/remote/api.rs`

- [ ] **Step 1: 写失败测试**：sessions 响应含 `pairingAmbiguous: true`（fake session_source 两条同工具同项目 + running_processes 计数 ≥2）；session-send 回执含 `pairingHint: true`（同态，**不拦截**，delivered 照常）。
- [ ] **Step 2: 实现**：api.rs 增内部函数 `pairing_ambiguous_set() -> HashSet<(String,String)>`（sysinfo 快照 + 复用升可见性的 `running_projects_from_processes` 计数 ≥2，工具+项目小写归一——与桌面门同判据同口径）；sessions 序列化前打标；session_send 成功回执 JSON 追加 `pairingHint`。桌面门（commands/session.rs require_evidence）**不动**——只共享判据函数。
- [ ] **Step 3: 测试过 → 门禁 → Commit** `feat(create): 配对不确定信号暴露（sessions 标记+投递回执提示，不拦截）`

### Task C8: 实机 E2E + 批次一门禁终跑

**Files:** Create `src-tauri/tests/create_e2e.rs`

- [ ] **Step 1: 写 E2E**（`#[ignore]` 全部，模式复用 m9r_e2e 的 Ev/清场/纪律；不依赖 MAM dev 运行——直接调 `create::run_pipeline` 生产内核 + 真屏读/真注入缝）：

```rust
/// 四家矩阵：全新 temp 目录 → build_create_spawn_spec 起窗 → find_tui_pid →
/// run_pipeline（真缝）→ 物化轮询 → 断言 stamp 命中 + 会话文件首条用户消息
/// 含 "hi [mobile <设备名>]"（compose 签名）→ taskkill 清场。
/// 附加用例：① claude 信任框路径断言键序 down+enter（audit 留痕核对）；
/// ② 未识别界面注入（屏读喂假行）→ failed(unrecognized_screen) 零发键。
#[test] #[ignore = "实机显式跑"] fn e2e_create_matrix_four_tools() { /* … */ }
```

- [ ] **Step 2: 实机跑** `cargo test --test create_e2e -- --ignored --nocapture --test-threads=1`，四家全过（操作者 = 主线/用户在场机器）；把实跑台账写进本文件头注释（m9r_e2e 同款）。
- [ ] **Step 3: 六门禁终跑**（fmt/clippy/lib test/main/dao/linker/preset_v2 + pnpm check/test/build:mobile 零回归）→ Commit `test(create): 四家新建全链实机 E2E + 台账`
- [ ] **Step 4: 【批次一停点】**——全部本地提交不 push，向主线交付简报（任务×commit×门禁表 + E2E 实跑数字 + 待评审自裁决清单），**等主线评审通过后放行批次二**。

---

# 批次二（前端，C9–C12；批次一评审通过后执行）

### Task C9: 移动端 API 客户端

**Files:** Modify `src/mobile/api.ts`
- [ ] 写类型与三方法：`fetchCreateProjects(days)`、`createSession({tool, projectPath, firstMessage?})` → `{taskId}`、`fetchCreateStatus(taskId)`（phase/detail/sessionId）；vitest 单测（mock fetch：409/404/400 reasonCode 分支）→ `pnpm test` 过 → Commit。

### Task C10: 新建会话表单

**Files:** Create `src/mobile/CreateSessionSheet.tsx`；Modify `src/mobile/Board.tsx`（头部操作区加「+」入口，样式对齐既有 BookmarkBar/工具 chips 组件）、`src/mobile/mobile.css`（如需）
- [ ] 表单三步（工具四选——未装置灰带原因；目录——候选下拉（create-projects，显示最近活跃+工具集合）+ 手填切换（前端 ASCII/形态提示，服务端为权威）；首句——默认占位 `hi` 可改）；同项目同工具活跃会话时黄字（消费 sessions 快照的 `pairingAmbiguous` 同判据：同 tool+project 已有活跃卡）；vitest（组件渲染/校验分支）→ `pnpm check && pnpm test` → Commit。

### Task C11: 进度态与自动跳转

**Files:** Modify `src/mobile/CreateSessionSheet.tsx`（或伴生 `CreateProgress.tsx`）、`src/mobile/Board.tsx`
- [ ] 提交后进入进度视图：phase → 中文文案（校验/开终端/处置弹窗/注入首句/等待上板）+ 2s 轮询 status；`done.sessionId` → 等 SSE/轮询见新卡后跳 `SessionDetail`（复用既有导航路径）；`failed` → 分阶段原因 + 重试按钮；404（重启丢任务）→ 引导重试文案。vitest（状态机映射/轮询停止条件）→ 门禁 → Commit。

### Task C12: 手工验收清单 + 批次二门禁终跑

**Files:** Create `docs/release-notes/create-acceptance-checklist.md`
- [ ] 清单覆盖 spec §7 全部人工项（四家全链含信任框/更新提示路径/未识别界面报错/多实例黄字/路径边界含 non_ascii+UNC+黑名单+盘不存在/审计三类行逐条可查）+ 用户真机手机走查项；
- [ ] `pnpm check && pnpm test && pnpm build:mobile` + 后端门禁复跑零回归 → Commit → **批次二停点：主线评审 → 用户真机验收**。

---

## Self-Review（计划自审记录）

- **Spec 覆盖**：§2 表单/黄字/异步体验→C6/C7/C10/C11；§3 三端点+单飞+审计→C6；§4.1-4.8→C2/C3/C4/C5/C6（4.7 失败不清场=状态机只回执不杀进程，E2E 显式清场为测试纪律，两者不冲突）；§5→C7；§6 宪法→C0；§7→C8/C12。§4.8 codex hooks 信任提示：**挂 C6**（Done 回执 detail 附信号健康度提示，复用现有 T5 状态读取——实现时若无现成查询口，读 `hooks_registered_codex` 设置 + trust 状态 KV，一行判断，不新建机制）。
- **占位扫描**：`find_tui_pid`/`run_pipeline`/E2E 体内为「按既定语义直写」型骨架（判据、缝、断言全部给出，属实现填写而非 TBD）；其余步骤均有完整代码或逐字内容。
- **类型一致性**：`CreateStatus`/`Params`/`CreateDeps`/`PathReject.code` 谓词在 C2/C5/C6 间已对齐（`non_ascii_path` 等原因码与 spec §2 逐一对应）。
