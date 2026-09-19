//! R5 一键 resume 窗口（M6R–M9R 批次 Task 11）：
//! 手机或桌面点一下 → 本机自动打开终端 + 进入项目目录 + 恢复会话 + 置前聚焦，
//! 用户零额外操作。终端选择：Windows 优先 Windows Terminal、未装回退 conhost；
//! macOS 优先 iTerm2、次选 Terminal.app（osascript **等待执行 + 失败回执**——M2
//! 修法，见 [`run_osascript_wait`] / [`classify_resume_error`] / [`spawn_terminal`]）。
//!
//! ## 结构（构造与执行分离，engine.rs 同款纪律；评审 I2：macOS 同样入缝）
//! - [`resume_command`]：命令表纯函数（Step 1 实测取证，**未查证/不存在的工具绝不入表**
//!   ——前端按钮禁用 + 原因「该工具 resume 命令待查证」）；
//! - [`SpawnSpec`]：spawn 计划载荷（枚举两变体——Windows CreateProcess / macOS
//!   AppleScript），构造与执行分离，跨平台 pub 可测不真 spawn；
//! - [`build_spawn_command_windows`]：Windows 变体纯构造（wt / conhost 两形态）；
//! - [`iterm_open_window_script`] / [`terminal_open_script`]：macOS AppleScript 纯构造
//!   （构造只是字符串拼接，**Windows 上即可测**，实机开窗验证归 Mac 回传清单）；
//! - [`open_macos_with`]：macOS 双通道缝出手顺序（iTerm2 优先、Terminal.app 次选），
//!   跨平台可测；
//! - [`open_session_terminal_with`]：核心入口（**接受 spawner 缝**，远端端点测试注入
//!   记录型假 spawner，零真开窗——两平台皆然）；[`open_session_terminal`] =
//!   生产装配（真 spawn / 真 AppleScript）。
//!
//! ## cwd 落点机制（评审 I1 修正）
//! conhost 回退无 `-d` 等价参数，cwd 继承靠生产 spawner 的 `Command::current_dir`：
//! conhost → cmd 逐级继承进程工作目录，落在项目目录（免 `cd /d` 复合串的引号地狱）；
//! wt 分支另带 `-d <cwd>`（双保险），`current_dir` 同设无害。codex resume 默认按
//! cwd 过滤候选（`--all` 才解除）——spawn 落在项目目录即满足该工作区上下文。
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

/// 终端 spawn 计划载荷（枚举两变体；评审 I2：macOS 同样走缝，「测试零真开窗」
/// 跨平台为真，且 AppleScript 构造获得 Windows 可测性——对齐 R6 精神）。
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnSpec {
    /// Windows：CreateProcess 开新窗/新标签
    Windows {
        /// 可执行程序（wt 探测命中的完整路径 / "conhost.exe"）
        program: String,
        /// 参数向量（resume 命令整体作为一个参数由 CreateProcess 引号传递，不再二次分词）
        args: Vec<String>,
        /// 进程工作目录（生产 spawner 经 `Command::current_dir` 消费——conhost 回退
        /// 无 `-d` 等价物，cwd 靠继承落在项目目录；wt 分支同设无害）
        cwd: String,
        /// CREATE_NEW_CONSOLE 标记：conhost 回退必须自带（0x10）才开新控制台窗；
        /// wt 自开新标签无此需求
        new_console: bool,
    },
    /// macOS：AppleScript 脚本整体（生产 = osascript 执行）
    MacosApplescript {
        /// 完整脚本（[`iterm_open_window_script`] / [`terminal_open_script`] 构造）
        script: String,
    },
}

