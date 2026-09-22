//! AskUserQuestion 问答卡片内核（批次乙 T8，claude 先行）：questions 解析 +
//! 应答注入序列构造（纯函数，跨平台可测；执行一律经 [`crate::inject::engine::Injector`]
//! 缝——`locate_and_send_key_spec` 按族分发，与审批端点同一装配）。
//!
//! # 实机探测定案（2026-09-21，本文件序列构造的唯一依据）
//!
//! 档案：`research/refs/phase2-消息注入/2026-09-21-claude-askuserquestion-按键语义
//! 探测.md`（实机三重证据：注入日志 + 截图 + 会话 JSONL）。键位语义：
//! - **单选 · 数字**：数字键（字符或 VK 形态均可）直接勾选并提交，**无需回车**，
//!   **严禁后补 Esc**（K1/K2——后补 Esc 落入模型 busy 回合 = 中断模型回合）；
//! - **单选 · Esc**：取消/拒绝整个问题（模型收 is_error=true 拒绝回执，K3）
//!   ——对应「取消」按钮；
//! - **多选 · 数字**：逐个**切换勾选**，不提交（K8）；
//! - **多选 · Enter**：**不是提交**——是切换光标所在行（K9，实测抓获的反直觉点），
//!   因此提交绝不能只发 Enter；
//! - **多选 · 提交**：**三段式**（K10）——VT 下箭头 ×(选项数+1)（从首选项行跨过
//!   TUI 自动追加的 "N. Type something." 行到 Submit 行）→ VK 回车进 Review 确认屏
//!   → 数字 '1' 确认；
//! - **自由文本**：UI 自动追加 "N. Type something." 行，先注入该行数字（仅定位，
//!   K4）→ 文本字符 → VK 回车（K6）；未定位直接打字无效（K7）。**卡片 v1 不做
//!   自由文本注入，引导普通消息发送**（前端文案，本文件不承载）；
//! - **多问题数组**（questions.length>1）：翻页键序（←→）**未测**——v1 只读展示+
//!   引导终端作答，端点对 questions != 1 的注入请求直接拒绝（不超证据出手）；
//! - **空闲对照**：问题未出现时数字只是进输入行（K11）——注入前必须确认问答在场
//!   （标记/通道 B 判定，见 remote/api.rs）。
//!
//! 注意：questions 载荷里只有模型给的 options——"Type something." 行是 TUI 渲染时
//! 自动追加的，**不在** questions JSON 中（探测档案 §2 UI 结构备注），故本模块的
//! 选项数 n 即纯模型选项，三段式下箭头数 = n+1 恰好跨到 Submit 行。
//!
//! # 丁T5：提交与自由作答改为**阶段机闭环**（§2.3 裁4 / §2.4 裁3）
//!
//! 批次丙的 [`answer_key_sequence`] 是「一次算步进 + 盲发序列」：多选提交直接把
//! `down ×(n+1) → enter → '1'` 投完，**中途没有任何屏读复核**——若第一条 down 被
//! 吞（或 TUI 行序与我们的假设不一致），后面的 `enter` 会落在**别的行**上，而 `'1'`
//! 更是在**没确认 Review 屏在场**的情况下发出（正是 §2.3 点名禁止的形态）。
//! 故本模块新增两条**闭环编排**（[`run_submit_stages`] / [`run_free_text_stages`]），
//! 读屏与发键全部经 [`MenuTerminal`] 注入——控制流因此可在门禁内用脚本化屏序列
//! 覆盖（同 [`crate::inject::mode::run_menu_stages`] 的做法与理由）。
//!
//! 两条编排的**判据来源**（真机取证，勿凭源码猜测）：
//!
//! | 判据 | 实测原文 | 证据 |
//! |---|---|---|
//! | 选项/提交行**焦点标记** | `❯`（U+276F，`pointer`） | `C-s8-cursor-submit-*.png`（光标在 Submit 行）+ claude 二进制 `pointer:"\u276F"` 与 `isFocused → L.pointer` 渲染分支 |
//! | Submit 行文本 | `Submit`（多选；`submitButtonText` 单题形态） | `C-s8-cursor-submit-*.png` + 二进制 |
//! | Review 屏标题 | `Review your answers` | `C-s8-submitted-*.png` + 二进制 `title:"Review your answers"` |
//! | Review 屏副题 | `Ready to submit your answers?` | 同上 |
//! | Review 屏选项 | `1. Submit answers` / `2. Cancel` | 同上（`confirmLabel:"Submit answers"`） |
//! | 完成态 | 对话流出现 `User answered Claude's questions:` | `C-s8-final-*.png` + 二进制 |
//! | 自由作答行 | `N. Type something.`（单选）/`N. Type something`（多选） | `C-s7-digit3-result-*.png` + 二进制 `Sa.multiSelect?"Type something":"Type something."` |
//!
//! 截图证据目录：`%TEMP%\mam-probe-askq-20260921-013027\evidence\`（本批已逐张核对）。

use crate::inject::families::FamilySpec;
use crate::inject::mode::MenuTerminal;

/// 单个选项（questions[].options[] 条目）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// 单个问题（questions[] 条目；multiSelect 缺省 = false——探测档案单选夹具实测形态）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub header: String,
    pub question: String,
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

/// 应答动作（POST /session-question/answer 的 action 域）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerAction {
    /// 单选点选项：注入对应数字（直接勾选并提交，无回车无 Esc）
    Select,
    /// 多选点选项：注入对应数字切换勾选（不提交）
    Toggle,
    /// 多选提交：三段式（down ×(n+1) → enter → '1'）——**丁T5 起走阶段机**
    /// [`run_submit_stages`]，本枚举值只作 wire 词与审计标签用
    Submit,
    /// 取消：Esc（模型收 is_error=true 拒绝回执）
    Cancel,
    /// 自由作答（丁T5 §2.4 入口 1）：数字定位 `Type something` 行 → 文本字符 → 回车
    /// ——**走阶段机** [`run_free_text_stages`]（键序依赖屏读定位，不产静态序列）
    FreeText,
}

impl AnswerAction {
    /// wire 字符串 ↔ 动作（camelCase 请求体用小写词）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "select" => Some(Self::Select),
            "toggle" => Some(Self::Toggle),
            "submit" => Some(Self::Submit),
            "cancel" => Some(Self::Cancel),
            "freeText" => Some(Self::FreeText),
            _ => None,
        }
    }

    /// 审计摘要形态（action#index，编号对齐 UI 从 1 起）
    ///
    /// **自由作答只记动作名，不记正文**——正文是用户答案，审计表存摘要的既有口径
    /// （W5「只存摘要不入全文」）在此更严一档：连截断后的正文都不落，避免把用户
    /// 回答内容二次写进审计库（终端已收到，审计只需证明「发生过一次自由作答」）。
    pub fn audit_label(self, index: Option<usize>) -> String {
        match (self, index) {
            (Self::Select, Some(i)) => format!("select#{}", i + 1),
            (Self::Toggle, Some(i)) => format!("toggle#{}", i + 1),
            (Self::Submit, _) => "submit".to_string(),
            (Self::Cancel, _) => "cancel".to_string(),
            (Self::FreeText, _) => "freeText".to_string(),
            // select/toggle 缺 index 在端点参数校验已拦（400），防御形态原样
            (Self::Select, None) => "select".to_string(),
            (Self::Toggle, None) => "toggle".to_string(),
        }
    }
}

/// 解析 tool_input 原文 JSON（helper 问答通道携带 / 通道 B 的 tool_args）中的
/// questions[]。结构不符（缺 questions 数组 / 空 / 条目缺 question 或 options）→
/// None（端点据此回落通道 B 或判不可用）。
/// 宽容口径：header 缺省空串、multiSelect 缺省 false、description 缺省空串——
/// 与探测档案单选夹具的缺省形态一致。
pub fn parse_questions(tool_input_json: &str) -> Option<Vec<Question>> {
    let v: serde_json::Value = serde_json::from_str(tool_input_json).ok()?;
    let arr = v.get("questions")?.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(arr.len());
    for q in arr {
        // question 与 options 是出卡的最小集（缺一即结构不符）
        q.get("question")?.as_str()?;
        let opts = q.get("options")?.as_array()?;
        if opts.is_empty() {
            return None;
        }
        let mut options = Vec::with_capacity(opts.len());
        for o in opts {
            options.push(QuestionOption {
                label: o.get("label")?.as_str()?.to_string(),
                description: o
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        out.push(Question {
            header: q
                .get("header")
                .and_then(|h| h.as_str())
                .unwrap_or_default()
                .to_string(),
            question: q
                .get("question")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            multi_select: q
                .get("multiSelect")
                .and_then(|m| m.as_bool())
                .unwrap_or(false),
            options,
        });
    }
    Some(out)
}

/// 选项序号 → 数字键（UI 编号从 1 起；键域 = 单字符 '1'…'9'——经
/// `locate_and_send_key` 的单字符域分发，字符/VK 形态按族规格决定，探测 K1 实证
/// 两形态均直接提交）。9 选项以上越界 → None（探测档案：超范围数字未测，不出手）
fn digit_key(index: usize) -> Option<String> {
    if index >= 9 {
        return None;
    }
    Some((((index + 1) as u8 + b'0') as char).to_string())
}

/// 应答动作 → 按键序列（探测定案的纯函数化；键名走 `locate_and_send_key(_spec)`
/// 键域：单字符数字 / "enter" / "esc" / "down"——"down" 按族分发为 A 族 VT 序列
/// 或 B 族 VK 形态，探测 K10 实证 VT 形态对 claude 生效）。
///
/// **本函数是 claude 形态**（批次乙 T8 探测定案，K1–K11 实机取证）。跨工具序列见
/// [`answer_key_sequence_for`]——它按会话工具分发，未验工具降级为只读。
/// - Select/Toggle：`index` 越界（≥ 选项数）或 ≥9 → Err；
/// - Submit：仅多选合法（单选点数字即提交，无独立提交步）；序列 = down ×(选项数+1)
///   （跨过 TUI 自动追加的 "Type something." 行到 Submit 行，探测 K10）→ enter 进
///   Review 确认屏 → '1' 确认；
/// - Cancel：单键 esc。
pub fn answer_key_sequence(
    action: AnswerAction,
    index: Option<usize>,
    q: &Question,
) -> Result<Vec<String>, String> {
    match action {
        AnswerAction::Select | AnswerAction::Toggle => {
            let i = index.ok_or_else(|| "缺少选项序号".to_string())?;
            if i >= q.options.len() {
                return Err(format!("选项序号越界：{i}"));
            }
            digit_key(i)
                .ok_or_else(|| format!("选项序号超键域：{i}"))
                .map(|k| vec![k])
        }
        AnswerAction::Submit => {
            if !q.multi_select {
                return Err("submit 仅用于多选题".to_string());
            }
            // **丁T5 起本分支不再产出键序**：多选提交改走**阶段机闭环**
            // （[`run_submit_stages`]——每段屏读复核后才推进）。保留一个显式 Err 而
            // 不是删掉分支，是为了让「谁再想盲发三段式」立刻撞上这条判据：调用方
            // 若改回 `answer_key_sequence` 会拿到可读的 Err，而不是悄悄发出六键。
            // 原实现（批次丙）的盲发序列 = down ×(n+1) → enter → '1'，其风险见模块
            // 文档「丁T5」小节（`'1'` 在没确认 Review 屏在场时发出）。
            Err("多选题提交必须经阶段机（run_submit_stages）——盲发序列已废弃".to_string())
        }
        AnswerAction::Cancel => Ok(vec!["esc".to_string()]),
        // 自由作答**不产键序**：它的键序依赖屏读（定位数字取自屏上编号），由
        // [`run_free_text_stages`] 在门禁可覆盖的编排里产生。此处显式 Err 而非
        // `unreachable!`：万一有调用方把它接到这里，必须拿到可读的拒绝。
        AnswerAction::FreeText => {
            Err("自由作答必须经阶段机（run_free_text_stages）——键序依赖屏读定位".to_string())
        }
    }
}

/// 问答注入键序的**工具支持面**（批次丙 T3）：哪些工具已有实机取证的键序、哪些
/// 只能只读展示。**未验不出键**（探测纪律：结论不得超过证据）。
///
/// | 工具 | 键序证据 | 状态 |
/// |---|---|---|
/// | claude | 2026-09-21 按键语义探测 K1–K11（本机实机，三重证据） | 全支持（单/多选 + cancel） |
/// | opencode | 2026-09-20 跨工具矩阵 §2.2（本机实机 ×2：数字单键即选即交；↓+Enter） | 单选 select；多选/取消**未测** → 拒 |
/// | kimi | 同上 §2.3（本机实机 ×2：数字 → Review 屏 → 数字确认的**两段式**） | 单选 select 两段式；多选/取消未测 → 拒 |
/// | codex | **2026-09-21 实机补测**（T3 条件项，见下） | 单选 select 数字即选即交；多选/取消 → 拒 |
///
/// **codex 2026-09-21 实机补测（矩阵遗留条件项已补齐）**：档案
/// `research/refs/phase2-消息注入/2026-09-21-codex-request_user_input-键位实机补测.md`
/// （conhost + codex 0.155.1 + 三重证据）。定案：
///
/// - **数字单键即选即交**（'2'/'1' 三取样，注入→`function_call_output.answers` 落盘
///   ≈277–363ms）——与 claude K1 同形，故并入 [`QuestionKeyProfile::SingleDigitSubmit`]；
/// - ↓+Enter 亦可（移动高亮 + 提交）；
/// - 多选形态本轮未触发（弹窗均为单选）→ toggle/submit 拒绝（未验不出键）；
/// - 确认屏（未答完提交时弹）**数字无效**、只认 Enter/Esc——与源码级记载背离，
///   以实测为准（本档不出手该屏，仅记录）。
///
/// **cancel 不代按**：codex 的 Esc 是「中断整个回合」（`function_call_output` 是纯
/// 字符串 "aborted by user…" + `turn_aborted` 事件，**不是** claude 那种 is_error
/// 拒答回执）——语义破坏性远大于 claude K3（会打断模型正在做的事），故不出手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionKeyProfile {
    /// 全键序已验：单选直选、多选 toggle+三段式提交、esc 取消
    ClaudeFull,
    /// 单选已验（数字即选即交）；多选/提交/取消未测 → 这些动作拒绝
    SingleDigitSubmit,
    /// 单选两段式（数字选中 → Review 屏 → 数字确认）；多选未测
    TwoPhaseSelect,
    /// 未验：不出任何键（只读卡）
    ReadOnly,
}

/// 会话工具 → 问答键序档（纯函数，可测；未知工具保守走只读）
///
/// # 丁T2 待复核项：kimi **question 类**的数字通道可靠性（R1 Important-1）
///
/// **证据现状（如实申报）**：R1 的证伪（`'2'+Enter` 误批准、`'3'` 单键拒绝）针对的是
/// kimi 的 **`plan_review` 审批框**（`inject::dialog` 的导航确认族，approve 端点走
/// 「屏读 + ↓×k + Enter」，本批已按此实现）——**不是** question 类框。question 类的
/// 数字可靠性**没有**同等强度的实机证据（唯一记载是 2026-09-20 矩阵 §2.3 的双路径
/// 取样：数字 '2' → Review 屏 → '1' 提交，与 Enter 路径并列）。
///
/// **本轮处置：维持 [`QuestionKeyProfile::TwoPhaseSelect`]，不超证据改行为**——
/// 把 question 类一并改成导航确认会**改掉一个矩阵实测过并已上线的行为**，而改它的
/// 依据（plan_review 的证伪）属于**另一个对话框族**（两者渲染不同：审批框是 `▶ 1. …`
/// 三键单选，question 框是 `→ [1] …` 选项卡 + Review 确认屏）。以一族证据改另一族
/// 行为，正是「结论超过证据」的反面。
///
/// **残余风险与缓解**（不隐瞒）：若 kimi 的 question 类数字通道与 plan_review 同样
/// 不可靠，则 `[数字, "enter"]` 会答错选项（用户点「拒绝」类选项却选中高亮行）。
/// 缓解：① 两段式的第二段是 **Enter**（提交**当前高亮行**）——数字被吞时提交的是
/// 用户注入前的原位行，属**可见可复核**的偏差（对话框在注入后会重绘，用户能立刻看到
/// 结果）；② 本档只对**单选**放行（多选/取消已拒）；③ 下批实机探测（`#[ignore]` 占位
/// 见本模块 `live_probe_tests`）定案后按族收口。
///
/// **下批定案的两个分支**（探测后二选一，勿凭猜提前改）：
/// - 证实数字不可靠 → 与 approve 侧同族化：question 类也走「屏读高亮位 + ↓×k + Enter」
///   （`inject::dialog::navigation_sequence` 复用），并补反例夹具；
/// - 证实数字可靠（或仅 plan_review 特殊）→ 保持本档，把探测档案挂到本注释。
pub fn question_key_profile(tool: &str) -> QuestionKeyProfile {
    match tool {
        "claude" => QuestionKeyProfile::ClaudeFull,
        // 矩阵 §2.2 实测：VK 数字 '2' 单键即选中并提交；↓+Enter 亦可（同为直选即交）
        "opencode" => QuestionKeyProfile::SingleDigitSubmit,
        // 矩阵 §2.3 实测：数字选中 → 自动进 "Review your answer before submit"
        // 确认屏（[1] Submit / [2] Cancel）→ 数字 '1' 提交。**两段式**
        "kimi" => QuestionKeyProfile::TwoPhaseSelect,
        // T3 实机补测（2026-09-21）：数字单键即选即交（三取样），与 opencode 同档；
        // cancel 因 Esc=中断回合而拒（见上方表格注释）
        "codex" => QuestionKeyProfile::SingleDigitSubmit,
        _ => QuestionKeyProfile::ReadOnly,
    }
}

/// 跨工具应答序列（批次丙 T3 分发点；端点调用它而非直接调
/// [`answer_key_sequence`]）。
///
/// **kimi 两段式**（矩阵 §2.3 实机）：数字选中后 TUI 进 Review 确认屏，需再按 '1'
/// 才提交——故 select 序列是 `[数字, "1"]`（两键）。取消键 kimi 未测 → 走只读
/// （返回 Err，端点 409，前端只读卡）。
///
/// **codex ReadOnly**：任何动作都 Err（端点据此拒注入，前端只读卡）——「未验不出键」。
pub fn answer_key_sequence_for(
    tool: &str,
    action: AnswerAction,
    index: Option<usize>,
    q: &Question,
) -> Result<Vec<String>, String> {
    match question_key_profile(tool) {
        QuestionKeyProfile::ClaudeFull => answer_key_sequence(action, index, q),
        QuestionKeyProfile::SingleDigitSubmit => match action {
            // 矩阵 §2.2：数字单键即选即交（与 claude 同形）；多选 toggle 同键（若
            // 该工具支持多选，勾选语义未测——故 multi_select 的 toggle 也拒）
            AnswerAction::Select => {
                if q.multi_select {
                    return Err("opencode 多选未实测，不出键".to_string());
                }
                answer_key_sequence(action, index, q)
            }
            AnswerAction::Toggle => Err("opencode 多选未实测，不出键".to_string()),
            // 提交（多选三段式）未测
            AnswerAction::Submit => Err("opencode 多选提交未实测，不出键".to_string()),
            // esc dismiss 有 footer 提示但**未实机验证**（矩阵 §5 未测面）→ 不出键
            AnswerAction::Cancel => Err("opencode 取消未实测，不出键".to_string()),
            // 自由作答序列未定案（§2.4 列的是 `own answer → 文本 → Enter`，未实机）→ 拒
            AnswerAction::FreeText => Err("opencode 自由作答未实测，不出键".to_string()),
        },
        QuestionKeyProfile::TwoPhaseSelect => match action {
            AnswerAction::Select => {
                if q.multi_select {
                    // E4：多选的「点选项」= **单次切勾**（数字），与 Toggle 同键——
                    // 前端多选卡逐项点按即逐项切勾（单次，无尾随键）
                    return kimi_toggle_keys(index, q);
                }
                // 两段式：数字**选中**（直达 Review 汇总屏）→ **Enter 确认**
                //
                // **2026-09-21 实机复验修正**（本轮补跑的 T5/T6 探测抓获）：
                // 原实现按 2026-09-20 矩阵记载发 `[数字, "1"]`（数字选中后按 '1'
                // 提交）——实机证明**第二个数字无效**，确认键是 **Enter**：
                // kimi 对话框底栏原文 `↑/↓ select · 1/2/3 choose · ↵ confirm`
                // （证据 `%TEMP%\mam-probe-c3-20260921-150000\evidence\
                // screen-t5-kimi-ready-before.txt` 行 26）。实测：注入 '1' 后对话框
                // **仍在**（高亮项不变），再注入 Enter 才关闭并推进回合
                // （screen-kimi-after-approve.txt → screen-kimi-after-enter-confirm.txt）。
                let i = index.ok_or_else(|| "缺少选项序号".to_string())?;
                if i >= q.options.len() {
                    return Err(format!("选项序号越界：{i}"));
                }
                let digit = digit_key(i).ok_or_else(|| format!("选项序号超键域：{i}"))?;
                Ok(vec![digit, "enter".to_string()])
            }
            // E4：多选切勾 = **单次数字**（N7 修复——`[数字,回车]` 双切抵消净零，
            // 用户实测 KIMI3 缺陷形态；戊探B ×3 证数字单次切勾可靠）
            AnswerAction::Toggle => kimi_toggle_keys(index, q),
            // E4：多选提交走阶段机（run_kimi_submit_stages——tab → Review 汇总屏 →
            // 屏上编号确认），静态序列已废弃
            AnswerAction::Submit => Err(kimi_submit_static_keys_refused()),
            AnswerAction::Cancel => Err("kimi 取消未实测，不出键".to_string()),
            // E4：自由作答走阶段机（run_kimi_free_text_stages——Other 行数字 →
            // 打字 → 回车保存 → Review → 确认），键序依赖屏读不产静态序列
            AnswerAction::FreeText => {
                Err("kimi 自由作答必须经阶段机（run_kimi_free_text_stages）".to_string())
            }
        },
        QuestionKeyProfile::ReadOnly => Err(format!("{tool} 问答键序未实测，只读展示")),
    }
}

// ============================================================
// 批次戊 E4：kimi 问答全链（N7 双切抵消修复 + A3 多题禁尾 Enter + Other 自由作答）
// 定案输入 = 键序大词典 §3 问答节（清淤版）+ 戊探B + 用户 K-5 实录；spec §2
// ============================================================

/// kimi Review 汇总屏副题锚（K-5 实录 / 戊探B 实机逐字 `Ready to submit your answers?`；
/// 单题 Review 屏同样含此行——screen-ki1-b1-after-char3.txt 原件）。
pub const KIMI_REVIEW_SUMMARY_ANCHOR: &str = "ready to submit your answers?";
/// kimi 确认后**终态锚**（transcript `● Collected your answers`，戊探B M1/M2 提交后
/// 屏读原件逐字；落账侧另有 wire `interaction.resolved` 对账）。
pub const KIMI_ANSWERED_ANCHOR: &str = "collected your answers";
/// kimi Other（自由作答）行标签（`→ [5] Other` 行形；多选形态 Other 行无编号
/// `[/] Other`——故自由作答只对**单选**放行，多选到终端作答）。
pub const KIMI_OTHER_ROW_LABEL: &str = "other";

/// kimi Review 汇总屏**在场**判定（纯函数）：副题锚在场，且存在 `[N] <文字>` 形态的
/// 确认项行（`→ [1] Submit` / `[2] Cancel`——戊探B 原件行形）。
///
/// **为什么不用 claude 的 [`probe_review_screen`]**：claude 的确认项是 `1. Submit
/// answers`（编号点隔形态 + "submit answers" 词），kimi 是**方括号编号** `[1] Submit`
/// （无 "answers" 词）——判据同源不同形，各自锚定各家的真机原文。
pub fn kimi_review_present(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.to_lowercase().contains(KIMI_REVIEW_SUMMARY_ANCHOR))
        && kimi_review_confirm_digit(lines).is_some()
}

/// kimi Review 确认项的**确认键**（`→ [1] Submit` → `"1"`；戊探B：确认键 char '1'
/// ×3 / VK '1' ×2 / VK Enter ×2 全通——取编号键最稳）。读不到 → None（调用方中止）。
pub fn kimi_review_confirm_digit(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter_map(|l| parse_kimi_bracket_row(l))
        .find(|(_, text)| text.to_lowercase().contains("submit"))
        .map(|(digit, _)| digit)
}

/// kimi **Other 行定位**（自由作答入口）：`[N] Other` 行形 → 定位数字键（如 `"5"`）。
/// 多选形态的 Other 行无编号（`[/] Other`）→ None（调用方如实拒绝：自由作答仅单选）。
pub fn locate_kimi_other_digit(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .filter_map(|l| parse_kimi_bracket_row(l))
        .find(|(_, text)| text.to_lowercase().trim().starts_with(KIMI_OTHER_ROW_LABEL))
        .map(|(digit, _)| digit)
}

/// kimi 终态在场（提交完成判据）
pub fn kimi_answered_present(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.to_lowercase().contains(KIMI_ANSWERED_ANCHOR))
}

