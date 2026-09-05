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
    EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, SwitchToThisWindow, GWL_EXSTYLE,
    SW_MINIMIZE, SW_RESTORE, WS_EX_TOOLWINDOW,
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
    ("kimi", &["kimi"]),
    // WorkBuddy 桌面 APP（Electron）窗口标题即 "WorkBuddy"，缺失会导致
    // 多窗口消歧无法认领、无谓落入 Ambiguous 选择器（spec W2/W7 Windows 侧）
    ("workbuddy", &["workbuddy"]),
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
    /// UIA 正文前缀命中长度（部分命中只作选择器排序依据，不作自动锁定）
    pub uia_prefix: i32,
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
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    let hwnd = HWND(hwnd_val);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // AttachThreadInput 前台强抢：把当前线程挂到前台线程输入队列，取得置前资格。
        // 对症：浮窗 focusable(false) 点击不激活本进程（非前台发起）、目标为提权窗口（ChatGPT 链）
        let fg = GetForegroundWindow();
        if fg.0 != 0 {
            let fg_tid = GetWindowThreadProcessId(fg, None);
            let cur_tid = GetCurrentThreadId();
            if fg_tid != 0
                && fg_tid != cur_tid
                && AttachThreadInput(cur_tid, fg_tid, true).as_bool()
            {
                let ok = SetForegroundWindow(hwnd).as_bool();
                let _ = AttachThreadInput(cur_tid, fg_tid, false);
                if ok {
                    return true;
                }
            }
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

/// 锁定并聚焦：置前失败返回错误（不再静默报成功，错误可传到前端 toast）
fn try_lock(hwnd_val: isize) -> Result<FocusOutcome, String> {
    if force_foreground(hwnd_val) {
        Ok(FocusOutcome::Focused)
    } else {
        Err("窗口聚焦被系统拒绝（目标窗口可能提权或被系统锁定）".to_string())
    }
}

/// 空白归一化：连续空白/换行压为单空格（UIA 正文匹配的预处理，两侧同规）
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 空壳终端判定（按内容而非标题——"标题=项目名"有 codex 运行态/PowerShell 停留/claude 长任务
/// 改名三种来源不可信）。依据：TUI 会话（claude REPL 等）用备用缓冲区，文档文本以 REPL UI 开头
/// （实测样本 "⏸ manual mode on…"）；空 PowerShell 停留主缓冲区，头部是控制台 banner
/// （实测样本 "Windows PowerShell…"）
fn is_shell_console(text: &str) -> bool {
    let head: String = text.chars().take(120).collect::<String>().to_lowercase();
    head.contains("windows powershell") || head.contains("命令提示符")
}

/// 并行读取候选窗口文本（每窗独立线程，COM 各自初始化），800ms 总预算；
/// 32 为防御上限而非 Z 序截断。超预算的线程放弃等待（挂起泄漏有界，已知权衡）
fn read_window_texts_parallel(cands: &[&(isize, String)]) -> HashMap<isize, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(800);
    let mut rxs = Vec::new();
    for (h, _) in cands.iter().take(32) {
        let (tx, rx) = std::sync::mpsc::channel();
        let hwnd = *h;
        std::thread::spawn(move || {
            let _ = tx.send(read_window_text(hwnd));
        });
        rxs.push((hwnd, rx));
    }
    let mut out = HashMap::new();
    for (h, rx) in rxs {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Ok(Some(t)) = rx.recv_timeout(remaining) {
            out.insert(h, t);
        }
    }
    out
}

/// 单幸存者演绎锁定：候选仅剩一个且未被他工具认领时返回它（否则 None）
fn single_survivor<'a>(cands: &[&'a (isize, String)], agent: &str) -> Option<&'a (isize, String)> {
    if cands.len() == 1 && claim_owner(&cands[0].1).is_none_or(|o| o == agent) {
        Some(cands[0])
    } else {
        None
    }
}