/// resume 命令表（Step 1 实测取证：Windows 本机 `--help` 实跑，2026-09-19）。
/// **未查证/不存在的工具绝不入表**（严格档同理），各条注明查证版本与 --help 原文：
/// - claude 2.1.251：`-r, --resume [value]  Resume a conversation by session ID, or
///   open interactive picker with optional search term`；
/// - codex-cli 0.154.0：子命令 `codex resume [OPTIONS] [SESSION_ID] [PROMPT]`
///   （"Resume a previous interactive session"）。**工作区上下文限定**：resume 默认按
///   cwd 过滤候选（`--all` 才解除），本实现 spawn 以 current_dir 落在项目目录即满足；
///   zcode 的 D6 在册工作区限定同口径——但 zcode CLI 本机未安装，如实记 None 不入表；
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

/// Windows spawn 计划纯构造（**不真 spawn**，跨平台可测；评审 M1：入参为 wt 完整
/// 路径 Option 而非在场布尔——路径进 spec 由生产 spawner 直 spawn，绕开应用执行
/// 别名（App Execution Alias）停用/损坏场景）。两分支 `cwd` 字段同设——生产
/// spawner 经 current_dir 消费（见 [`spawn_terminal`]）：
/// - 有 wt：`<wt 路径> -d <cwd> cmd /k <resume>`（wt 自开新标签并落在 cwd）；
/// - 无 wt：`conhost.exe cmd /k <resume>` + CREATE_NEW_CONSOLE（全新控制台窗）。
pub fn build_spawn_command_windows(wt: Option<&str>, cwd: &str, resume: &str) -> SpawnSpec {
    match wt {
        Some(path) => SpawnSpec::Windows {
            program: path.to_string(),
            args: vec![
                "-d".to_string(),
                cwd.to_string(),
                "cmd".to_string(),
                "/k".to_string(),
                resume.to_string(),
            ],
            cwd: cwd.to_string(),
            new_console: false,
        },
        None => SpawnSpec::Windows {
            program: "conhost.exe".to_string(),
            args: vec!["cmd".to_string(), "/k".to_string(), resume.to_string()],
            cwd: cwd.to_string(),
            new_console: true,
        },
    }
}

/// `where wt` 探测 Windows Terminal（进程级缓存 OnceLock，approve.rs VERSION_CACHE
/// 先例：首调一次探测，结果含 None 一并入缓存，不重复刷进程）。缓存**首行完整
/// 路径**（评审 M1：生产直 spawn 该路径——`wt` 应用执行别名可能被停用/损坏，
/// where 命中的真实 exe 不受影响）。`where.exe` 是 System32 上的真实可执行文件
/// （非 cmd 内建），裸名直 spawn 即可。
#[cfg(windows)]
fn windows_terminal_path() -> Option<String> {
    static WT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    WT.get_or_init(|| {
        std::process::Command::new("where")
            .arg("wt")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .map(|l| l.trim().to_string())
            })
            .filter(|l| !l.is_empty())
    })
    .clone()
}

