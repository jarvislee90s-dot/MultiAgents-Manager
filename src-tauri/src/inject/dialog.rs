//! 通用 N 选项审批对话框屏读解析（批次丙 T5）。
//!
//! # 要解决的问题（图2/图3 实证）
//!
//! 审批对话框是 **N 选一**，而既有审批映射表只建模二元 approve/reject——三类
//! 真实对话框全部被降级成二元卡，用户盲发数字碰运气（实测：点「允许」注入 "1"
//! 恰好命中推荐项）：
//!
//! - claude 计划批准：`1. Yes, and use auto mode` / `2. Yes, manual` /
//!   `3. Tell Claude what to do differently`；
//! - codex `Implement this plan?`：1/2/3；
//! - kimi `Ready to build`：1=Approve / 2=Reject / 3=Revise。
//!
//! # 解析口径（本模块的唯一职责）
//!
//! 屏读（[`crate::inject::windows_console::read_screen_window`]）拿到可见窗口的
//! 逐行文本 → 本模块把「编号选项行」解析成结构化选项表 → 端点下发给前端渲染
//! 编号按钮（点按 → 注入对应数字键）。
//!
//! **解析失败必须降级**（计划书红线 3）：不猜选项、不盲出键——返回 None，端点
//! 回落现二元卡 + 防重警示。happy「兜底渲染」原则：会话永不被前端卡死。
//!
//! # 行模式（实测形态族）
//!
//! 选项行 = `^\s*(\d+)[.)]\s+(.+)$`（编号 + 点或右括号 + 空白 + 文本）。三类
//! 对话框的选项行都符合（claude/codex/kimi 的 TUI 都用 `N. ` 前缀）。
//! 解析器取**连续编号序列**（1,2,3,…）的最长一簇——TUI 正文里偶然出现的
//! 「1. 某段列表」不会凑出连续序列（实测正文列表罕见从 1 连续编号且紧跟对话选项
//! 语义），且本解析只用于「已经在等待审批的会话」，误判面天然收窄。
//!
//! 超 9 个选项 → 不出键（数字键域上限 '1'..'9'，与 [`super::question::digit_key`]
//! 同口径）：返回 None 让端点降级。
//!
//! # 不做（边界）
//!
//! 不做各工具像素级复刻（计划书 §2.5）；不解析对话框**语义**（哪项是「批准」）——
//! 只做「编号 + 文本」的结构化，语义由用户从文本判断（这正是 N 选一的修复目标：
//! 把真实选项文本交给用户，而不是代它猜）。
//!
//! # macOS
//!
//! 屏读是 Windows 能力；macOS 无屏读 → [`parse_dialog_options`] 的调用方（端点）
//! 拿不到屏幕文本 → 自然降级二元卡（红线 4 同款语义）。本模块本身纯函数、跨平台
//! 可测（文本输入 → 选项表输出）。

/// 单个对话框选项（编号 + 文本原文）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogOption {
    /// 编号（屏幕上显示的 1 起数字）
    pub number: u32,
    /// 选项文本原文（可能含该工具追加的说明；不裁剪、不改写——把终端所见原样
    /// 交给用户是 T5 的目标）
    pub label: String,
    /// 该行是否带**光标标记**（`›`/`❯`/`▶`/`>`）——即 TUI 当前高亮项。
    ///
    /// **R1-2 起本字段是导航确认的必需输入**：对话框的 Enter 提交的是**高亮行**
    /// 而非「编号 = 用户点击项」，故必须知道起点在哪一行才能算出步进数（见
    /// [`navigation_sequence`]）。实测（2026-09-21）：claude 计划批准框 Enter
    /// 提交高亮行，`↓×k` 使高亮前进 k 行（**循环**：3 行时 ↓ 从 3 回到 1）。
    pub highlighted: bool,
}

/// 光标标记集合（实机三类，见 [`parse_option_line`] 注）
const CURSOR_MARKERS: [char; 4] = ['\u{203a}', '\u{276f}', '\u{25b6}', '>'];

/// 数字键域上限（与 [`super::question::digit_key`] 同口径：'1'..'9'）
pub const MAX_DIALOG_OPTIONS: usize = 9;

/// 剥掉行首的**前导空白 + 光标标记**，返回 (剩余文本, 是否带光标标记)。
///
/// 光标标记必须出现在**编号之前**（前导区）才算高亮——`› 1. Yes` 是高亮项，
/// `1. › 不是` 不是（标记在编号之后）。
///
/// **丁T4 起抽为 pub(crate) 单点**：模式权限菜单（`inject::mode::locate_menu_items`）
/// 也要做同一件事（菜单项同样以光标标记标出当前档），而「光标标记集合」与「标记
/// 必须在编号前」这两个判据必须**只有一份**——两处各写一遍就是本仓既往的
/// 「同一判据两处实现 → 口径漂移」老路。
pub(crate) fn strip_cursor_marker(line: &str) -> (&str, bool) {
    let mut idx = 0usize;
    let mut highlighted = false;
    for c in line.chars() {
        if c.is_whitespace() {
            idx += c.len_utf8();
        } else if CURSOR_MARKERS.contains(&c) {
            highlighted = true;
            idx += c.len_utf8();
        } else {
            break;
        }
    }
    (&line[idx..], highlighted)
}

