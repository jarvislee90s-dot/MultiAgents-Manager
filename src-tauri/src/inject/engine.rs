//! 注入引擎（M7）：构造纯核（跨平台可测）+ macOS 执行层 + [`Injector`] 缝。
//!
//! ## 语义铁则（三通道注入形态，一律「打字+回车」，不用 paste-buffer）
//! - **tmux**：文本一律 `-l` 字面量发送（`send-keys -t <target> -l -- <text>`），
//!   杜绝以 `:` 开头的文本被 tmux 当作键名/命令前缀解析；回车单独一步
//!   `send-keys -t <target> Enter`（键名形态，与文本构成两连发）。
//! - **iTerm2**：`write text` 两步走——先 `write s text "<escaped>" newline NO`
//!   （纯文本、不带换行），再 `write s text ""`（补回车）。
//! - **Terminal.app**：`do script "<escaped>" in w`（do script 自带回车，
//!   注意反斜杠/双引号转义，见 [`applescript_escape`]）。
//!
//! 构造与执行分离：本文件 `*Args` / `*Script` 纯函数全部跨平台 `pub` 可测
//! （Windows 上必须全过；脚本构造内部完成 AppleScript 转义，调用方传原文，
//! 避免执行层二次转义）；执行层按平台 cfg 装配——macOS 真实现（归回传清单
//! 实测复核）、Windows ConPTY 真实现（M9 Task 14，一跳验证归 Task 15/16）、
//! 其他平台兜底 Err。

/// Windows 单键名 → 虚拟键码（VK_）：单字符 ASCII 字母/数字 → 大写 VK
/// （'1'→0x31 … 'a'→0x41 大写位）；"enter"→VK_RETURN；"esc"→VK_ESCAPE；
/// "tab"→VK_TAB；多字符/其他 → None（调用方走文本通道）。
/// 跨平台纯函数（Windows 执行层消费，测试跨平台跑）。
pub fn key_to_windows_vk(key: &str) -> Option<u16> {
    match key {
        "enter" => Some(0x0D), // VK_RETURN
        "esc" => Some(0x1B),   // VK_ESCAPE
        "tab" => Some(0x09),   // VK_TAB
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => match c {
                    'a'..='z' => Some(0x41 + (c as u16 - 'a' as u16)),
                    'A'..='Z' => Some(0x41 + (c as u16 - 'A' as u16)),
                    '0'..='9' => Some(0x30 + (c as u16 - '0' as u16)),
                    _ => None,
                },
                _ => None,
            }
        }
    }
}

/// 键事件记录规格（纯层自有类型，不引 windows 类型）：`vk` 虚拟键码；
/// `ch` UTF-16 code unit；`down` 按下（false 为抬起）。
pub struct KeyRecordSpec {
    pub vk: u16,
    pub ch: u16,
    pub down: bool,
}

/// 文本 → 键事件序列：每字符按 UTF-16 code unit 生成 keydown+keyup 事件对
/// （统一 vk=0 + UnicodeChar，与 ConIn.ps1 一致——vk=0 也被消费；`\n` 字符
/// 生成 VK_RETURN 事件对 vk=0x0D ch='\r'）。
pub fn text_to_key_records(text: &str) -> Vec<KeyRecordSpec> {
    let mut records = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        // \n（0x0A）→ VK_RETURN 事件对；其余统一 vk=0 + UnicodeChar
        let (vk, ch) = if unit == u16::from(b'\n') {
            (0x0Du16, 0x0Du16)
        } else {
            (0u16, unit)
        };
        records.push(KeyRecordSpec { vk, ch, down: true });
        records.push(KeyRecordSpec {
            vk,
            ch,
            down: false,
        });
    }
    records
}