/// kimi 方括号行形解析：`→ [1] Submit` / `  [2] Cancel` → `("1", "Submit")`。
/// 行首允许空白与 `→`/`❯` 引导符；编号必须在方括号内（1 位数字），后随空白与文字。
fn parse_kimi_bracket_row(line: &str) -> Option<(String, &str)> {
    let t = line
        .trim()
        .trim_start_matches(['\u{2192}', '\u{276f}'])
        .trim_start();
    if !t.starts_with('[') {
        return None;
    }
    let close = t.find(']')?;
    let num = &t[1..close];
    if num.len() != 1 || !num.chars().next()?.is_ascii_digit() {
        return None;
    }
    let text = t[close + 1..].trim();
    if text.is_empty() {
        return None;
    }
    Some((num.to_string(), text))
}

/// kimi **多选切勾键序**（N7 修复的核心锁）：切勾 = **单次数字**（char/VK 两形态皆可，
/// 戊探B ×3）——`[数字, 回车]` 序列=勾上又被回车切掉、净效果零（用户实测 KIMI3 缺陷
/// 形态），**禁止尾随回车**。
pub fn kimi_toggle_keys(index: Option<usize>, q: &Question) -> Result<Vec<String>, String> {
    let i = index.ok_or_else(|| "缺少选项序号".to_string())?;
    if i >= q.options.len() {
        return Err(format!("选项序号越界：{i}"));
    }
    digit_key(i)
        .map(|k| vec![k])
        .ok_or_else(|| format!("选项序号超键域：{i}"))
}

/// kimi **多题 DigitAdvance 键序**（K-5 定案；A3 禁令的锁）：题屏数字=**直选+自动推进
/// 下一题**——序列就是 `[数字]`，**禁止尾随 Enter**（Enter 会误作用在下一题/Review 屏，
/// 矩阵 §6.6-A3）。单题与多题共用数字直选；差异只在「不补确认键」。
pub fn kimi_digit_advance_keys(index: Option<usize>, q: &Question) -> Result<Vec<String>, String> {
    // 键序与切勾同形（单次数字）；独立成函数是为了判据可读与各自演化（N7/A3 的
    // 锁分别钉在两处的测试上，共享实现不共享语义）
    kimi_toggle_keys(index, q)
}

/// kimi 多选**提交**的静态键序**已废弃**（走阶段机 [`run_kimi_submit_stages`]）——
/// 与 claude 的 [`answer_key_sequence`] Submit 分支同一纪律：谁再想盲发立刻撞上可读 Err。
pub fn kimi_submit_static_keys_refused() -> String {
    "kimi 多选提交必须经阶段机（run_kimi_submit_stages）——盲发序列已废弃".to_string()
}

/// kimi 多选/多题**提交阶段机**（批次戊 E4；读屏/发键/等待全走闭包——门禁内脚本化
/// 屏序列可覆盖，抽法理由同 [`run_submit_stages`]）。
///
/// # 各段与中止点（每步发键前屏读）
///
/// 1. **Review 已在场？**：多题流末题答完 TUI **自动**进 Review 汇总屏（K-5），多选
///    单题则需要 `tab` 一次直达（戊探B tab→Submit 定案）。先读一屏：已在场 → **跳过
///    tab**（在 Review 屏上再按 tab 会切页离开——危害防御）；不在场 → 发 `tab`；
/// 2. **Review 汇总屏段**：轮询 [`kimi_review_present`]（副题锚 + `[N]` 确认项）。
///    窗内未出现 → 中止，**不发确认键**（与 claude 的「未见 Review 不发数字」同一纪律）；
/// 3. **确认段**：确认键 = 屏上编号（[`kimi_review_confirm_digit`]，不硬编码）；
/// 4. **终态段**：轮询 [`KIMI_ANSWERED_ANCHOR`]——未见**不是失败**
///    （`receipt_seen = Some(false)`，调用方如实下发「请人工核对」；落账侧另有 wire
///    `interaction.resolved` 对账）。
pub fn run_kimi_submit_stages<Rd, P, Q, T>(
    mut read: Rd,
    mut poll_review: P,
    mut poll_receipt: Q,
    terminal: &mut T,
) -> Result<SubmitOutcome, StageAbort>
where
    Rd: FnMut() -> Option<Vec<String>>,
    P: FnMut() -> Result<Option<Vec<String>>, String>,
    Q: FnMut() -> Result<Option<Vec<String>>, String>,
    T: MenuTerminal,
{
    let mut sent_keys: Vec<String> = Vec::new();
    // 1. Review 已在场？（多题自动推进形态）——读不到屏 → 中止零按键
    let initial = read().ok_or_else(|| {
        StageAbort::screen("kimi 提交：读不到屏幕——已中止，未发任何键；请人工核对终端")
    })?;
    if !kimi_review_present(&initial) {
        terminal
            .send("tab")
            .map_err(|e| StageAbort::delivery(format!("tab 投递失败（{e}）")))?;
        sent_keys.push("tab".to_string());
        terminal.settle();
    }
    // 2. Review 汇总屏（未见即中止，不发确认键）
    let review = poll_review().map_err(StageAbort::screen)?.ok_or_else(|| {
        StageAbort::screen(format!(
            "已发 tab 但屏上未出现 Review 汇总屏（未见「{KIMI_REVIEW_SUMMARY_ANCHOR}」）——已中止，未发确认键；请人工核对终端"
        ))
    })?;
    if !kimi_review_present(&review) {
        return Err(StageAbort::screen(
            "Review 汇总屏形态不符（锚或确认项缺失）——已中止，未发确认键；请人工核对终端",
        ));
    }
    // 3. 确认键 = 屏上编号（`→ [1] Submit` → '1'）
    let confirm_key = kimi_review_confirm_digit(&review).ok_or_else(|| {
        StageAbort::screen(
            "Review 汇总屏在场但读不到确认项编号——已中止，未发确认键；请人工核对终端",
        )
    })?;
    terminal
        .send(&confirm_key)
        .map_err(|e| StageAbort::delivery(format!("确认键 {confirm_key} 投递失败（{e}）")))?;
    sent_keys.push(confirm_key);
    terminal.settle();
    // 4. 终态（未见不是失败——语义与 run_submit_stages 同源）
    let receipt_seen = match poll_receipt() {
        Ok(Some(lines)) => Some(kimi_answered_present(&lines)),
        Ok(None) => Some(false),
        Err(_) => None,
    };
    Ok(SubmitOutcome {
        sent_keys,
        down_steps: 0,
        review_confirmed: true,
        receipt_seen,
    })
}

/// kimi **Other 自由作答阶段机**（批次戊 E4；`Other` 行数字 → 打字 → 回车保存 →
/// Review → 确认——戊探B E-B8 全链定案）。仅**单选**形态放行（多选 Other 行无编号，
/// [`locate_kimi_other_digit`] 恒 None → 第 1 段如实中止）。
pub fn run_kimi_free_text_stages<Rd, P, Q, T>(
    text: &str,
    mut read: Rd,
    mut poll_review: P,
    mut poll_receipt: Q,
    terminal: &mut T,
) -> Result<FreeTextOutcome, StageAbort>
where
    Rd: FnMut() -> Option<Vec<String>>,
    P: FnMut() -> Result<Option<Vec<String>>, String>,
    Q: FnMut() -> Result<Option<Vec<String>>, String>,
    T: FreeTextTerminal,
{
    if text.trim().is_empty() {
        return Err(StageAbort::screen("自由作答文本为空——已中止，未发任何键"));
    }
    let mut sent_keys: Vec<String> = Vec::new();
    // 1. Other 行定位（读不到/无编号 → 中止零按键）
    let first = read().ok_or_else(|| {
        StageAbort::screen("kimi 自由作答：读不到屏幕——已中止，未发任何键；请人工核对终端")
    })?;
    let other_digit = locate_kimi_other_digit(&first).ok_or_else(|| {
        StageAbort::screen(
            "屏读未定位到 Other 行（自由作答入口；多选题的 Other 无编号请到终端作答）——已中止，未发任何键",
        )
    })?;
    terminal
        .send(&other_digit)
        .map_err(|e| StageAbort::delivery(format!("Other 定位键投递失败（{e}）")))?;
    sent_keys.push(other_digit);
    terminal.settle();
    // 2. 打字（字符通道——用户文本绝不进键通道）
    terminal
        .send_text(text)
        .map_err(|e| StageAbort::delivery(format!("作答文本投递失败（{e}）")))?;
    sent_keys.push("<text>".to_string());
    terminal.settle();
    // 3. 回车保存（自动进 Review）
    terminal
        .send("enter")
        .map_err(|e| StageAbort::delivery(format!("保存回车投递失败（{e}）")))?;
    sent_keys.push("enter".to_string());
    terminal.settle();
    // 4. Review 汇总屏（未见即中止，不发确认键）
    let review = poll_review().map_err(StageAbort::screen)?.ok_or_else(|| {
        StageAbort::screen(format!(
            "已保存作答但屏上未出现 Review 汇总屏（未见「{KIMI_REVIEW_SUMMARY_ANCHOR}」）——已中止，未发确认键；请人工核对终端"
        ))
    })?;
    if !kimi_review_present(&review) {
        return Err(StageAbort::screen(
            "Review 汇总屏形态不符——已中止，未发确认键；请人工核对终端",
        ));
    }
    let confirm_key = kimi_review_confirm_digit(&review).ok_or_else(|| {
        StageAbort::screen("读不到确认项编号——已中止，未发确认键；请人工核对终端")
    })?;
    terminal
        .send(&confirm_key)
        .map_err(|e| StageAbort::delivery(format!("确认键 {confirm_key} 投递失败（{e}）")))?;
    sent_keys.push(confirm_key);
    terminal.settle();
    // 5. 终态
    let receipt_seen = match poll_receipt() {
        Ok(Some(lines)) => Some(kimi_answered_present(&lines)),
        Ok(None) => Some(false),
        Err(_) => None,
    };
    Ok(FreeTextOutcome {
        sent_keys,
        receipt_seen,
    })
}

// ============================================================
// 丁T5：多选提交阶段机 + 自由作答阶段机（§2.3 裁4 / §2.4 裁3）
// ============================================================

