use super::applescript::execute_applescript;

/// 通过 TTY 聚焦 iTerm2 标签页。tty 匹配为**全路径相等**（Task 9 清单外必要
/// 扩展，R6「三处统一」明文要求：聚焦侧与注入侧同口径，contains 有 ttys005
/// 撞 ttys0050 前缀撞号）；入参裸后缀先归一 `/dev/` 全路径。
pub fn focus_iterm_by_tty(tty: &str) -> Result<(), String> {
    let full_tty = super::normalize_dev_tty(tty);
    let script = format!(
        r#"
        tell application "System Events"
            if not (exists process "iTerm2") then
                error "iTerm2 not running"
            end if
        end tell
        tell application "iTerm2"
            activate
            repeat with w in windows
                repeat with t in tabs of w
                    repeat with s in sessions of t
                        if tty of s is "{}" then
                            select s
                            select t
                            set index of w to 1
                            return "found"
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
        return "not found"
    "#,
        full_tty
    );
    execute_applescript(&script)
}