/// 非 Windows 平台无 wt 语义（构造层恒 None；该平台走 AppleScript 分支）。
#[cfg(not(windows))]
fn windows_terminal_path() -> Option<String> {
    None
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

// ==== M2（Mac 验收 D-5 根因①处置）：macOS spawn 回执化 ====
// 实机教训（mac-acceptance-report-55f37e7 §四-B）：MAM dev 二进制（无 bundle id）缺
// TCC 自动化授权时，osascript 以 -1743 失败，旧路径发射后不管 → stderr 被吞、audit
// 照记 open ok——账实背离。现 macOS 出手等待完成（wait_timeout 10s）+ stderr/退出码
// 捕获，失败经错误分类给可行动回执（200 failed + 审计 failed，端点既有 Err 臂承接）。

/// osascript 等待上限（秒）：osascript 下发脚本即返回，正常 <1s；10s 给 TCC 首次
/// 授权弹窗与冷启动留裕量，超时视为失败（kill + wait 收尸防僵尸，approve.rs 同口径）
const OSASCRIPT_WAIT_SECS: u64 = 10;

/// stderr 摘要截断上限（失败回执与审计 result 双消费防膨胀；normalize::summarize
/// 按 chars 计，中文多字节安全）
const RESUME_STDERR_SUMMARY_CHARS: usize = 120;

/// TCC -1743（errAEEventNotHandled / 自动化授权拒绝）的中文指引文案（用户裁决口径，
/// 固定长度有界——审计 result 直接承载）
pub const MACOS_TCC_GUIDANCE: &str =
    "macOS 自动化授权缺失：系统设置 > 隐私与安全性 > 自动化 中允许控制 Terminal/iTerm2 后重试（开发版首次需手动授权一次）";

/// osascript 失败分类（**纯函数跨平台可测**，M2 裁决：错误分类与执行层分离——
/// Windows 上即可单测，真实 osascript 路径只做编译验证）：
/// - stderr 含 `-1743` 字样或 osascript 标准错误「Not authorized to send Apple
///   events」（大小写不敏感）→ [`MACOS_TCC_GUIDANCE`] 指引文案；
/// - 其他 → stderr 摘要（[`RESUME_STDERR_SUMMARY_CHARS`] 截断防膨胀）；
/// - 空 stderr → 有界兜底文案（回执不悬空）。
pub fn classify_resume_error(stderr: &str) -> String {
    let tcc_hit = stderr.contains("-1743")
        || stderr
            .to_ascii_lowercase()
            .contains("not authorized to send apple events");
    if tcc_hit {
        MACOS_TCC_GUIDANCE.to_string()
    } else {
        let s = stderr.trim();
        if s.is_empty() {
            "osascript 执行失败（无 stderr 输出）".to_string()
        } else {
            crate::inject::normalize::summarize(s, RESUME_STDERR_SUMMARY_CHARS)
        }
    }
}

/// resume 专用 osascript 等待执行（macOS 生产路径；区别于聚焦层
/// window::applescript::execute_applescript——后者服务窗口聚焦/注入路径维持原语义
/// 不动，M2 只收窄 resume 出手）：等待子进程完成（wait_timeout 10s）+ stderr/退出码
/// 捕获，失败 Err 携带分类产物 → open_macos_with 双通道合并 → 端点既有 Err 臂 →
/// 200 failed 回执 + 审计 failed。
///
/// 管道口径（approve.rs probe_cli_version 同源论证）：osascript 输出远小于管道缓冲
/// （成功仅 `opened` 一行），先等后读无死锁风险；披露：输出超 ~64KB 管道缓冲的极端
/// 脚本会在 wait 时被写满阻塞而假性超时——结果有界（10s 杀掉）非死锁。stdin 置
/// null 防 osascript 等 stdin 白耗超时窗。不读 stdout 判「not found」：resume 脚本
/// `return "opened"`，无聚焦层的 tab 查找语义。
///
/// 函数体全为跨平台 API（Command/Stdio/wait_timeout），故**不 cfg 隔离**——Windows
/// 构建持续编译验证本函数（M2 裁决「cfg macos 构造层只做编译验证」的可执行化）；
/// 仅调用点（[`spawn_terminal`] 的 macOS 臂）cfg 分派，运行时 macOS 独占触达，
/// 非 macOS 构建下它不被调用故显式 allow(dead_code)。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn run_osascript_wait(script: &str) -> Result<(), String> {
    use std::io::Read;
    use wait_timeout::ChildExt;
    let mut child = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("osascript 启动失败：{e}"))?;
    match child.wait_timeout(std::time::Duration::from_secs(OSASCRIPT_WAIT_SECS)) {
        // 正常退出：先收退出码分诊成败，再排空 stderr（子进程已退出，缓冲安全读）
        Ok(Some(status)) => {
            let mut raw = Vec::new();
            if let Some(pipe) = child.stderr.as_mut() {
                let _ = pipe.read_to_end(&mut raw);
            }
            let stderr = String::from_utf8_lossy(&raw);
            if status.success() {
                Ok(())
            } else if stderr.trim().is_empty() {
                // 退出码捕获：无 stderr 时它是唯一线索，直接进回执
                Err(format!(
                    "osascript 执行失败（退出码 {}）",
                    status.code().unwrap_or(-1)
                ))
            } else {
                Err(classify_resume_error(&stderr))
            }
        }
        // 超时：杀 + 收尸（防僵尸），失败回执（与 spawn Err 同管道走端点 failed）
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(format!(
                "osascript 执行超时（{OSASCRIPT_WAIT_SECS}s 上限，已终止）——请重试；若持续失败检查 macOS 自动化授权"
            ))
        }
        Err(e) => Err(format!("osascript 等待失败：{e}")),
    }
}