/// 多选题提交屏上的**焦点标记**（claude 的 `pointer` 主题字形）。
///
/// 实测：`C-s8-cursor-submit-*.png` 的 `Submit` 行首有该字形、同屏其它选项行没有
/// （逐行像素比对：标记列的 ink 段只在焦点行出现——`C-s8-digit1-checked-*.png` 的
/// ink 在第 1 选项行、`C-s8-cursor-submit-*.png` 的在 Submit 行）。claude 二进制
/// `pointer:"\u276F"`，渲染分支见其 `rr()` 组件：`isFocused → L.pointer`（否则空格）。
///
/// **为什么不复用 [`crate::inject::dialog`] 的光标标记集合**：那套集合是**跨工具**
/// 的兜底（codex `›` / claude `❯` / kimi `▶` / ASCII `>`），而这里要的是「提交屏这
/// 一行**确实**是焦点行」的判据——把 ASCII `>` 也算进来会让正文里的引用行冒充焦点行。
/// 故本判据单列一字形，且**只对 claude 的提交屏用**（该屏形态已实机取证；其它工具
/// 本就走不到这条编排）。
pub const SUBMIT_FOCUS_MARKER: char = '\u{276F}';

/// 多选题的提交行文本（实机 `Submit`；claude 二进制 `submitButtonText` 单题时为
/// `"Submit"`、多题时为 `"Next"`——v1 只对单题卡放行，故此常量即单题形态）。
pub const SUBMIT_ROW_LABEL: &str = "Submit";

/// Review 确认屏的**标题锚**（实机逐字，见模块文档判据表）。
pub const REVIEW_TITLE_ANCHOR: &str = "review your answers";
/// Review 确认屏的**副题锚**（实机逐字；与标题**任一**在场即判 Review 屏在场）。
///
/// 为什么允许「任一」而不是「必须两者同时」：屏读只读**可见窗口**，而 Review 屏出现
/// 在提问 UI 的位置——用户终端若字号较大/窗口较矮，标题行可能被滚出可见区（副题与
/// 选项行仍在）。两个锚都是该屏的**专属文案**（正常对话流不会出现），取「任一在场」
/// 既保住判据的排他性，又不在矮窗口下误判为缺席。
pub const REVIEW_SUBTITLE_ANCHOR: &str = "ready to submit";
/// Review 确认屏的确认项文本（实机 `1. Submit answers`）。
pub const REVIEW_CONFIRM_ANCHOR: &str = "submit answers";
/// 确认后**终态锚**（实机 `User answered Claude's questions:`——工具回执落进对话流）。
pub const ANSWERED_RECEIPT_ANCHOR: &str = "user answered claude";

/// 自由作答行（TUI 自动追加）的标签文本。**两形态**：单选是 `Type something.`
/// （带句点）、多选是 `Type something`（无句点）——claude 二进制
/// `Sa.multiSelect?"Type something":"Type something."`。判据按**前缀**匹配
/// （`Type something`），故两形态同一判据覆盖。
pub const FREE_TEXT_ROW_LABEL: &str = "Type something";

/// 阶段机中止的**类别**——决定端点回执的形态与审计的措辞（复评 F6-2）。
///
/// # 为什么要分两类（这是回执语义的一部分，不是内部细节）
///
/// 「中止」有两种成因，用户要做的事完全不同：
/// - [`Screen`](Self::Screen)：**屏上形态与预期不符**（读不到屏 / 没有提交行 /
///   Review 屏没出现 / 焦点走不动）。键**没有**发失败，是「按我们的判据不该继续发」
///   → 回执 `failed{aborted:true, stage}`（前端显示中止段 + 引到终端）；
/// - [`Delivery`](Self::Delivery)：**投递本身失败**（`Injector` 返回 Err——终端不可达、
///   控制台附加失败、写超时）。这与阶段判据无关，语义等同批次丙的「投递失败」→
///   回执 `failed{error}`（**不带** `aborted`），审计 `failed:<e>`。
///
/// 混为一谈的后果（本类存在的原因）：投递失败被报成「中止于某段」，用户会去终端找
/// 「形态问题」，而实际问题是通道打不通——那是误导。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageAbort {
    /// 中止类别
    pub kind: StageAbortKind,
    /// 面向用户的整句中文说明（端点直接透传到 `error` 字段）
    pub message: String,
}

/// 见 [`StageAbort`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAbortKind {
    /// 屏读形态不符（阶段判据拒绝继续）
    Screen,
    /// 投递失败（`Injector`/终端通道返回 Err）
    Delivery,
}

impl StageAbort {
    /// 屏读形态不符的中止
    fn screen(message: impl Into<String>) -> Self {
        Self {
            kind: StageAbortKind::Screen,
            message: message.into(),
        }
    }
    /// 投递失败的中止（包装 `Injector` 或终端写入返回的错误文案）
    fn delivery(message: impl Into<String>) -> Self {
        Self {
            kind: StageAbortKind::Delivery,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StageAbort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 阶段机的一次屏读**语义结论**（三态语义与 [`crate::inject::mode::PollStep`] 一致：
/// 可行动 / 可重试 / 形态异常）。载荷类型与那个泛型枚举不同（这里各段要的东西不一
/// 样），故不复用——共用一个会让两套判据的语义混进同一类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenStep {
    /// 目标形态已出现（附该屏行集）
    Ready(Vec<String>),
    /// 本轮还看不到目标形态（**可重试**，`String` = 用户看得懂的原因）
    NotYet(String),
    /// 形态异常（**立即中止**，`String` = 用户看得懂的原因）
    Fatal(String),
}

/// 一行是否是**带焦点标记**的提交行：剥掉行首空白与 [`SUBMIT_FOCUS_MARKER`] 后，
/// 剩余文本（trim）以 [`SUBMIT_ROW_LABEL`] 开头。
///
/// **为什么判「前缀」而不是精确相等**：实机 Submit 行是 `❯   Submit`（标记后有缩进
/// ——见 `C-s8-cursor-submit-*.png`），且该行**没有编号**（与选项行不同）。前缀匹配
/// 同时容忍行尾可能的装饰；而「不含编号」这一形态差异正是它与选项行不会混淆的原因。
fn submit_row_focused(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix(SUBMIT_FOCUS_MARKER) else {
        return false;
    };
    rest.trim_start().starts_with(SUBMIT_ROW_LABEL)
}

/// 屏上是否**存在**提交行（不论焦点在哪）——用于把「没找到提交行」与「提交行在但
/// 焦点不在它上面」两种失败分开报告（回执要能说清卡在哪一段）。
///
/// 两种屏上形态都算「在场」（实机两张截图各取其一）：焦点行 `❯   Submit`、
/// 非焦点行 `    Submit`。
fn submit_row_present(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix(SUBMIT_FOCUS_MARKER).unwrap_or(t);
    t.trim_start().starts_with(SUBMIT_ROW_LABEL)
}

/// **提交屏在场**探测（**测试专用**——生产侧第 1 段轮询只负责「读一屏」，在场判据由
/// [`run_submit_stages`] 自己对 `poll_submit` 的返回调 [`submit_row_present`]）。
///
/// 为什么还要这个测试用变体：脚本驱动的轮询需要「读到提交屏就停」的语义（否则窗尽前
/// 会一路推 cursor，把 Review 屏也吃掉）；生产侧的 `poll_question_stage` 是「只读不判」
/// ——判据单点在编排里。两者的**判据是同一份**（都调 `submit_row_present`），故驱动
/// 语义不同而结论一致；这也是把探测抽成函数、而不是在测试里另写一个 `any()` 的理由
/// （另写就会与编排判据漂移）。
#[cfg(test)]
fn probe_submit_screen(lines: &[String]) -> ScreenStep {
    if lines.iter().any(|l| submit_row_present(l)) {
        return ScreenStep::Ready(lines.to_vec());
    }
    ScreenStep::NotYet("屏上见不到 Submit 行（多选提交入口未画出/被滚出可见窗口）".to_string())
}

/// 提交屏的**走位**探测：给出「焦点行是否已落在提交行」这一结论。
/// - 焦点在提交行 → `Ready`；
/// - 提交行在场但焦点不在它上面 → `NotYet`（还没走到，可继续按 ↓）；
/// - 提交行**不在场** → `NotYet`（可能是 TUI 还没画出/被滚出窗口——由调用方的窗尽
///   逻辑判失败，而不是在这里武断 `Fatal`：多选界面本身可能因窗口高度只显示部分行）。
fn probe_submit_row(lines: &[String]) -> ScreenStep {
    if lines.iter().any(|l| submit_row_focused(l)) {
        return ScreenStep::Ready(lines.to_vec());
    }
    if lines.iter().any(|l| submit_row_present(l)) {
        return ScreenStep::NotYet(
            "提交行在场但焦点不在它上面（光标还没走到 Submit 行）".to_string(),
        );
    }
    ScreenStep::NotYet("屏上见不到 Submit 行（多选提交入口未画出/被滚出可见窗口）".to_string())
}

/// Review 确认屏的**确认项**抽取（判据的单点实现）：屏上**带编号**且文本含
/// [`REVIEW_CONFIRM_ANCHOR`] 的行 → `(屏上编号, 行原文)`。
///
/// **为什么返回编号而不只是行原文**：确认键要发的是**屏上那个编号**（不硬编码 `'1'`
/// ——屏上编号由 TUI 给，抄它比自己编号稳）。编号在这里就解析出来了，调用方拿不到
/// 「行在但数字抽不出」这种中间态（那种状态在本判据下不可达：`parse_option_line`
/// 已经要求行首是数字）——把不变式收在类型里，而不是在下游写一个永不成立的 `ok_or_else`
/// （本仓的「不许空断言」纪律：不可达的失败分支就是虚假的安全感）。
pub(crate) fn review_confirm_items(lines: &[String]) -> Vec<(u32, String)> {
    lines
        .iter()
        .filter_map(|l| crate::inject::dialog::parse_option_line(l))
        .filter(|(_, label, _)| label.to_lowercase().contains(REVIEW_CONFIRM_ANCHOR))
        .map(|(num, label, _)| (num, format!("{num}. {label}")))
        .collect()
}

/// 见 [`review_confirm_items`]（只要行原文的便利视图；判据同一份）
pub(crate) fn review_confirm_lines(lines: &[String]) -> Vec<String> {
    review_confirm_items(lines)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Review 确认屏的一次屏读探测（判据锚见各常量文档）。`Ready` 载荷 = **该屏行集**
/// （与其它探测器同形态）。
/// - 命中「标题锚 ∨ 副题锚」且**恰好一行**确认项 → `Ready(屏)`；
/// - 命中标题/副题但**读不到带编号的确认项** → `Fatal`（Review 屏在场却无法确认，
///   继续发数字就是盲发）；
/// - 确认项**多项**命中（`Submit answers` 与别的含该词的编号行并存）→ `Fatal`（不猜）；
/// - 全都未见 → `NotYet`（可能还没切屏）。
pub(crate) fn probe_review_screen(lines: &[String]) -> ScreenStep {
    let has_title = lines.iter().any(|l| {
        let low = l.to_lowercase();
        low.contains(REVIEW_TITLE_ANCHOR) || low.contains(REVIEW_SUBTITLE_ANCHOR)
    });
    if !has_title {
        return ScreenStep::NotYet(
            "屏上未出现 Review 确认屏（未见「Review your answers」/「Ready to submit」）"
                .to_string(),
        );
    }
    match review_confirm_lines(lines).len() {
        0 => ScreenStep::Fatal(
            "Review 确认屏在场但屏上读不到带编号的确认项（含「Submit answers」的编号行）——已中止（不盲发数字）"
                .to_string(),
        ),
        1 => ScreenStep::Ready(lines.to_vec()),
        n => ScreenStep::Fatal(format!(
            "Review 确认屏上含「{REVIEW_CONFIRM_ANCHOR}」的编号行有 {n} 行（应恰好 1 行）——不猜，已中止"
        )),
    }
}

/// **终态锚**探测：屏上是否出现「本问题已答完」的回执行（[`ANSWERED_RECEIPT_ANCHOR`]）。
///
/// 用于提交/自由作答的最后一段回执核验——**未见不是失败**：回执可能被后续输出刷走，
/// 也可能该版本的文案不同，调用方据此下发 `Some(false)` 而不是谎报完成（同 `mode`
/// 侧 `receipt_seen` 的裁决）。
pub(crate) fn probe_answered_receipt(lines: &[String]) -> bool {
    lines
        .iter()
        .any(|l| l.to_lowercase().contains(ANSWERED_RECEIPT_ANCHOR))
}

/// 多选题提交的**阶段机编排结果**（[`run_submit_stages`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitOutcome {
    /// 各阶段**实际发出**的键序（按发出顺序；中止时含已发出的那部分）
    pub sent_keys: Vec<String>,
    /// 走位段实际按下的 ↓ 次数（0 = 焦点本来就在 Submit 行）
    pub down_steps: usize,
    /// Review 屏是否屏读确认过（`false` 只可能出现在中止路径上——本字段在成功路径
    /// 恒 `true`；保留它是为了让回执能区分「走完闭环」与「半路停住」）
    pub review_confirmed: bool,
    /// 确认后是否屏读到终态回执行（`None` = 整个窗口一次都没读到屏，无法核验）
    pub receipt_seen: Option<bool>,
}

/// **多选题提交的闭环编排**（§2.3 裁4）。读屏/发键/等待全走 [`MenuTerminal`]，
/// 故整条控制流在门禁里可用脚本化屏序列覆盖（同
/// [`crate::inject::mode::run_menu_stages`] 的抽法，理由同源：段与段之间的控制流是
/// 判据的一部分，写在 `#[cfg(windows)]` 的端点闭包里就只有实机能覆盖）。
///
/// # 各段与中止点（**每一步都在发键前屏读**，任何一段不符即中止且不再发键）
///
/// 1. **提交屏段**：屏读确认 `Submit` 行在场（[`probe_submit_screen`]）。不在场 →
///    中止，**零按键**；
/// 2. **走位段**：每次发一个 `↓` → `settle()` → **重读屏复核**；复核判据 = 提交行仍在
///    场（形态未崩）+ 焦点标记已落到提交行（后者成立即停手）。走位次数上限
///    `max_down_steps` 由调用方给（选项数 + 2：跨过模型选项与 TUI 追加的
///    `Type something` 行），超限 → 中止**不发回车**（防死循环、防在错误位置上回车）；
/// 3. **提交段**：焦点确认落在提交行后才发 `enter`（**唯一的回车点**）；
/// 4. **Review 段**：发完 `enter` 后轮询屏读等 Review 屏画出。**未见 → 中止且不发
///    数字**（这正是任务书点名的缺陷：原实现在没确认 Review 屏在场时就发 `'1'`）。
///    在场 → 取屏上确认项原文的**首个数字**作为确认键（屏上编号由 TUI 给，不硬编码）；
/// 5. **终态段**：发确认键后轮询屏读终态锚——**未见不是失败**：
///    `receipt_seen = Some(false)`，调用方如实下发「已按闭环提交，但未屏读到完成回执，
///    请人工核对」。这一段的结论**不**回滚已发生的事实（键确实发出去了）。
///
/// # 参数
///
/// `poll_submit`——第 1 段轮询（**提交屏在场**，不看焦点）。`poll_review`——Review 屏
/// 轮询。二者语义与 [`crate::inject::mode::run_menu_stages`] 的 `poll` 一致：
/// `Ok(Some(屏))` = 目标形态出现；`Ok(None)` = 窗尽未出现；`Err` = 形态异常（立即中止）。
///
/// `poll_receipt`——终态回执轮询，**语义与 `run_menu_stages` 的 `poll_receipt` 逐条
/// 对齐**（本函数不复用那个闭包，但语义必须同源，否则两处的 `receipt_seen` 三态会在
/// 用户可见文案上漂移）：`Ok(Some(屏))` = 屏上出现终态回执行；`Ok(None)` = 窗内读到
/// 了屏但**未命中**回执行（读屏能力正常，只是没看到）；`Err` = 读屏本身不可用。
/// 后两者在编排里都收敛为「无法断言成功」（`Some(false)` / `None`），差别只在措辞。
///
/// 生产实现见 `remote::api` 的对应轮询；测试用脚本化序列。
pub fn run_submit_stages<P, Q, R, T>(
    mut poll_submit: P,
    mut poll_review: Q,
    mut poll_receipt: R,
    terminal: &mut T,
    max_down_steps: usize,
) -> Result<SubmitOutcome, StageAbort>
where
    P: FnMut() -> Result<Option<Vec<String>>, String>,
    Q: FnMut() -> Result<Option<Vec<String>>, String>,
    R: FnMut() -> Result<Option<Vec<String>>, String>,
    T: MenuTerminal,
{
    let mut sent_keys: Vec<String> = Vec::new();
    // ===== 第 1 段：提交屏在场（不在场即中止，零按键）=====
    // poll 的 Err = 形态异常（屏读类）——闭包签名保持既有 `Result<_, String>` 契约，
    // 类别在编排这一层补齐（判据与分类同处一地，调用方不必知道）
    let first = poll_submit().map_err(StageAbort::screen)?.ok_or_else(|| {
        StageAbort::screen(
            "多选提交屏未出现或读不到屏幕（屏上无 Submit 行）——已中止，未发任何键；请人工核对终端",
        )
    })?;
    if !first.iter().any(|l| submit_row_present(l)) {
        return Err(StageAbort::screen(
            "屏读未见到 Submit 行（多选提交入口）——已中止，未发任何键；请人工核对终端",
        ));
    }
    // ===== 第 2 段：走位（每步复核；到位才停）=====
    let mut down_steps = 0usize;
    loop {
        let lines = terminal.read().ok_or_else(|| {
            StageAbort::screen(format!(
                "走位段读不到屏幕（已发 {down_steps} 个下箭头）——已中止，未发回车（不盲提交）"
            ))
        })?;
        match probe_submit_row(&lines) {
            ScreenStep::Ready(_) => break, // 焦点已在提交行
            ScreenStep::NotYet(_) => {}
            ScreenStep::Fatal(why) => {
                return Err(StageAbort::screen(format!("{why}；请人工核对终端")))
            }
        }
        if down_steps >= max_down_steps {
            return Err(StageAbort::screen(format!(
                "已发 {down_steps} 个下箭头仍未把焦点移到 Submit 行（上限 {max_down_steps}）——已中止，未发回车；请人工核对终端"
            )));
        }
        terminal
            .send("down")
            .map_err(|e| StageAbort::delivery(format!("下箭头投递失败（{e}）")))?;
        sent_keys.push("down".to_string());
        down_steps += 1;
        terminal.settle();
    }
    // ===== 第 3 段：提交（焦点已在提交行——唯一的回车点）=====
    terminal
        .send("enter")
        .map_err(|e| StageAbort::delivery(format!("提交回车投递失败（{e}）")))?;
    sent_keys.push("enter".to_string());
    terminal.settle();
    // ===== 第 4 段：Review 屏（未见即中止，**不发数字**）=====
    let review_lines = poll_review().map_err(StageAbort::screen)?.ok_or_else(|| {
        StageAbort::screen(format!(
            "已发回车但屏上未出现 Review 确认屏（未见「{REVIEW_TITLE_ANCHOR}」/「{REVIEW_SUBTITLE_ANCHOR}」）——已中止，未发确认键；请人工核对终端"
        ))
    })?;
    // 复核（轮询拿到的屏再走一次同一判据——轮询闭包与复核用同一份探测器，
    // 不假定「轮询返回的就一定合判据」；形态在两次读之间变化时这里会如实拒）
    if let ScreenStep::NotYet(why) | ScreenStep::Fatal(why) = probe_review_screen(&review_lines) {
        return Err(StageAbort::screen(format!("{why}；请人工核对终端")));
    }
    // 确认项行 → 屏上编号（屏上编号由 TUI 给，不硬编码 '1'）
    // 确认项的唯一性已由上面那次 `probe_review_screen` 复核过（恰 1 行），此处取它，
    // 并把**屏上编号**作为确认键（`review_confirm_items` 直接给出编号与行原文）
    let (confirm_num, confirm_line) = review_confirm_items(&review_lines)
        .into_iter()
        .next()
        .ok_or_else(|| {
            StageAbort::screen(
                "Review 确认屏在场但读不到带编号的确认项——已中止，未发确认键；请人工核对终端",
            )
        })?;
    let confirm_key = confirm_num.to_string();
    log::debug!("问答提交阶段机：Review 确认项「{confirm_line}」→ 确认键 '{confirm_key}'");
    terminal
        .send(&confirm_key)
        .map_err(|e| StageAbort::delivery(format!("确认键 {confirm_key} 投递失败（{e}）")))?;
    sent_keys.push(confirm_key);
    terminal.settle();
    // ===== 第 5 段：终态回执核验（未见**不是失败**）=====
    //
    // 语义（与 `mode::run_menu_stages` 的 `receipt_seen` 同源）：
    // - `Some(true)`：屏上出现终态回执行（工具自己宣布答完——比任何推断强）；
    // - `Some(false)`：**读到了屏但没有回执行**——「可能答完了但没看到确认」，
    //   如实回执 `verified=false` + 请人工核对；
    // - `None`：读屏不可用（`Err`/窗尽无一屏）——无法核验。
    //   三者中后两者都**不**谎报完成，差别只在回执文案的措辞。
    let receipt_seen = match poll_receipt() {
        Ok(Some(lines)) => Some(probe_answered_receipt(&lines)),
        Ok(None) => Some(false),
        Err(_) => None,
    };
    Ok(SubmitOutcome {
        sent_keys,
        down_steps,
        review_confirmed: true,
        receipt_seen,
    })
}

/// 自由作答的**阶段机编排结果**（[`run_free_text_stages`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeTextOutcome {
    /// 实际发出的键（含定位数字与末尾回车；文本以占位符形式出现——不落正文）
    pub sent_keys: Vec<String>,
    /// 是否屏读到终态回执行（`None` = 读不到屏，无法核验）
    pub receipt_seen: Option<bool>,
}

/// 屏上的自由作答行（[`locate_free_text_row`] 的产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeTextRow {
    /// 屏上编号（TUI 给的，作为定位键）
    pub digit: Option<String>,
    /// 该行原文（回执与日志用）
    pub text: String,
}

