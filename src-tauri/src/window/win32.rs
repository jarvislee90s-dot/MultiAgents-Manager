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
/// opencode 的终端标题是缩写 "OC | <会话标题>"；codex App（ChatGPT 桌面版）窗口标题为 "ChatGPT"。
/// codex CLI 的终端标题是项目目录名（无 "codex" 字样），由"面板反推"（running_projects）补充认领。
const TOOL_CLAIM_KEYWORDS: &[(&str, &[&str])] = &[
    ("claude", &["claude"]),
    ("codex", &["codex", "chatgpt"]),
    ("opencode", &["opencode", "oc |"]),
    ("openclaw", &["openclaw"]),
];

/// 归一化窗口标题用于项目名比对：剥离 spinner 前缀（codex 运行时标题形态 "⠙ 项目名"，
/// 盲文区 U+2800–U+28FF）、去空白、转小写
fn normalize_title_for_project(title: &str) -> String {
    let stripped = title
        .trim()
        .trim_start_matches(|c: char| ('\u{2800}'..='\u{28FF}').contains(&c));
    stripped.trim().to_lowercase()
}

/// 计算 needle 在 haystack（均已空白归一化）中的最长可命中前缀字符数。
/// 终端渲染会消费 markdown 结构（粗体星号、列表符、波浪号），整串匹配常在尾部断裂，
/// 前缀评分可容忍渲染差异（实测断裂点在 16/40 处，阈值取 12）
fn longest_prefix_len(haystack_norm: &str, needle_norm: &str) -> usize {
    let chars: Vec<char> = needle_norm.chars().collect();
    let mut best = 0usize;
    for end in 1..=chars.len() {
        let cand: String = chars[..end].iter().collect();
        if haystack_norm.contains(&cand) {
            best = end;
        } else {
            break;
        }
    }
    best
}

/// 明显的空终端窗口标题（无 CLI 会话在跑），从候选池排除；
/// 池空回退全量时仍可能出现（保底有得选）
const IDLE_TERMINAL_TITLES: &[&str] = &[
    "windows powershell",
    "powershell",
    "pwsh",
    "命令提示符",
    "cmd",
    "cmd.exe",
    "windows terminal",
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

/// 空白归一化后取尾部 n 个字符（UIA 正文匹配用：终端渲染与 jsonl 原文的差异主要在空白与折行）
fn normalized_tail(s: &str, n: usize) -> String {
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= n {
        collapsed
    } else {
        chars[chars.len() - n..].iter().collect()
    }
}

/// 读取窗口的终端可见文本（UI Automation TextPattern，屏幕阅读器通道）。
/// 尝试顺序：根元素直取 → 查找 Document 类型后代（Windows Terminal 的 TermControl）。
/// COM 初始化失败 / 模式不可用 / 超时 → None（视为 miss，不报错）
fn read_window_text(hwnd_val: isize) -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationCondition, IUIAutomationElement,
        IUIAutomationTextPattern, TreeScope_Descendants, UIA_TextPatternId,
    };
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let co_initialized = hr.is_ok();
        let result = (|| -> Option<String> {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let root: IUIAutomationElement = automation.ElementFromHandle(HWND(hwnd_val)).ok()?;

            let try_text = |el: &IUIAutomationElement| -> Option<String> {
                let pattern = el
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .ok()?;
                let range = pattern.DocumentRange().ok()?;
                let text = range.GetText(-1).ok()?;
                let s = text.to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            };

            if let Some(s) = try_text(&root) {
                return Some(s);
            }
            // 遍历全部后代，逐个尝试 TextPattern（Windows Terminal 的 TermControl 携带
            // ControlType.Text 的正文；按类型过滤会漏/误判，TrueCondition 全量遍历最可靠）。
            // 同窗口存在窗口标题等装饰性短文本与终端正文并存，取最长者——
            // 终端可见正文远长于装饰性短文本（实测 WT 标题 len=3 vs 正文 len=366）
            let cond: IUIAutomationCondition = automation.CreateTrueCondition().ok()?;
            let arr = root.FindAll(TreeScope_Descendants, &cond).ok()?;
            let count = arr.Length().ok()?; // 循环外取一次：单次失败不应丢弃已收集结果
            let mut best: Option<String> = None;
            for i in 0..count {
                if let Ok(el) = arr.GetElement(i) {
                    if let Some(s) = try_text(&el) {
                        if best
                            .as_ref()
                            .is_none_or(|b| s.chars().count() > b.chars().count())
                        {
                            best = Some(s);
                        }
                    }
                }
            }
            best
        })();
        if co_initialized {
            CoUninitialize();
        }
        result
    }
}

