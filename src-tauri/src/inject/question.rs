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
    /// 多选提交：三段式（down ×(n+1) → enter → '1'）
    Submit,
    /// 取消：Esc（模型收 is_error=true 拒绝回执）
    Cancel,
}

impl AnswerAction {
    /// wire 字符串 ↔ 动作（camelCase 请求体用小写词）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "select" => Some(Self::Select),
            "toggle" => Some(Self::Toggle),
            "submit" => Some(Self::Submit),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }

    /// 审计摘要形态（action#index，编号对齐 UI 从 1 起）
    pub fn audit_label(self, index: Option<usize>) -> String {
        match (self, index) {
            (Self::Select, Some(i)) => format!("select#{}", i + 1),
            (Self::Toggle, Some(i)) => format!("toggle#{}", i + 1),
            (Self::Submit, _) => "submit".to_string(),
            (Self::Cancel, _) => "cancel".to_string(),
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
            // 探测 K10 三段式：down ×(n+1)（n=选项数，不含 TUI 追加行）→ enter → '1'
            let mut seq = vec!["down"; q.options.len() + 1]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>();
            seq.push("enter".to_string());
            seq.push("1".to_string());
            Ok(seq)
        }
        AnswerAction::Cancel => Ok(vec!["esc".to_string()]),
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
        },
        QuestionKeyProfile::TwoPhaseSelect => match action {
            AnswerAction::Select => {
                if q.multi_select {
                    return Err("kimi 多选未实测，不出键".to_string());
                }
                // 两段式：数字选中 → Review 屏 → 数字 '1' Submit 确认
                let i = index.ok_or_else(|| "缺少选项序号".to_string())?;
                if i >= q.options.len() {
                    return Err(format!("选项序号越界：{i}"));
                }
                let digit = digit_key(i).ok_or_else(|| format!("选项序号超键域：{i}"))?;
                Ok(vec![digit, "1".to_string()])
            }
            AnswerAction::Toggle | AnswerAction::Submit => {
                Err("kimi 多选提交未实测，不出键".to_string())
            }
            AnswerAction::Cancel => Err("kimi 取消未实测，不出键".to_string()),
        },
        QuestionKeyProfile::ReadOnly => Err(format!("{tool} 问答键序未实测，只读展示")),
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

    /// 探测 K10：多选提交 = 三段式 down ×(n+1) → enter → '1'（n=选项数；
    /// Enter 当提交是反直觉反例 K9，序列里 enter 只出现在 Submit 行）
    #[test]
    fn submit_sequence_is_three_phase() {
        let q = multi(); // 3 选项
        assert_eq!(
            answer_key_sequence(AnswerAction::Submit, None, &q).unwrap(),
            vec!["down", "down", "down", "down", "enter", "1"],
            "submit → [down×(n+1), enter, 1]（n=3）"
        );
        // 2 选项：down ×3
        let mut q2 = multi();
        q2.options.truncate(2);
        assert_eq!(
            answer_key_sequence(AnswerAction::Submit, None, &q2).unwrap(),
            vec!["down", "down", "down", "enter", "1"]
        );
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
        assert_eq!(
            answer_key_sequence_for("claude", AnswerAction::Submit, None, &m).unwrap(),
            answer_key_sequence(AnswerAction::Submit, None, &m).unwrap()
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

    /// T3 · kimi 档（矩阵 §2.3 实机：数字 → Review 屏 → 数字确认的**两段式**）
    #[test]
    fn kimi_profile_is_two_phase() {
        let q = single();
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Select, Some(0), &q).unwrap(),
            vec!["1", "1"],
            "两段式：选中数字 → Review 屏 Submit 数字 '1'"
        );
        assert_eq!(
            answer_key_sequence_for("kimi", AnswerAction::Select, Some(2), &q).unwrap(),
            vec!["3", "1"]
        );
        // 越界/缺序号仍拒
        assert!(answer_key_sequence_for("kimi", AnswerAction::Select, Some(9), &q).is_err());
        assert!(answer_key_sequence_for("kimi", AnswerAction::Select, None, &q).is_err());
        // 未测面拒绝（多选未验、取消未验）
        assert!(answer_key_sequence_for("kimi", AnswerAction::Toggle, Some(0), &multi()).is_err());
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
}