/// 在屏上定位自由作答行（**第一个**匹配行）。判据：编号行 + 文本以
/// [`FREE_TEXT_ROW_LABEL`] 开头（前缀匹配，覆盖 `Type something` / `Type something.`
/// 两形态）。
///
/// **为什么取第一个**：单题卡只有一行自由作答入口（多题卡的翻页形态 v1 只读，
/// 走不到这条编排）；真出现两行时取第一行与「用户看到的第一个入口」一致。
fn locate_free_text_row(lines: &[String]) -> Option<FreeTextRow> {
    lines.iter().find_map(|line| {
        // 复用 dialog 的编号行解析（行模式与光标标记剥离口径只此一份，
        // 见 `crate::inject::dialog::parse_option_line` 的丁T5 注）
        let (num, label, _hl) = crate::inject::dialog::parse_option_line(line)?;
        if !label.starts_with(FREE_TEXT_ROW_LABEL) {
            return None;
        }
        Some(FreeTextRow {
            digit: Some(num.to_string()),
            text: format!("{num}. {label}"),
        })
    })
}

/// 自由作答的**终端 IO 能力包**——比 [`MenuTerminal`] 多一个**文本通道**
/// （[`MenuTerminal`] 只有单键 `send`）。
///
/// # 为什么必须是独立 trait（裁3 的安全面）
///
/// 「文本」与「键」是两条**语义完全不同**的通道：文本走字符注入（任意字符都只是
/// 字符），键走键通道（`down`/`enter`/`esc` 是控制语义）。若让自由作答复用
/// [`MenuTerminal::send`]，用户文本就会被送进**键通道**——那正是裁3 禁止的
/// 「用户输入被对话框理解成选项选择」的形态之一（K9：多选框的 Enter 是切换勾选）。
/// 类型上分开，写错就编译不过。
pub trait FreeTextTerminal {
    /// 读一屏（`None` = 读不到屏）
    fn read(&mut self) -> Option<Vec<String>>;
    /// 发一个**键**（`Err` = 投递失败，立即中止）
    fn send(&mut self, key: &str) -> Result<(), String>;
    /// 发**文本**（字符通道；实现方须保证这是文本注入而非键注入）
    fn send_text(&mut self, text: &str) -> Result<(), String>;
    /// 按键/文本后给 TUI 重绘留时间
    fn settle(&mut self);
}

/// 三闭包 + 文本闭包 → [`FreeTextTerminal`] 的生产/测试适配器（零语义纯转发，
/// 与 [`crate::inject::mode::Closures`] 同款）。
pub struct FreeTextClosures<R, S, X, W> {
    /// 读屏
    pub read: R,
    /// 发键
    pub send: S,
    /// 文本注入通道
    pub send_text: X,
    /// 步进等待
    pub settle: W,
}

impl<R, S, X, W> FreeTextTerminal for FreeTextClosures<R, S, X, W>
where
    R: FnMut() -> Option<Vec<String>>,
    S: FnMut(&str) -> Result<(), String>,
    X: FnMut(&str) -> Result<(), String>,
    W: FnMut(),
{
    fn read(&mut self) -> Option<Vec<String>> {
        (self.read)()
    }
    fn send(&mut self, key: &str) -> Result<(), String> {
        (self.send)(key)
    }
    fn send_text(&mut self, text: &str) -> Result<(), String> {
        (self.send_text)(text)
    }
    fn settle(&mut self) {
        (self.settle)()
    }
}

/// **自由作答的闭环编排**（§2.4 入口 1，v1 仅 claude 单题卡）。
///
/// # 序列与各段判据（实机 K4–K6 定案 + 本批的屏读闭环）
///
/// 1. **定位段**：屏读找 `N. Type something`（[`FREE_TEXT_ROW_LABEL`]）。找得到即取
///    该行**屏上编号**作定位键（抄屏上编号，不猜 `选项数+1`）；找不到 → 中止且
///    **不发任何键**（直接打字是无效操作，K7 实机抓获）；
/// 2. **文本段**：发定位数字（**仅移动焦点，不提交**，K4）→ `settle()` → **重读复核**
///    焦点确已落上（复核判据 = 该行**仍在场**；claude 的自由作答行获得焦点后会把占位
///    文本换成可编辑输入，行内文本会变但行标签保留）→ 再发文本。**顺序倒过来
///    （先文本后定位）是 K7 的无效操作**；
/// 3. **提交段**：发 `enter`（K6——实测提交该文本为答案）；
/// 4. **终态段**：轮询终态锚，语义同 [`run_submit_stages`] 的第 5 段。
///
/// # 裁3 的安全面（**本函数的存在理由**）
///
/// 用户文本**只经** [`FreeTextTerminal::send_text`]（字符通道）投递，**绝不**走
/// [`FreeTextTerminal::send`]（键通道）——因此文本里的任意字符（含数字、含 `Esc`
/// 字面）都不可能被当成选项选择或控制键。定位数字是**本函数自己产生**的（来自屏上
/// 编号），与用户文本零关系；文本之后**只**跟一个回车（提交），**不带任何数字或
/// Esc**（K2：数字后补 Esc 会中断模型回合）。
///
/// # 参数
///
/// `poll_free_row`：轮询屏读自由作答行（`Ok(Some(屏))` / `Ok(None)` 窗尽 / `Err`）。
/// `poll_receipt`：终态轮询（语义同提交路径）。
/// `text`：**已归一**的用户文本（调用方负责过
/// [`crate::inject::normalize::normalize_newlines`]——注入通道唯一出口的安全面）。
pub fn run_free_text_stages<P, R, T>(
    text: &str,
    mut poll_free_row: P,
    mut poll_receipt: R,
    terminal: &mut T,
) -> Result<FreeTextOutcome, StageAbort>
where
    P: FnMut() -> Result<Option<Vec<String>>, String>,
    R: FnMut() -> Result<Option<Vec<String>>, String>,
    T: FreeTextTerminal,
{
    if text.trim().is_empty() {
        // 「文本为空」是**参数问题**（端点已在入口 400 拦），落到这里属防御：
        // 归为屏读类（形态不符）而非投递类——**没有发生任何投递**
        return Err(StageAbort::screen("自由作答文本为空——已中止，未发任何键"));
    }
    let mut sent_keys: Vec<String> = Vec::new();
    // ===== 第 1 段：自由作答行在场（不在场即中止，零按键——K7：未定位直接打字无效）=====
    // poll 的 Err = 形态异常（屏读类）——闭包签名保持既有契约，类别在这一层补齐
    let lines = poll_free_row()
        .map_err(StageAbort::screen)?
        .ok_or_else(|| {
            StageAbort::screen(format!(
            "屏上未出现自由作答行（「{FREE_TEXT_ROW_LABEL}」）——已中止，未发任何键；请人工核对终端"
        ))
        })?;
    let row = locate_free_text_row(&lines).ok_or_else(|| {
        StageAbort::screen(format!(
            "屏读未定位到自由作答行（「{FREE_TEXT_ROW_LABEL}」）——已中止，未发任何键；请人工核对终端"
        ))
    })?;
    let locate_key = row.digit.ok_or_else(|| {
        StageAbort::screen(format!(
            "自由作答行「{}」里读不到编号——已中止，未发任何键；请人工核对终端",
            row.text
        ))
    })?;
    // ===== 第 2 段：定位（仅移动焦点，不提交）=====
    terminal
        .send(&locate_key)
        .map_err(|e| StageAbort::delivery(format!("定位键 {locate_key} 投递失败（{e}）")))?;
    sent_keys.push(locate_key.clone());
    terminal.settle();
    // **重读复核**：定位键之后必须重新屏读、确认自由作答行**仍在场**（焦点确实落上
    // 去了），而不是盲发作答文本。
    let after = terminal.read().ok_or_else(|| {
        StageAbort::screen(format!(
            "发定位键 {locate_key} 后读不到屏幕——已中止，未发作答文本（不盲打字）；请人工核对终端"
        ))
    })?;
    if locate_free_text_row(&after).is_none() {
        return Err(StageAbort::screen(format!(
            "发定位键 {locate_key} 后屏上已见不到自由作答行——形态与预期不符，已中止，未发作答文本；请人工核对终端"
        )));
    }
    // ===== 第 3 段：文本（唯一走字符通道的一步）=====
    terminal
        .send_text(text)
        .map_err(|e| StageAbort::delivery(format!("作答文本投递失败（{e}）")))?;
    // 审计/回执里的键序用**占位符**记文本（不落正文——正文属用户隐私，且长度可泄露
    // 答案形态）；字符数足以证明「投了什么规模的东西」。
    sent_keys.push(format!("<text:{} chars>", text.chars().count()));
    terminal.settle();
    // ===== 第 4 段：提交（回车；**不带数字、不带 Esc**——K2）=====
    terminal
        .send("enter")
        .map_err(|e| StageAbort::delivery(format!("提交回车投递失败（{e}）")))?;
    sent_keys.push("enter".to_string());
    terminal.settle();
    // ===== 第 5 段：终态回执核验（未见**不是失败**，同提交路径）=====
    let receipt_seen = match poll_receipt() {
        Ok(Some(lines)) => Some(probe_answered_receipt(&lines)),
        Ok(None) => Some(false),
        Err(_) => None,
    };
    Ok(FreeTextOutcome {
        sent_keys,
        receipt_seen,
    })
}

/// 自由作答的**工具支持面**（v1 只对 claude 放行，§2.4）：
/// - claude：K4–K7 实机定案（数字定位 → 文本 → 回车）；
/// - 其余工具的自由作答序列**未定案**（§2.4 列的 codex `tab notes` / opencode
///   `own answer` / kimi `Other` 均未实机取证）→ 端点按 §2.8 降级：前端渲染
///   「请在终端作答」，**不假装能发**。
pub fn free_text_supported(tool: &str) -> bool {
    // E4 起 kimi 支持（Other 行自由作答，戊探B E-B8 全链定案）；形态门仍只放单选
    // （多选 Other 行无编号）——见 [`free_text_shape_supported`]
    matches!(
        question_key_profile(tool),
        QuestionKeyProfile::ClaudeFull | QuestionKeyProfile::TwoPhaseSelect
    )
}

/// 自由作答的**题目形态支持面**（复评 F6-3：多选卡不提供自由作答）。
///
/// # 为什么多选卡不能提供（实机证据，不是保守猜测）
///
/// 多选屏的自由作答行**不是** [`FREE_TEXT_ROW_LABEL`] 的裸标签，而是带**勾选框**的
/// `4. [ ] Type something`——实机截图 `C-s8-cursor-submit-20260921-015844.png`
/// 的第 4 行逐字如此（本批复评时重新核对过该 PNG：`4. [ ] Type something`，
/// 与单选屏 `C-s7-digit3-result-*.png` 的 `3. Type something.` 形态**不同**）。
///
/// 而 [`locate_free_text_row`] 的判据是「编号行的 label 以 `Type something` 开头」——
/// 该行剥掉编号后的 label 是 `[ ] Type something`，**前缀不匹配** → 定位恒失败 →
/// 自由作答在**多选屏上必然中止在 `free-row` 段**。
///
/// 这不是安全问题（中止即零投递，不会误勾选），但**功能对多选不可用**——所以：
/// - **拒绝而不是放宽判据**（本函数的存在理由）：放宽需要「剥掉行首勾选框再匹配」
///   的判据，而勾选框的**确切形态族**（`[ ]` / `[✓]` / 其它宽度对齐变体）没有取证
///   （本批只有一张多选截图与二进制里的 `[" ","]` 拼接代码；单选屏根本不带勾选框）。
///   在证据不足时收紧能力（拒绝）而不是放宽判据，是本仓一贯的「结论不超证据」；
/// - 待办：若后续实机取证勾选框形态族（`#[ignore]` 占位已登记该观测点），再按证据
///   放宽判定并补夹具。
pub fn free_text_shape_supported(q: &Question) -> bool {
    !q.multi_select
}

/// 自由作答形态的静态描述（见 [`free_text_shape`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreeTextShape {
    /// 定位行标签（屏上判据）
    pub locate_label: &'static str,
    /// 提交键（文本之后**唯一**的键——不带数字不带 Esc）
    pub submit_key: &'static str,
    /// **是否只对单题卡成立**（当前恒 `true`——多选屏的行带勾选框，判据不匹配；
    /// 见 [`free_text_shape_supported`] 的实机证据与放宽前提）
    pub single_question_only: bool,
}

/// 自由作答序列的**纯构造快照**（供端点先做「能不能发」的判定与前端契约）。
///
/// 返回「定位行判据 + 提交键」两项的静态描述——**不是**可直接投递的键序（键序由
/// [`run_free_text_stages`] 在屏读之后产生，定位数字取自屏上编号）。存在理由：
/// 端点与测试要对「这条路的形态」做一次**不依赖屏读**的断言（例如「claude 的形状是
/// 数字→文本→回车」「不支持的工具必须 Err」），故给一个纯函数入口。
pub fn free_text_shape(tool: &str) -> Result<FreeTextShape, String> {
    if !free_text_supported(tool) {
        return Err(format!("{tool} 自由作答序列未实测，请在终端作答"));
    }
    Ok(FreeTextShape {
        locate_label: FREE_TEXT_ROW_LABEL,
        submit_key: "enter",
        // **题目形态限制**（复评 F6-3）：本形态只对单题卡成立——多选屏的自由作答行
        // 带勾选框（`4. [ ] Type something`），与 locate_label 的前缀判据不符。
        // 消费方必须同时过 [`free_text_shape_supported`]（端点已如此）。
        single_question_only: true,
    })
}

/// 自由作答注入的**族规格**入口——本编排只对 claude 放行，故规格取
/// `families::family_for("claude")`；显式函数便于测试对「规格确实是 claude 的」下断言
/// （而不是散在端点里写魔法字符串）。
pub fn free_text_family_spec() -> FamilySpec {
    crate::inject::families::family_for("claude").unwrap_or(crate::inject::families::FALLBACK_SPEC)
}