/// 硬排除 + 洋葱回退（返回幸存窗口，保序）：
/// L1 他工具认领（全部已知标题形态）→ L2 空终端标题 → L3 面板反推（标题规范化后
/// ==其他工具运行中项目名，覆盖 ⠙spinner/停留目录/长任务改名三形态）。
/// 三层全开排空则从最软层逐层撤销（L3 → L2），仍空回退全量（连 L1 都排空=极端异常）
fn hard_survivors<'a>(
    cands: &[&'a (isize, String)],
    agent: &str,
    running_projects: &[(String, String)],
) -> Vec<&'a (isize, String)> {
    fn filter_ss<'a>(
        cands: &[&'a (isize, String)],
        agent: &str,
        running_projects: &[(String, String)],
        use_l3: bool,
        use_l2: bool,
    ) -> Vec<&'a (isize, String)> {
        let l1 = |t: &str| claim_owner(t).is_some_and(|o| o != agent);
        let l2 = |t: &str| IDLE_TERMINAL_TITLES.contains(&t.trim().to_lowercase().as_str());
        let l3 = |t: &str| {
            let nt = normalize_title_for_project(t);
            running_projects
                .iter()
                .any(|(a, p)| a.as_str() != agent && !p.is_empty() && nt == p.trim().to_lowercase())
        };
        cands
            .iter()
            .filter(|(_, t)| !l1(t) && !(use_l2 && l2(t)) && !(use_l3 && l3(t)))
            .copied()
            .collect()
    }
    let s = filter_ss(cands, agent, running_projects, true, true);
    if !s.is_empty() {
        return s;
    }
    let s = filter_ss(cands, agent, running_projects, false, true); // 撤 L3（最软层），L1/L2 成果保留
    if !s.is_empty() {
        return s;
    }
    let s = filter_ss(cands, agent, running_projects, false, false); // 再撤 L2，L1 仍生效
    if !s.is_empty() {
        return s;
    }
    cands.to_vec() // 极端异常：全量回退
}

/// 空白归一化后取尾部 n 个字符（UIA 正文匹配用：终端渲染与 jsonl 原文的差异主要在空白与折行）
fn normalized_tail(s: &str, n: usize) -> String {
    let collapsed: String = collapse_ws(s);
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
            return try_lock(cands[0].0);
        }
        // ===== 多窗口消歧：硬逻辑先行 + 排除制 + 洋葱回退 =====
        // ① marker 精确匹配（hook 注入标题标记；通道当前 no-go，代码保留）
        if let Some(marker) = session_marker {
            let hits: Vec<_> = cands
                .iter()
                .filter(|(_, t)| t.to_lowercase().contains(&marker.to_lowercase()))
                .collect();
            if hits.len() == 1 {
                return try_lock(hits[0].0);
            }
        }
        // ② 硬排除（L1 认领/L2 空终端/L3 面板反推，洋葱回退）→ 幸存 1 且非他工具认领 → 演绎锁定
        let agent = agent_keyword.unwrap_or_default().to_lowercase();
        let cand_refs: Vec<&(isize, String)> = cands.iter().collect();
        let survivors = hard_survivors(&cand_refs, &agent, running_projects);
        if let Some((hwnd, _)) = single_survivor(&survivors, &agent) {
            return try_lock(*hwnd);
        }
        // ③ UIA 阶段（仅幸存 ≥2 才读，候选已被硬排除缩小）
        let msg = last_message.unwrap_or_default();
        let tail = if msg.is_empty() {
            String::new()
        } else {
            normalized_tail(msg, 40)
        };
        let texts: HashMap<isize, String> = if tail.is_empty() {
            HashMap::new()
        } else {
            read_window_texts_parallel(&survivors)
        };
        // L4 壳窗口排除（内容级；未读到文本的窗口不按壳排除）；排空只撤 L4、硬排除成果保留
        let after_shell: Vec<&(isize, String)> = {
            let ns: Vec<&(isize, String)> = survivors
                .iter()
                .filter(|(h, _)| texts.get(h).map(|t| !is_shell_console(t)).unwrap_or(true))
                .copied()
                .collect();
            if ns.is_empty() {
                survivors.clone()
            } else {
                ns
            }
        };
        if let Some((hwnd, _)) = single_survivor(&after_shell, &agent) {
            return try_lock(*hwnd);
        }
        // ④ 完整尾串包含且唯一 → 锁定（最强正向证据：会话正文完整出现在哪个窗口哪个就是它；
        //    渲染差异导致整串断裂时不自动锁定，交选择器）
        if !tail.is_empty() {
            let full: Vec<isize> = after_shell
                .iter()
                .filter(|(h, _)| {
                    texts
                        .get(h)
                        .map(|t| collapse_ws(t).contains(&tail))
                        .unwrap_or(false)
                })
                .map(|(h, _)| *h)
                .collect();
            if full.len() == 1 {
                return try_lock(full[0]);
            }
        }
        // ⑤ 选择器：按 UIA 前缀命中长度降序 → 标题分（项目名+2/工具名+1）降序；交人工
        let title_score = |title: &str| -> i32 {
            let t = title.to_lowercase();
            let mut sc = 0;
            if let Some(pn) = project_name {
                if !pn.is_empty() && t.contains(&pn.to_lowercase()) {
                    sc += 2;
                }
            }
            if !agent.is_empty() && t.contains(&agent) {
                sc += 1;
            }
            sc
        };
        let mut candidates: Vec<WindowCandidate> = after_shell
            .iter()
            .map(|(h, t)| WindowCandidate {
                hwnd: *h,
                title: t.clone(),
                process: proc_name.clone(),
                score: title_score(t),
                uia_prefix: texts
                    .get(h)
                    .map(|t2| longest_prefix_len(&collapse_ws(t2), &tail) as i32)
                    .unwrap_or(0),
            })
            .collect();
        candidates.sort_by(|a, b| b.uia_prefix.cmp(&a.uia_prefix).then(b.score.cmp(&a.score)));
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

