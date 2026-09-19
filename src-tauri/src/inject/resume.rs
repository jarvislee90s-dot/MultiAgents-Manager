//! R5 一键 resume 窗口（M6R–M9R 批次 Task 11）：
//! 手机或桌面点一下 → 本机自动打开终端 + 进入项目目录 + 恢复会话 + 置前聚焦，
//! 用户零额外操作。终端选择：Windows 优先 Windows Terminal、未装回退 conhost；
//! macOS 优先 iTerm2、次选 Terminal.app（复用 window::applescript::execute_applescript）。
//!
//! ## 结构（构造与执行分离，engine.rs 同款纪律）
//! - [`resume_command`]：命令表纯函数（Step 1 实测取证，**未查证/不存在的工具绝不入表**
//!   ——前端按钮禁用 + 原因「该工具 resume 命令待查证」）；
//! - [`build_spawn_command_windows`]：Windows spawn 计划纯构造（wt / conhost 两形态），
//!   跨平台 pub 可测，不真 spawn；
//! - [`iterm_open_window_script`] / [`terminal_open_script`]：macOS AppleScript 纯构造
//!   （跨平台可测；实机开窗验证归 Mac 回传清单）；
//! - [`open_session_terminal_with`]：核心入口（**接受 spawner 缝**，远端端点测试注入
//!   记录型假 spawner，零真开窗）；[`open_session_terminal`] = 生产装配（真 spawn）。
//!
//! ## 审计
//! 远端端点（POST /m/api/v1/session-open）在 spawn 出手时写审计 action=`open`
//! （W5 词表 send|queue|flush|jump|retract|approve|reject|fail|key|open——Task 7
//! 的「open 预留：Task 11」标注兑现）。桌面 Tauri 命令路径不写审计：audit_write
//! 需要设备身份（device_id/device_name），本机点击无设备身份，审计口径 = 远端设备
//! 动作留痕（与 session-send 等既有写端点一致）。
//!
//! ## 注入面说明（安全）
//! resume 命令里的会话 id 一律取自**本机会话快照命中项**（远端只送 session_id 用于
//! 查找，未命中即 404 no_session），不回显请求原文——`cmd /k` 对 `&`/引号等元字符
//! 的解析面不接触远端输入。

use crate::session::Session;

/// spawner 缝类型（对齐 remote::server::ViaHostsSource 的 type_complexity 收敛别名）：
/// 消费 [`SpawnSpec`] 完成「开终端窗口」副作用。生产 = [`spawn_terminal`]（真开窗）；
/// 测试 = 记录型假体（零真开窗）。
pub type SpawnFn = dyn Fn(&SpawnSpec) -> Result<(), String> + Send + Sync;

/// 终端 spawn 计划（纯构造产物，字段即测试断言面）。
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnSpec {
    /// 可执行程序（"wt" / "conhost.exe"）
    pub program: String,
    /// 参数向量（resume 命令整体作为一个参数由 CreateProcess 引号传递，不再二次分词）
    pub args: Vec<String>,
    /// Windows CREATE_NEW_CONSOLE 标记：conhost 回退必须自带（0x10）才开新控制台窗；
    /// wt 自开新标签无此需求
    pub new_console: bool,
}

/// resume 命令表（Step 1 实测取证：Windows 本机 `--help` 实跑，2026-09-19）。
/// **未查证/不存在的工具绝不入表**（严格档同理），各条注明查证版本与 --help 原文：
/// - claude 2.1.251：`-r, --resume [value]  Resume a conversation by session ID, or
///   open interactive picker with optional search term`；
/// - codex-cli 0.154.0：子命令 `codex resume [OPTIONS] [SESSION_ID] [PROMPT]`
///   （"Resume a previous interactive session"）。**工作区上下文限定**：resume 默认按
///   cwd 过滤候选（`--all` 才解除），本实现 spawn 先 cd 项目目录即满足；zcode 的
///   D6 在册工作区限定同口径——但 zcode CLI 本机未安装，如实记 None 不入表；
/// - kimi 2.0.0：`-S, --session [id]  Resume a session. With ID: resume that session.
///   Without ID: interactively pick.`；
/// - opencode 1.18.31：`-s, --session  session id to continue`（SQLite 类工具，
///   扫描契约豁免 L2/L3，与本表无关）；
/// - 未安装/不适用（Step 1 如实记录，均 None）：zcode（CLI 未安装）、openclaw
///   （CLI 未安装）、workbuddy（桌面 APP 形态无 CLI）、dsh（web 宿主）。
const RESUME_TABLE: &[(&str, &str)] = &[
    ("claude", "claude --resume {id}"),
    ("codex", "codex resume {id}"),
    ("kimi", "kimi --session {id}"),
    ("opencode", "opencode --session {id}"),
];

