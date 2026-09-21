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
    /// 编号（屏幕上显示的 1 起数字，即注入该数字键即可选中）
    pub number: u32,
    /// 选项文本原文（可能含该工具追加的说明；不裁剪、不改写——把终端所见原样
    /// 交给用户是 T5 的目标）
    pub label: String,
}

/// 数字键域上限（与 [`super::question::digit_key`] 同口径：'1'..'9'）
pub const MAX_DIALOG_OPTIONS: usize = 9;

/// 判定一行是否是「编号选项行」并抽出 (编号, 文本)。行模式：
/// `^\s*(\d+)\s*[.)]\s*(.+)$`——允许前导空白（对话框常在缩进区）、编号后跟
/// `.` 或 `)`、其后至少一个空白（防把 `1.5x` 这类数字当选项）。
fn parse_option_line(line: &str) -> Option<(u32, String)> {
    let t = line.trim_start();
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
    Some((num, label.to_string()))
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
            Some((num, label)) if num == expect => {
                cur.push(DialogOption { number: num, label });
                expect += 1;
            }
            Some((num, label)) if num == 1 => {
                // 新的簇从 1 重新开始：保留更长的那个
                if cur.len() > best.len() {
                    best = std::mem::take(&mut cur);
                } else {
                    cur.clear();
                }
                cur.push(DialogOption { number: num, label });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
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