/// Windows 出手链（跨平台纯逻辑，wt 路径由调用方注入便于测试降级链）：wt 在场
/// 先试；spawn 失败（应用执行别名停用/损坏等场景）**降级重试一次 conhost** 再报
/// failed（两次错误合并披露，cwd 经 current_dir 继承不丢）。
fn open_windows_with(
    wt: Option<&str>,
    cwd: &str,
    resume: &str,
    spawner: &SpawnFn,
) -> Result<(), String> {
    let mut wt_err: Option<String> = None;
    if let Some(wt) = wt {
        let spec = build_spawn_command_windows(Some(wt), cwd, resume);
        match spawner(&spec) {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::warn!("wt spawn 失败，降级重试 conhost：{e}");
                wt_err = Some(e);
            }
        }
    }
    let spec = build_spawn_command_windows(None, cwd, resume);
    match spawner(&spec) {
        Ok(()) => Ok(()),
        Err(e) => Err(match wt_err {
            Some(w) => format!("{w}；{e}"),
            None => e,
        }),
    }
}

/// macOS 双通道缝出手（跨平台纯逻辑，Windows 上即可测）：iTerm2 优先、Terminal.app
/// 次选——依次组装 [`SpawnSpec::MacosApplescript`] 交 spawner，首通道 Ok 即返回；
/// 全败合并中文错误。实机开窗验证归 Mac 回传清单（执行层 = 生产 spawner 的
/// AppleScript 臂）。
fn open_macos_with(cwd: &str, resume: &str, spawner: &SpawnFn) -> Result<(), String> {
    let mut errs: Vec<String> = Vec::new();
    for script in [
        iterm_open_window_script(cwd, resume),
        terminal_open_script(cwd, resume),
    ] {
        match spawner(&SpawnSpec::MacosApplescript { script }) {
            Ok(()) => return Ok(()),
            Err(e) => errs.push(e),
        }
    }
    Err(format!("全部注入通道失败：{}", errs.join("；")))
}