/// 动作可用性门的**拒绝原因**（决定端点回哪个错误码——这是「用户要看到什么」的一部分：
/// 「序号不对」（400，改序号即可）与「这个工具还没实测」（409，去终端做）是两回事）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionRefusal {
    /// 参数与题目形态不合（序号越界 / submit 用在单选题）→ 端点 400 bad_index
    BadParameter(String),
    /// 该工具的这个动作**未实测**（未验不出键）→ 端点 409 tool_readonly
    ToolUnverified(String),
}

impl ActionRefusal {
    /// 供日志与测试读的中文原因
    pub fn reason(&self) -> &str {
        match self {
            Self::BadParameter(s) | Self::ToolUnverified(s) => s,
        }
    }
}

/// **动作可用性门**（丁T5）：该工具支持这个动作吗？
///
/// 与 [`answer_key_sequence_for`] 的关系：后者产**静态键序**，而丁T5 起 `Submit` /
/// `FreeText` 的键序依赖屏读（阶段机在运行时产生）——它们**没有**静态序列可返回，
/// 却仍然有「支不支持」这件事要说（claude 支持、其余工具未定案）。故本函数是
/// **可用性判据的单点**：端点用它做门（不支持的 → 409/400），键序只在支持之后再问
/// [`answer_key_sequence_for`]（单键动作）。
///
/// 逐动作：
/// - `Select` / `Toggle` / `Cancel`：委托 [`answer_key_sequence_for`] 的既有键序档
///   （「未验不出键」的边界原样）；
/// - `Submit`：仅 claude（多选三段式的 Review 屏判据在 claude 2.1.251 实机取证；
///   其余工具的多选形态**未实测**——opencode/kimi 的多选框形态都没有实机样本）；
/// - `FreeText`：仅 claude（K4–K7 实机定案；其余工具的自由作答序列未定案）。
///
/// **拒绝原因分两档**（见 [`ActionRefusal`]）：端点据此把 400（改参数即可）与
/// 409（去终端做）分开——批次丙的旧实现把两者都塞进 `bad_index` 是对用户的误导
/// （「序号无效」会让用户以为改个序号就行）。
pub fn action_supported(
    tool: &str,
    action: AnswerAction,
    index: Option<usize>,
    q: &Question,
) -> Result<(), ActionRefusal> {
    // 工具未实测（只读档）优先于动作级判断：该工具任何动作都不可用
    if matches!(question_key_profile(tool), QuestionKeyProfile::ReadOnly) {
        return Err(ActionRefusal::ToolUnverified(format!(
            "{tool} 问答键序未实测，只读展示"
        )));
    }
    match action {
        AnswerAction::Submit => {
            if !q.multi_select {
                return Err(ActionRefusal::BadParameter(
                    "submit 仅用于多选题".to_string(),
                ));
            }
            match question_key_profile(tool) {
                // claude：多选三段式（Review 屏判据实机取证）
                QuestionKeyProfile::ClaudeFull => Ok(()),
                // E4 kimi：多选提交走 run_kimi_submit_stages（tab → Review 汇总屏 →
                // 屏上编号确认——戊探B M1/M2 + K-5 实机定案）
                QuestionKeyProfile::TwoPhaseSelect => Ok(()),
                _ => Err(ActionRefusal::ToolUnverified(format!(
                    "{tool} 多选提交未实测，不出键"
                ))),
            }
        }
        AnswerAction::FreeText => {
            if !free_text_supported(tool) {
                return Err(ActionRefusal::ToolUnverified(format!(
                    "{tool} 自由作答未实测，不出键"
                )));
            }
            // 题目形态门（复评 F6-3）：多选屏的自由作答行带勾选框，定位判据（前缀
            // `Type something`）不匹配 → 恒在 free-row 段中止。**在端点就拒绝**
            // （而不是让用户白等一次必然失败的全链），前端据此不渲染输入框。
            // 归 ToolUnverified（409 tool_readonly）而不是 BadParameter：用户要做的
            // 是「去终端作答」，不是「改个参数重试」。
            if !free_text_shape_supported(q) {
                return Err(ActionRefusal::ToolUnverified(
                    "多选题的自由作答请到终端完成（远程入口仅支持单题卡）".to_string(),
                ));
            }
            Ok(())
        }
        // 单键动作：键序档即可用性。键序档的 Err 里既有「序号越界」（参数问题）也有
        // 「多选未测」（工具问题）——按文案前缀粗分（键序档的文案是稳定的：越界类
        // 都含「越界」/「超键域」/「缺少选项序号」）
        _ => answer_key_sequence_for(tool, action, index, q)
            .map(|_| ())
            .map_err(|e| {
                if e.contains("越界") || e.contains("超键域") || e.contains("缺少选项序号")
                {
                    ActionRefusal::BadParameter(e)
                } else {
                    ActionRefusal::ToolUnverified(e)
                }
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探测档案 §3 单选真实夹具（缩录）——结构往返
    const SINGLE_JSON: &str = r#"{"questions":[{"header":"Next step","multiSelect":false,"options":[{"description":"Explain how AskUserQuestion works and when it's used.","label":"Tool demo"},{"description":"Start a coding or file task in this directory.","label":"Start a task"},{"description":"You have no further request for now.","label":"Nothing yet"}],"question":"This is a demo question — what would you like to do next?"}]}"#;

    /// 探测档案 §3 多选真实夹具（缩录）
    const MULTI_JSON: &str = r#"{"questions":[{"header":"Favorite fruits","multiSelect":true,"options":[{"description":"A sweet, crisp fruit available in many varieties.","label":"Apple"},{"description":"A soft, tropical fruit rich in potassium.","label":"Banana"},{"description":"A juicy summer fruit with a stone pit.","label":"Peach"}],"question":"Which fruits are your favorites? (Select all that apply)"}]}"#;

    #[test]
    fn parse_probe_single_fixture() {
        let qs = parse_questions(SINGLE_JSON).expect("单选夹具必须可解析");
        assert_eq!(qs.len(), 1);
        let q = &qs[0];
        assert_eq!(q.header, "Next step");
        assert_eq!(
            q.question,
            "This is a demo question — what would you like to do next?"
        );
        assert!(!q.multi_select, "multiSelect 缺省/显式 false → 单选");
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0].label, "Tool demo");
        assert_eq!(
            q.options[0].description,
            "Explain how AskUserQuestion works and when it's used."
        );
        assert_eq!(q.options[2].label, "Nothing yet");
    }

    #[test]
    fn parse_probe_multi_fixture() {
        let qs = parse_questions(MULTI_JSON).expect("多选夹具必须可解析");
        assert!(qs[0].multi_select, "multiSelect: true → 多选");
        let labels: Vec<&str> = qs[0].options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["Apple", "Banana", "Peach"]);
    }

    #[test]
    fn parse_rejects_structural_mismatch() {
        // 缺 questions / 非数组 / 空 / 条目缺 question / 缺 options / options 空 /
        // 非 JSON —— 全部 None（端点据此判不可用）
        for bad in [
            r#"{}"#,
            r#"{"questions":"x"}"#,
            r#"{"questions":[]}"#,
            r#"{"questions":[{"options":[{"label":"a"}]}]}"#,
            r#"{"questions":[{"question":"q"}]}"#,
            r#"{"questions":[{"question":"q","options":[]}]}"#,
            r#"{"questions":[{"question":"q","options":[{"description":"no label"}]}]}"#,
            "not json",
        ] {
            assert!(parse_questions(bad).is_none(), "{bad}");
        }
        // 宽容缺省：header / multiSelect / description 缺省
        let qs = parse_questions(
            r#"{"questions":[{"question":"q","options":[{"label":"a"},{"label":"b","description":"d"}]}]}"#,
        )
        .unwrap();
        assert_eq!(qs[0].header, "");
        assert!(!qs[0].multi_select);
        assert_eq!(qs[0].options[0].description, "");
        assert_eq!(qs[0].options[1].description, "d");
    }

    fn single() -> Question {
        parse_questions(SINGLE_JSON).unwrap().remove(0)
    }
    fn multi() -> Question {
        parse_questions(MULTI_JSON).unwrap().remove(0)
    }

    /// 探测 K1：单选点选项 = 单个数字键，**无回车无 Esc**（严禁后补 Esc——K2：
    /// 后补 Esc 中断模型回合）
    #[test]
    fn select_sequence_is_single_digit_no_enter_no_esc() {
        let q = single();
        assert_eq!(
            answer_key_sequence(AnswerAction::Select, Some(1), &q).unwrap(),
            vec!["2"],
            "select#2 → [\"2\"]（任务书定案序列）"
        );
        assert_eq!(
            answer_key_sequence(AnswerAction::Select, Some(0), &q).unwrap(),
            vec!["1"]
        );
    }

    /// 探测 K8：多选点选 = 单个数字键切换勾选（不提交——与单选同键不同义，
    /// 提交走三段式）
    #[test]
    fn toggle_sequence_is_single_digit() {
        let q = multi();
        assert_eq!(
            answer_key_sequence(AnswerAction::Toggle, Some(0), &q).unwrap(),
            vec!["1"],
            "toggle#1 → [\"1\"]（任务书定案序列）"
        );
    }

    /// **丁T5 改写**：多选提交不再产「一次算完的盲发序列」——`answer_key_sequence`
    /// 对 Submit 返回 Err（必须走 [`run_submit_stages`] 的阶段机）。
    /// 还原动作：把 Submit 分支改回 `Ok(down×n+1 + enter + '1')` → 本断言先红。
    #[test]
    fn submit_sequence_is_no_longer_blind_fired() {
        let q = multi(); // 3 选项
        let err = answer_key_sequence(AnswerAction::Submit, None, &q)
            .expect_err("Submit 必须拒绝盲发序列（改走阶段机）");
        assert!(
            err.contains("阶段机"),
            "拒绝原因须指向阶段机（维护者可读）：{err}"
        );
        // 分发层同结论（claude 档也走这条 Err——它不再是「claude 有、别家没有」的差别）
        assert!(answer_key_sequence_for("claude", AnswerAction::Submit, None, &q).is_err());
    }

    /// submit 仅多选合法（单选点数字即提交，无独立提交步——防误构造空转序列）
    #[test]
    fn submit_on_single_select_is_rejected() {
        assert!(answer_key_sequence(AnswerAction::Submit, None, &single()).is_err());
    }

    /// 探测 K3：取消 = 单键 esc（模型收 is_error=true 拒绝回执）
    #[test]
    fn cancel_sequence_is_esc() {
        assert_eq!(
            answer_key_sequence(AnswerAction::Cancel, None, &single()).unwrap(),
            vec!["esc"]
        );
    }

    /// 越界/缺序号/超键域防御：select index ≥ 选项数 → Err；≥9 → Err；缺 index → Err
    #[test]
    fn select_index_domain() {
        let q = single(); // 3 选项
        assert!(answer_key_sequence(AnswerAction::Select, Some(3), &q).is_err());
        assert!(answer_key_sequence(AnswerAction::Select, None, &q).is_err());
        let mut many = multi();
        many.options = (0..12)
            .map(|i| QuestionOption {
                label: format!("o{i}"),
                description: String::new(),
            })
            .collect();
        assert!(
            answer_key_sequence(AnswerAction::Select, Some(9), &many).is_err(),
            "9 选项以上超数字键域（探测档案：超范围数字未测，不出手）"
        );
    }

    /// wire ↔ 动作解析 + 审计摘要形态
    #[test]
    fn action_parse_and_audit_label() {
        assert_eq!(AnswerAction::parse("select"), Some(AnswerAction::Select));
        assert_eq!(AnswerAction::parse("toggle"), Some(AnswerAction::Toggle));
        assert_eq!(AnswerAction::parse("submit"), Some(AnswerAction::Submit));
        assert_eq!(AnswerAction::parse("cancel"), Some(AnswerAction::Cancel));
        assert_eq!(AnswerAction::parse("bogus"), None);
        assert_eq!(
            AnswerAction::Select.audit_label(Some(1)),
            "select#2",
            "审计编号对齐 UI 从 1 起"
        );
        assert_eq!(AnswerAction::Submit.audit_label(None), "submit");
        assert_eq!(AnswerAction::Cancel.audit_label(None), "cancel");
    }

    // ---------- T3：形态化匹配 + 跨工具键序分发 ----------

    /// T3 核心：**工具名不参与判据**——codex `request_user_input` 与 opencode
    /// `question` 的 args 形态与 claude 相同，均被 `parse_questions` 接住
    ///（真实形态取自 2026-09-20 跨工具矩阵 §2.1/§2.2 的实机摘录）
    #[test]
    fn parse_accepts_codex_and_opencode_shapes() {
        // codex：function_call.arguments 字符串（rollout JSONL 实录形态）
        let codex = r#"{"questions": [{"header": "历史档案", "id": "history_docs", "options": [{"description": "保留原样", "label": "保留原样 (Recommended)"}], "question": "历史文档如何处理？"}]}"#;
        let qs = parse_questions(codex).expect("codex request_user_input 形态必须可解析");
        assert_eq!(qs[0].header, "历史档案");
        assert_eq!(qs[0].options[0].label, "保留原样 (Recommended)");
        assert!(!qs[0].multi_select, "codex 无 multiSelect 字段 → 缺省单选");

        // opencode：state.input（SQLite part.data 实录形态）
        let oc = r#"{"questions": [{"header": "Build output folder", "options": [{"description": "Place build output in the dist folder", "label": "dist"}, {"description": "Place build output in the out folder", "label": "out"}], "question": "Which folder should hold build output?"}]}"#;
        let qs = parse_questions(oc).expect("opencode question 形态必须可解析");
        assert_eq!(qs[0].options.len(), 2);
        assert_eq!(qs[0].options[1].label, "out");

        // kimi：interaction.request.request.questions 同形（含 multiSelect:false）
        let kimi = r#"{"questions":[{"question":"Which output folder should the build use?","header":"Output dir","options":[{"label":"dist","description":"d"},{"label":"out","description":"o"}],"multiSelect":false}]}"#;
        assert!(parse_questions(kimi).is_some(), "kimi 形态必须可解析");
    }

    /// T3 键序档分发：工具 → 档（未知工具保守只读）
    #[test]
    fn key_profile_routing() {
        assert_eq!(
            question_key_profile("claude"),
            QuestionKeyProfile::ClaudeFull
        );
        assert_eq!(
            question_key_profile("opencode"),
            QuestionKeyProfile::SingleDigitSubmit
        );
        assert_eq!(
            question_key_profile("kimi"),
            QuestionKeyProfile::TwoPhaseSelect
        );
        // T3 实机补测升格：codex 由只读 → SingleDigitSubmit（数字即选即交三取样）
        assert_eq!(
            question_key_profile("codex"),
            QuestionKeyProfile::SingleDigitSubmit
        );
        for unknown in ["zcode", "dsh", "workbuddy", "openclaw", ""] {
            assert_eq!(
                question_key_profile(unknown),
                QuestionKeyProfile::ReadOnly,
                "{unknown} 未实测 → 只读（未验不出键）"
            );
        }
    }

    /// T3：claude 档与原序列逐字节一致（分发不改变既有行为——回归锁）
    #[test]
    fn claude_profile_matches_legacy_sequences() {
        let q = single();
        assert_eq!(
            answer_key_sequence_for("claude", AnswerAction::Select, Some(1), &q).unwrap(),
            answer_key_sequence(AnswerAction::Select, Some(1), &q).unwrap()
        );
        assert_eq!(
            answer_key_sequence_for("claude", AnswerAction::Cancel, None, &q).unwrap(),
            vec!["esc"]
        );
        let m = multi();
        assert!(
            answer_key_sequence_for("claude", AnswerAction::Submit, None, &m).is_err(),
            "submit 两侧同拒（丁T5：阶段机接管）"
        );
    }

    /// T3 · opencode 档（矩阵 §2.2 实机：VK 数字单键即选即交）——select 已验；
    /// 多选/提交/取消未测 → 全部拒绝（未验不出键）
    #[test]
    fn opencode_profile_select_only() {
        let q = single();
        assert_eq!(
            answer_key_sequence_for("opencode", AnswerAction::Select, Some(1), &q).unwrap(),
            vec!["2"],
            "数字单键即选即交（矩阵实测）"
        );
        assert!(
            answer_key_sequence_for("opencode", AnswerAction::Select, None, &q).is_err(),
            "缺序号 → 拒"
        );
        // 未测面全部拒绝
        assert!(answer_key_sequence_for("opencode", AnswerAction::Cancel, None, &q).is_err());
        assert!(answer_key_sequence_for("opencode", AnswerAction::Submit, None, &multi()).is_err());
        assert!(
            answer_key_sequence_for("opencode", AnswerAction::Toggle, Some(0), &multi()).is_err()
        );
    }

    /// T3 · kimi 档（**2026-09-21 实机复验修正**：数字选中 → **Enter 确认**）
    ///
    /// 矩阵原记载「数字 → Review 屏 → 数字 '1' 提交」被实机证伪：第二个数字无效，
    /// 确认键是 Enter（底栏 `1/2/3 choose · ↵ confirm`）。
    #[test]
    fn kimi_profile_is_two_phase() {
        let q = single();
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Select, Some(0), &q).unwrap(),
            vec!["1", "enter"],
            "两段式：数字选中 → Enter 确认（实机复验修正，原记为 [数字, '1']）"
        );
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Select, Some(2), &q).unwrap(),
            vec!["3", "enter"]
        );
        // 越界/缺序号仍拒
        assert!(answer_key_sequence_for("kimi", AnswerAction::Select, Some(9), &q).is_err());
        assert!(answer_key_sequence_for("kimi", AnswerAction::Select, None, &q).is_err());
        // E4：多选切勾 = 单次数字（N7 修复）；取消仍未验拒绝
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Toggle, Some(0), &multi()).unwrap(),
            vec!["1"],
            "N7 锁：切勾序列必须不含尾随回车（[数字,回车]=双切抵消净零）"
        );
        assert!(answer_key_sequence_for("kimi", AnswerAction::Cancel, None, &q).is_err());
    }

    /// T3 · codex 档（2026-09-21 实机补测后）→ 单选 select 数字即选即交；
    /// 多选（toggle/submit）与 cancel 仍拒（未验 / Esc=中断回合语义破坏性）
    #[test]
    fn codex_profile_digit_select_only() {
        let q = single();
        // 实机三取样：数字单键即选即交
        assert_eq!(
            answer_key_sequence_for("codex", AnswerAction::Select, Some(1), &q).unwrap(),
            vec!["2"],
            "数字单键即选即交（实机补测三取样）"
        );
        assert_eq!(
            answer_key_sequence_for("codex", AnswerAction::Select, Some(0), &q).unwrap(),
            vec!["1"]
        );
        // 越界/缺序号拒
        assert!(answer_key_sequence_for("codex", AnswerAction::Select, Some(9), &q).is_err());
        assert!(answer_key_sequence_for("codex", AnswerAction::Select, None, &q).is_err());
        // 多选形态未验 → 拒
        assert!(answer_key_sequence_for("codex", AnswerAction::Toggle, Some(0), &multi()).is_err());
        assert!(answer_key_sequence_for("codex", AnswerAction::Submit, None, &multi()).is_err());
        // cancel 拒：Esc 在 codex 是**中断整个回合**（实录 "aborted by user…" +
        // turn_aborted），非 claude 的 is_error 拒答——不出手
        assert!(
            answer_key_sequence_for("codex", AnswerAction::Cancel, None, &q).is_err(),
            "Esc=中断回合，语义破坏性 → 不代按"
        );
    }

    /// 丁T2：kimi 档的**证据边界**回归锁——证明 TwoPhaseSelect 只对单选放行、
    /// 且不因 plan_review 的证伪而漂移（改它必须由实机探测定案，见
    /// [`question_key_profile`] 的丁T2 注释）。
    #[test]
    fn kimi_question_profile_stays_two_phase_pending_probe() {
        let q = single();
        assert_eq!(
            question_key_profile("kimi"),
            QuestionKeyProfile::TwoPhaseSelect,
            "kimi question 类数字可靠性未经独立实测——不超证据改行为（R1 证伪的是 \
             plan_review 审批框，属另一对话框族）"
        );
        // 两段式序列形态不变（数字选中 → Enter 确认）
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Select, Some(0), &q).unwrap(),
            vec!["1", "enter"]
        );
        // E4 更新：多选切勾=单次数字（戊探B ×3 定案，N7 修复）；submit 的**静态序列**
        // 仍拒（走 run_kimi_submit_stages 阶段机）；取消仍未验拒绝
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Toggle, Some(0), &multi()).unwrap(),
            vec!["1"],
            "N7 锁：切勾禁尾随回车"
        );
        assert!(answer_key_sequence_for("kimi", AnswerAction::Submit, None, &multi()).is_err());
        assert!(answer_key_sequence_for("kimi", AnswerAction::Cancel, None, &q).is_err());
    }

    // ============================================================
    // 丁T5：多选提交阶段机（§2.3 裁4）——脚本化屏幕序列驱动
    // ============================================================

    // ---- 真机屏幕夹具（2026-09-21 探测档案原文，逐字抄录）----

    // ==== 批次戊 E4：kimi 问答全链（真机夹具 + 阶段机脚本锁）====

    /// e-stage2 屏读夹具读取（confirm/dialog/mode tests 同款）
    #[cfg(test)]
    fn e_stage2_screen(name: &str) -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/e-stage2")
            .join(name);
        let raw =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取夹具失败 {path:?}: {e}"));
        raw.trim_start_matches('\u{feff}')
            .lines()
            .filter(|l| !l.starts_with("# "))
            .map(|l| l.trim_end_matches('\r').to_string())
            .collect()
    }

    /// **kimi Review 汇总屏锚 × 真机夹具**（戊探B B1 单题 Review 屏原件）：
    /// `Ready to submit your answers?` + `→ [1] Submit` / `[2] Cancel`。
    #[test]
    fn e4_kimi_review_anchor_on_real_fixture() {
        let screen = e_stage2_screen("kimi-question-review.txt");
        assert!(kimi_review_present(&screen), "真机 Review 屏必须在场");
        assert_eq!(
            kimi_review_confirm_digit(&screen).as_deref(),
            Some("1"),
            "确认键 = 屏上编号（→ [1] Submit）"
        );
        // 对照：多值答案 Review（m1-after-tab1 原件）同样命中
        let multi_ans = e_stage2_screen("kimi-question-multi-q.txt");
        assert!(kimi_review_present(&multi_ans));
        assert_eq!(kimi_review_confirm_digit(&multi_ans).as_deref(), Some("1"));
        // 普通问题屏（非 Review）不得误判
        let dialog = e_stage2_screen("kimi-question-multiselect.txt");
        assert!(
            !kimi_review_present(&dialog),
            "多选题屏（[ ] 复选形态）不是 Review 汇总屏"
        );
    }

    /// **kimi Other 行定位**：单选形态 `[5] Other:` 可定位（戊探B B8 行形）；
    /// 多选真机夹具的 Other 行无编号（`[ ] Other`）→ 不可定位（如实拒绝）。
    #[test]
    fn e4_kimi_other_row_locate() {
        let screen = lines(&[
            "   → [1] red",
            "     [2] green",
            "     [3] blue",
            "     [4] yellow",
            "   → [5] Other:",
        ]);
        assert_eq!(locate_kimi_other_digit(&screen).as_deref(), Some("5"));
        let multi = e_stage2_screen("kimi-question-multiselect.txt");
        assert_eq!(
            locate_kimi_other_digit(&multi),
            None,
            "多选 Other 行无编号 → 自由作答不可达（到终端作答）"
        );
    }

    /// **kimi 提交阶段机脚本锁（两形态）**：
    /// ① 多题流（Review 已在场）→ **零 tab**，直接确认键 '1'；
    /// ② 多选流（勾完在题目页）→ tab → Review → '1'。终态锚在场 → Some(true)、
    /// 未见 → Some(false)（不谎报完成）。
    #[test]
    fn e4_kimi_submit_stage_scripted() {
        let review = e_stage2_screen("kimi-question-review.txt");
        let receipt = lines(&["● Collected your answers"]);
        let mut sent: Vec<String> = Vec::new();
        let mut terminal = crate::inject::mode::Closures {
            read: || Some(review.clone()),
            send: |k: &str| {
                sent.push(k.to_string());
                Ok(())
            },
            settle: || {},
        };
        let out = run_kimi_submit_stages(
            || Some(review.clone()),
            || Ok(Some(review.clone())),
            || Ok(Some(receipt.clone())),
            &mut terminal,
        )
        .expect("Review 在场 → 直接确认");
        assert_eq!(
            out.sent_keys,
            vec!["1"],
            "Review 已在场 → 零 tab，直发屏上编号"
        );
        assert_eq!(out.receipt_seen, Some(true), "终态锚在场");

        // 多选流：题目页（多选夹具）→ tab → Review → '1'
        let dialog = e_stage2_screen("kimi-question-multiselect.txt");
        let mut sent2: Vec<String> = Vec::new();
        let mut terminal2 = crate::inject::mode::Closures {
            read: || Some(dialog.clone()),
            send: |k: &str| {
                sent2.push(k.to_string());
                Ok(())
            },
            settle: || {},
        };
        let out2 = run_kimi_submit_stages(
            || Some(dialog.clone()),
            || Ok(Some(review.clone())),
            || Ok(None),
            &mut terminal2,
        )
        .expect("tab 后 Review 出现 → 走完整条");
        assert_eq!(
            out2.sent_keys,
            vec!["tab", "1"],
            "题目页 → tab 切 Submit → 屏上编号确认"
        );
        assert_eq!(
            out2.receipt_seen,
            Some(false),
            "终态未见 → 如实 Some(false)"
        );
    }

    /// **kimi Other 自由作答阶段机脚本锁**：Other 行数字 → 文本 → 回车 → Review →
    /// 确认（戊探B E-B8 全链）；多选屏（Other 无编号）→ 第 1 段中止零按键。
    #[test]
    fn e4_kimi_free_text_stage_scripted() {
        let screen = lines(&["   → [1] red", "     [2] green", "   → [5] Other:"]);
        let review = e_stage2_screen("kimi-question-review.txt");
        let mut sent: Vec<String> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut terminal = crate::inject::question::FreeTextClosures {
            read: || Some(screen.clone()),
            send: |k: &str| {
                sent.push(k.to_string());
                Ok(())
            },
            send_text: |t: &str| {
                texts.push(t.to_string());
                Ok(())
            },
            settle: || {},
        };
        let out = run_kimi_free_text_stages(
            "atlantis",
            || Some(screen.clone()),
            || Ok(Some(review.clone())),
            || Ok(Some(lines(&["● Collected your answers"]))),
            &mut terminal,
        )
        .expect("Other 行可定位 → 全链走通");
        assert_eq!(out.sent_keys, vec!["5", "<text>", "enter", "1"]);
        assert_eq!(texts, vec!["atlantis"], "文本走字符通道");
        assert_eq!(out.receipt_seen, Some(true));

        // 多选屏：Other 无编号 → 中止零按键
        let multi = e_stage2_screen("kimi-question-multiselect.txt");
        let mut sent2: Vec<String> = Vec::new();
        let mut terminal2 = crate::inject::question::FreeTextClosures {
            read: || Some(multi.clone()),
            send: |k: &str| {
                sent2.push(k.to_string());
                Ok(())
            },
            send_text: |_: &str| Ok(()),
            settle: || {},
        };
        let err = run_kimi_free_text_stages(
            "x",
            || Some(multi.clone()),
            || Ok(None),
            || Ok(None),
            &mut terminal2,
        )
        .expect_err("多选 Other 无编号 → 中止");
        assert!(err.message.contains("Other 行"), "{err:?}");
        assert!(sent2.is_empty(), "中止零按键");
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// **多选提交屏**（`C-s8-cursor-submit-20260921-015844.png` 的可见窗口内容，按
    /// `read_screen_window` 的返回形态抄录：行尾 trim、行内前导空白保留）。
    ///
    /// 该屏的关键形态（判据的实机依据）：
    /// - 焦点标记 `❯`（U+276F）在 **Submit 行**，且该行**无编号**（与选项行不同）；
    /// - 选项行是 `N. [✓]/[ ] 标签`（勾选框 + 描述行夹在其间）；
    /// - TUI 自动追加的 `4. [ ] Type something` 在最后一个模型选项之后；
    /// - 底栏 `Enter to select · ↑/ to navigate · Esc to cancel`。
    /// - 头部 `← ☒ Favorite fruits  ✔Submit  →`（非 ASCII 箭头/勾选框——夹具纪律要求
    ///   含非 ASCII，本屏天然满足）。
    fn multi_submit_screen_focus_on_submit() -> Vec<String> {
        lines(&[
            " ← ☒ Favorite fruits  ✔Submit  →",
            "",
            " Which fruits are your favorites? (Select all that apply)",
            "",
            " 1. [✓] Apple",
            " A sweet, crisp fruit available in many varieties.",
            " 2. [ ] Banana",
            " A soft, tropical fruit rich in potassium.",
            " 3. [ ] Peach",
            " A juicy summer fruit with a stone pit.",
            " 4. [ ] Type something",
            " ❯   Submit",
            " 5. Chat about this",
            "",
            " Enter to select · ↑/ to navigate · Esc to cancel",
        ])
    }

    /// 同屏但**焦点在选项行**（`C-s8-digit1-checked-20260921-015738.png` 的形态：
    /// `❯` 在 `1. [✓] Apple` 行首，Submit 行是纯空白缩进）——提交路径必须先走位。
    fn multi_submit_screen_focus_on_option() -> Vec<String> {
        lines(&[
            " ← ☒ Favorite fruits  ✔Submit  →",
            "",
            " Which fruits are your favorites? (Select all that apply)",
            "",
            " ❯ 1. [✓] Apple",
            " A sweet, crisp fruit available in many varieties.",
            " 2. [ ] Banana",
            " A soft, tropical fruit rich in potassium.",
            " 3. [ ] Peach",
            " A juicy summer fruit with a stone pit.",
            " 4. [ ] Type something",
            "    Submit",
            " 5. Chat about this",
            "",
            " Enter to select · ↑/ to navigate · Esc to cancel",
        ])
    }

    /// **Review 确认屏**（`C-s8-submitted-20260921-015906.png` 原文，逐字）。
    /// 关键形态：标题 `Review your answers`、已选项回显 `→ Banana, Apple`（非 ASCII
    /// 箭头）、副题 `Ready to submit your answers?`、编号选项 `1. Submit answers` /
    /// `2. Cancel`。该屏的 `1.` 行首**无** `❯`（实测截图里焦点由行内高亮表达）。
    fn review_screen_real() -> Vec<String> {
        lines(&[
            " ← ☒ Favorite fruits  ✔Submit  →",
            "",
            " Review your answers",
            "",
            " ● Which fruits are your favorites? (Select all that apply)",
            " → Banana, Apple",
            "",
            " Ready to submit your answers?",
            "",
            " 1. Submit answers",
            " 2. Cancel",
        ])
    }

    /// **终态屏**（`C-s8-final-20260921-015941.png`）：对话流出现
    /// `User answered Claude's questions:` + 回显行。
    fn answered_final_screen() -> Vec<String> {
        lines(&[
            " ● User answered Claude's questions:)",
            " L  • Which fruits are your favorites? (Select all that apply) → Banana, Apple",
            "",
            " Thought for 2s (ctrl+o to expand)",
            "",
            " ● Done — you selected Banana and Apple (multi-select worked, two picks accepted).",
        ])
    }

    /// 阶段机脚本驱动器的返回：`(结果, 实际发出的键)`。
    type SubmitScript = (Result<SubmitOutcome, StageAbort>, Vec<String>);

    /// 轮询探测的**唯一实现**（三个轮询闭包共用）：循环读当前屏直到 `probe` 命中。
    ///
    /// - `Ready` → `Ok(Some(屏))`；`Fatal` → `Err`（立即中止）；`NotYet` → 推进 cursor
    ///   再试；**序列耗尽 = 生产侧「轮询窗尽」→ `Ok(None)`**。
    fn poll_probe(
        cursor: &std::cell::Cell<usize>,
        screens: &[Vec<String>],
        probe: fn(&[String]) -> ScreenStep,
    ) -> Result<Option<Vec<String>>, String> {
        let n = screens.len();
        loop {
            let cur = screens[cursor.get().min(n - 1)].clone();
            match probe(&cur) {
                ScreenStep::Ready(l) => return Ok(Some(l)),
                ScreenStep::Fatal(why) => return Err(format!("{why}；请人工核对终端")),
                ScreenStep::NotYet(_) => {
                    if cursor.get() + 1 < n {
                        cursor.set(cursor.get() + 1);
                    } else {
                        return Ok(None); // 窗尽
                    }
                }
            }
        }
    }

    /// **脚本化提交驱动器**：读屏与发键全脚本化，跑 [`run_submit_stages`]——不依赖真
    /// conhost（这是把编排抽进内核的全部意义，同 `mode::run_stage_script`）。
    ///
    /// # 脚本语义（`screens` 是终端内容的**时间序列**）
    ///
    /// - `send(key)`：记录按键并推进 cursor（按键会让 TUI 重绘）；
    /// - `read()`：返回**当前**屏；
    /// - 三段轮询各调用 [`poll_probe`]（同一个 cursor，故「按键 → 屏推进 → 下一段在
    ///   新屏上复核」的时序与生产一致）。
    fn run_submit_script(screens: Vec<Vec<String>>, max_down_steps: usize) -> SubmitScript {
        use std::cell::{Cell, RefCell};
        let n = screens.len();
        let cursor = Cell::new(0usize);
        let cur = || screens[cursor.get().min(n - 1)].clone();
        let sent: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = run_submit_stages(
            || poll_probe(&cursor, &screens, probe_submit_screen),
            || poll_probe(&cursor, &screens, probe_review_screen),
            || {
                poll_probe(&cursor, &screens, |l: &[String]| {
                    if probe_answered_receipt(l) {
                        ScreenStep::Ready(l.to_vec())
                    } else {
                        ScreenStep::NotYet("未见终态回执行".to_string())
                    }
                })
            },
            &mut crate::inject::mode::Closures {
                read: || Some(cur()),
                send: |k: &str| {
                    sent.borrow_mut().push(k.to_string());
                    if cursor.get() + 1 < n {
                        cursor.set(cursor.get() + 1);
                    }
                    Ok(())
                },
                settle: || {},
            },
            max_down_steps,
        );
        (result, sent.into_inner())
    }

    /// **场景①：正常三段（焦点已在 Submit 行）**——提交屏在场 → 直接回车 → Review 屏
    /// → 抄屏上编号发确认键 → 终态回执。
    ///
    /// 断言：键序 = `[enter, "1"]`（**零 down**——焦点本就在提交行，走位段一次都不该发）；
    /// Review 屏确认在场；终态回执屏读到。
    /// 还原动作：把走位段改成「无条件先发 down ×(n+1)」→ 第一句断言先红。
    #[test]
    fn submit_stage_happy_path_focus_already_on_submit() {
        let script = vec![
            multi_submit_screen_focus_on_submit(),
            review_screen_real(),
            answered_final_screen(),
        ];
        let (r, sent) = run_submit_script(script, 5);
        let out = r.expect("正常三段必须走通");
        assert_eq!(out.down_steps, 0, "焦点已在 Submit 行 → 零走位");
        assert_eq!(
            sent,
            vec!["enter".to_string(), "1".to_string()],
            "键序 = [enter, '1']（确认键取自 Review 屏上编号）"
        );
        assert!(
            out.review_confirmed,
            "Review 屏必须被屏读确认过（不是盲发 '1'）"
        );
        assert_eq!(
            out.receipt_seen,
            Some(true),
            "终态屏有 `User answered Claude's questions:` → 回执核验通过"
        );
    }

    /// **场景②：走位（焦点在选项行 → 逐步按 ↓ → 落到 Submit 行 → 回车）**。
    /// 断言：down 数 = 1（脚本里第二屏就是焦点已到 Submit 的形态）+ 后续 enter/'1'；
    /// 且**每步都复核过**（脚本驱动下「复核」体现为：down 后读的是**新屏**，新屏的
    /// 焦点行确实变了才停手）。
    /// 还原动作：把走位段的复核去掉（发完 down 就 break）→ 本用例的 `down_steps` 仍为 1
    /// 但焦点未复核；把复核改成「发满 max_down_steps 才停」→ 断言先红。
    #[test]
    fn submit_stage_walks_down_until_submit_row_focused() {
        let script = vec![
            multi_submit_screen_focus_on_option(),
            multi_submit_screen_focus_on_submit(),
            review_screen_real(),
            answered_final_screen(),
        ];
        let (r, sent) = run_submit_script(script, 5);
        let out = r.expect("走位后必须走通");
        assert_eq!(out.down_steps, 1, "恰按一个 ↓ 就到位（每步复核后停手）");
        assert_eq!(
            sent,
            vec!["down".to_string(), "enter".to_string(), "1".to_string()],
            "键序 = [down, enter, '1']：走位键在提交键之前，且只按到到位为止"
        );
    }

    /// **场景③：光标不动 → 中止且不 post enter**（任务书测试项②）。
    ///
    /// 脚本只给一屏、焦点在选项行（走位键发出后屏不重绘 = 终端吞键/未响应）：
    /// 走位上限耗尽 → 中止。断言：**发过 down 但从未发 enter**——这正是「不盲提交」。
    /// 还原动作：把走位上限判据删掉、或改成「超限也照发 enter」→ 第二句断言先红。
    #[test]
    fn submit_stage_aborts_without_enter_when_cursor_never_moves() {
        let script = vec![multi_submit_screen_focus_on_option()];
        let (r, sent) = run_submit_script(script, 5);
        let err = r.as_ref().unwrap_err();
        assert!(
            err.message.contains("仍未把焦点移到 Submit 行"),
            "中止原因须点名走位失败：{err}"
        );
        assert!(
            !sent.iter().any(|k| k == "enter"),
            "焦点未到位 ⇒ **绝不发回车**（不盲提交）：{sent:?}"
        );
        assert!(sent.iter().all(|k| k == "down"), "只发过 down：{sent:?}");
    }

    /// **场景④：发了 enter 但没进 Review 屏 → 中止且不发 '1'**（任务书测试项③，
    /// 问题 7 的正解）。
    ///
    /// 脚本：提交屏 → 回车后 TUI 停在原地（Review 屏未出现，轮询窗尽）。断言：键里
    /// **有** enter、**没有**任何数字键；回执点名「未出现 Review 确认屏」。
    /// 还原动作：把 Review 段改回「不等屏读、直接发 '1'」（批次丙的盲发形态）→
    /// 第二句断言先红（键里出现了 '1'）。
    #[test]
    fn submit_stage_aborts_without_digit_when_review_screen_absent() {
        // 两屏：提交屏（焦点已在 Submit 行）→ 回车后仍是提交屏（Review 未出现）
        let script = vec![
            multi_submit_screen_focus_on_submit(),
            multi_submit_screen_focus_on_submit(),
        ];
        let (r, sent) = run_submit_script(script, 5);
        let err = r.as_ref().unwrap_err();
        assert!(
            err.message.contains("未出现 Review 确认屏"),
            "中止原因须点名缺 Review 屏：{err}"
        );
        assert!(sent.contains(&"enter".to_string()), "回车已发出：{sent:?}");
        assert!(
            !sent.iter().any(|k| k.chars().all(|c| c.is_ascii_digit())),
            "Review 屏不在场 ⇒ **绝不发数字确认键**：{sent:?}"
        );
    }

    /// **场景⑤：Review 屏出现但确认后无终态 → 如实回执（不谎报完成）**（任务书测试项④）。
    ///
    /// 脚本：提交屏 → Review 屏 → 确认后是普通输出（无终态锚）。断言：整条编排仍 `Ok`
    /// （确认键确实发出去了，事实是「投递完成」），但 `receipt_seen = Some(false)`——
    /// 调用方据此下发「请人工核对」而不是「已完成」。
    /// 还原动作：把终态段的 `Ok(None)` 改成 `Err`（把「未见」当失败）→ `r.expect` 先红；
    /// 改成 `Some(true)`（谎报）→ 末句断言先红。
    #[test]
    fn submit_stage_receipt_absent_is_not_a_failure() {
        let script = vec![
            multi_submit_screen_focus_on_submit(),
            review_screen_real(),
            lines(&["  > ", " 普通对话输出"]),
        ];
        let (r, sent) = run_submit_script(script, 5);
        let out = r.expect("终态未屏读到**不是**投递失败：整条编排仍 Ok");
        assert_eq!(sent, vec!["enter".to_string(), "1".to_string()]);
        assert_eq!(
            out.receipt_seen,
            Some(false),
            "读到屏但无终态回执行 → Some(false)（不谎报完成）"
        );
    }

    /// **场景⑥：提交屏根本不在场 → 零按键中止**（防「不在多选提交屏上却按了键」）。
    ///
    /// **本用例覆盖的是「轮询窗尽未拿到屏」那条路径**（`poll_submit` → `Ok(None)`）：
    /// 脚本驱动的 `poll_probe` 自带判据，单选屏永远不满足 → 窗尽 → 第 1 段的
    /// `ok_or_else` 分支中止。
    /// 还原动作：把第 1 段的 `ok_or_else` 去掉（`unwrap_or_default()`）→ 本用例先红
    /// （会继续往下走并发键）。
    ///
    /// **注（复评 F6-5 订正）**：本条**不覆盖**紧随其后的「拿到了屏但屏上没有 Submit 行」
    /// 那道检查——那是另一条分支，由下一条用例专门覆盖（脚本驱动的轮询判据会先把这种屏
    /// 拦在窗尽，故必须换一种驱动方式）。
    #[test]
    fn submit_stage_aborts_with_zero_keys_when_poll_window_exhausts() {
        // 单选屏（无 Submit 行）——轮询判据永不满足 → 窗尽
        let script = vec![lines(&[
            " Which drink do you prefer?",
            " 1. Coffee",
            " 2. Tea",
            " 3. Type something.",
            " 4. Chat about this",
        ])];
        let (r, sent) = run_submit_script(script, 5);
        let err = r.as_ref().unwrap_err();
        assert!(
            err.message.contains("Submit 行"),
            "中止原因须点名缺提交入口：{err}"
        );
        assert!(sent.is_empty(), "零按键中止（一个键都不能发）：{sent:?}");
    }

    /// **第 1 段的第二道检查：轮询拿到了屏，但屏上没有 Submit 行 → 零按键中止**。
    ///
    /// # 为什么要单独一条（复评 F6-5 的发现）
    ///
    /// 上一条用例走的是「窗尽」分支，**到不了**这道 `submit_row_present` 检查——脚本
    /// 驱动的轮询自带判据，会把「没有 Submit 行的屏」先拦掉。而**生产侧不是这样**：
    /// `remote::api::poll_question_stage` 只负责**读一屏**（它没有判据，判据单点在编排
    /// 里）——所以生产上「读到屏但形态不符」是**真实可达**的路径，这条检查是承重的。
    /// 本用例用**直调**（poll 无条件返回当前屏，模拟生产轮询的「只读不判」）覆盖它。
    ///
    /// 还原动作：删掉第 1 段的 `if !first.iter().any(|l| submit_row_present(l))` 检查
    /// → 本用例先红（会继续往下走并发键）。
    #[test]
    fn submit_stage_aborts_with_zero_keys_when_polled_screen_lacks_submit_row() {
        use std::cell::RefCell;
        let single_select = lines(&[
            " Which drink do you prefer?",
            " 1. Coffee",
            " 2. Tea",
            " 3. Type something.",
            " 4. Chat about this",
        ]);
        let sent: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let out = run_submit_stages(
            // 生产语义的轮询：**只读不判**（返回当前屏，不挑形态）
            || Ok(Some(single_select.clone())),
            || Ok(None),
            || Ok(None),
            &mut crate::inject::mode::Closures {
                read: || Some(single_select.clone()),
                send: |k: &str| {
                    sent.borrow_mut().push(k.to_string());
                    Ok(())
                },
                settle: || {},
            },
            5,
        );
        let err = out.expect_err("屏上没有 Submit 行 → 必须中止");
        assert_eq!(
            err.kind,
            StageAbortKind::Screen,
            "形态不符属屏读类中止（不是投递失败）"
        );
        assert!(
            err.message.contains("Submit 行"),
            "中止原因须点名缺提交入口：{err}"
        );
        assert!(
            sent.borrow().is_empty(),
            "零按键中止（一个键都不能发）：{:?}",
            sent.borrow()
        );
    }

    /// **场景⑦：Review 屏在场但确认项读不到编号 → 中止不发数字**（抄不到编号就不猜）。
    ///
    /// 形态：确认项被渲染成**无编号行**（`parse_option_line` 不认 → 判「读不到带编号的
    /// 确认项」）→ 中止。
    /// 还原动作：把 `probe_review_screen` 的 `0 => Fatal` 分支改成 `Ready`（无编号也放行）
    /// → 本用例先红（会往下走到发键）。
    #[test]
    fn submit_stage_aborts_when_review_confirm_has_no_number() {
        let mut review = review_screen_real();
        for l in review.iter_mut() {
            if l.contains("Submit answers") {
                *l = "   Submit answers".to_string();
            }
        }
        let script = vec![multi_submit_screen_focus_on_submit(), review];
        let (r, sent) = run_submit_script(script, 5);
        let err = r.as_ref().unwrap_err();
        assert!(
            err.message.contains("读不到带编号的确认项"),
            "中止原因须点名编号缺失：{err}"
        );
        assert!(
            sent.iter().all(|k| k != "1" && k != "2"),
            "抄不到编号 ⇒ 不发任何数字键：{sent:?}"
        );
    }

    /// **确认键取自屏上编号（不是硬编码 '1'）**——本用例的 Review 屏把确认项排在 2
    /// 号（合成的**结构测试**夹具：真机形态里它恒为 1 号，故这条不变式只有合成屏能
    /// 区分开）。
    ///
    /// 这不是「凭空造屏」：它要钉的判据是**代码里的**（`confirm_num.to_string()`），
    /// 而真机夹具恰好让「抄屏上编号」与「硬编码 '1'」同解——只靠真机夹具，这条实现
    /// 细节被误改成硬编码时**门禁不会红**（本批的变异验证就抓到了这一点）。故补一条
    /// 合成屏把两者分开，同时如实标注它**不是**实机形态记录。
    ///
    /// 还原动作：把确认键改成 `"1".to_string()`（硬编码）→ 本用例先红。
    #[test]
    fn submit_stage_confirm_key_comes_from_screen_number_not_hardcoded() {
        let mut review = review_screen_real();
        // 把两条确认/取消行对调编号（合成形态：确认项在 2 号）
        for l in review.iter_mut() {
            if l.trim() == "1. Submit answers" {
                *l = " 2. Submit answers".to_string();
            } else if l.trim() == "2. Cancel" {
                *l = " 1. Cancel".to_string();
            }
        }
        let script = vec![
            multi_submit_screen_focus_on_submit(),
            review,
            answered_final_screen(),
        ];
        let (r, sent) = run_submit_script(script, 5);
        r.expect("合成屏（确认项在 2 号）也必须走通");
        assert_eq!(
            sent,
            vec!["enter".to_string(), "2".to_string()],
            "确认键必须抄**屏上编号**（本例 = 2），不得硬编码 '1'：{sent:?}"
        );
    }

    // ============================================================
    // 丁T5：自由作答阶段机（§2.4 裁3）——脚本化驱动
    // ============================================================

    /// **单选自由作答屏**（`C-s7-q-ui-20260921-015440.png` / `C-s7-digit3-result-*.png`
    /// 原文形态）：`3. Type something.` 在模型选项之后、`4. Chat about this` 在分隔线之下。
    fn single_free_text_screen() -> Vec<String> {
        lines(&[
            " ☐ Preferred drink",
            "",
            " Which drink do you prefer?",
            "",
            " 1. Coffee",
            " A brewed beverage made from roasted beans, containing caffeine.",
            " 2. Tea",
            " A steeped beverage made from leaves, available in many varieties.",
            " 3. Type something.",
            "",
            " 4. Chat about this",
        ])
    }

    /// 焦点已落在自由作答行（`C-s7-text-in-input-*.png` 形态；行标签保留）。
    fn single_free_text_screen_focused() -> Vec<String> {
        lines(&[
            " ☐ Preferred drink",
            "",
            " Which drink do you prefer?",
            "",
            " 1. Coffee",
            " A brewed beverage made from roasted beans, containing caffeine.",
            " 2. Tea",
            " A steeped beverage made from leaves, available in many varieties.",
            " ❯ 3. Type something.",
            "",
            " 4. Chat about this",
        ])
    }

    /// 自由作答脚本驱动器的返回：`(结果, 发出的键, 发出的文本)`。
    type FreeTextScript = (
        Result<FreeTextOutcome, StageAbort>,
        Vec<String>,
        Vec<String>,
    );

    /// 自由作答的脚本驱动器（同 [`run_submit_script`] 的时序语义；文本通道单独记录）。
    fn run_free_text_script(screens: Vec<Vec<String>>, text: &str) -> FreeTextScript {
        use std::cell::{Cell, RefCell};
        let n = screens.len();
        let cursor = Cell::new(0usize);
        let cur = || screens[cursor.get().min(n - 1)].clone();
        let sent: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let texts: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let result = run_free_text_stages(
            text,
            || {
                poll_probe(&cursor, &screens, |l: &[String]| {
                    if locate_free_text_row(l).is_some() {
                        ScreenStep::Ready(l.to_vec())
                    } else {
                        ScreenStep::NotYet("未见自由作答行".to_string())
                    }
                })
            },
            || {
                poll_probe(&cursor, &screens, |l: &[String]| {
                    if probe_answered_receipt(l) {
                        ScreenStep::Ready(l.to_vec())
                    } else {
                        ScreenStep::NotYet("未见终态回执行".to_string())
                    }
                })
            },
            &mut FreeTextClosures {
                read: || Some(cur()),
                send: |k: &str| {
                    sent.borrow_mut().push(k.to_string());
                    if cursor.get() + 1 < n {
                        cursor.set(cursor.get() + 1);
                    }
                    Ok(())
                },
                send_text: |t: &str| {
                    texts.borrow_mut().push(t.to_string());
                    if cursor.get() + 1 < n {
                        cursor.set(cursor.get() + 1);
                    }
                    Ok(())
                },
                settle: || {},
            },
        );
        (result, sent.into_inner(), texts.into_inner())
    }

    /// **自由作答正常链**：定位数字（**抄屏上编号 3**，不是「选项数+1」）→ 文本 → 回车。
    /// 断言文本**只经字符通道**、键通道里只有定位数字与回车（**没有 Esc、没有第二个
    /// 数字**——K2 的教训）。
    /// 还原动作：把 `send_text` 改走 `send`（文本进键通道）→ 第三句断言先红。
    #[test]
    fn free_text_stage_sends_digit_then_text_then_enter() {
        let script = vec![
            single_free_text_screen(),
            // 定位键按下后（焦点落到该行）→ 文本注入后 → 回车后（终态）
            single_free_text_screen_focused(),
            single_free_text_screen_focused(),
            answered_final_screen(),
        ];
        let (r, keys, texts) = run_free_text_script(script, "green tea please");
        let out = r.expect("自由作答链必须走通");
        assert_eq!(
            keys,
            vec!["3".to_string(), "enter".to_string()],
            "键通道恰两键：定位数字（屏上编号 3）+ 提交回车——**无 Esc 无第二数字**"
        );
        assert_eq!(
            texts,
            vec!["green tea please".to_string()],
            "文本只经字符通道投递一次（裁3：文本绝不进键通道）"
        );
        assert_eq!(out.receipt_seen, Some(true));
        assert_eq!(
            out.sent_keys,
            vec![
                "3".to_string(),
                "<text:16 chars>".to_string(),
                "enter".to_string()
            ],
            "回执键序用占位符记文本（不落正文）：16 = \"green tea please\" 的字符数"
        );
    }

    /// **K7 回归**：自由作答行不在场 → 零按键零文本中止（未定位直接打字是无效操作）。
    /// 还原动作：删掉第 1 段的在场检查 → 本用例先红（会发文本）。
    #[test]
    fn free_text_stage_aborts_with_no_keys_when_row_absent() {
        let script = vec![lines(&[
            " Which drink do you prefer?",
            " 1. Coffee",
            " 2. Tea",
        ])];
        let (r, keys, texts) = run_free_text_script(script, "hi");
        let err = r.as_ref().unwrap_err();
        assert!(err.message.contains("自由作答行"), "中止原因须点名：{err}");
        assert!(keys.is_empty() && texts.is_empty(), "零投递中止");
    }

    /// **定位后形态不符 → 不发文本**（防「定位键被吞了还照打」：那会把答案打进别处）。
    /// 还原动作：删掉定位后的重读复核 → 本用例先红（文本会被发出）。
    #[test]
    fn free_text_stage_aborts_before_text_when_row_disappears_after_locate() {
        let script = vec![
            single_free_text_screen(),
            lines(&[" 已切到别的界面", " > "]),
        ];
        let (r, keys, texts) = run_free_text_script(script, "green tea please");
        let err = r.as_ref().unwrap_err();
        assert!(
            err.message.contains("已见不到自由作答行"),
            "中止原因须点名定位后形态不符：{err}"
        );
        assert_eq!(keys, vec!["3".to_string()], "定位键已发出（事实如实记录）");
        assert!(
            texts.is_empty(),
            "形态不符 ⇒ 绝不发作答文本（不盲打字）：{texts:?}"
        );
    }

    /// **裁3 安全面（端到端锁）**：用户文本里的数字/控制字面**不可能**变成选择——
    /// 定位数字来自屏上编号、文本走独立通道。本用例钉住「键通道里数字只可能是定位
    /// 数字」这条不变量：喂一个「全是数字」的文本，键通道仍只有一个定位数字与一个回车。
    #[test]
    fn free_text_user_digits_never_reach_key_channel() {
        let script = vec![
            single_free_text_screen(),
            single_free_text_screen_focused(),
            single_free_text_screen_focused(),
            answered_final_screen(),
        ];
        let (r, keys, texts) = run_free_text_script(script, "1234\\n5");
        r.expect("含数字文本必须正常作答");
        assert_eq!(
            keys,
            vec!["3".to_string(), "enter".to_string()],
            "文本里的 1234 只出现在字符通道，键通道仅 [定位, 回车]"
        );
        assert_eq!(texts, vec!["1234\\n5".to_string()]);
    }

    /// **空文本 → 零投递拒绝**（防「点了发送但输入框空」把对话框搞乱：空回车在多选框
    /// 里是**切换高亮行勾选**——K9）。还原动作：删掉空文本检查 → 本用例先红。
    #[test]
    fn free_text_empty_is_rejected_without_any_key() {
        let script = vec![single_free_text_screen()];
        // 空串 / 纯空白 / 真换行（归一后是字面 \n，trim 后仍空——注意 `"\\n"` 是
        // **字面反斜杠+n 两字符**，那是合法文本不是空文本，故此处用真换行 `"\n"`）
        for empty in ["", "   ", "\n", "\t"] {
            let (r, keys, texts) = run_free_text_script(script.clone(), empty);
            assert!(r.is_err(), "{empty:?} 必须被拒");
            assert!(keys.is_empty() && texts.is_empty(), "空文本零投递");
        }
    }

    /// 形态门（§2.4/§2.8）：只有 claude 的自由作答序列已定案；其余工具返回 Err
    /// （端点据此降级为「请在终端作答」，**不假装能发**）。
    /// 还原动作：把 `free_text_supported` 改成恒 true → 循环里的断言先红。
    #[test]
    fn free_text_supported_only_for_claude() {
        let shape = free_text_shape("claude").expect("claude 自由作答已实机定案");
        assert_eq!(shape.locate_label, "Type something");
        assert_eq!(
            shape.submit_key, "enter",
            "文本之后唯一按键是回车（不带数字不带 Esc）"
        );
        // E4：kimi 升级为支持（Other 行阶段机），不再在本拒绝清单里
        for tool in ["codex", "opencode", "zcode", "dsh", "workbuddy", ""] {
            let err =
                free_text_shape(tool).expect_err(&format!("{tool} 的自由作答序列未定案，必须拒绝"));
            assert!(
                err.contains("未实测"),
                "{tool} 的拒绝原因须如实（未实测），不得假装可发：{err}"
            );
            assert!(!free_text_supported(tool), "{tool} 必须走降级文案");
        }
        // 族规格确实是 claude 的（A 族 RawVt；写死在此防端点上写成别的工具）
        let spec = free_text_family_spec();
        assert_eq!(spec.family, crate::inject::families::TuiFamily::RawVt);
        assert_eq!(spec.verified_with, "2.1.251");
    }

    /// wire ↔ 动作：`freeText` 可解析（camelCase）+ 审计摘要只记动作名（**不记正文**）。
    #[test]
    fn free_text_action_wire_and_audit_label() {
        assert_eq!(
            AnswerAction::parse("freeText"),
            Some(AnswerAction::FreeText),
            "wire 词形是 camelCase（与 multiSelect 同风格）"
        );
        assert_eq!(AnswerAction::parse("freetext"), None, "不做大小写容忍");
        assert_eq!(
            AnswerAction::FreeText.audit_label(None),
            "freeText",
            "审计摘要不承载用户正文（只记动作名——正文属用户内容）"
        );
    }

    /// 屏读判据的**形态细节**（防未来「顺手放宽」把安全面磨掉）：
    /// - 焦点标记只在行首（前缀）算数——正文里出现 `❯` 不算焦点行；
    /// - `Submit` 行判据是前缀匹配，故 `Submit answers`（Review 屏那行）**也**命中
    ///   ——这是**刻意的**：提交屏段只问「有多选提交入口吗」，Review 屏在场时它当然
    ///   也在（此时走位段的复核会因焦点不在提交行而继续走位，最终在上限处如实中止）。
    ///   把这条形态写进测试，是为了让「两个屏的判据边界」可读且可回归。
    #[test]
    fn submit_row_predicate_shape_details() {
        assert!(submit_row_focused(" ❯   Submit"), "焦点行（实机形态）");
        assert!(submit_row_focused("❯ Submit"), "无缩进焦点行");
        assert!(!submit_row_focused("    Submit"), "非焦点行不算");
        assert!(
            !submit_row_focused(" 正文里提到 ❯ 但不在行首 Submit"),
            "标记不在行首不算"
        );
        assert!(submit_row_present("    Submit"), "非焦点行在场");
        assert!(submit_row_present(" ❯   Submit"), "焦点行在场");
        assert!(!submit_row_present(" 提交"), "无关行不算");
        // Review 屏的确认项**不**命中「提交行在场」（它带编号 `1. `，而提交行无编号）
        assert!(
            !submit_row_present(" 1. Submit answers"),
            "带编号的 Review 确认项不是提交行（两者形态互斥：提交行无编号）"
        );
    }
}