/// 判定一行是否是「编号选项行」并抽出 (编号, 文本, 是否高亮)。行模式：
/// `^[\s›❯>]*(\d+)\s*[.)]\s*(.+)$`——允许前导空白**与光标标记**、编号后跟
/// `.` 或 `)`、其后至少一个空白（防把 `1.5x` 这类数字当选项）。
///
/// **光标标记为什么必须剥**（2026-09-21 实机探测抓获的真实缺陷）：codex 的
/// `Implement this plan?` 对话框把**当前高亮项**渲染为 `› 1. Yes, ...`（U+203A），
/// 未高亮项是 `  2. ...`。原实现只 `trim_start()`（仅空白）→ 高亮项**永不匹配** →
/// 第一项编号缺失 → 连续簇从 2 起 → 解析返回 None → 整个对话框降级二元卡。
/// 实测证据：`%TEMP%\mam-probe-c3-20260921-150000\evidence\
/// screen-t5-codex-implement-before.txt`（行 25 `› 1. Yes, implement this plan`）。
/// claude 的同类标记是 `❯ `（U+276F，见同目录 screen-t5-claude-plan-before.txt），
/// kimi 是 `▶ `（U+25B6）。`>` 是兜底形态（部分 TUI 用 ASCII 箭头）。
///
/// **返回值第三项=高亮**（R1-2 起）：行首出现光标标记即该选项是 TUI 当前高亮项。
/// 这是导航确认的起点（Enter 提交高亮行，故须知道起点才能算步进）。
fn parse_option_line(line: &str) -> Option<(u32, String, bool)> {
    let (t, highlighted) = strip_cursor_marker(line);
    let digits_len = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits_len == 0 {
        return None;
    }
    let (num_str, rest) = t.split_at(digits_len);
    // 编号上限防御：超 u32 或过长串不是选项行
    if digits_len > 3 {
        return None;
    }
    let num: u32 = num_str.parse().ok()?;
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    // 分隔符后必须紧跟空白（防 `1.5x`）——且文本非空
    let label = rest.strip_prefix(' ').or_else(|| rest.strip_prefix('\t'))?;
    let label = label.trim_end();
    if label.is_empty() {
        return None;
    }
    Some((num, label.to_string(), highlighted))
}