/// 生产 spawner（RemoteState 生产装配 / Tauri 命令共用），按变体 cfg 分派：
/// - Windows 变体：CreateProcess **fire-and-forget（不 wait）**——wt/conhost 承载的
///   `cmd /k` 是常驻交互 shell，等它退出只会空耗超时窗（M2 裁决：与 macOS 的不对称
///   以此注释存证）；spawn 失败已由 `Command::spawn` Err → failed 回执覆盖。
///   `current_dir` 消费 cwd（评审 I1——conhost 回退靠进程工作目录继承落在项目目录）；
///   conhost 路径加 CREATE_NEW_CONSOLE(0x10)。
/// - macOS 变体：**等待完成**（[`run_osascript_wait`]：wait_timeout 10s + stderr/
///   退出码捕获）——osascript 下发脚本即返回（正常 <1s），等待代价可忽略，而失败
///   回执杜绝「audit open ok 但无窗」的账实背离（M2：Mac 验收 D-5 唯一实机 FAIL
///   根因①）。
pub fn spawn_terminal(spec: &SpawnSpec) -> Result<(), String> {
    match spec {
        #[cfg(windows)]
        SpawnSpec::Windows {
            program,
            args,
            cwd,
            new_console,
        } => {
            use std::os::windows::process::CommandExt;
            let mut cmd = std::process::Command::new(program);
            cmd.args(args);
            // conhost 回退的 cwd 继承点（wt 同设无害：-d 已双保险）
            if !cwd.is_empty() {
                cmd.current_dir(cwd);
            }
            if *new_console {
                // CREATE_NEW_CONSOLE：为 conhost 回退开全新控制台窗
                cmd.creation_flags(0x0000_0010);
            }
            cmd.spawn()
                .map(|_| ())
                .map_err(|e| format!("终端启动失败（{program}）：{e}"))
        }
        #[cfg(not(windows))]
        SpawnSpec::Windows { program, .. } => Err(format!("当前平台不支持终端 spawn（{program}）")),
        #[cfg(target_os = "macos")]
        SpawnSpec::MacosApplescript { script } => run_osascript_wait(script),
        #[cfg(not(target_os = "macos"))]
        SpawnSpec::MacosApplescript { .. } => {
            Err("AppleScript 执行层仅在 macOS 构建装配".to_string())
        }
    }
}