/// 丁T2 **实机探测占位**（`#[ignore]`——常规门禁只编译不跑）。
///
/// 本批两项实机定案工作**无法在无真实会话的前提下完成**（需要真实 CLI 会话触发 +
/// 屏读 + 人工观察），故按任务书「`#[ignore]` 实机探测定案后实现；定案前多题卡只读」
/// 的口径：**先落占位与只读语义，把定案后要改的点写死在注释里**。
///
/// 两项待定案：
/// 1. **codex 多问题键序**（`Question n/N` 的 ←→ 导航、每题 enter 提交、全部答完后的
///    终态判定）——定案前的行为 = 多题只读卡（端点 `multi_questions` 409 + 前端只读
///    分支，已由 `question_answer_multi_questions_refused` 与 QuestionCard 的
///    `questions.length > 1` 分支锁定）；定案后 = `answer_key_sequence_for` 增加多题
///    分支（逐题导航），并补三取样夹具。
/// 2. **kimi question 类数字可靠性**（R1 Important-1 的遗留——R1 只证伪了 plan_review
///    审批框）——定案后的两个分支见 [`question_key_profile`] 的丁T2 注释。
///
/// **本占位只做一件事**：校验前置可满足性并打印探测指引（不发起任何注入——实机注入
/// 需要受控的会话与人工观察窗口，由本机人工执行）。跑法：
/// `cargo test --lib live_probe -- --ignored --nocapture`（filter 取模块名子串
/// `live_probe`——它命中 `live_probe_tests` 与 `live_probe_t5` 两个模块的全部用例；
/// 原先写的 `question_live_probe` **命中不到任何用例**，复评 F6-4 已实测订正）
#[cfg(test)]
mod live_probe_tests {
    // 本模块只做前置可满足性检查与探测指引打印（**零注入**）——不消费 `super::*` 的
    // 任何原语，故不引入它（定案后补真断言时再加回）。

