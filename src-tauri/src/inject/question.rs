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
    Some(((index + 1) as u8 + b'0') as char).map(|c| c.to_string())
}

/// 应答动作 → 按键序列（探测定案的纯函数化；键名走 `locate_and_send_key(_spec)`
/// 键域：单字符数字 / "enter" / "esc" / "down"——"down" 按族分发为 A 族 VT 序列
/// 或 B 族 VK 形态，探测 K10 实证 VT 形态对 claude 生效）。
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
}
