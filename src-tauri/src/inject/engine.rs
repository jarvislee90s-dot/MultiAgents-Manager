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
//!
//! ## M9R 事件构造纯核（按族分支，M6R §8.1）
//!
//! 键事件统一由 [`KeyRecordSpec`]（vk + scan + UTF-16 字符 + down/up）描述，
//! 按 TUI 族分支构造（族表见 `super::families`）：
//! - **A 族（claude/kimi/opencode，原生 VT 流）**：方向键走 [`vt_seq_records`]
//!   整条 VT 序列字符流（vk=0/scan=0，执行层须单批原子写）；正文非 ASCII 字符
//!   一律 vk=0 纯字符流（M6R 实证 vk=0 被消费，与现役 ConIn.ps1 口径一致）；
//! - **B 族（codex，crossterm）**：控制键/回车/方向键必须 VK+scan 键形态
//!   （B 族丢弃 vk=0 控制字符）——[`enter_records`] / [`control_records`] /
//!   [`vk_arrow_records`]。
//!
//! 键位映射走 [`KeyLayout`] 缝：Windows 真 FFI 实现是 `windows_console::WinKeyLayout`
//! （VkKeyScanW/MapVirtualKeyW，M6R §8.1），测试用假实现；纯核零平台 cfg，
//! Windows 上全绿可测。字符事件一律 keydown+keyup 成对构造，keyup 由各家执行层
//! 过滤/忽略（M6R F4 无双写）；回车独立提交（[`enter_records`]），正文不再特判
//! `\n`（上游 normalize 保证正文无裸换行）。

/// 键事件记录规格（纯层自有类型，不引 windows 类型）：`vk` 虚拟键码
/// （0 = 纯字符流形态）；`scan` 扫描码（[`KeyLayout::scan_of`] 派生，VK 形态
/// 事件必带——B 族 crossterm 以 VK+scan 为准）；`ch` UTF-16 code unit
/// （Windows UnicodeChar 口径）；`down` 按下（false 为抬起）。
pub struct KeyRecordSpec {
    pub vk: u16,
    pub scan: u16,
    pub ch: u16,
    pub down: bool,
}

/// 键位布局缝（M9R）：字符 → 虚拟键码、虚拟键码 → 扫描码。
/// Windows 真 FFI 实现 `windows_console::WinKeyLayout`（VkKeyScanW /
/// MapVirtualKeyW）；测试装配假实现。零平台 cfg——纯核跨平台可测。
pub trait KeyLayout {
    /// 字符 → 虚拟键码；真实布局对不可键入字符返回 0（调用方按纯字符流处理）。
    fn vk_of(&self, c: char) -> u16;
    /// 虚拟键码 → 扫描码（Windows：`MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)`）。
    fn scan_of(&self, vk: u16) -> u16;
}

/// 文本 → 键事件序列（一律 keydown+keyup 成对，M9R 按字符分流构造）：
/// - **ASCII 字符** → VK 形态：`vk = layout.vk_of(c)`（真实布局不可键入时为 0）、
///   `scan = layout.scan_of(vk)`、`ch = c`；
/// - **非 ASCII 字符**（CJK/emoji）→ 纯字符流：`vk = 0, scan = 0`，按
///   `encode_utf16()` 逐 code unit 成对（`ch = unit`，代理对天然展开——
///   `ch: u16` 与 UTF-16 口径对齐）。真实布局下非 ASCII 一律走 vk=0 字符流，
///   与现役 ConIn.ps1 口径一致（M6R 实证 vk=0 被消费）。
///
/// 旧实现对 `\n` 特判 VK_RETURN 的逻辑**不再保留**：M9R 新架构回车独立提交
/// （[`enter_records`]），上游 normalize 保证正文无裸换行。
pub fn text_records(text: &str, layout: &dyn KeyLayout) -> Vec<KeyRecordSpec> {
    let mut records = Vec::new();
    for c in text.chars() {
        if c.is_ascii() {
            let vk = layout.vk_of(c);
            let scan = layout.scan_of(vk);
            let ch = c as u16;
            records.push(KeyRecordSpec {
                vk,
                scan,
                ch,
                down: true,
            });
            records.push(KeyRecordSpec {
                vk,
                scan,
                ch,
                down: false,
            });
        } else {
            // char::encode_utf16 写入栈上缓冲（单字符最长代理对 2 个 code unit）
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                records.push(KeyRecordSpec {
                    vk: 0,
                    scan: 0,
                    ch: *unit,
                    down: true,
                });
                records.push(KeyRecordSpec {
                    vk: 0,
                    scan: 0,
                    ch: *unit,
                    down: false,
                });
            }
        }
    }
    records
}

/// 回车 → VK 形态事件对（三家统一，M6R F3：B 族丢弃 vk=0 控制字符）：
/// `vk = layout.vk_of('\r')`、`scan = layout.scan_of(vk)`、`ch = 0x0D`，
/// 成对 down/up（len = 2）。
pub fn enter_records(layout: &dyn KeyLayout) -> Vec<KeyRecordSpec> {
    let vk = layout.vk_of('\r');
    let scan = layout.scan_of(vk);
    vec![
        KeyRecordSpec {
            vk,
            scan,
            ch: 0x0D,
            down: true,
        },
        KeyRecordSpec {
            vk,
            scan,
            ch: 0x0D,
            down: false,
        },
    ]
}