/// 核心入口（spawner 缝版本）：命令表解析 → cwd 预检 → 按平台组装 spec 交缝出手。
/// 错误契约：哨兵串 `"no_resume_command"` / `"no_cwd"`（远端端点映射 404；
/// 前端按钮同因禁用），其余为人类可读失败文案。
///
/// Windows 出手序（评审 M1）：wt 探测命中 → 先试 wt；spawn 失败（应用执行别名
/// 停用/损坏等场景）**降级重试一次 conhost** 再报 failed（两次错误合并披露，
/// cwd 经 current_dir 继承不丢）。
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
        // Windows：wt 在场先试、败降级 conhost 一次（open_windows_with 跨平台可测）
        "windows" => open_windows_with(windows_terminal_path().as_deref(), cwd, &resume, spawner),
        // macOS：iTerm2 优先、Terminal.app 次选（双通道缝出手，open_macos_with
        // 跨平台可测；实机验证归 Mac 回传清单）
        "macos" => open_macos_with(cwd, &resume, spawner),
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

    /// Step 2 失败测试 ②：Windows spawn 计划纯构造——有 wt（完整路径）→
    /// `<路径> -d cwd cmd /k`；无 wt → conhost + CREATE_NEW_CONSOLE（纯构造断言，
    /// 不真 spawn）。cwd 字段两分支同设（评审 I1：conhost 的 cwd 靠 current_dir
    /// 继承，生产 spawner 消费点见 spawn_terminal 的 cwd 臂注释）。
    /// 函数与测试同名：测试体内用 `super::` 显式路径防 glob 导入被本测试名遮蔽
    #[test]
    fn build_spawn_command_windows() {
        let wt = super::build_spawn_command_windows(
            Some(r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\wt.exe"),
            r"E:\proj",
            "claude --resume abc",
        );
        let SpawnSpec::Windows {
            program,
            args,
            cwd,
            new_console,
        } = wt
        else {
            panic!("wt 在场必须构造 Windows 变体");
        };
        assert_eq!(
            program, r"C:\Users\x\AppData\Local\Microsoft\WindowsApps\wt.exe",
            "评审 M1：缓存 wt 完整路径并进 spec（绕开应用执行别名停用场景）"
        );
        assert_eq!(
            args,
            vec![
                "-d".to_string(),
                r"E:\proj".to_string(),
                "cmd".to_string(),
                "/k".to_string(),
                "claude --resume abc".to_string(),
            ],
            "有 wt → <路径> -d <cwd> cmd /k <resume>"
        );
        assert_eq!(cwd, r"E:\proj", "cwd 字段两分支同设（current_dir 消费）");
        assert!(!new_console, "wt 自开新标签，无需 CREATE_NEW_CONSOLE");

        let conhost = super::build_spawn_command_windows(None, r"E:\proj", "claude --resume abc");
        let SpawnSpec::Windows {
            program,
            args,
            cwd,
            new_console,
        } = conhost
        else {
            panic!("无 wt 必须构造 Windows 变体（conhost）");
        };
        assert_eq!(program, "conhost.exe");
        assert_eq!(
            args,
            vec![
                "cmd".to_string(),
                "/k".to_string(),
                "claude --resume abc".to_string(),
            ],
            "无 wt → conhost.exe cmd /k <resume>"
        );
        assert_eq!(cwd, r"E:\proj", "评审 I1：conhost 回退不得丢 cwd");
        assert!(new_console, "conhost 必须 CREATE_NEW_CONSOLE 开新窗");
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
            recorded
                .iter()
                .any(|spec| matches!(spec, SpawnSpec::Windows { args, .. } if args.iter().any(|a| a == "claude --resume abc"))),
            "spawn 计划必须携带命令表产物：{recorded:?}"
        );
    }

    /// 评审 M1 降级链（驱动 open_windows_with 真链）：wt 出手失败 → 降级重试一次
    /// conhost，两次皆败错误合并；wt 不在场 → 只有 conhost 一次出手
    #[test]
    fn windows_wt_failure_falls_back_to_conhost_once() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let spawner = {
            let calls = calls.clone();
            move |spec: &SpawnSpec| {
                let is_wt = matches!(spec, SpawnSpec::Windows { program, .. } if program.ends_with("wt.exe"));
                let cwd = match spec {
                    SpawnSpec::Windows { cwd, .. } => cwd.clone(),
                    _ => String::new(),
                };
                calls.lock().unwrap().push(spec.clone());
                if is_wt {
                    // cwd 经参数直传（current_dir 在生产 spawner 消费）——降级链
                    // 两次出手必须携带同一 cwd
                    assert_eq!(cwd, "/tmp/proj", "降级链不得丢 cwd");
                    Err("wt 启动失败（模拟别名停用）".to_string())
                } else {
                    Ok(())
                }
            }
        };
        // wt 败 → conhost 接力成功：恰两次出手，Ok
        assert_eq!(
            open_windows_with(
                Some("C:\\fake\\wt.exe"),
                "/tmp/proj",
                "claude --resume abc",
                &spawner
            ),
            Ok(()),
            "wt 失败必须降级 conhost 重试"
        );
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2, "降级链恰两次出手（wt → conhost）");
        assert!(
            matches!(&recorded[0], SpawnSpec::Windows { program, .. } if program.ends_with("wt.exe")),
            "首次出手是 wt"
        );
        assert!(
            matches!(&recorded[1], SpawnSpec::Windows { program, new_console, cwd, .. }
                if program == "conhost.exe" && *new_console && cwd == "/tmp/proj"),
            "降级出手是 conhost + CREATE_NEW_CONSOLE + 原 cwd"
        );

        // wt 不在场：只有 conhost 一次出手（链路入口直接 None）
        let calls2 = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let spawner2 = {
            let calls2 = calls2.clone();
            move |spec: &SpawnSpec| {
                assert!(
                    matches!(spec, SpawnSpec::Windows { program, .. } if program == "conhost.exe"),
                    "无 wt 只应出手 conhost"
                );
                *calls2.lock().unwrap() += 1;
                Ok(())
            }
        };
        assert_eq!(
            open_windows_with(None, "/tmp/proj", "claude --resume abc", &spawner2),
            Ok(())
        );
        assert_eq!(*calls2.lock().unwrap(), 1, "无 wt 恰一次出手");
    }

    /// 评审 I2：macOS 双通道入缝（跨平台可测——构造与出手顺序在 Windows 上即可测）：
    /// iTerm2 优先；首通道败 → Terminal.app 次选；全败合并错误。spec 恒
    /// MacosApplescript 变体且携带 cd '<cwd>' && <resume> 载荷
    #[test]
    fn macos_dual_channel_dispatches_via_seam() {
        // ① 首通道（iTerm2）成功：恰一次出手，script 含 cd + activate
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let spawner = {
            let calls = calls.clone();
            move |spec: &SpawnSpec| {
                calls.lock().unwrap().push(spec.clone());
                Ok(())
            }
        };
        assert_eq!(
            open_macos_with("/tmp/proj", "claude --resume abc", &spawner),
            Ok(())
        );
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "首通道成功恰一次出手");
        let SpawnSpec::MacosApplescript { script } = &recorded[0] else {
            panic!("macOS 缝必须收 AppleScript 变体");
        };
        assert!(script.contains("cd '/tmp/proj' && claude --resume abc"));
        assert!(script.contains("activate"));

        // ② 首通道失败：降级 Terminal.app 次选（第二次出手 do script 形态）
        let calls2 = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let spawner2 = {
            let calls2 = calls2.clone();
            move |spec: &SpawnSpec| {
                calls2.lock().unwrap().push(spec.clone());
                Err("osascript 失败".to_string())
            }
        };
        let _ = open_macos_with("/tmp/proj", "claude --resume abc", &spawner2);
        let recorded2 = calls2.lock().unwrap().clone();
        assert_eq!(recorded2.len(), 2, "首通道败必须降级第二通道");
        let SpawnSpec::MacosApplescript { script: second } = &recorded2[1] else {
            panic!("第二通道同为 AppleScript 变体");
        };
        assert!(
            second.contains(r#"do script "cd '/tmp/proj'"#),
            "次选是 Terminal.app 形态"
        );
    }

    /// M2：osascript 错误分类（纯函数跨平台可测）——TCC -1743 /「Not authorized to
    /// send Apple events」→ 中文授权指引（用户裁决口径原文）
    #[test]
    fn classify_resume_error_maps_tcc_to_guidance() {
        // Mac 验收 D-5 实机 stderr 形态（osascript 标准错误原文，§四-B）
        let real =
            "script: execution error: Not authorized to send Apple events to Terminal (-1743).";
        assert_eq!(classify_resume_error(real), MACOS_TCC_GUIDANCE);
        // 仅 -1743 字样（退出码形态兜底路径）同样命中
        assert_eq!(
            classify_resume_error("execution error: (-1743)"),
            MACOS_TCC_GUIDANCE
        );
        // 大小写不敏感（osascript 输出口径漂移容错）
        assert_eq!(
            classify_resume_error("not authorized to send apple events"),
            MACOS_TCC_GUIDANCE
        );
    }

    /// M2：osascript 错误分类——非 TCC 错误保持 stderr 摘要（截断防膨胀）；空
    /// stderr 有界兜底（回执不悬空）
    #[test]
    fn classify_resume_error_other_keeps_summary() {
        // 非 TCC 错误保留原始摘要（-1728 找不到应用），不得误报授权指引
        let out =
            classify_resume_error("execution error: Can't get application \"iTerm2\". (-1728)");
        assert!(out.contains("-1728"), "摘要保留原始错误：{out}");
        assert!(
            !out.contains("自动化授权"),
            "非 TCC 错误不得误报授权指引：{out}"
        );
        // 超长 stderr 截断（+1 为截断省略号）——回执与审计双消费均有界
        let long = format!("execution error: {}", "x".repeat(500));
        let out = classify_resume_error(&long);
        assert!(
            out.chars().count() <= RESUME_STDERR_SUMMARY_CHARS + 1,
            "stderr 摘要必须截断：{} chars",
            out.chars().count()
        );
        // 空/纯空白 stderr → 有界兜底文案
        assert_eq!(
            classify_resume_error("   "),
            "osascript 执行失败（无 stderr 输出）"
        );
    }
}