/// 跳转解析结果
pub enum FocusOutcome {
    Focused,
    Ambiguous(Vec<WindowCandidate>),
}

/// 解析并聚焦（CLI 与 App 统一入口，路径差异见模块头注释）
/// session_marker: 如 "MAM:1ba8e2f7"（hook 注入的标题标记，精确匹配用）
/// running_projects: 当前运行会话的 (工具id, 项目名) 列表——用于"面板反推"排除
/// 其他工具的终端窗口（codex 终端标题=项目名，无 "codex" 关键词可认领）
pub fn resolve_and_focus(
    system: &sysinfo::System,
    pid: u32,
    session_marker: Option<&str>,
    agent_keyword: Option<&str>,
    project_name: Option<&str>,
    last_message: Option<&str>,
    running_projects: &[(String, String)],
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
        // ①5 UIA 正文匹配：候选窗口的终端可见文本含卡片最新消息尾部（归一化后）
        // 且唯一命中 → 锁定。点击跳转通常发生在任务刚结束（最终回复仍在屏幕上），命中率高。
        if let Some(msg) = last_message {
            if !msg.is_empty() {
                let tail = normalized_tail(msg, 40);
                // 并行读取（每窗独立线程，COM 各自初始化），总预算 800ms。
                // 实测单窗全缓冲读取 127-400ms：顺序读最坏超预算，并行墙钟≈最慢单窗。
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
                let mut rxs: Vec<(isize, std::sync::mpsc::Receiver<Option<String>>)> = Vec::new();
                for (h, _) in cands.iter().take(8) {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let hwnd = *h;
                    std::thread::spawn(move || {
                        let _ = tx.send(read_window_text(hwnd));
                    });
                    rxs.push((hwnd, rx));
                }
                let mut scored_hits: Vec<(isize, usize)> = Vec::new();
                for (h, rx) in rxs {
                    // 超预算即放弃等待（挂起线程泄漏有界 ≤8/次点击，已知权衡）
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    if let Ok(Some(text)) = rx.recv_timeout(remaining) {
                        // 前缀评分代替整串匹配：终端渲染消费 markdown 结构（粗体星号、
                        // 列表符），整串常在尾部断裂；最长命中前缀 ≥12 且唯一最高 → 锁定
                        let hn: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let s = longest_prefix_len(&hn, &tail);
                        if s >= 12 {
                            scored_hits.push((h, s));
                        }
                    }
                }
                if !scored_hits.is_empty() {
                    scored_hits.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
                    let unique_top = scored_hits.len() == 1 || scored_hits[0].1 > scored_hits[1].1;
                    if unique_top {
                        force_foreground(scored_hits[0].0);
                        return Ok(FocusOutcome::Focused);
                    }
                }
            }
        }
        // ①6 项目名精确匹配：标题（剥离 spinner 前缀后）恰为项目目录名（codex CLI 终端
        // 标题形态：空闲=项目名，运行="⠙ 项目名"）且唯一 → 锁定
        if let Some(p) = project_name {
            if !p.is_empty() {
                let pl = p.trim().to_lowercase();
                let exact: Vec<isize> = cands
                    .iter()
                    .filter(|(_, t)| normalize_title_for_project(t) == pl)
                    .map(|(h, _)| *h)
                    .collect();
                if exact.len() == 1 {
                    force_foreground(exact[0]);
                    return Ok(FocusOutcome::Focused);
                }
            }
        }
        // ② 标题打分：项目名 +2、工具名 +1，最高分唯一才锁定。
        // 打分与候选列表共用"认领过滤候选池"：他工具认领的窗口无条件出局
        // （防止打分层凭项目名错锁他工具窗口）；池空（目标工具可能一个窗口都不在）
        // 时回退全量防御，保证仍有得选；agent 为空（兼容入口）时不过滤
        let agent = agent_keyword.unwrap_or_default().to_lowercase();
        let pool: Vec<&(isize, String)> = {
            let filtered: Vec<&(isize, String)> = cands
                .iter()
                .filter(|(_, t)| {
                    let tl = t.trim().to_lowercase();
                    // 排除：他工具认领（关键词）、明显空终端、其他工具运行会话的项目名窗口
                    // （codex 终端标题=项目名无 "codex" 字样，由面板数据反推认领）
                    if !claim_owner(t).is_none_or(|o| o == agent.as_str()) {
                        return false;
                    }
                    if IDLE_TERMINAL_TITLES.contains(&tl.as_str()) {
                        return false;
                    }
                    let nt = normalize_title_for_project(t);
                    if running_projects.iter().any(|(a, p)| {
                        a != agent.as_str() && !p.is_empty() && nt == p.trim().to_lowercase()
                    }) {
                        return false;
                    }
                    true
                })
                .collect();
            if filtered.is_empty() {
                cands.iter().collect()
            } else {
                filtered
            }
        };
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
        let mut scored: Vec<_> = pool.into_iter().map(|c| (score(&c.1), c)).collect();
        scored.sort_by_key(|c| std::cmp::Reverse(c.0));
        if scored.len() >= 2 && scored[0].0 > scored[1].0 && scored[0].0 > 0 {
            force_foreground(scored[0].1 .0);
            return Ok(FocusOutcome::Focused);
        }
        // ③ 候选排序：先本工具认领、后中立，组内按打分降序（他工具已在池构建时排除；
        // 池为全量回退时此处的排除分支兜底）
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
    match resolve_and_focus(&system, pid, None, None, None, None, &[]) {
        Ok(FocusOutcome::Focused) => Ok(()),
        Ok(FocusOutcome::Ambiguous(_)) => Err("存在多个候选窗口，请重试以打开选择器".to_string()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        claim_owner, collect_ancestor_pids_with, longest_prefix_len, normalize_title_for_project,
    };

    #[test]
    fn normalize_title_strips_spinner_prefix() {
        // codex 运行态标题 "⠙ 项目名"（盲文 spinner）与空闲态 "项目名" 归一化后一致
        assert_eq!(
            normalize_title_for_project("⠙ MinerU_Convert"),
            "mineru_convert"
        );
        assert_eq!(
            normalize_title_for_project("MinerU_Convert"),
            "mineru_convert"
        );
        assert_eq!(
            normalize_title_for_project("  local-datasource "),
            "local-datasource"
        );
    }

    #[test]
    fn longest_prefix_tolerates_render_diff() {
        // 消息尾 40 字在终端渲染后尾部断裂（markdown 符号被消费），前缀 16 字仍可命中
        let hay = "前面一大段内容 项目。 需要我帮你做什么吗 比如查看代码";
        let needle =
            "项目。 需要我帮你做什么吗 比如查看代码、调试问题、或者继续某个功能开发都可以。";
        assert_eq!(longest_prefix_len(hay, needle), 20);
        assert_eq!(longest_prefix_len("完全不相关", needle), 0);
        assert_eq!(longest_prefix_len("整串都在 完全命中", "完全命中"), 4);
    }

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

    #[test]
    fn normalized_tail_collapses_whitespace() {
        use super::normalized_tail;
        assert_eq!(normalized_tail("你好  世界\n\n下一行", 3), "下一行");
        assert_eq!(normalized_tail("short", 40), "short");
        // 长文本取尾部 n 个字符
        let long = "a ".repeat(60);
        assert_eq!(normalized_tail(&long, 10).chars().count(), 10);
    }
}