/// 控制键 → VK 形态事件对；**键域校验（P2-2）**：键域 = `"enter"`/`"esc"`/
/// `"tab"` + 单字符 ASCII 字母数字（审批键位 "y"/"1" 走这里）；域外（空串/
/// 多字符/非 ASCII）→ `None`。构造：enter/esc/tab → VK_RETURN/VK_ESCAPE/
/// VK_TAB 且 ch 同码；单字符 → `vk = layout.vk_of(c)`、`ch = c`；scan 一律
/// `layout.scan_of(vk)` 派生；成对 down/up。
pub fn control_records(key: &str, layout: &dyn KeyLayout) -> Option<Vec<KeyRecordSpec>> {
    let (vk, ch) = match key {
        "enter" => (0x0Du16, 0x0Du16), // VK_RETURN
        "esc" => (0x1Bu16, 0x1Bu16),   // VK_ESCAPE
        "tab" => (0x09u16, 0x09u16),   // VK_TAB
        _ => {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => (layout.vk_of(c), c as u16),
                _ => return None, // 域外
            }
        }
    };
    let scan = layout.scan_of(vk);
    Some(vec![
        KeyRecordSpec {
            vk,
            scan,
            ch,
            down: true,
        },
        KeyRecordSpec {
            vk,
            scan,
            ch,
            down: false,
        },
    ])
}

/// VT 序列 → 整条字符流事件（A 族方向键等，M6R 定案）：按 `encode_utf16()`
/// 逐 code unit 成对 down/up，全部 `vk = 0, scan = 0, ch = unit`。
/// **执行层须整条单批原子写**（M6R：ESC 拆批会被 B 族当按键吃）。
pub fn vt_seq_records(seq: &str) -> Vec<KeyRecordSpec> {
    seq.encode_utf16()
        .flat_map(|unit| {
            [
                KeyRecordSpec {
                    vk: 0,
                    scan: 0,
                    ch: unit,
                    down: true,
                },
                KeyRecordSpec {
                    vk: 0,
                    scan: 0,
                    ch: unit,
                    down: false,
                },
            ]
        })
        .collect()
}

/// 方向键名 → VK+scan 键形态事件对（B 族方向键，M6R 定案）：`"up"`/`"down"`/
/// `"left"`/`"right"` → VK_UP(0x26)/VK_DOWN(0x28)/VK_LEFT(0x25)/VK_RIGHT(0x27)，
/// `ch = 0`、`scan = layout.scan_of(vk)`，成对 down/up（len = 2）；域外键名
/// → `None`。
pub fn vk_arrow_records(seq: &str, layout: &dyn KeyLayout) -> Option<Vec<KeyRecordSpec>> {
    let vk = match seq {
        "up" => 0x26u16,    // VK_UP
        "down" => 0x28u16,  // VK_DOWN
        "left" => 0x25u16,  // VK_LEFT
        "right" => 0x27u16, // VK_RIGHT
        _ => return None,
    };
    let scan = layout.scan_of(vk);
    Some(vec![
        KeyRecordSpec {
            vk,
            scan,
            ch: 0,
            down: true,
        },
        KeyRecordSpec {
            vk,
            scan,
            ch: 0,
            down: false,
        },
    ])
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

    /// 假布局（测试缝）：vk_of 恒等映射、scan_of = vk+1——不触任何平台 FFI，
    /// 跨平台确定可测（真实键位映射归 WinKeyLayout，实机验证归 Task 12）。
    struct FakeLayout;

    impl KeyLayout for FakeLayout {
        fn vk_of(&self, c: char) -> u16 {
            c as u16
        }
        fn scan_of(&self, vk: u16) -> u16 {
            vk + 1
        }
    }

    #[test]
    fn text_records_pair_with_vk_scan() {
        let r = text_records("ab", &FakeLayout);
        assert_eq!(r.len(), 4);
        assert_eq!(
            (r[0].vk, r[0].scan, r[0].ch, r[0].down),
            (b'a' as u16, b'a' as u16 + 1, b'a' as u16, true)
        );
        assert!(!r[1].down); // 成对；keyup 由各家过滤/忽略（M6R F4 无双写）
    }

    #[test]
    fn enter_is_vk_form() {
        // 三家统一 VK 回车（B 族丢 vk=0 控制字符，M6R F3）
        let r = enter_records(&FakeLayout);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].vk, FakeLayout.vk_of('\r'));
        assert_eq!(r[0].ch, 0x0D);
        assert!(r[0].scan > 0);
    }

    #[test]
    fn control_keys_and_domain() {
        assert!(control_records("esc", &FakeLayout).is_some());
        assert!(control_records("tab", &FakeLayout).is_some());
        assert!(control_records("我们", &FakeLayout).is_none()); // 域外 None——域校验（P2-2）
    }

    #[test]
    fn vt_seq_is_char_stream() {
        // A 族方向键：整条单批原子写的字符流
        let r = vt_seq_records("\x1b[A");
        assert!(r.iter().all(|k| k.vk == 0 && k.scan == 0));
        assert_eq!(r[0].ch, 0x1B_u16);
    }

    #[test]
    fn vk_arrow_for_crossterm() {
        // B 族方向键：VK+scan、char=0
        let r = vk_arrow_records("up", &FakeLayout).unwrap();
        assert_eq!(r[0].ch, 0);
        assert!(r[0].vk > 0 && r[0].scan > 0);
    }
}
