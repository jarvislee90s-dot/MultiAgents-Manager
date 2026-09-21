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

/// 判定一行是否是「编号选项行」并抽出 (编号, 文本)。行模式：
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
    // 前导空白 + 光标标记的位置判定：标记必须出现在**编号之前**（前导区）才算高亮
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
    let t = &line[idx..];
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

/// 从屏读行集解析对话框选项表（纯函数，可测）。
///
/// 算法：逐行找**连续编号簇**（1 → 2 → 3 …，步长必须为 1）；取最长簇；簇内每行
/// 产一个 [`DialogOption`]。要求簇长 ≥ 2（单个 `1.` 行不是「N 选一」对话框——
/// 可能只是正文列表，不出手）。簇长 > [`MAX_DIALOG_OPTIONS`] → None（超数字键域，
/// 降级）。
///
/// 返回 None 的所有情形（调用方据此降级二元卡 + 防重警示）：无簇 / 簇长 < 2 /
/// 簇长 > 9。
pub fn parse_dialog_options(lines: &[String]) -> Option<Vec<DialogOption>> {
    let mut best: Vec<DialogOption> = Vec::new();
    let mut cur: Vec<DialogOption> = Vec::new();
    let mut expect: u32 = 1;
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
                // 新的簇从 1 重新开始：保留更长的那个
                if cur.len() > best.len() {
                    best = std::mem::take(&mut cur);
                } else {
                    cur.clear();
                }
                cur.push(DialogOption {
                    number: num,
                    label,
                    highlighted: hl,
                });
                expect = 2;
            }
            Some(_) => {
                // 编号不连续（跳到 3 而期待 2 等）→ 当前簇终止
                if cur.len() > best.len() {
                    best = std::mem::take(&mut cur);
                } else {
                    cur.clear();
                }
                expect = 1;
            }
            None => {
                // 非选项行：**不立刻终止簇**——对话框选项行之间可能夹着空行/说明行
                // （实测 TUI 布局有分隔线）；但也不推进 expect。为防跨段落误连，只在
                // 连续两个非选项行后终止簇。
                // 简化实现：空行不终止（分隔线常为空或全横线），其余非选项行终止。
                if !line.trim().is_empty() && !line.trim().chars().all(|c| c == '-' || c == '─') {
                    if cur.len() > best.len() {
                        best = std::mem::take(&mut cur);
                    } else {
                        cur.clear();
                    }
                    expect = 1;
                }
            }
        }
    }
    if cur.len() > best.len() {
        best = cur;
    }
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
    let n = options.len();
    // 循环前进距离：从 start 走到 target（0..n 之间）
    let steps = (target_idx + n - start) % n;
    let mut seq = vec!["down".to_string(); steps];
    seq.push("enter".to_string());
    Ok(seq)
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