    #[test]
    #[ignore = "实机验证：codex 多问题（Question n/N）导航序探测——前置=codex 已装 + 人工触发多问题 + 屏读逐屏抄录"]
    fn codex_multi_question_navigation_live_probe() {
        eprintln!(
            "丁T2 实机探测占位（codex 多问题导航序）\n\
             \n\
             定案前状态：多题卡**只读**（§2.3）——端点对 questions.len()!=1 回 409 \
             multi_questions，前端 QuestionCard 走 `questions.length > 1` 只读分支，\
             零注入按钮。本状态由自动化用例锁定（server.rs 的 \
             question_answer_multi_questions_refused + QuestionCard 只读分支用例）。\n\
             \n\
             待定案矩阵（探测时逐项抄录，勿凭源码猜测）：\n\
             ① 题间导航：←/→ 与 tab 的等价性（源码级记载两者皆有）；\n\
             ② 每题提交：enter 是「提交本题并前进」还是「提交整卷」；\n\
             ③ 全部答完后的终态：Proceed/Go back 确认屏是否存在（丁T1 codex 补测曾见\
             「确认屏数字无效、只认 Enter/Esc」——那属**未答完提交**的形态，与本项是否同屏待查）；\n\
             ④ 数字直选在多题下是否仍有效（单题已验：数字即选即交）；\n\
             ⑤ 中途 Esc 的语义（中断整回合 vs 退回上一题）。\n\
             \n\
             定案后落点：`answer_key_sequence_for` 增加 codex 多题分支；\
             `session_question_answer` 放开 len!=1 的拒绝；QuestionCard 多题分支改写入按钮。"
        );
        // 前置提示：codex 未装也能跑（本占位零副作用），只是探测无法进行
        let cli = crate::inject::approve::cached_cli_version("codex");
        eprintln!("前置检查：codex CLI 版本探测 = {cli:?}（None = 未装/探测失败——探测无法进行）");
    }