/// pid 失效兜底（W2）：枚举该工具 App 进程的可见顶层窗口并聚焦。
/// Electron 单实例应用直接重拉 exe 亦可聚焦，但窗口枚举不引入子进程，优先采用
pub fn reactivate_tool_app(
    system: &sysinfo::System,
    agent_type: Option<&str>,
) -> Result<(), String> {
    // P2-4（issue #34）：宿主判定统一走 monitor::host::is_host_process（与前台验证
    // 同口径）。原整路径子串匹配（exe.contains("chatgpt")）会误命中路径含关键词的
    // 无关进程（如 D:\tools\chatgpt-clone\app.exe）；is_host_process 按可执行文件名
    // 判定，并排除会话进程（codebuddy）与内嵌框架进程，聚焦的是真正的宿主窗口
    let Some(tool_id) = agent_type
        .map(|a| a.to_lowercase())
        .filter(|t| matches!(t.as_str(), "workbuddy" | "codex"))
    else {
        return Err("未知工具，无法兜底激活".to_string());
    };
    let wins = all_windows();
    // AllWindows.by_pid：pid → 该进程名下全部可见顶层窗口 (hwnd, title)，按 pid 分组迭代
    for (pid, hwnds) in wins.by_pid.iter() {
        let Some(proc) = system.process(sysinfo::Pid::from_u32(*pid)) else {
            continue;
        };
        let exe = proc
            .exe()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !crate::monitor::host::is_host_process(&exe, &tool_id) {
            continue;
        }
        for (hwnd, _) in hwnds {
            if force_foreground(*hwnd) {
                return Ok(());
            }
        }
    }
    Err("未找到该工具的可聚焦窗口".to_string())
}

// ===== P1-2 B：深度链接派发后的前台验证（仅 Windows） =====
//
// 背景：URL 协议派发是异步无回执交互，spawn 成功 ≠ 路由成功。实测（2026-09-04）：
// 派发瞬间第三方窗口会闪现 ~200ms（如 Weixin 短暂置前）后才稳定到目标 APP——
// 因此验证判定必须以窗口期终点为准，不能见到非目标窗口即判失败。
// 机制：轮询 GetForegroundWindow → GetWindowThreadProcessId → 该 pid 进程是否属于
// 目标工具宿主（复用 monitor/host.rs::is_host_process 语义），最后两次轮询连续命中
// 才判成功（消除闪现噪声）。UIA 不可用于 Electron（Chromium 不暴露文本树，实测），
// 故只用前台窗口归属判定，不读内容。

