// Windows 窗口聚焦 v2 — 近祖优先 + shell 黑名单 + 消歧降级
//
// 跳转语义（CLI 类与 App 类统一实现）：
// - CLI 类（claude/codex/opencode 终端会话）：agent PID → 父链从近到远扫描（跳过 shell 黑名单）
//   → 第一个拥有可见顶层窗口的祖先 = 终端宿主；多窗口时按 marker → 标题打分 → 选择器消歧
//   典型链: claude.exe → pwsh.exe → OpenConsole.exe(无窗口) → WindowsTerminal.exe(单进程多窗口!)
// - App 类（ChatGPT 内嵌 Codex）：codex.exe → 近祖 ChatGPT.exe（恰 1 个可见窗口）→ 直接聚焦

use std::collections::HashMap;

/// 沿父进程链收集 PID 序列（含起始 PID 自身），保持插入顺序即近→远
/// 遇到环（重复 PID）或父进程缺失即停止；64 层防御异常深链。父进程查询由闭包注入便于单测
fn collect_ancestor_pids_with(pid: u32, mut parent_of: impl FnMut(u32) -> Option<u32>) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = pid;
    for _ in 0..64 {
        if chain.contains(&current) {
            break; // 环
        }
        chain.push(current);
        match parent_of(current) {
            Some(p) => current = p,
            None => break,
        }
    }
    chain
}

/// 收集指定进程的祖先链 PID 序列（含自身，近→远有序）
fn collect_ancestor_pids(system: &sysinfo::System, pid: u32) -> Vec<u32> {
    collect_ancestor_pids_with(pid, |p| {
        system
            .process(sysinfo::Pid::from_u32(p))
            .and_then(|proc| proc.parent())
            .map(|pp| pp.as_u32())
    })
}

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetForegroundWindow, ShowWindow, SwitchToThisWindow, GWL_EXSTYLE, SW_MINIMIZE,
    SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// 不可作为跳转宿主的系统 shell / 服务进程（其窗口与目标会话无关，
/// 例如 explorer.exe 拥有的文件资源管理器窗口会抢命中）
const SHELL_BLACKLIST: &[&str] = &[
    "explorer.exe",
    "sihost.exe",
    "svchost.exe",
    "ctfmon.exe",
    "runtimebroker.exe",
    "applicationframehost.exe",
    "searchhost.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "taskhostw.exe",
    "dwm.exe",
];

/// 工具认领关键词：窗口标题（不区分大小写）命中某工具任一关键词 → 该窗口被视为该工具的。
/// opencode 的终端标题是缩写 "OC | <会话标题>"，故含别名。
const TOOL_CLAIM_KEYWORDS: &[(&str, &[&str])] = &[
    ("claude", &["claude"]),
    ("codex", &["codex"]),
    ("opencode", &["opencode", "oc |"]),
    ("openclaw", &["openclaw"]),
];

/// 判定窗口标题被哪个工具认领；命中多个工具（罕见）视为中立返回 None
fn claim_owner(title: &str) -> Option<&'static str> {
    let t = title.to_lowercase();
    let owners: Vec<&str> = TOOL_CLAIM_KEYWORDS
        .iter()
        .filter(|(_, kws)| kws.iter().any(|k| t.contains(k)))
        .map(|(tool, _)| *tool)
        .collect();
    if owners.len() == 1 {
        Some(owners[0])
    } else {
        None
    }
}

/// 候选窗口（歧义时返回给前端选择器）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowCandidate {
    pub hwnd: isize,
    pub title: String,
    pub process: String,
    pub score: i32,
}

struct AllWindows {
    by_pid: HashMap<u32, Vec<(isize, String)>>,
}

unsafe extern "system" fn enum_all_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut AllWindows);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1); // 跳过不可见窗口
    }
    if (GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW.0 as i32) != 0 {
        return BOOL(1); // 跳过工具窗口
    }
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let mut buf = [0u16; 256];
    let len = GetWindowTextW(hwnd, &mut buf);
    let title = String::from_utf16_lossy(&buf[..len as usize]);
    ctx.by_pid.entry(pid).or_default().push((hwnd.0, title));
    BOOL(1)
}

/// 一次性枚举全部可见顶层窗口，按 PID 分组
fn all_windows() -> AllWindows {
    let mut ctx = AllWindows {
        by_pid: HashMap::new(),
    };
    let lparam = LPARAM(&mut ctx as *mut AllWindows as isize);
    unsafe {
        let _ = EnumWindows(Some(enum_all_proc), lparam);
    }
    ctx
}

/// 聚焦单个窗口：恢复最小化 → 置前，多级降级
fn force_foreground(hwnd_val: isize) -> bool {
    let hwnd = HWND(hwnd_val);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // 降级：SwitchToThisWindow（Win32 标记为 deprecated 但仍可用）
        #[allow(deprecated)]
        SwitchToThisWindow(hwnd, true);
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // 最后手段：最小化再恢复抖动，强制窗口进入前台
        let _ = ShowWindow(hwnd, SW_MINIMIZE);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd).as_bool()
    }
}

/// 跳转解析结果
pub enum FocusOutcome {
    Focused,
    Ambiguous(Vec<WindowCandidate>),
}