/// 从屏读行集解析**全部**连续编号簇（1 → 2 → 3 …，步长必须为 1），按出现顺序返回。
///
/// 这是 [`parse_dialog_options`] 的**同一套算法**的「不取最长」视图——丁T4 收尾的
/// Full Access 二次确认框需要它：确认框出现时，权限菜单可能**仍在屏上**（overlay），
/// 此时屏上同时有两个编号簇（菜单 1..4 与确认框 1..2），而「取最长簇」的
/// [`parse_dialog_options`] 会选中**菜单**（4 > 2）→ 肯定项关键词找不到 → 永远切不了
/// Full Access。按簇逐一看「哪一簇里恰好有一个肯定项」才能定位到确认框。
///
/// 簇的构造与切断规则与 [`parse_dialog_options`] **逐字同源**（空行/横线不切断，
/// 其余非选项行切断）——两处不得各写一遍（本仓既往的「同一判据两处实现」教训）。
pub fn parse_dialog_clusters(lines: &[String]) -> Vec<Vec<DialogOption>> {
    let mut out: Vec<Vec<DialogOption>> = Vec::new();
    let mut cur: Vec<DialogOption> = Vec::new();
    let mut expect: u32 = 1;
    // 收尾当前簇（与旧实现的 `if cur.len() > best.len() { best = take(cur) } else { cur.clear() }`
    // 的「finalize」语义一致：非空才产出）
    fn flush(cur: &mut Vec<DialogOption>, out: &mut Vec<Vec<DialogOption>>) {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    }
    for line in lines {
        match parse_option_line(line) {
            Some((num, label, hl)) if num == expect => {
                cur.push(DialogOption {
                    number: num,
                    label,
                    highlighted: hl,
                });
                expect += 1;
            }
            Some((num, label, hl)) if num == 1 => {
                // 新的簇从 1 重新开始
                flush(&mut cur, &mut out);
                cur.push(DialogOption {
                    number: num,
                    label,
                    highlighted: hl,
                });
                expect = 2;
            }
            Some(_) => {
                // 编号不连续（跳到 3 而期待 2 等）→ 当前簇终止
                flush(&mut cur, &mut out);
                expect = 1;
            }
            None => {
                // 非选项行：**不立刻终止簇**——对话框选项行之间可能夹着空行/说明行
                // （实测 TUI 布局有分隔线）；但也不推进 expect。分隔线（空行/全横线）
                // 不切断，其余非选项行切断（与旧实现逐字一致）。
                if !line.trim().is_empty() && !line.trim().chars().all(|c| c == '-' || c == '─') {
                    flush(&mut cur, &mut out);
                    expect = 1;
                }
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// 从屏读行集解析对话框选项表（纯函数，可测）。
///
/// 算法：逐行找**连续编号簇**（1 → 2 → 3 …，步长必须为 1）；取最长簇（**等长时取
/// 最先出现的**）；簇内每行产一个 [`DialogOption`]。要求簇长 ≥ 2（单个 `1.` 行不是
/// 「N 选一」对话框——可能只是正文列表，不出手）。簇长 > [`MAX_DIALOG_OPTIONS`] → None
/// （超数字键域，降级）。
///
/// 返回 None 的所有情形（调用方据此降级二元卡 + 防重警示）：无簇 / 簇长 < 2 /
/// 簇长 > 9。
pub fn parse_dialog_options(lines: &[String]) -> Option<Vec<DialogOption>> {
    // 取最长簇（等长时取最先出现的——`fold` 的 `>` 比较保持 `best` 不变，与旧实现的
    // `if cur.len() > best.len()` 同口径）
    let best = parse_dialog_clusters(lines)
        .into_iter()
        .fold(
            Vec::new(),
            |best, c| if c.len() > best.len() { c } else { best },
        );
    if best.len() < 2 || best.len() > MAX_DIALOG_OPTIONS {
        return None;
    }
    // 编号必须严格连续（1..=len）——簇的构造已保证，此处为显式不变式断言
    if best
        .iter()
        .enumerate()
        .any(|(i, o)| o.number != i as u32 + 1)
    {
        return None;
    }
    Some(best)
}

/// **对话框在场 = 控制类注入红线**（丁T3 §2.7，裁8/9）——判据的**单点实现**。
///
/// # 是什么、为什么要在这一层
///
/// 「在场」的判据就是本模块既有的解析能力本身：`parse_dialog_options` 返回 `Some`
/// 意味着屏读可见窗口里存在一个**编号选项簇**（≥2 项、连续编号、落在数字键域内）
/// ——那正是「TUI 正在等用户从编号项里选一个」的形态。丁T3 的问题 5/6 两条实机
/// 事故（模式按钮连点 17 次全落进待决对话框 → 变成「选第一项」；composer 自由文本
/// 被对话框理解成选项切换）根因都是**控制类注入（模式切换 / 斜杠命令）或自由文本
/// 在对话框在场时被直接打进终端**。
///
/// 判据抽成独立函数而**不是**在调用点各写一遍 `is_some()`：本仓既往教训是「同一
/// 判据两处实现 → 口径漂移」（见 `remote::api::plan_pending_tail_index` 的成对注释），
/// 而这条判据的松紧直接决定「拒」与「放行」——放行的代价是把用户消息变成一次误选。
///
/// # None（检测不可用）与「无对话框」为何同收敛为**不阻断**
///
/// 本函数只对 `Some` 的选项表判在场，`None` 一律 `false`（不阻断）。理由（**刻意
/// 选择，不是遗漏**）：
/// - `None` 有两种来源：①屏读成功但没解析出编号簇（= 确实无对话框）；②**检测能力
///   缺失**（非 Windows 无屏读 API / AttachConsole 失败 / 解析不到簇）。两者在调用
///   点无法区分（同一条降级链），而把 ② 当「在场」会让**非 Windows 平台（macOS）
///   的所有模式切换与斜杠命令永久 409**——那等于把一条已实测可用的能力线砍掉，
///   与「红线 4：不假装成功」无关（那是**谎报成功**的禁令；此处拒绝注入既不谎报
///   成功也不谎报失败，是能力缺失下的保守放行）。
/// - 反向（能力缺失时拒绝）的另一面代价：用户手里明明有一个能用的模式按钮，只因
///   为平台没有屏读就永远点不动，且回执语义变成「终端有对话框」——**那是假话**
///   （我们并不知道有没有）。诚实做法 = 放行 + 由调用点如实标注「本次未做在场检测」
///   （见 `remote::api::session_mode_switch` 的注释与 `dialog_probe` 缝的语义注）。
///
/// 实机依据（三类真实对话框的屏读原文见 [`parse_dialog_options`] 的测试夹具，
/// 取自 2026-09-21 探测档案 `screen-t5-*` 系列）。
pub fn blocks_control_injection(probe: Option<&[DialogOption]>) -> bool {
    // 项数 ≥ 2 是 [`parse_dialog_options`] 的既有不变式（单行 `1.` 不算 N 选一），
    // 此处复述为显式守卫：未来若解析器放宽下界，本红线判据不会跟着松掉
    probe.is_some_and(|opts| opts.len() >= 2)
}

/// 屏读在场探测（**单点实现，工具无关**）——Windows 读可见窗口 → 解析编号选项簇。
///
/// 返回 `Some` = 屏读确认存在编号选项对话框（在场）；`None` = 无法判定或确实无对话
/// 框（两义同收敛，见 [`blocks_control_injection`] 的裁决注）。
///
/// 入参是 **pid**（不是 `&Session`）：屏读只需要被 attach 的进程，接 pid 让本函数同时
/// 是 `RemoteState.dialog_probe` 缝（`fn(&str, u32) -> Option<Vec<DialogOption>>`）的
/// 生产实现本体——零适配层，也就零「两处实现」的漂移面。所有调用点（审批端点、
/// 模式切换守卫、缝）都经此处，**不得**在别处再写一遍「屏读 + 解析」。
///
/// 屏读失败只记 debug 日志（不打断调用链——调用方按「无法判定」处理）。
#[cfg(windows)]
pub fn probe_screen_dialog(pid: u32) -> Option<Vec<DialogOption>> {
    match crate::inject::windows_console::read_screen_window(pid) {
        Ok(lines) => match parse_dialog_options(&lines) {
            Some(opts) => {
                log::debug!("对话框屏读：解析出 {} 个编号选项（pid={pid}）", opts.len());
                Some(opts)
            }
            None => {
                log::debug!("对话框屏读：无连续编号簇（pid={pid}）");
                None
            }
        },
        Err(e) => {
            log::debug!("对话框屏读失败（pid={pid}: {e}）");
            None
        }
    }
}

/// 非 Windows 降级：无屏读 API（macOS 等）→ 恒 `None`（= 无法判定）。
/// 与 [`blocks_control_injection`] 的裁决配套：能力缺失**不阻断**控制类注入，
/// 由调用点如实标注「本次未做在场检测」。
#[cfg(not(windows))]
pub fn probe_screen_dialog(_pid: u32) -> Option<Vec<DialogOption>> {
    None
}

/// 目标选项的**导航确认**序列（R1 起：claude 计划批准 / kimi 计划批准类对话框）。
///
/// # 为什么需要（两处独立的实机证据）
///
/// **① claude 计划批准框：数字键无效。** 独立探测三样本零效果，而 `↓×(n-1)+Enter`
/// 分别正确选到 1/2/3（含 JSONL 批准回执）。本机复验（2026-09-21 同批探测）：
/// 高亮在第 1 行时注入 '2' → 对话框无变化；改为 ↓+Enter → 第 2 项被提交。
///
/// **② kimi 计划批准框：数字通道不可依赖（安全缺陷）。** 独立探测实测
/// `'2'+Enter` 产出的是 **Approve 且模型真的执行写了文件**（数字被忽略、Enter 提交
/// 的是**当前高亮行**），而另一实例 `'3'` 却单键即关框置 Rejected——行为不一致，
/// 存在「**想拒绝却批准**」的现实后果。可靠路径 = ↓+Enter（`· Rejected` 实锤）。
///
/// # 语义（实测，2026-09-21）
///
/// - **Enter 提交的是当前高亮行**，与编号无关；
/// - `↓×k` 使高亮**前进 k 行**，**到尾部循环回首个**（claude 3 行实测：↓ 从 3 回 1，
///   ↑ 从 1 回 3）；↑ 同理反向；
/// - 故步进数 = **从当前高亮位到目标位的循环距离**，不是 `target - 1`。
///
/// 本函数据此计算：取**唯一**高亮行（`highlighted`）为起点；无高亮信息（解析器未
/// 见光标标记，某些 TUI 形态可能不渲染）→ **保守返回 Err**（不猜起点——猜错会提交
/// 错误选项，正是我们要消除的「想拒绝却批准」）。多行同时带标记 → Err（形态异常）。
///
/// 返回：`[down × k, "enter"]`（k 可为 0，即高亮已在目标行时直接 Enter）。
pub fn navigation_sequence(
    options: &[DialogOption],
    target_number: u32,
) -> Result<Vec<String>, String> {
    let (start, target_idx) = navigation_anchors(options, target_number)?;
    let n = options.len();
    // 循环前进距离：从 start 走到 target（0..n 之间）
    let steps = (target_idx + n - start) % n;
    let mut seq = vec!["down".to_string(); steps];
    seq.push("enter".to_string());
    Ok(seq)
}

/// **方向感知**的导航确认序列（丁T4；**菜单路径已改为闭环、不再用它**——见下）。
///
/// # 与 [`navigation_sequence`] 的区别，以及为什么需要两个
///
/// `navigation_sequence` 只用 `↓`（**循环前进**），它成立的前提是「选择器到尾部
/// 回卷到首个」——这条前提对 claude/kimi/codex 的**编号对话框**有实测（R1：claude
/// 三行 ↓ 从 3 回 1），所以审批路径照用。
///
/// 而**权限菜单**（codex `/permissions` / kimi `/permission`）**是否回卷没有任何实测**。
/// 此时沿用循环前进会有一个危险的推论：目标在高亮位**之上**时算法会发出「↓ × (n-1)」
/// ——若该菜单不回卷，这串键会把高亮停在末项并回车，**切到错误的权限档**（正是本仓
/// 反复防的「想拒绝却批准」同类事故）。
///
/// M9R 的 codex 实机取证恰好给了反向证据：目标 `Read Only` 位于高亮项
/// `Ask for approval` **之上**，实机用的是 **↑+Enter**（见 `inject::approve` 的
/// 表注）——即**方向可以显式指定，且这条路上 ↑ 是通的**。
///
/// 故本变体「按方向走最少步、**不假设回卷**」：目标在下方 → `↓ × k`；目标在上方 →
/// `↑ × k`；同项 → 直接 Enter。两函数共享 [`navigation_anchors`]（目标越界 / 高亮
/// 唯一性判据**只有一份**）。
///
/// # 菜单路径自丁T4 收尾起**不再使用本函数**（**仅审批路径在用，勿动**）
///
/// 本变体仍是「**一次算步进 + 盲发序列**」：它假定「按 k 次键就一定前进 k 行」，而
/// 实机取证的**方法论要求**是「每按一次 ↓ 或 ↑ 就重新屏读、确认高亮确实移到下一项」
/// （用户实机取证档 §8 原文）。菜单路径因此改为
/// [`crate::inject::mode::navigate_until_highlighted`] 的**闭环**：每步复核、只有高亮确实
/// 落在目标行才发回车。
///
/// **审批路径不改**：那里的屏上对话框在**投递期间不会变**（选项固定、投递完即结束，
/// 且 approve 端点投递前刚做过一次现场重解析），不存在菜单那种「边发键边重绘」的窗口；
/// 更重要的是**本批没有审批路径的闭环证据**（没有实机观测到它出过错）——按「未验证不
/// 出手」不动它。**两个函数的保守面（不猜起点）仍然共享。**
pub fn navigation_sequence_directional(
    options: &[DialogOption],
    target_number: u32,
) -> Result<Vec<String>, String> {
    let (start, target_idx) = navigation_anchors(options, target_number)?;
    let mut seq: Vec<String> = if target_idx >= start {
        vec!["down".to_string(); target_idx - start]
    } else {
        vec!["up".to_string(); start - target_idx]
    };
    seq.push("enter".to_string());
    Ok(seq)
}

/// 两个导航序列的**公共锚点解析**：目标必须在表内 + 起点 = **唯一**高亮行。
///
/// 抽出来的理由与 [`blocks_control_injection`] 同源：这两条判据（尤其「不猜起点」）
/// 是安全面，散在两份实现里迟早一处松一处紧——`navigation_sequence_directional`
/// 与 `navigation_sequence` 共用本函数，任何一方都改不动另一半的严格度。
fn navigation_anchors(
    options: &[DialogOption],
    target_number: u32,
) -> Result<(usize, usize), String> {
    // 目标必须在选项表内
    let target_idx = options
        .iter()
        .position(|o| o.number == target_number)
        .ok_or_else(|| format!("目标编号 {target_number} 不在对话框选项表内"))?;
    // 起点 = 唯一高亮行
    let mut hl: Option<usize> = None;
    for (i, o) in options.iter().enumerate() {
        if o.highlighted {
            if hl.is_some() {
                return Err("对话框有多行高亮标记，形态异常，不出手".to_string());
            }
            hl = Some(i);
        }
    }
    let start = hl.ok_or_else(|| "解析不到当前高亮行，无法计算步进（不猜起点）".to_string())?;
    Ok((start, target_idx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 2026-09-21 实机探测抓获的**真实缺陷回归锁**：codex 把当前高亮项渲染为
    /// `› 1. ...`（U+203A），未高亮项是 `  2. ...`。原实现只 trim 空白 → 高亮项
    /// 永不匹配 → 首项缺失 → 连续簇失败 → 整个对话框降级二元卡。
    /// 夹具=实机屏幕原文（`screen-t5-codex-implement-before.txt` 行 25–27）。
    #[test]
    fn parses_dialog_with_cursor_marker_prefix() {
        let codex = lines(&[
            "  Implement this plan?",
            "",
            "› 1. Yes, implement this plan          Switch to Default and start coding.",
            "  2. Yes, clear context and implement  Fresh thread. Context: 2% used.",
            "  3. No, stay in Plan mode             Continue planning with the model.",
            "",
            "  Press enter to confirm or esc to go back",
        ]);
        let opts = parse_dialog_options(&codex).expect("带 › 光标标记的真实对话框必须解析");
        assert_eq!(opts.len(), 3, "高亮项不得因 › 前缀被漏掉");
        assert_eq!(opts[0].number, 1);
        assert_eq!(
            opts[0].label,
            "Yes, implement this plan          Switch to Default and start coding."
        );
        assert_eq!(
            opts[2].label,
            "No, stay in Plan mode             Continue planning with the model."
        );

        // claude 的同类标记 ❯（U+276F）——实机屏幕原文形态
        let claude = lines(&[
            " Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            " ❯ 1. Yes, and use auto mode",
            "   2. Yes, manually approve edits",
            "   3. Tell Claude what to change",
            "      shift+tab to approve with this feedback",
        ]);
        let opts = parse_dialog_options(&claude).expect("带 ❯ 光标标记的 claude 对话框必须解析");
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].label, "Yes, and use auto mode");
        assert_eq!(opts[2].label, "Tell Claude what to change");

        // ASCII 兜底形态
        let ascii = lines(&["> 1. First", "  2. Second"]);
        assert_eq!(parse_dialog_options(&ascii).unwrap().len(), 2);
    }

    /// 2026-09-21 实机探测第二例：kimi `Ready to build?` 用 `▶`（U+25B6）作光标标记
    /// ——夹具=实机屏幕原文（`screen-t5-kimi-ready-before.txt` 行 22–24）
    #[test]
    fn parses_kimi_ready_to_build_dialog() {
        let kimi = lines(&[
            "   ▶ Ready to build with this plan?",
            "",
            "   ▶ 1. Approve",
            "     2. Reject",
            "     3. Revise",
            "",
            "   ↑/↓ select · 1/2/3 choose · ↵ confirm",
        ]);
        let opts = parse_dialog_options(&kimi).expect("kimi Ready to build 必须解析（▶ 标记已剥）");
        assert_eq!(opts.len(), 3, "▶ 前缀不得吃掉首项");
        assert_eq!(opts[0].number, 1);
        assert_eq!(opts[0].label, "Approve");
        assert_eq!(opts[1].label, "Reject");
        assert_eq!(opts[2].label, "Revise");
    }

    /// 三类真实对话框形态（计划书 §1 问题 6 列举的选项文本）→ 全部解析成功
    #[test]
    fn parses_three_real_dialog_shapes() {
        // claude 计划批准
        let claude = lines(&[
            "Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            "  1. Yes, and use auto mode",
            "  2. Yes, manually approve edits",
            "  3. Tell Claude what to do differently",
        ]);
        let opts = parse_dialog_options(&claude).expect("claude 计划批准必须解析");
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].number, 1);
        assert_eq!(opts[0].label, "Yes, and use auto mode");
        assert_eq!(opts[2].label, "Tell Claude what to do differently");

        // codex Implement this plan
        let codex = lines(&[
            "Implement this plan?",
            "1. Yes, implement this plan",
            "2. No, keep planning",
        ]);
        let opts = parse_dialog_options(&codex).unwrap();
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[1].label, "No, keep planning");

        // kimi Ready to build
        let kimi = lines(&["Ready to build", "1. Approve", "2. Reject", "3. Revise"]);
        let opts = parse_dialog_options(&kimi).unwrap();
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].label, "Approve");
        assert_eq!(opts[2].label, "Revise");
    }

    /// 编号格式变体：右括号、无缩进、Tab 分隔
    #[test]
    fn accepts_numbering_variants() {
        let v = lines(&["1) First", "2) Second"]);
        assert_eq!(parse_dialog_options(&v).unwrap().len(), 2);
        let v = lines(&["1.\tTab separated", "2.\tSecond"]);
        assert_eq!(parse_dialog_options(&v).unwrap().len(), 2);
    }

    /// 降级面（红线 3）：无簇 / 簇长 1 / 超 9 项 → None（端点据此降级二元卡）
    #[test]
    fn degrades_when_unparseable() {
        // 无编号行
        assert!(parse_dialog_options(&lines(&["Do you want to proceed?", "[y/n]"])).is_none());
        // 只有一项（不是 N 选一）
        assert!(parse_dialog_options(&lines(&["1. Only one"])).is_none());
        // 超 9 项
        let many: Vec<String> = (1..=10).map(|i| format!("{i}. option {i}")).collect();
        assert!(
            parse_dialog_options(&many).is_none(),
            "超数字键域 → 不出手（降级）"
        );
        // 9 项恰好在上界内
        let nine: Vec<String> = (1..=9).map(|i| format!("{i}. option {i}")).collect();
        assert_eq!(parse_dialog_options(&nine).unwrap().len(), 9);
        // 空输入
        assert!(parse_dialog_options(&[]).is_none());
    }

    /// 编号不连续 → 不凑数（防把正文列表当选项）：1,2 后跳到 5 → 取 1,2 簇；
    /// 从 2 开始（无 1）→ 不成簇
    #[test]
    fn non_continuous_numbering_is_not_a_cluster() {
        let opts = parse_dialog_options(&lines(&["1. A", "2. B", "5. C"])).unwrap();
        assert_eq!(opts.len(), 2, "不连续的 5 不并入簇");
        assert!(
            parse_dialog_options(&lines(&["2. B", "3. C"])).is_none(),
            "无 1 不成簇"
        );
    }

    /// 分隔线（空行/横线）不切断簇；其他正文行切断
    #[test]
    fn separator_lines_do_not_break_cluster() {
        let v = lines(&["1. Yes", "", "────────", "2. No"]);
        assert_eq!(
            parse_dialog_options(&v).unwrap().len(),
            2,
            "空行与横线是布局分隔，不切断选项簇"
        );
        // 正文行切断：两个独立簇取更长的
        let v = lines(&["1. A", "2. B", "some prose here", "1. X", "2. Y", "3. Z"]);
        let opts = parse_dialog_options(&v).unwrap();
        assert_eq!(opts.len(), 3, "取更长的簇");
        assert_eq!(opts[0].label, "X");
    }

    // ---- R1：导航确认序列（↓×k + Enter）----

    /// R1 核心：步进数从**解析到的当前高亮位**算起（循环距离），不是 target-1
    #[test]
    fn navigation_steps_from_highlight_position() {
        // 起点=第 1 行（高亮在 1），目标 3 → ↓×2 + Enter
        let opts = vec![opt(1, "A", true), opt(2, "B", false), opt(3, "C", false)];
        assert_eq!(
            navigation_sequence(&opts, 3).unwrap(),
            vec!["down", "down", "enter"],
            "高亮在 1、目标 3 → ↓×2 + Enter"
        );
        // 起点=第 3 行（高亮在 3），目标 1 → **循环**前进 1 步（3→1）
        let opts2 = vec![opt(1, "A", false), opt(2, "B", false), opt(3, "C", true)];
        assert_eq!(
            navigation_sequence(&opts2, 1).unwrap(),
            vec!["down", "enter"],
            "高亮在 3、目标 1 → ↓×1（循环回卷）+ Enter，而非 ↓×2 反向"
        );
        // 高亮已在目标行 → 直接 Enter（零步进）
        let opts3 = vec![opt(1, "A", false), opt(2, "B", true)];
        assert_eq!(navigation_sequence(&opts3, 2).unwrap(), vec!["enter"]);
    }

    /// R1 安全面：**解析不到高亮 / 多行高亮 / 目标越界 → 一律 Err**（不猜起点）
    #[test]
    fn navigation_refuses_when_start_unknown() {
        // 无高亮信息（某些 TUI 形态可能不渲染光标标记）→ 拒绝（猜错会提交错误选项）
        let no_hl = vec![opt(1, "A", false), opt(2, "B", false)];
        assert!(
            navigation_sequence(&no_hl, 2).is_err(),
            "起点未知必须拒绝（这正是「想拒绝却批准」的防线）"
        );
        // 多行同时带标记 → 形态异常 → 拒绝
        let multi = vec![opt(1, "A", true), opt(2, "B", true)];
        assert!(navigation_sequence(&multi, 2).is_err());
        // 目标不在表内 → 拒绝
        let ok = vec![opt(1, "A", true), opt(2, "B", false)];
        assert!(navigation_sequence(&ok, 9).is_err());
    }

    // ---- 丁T4：方向感知变体（模式权限菜单用；与循环变体共享锚点判据）----

    /// 方向感知：目标在上走 ↑、在下走 ↓、同项直接 Enter——**不假设回卷**。
    /// 依据 = codex 权限菜单的 M9R 实机取证（高亮在 `Ask for approval`、目标
    /// `Read Only` 在其上，实机序列是 ↑+Enter）。还原动作：把本函数改回
    /// `navigation_sequence`（纯 ↓ 循环）→ 本断言先红（3 行时它会给出 ↓↓+Enter）。
    #[test]
    fn directional_navigation_picks_direction() {
        let opts = vec![opt(1, "A", false), opt(2, "B", true), opt(3, "C", false)];
        assert_eq!(
            navigation_sequence_directional(&opts, 1).unwrap(),
            vec!["up", "enter"],
            "目标在高亮之上 → ↑×1"
        );
        assert_eq!(
            navigation_sequence_directional(&opts, 3).unwrap(),
            vec!["down", "enter"],
            "目标在高亮之下 → ↓×1"
        );
        assert_eq!(
            navigation_sequence_directional(&opts, 2).unwrap(),
            vec!["enter"],
            "高亮已在目标项 → 零步进"
        );
        // 两变体在「目标在下方」时同解（循环前进 = 直接前进）
        assert_eq!(
            navigation_sequence(&opts, 3).unwrap(),
            navigation_sequence_directional(&opts, 3).unwrap()
        );
        // 两变体在「目标在上方」时**刻意分歧**（这正是本变体存在的理由）
        assert_ne!(
            navigation_sequence(&opts, 1).unwrap(),
            navigation_sequence_directional(&opts, 1).unwrap()
        );
    }

    /// 方向感知变体共享同一份保守面（不猜起点 / 目标越界 / 多高亮 → Err）
    #[test]
    fn directional_navigation_shares_refusals() {
        let no_hl = vec![opt(1, "A", false), opt(2, "B", false)];
        assert!(navigation_sequence_directional(&no_hl, 2).is_err());
        let multi = vec![opt(1, "A", true), opt(2, "B", true)];
        assert!(navigation_sequence_directional(&multi, 1).is_err());
        let ok = vec![opt(1, "A", true), opt(2, "B", false)];
        assert!(navigation_sequence_directional(&ok, 9).is_err());
    }

    /// R1：解析器必须**报告高亮位**——用实机屏幕原文夹具（claude 计划批准框）
    #[test]
    fn parse_reports_highlight_from_real_screen() {
        let claude = lines(&[
            " Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            " ❯ 1. Yes, and use auto mode",
            "   2. Yes, manually approve edits",
            "   3. Tell Claude what to change",
        ]);
        let opts = parse_dialog_options(&claude).unwrap();
        assert_eq!(opts.len(), 3);
        assert!(opts[0].highlighted, "❯ 在第 1 行 → 该行为高亮");
        assert!(!opts[1].highlighted);
        assert!(!opts[2].highlighted);
        // 端到端：点第 3 项 → 从高亮位 1 算 → ↓×2 + Enter（本机实机已验证该序列生效）
        assert_eq!(
            navigation_sequence(&opts, 3).unwrap(),
            vec!["down", "down", "enter"]
        );

        // codex `›` / kimi `▶` 同样报告高亮
        let codex = lines(&["› 1. Yes, implement", "  2. No, keep planning"]);
        let co = parse_dialog_options(&codex).unwrap();
        assert!(co[0].highlighted && !co[1].highlighted, "codex › 报高亮");
        let kimi = lines(&["   ▶ 1. Approve", "     2. Reject", "     3. Revise"]);
        let km = parse_dialog_options(&kimi).unwrap();
        assert!(km[0].highlighted, "kimi ▶ 报高亮");
        assert!(!km[2].highlighted);
    }

    fn opt(n: u32, label: &str, hl: bool) -> DialogOption {
        DialogOption {
            number: n,
            label: label.to_string(),
            highlighted: hl,
        }
    }

    // ---- 丁T3 接入①：对话框在场判据（控制类注入红线，§2.7）----

    /// 在场判据的**真机三形态**（夹具 = 2026-09-21 探测档案的屏幕原文，逐字抄录）：
    /// claude 计划批准 / codex Implement this plan / kimi Ready to build——解析得出
    /// 选项表 ⇒ 判在场（控制类注入必须被拒）。
    #[test]
    fn presence_blocks_on_three_real_dialogs() {
        // claude（`screen-t5-claude-plan-before.txt` 尾 10 行）
        let claude = parse_dialog_options(&lines(&[
            " Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            " ❯ 1. Yes, and use auto mode",
            "   2. Yes, manually approve edits",
            "   3. Tell Claude what to change",
            "      shift+tab to approve with this feedback",
        ]));
        assert!(
            blocks_control_injection(claude.as_deref()),
            "claude 计划批准框在场 ⇒ 控制类注入必须被拒"
        );

        // codex（`screen-t5-codex-implement-before.txt` 行 24–29）
        let codex = parse_dialog_options(&lines(&[
            "  Implement this plan?",
            "",
            "› 1. Yes, implement this plan          Switch to Default and start coding.",
            "  2. Yes, clear context and implement  Fresh thread. Context: 2% used.",
            "  3. No, stay in Plan mode             Continue planning with the model.",
            "",
            "  Press enter to confirm or esc to go back",
        ]));
        assert!(blocks_control_injection(codex.as_deref()));

        // kimi（`screen-t5-kimi-ready-before.txt` 行 20–26）
        let kimi = parse_dialog_options(&lines(&[
            "   ▶ Ready to build with this plan?",
            "",
            "   ▶ 1. Approve",
            "     2. Reject",
            "     3. Revise",
            "",
            "   ↑/↓ select · 1/2/3 choose · ↵ confirm",
        ]));
        assert!(blocks_control_injection(kimi.as_deref()));
    }

    /// 无对话框的普通输出屏（运行中的 TUI 常态）→ 不阻断；`None`（检测不可用 / 无簇）
    /// 同样不阻断——**能力缺失不阻断**是本判据的刻意裁决（理由见
    /// [`blocks_control_injection`] 文档；macOS 无屏读时若判阻断，模式切换会永久 409）
    #[test]
    fn presence_absent_on_plain_screen_and_none_probe() {
        // 普通输出（正文里出现孤立编号行也不成簇——解析器既有下界）
        let plain = parse_dialog_options(&lines(&[
            "> 帮我改一下 login.ts",
            "",
            "● 已完成修改，运行了 3 个测试",
            "",
            "  1. 只出现一项不算对话框",
            "",
            "> ",
        ]));
        assert!(!blocks_control_injection(plain.as_deref()));

        // 检测不可用（非 Windows / 屏读失败 / 无簇）→ None → 不阻断
        assert!(
            !blocks_control_injection(None),
            "无法判定 ≠ 在场（能力缺失放行，由调用点标注未检测）"
        );
        // 空表（构造上不可达——解析器下界 ≥2——但判据自身要守住下界）
        assert!(!blocks_control_injection(Some(&[])));
        assert!(!blocks_control_injection(Some(&[opt(1, "Only one", true)])));
    }

    /// 数字键域文本（`1.5x` 之类不误判）
    #[test]
    fn rejects_lookalikes() {
        let v = lines(&["1.5x zoom", "2.0 release"]);
        assert!(parse_dialog_options(&v).is_none(), "小数点变体不是选项行");
        assert!(parse_option_line("1. Yes").is_some());
        assert!(parse_option_line("1.Yes").is_none(), "分隔符后必须空白");
        assert!(parse_option_line("1. ").is_none(), "空文本不是选项");
        assert!(parse_option_line("1234. X").is_none(), "超长编号不是选项");
    }
}