/// 按工具查 resume 命令（`{id}` 占位替换为会话 id）。未入表工具 → None：
/// 端点映射 404 no_resume_command，前端按钮禁用 + 原因「该工具 resume 命令待查证」。
pub fn resume_command(tool: &str, session_id: &str) -> Option<String> {
    RESUME_TABLE
        .iter()
        .find(|(t, _)| *t == tool)
        .map(|(_, tpl)| tpl.replace("{id}", session_id))
}

/// Windows spawn 计划纯构造（**不真 spawn**，跨平台可测）：
/// - 有 wt：`wt -d <cwd> cmd /k <resume>`（wt 自开新标签并落在 cwd）；
/// - 无 wt：`conhost.exe cmd /k <resume>` + CREATE_NEW_CONSOLE（全新控制台窗）。
pub fn build_spawn_command_windows(has_wt: bool, cwd: &str, resume: &str) -> SpawnSpec {
    if has_wt {
        SpawnSpec {
            program: "wt".to_string(),
            args: vec![
                "-d".to_string(),
                cwd.to_string(),
                "cmd".to_string(),
                "/k".to_string(),
                resume.to_string(),
            ],
            new_console: false,
        }
    } else {
        SpawnSpec {
            program: "conhost.exe".to_string(),
            args: vec!["cmd".to_string(), "/k".to_string(), resume.to_string()],
            new_console: true,
        }
    }
}

/// `where wt` 探测 Windows Terminal（进程级缓存 OnceLock，approve.rs VERSION_CACHE
/// 先例：首调一次探测，结果含 false 一并入缓存，不重复刷进程）。`where.exe` 是
/// System32 上的真实可执行文件（非 cmd 内建），裸名直 spawn 即可。
#[cfg(windows)]
fn windows_terminal_available() -> bool {
    static WT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WT.get_or_init(|| {
        std::process::Command::new("where")
            .arg("wt")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    })
}

/// 非 Windows 平台无 wt 语义（构造层恒 false；该平台走 AppleScript 分支）。
#[cfg(not(windows))]
fn windows_terminal_available() -> bool {
    false
}

/// shell 单引号安全包装（macOS cd 参数用）：`'…'` 形态，内嵌单引号按 POSIX `'\''`
/// 序列转义——目录名带撇号/空格时裸拼会截断 cd 参数。
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 组装 macOS 命令载荷：`cd '<cwd>' && <resume>`（cwd 经 [`shell_single_quote`]，
/// 整条再经 applescript_escape 进 AppleScript 双引号字符串字面量）。
fn macos_command_payload(cwd: &str, resume: &str) -> String {
    crate::inject::engine::applescript_escape(&format!(
        "cd {} && {}",
        shell_single_quote(cwd),
        resume
    ))
}

/// 构造 iTerm2 新开窗脚本（macOS 优先通道）：`create window with default profile
/// command "cd '<cwd>' && <resume>"` + activate 置前聚焦。载荷转义复用 engine.rs
/// [`crate::inject::engine::applescript_escape`]（双引号形态：反斜杠/双引号/裸换行；
/// shell 单引号在 AppleScript 双引号字符串里是字面量，cwd 的单引号包裹安全穿过）。
pub fn iterm_open_window_script(cwd: &str, resume: &str) -> String {
    format!(
        r#"tell application "iTerm2"
	activate
	create window with default profile command "{payload}"
	return "opened"
end tell
"#,
        payload = macos_command_payload(cwd, resume)
    )
}

/// 构造 Terminal.app 执行脚本（macOS 次选通道）：`do script "cd '<cwd>' && <resume>"`
/// （do script 自带回车）+ activate 置前聚焦。转义同 [`iterm_open_window_script`]。
pub fn terminal_open_script(cwd: &str, resume: &str) -> String {
    format!(
        r#"tell application "Terminal"
	do script "{payload}"
	activate
	return "opened"
end tell
"#,
        payload = macos_command_payload(cwd, resume)
    )
}

/// macOS 执行层：iTerm2 优先、Terminal.app 次选（复用 execute_applescript），
/// 脚本自带 activate 置前聚焦。实机验证归 Mac 回传清单。
#[cfg(target_os = "macos")]
fn open_macos_terminal(cwd: &str, resume: &str) -> Result<(), String> {
    if crate::window::applescript::execute_applescript(&iterm_open_window_script(cwd, resume))
        .is_ok()
    {
        return Ok(());
    }
    crate::window::applescript::execute_applescript(&terminal_open_script(cwd, resume))
}