/// 解析并聚焦（CLI 与 App 统一入口，路径差异见模块头注释）
/// session_marker: 如 "MAM:1ba8e2f7"（hook 注入的标题标记，精确匹配用）
pub fn resolve_and_focus(
    system: &sysinfo::System,
    pid: u32,
    session_marker: Option<&str>,
    agent_keyword: Option<&str>,
    project_name: Option<&str>,
) -> Result<FocusOutcome, String> {
    let windows = all_windows();

    for ancestor in collect_ancestor_pids(system, pid) {
        // 黑名单进程的窗口与目标会话无关（explorer 的文件管理器窗口等），跳过
        let proc_name = system
            .process(sysinfo::Pid::from_u32(ancestor))
            .map(|p| p.name().to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if SHELL_BLACKLIST.contains(&proc_name.as_str()) {
            continue;
        }
        let Some(cands) = windows.by_pid.get(&ancestor) else {
            continue;
        };
        if cands.is_empty() {
            continue;
        }
        if cands.len() == 1 {
            let (hwnd, _) = cands[0];
            force_foreground(hwnd);
            return Ok(FocusOutcome::Focused);
        }
        // 多窗口消歧（Windows Terminal 单进程多窗口场景）
        // ① marker 精确匹配：标题含 hook 注入的 "MAM:<id 前 8 位>" 即锁定
        if let Some(marker) = session_marker {
            let hits: Vec<_> = cands
                .iter()
                .filter(|(_, t)| t.to_lowercase().contains(&marker.to_lowercase()))
                .collect();
            if hits.len() == 1 {
                force_foreground(hits[0].0);
                return Ok(FocusOutcome::Focused);
            }
        }
        // ② 标题打分：项目名 +2、工具名 +1，最高分唯一才锁定
        let score = |title: &str| -> i32 {
            let t = title.to_lowercase();
            let mut s = 0;
            if let Some(p) = project_name {
                if !p.is_empty() && t.contains(&p.to_lowercase()) {
                    s += 2;
                }
            }
            if let Some(a) = agent_keyword {
                if !a.is_empty() && t.contains(&a.to_lowercase()) {
                    s += 1;
                }
            }
            s
        };
        let mut scored: Vec<_> = cands.iter().map(|c| (score(&c.1), c)).collect();
        scored.sort_by_key(|c| std::cmp::Reverse(c.0));
        if scored.len() >= 2 && scored[0].0 > scored[1].0 && scored[0].0 > 0 {
            force_foreground(scored[0].1 .0);
            return Ok(FocusOutcome::Focused);
        }
        // ③ 候选池认领过滤：本工具认领的窗口 + 中立窗口；其他工具认领的窗口无条件排除。
        // 排序：先本工具认领、后中立，组内按现有打分降序。
        let agent = agent_keyword.unwrap_or_default().to_lowercase();
        let mut mine: Vec<&(i32, &(isize, String))> = Vec::new();
        let mut neutral: Vec<&(i32, &(isize, String))> = Vec::new();
        for item in scored.iter() {
            match claim_owner(&(item.1).1) {
                Some(owner) if owner != agent.as_str() => continue, // 他工具认领 → 排除
                Some(_) => mine.push(item),
                None => neutral.push(item),
            }
        }
        let candidates: Vec<WindowCandidate> = mine
            .into_iter()
            .chain(neutral)
            .map(|(s, (hwnd, title))| WindowCandidate {
                hwnd: *hwnd,
                title: title.clone(),
                process: proc_name.clone(),
                score: *s,
            })
            .collect();
        return Ok(FocusOutcome::Ambiguous(candidates));
    }
    Err("未找到可聚焦的窗口（终端可能已关闭）".to_string())
}

/// 前端选择器点选后按句柄聚焦
pub fn focus_hwnd(hwnd_val: isize) -> Result<(), String> {
    if force_foreground(hwnd_val) {
        Ok(())
    } else {
        Err("窗口聚焦被系统拒绝（窗口可能已关闭）".to_string())
    }
}

/// 兼容旧入口（mod.rs 的 focus_terminal_for_pid 内部使用）
pub fn focus_window_for_pid(pid: u32) -> Result<(), String> {
    let system = sysinfo::System::new_all();
    match resolve_and_focus(&system, pid, None, None, None) {
        Ok(FocusOutcome::Focused) => Ok(()),
        Ok(FocusOutcome::Ambiguous(_)) => Err("存在多个候选窗口，请重试以打开选择器".to_string()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{claim_owner, collect_ancestor_pids_with};

    #[test]
    fn collects_chain_until_no_parent() {
        // 5 -> 3 -> 1 -> 无父（顺序近→远）
        let chain = collect_ancestor_pids_with(5, |p| match p {
            5 => Some(3),
            3 => Some(1),
            _ => None,
        });
        assert_eq!(chain, vec![5, 3, 1]);
    }

    #[test]
    fn stops_on_cycle() {
        // 7 -> 8 -> 7（环），不得死循环，前两个元素为 [7, 8]
        let chain = collect_ancestor_pids_with(7, |p| if p == 7 { Some(8) } else { Some(7) });
        assert_eq!(&chain[..2], &[7, 8]);
        assert!(chain.len() <= 3);
    }

    #[test]
    fn includes_self_when_no_parent() {
        let chain = collect_ancestor_pids_with(42, |_| None);
        assert_eq!(chain, vec![42]);
    }

    #[test]
    fn claim_owner_matches_keywords() {
        assert_eq!(claim_owner("✳ Claude Code"), Some("claude"));
        assert_eq!(claim_owner("OC | 问候与开场"), Some("opencode"));
        assert_eq!(claim_owner("codex: working"), Some("codex"));
        // 中立：无命中
        assert_eq!(claim_owner("Windows PowerShell"), None);
        assert_eq!(claim_owner("MultiAgents-Manager"), None);
        // 多工具命中 → 中立
        assert_eq!(claim_owner("claude and codex"), None);
    }
}