/// 前台验证状态机（纯函数，可测）：喂入「前台窗口归属工具?」样本序列，判定是否成功。
/// 规则：满 2 次轮询连续命中 → 成功；序列结束仍未达 2 连中 → 失败（超时）。
/// last 连续命中计数与 seen_success 状态由此状态机维护，闪现（非目标一次）不会
/// 打断已达成的 2 连中（窗口期终点语义）。
struct ForegroundVerifyState {
    consecutive_hits: u32,
    done: bool,
    success: bool,
}

fn foreground_verify_step(state: &mut ForegroundVerifyState, is_target: bool) {
    if state.done {
        return;
    }
    if is_target {
        state.consecutive_hits += 1;
        if state.consecutive_hits >= 2 {
            state.done = true;
            state.success = true;
        }
    } else {
        state.consecutive_hits = 0;
    }
}

/// 驱动状态机跑完整样本序列（测试入口）
#[cfg(test)]
fn foreground_verify_with_samples(samples: &[bool]) -> bool {
    let mut state = ForegroundVerifyState {
        consecutive_hits: 0,
        done: false,
        success: false,
    };
    for &s in samples {
        foreground_verify_step(&mut state, s);
    }
    state.success
}

/// 目标进程 pid 是否属于工具宿主：exe 路径小写后经 is_host_process 判定。
/// system 复用调用方快照（不另起全量扫描）；新鲜度由调用方维护
/// （P2-1：验证轮询每轮刷新，兜底前由 commands/session.rs 重刷）
fn foreground_pid_is_tool(system: &sysinfo::System, pid: u32, tool_id: &str) -> bool {
    system
        .process(sysinfo::Pid::from_u32(pid))
        .and_then(|p| p.exe())
        .map(|e| {
            crate::monitor::host::is_host_process(&e.to_string_lossy().to_lowercase(), tool_id)
        })
        .unwrap_or(false)
}

