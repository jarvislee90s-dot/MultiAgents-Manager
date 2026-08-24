// Windows 窗口聚焦 — 进程祖先链 + EnumWindows（纯 Win32 API，不 spawn 子进程）

use std::collections::HashSet;

/// 沿父进程链收集 PID 集合（含起始 PID 自身）；父进程查询由闭包注入便于单测
/// 遇到环（重复 PID）或父进程缺失即停止；max_depth 防御异常深链
fn collect_ancestor_pids_with(
    pid: u32,
    mut parent_of: impl FnMut(u32) -> Option<u32>,
) -> HashSet<u32> {
    let mut set = HashSet::new();
    let mut current = pid;
    for _ in 0..64 {
        if !set.insert(current) {
            break; // 已见过 → 环
        }
        match parent_of(current) {
            Some(p) => current = p,
            None => break,
        }
    }
    set
}

/// 收集指定进程的祖先链 PID 集合（含自身）
fn collect_ancestor_pids(system: &sysinfo::System, pid: u32) -> HashSet<u32> {
    collect_ancestor_pids_with(pid, |p| {
        system
            .process(sysinfo::Pid::from_u32(p))
            .and_then(|proc| proc.parent())
            .map(|pp| pp.as_u32())
    })
}

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, SwitchToThisWindow, GWL_EXSTYLE, SW_RESTORE, WS_EX_TOOLWINDOW,
};

struct EnumContext {
    pid_set: HashSet<u32>,
    found: Option<HWND>,
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumContext);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1); // 继续枚举
    }
    let style = GetWindowLongW(hwnd, GWL_EXSTYLE);
    if (style & WS_EX_TOOLWINDOW.0 as i32) != 0 {
        return BOOL(1); // 跳过工具窗口
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if ctx.pid_set.contains(&pid) && ctx.found.is_none() {
        ctx.found = Some(hwnd);
        return BOOL(0); // Z 序最上的第一个命中即停止
    }
    BOOL(1)
}

/// 聚焦 PID 祖先链上进程拥有的可见顶层窗口
/// CLI 场景：链上含终端宿主（WindowsTerminal/mintty/conhost/Code.exe 等）
/// App 场景：链上含 ChatGPT.exe 主进程（内嵌 codex.exe 的父进程）
fn focus_window(pid_set: &HashSet<u32>) -> Result<(), String> {
    let mut ctx = EnumContext {
        pid_set: pid_set.clone(),
        found: None,
    };
    let lparam = LPARAM(&mut ctx as *mut EnumContext as isize);
    unsafe {
        let _ = EnumWindows(Some(enum_windows_proc), lparam);
    }
    let hwnd = ctx
        .found
        .ok_or_else(|| "未找到可聚焦的窗口（终端可能已关闭）".to_string())?;

    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return Ok(());
        }
    }
    // 降级：SwitchToThisWindow（Win32 标记为 deprecated 但仍可用）
    #[allow(deprecated)]
    unsafe {
        SwitchToThisWindow(hwnd, true);
    }
    Ok(())
}

/// 聚焦指定进程所在终端 / 应用窗口（focus_session IPC 入口）
pub fn focus_window_for_pid(pid: u32) -> Result<(), String> {
    let system = sysinfo::System::new_all();
    let ancestors = collect_ancestor_pids(&system, pid);
    focus_window(&ancestors)
}

#[cfg(test)]
mod tests {
    use super::collect_ancestor_pids_with;
    use std::collections::HashSet;

    #[test]
    fn collects_chain_until_no_parent() {
        // 5 -> 3 -> 1 -> 无父
        let set = collect_ancestor_pids_with(5, |p| match p {
            5 => Some(3),
            3 => Some(1),
            _ => None,
        });
        assert_eq!(set, HashSet::from([5, 3, 1]));
    }

    #[test]
    fn stops_on_cycle() {
        // 7 -> 8 -> 7（环），不得死循环
        let set = collect_ancestor_pids_with(7, |p| if p == 7 { Some(8) } else { Some(7) });
        assert_eq!(set, HashSet::from([7, 8]));
    }

    #[test]
    fn includes_self_when_no_parent() {
        let set = collect_ancestor_pids_with(42, |_| None);
        assert_eq!(set, HashSet::from([42]));
    }
}
