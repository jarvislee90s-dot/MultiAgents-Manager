//! 题屏行块解析 → 统一快照 schema（2026-10-05 推广批 F3；2026-10-05 深夜扩
//! kimi 多选页——文件名沿用 oc，模块顶注释申报双工具）。
//! 与 claude 的 `question.rs::question_screen_snapshot` 同形（`QuestionScreenSnapshot`，
//! 前端对位逻辑零改动复用）。方言来源 = `dialect::own_answer_dialect("opencode")`
//! （勾选标记/入口标签单一真源）。
//!
//! 页判据：题页 footer 含 `enter toggle`（用户 2026-10-04 截图实测；Confirm 页是
//! `enter submit`、题页无 submit 锚——戊探A ④）→ 非题页返回 None（GET/回执按
//! 「读不到快照」保守降级，前端维持本地状态）。
//!
//! 字段口径：
//! - `checked` = 编号勾选行按屏上序（`Some(bool)`；opencode 全选项带勾选框）；
//! - `heading` = 首个编号行之前的非空行拼接（题干区，含页签行——前端做包含匹配，
//!   多余行不影响对位）；
//! - `free_text` / `free_text_present` = own answer 行内容（标签行 = 未填 →
//!   `present=true, text=None`；已保存 = 内容；无编号行形态异常时不误报）。

use crate::inject::dialect::own_answer_dialect;
use crate::inject::question::QuestionScreenSnapshot;

pub const OPENCODE_QUESTION_PAGE_ANCHOR: &str = "enter toggle";

fn strip(l: &str) -> &str {
    l.trim()
        .trim_start_matches(['\u{2503}', '\u{2502}', '\u{2192}', '\u{276f}'])
        .trim_start()
}

/// opencode **单选页快照**（2026-10-05 单选适配）：单选页无勾选框（编号行无
/// 标记），footer `enter submit` 与确认页同词形不能当页锚——结构判据：连续
/// 编号行（1..=n）≥2 且全部无勾选标记，末行 = own answer 行。
///
/// 产出：`checked = [null × 选项数]`（own 行不计；单选无勾选语义，前端同步
/// 链路原生支持 null 项）、`free_text = 末行内容（≠占位标签时，含用户残留/
/// 手动作答——正是「确认卡显示未作答」「无覆盖写入按钮」两个实机症状的
/// 同步通道）。
///
/// 误报面（会话正文里的编号列表）：由 GET 的 pending-question 门（仅挂卡会话
/// 拉快照）+ 前端 heading 对位（findQuestionByHeading）双重兜底。
fn opencode_single_select_snapshot(
    lines: &[String],
    d: &crate::inject::dialect::OwnAnswerDialect,
) -> Option<QuestionScreenSnapshot> {
    // **只看页切片**（2026-10-06 GET 断链定案）：transcript 里模型的「1. …
    // 2. …」编号说明文字先于真题页被扫进 numbered，真题页首行断号 → 整屏
    // 放弃——18:03-18:26 全部 GET「解析失败」的根因。从最后一个 `Questions`
    // 页首锚之后扫 = 只看活对话框区（heading 也随之变干净：tab 栏+题干）。
    let lines = crate::inject::question::opencode_question_page_slice(lines)?;
    let mut numbered: Vec<(u32, String)> = Vec::new();
    let mut heading_parts: Vec<&str> = Vec::new();
    for l in lines {
        let t = strip(l);
        if let Some((n, text, _)) = crate::inject::dialog::parse_option_line(t) {
            // 勾选标记在场 = 多选形态页（应由多选分支处理；这里出现 = 形态混杂，
            // 保守放弃）
            let lower = t.to_lowercase();
            if d.checked_markers.iter().any(|m| lower.contains(m))
                || d.unchecked_markers.iter().any(|m| lower.contains(m))
            {
                return None;
            }
            // 连续性：编号必须严格 1..=n（断号 = 会话正文里的编号列表，非题页）
            if n as usize != numbered.len() + 1 {
                return None;
            }
            numbered.push((n, text));
            continue;
        }
        if numbered.is_empty() && !t.is_empty() {
            heading_parts.push(t);
        }
    }
    if numbered.len() < 2 {
        return None; // 选项 + own 行至少 2 行
    }
    let (_, own_content) = numbered.last()?;
    // own 行若是当前答案，行尾带 " ✓"（2026-10-06 活体定案）——剥掉再入
    // free_text，卡面「已写入」不该显示对勾
    let own_clean = own_content.trim_end().trim_end_matches('✓').trim_end();
    let free_text = if own_clean.to_lowercase().contains(d.label) {
        None // 鲜态占位标签 = 未填
    } else {
        Some(own_clean.to_string()) // 用户残留/手动作答（本轮同步目标）
    };
    // checked：**逐行真值**（2026-10-06 活体定案：单选选中行尾 " ✓" →
    // Some(true)，未选行 Some(false)）——卡面勾选态/DigitKey 翻转判定的数据源
    let opt_count = numbered.len().saturating_sub(1);
    let checked: Vec<Option<bool>> = (1..=opt_count)
        .map(|n| crate::inject::question::opencode_option_checked_at(lines, n, d))
        .collect();
    Some(QuestionScreenSnapshot {
        // 长度 = 选项数（own 行不计）——前端 advance 归属校验按载荷
        // options.length 比长度，own 行混入恒错 1（01:31 断链教训）
        checked,
        free_text,
        free_text_present: true,
        heading: heading_parts.join(" "),
    })
}