/// 生产 spawner（RemoteState 生产装配 / Tauri 命令共用）：spawn fire-and-forget
/// （不 wait——终端窗口生命周期独立于 MAM 进程）；conhost 路径加
/// CREATE_NEW_CONSOLE(0x10)——父进程退出后新控制台照常存活。
pub fn spawn_terminal(spec: &SpawnSpec) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new(&spec.program);
        cmd.args(&spec.args);
        if spec.new_console {
            // CREATE_NEW_CONSOLE：为 conhost 回退开全新控制台窗
            cmd.creation_flags(0x0000_0010);
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("终端启动失败（{}）：{e}", spec.program))
    }
    #[cfg(not(windows))]
    {
        Err(format!("当前平台不支持终端 spawn（{}）", spec.program))
    }
}

/// 核心入口（spawner 缝版本）：命令表解析 → cwd 预检 → 按平台构造并出手。
/// 错误契约：哨兵串 `"no_resume_command"` / `"no_cwd"`（远端端点映射 404；
/// 前端按钮同因禁用），其余为人类可读失败文案。
pub fn open_session_terminal_with(session: &Session, spawner: &SpawnFn) -> Result<(), String> {
    let tool = session.agent_type.tool_id();
    // 未查证/未入表工具：出手前拦截（spawner 不被调用）
    let Some(resume) = resume_command(tool, &session.id) else {
        return Err("no_resume_command".to_string());
    };
    let cwd = session.project_path.trim();
    if cwd.is_empty() {
        return Err("no_cwd".to_string());
    }
    match std::env::consts::OS {
        "windows" => {
            let spec = build_spawn_command_windows(windows_terminal_available(), cwd, &resume);
            spawner(&spec)
        }
        // macOS 构造层：执行层随 cfg 装配（本平台不编译 AppleScript 执行层时恒 Err）
        #[cfg(target_os = "macos")]
        "macos" => open_macos_terminal(cwd, &resume),
        #[cfg(not(target_os = "macos"))]
        "macos" => Err("macOS 执行层仅在 macOS 构建装配".to_string()),
        _ => Err("当前平台不支持一键恢复会话".to_string()),
    }
}