/// 构造 tmux 发送文本参数：`-l` 字面量 + `--` 终止选项解析。
pub fn tmux_send_args(target: &str, text: &str) -> Vec<String> {
    ["send-keys", "-t", target, "-l", "--", text]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// 构造 tmux 回车参数：Enter 走键名（与文本字面量构成两连发）。
pub fn tmux_enter_args(target: &str) -> Vec<String> {
    ["send-keys", "-t", target, "Enter"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// 构造 tmux 单键参数："enter"→Enter 键名；"esc"→Escape 键名；其余一律 `-l` 字面。
pub fn tmux_key_args(target: &str, key: &str) -> Vec<String> {
    match key {
        "enter" => tmux_enter_args(target),
        "esc" => ["send-keys", "-t", target, "Escape"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        // 未知键名（含单字符）一律字面量，避免撞 tmux 键表
        _ => tmux_send_args(target, key),
    }
}

/// AppleScript 字符串字面量转义：反斜杠→`\\`、引号→`\"`（先反斜杠后引号，
/// 顺序颠倒会把已转义内容的反斜杠再次转义，产生错误字面量）。裸换行/回车转义为
/// 字面 `\n`/`\r` 两字符——AppleScript 字符串字面量不允许裸换行，且归一后的载荷
/// 本就以字面 `\n` 语义上行（裁决 6），两处语义对齐。
pub fn applescript_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// 运行态守卫（仓内 window/iterm.rs 既有约定）：App 未运行直接返回哨兵，
/// 杜绝 fallback 探测冷启动终端 App 抢焦点。
fn app_running_guard(process: &str) -> String {
    format!(
        r#"
        tell application "System Events"
            if not (exists process "{process}") then
                return "not found"
            end if
        end tell"#
    )
}

/// 构造 iTerm2 写入脚本：遍历窗口/tab/session 定位 TTY 后缀，两步写入
/// （文本 `newline NO` + 空文本补回车）。`text` 为原文，内部完成转义。
pub fn iterm_write_script(tty_suffix: &str, text: &str) -> String {
    format!(
        r#"{guard}
        tell application "iTerm2"
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s contains "{suffix}" then
                            write s text "{text}" newline NO
                            write s text ""
                            return "found"
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
        return "not found"
    "#,
        guard = app_running_guard("iTerm2"),
        suffix = tty_suffix,
        text = applescript_escape(text)
    )
}

/// 构造 iTerm2 单键脚本：enter→`keystroke return`；esc→`key code 53`；
/// 其余（单字符/未知多字符）→`keystroke "<key>"` 字面（内部完成转义）。
pub fn iterm_send_key_script(tty_suffix: &str, key: &str) -> String {
    format!(
        r#"{guard}
        tell application "iTerm2"
            activate
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s contains "{suffix}" then
                            select s
                            select t
                            set index of w to 1
                            tell application "System Events"
                                {action}
                            end tell
                            return "found"
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
        return "not found"
    "#,
        guard = app_running_guard("iTerm2"),
        suffix = tty_suffix,
        action = key_action_line(key)
    )
}

/// 构造 Terminal.app 执行脚本：窗口 tty 精确匹配（`/dev/` 前缀）后 `do script`
/// （do script 自带回车）。`text` 为原文，内部完成转义。
pub fn terminal_do_script(tty_suffix: &str, text: &str) -> String {
    format!(
        r#"{guard}
        tell application "Terminal"
            repeat with w in windows
                if tty of w is "/dev/{suffix}" then
                    do script "{text}" in w
                    set index of w to 1
                    return "found"
                end if
            end repeat
        end tell
        return "not found"
    "#,
        guard = app_running_guard("Terminal"),
        suffix = tty_suffix,
        text = applescript_escape(text)
    )
}

/// 构造 Terminal.app 单键脚本（「activate + keystroke」变体，无 do-script
/// 单键等价形态；按键形态归回传清单实测复核）。
pub fn terminal_send_key_script(tty_suffix: &str, key: &str) -> String {
    format!(
        r#"{guard}
        tell application "Terminal"
            activate
            set found to false
            repeat with w in windows
                if tty of w is "/dev/{suffix}" then
                    set index of w to 1
                    set found to true
                    exit repeat
                end if
            end repeat
        end tell
        if found then
            tell application "System Events"
                {action}
            end tell
        else
            return "not found"
        end if
    "#,
        guard = app_running_guard("Terminal"),
        suffix = tty_suffix,
        action = key_action_line(key)
    )
}

/// 单键动作行（iTerm2/Terminal 通用派发）：enter→`keystroke return`；
/// esc→`key code 53`；其余→`keystroke "<key>"` 字面（内部完成转义）。
fn key_action_line(key: &str) -> String {
    match key {
        "enter" => "keystroke return".to_string(),
        "esc" => "key code 53".to_string(),
        _ => format!("keystroke \"{}\"", applescript_escape(key)),
    }
}

/// 注入器缝（M7）：定位会话进程所在终端并注入文本/按键。
/// 生产装配 [`RealInjector`]；测试可替换实现。
pub trait Injector: Send + Sync {
    fn name(&self) -> &'static str {
        "real"
    }
    fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String>;
    fn locate_and_send_key(&self, pid: u32, key: &str) -> Result<(), String>;
}

pub struct RealInjector;

/// macOS 执行层：tmux → iTerm2 → Terminal.app 三通道链式降级，任一成功即 Ok，
/// 全败合并中文错误。
#[cfg(target_os = "macos")]
impl Injector for RealInjector {
    fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String> {
        let tty =
            crate::window::get_tty_for_pid(pid).map_err(|e| format!("定位终端失败：{}", e))?;
        // tty 形如 /dev/ttys005；AppleScript 侧改用后缀匹配（ttys005）
        let suffix = tty.rsplit('/').next().unwrap_or(&tty);
        let mut errs: Vec<String> = Vec::new();

        // ① tmux：命中 pane 则文本字面量 + 回车键名两连发。
        // 文本已发出而回车失败 = 半成功态：pane 输入行挂着无回车文本，不可再让
        // fallback 通道对同一会话重复注入（也不该诱导用户重试叠加）——立即返回 Err。
        if let Some(target) = find_tmux_pane(&tty) {
            match run_tmux(&tmux_send_args(&target, text)) {
                Ok(()) => {
                    return match run_tmux(&tmux_enter_args(&target)) {
                        Ok(()) => Ok(()),
                        Err(e) => Err(format!("tmux 回车失败（文本已入 pane，勿重试）：{}", e)),
                    };
                }
                Err(e) => errs.push(format!("tmux 文本注入失败：{}", e)),
            }
        }

        // ② iTerm2：先文本（newline NO）后回车两步（脚本构造内部完成转义）
        if let Err(e) =
            crate::window::applescript::execute_applescript(&iterm_write_script(suffix, text))
        {
            errs.push(format!("iTerm2 注入失败：{}", e));
        } else {
            return Ok(());
        }

        // ③ Terminal.app：do script（自带回车，脚本构造内部完成转义）
        match crate::window::applescript::execute_applescript(&terminal_do_script(suffix, text)) {
            Ok(()) => Ok(()),
            Err(e) => {
                errs.push(format!("Terminal.app 注入失败：{}", e));
                Err(format!("全部注入通道失败：{}", errs.join("；")))
            }
        }
    }

    fn locate_and_send_key(&self, pid: u32, key: &str) -> Result<(), String> {
        let tty =
            crate::window::get_tty_for_pid(pid).map_err(|e| format!("定位终端失败：{}", e))?;
        let suffix = tty.rsplit('/').next().unwrap_or(&tty);
        let mut errs: Vec<String> = Vec::new();

        // ① tmux：单键参数派发（enter/esc 键名，其余字面）
        if let Some(target) = find_tmux_pane(&tty) {
            match run_tmux(&tmux_key_args(&target, key)) {
                Ok(()) => return Ok(()),
                Err(e) => errs.push(format!("tmux 按键失败：{}", e)),
            }
        }

        // ② iTerm2：keystroke / key code
        if let Err(e) =
            crate::window::applescript::execute_applescript(&iterm_send_key_script(suffix, key))
        {
            errs.push(format!("iTerm2 按键失败：{}", e));
        } else {
            return Ok(());
        }

        // ③ Terminal.app：activate + keystroke 变体（按键形态归回传清单实测复核）
        match crate::window::applescript::execute_applescript(&terminal_send_key_script(
            suffix, key,
        )) {
            Ok(()) => Ok(()),
            Err(e) => {
                errs.push(format!("Terminal.app 按键失败：{}", e));
                Err(format!("全部注入通道失败：{}", errs.join("；")))
            }
        }
    }
}

/// 在 tmux 全局 pane 列表中按 tty 定位 pane target（`session:win.pane`）。
/// tmux 不存在/无命中 → None。
#[cfg(target_os = "macos")]
fn find_tmux_pane(tty: &str) -> Option<String> {
    let output = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_tty} #{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let panes = String::from_utf8_lossy(&output.stdout);
    for line in panes.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(pane_tty), Some(target)) = (parts.next(), parts.next()) {
            if pane_tty.contains(tty) {
                return Some(target.to_string());
            }
        }
    }
    None
}