pub fn opencode_question_screen_snapshot(lines: &[String]) -> Option<QuestionScreenSnapshot> {
    let d = own_answer_dialect("opencode")?;
    if !lines
        .iter()
        .any(|l| l.to_lowercase().contains(OPENCODE_QUESTION_PAGE_ANCHOR))
    {
        // 多选页锚不在场 → 尝试**单选页**结构判据（2026-10-05 单选适配）：单选页
        // 无勾选框（编号行无标记），footer `enter submit` 与确认页同词形不能当
        // 页锚——改用**结构判据**：连续编号行（1..=n）≥2 且全部无勾选标记，末行
        // = own answer 行。确认页（Submit tab）无编号行 → None ✓。
        return opencode_single_select_snapshot(lines, &d);
    }
    let is_numbered_checked = |t: &str| -> Option<bool> {
        crate::inject::dialog::parse_option_line(t)?;
        let lower = t.to_lowercase();
        if d.checked_markers.iter().any(|m| lower.contains(m)) {
            Some(true)
        } else if d.unchecked_markers.iter().any(|m| lower.contains(m)) {
            Some(false)
        } else {
            None // 编号行但无勾选框（形态异常）——不计入
        }
    };
    let mut checked: Vec<Option<bool>> = Vec::new();
    let mut heading_parts: Vec<&str> = Vec::new();
    // 编号行内容（勾选标记 `]` 之后）——末行用于 own answer 保存态判定
    let mut last_row_text: Option<String> = None;
    for l in lines {
        let t = strip(l);
        if let Some(state) = is_numbered_checked(t) {
            checked.push(Some(state));
            last_row_text = Some(
                t.rfind(']')
                    .map(|i| t[i + 1..].trim_start().to_string())
                    .unwrap_or_default(),
            );
            continue;
        }
        if checked.is_empty() && !t.is_empty() {
            // 题干区：首个编号行之前的非空行
            heading_parts.push(t);
        }
    }
    if checked.is_empty() {
        return None; // 无选项行 = 不是可对位的题屏
    }
    // own answer 行（末编号行）：内容 == 标签 → 鲜态（present、无文本）；
    // 内容 != 标签 → 已保存（present + 屏上文本）
    let free_text_present = true;
    let saved = last_row_text.unwrap_or_default();
    let free_text = if saved.is_empty() || saved.to_lowercase().contains(d.label) {
        None
    } else {
        Some(saved)
    };
    Some(QuestionScreenSnapshot {
        checked,
        free_text,
        free_text_present,
        heading: heading_parts.join(" "),
    })
}