/// 生产装配（Tauri 命令路径）：真 spawn 终端。
pub fn open_session_terminal(session: &Session) -> Result<(), String> {
    open_session_terminal_with(session, &spawn_terminal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小会话夹具（字段形状对齐 remote/server.rs inj_sess 先例）
    fn sess(agent_type: crate::session::AgentType, cwd: &str) -> Session {
        Session {
            id: "abc".into(),
            agent_type,
            project_name: "proj".into(),
            project_path: cwd.into(),
            title: None,
            git_branch: None,
            github_url: None,
            status: crate::session::SessionStatus::Waiting,
            last_message: None,
            last_message_role: None,
            last_activity_at: "2026-09-19T00:00:00Z".into(),
            pid: 7,
            cpu_usage: 0.0,
            active_subagent_count: 0,
            form: crate::session::ProcessForm::Cli,
            jump_supported: false,
            unread: false,
        }
    }

    /// Step 2 失败测试 ①：命令表按 Step 1 实测断言——四工具命中，未知/未查证工具
    /// 一律 None（绝不入表）
    #[test]
    fn resume_command_table() {
        assert_eq!(
            resume_command("claude", "abc").as_deref(),
            Some("claude --resume abc")
        );
        assert_eq!(
            resume_command("codex", "abc").as_deref(),
            Some("codex resume abc"),
            "codex 按 Step 1 实测（resume 子命令）"
        );
        assert_eq!(
            resume_command("kimi", "abc").as_deref(),
            Some("kimi --session abc")
        );
        assert_eq!(
            resume_command("opencode", "abc").as_deref(),
            Some("opencode --session abc")
        );
        // 未安装 / 不适用（Step 1 如实记录）+ 未知工具：None → 按钮禁用 + 待查证
        for t in ["zcode", "openclaw", "workbuddy", "dsh", "unknown-tool", ""] {
            assert_eq!(resume_command(t, "abc"), None, "{t} 不得有 resume 映射");
        }
    }

    /// Step 2 失败测试 ②：Windows spawn 计划纯构造——有 wt → wt -d cwd cmd /k；
    /// 无 wt → conhost + CREATE_NEW_CONSOLE（纯构造断言，不真 spawn）。
    /// 函数与测试同名：测试体内用 `super::` 显式路径防 glob 导入被本测试名遮蔽
    #[test]
    fn build_spawn_command_windows() {
        let wt = super::build_spawn_command_windows(true, r"E:\proj", "claude --resume abc");
        assert_eq!(wt.program, "wt");
        assert_eq!(
            wt.args,
            vec![
                "-d".to_string(),
                r"E:\proj".to_string(),
                "cmd".to_string(),
                "/k".to_string(),
                "claude --resume abc".to_string(),
            ],
            "有 wt → wt -d <cwd> cmd /k <resume>"
        );
        assert!(!wt.new_console, "wt 自开新标签，无需 CREATE_NEW_CONSOLE");

        let conhost = super::build_spawn_command_windows(false, r"E:\proj", "claude --resume abc");
        assert_eq!(conhost.program, "conhost.exe");
        assert_eq!(
            conhost.args,
            vec![
                "cmd".to_string(),
                "/k".to_string(),
                "claude --resume abc".to_string(),
            ],
            "无 wt → conhost.exe cmd /k <resume>"
        );
        assert!(
            conhost.new_console,
            "conhost 回退必须 CREATE_NEW_CONSOLE 开新窗"
        );
    }

    /// macOS 构造层（跨平台可测）：cd '<cwd>' && <resume> 进脚本 + activate 置前；
    /// 转义路径——反斜杠/双引号经 applescript_escape，内嵌单引号经 POSIX '\'' 转义
    #[test]
    fn macos_scripts_carry_cd_focus_and_escaping() {
        let s = iterm_open_window_script("/tmp/proj", "claude --resume abc");
        assert!(s.contains("create window with default profile command"));
        assert!(s.contains(r#"cd '/tmp/proj' && claude --resume abc"#));
        assert!(s.contains("activate"), "完成后置前聚焦");

        let t = terminal_open_script("/tmp/proj", "claude --resume abc");
        assert!(t.contains(r#"do script "cd '/tmp/proj' && claude --resume abc""#));
        assert!(t.contains("activate"));

        // 双引号/反斜杠经 applescript_escape（C:\wei"rd → C:\\wei\"rd），字面量不破
        let e = terminal_open_script(r#"C:\wei"rd"#, "claude --resume abc");
        assert!(e.contains(r#"cd 'C:\\wei\"rd'"#));
        // 内嵌单引号：先 shell '\'' 转义、后 applescript_escape 把该反斜杠再翻倍
        // （\\）——AppleScript 字面量解一转义后 shell 收到的仍是 `'\''`，语义正确
        let q = iterm_open_window_script("/tmp/it's", "claude --resume abc");
        assert!(q.contains(r#"cd '/tmp/it'\\''s'"#));
    }

    /// 核心错误契约：无映射 / 无 cwd 哨兵在出手前返回（spawner 不得被调用）
    #[test]
    fn core_open_sentinels_before_spawn() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let spawner = {
            let calls = calls.clone();
            move |_spec: &SpawnSpec| {
                *calls.lock().unwrap() += 1;
                Ok(())
            }
        };
        // 无映射工具（workbuddy 未入表）
        let no_map = sess(crate::session::AgentType::WorkBuddy, "/tmp/p");
        assert_eq!(
            open_session_terminal_with(&no_map, &spawner).unwrap_err(),
            "no_resume_command"
        );
        // 无 cwd（空白按无目录处理）
        let no_cwd = sess(crate::session::AgentType::Claude, "   ");
        assert_eq!(
            open_session_terminal_with(&no_cwd, &spawner).unwrap_err(),
            "no_cwd"
        );
        assert_eq!(*calls.lock().unwrap(), 0, "哨兵错误必须在出手前返回");
    }

    /// 核心出手路径（Windows）：Ok 且 spawner 恰被调用一次，携带命令表产物
    /// （探测 wt 真跑一次 `where`，OnceLock 缓存——approve.rs 实测探测先例同口径）
    #[cfg(windows)]
    #[test]
    fn core_open_dispatches_spawn_on_windows() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let spawner = {
            let calls = calls.clone();
            move |spec: &SpawnSpec| {
                calls.lock().unwrap().push(spec.clone());
                Ok(())
            }
        };
        let s = sess(crate::session::AgentType::Claude, "/tmp/proj");
        assert_eq!(open_session_terminal_with(&s, &spawner), Ok(()));
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "spawner 恰被调用一次");
        assert!(
            recorded[0].args.iter().any(|a| a == "claude --resume abc"),
            "spawn 计划必须携带命令表产物：{:?}",
            recorded[0]
        );
    }
}