    #[test]
    #[ignore = "实机验证：kimi question 类（非审批框）数字通道可靠性——前置=kimi 已装 + 人工触发单选提问 + 屏读前后对照"]
    fn kimi_question_digit_reliability_live_probe() {
        eprintln!(
            "丁T2 实机探测占位（kimi **question 类**数字可靠性，R1 Important-1 遗留）\n\
             \n\
             证据现状：R1 的证伪（'2'+Enter 误批准、'3' 单键拒绝）针对 \
             **plan_review 审批框**；question 类**没有**同等强度证据（唯一记载是 \
             2026-09-20 矩阵 §2.3 的双路径取样）。当前实现保持 TwoPhaseSelect \
             `[数字, enter]`（不超证据改行为）——见 `question_key_profile` 的丁T2 注释。\n\
             \n\
             探测要求（三取样起，逐项抄录）：\n\
             ① 高亮在第 1 项时注入 '2'：高亮是否移动？（plan_review 框实测「无效果」）\n\
             ② 紧随的 Enter 提交的是哪一项？（提交高亮行 = 数字被吞）\n\
             ③ 数字单键是否直接进 Review 屏（矩阵记载）还是无效果（plan_review 行为）？\n\
             ④ Review 屏的确认键：Enter 还是数字？\n\
             ⑤ 多题形态下上述是否变化（本题同时是 codex 多题占位的 kimi 侧对照）。\n\
             \n\
             定案后落点：见 `question_key_profile` 丁T2 注释的两个分支（证实不可靠 → \
             与 approve 侧同族化走 ↓×k + Enter；证实可靠 → 保持本档并归档探测档案到 \
             research/refs/phase2-消息注入/）。"
        );
        let cli = crate::inject::approve::cached_cli_version("kimi");
        eprintln!("前置检查：kimi CLI 版本探测 = {cli:?}（默认表 verified_with 记为 2.0.2）");
    }
}

/// 丁T5 **实机探测占位**（`#[ignore]`——常规门禁只编译不跑）。
///
/// # 为什么还要一条实机占位（本批的自动化面已经覆盖了阶段机）
///
/// 丁T5 把提交/自由作答抽成了**可注入的编排**（`run_submit_stages` /
/// `run_free_text_stages`），门禁里已用**真机屏幕原文**的脚本序列覆盖了段推进、
/// 段中止、回执三态（`inject::question` 的 8 例 + `remote::server` 的 10 例）。
/// 但脚本覆盖的是**我们的判据**，它无法回答两个只有真机能答的问题：
///
/// 1. **判据本身是否在真机上成立**——`SUBMIT_FOCUS_MARKER`（U+276F）、
///    `Submit` 行、`Review your answers` / `Ready to submit` 三个锚、终态
///    `User answered Claude's questions:` 都取自 2026-09-21 的截图与二进制字符串，
///    而**屏读 API（`read_screen_window`）拿到的是同一屏的另一种表达**（逐行 trim、
///    窗口宽裁剪、行序）。截图上有、屏读里没有（或反过来）是真实存在的风险——只能
///    在真会话上对照一次。
/// 2. **每段之间的时间窗**是否够（`QUESTION_STAGE_POLL_TOTAL_MS = 2000ms` 是自裁值，
///    依据是探测档案里「多选三段式更久」与 ≤1s 的观察——不是测量）。
///
/// 跑法（需要真实 claude 会话与人工观察窗口；本占位**零注入**，只打印指引）：
/// `cargo test --lib live_probe -- --ignored --nocapture`（filter 取模块名子串
/// `live_probe`——它命中 `live_probe_tests` 与 `live_probe_t5` 两个模块的全部用例；
/// 原先写的 `question_live_probe` **命中不到任何用例**，复评 F6-4 已实测订正）
#[cfg(test)]
mod live_probe_t5 {
    /// **丁T5 实机定案：claude 多选提交全链 + 自由作答全链的屏读对照**
    ///
    /// ## 前置条件
    ///
    /// - claude CLI 已装（下表记 2.1.251）且已登录；
    /// - 在一个临时目录里起一个真会话（conhost 宿主，**不要**用 WT——本次取证的宿主
    ///   是 conhost，WT 的屏读行为未验）；
    /// - 会话里发一句让模型调 `AskUserQuestion` 且 `multiSelect: true`、3 个选项、
    ///   选项文本**用中文**（非 ASCII 是夹具纪律：本批两次栽在纯 ASCII 夹具上）。
    ///
    /// ## 观测点（逐项抄录，勿凭源码猜）
    ///
    /// ① 问题弹出后 `read_screen_window(pid)` 的逐行输出：`Submit` 行是否带 `❯`？
    ///    该行的**行首空白形态**如何（判据用 `trim_start` + strip 标记，需验证）？
    /// ② 点「提交勾选」后：MAM 侧日志里 `submit` 段的轮询次数、各次探测结论
    ///    （`NotYet` 的原因文案）。**Review 屏出现耗时**（决定 2000ms 是否够）。
    /// ③ Review 屏的屏读行：`Review your answers` / `Ready to submit your answers?`
    ///    是否**都**在可见窗口内（矮窗口下可能只剩后者——判据是「任一」，但要看实况）；
    ///    `1. Submit answers` 的编号与文本；
    /// ④ 确认后终态：`User answered Claude's questions:` 是否在**同一次屏读**里出现
    ///    （回执行可能被后续输出顶出窗口 → `receipt_seen` 会如实为 false，那不是 bug
    ///    而是窗口限制——需据实调整 `QUESTION_STAGE_POLL_TOTAL_MS` 或判据措辞）；
    /// ⑤ 自由作答链：`Type something` 行的屏读形态（**单选**带句点 `3. Type something.`）、
    ///    定位数字是否等于屏上编号、文本注入后该行是否变成可编辑形态（复核判据是否仍成立）；
    /// ⑤' **多选屏的自由作答行形态**（复评 F6-3 的定案项——**当前已按「不可用」处置**）：
    ///    MAM 现按截图判定为 `4. [ ] Type something`（带勾选框，故前缀判据不匹配 →
    ///    端点 409 拒绝、前端不渲染输入框）。**探测要确认的事**：该行在屏读产物里的
    ///    **确切文本**（勾选框是 `[ ]` / `[✓]` 还是别的宽度对齐变体？行内是否有额外空白？）
    ///    ——若形态明确，可按「剥掉行首勾选框前缀再匹配」放宽判据（并在
    ///    `free_text_shape_supported` 里按形态族收口），届时同步改前端
    ///    `freeTextEnabled` 的多选排除；若形态不稳定，维持拒绝。
    /// ⑥ **中止路径的真机验证**：在 Review 屏出现前人为按 Esc（模拟「屏读没等到」），
    ///    观察 MAM 是否如实中止且**没有**发出确认数字（对照审计 result=aborted:review）。
    ///
    /// ## 定案后落点
    ///
    /// 判据漂移 → 改对应常量（`SUBMIT_FOCUS_MARKER` / `REVIEW_*_ANCHOR` /
    /// `ANSWERED_RECEIPT_ANCHOR` / `FREE_TEXT_ROW_LABEL`）并把真机屏读**原文**存成
    /// 下一批的夹具；时间窗不足 → 调 `QUESTION_STAGE_POLL_TOTAL_MS` 并在此注记实测值。
    #[test]
    #[ignore = "实机验证：claude 多选提交/自由作答全链的屏读对照——前置=claude 已装 + 人工起会话 + 屏读逐屏抄录"]
    fn claude_submit_and_free_text_full_chain_live_probe() {
        eprintln!(
            "丁T5 实机探测占位（claude 多选提交 + 自由作答全链）
             
             本占位**零注入**：只校验前置可满足性并打印观测点（实机注入需要受控会话
             与人工观察窗口，由本机人工执行）。逐项观测点见本测试的 doc 注释。
             
             关键对照面（自动化已覆盖的部分与只有真机能答的部分）：
             - 自动化已覆盖：段推进/段中止/回执三态（8 例内核 + 10 例端点，全部用
               2026-09-21 探测档案的**真机屏幕原文**做夹具）；
             - 只有真机能答：（a）`read_screen_window` 的产物与截图是否逐行一致
               （本批判据全部取自截图与二进制字符串）；（b）2000ms 单段窗是否够。"
        );
        let cli = crate::inject::approve::cached_cli_version("claude");
        eprintln!("前置检查：claude CLI 版本探测 = {cli:?}（None = 未装/探测失败——探测无法进行；本批判据记 2.1.251）");
        // 屏读能力自检（不需要目标会话，只是证明本机有屏读 API）
        #[cfg(windows)]
        eprintln!("前置检查：Windows 屏读能力可用（read_screen_window 存在；无目标 pid 故不调用）");
        #[cfg(not(windows))]
        eprintln!("前置检查：本平台**无屏读能力** → 阶段机各段必然中止（如实回执 + 引导终端）");
    }
}