/// kimi **多选页快照**（2026-10-05 探测批 K 形态 + evidence/kimi-multi 实拍）：
/// 页锚 = footer 含 `tab switch`（kimi 多选页专属词形；编辑态 footer 亦含之，
/// 但编辑态行内容即输入中文字——快照如实回传，属可接受）。行语法（K2 定案）：
/// `[ ] label` / 已选 `[?] label`；Other 行**无编号**（`[ ] Other` 鲜态 /
/// `[?] Other: <文本>` 已存）。
///
/// 产出：`checked` = 逐选项 Some(已选)；`free_text` = Other 行文本（`Other: `
/// 之后；鲜态 None）；`free_text_present` = Other 行在场；`heading` = 首个
/// checkbox 行之前的非空行拼接。
///
/// 无 checkbox 行 → None（非 kimi 多选页：单选页/会话正文）。误报面 = GET
/// pending 门 + 前端 heading 对位双重兜底（同单选分支）。
/// kimi **Review 汇总页逐题摘要**（2026-10-07 新增，确认卡权威源切换）：
/// 形态（2.1.1 活体截图）：
///
/// ```text
/// Review your answer before submit
///  Q  <题干>
///  →  <答案>            （答案折行归并待实测样本）
///  Ready to submit your answers?
///  → [1] Submit
///    [2] Cancel
/// ```
///
/// 解析规则：`Q ` 前缀行开新题，其后第一个 `→ ` 行（非 `→ [N]` 确认区）为该题
/// 答案；`Ready to submit` 后的 Submit/Cancel 区不计入。未答题答案为空串。
/// 返回 (题干, 答案) 有序对；无 Q 行 → None（非 Review 页/解析不出）。
const REVIEW_ARROW: char = '→';

pub fn kimi_review_summary(lines: &[String]) -> Option<Vec<(String, String)>> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut in_review = false;
    for raw in lines {
        let t = raw.trim();
        if !in_review {
            if t.starts_with("Review your answer before submit") {
                in_review = true;
            }
            continue;
        }
        if t.starts_with("Ready to submit") {
            break; // Submit/Cancel 区及其后不计入
        }
        if t.is_empty() {
            continue;
        }
        if let Some(q) = t.strip_prefix("Q ") {
            pairs.push((q.trim().to_string(), String::new()));
            continue;
        }
        // → 答案行（非 → [N] 确认区）：首行或折行追加
        if let Some(ans) = t.strip_prefix(REVIEW_ARROW).map(str::trim) {
            if ans.starts_with('[') {
                continue; // [1] Submit / [2] Cancel 确认区
            }
            if let Some(last) = pairs.last_mut() {
                if last.1.is_empty() {
                    last.1 = ans.to_string();
                } else {
                    last.1.push(' ');
                    last.1.push_str(ans);
                }
            }
            continue;
        }
        // 折行归并（2026-10-07 17:03 实录形态）：无前缀非空行 = 上一项折行
        // ——Q 行后 → 题干折行（Q3「（单/选）」）；→ 行后 → 答案折行
        if let Some(last) = pairs.last_mut() {
            if last.1.is_empty() {
                last.0.push(' ');
                last.0.push_str(t);
            } else {
                last.1.push(' ');
                last.1.push_str(t);
            }
        }
    }
    (!pairs.is_empty()).then_some(pairs)
}