/// 执行 tmux 子命令：status.success() 判定，失败携带 stderr。
#[cfg(target_os = "macos")]
fn run_tmux(args: &[String]) -> Result<(), String> {
    let output = std::process::Command::new("tmux")
        .args(args)
        .output()
        .map_err(|e| format!("tmux 执行失败：{}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tmux 退出码 {:?}：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Windows 执行层（M9，Task 14）：ConPTY 通道——AttachConsole + CONIN$ +
/// WriteConsoleInput 逐片键事件（M6 探测结论落地），实现见
/// [`crate::inject::windows_console`]；真实一跳验证归 Task 15/16。
#[cfg(windows)]
impl Injector for RealInjector {
    fn locate_and_inject(&self, pid: u32, text: &str) -> Result<(), String> {
        crate::inject::windows_console::locate_and_inject(pid, text)
    }
    fn locate_and_send_key(&self, pid: u32, key: &str) -> Result<(), String> {
        crate::inject::windows_console::locate_and_send_key(pid, key)
    }
}

/// 其他平台兜底。
#[cfg(not(any(target_os = "macos", windows)))]
impl Injector for RealInjector {
    fn locate_and_inject(&self, _pid: u32, _text: &str) -> Result<(), String> {
        Err("平台不支持".into())
    }
    fn locate_and_send_key(&self, _pid: u32, _key: &str) -> Result<(), String> {
        Err("平台不支持".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_literal_and_enter() {
        // -l 字面量杜绝 ":开头被当键名"（参考实现铁则）
        assert_eq!(
            tmux_send_args("ses:0.1", "hi"),
            vec!["send-keys", "-t", "ses:0.1", "-l", "--", "hi"]
        );
        assert_eq!(
            tmux_enter_args("ses:0.1"),
            vec!["send-keys", "-t", "ses:0.1", "Enter"]
        );
    }

    #[test]
    fn tmux_key_args_literal_vs_named() {
        assert_eq!(
            tmux_key_args("t", "1"),
            vec!["send-keys", "-t", "t", "-l", "--", "1"]
        );
        assert_eq!(
            tmux_key_args("t", "esc"),
            vec!["send-keys", "-t", "t", "Escape"]
        );
        assert_eq!(
            tmux_key_args("t", "enter"),
            vec!["send-keys", "-t", "t", "Enter"]
        );
        // 未知一律字面
        assert_eq!(
            tmux_key_args("t", "bad!"),
            vec!["send-keys", "-t", "t", "-l", "--", "bad!"]
        );
    }

    #[test]
    fn applescript_escape_backslash_and_quotes() {
        assert_eq!(applescript_escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn iterm_script_finds_tty_and_writes_without_newline() {
        let s = iterm_write_script("ttys005", r#"say "hi""#);
        assert!(s.contains(r#"tty of s contains "ttys005""#));
        // 先文本后回车两步（可校验）
        assert!(s.contains(r#"write s text "say \"hi\"" newline NO"#));
        assert!(s.contains(r#"write s text """#)); // 补回车
    }

    #[test]
    fn iterm_send_key_script_dispatch() {
        let s = iterm_send_key_script("ttys005", "1");
        assert!(s.contains(r#"tty of s contains "ttys005""#));
        assert!(s.contains(r#"keystroke "1""#));
        // esc 键码
        assert!(iterm_send_key_script("ttys005", "esc").contains("key code 53"));
    }

    #[test]
    fn terminal_script_uses_do_script() {
        let s = terminal_do_script("ttys005", "hi");
        assert!(s.contains(r#"tty of w is "/dev/ttys005""#));
        assert!(s.contains(r#"do script "hi" in w"#));
    }

    #[test]
    fn terminal_send_key_script_activate_and_keystroke() {
        // Terminal.app 单键走「activate + keystroke」变体，同窗口 tty 精确匹配
        let s = terminal_send_key_script("ttys005", "1");
        assert!(s.contains(r#"tty of w is "/dev/ttys005""#));
        assert!(s.contains(r#"keystroke "1""#));
        assert!(terminal_send_key_script("ttys005", "enter").contains("keystroke return"));
        assert!(terminal_send_key_script("ttys005", "esc").contains("key code 53"));
    }

    #[test]
    fn vk_mapping() {
        assert_eq!(key_to_windows_vk("enter"), Some(0x0D));
        assert_eq!(key_to_windows_vk("esc"), Some(0x1B));
        assert_eq!(key_to_windows_vk("tab"), Some(0x09));
        assert_eq!(key_to_windows_vk("y"), Some(0x59));
        assert_eq!(key_to_windows_vk("1"), Some(0x31));
        // 字母统一大写 VK 位（大小写字面等价）
        assert_eq!(key_to_windows_vk("Y"), Some(0x59));
        assert_eq!(key_to_windows_vk("a"), Some(0x41));
        assert_eq!(key_to_windows_vk("0"), Some(0x30));
        // 非单键 → 走文本通道
        assert_eq!(key_to_windows_vk("我们"), None);
        assert_eq!(key_to_windows_vk("ok"), None);
        assert_eq!(key_to_windows_vk("!"), None);
        assert_eq!(key_to_windows_vk(""), None);
    }

    #[test]
    fn text_records_pair_down_up() {
        let recs = text_to_key_records("ab\n");
        assert_eq!(recs.len(), 6);
        assert_eq!(recs[0].ch, 'a' as u16);
        assert!(recs[0].down);
        assert!(!recs[1].down);
        // \n → VK_RETURN 事件对（vk=0x0D ch='\r'）
        assert_eq!((recs[4].vk, recs[4].ch), (0x0D, 0x0D));
        assert!(recs[4].down);
        assert!(!recs[5].down);
    }

    #[test]
    fn text_records_non_ascii_vk_zero() {
        // 非 ASCII 按 UTF-16 code unit 拆对，统一 vk=0（与 ConIn.ps1 一致）
        let recs = text_to_key_records("我");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].vk, 0);
        assert_eq!(recs[0].ch, 0x6211); // '我' 的 UTF-16 code unit
        assert!(recs[0].down);
        assert!(!recs[1].down);
        // emoji 代理对：2 个 code unit → 4 事件
        assert_eq!(text_to_key_records("😀").len(), 4);
    }

    #[test]
    fn tail_enter_pair_matches_vk_return_spec() {
        // 尾部回车事件对与 text_to_key_records("\n") 同规格（执行层直接复用）
        let tail = text_to_key_records("\n");
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].vk, 0x0D);
        assert_eq!(tail[0].ch, 0x0D);
        assert!(tail[0].down && !tail[1].down);
    }
}