/// 深度链接派发后前台验证（Windows）：轮询前台窗口归属，最后两次连续命中该工具宿主
/// → true。timeout 内未达成 → false（调用方落回保底聚焦、不标已读）。
/// interval 轮询间隔；判定以窗口期终点为准（见模块注释：闪现噪声不误判）。
/// P2-1（issue #34）：每轮轮询前刷新进程表——深链可能冷启动宿主，新 pid 不在派发前
/// 的快照里，持旧快照轮询恒 miss（APP 已在前台仍整链报失败）；仅刷进程+exe（判定
/// 只消费 exe），判定标准本身不变
pub fn verify_foreground_tool(
    system: &mut sysinfo::System,
    tool_id: &str,
    timeout_ms: u64,
    interval_ms: u64,
) -> bool {
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut state = ForegroundVerifyState {
        consecutive_hits: 0,
        done: false,
        success: false,
    };
    while Instant::now() < deadline {
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::Always),
        );
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0 != 0 {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let is_target = pid != 0 && foreground_pid_is_tool(system, pid, tool_id);
                foreground_verify_step(&mut state, is_target);
                if state.done {
                    return state.success;
                }
            }
        }
        sleep(Duration::from_millis(interval_ms));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        claim_owner, collapse_ws, collect_ancestor_pids_with, hard_survivors, is_shell_console,
        longest_prefix_len, normalize_title_for_project,
    };

    #[test]
    fn reactivate_rejects_tools_without_app_form() {
        // P2-4 门：无 APP 形态/未知工具在窗口枚举前即拒绝（不得触碰 Win32 枚举）；
        // 宿主匹配口径统一见 reactivate_tool_app（is_host_process，host.rs 已有测试）
        assert!(super::reactivate_tool_app(&sysinfo::System::new(), None).is_err());
        assert!(super::reactivate_tool_app(&sysinfo::System::new(), Some("claude")).is_err());
    }

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
        // WorkBuddy 窗口（App 标题即 "WorkBuddy"）必须被 workbuddy 认领，
        // 否则多窗口消歧无法锁定 WorkBuddy、误入 Ambiguous 选择器
        assert_eq!(claim_owner("WorkBuddy"), Some("workbuddy"));
        assert_eq!(claim_owner("WorkBuddy - 项目设置"), Some("workbuddy"));
        // 中立：无命中
        assert_eq!(claim_owner("Windows PowerShell"), None);
        assert_eq!(claim_owner("MultiAgents-Manager"), None);
        // 多工具命中 → 中立
        assert_eq!(claim_owner("claude and codex"), None);
    }

    #[test]
    fn collapse_ws_normalizes() {
        assert_eq!(
            collapse_ws(
                "你好  世界

下一行"
            ),
            "你好 世界 下一行"
        );
        assert_eq!(collapse_ws("  spaced 	 out "), "spaced out");
    }

    #[test]
    fn shell_console_detected_by_banner_head() {
        // 实测样本：壳窗口头部是控制台 banner（主缓冲区）；TUI 会话头部是 REPL UI（备用缓冲区）
        assert!(is_shell_console(
            "Windows PowerShell
版权所有 (C) Microsoft Corporation。
PS E:/proj> "
        ));
        assert!(is_shell_console(
            "命令提示符
Microsoft Windows [版本 10.0.26200]"
        ));
        assert!(!is_shell_console(
            "⏸ manual mode on · ? for shortcuts · ← for agents"
        ));
        assert!(!is_shell_console("一段不含 banner 的长会话正文……"));
    }

    #[test]
    fn hard_survivors_onion_fallback_order() {
        let claude_win = (1isize, "✳ Claude Code".to_string()); // L1 排除（codex 卡视角）
        let idle_win = (2isize, "Windows PowerShell".to_string()); // L2 排除
        let panel_win = (3isize, "华为投资".to_string()); // L3 排除（面板反推：claude 正跑该项目）
        let target_win = (4isize, "⠙ 我的项目".to_string()); // 目标（不被任何层排除）
        let running = vec![("claude".to_string(), "华为投资".to_string())];

        // 常规：三层排除后仅剩目标 → 演绎锁定路径
        let cands: Vec<&(isize, String)> = vec![&claude_win, &idle_win, &panel_win, &target_win];
        let s = hard_survivors(&cands, "codex", &running);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].0, 4);

        // L3 排空（只剩 L1/L2/L3 排除对象）→ 撤 L3：L3 窗口回来，L1/L2 成果保留
        let all_l3: Vec<&(isize, String)> = vec![&claude_win, &idle_win, &panel_win];
        let s2 = hard_survivors(&all_l3, "codex", &running);
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].0, 3);

        // 连 L2 撤掉后仍空（只剩 L1 排除对象）→ 全量回退（极端异常）
        let only_l1: Vec<&(isize, String)> = vec![&claude_win];
        let s3 = hard_survivors(&only_l1, "codex", &running);
        assert_eq!(s3.len(), 1);
        assert_eq!(s3[0].0, 1);
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

    // ---- P1-2 B：前台验证状态机（喂样本序列断言判定） ----

    mod foreground_verify {
        use super::super::foreground_verify_with_samples;

        #[test]
        fn two_consecutive_hits_success() {
            assert!(foreground_verify_with_samples(&[true, true]));
            assert!(foreground_verify_with_samples(&[false, true, true]));
            assert!(foreground_verify_with_samples(&[true, false, true, true]));
        }

        #[test]
        fn single_hit_then_other_window_fails() {
            // 目标窗口只出现一次即被第三方窗口顶掉（闪现噪声，无后续确认）→ 失败
            assert!(!foreground_verify_with_samples(&[true, false, false]));
            assert!(!foreground_verify_with_samples(&[false]));
            assert!(!foreground_verify_with_samples(&[true]));
        }

        #[test]
        fn flash_noise_before_stable_target_still_succeeds() {
            // 实测序列（附录 A）：+203ms Weixin 瞬时闪现 → +1235ms WorkBuddy 稳定前台。
            // 窗口期终点为准：闪现（非目标）不判失败，最后两次连续命中才成功
            assert!(foreground_verify_with_samples(&[
                false, false, false, true, true
            ]));
            assert!(foreground_verify_with_samples(&[true, false, true, true]));
        }

        #[test]
        fn target_then_never_confirmed_fails() {
            // 目标出现过但未达 2 连中（如又被顶掉）→ 失败
            assert!(!foreground_verify_with_samples(&[
                true, false, true, false, true, false
            ]));
        }

        #[test]
        fn empty_sequence_fails() {
            assert!(!foreground_verify_with_samples(&[]));
        }
    }
}