pub fn kimi_question_screen_snapshot(lines: &[String]) -> Option<QuestionScreenSnapshot> {
    // kimi 不在 OwnAnswerDialect（own answer 编排未接入）——标记直接内联。
    // **已选字形多候选**（账本吸收文案漂移的同一口径）：`[?]`=探测批 K（2.x），
    // `[✓]`=戊探B（2.0.2 空格/回车切勾实录），`[√]`=2.1.1 用户实机（2026-10-06
    // 截图）——三版本三字形，逐行任一命中即已选。未选恒 `[ ]`。
    const KIMI_CHECKED: &[&str] = &["[?]", "[✓]", "[√]"];
    const KIMI_UNCHECKED: &[&str] = &["[ ]"];
    let mut checked: Vec<Option<bool>> = Vec::new();
    let mut free_text: Option<String> = None;
    let mut other_seen = false;
    // **heading = `? ` 题干行**（2026-10-06 刷新对位修复）：旧形态「checkbox 行
    // 之前的非空行拼接」会把页头杂讯（`question` 标题、tab 栏 `优化方向 新增能力
    // …`、footer 片段）拼进 heading → 前端 findQuestionByHeading 对位必然失败 →
    // GET 刷新永远不知道终端停在第几题（用户实录）。2.1.1 活体定案：题干行形
    // `? <题干>`（单选/多选页同形，夹具 kimi-211-question-single.txt）——以它为
    // heading，剥掉 `? ` 前缀后与载荷 question 字段对位（前端 exact/partial 单一
    // 命中判据）。无 `? ` 行 → heading 空（对位自然放弃，保守不猜）。
    let mut heading: String = String::new();
    for l in lines {
        let t = strip(l);
        if heading.is_empty() && t.starts_with("? ") {
            heading = t[2..].trim().to_string();
        }
        let lower = t.to_lowercase();
        let boxed = KIMI_CHECKED
            .iter()
            .chain(KIMI_UNCHECKED.iter())
            .any(|m| lower.contains(m));
        if !boxed {
            continue;
        }
        // 内容 = 勾选标记之后的部分
        let content = KIMI_CHECKED
            .iter()
            .chain(KIMI_UNCHECKED.iter())
            .find_map(|m| lower.find(m).and_then(|i| t.get(i + m.len()..)))
            .map(|c| c.trim_start().to_string())
            .unwrap_or_default();
        let is_selected = KIMI_CHECKED.iter().any(|m| lower.contains(m));
        // Other 行（kimi 多选 Other 无编号）：内容以 `other` 开头
        if content.to_lowercase().starts_with("other") {
            other_seen = true;
            // `[?] Other: <文本>` → 冒号后为已存文本；`[ ] Other` → 鲜态
            free_text = content
                .split_once(':')
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty());
            continue;
        }
        checked.push(Some(is_selected));
    }
    if checked.is_empty() {
        // **单选页兜底**（2026-10-06 刷新对位修复）：无勾选框行可能是**单选题页**
        // （选项 = `[N] label` 编号行形，无勾选框）——此前返回 None → GET 无快照 →
        // 刷新永远回第 1 题。单选页出 heading-only 快照（checked 空 = 前端 heading
        // 对位后不写勾选态）+ Other 行文本（单选 Other 有编号，free_text 同面）。
        let arrow = char::from_u32(0x2192).unwrap();
        let chevron = char::from_u32(0x276F).unwrap();
        let has_bracket_row = lines.iter().any(|l| {
            let t = l.trim().trim_start_matches([arrow, chevron]).trim_start();
            t.starts_with('[')
                && t.find(']').is_some_and(|c| {
                    let n = &t[1..c];
                    n.len() == 1 && n.chars().next().is_some_and(|d| d.is_ascii_digit())
                })
        });
        if !has_bracket_row {
            return None;
        }
        let free_text = lines
            .iter()
            .rev()
            .filter_map(|l| {
                let t = l.trim().trim_start_matches([arrow, chevron]).trim_start();
                if !t.starts_with('[') {
                    return None;
                }
                let close = t.find(']')?;
                let after = t[close + 1..].trim();
                after.to_lowercase().starts_with("other")
                    .then(|| after.split_once(':').map(|(_, v)| v.trim().to_string()))
                    .flatten()
                    .filter(|v| !v.is_empty())
            })
            .next();
        return Some(QuestionScreenSnapshot {
            checked: Vec::new(),
            free_text: free_text.clone(),
            free_text_present: free_text.is_some(),
            heading,
        });
    }
    Some(QuestionScreenSnapshot {
        checked,
        free_text,
        free_text_present: other_seen,
        heading,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn multi_page() -> Vec<String> {
        vec![
            "OC | 优化重点: 你想加的动效".to_string(),
            "你希望这次优化重点放在哪些方面？（可多选）".to_string(),
            "".to_string(),
            "1. [ ] 画面美感与细节".to_string(),
            "2. [ ] 交互动效".to_string(),
            "3. [v] 性能与兼容性".to_string(),
            "4. [ ] 无障碍与可用性".to_string(),
            "5. [ ] 代码结构整洁".to_string(),
            "6. [ ] Type your own answer".to_string(),
            "⇆ tab  ↑↓ select  enter toggle  esc dismiss".to_string(),
        ]
    }

    #[test]
    fn oc_snapshot_parses_multi_select_page() {
        let snap = opencode_question_screen_snapshot(&multi_page()).expect("题页必须出快照");
        assert_eq!(
            snap.checked,
            vec![
                Some(false),
                Some(false),
                Some(true),
                Some(false),
                Some(false),
                Some(false)
            ]
        );
        assert!(snap.heading.contains("你希望这次优化重点放在哪些方面"));
        // own answer 鲜态：present=true、text=None
        assert!(snap.free_text_present);
        assert_eq!(snap.free_text, None);
    }

    #[test]
    fn oc_snapshot_saved_content_surface() {
        let mut saved = multi_page();
        saved[8] = "6. [v] 出场的人物需要增加一些".to_string();
        let snap = opencode_question_screen_snapshot(&saved).expect("已保存题页必须出快照");
        assert_eq!(snap.checked[5], Some(true));
        assert_eq!(snap.free_text.as_deref(), Some("出场的人物需要增加一些"));
    }

    #[test]
    fn oc_snapshot_parses_single_select_page() {
        // 2026-10-05 单选页实拍形态（34808 会话 + 用户截图）：无勾选框、
        // own 行内容为手写残留
        let single = vec![
            "Questions".to_string(),
            "使用场景".to_string(),
            "这个页面的主要使用场景是？（单选）".to_string(),
            "1. 手机浏览器独立打开".to_string(),
            "2. 手机+电脑都要好看".to_string(),
            "3. 嵌入到其他网页里".to_string(),
            "4. 能否融合上述2篇的".to_string(),
            "⇆ tab  ↑↓ select  enter confirm  esc dismiss".to_string(),
        ];
        let snap = opencode_question_screen_snapshot(&single).expect("单选页必须出快照");
        assert_eq!(
            snap.checked,
            vec![Some(false), Some(false), Some(false)],
            "单选未选行 = Some(false)（无任何标记）；own 行不计——长度必须等于\
             选项数，否则 advance 回执快照被前端归属校验整体拒收（2026-10-06 修复）"
        );
        assert_eq!(snap.free_text.as_deref(), Some("能否融合上述2篇的"), "残留必须同步（覆盖写入按钮的数据源）");
        assert!(snap.free_text_present);
        assert!(snap.heading.contains("这个页面的主要使用场景是"));
    }

    #[test]
    fn oc_snapshot_single_select_fresh_placeholder_is_none_text() {
        let fresh = vec![
            "Questions".to_string(),
            "1. alpha".to_string(),
            "2. bravo".to_string(),
            "3. charlie".to_string(),
            "4. Type your own answer".to_string(),
            "↑ select  enter submit  esc dismiss".to_string(),
        ];
        let snap = opencode_question_screen_snapshot(&fresh).expect("鲜态单选页必须出快照");
        assert!(snap.free_text_present);
        assert_eq!(snap.free_text, None, "占位标签 = 未填");
        // 3 选项 + own 行：checked 只含选项行（own 行不计，2026-10-06 修复）；
        // 未选行 = Some(false)（裸 ✓ 定案后与括号页同口径）
        assert_eq!(snap.checked, vec![Some(false), Some(false), Some(false)]);
    }

    #[test]
    fn oc_snapshot_single_select_checkmark_row_is_true_and_own_tail_stripped() {
        // 2026-10-06 活体定案（PID 1880 dump 行35/行41）：单选选中行尾渲染
        // " ✓"（U+2713，无括号）；own 行是当前答案时同样带 ✓
        let marked = vec![
            "Questions".to_string(),
            "你说的「上述2篇」具体指什么？".to_string(),
            "1. 两张 SVG 插画".to_string(),
            "2. 两篇悬疑小说 ✓".to_string(),
            "3. 画面里的两/三组角色".to_string(),
            "4. 其他（我来说明）".to_string(),
            "5. 出场的人物需要增加一些 ✓".to_string(),
            "⇆ tab  ↑↓ select  enter confirm  esc dismiss".to_string(),
        ];
        let snap = opencode_question_screen_snapshot(&marked).expect("✓ 页必须出快照");
        assert_eq!(
            snap.checked,
            vec![Some(false), Some(true), Some(false), Some(false)],
            "行尾裸 ✓ = 选中（DigitKey 翻转判定 Some(false)→Some(true) 的数据源）"
        );
        // own 行 ✓ 尾剥掉——卡面「已写入」不显示对勾
        assert_eq!(snap.free_text.as_deref(), Some("出场的人物需要增加一些"));
    }

    #[test]
    fn oc_snapshot_single_select_none_on_broken_numbering() {
        // 会话正文里的编号列表（断号）不得误判为单选题页
        let list = vec![
            "1. 第一点".to_string(),
            "3. 第三点".to_string(),
            "4. 补充".to_string(),
        ];
        assert_eq!(opencode_question_screen_snapshot(&list), None);
    }

    #[test]
    fn kimi_snapshot_parses_multi_page_with_other_residue() {
        // 2026-10-05 探测批 K 实拍形态（evidence/kimi-multi/probe-scr-km 原样词形）
        let kimi_page = vec![
            " Shui guo    Submit".to_string(),
            "".to_string(),
            " ? ni xi huan na xie shui guo?".to_string(),
            "".to_string(),
            "  [ ] A apple".to_string(),
            "        Xuan xiang A: apple".to_string(),
            "  [?] B banana".to_string(),
            "  [ ] C cherry".to_string(),
            "  [?] Other: 有个想法".to_string(),
            "".to_string(),
            "  ↑↓ select  1-4 / ? toggle  ←/→/tab switch  esc cancel".to_string(),
        ];
        let snap =
            kimi_question_screen_snapshot(&kimi_page).expect("kimi 多选页必须出快照");
        assert_eq!(
            snap.checked,
            vec![Some(false), Some(true), Some(false)],
            "逐选项勾选态（Other 行不计入 checked）"
        );
        assert_eq!(snap.free_text.as_deref(), Some("有个想法"), "Other 残留同步");
        assert!(snap.free_text_present);
        // heading = `? ` 题干行（2026-10-06 刷新对位修复：不再拼 tab 栏杂讯——
        // 前端 findQuestionByHeading 拿它与载荷 question 字段 exact/partial 对位）
        assert!(
            snap.heading.contains("ni xi huan na xie shui guo"),
            "heading 应为题干行剥 ? 前缀：{:?}",
            snap.heading
        );
        assert!(!snap.heading.contains("Submit"), "不含 tab 栏杂讯");
    }

    #[test]
    fn kimi_snapshot_fresh_other_is_none_text() {
        let fresh = vec![
            "  [ ] A apple".to_string(),
            "  [ ] B banana".to_string(),
            "  [ ] Other".to_string(),
            "  ↑↓ select  1-4 / ? toggle  ←/→/tab switch  esc cancel".to_string(),
        ];
        let snap = kimi_question_screen_snapshot(&fresh).expect("鲜态必须出快照");
        assert_eq!(snap.free_text, None, "鲜态 Other = 未填");
        assert!(snap.free_text_present);
    }

    #[test]
    fn kimi_snapshot_none_without_other_row() {
        // 无 Other 行（模型未给 Other 选项）→ 仍出快照（free_text_present=false）
        let no_other = vec![
            "  [ ] A apple".to_string(),
            "  [ ] B banana".to_string(),
            "  ↑↓ select  1-4 / ? toggle  ←/→/tab switch  esc cancel".to_string(),
        ];
        let snap = kimi_question_screen_snapshot(&no_other).expect("有选项行应出快照");
        assert_eq!(snap.checked, vec![Some(false), Some(false)]);
        assert!(!snap.free_text_present);
        // 无 checkbox 行（普通正文）→ None
        assert_eq!(kimi_question_screen_snapshot(&["1. 普通".to_string()]), None);
    }

    #[test]
    fn oc_snapshot_none_on_confirm_page() {
        let confirm = vec![
            "Questions".to_string(),
            "⇆ tab  enter submit  esc dismiss".to_string(),
        ];
        assert_eq!(opencode_question_screen_snapshot(&confirm), None);
    }
}
