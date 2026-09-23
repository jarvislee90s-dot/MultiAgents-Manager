//! **屏读锚点账本**（2026-09-23 用户裁决）——全工具、全场景的屏读文案单一真源。
//!
//! # 要解决的问题（实机事故驱动）
//!
//! 2026-09-23 用户实机走查：codex 权限切换恒报「权限菜单未出现或读不到档位表」，
//! 而终端里菜单**明明开着**。根因 = codex 0.156.1 把菜单 footer 从
//! `Press enter to confirm or esc to go back` 改成了 `enter select · esc back`，
//! 而代码里该锚是**写死的一句常量** → 菜单窗（标题锚↔footer 锚）不成立 →
//! 数字键定位永不通过。**一次上游改文案 = 一条功能静默失效**。
//!
//! # 设计裁决（用户 2026-09-23 提问后定案，三条）
//!
//! 1. **精确匹配保留，问题不在精确**：锚点必须逐字匹配（模糊匹配会让守卫自信地
//!    匹配到错误的行，把「读不到就拒绝」退化成「读错了也照做」）。修法是**每个
//!    槽位从「一句话」变成「一组已知文案，认任意一句」**。
//! 2. **版本号不是判据，是排序与诊断备注**：用户提出的「版本最近邻取表」作为
//!    排序规则可用，作为判据不可用——它把「我不知道」包装成「我知道」（猜错与猜对
//!    在界面上完全一样）；且版本号本身不可靠（仓内实证：codex npm 0.154.0 vs TUI
//!    自报 0.155.1；探测失败会缓存为无版本）；文案变化也不是单调的（可能改回去）。
//!    故：**屏幕证据是主键**，版本号只用于「命中项离本机多远」的软提示与报错文案。
//! 3. **不按版本筛候选**：取候选时把该 `(tool, scenario, slot)` 下**所有**已知文案
//!    都拿去试，屏上出现哪句就认哪句（多条同时命中取最长/最具体）。
//!
//! # 匹配语义
//!
//! - 调用方传**已小写化**的屏读行（本模块内所有比较均小写）；
//! - 命中 = 某行 `contains(文案)`；多条候选同时命中 → 取 `text` 最长者（更长 =
//!   更具体 = 更可能是该形态的真实文案，而非短子串偶然命中）；
//! - 一条都不命中 → [`AnchorMiss`]（调用方据此**拒绝动手**，并把候选清单与
//!   本机版本写进报错——这是本设计最重要的可观测性：下次一眼看出「在找什么」）。
//!
//! # append-only
//!
//! **新文案一律追加，旧文案永不删除**（老版本工具仍在用）。删除 = 砍掉一个还能
//! 工作的版本的支持，无收益。
//!
//! # 账本 ≠ 整屏 dump
//!
//! 账本存**锚点**（短、可判别，运行时用）；整屏 dump 存成**测试夹具**（TDD 用）。
//! 每行的 `evidence` 字段指向其来源夹具/截图——**禁止手造**（与夹具红线同源）。
//!
//! # KV 覆盖通道
//!
//! 与 `inject.approve_map` 同款：`inject.anchor_ledger` 可覆盖/追加条目，上游改词
//! 时用户可不重编译热修。**本期只做内置表**（KV 通道留待有真实热修需求时接入，
//! 避免「通道建了没人用」的死代码——台账登记）。

/// 场景 wire 词（账本 `scenario` 字段）。用字符串常量而非大枚举：迁移时
/// `scenario: S_PERMISSION_MENU` 一行替换即可，diff 小、易复核。
pub mod scenario {
    /// 权限档位菜单（codex `Update Model Permissions` / kimi `Select permission mode`）
    pub const PERMISSION_MENU: &str = "permission_menu";
    /// Full Access 二次确认框（codex `Enable full access?`）
    pub const FULL_ACCESS_CONFIRM: &str = "full_access_confirm";
    /// claude **计划批准框**（`Claude has written up a plan`，dialog.rs 的
    /// `locate_plan_dialog` 用它定位计划体范围）
    pub const PLAN_APPROVE: &str = "plan_approve";
    /// **回合忙/闲**屏读判据（`esc to interrupt` 等底栏标记；confirm.rs 的
    /// 回合停等 + queue.rs 插队等回合停都消费）
    pub const TURN_STATE: &str = "turn_state";
    /// claude **队列在场**提示（`press up to edit queued messages`；confirm.rs 用它
    /// 判输入行内容是否其实是排队消息回显）
    pub const QUEUE_HINT: &str = "queue_hint";
    /// **问答场景**（AskUserQuestion / request_user_input 卡片）：答题完成的
    /// **终态回执锚**（claude `User answered Claude's questions:` / kimi
    /// `● Collected your answers` / codex `• Questions 1/1 answered` / opencode
    /// `# Questions`）——question.rs 各阶段机的终态段消费
    pub const QUESTION: &str = "question";
    /// 问答**提交前确认屏**（claude `Review your answers` 确认屏 / kimi
    /// `Ready to submit your answers?` Review 汇总屏）——question.rs 各阶段机
    /// 判「确认屏在场」的标题锚
    pub const QUESTION_REVIEW: &str = "question_review";
}

/// 槽位 wire 词（账本 `slot` 字段）：同一场景内的不同位置。
pub mod slot {
    /// 标题锚（菜单/对话框顶部的标题行）
    pub const TITLE: &str = "title";
    /// footer 锚（底部操作提示行；与标题锚成对圈出窗口）
    pub const FOOTER: &str = "footer";
    /// 成功回执行锚（工具自己宣布完成）
    pub const RECEIPT: &str = "receipt";
    /// **忙态标记**（回合运行中屏上的提示语，如 `esc to interrupt`）
    pub const BUSY: &str = "busy";
    /// **在场提示**（某状态在场的证据行，如排队消息提示）
    pub const PRESENT: &str = "present";
    /// **确认项行锚**（Review 确认屏上的提交选项行，如 claude `1. Submit answers`）
    pub const CONFIRM: &str = "confirm";
}

/// 账本单行：一条**屏读文案**及其取证元数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorRow {
    /// 工具 id（`codex` / `claude` / `kimi` / `opencode`，与 adapter 的 tool_id 同域）
    pub tool: &'static str,
    /// 场景（见 [`scenario`]）
    pub scenario: &'static str,
    /// 槽位（见 [`slot`]）
    pub slot: &'static str,
    /// 屏上原文（**小写**；比较时调用方也小写化）
    pub text: &'static str,
    /// 该文案实测到的工具版本（**诊断用**，不参与筛选）
    pub observed_version: &'static str,
    /// 取证出处（探针日期 + dump 文件名 / 用户截图；禁止手造）
    pub evidence: &'static str,
}

/// 账本内置表（**append-only**；新文案追加在末尾，旧行永不移除）。
///
/// 每行的 `evidence` 必须能指到一份真实取证（探针 dump 文件 / 用户截图 / 调研
/// 文档），这是「结论不超过证据」纪律在账本上的投影。
pub const ANCHOR_LEDGER: &[AnchorRow] = &[
    // ===== codex 权限菜单：标题 =====
    AnchorRow {
        tool: "codex",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::TITLE,
        text: "update model permissions",
        observed_version: "0.154.0",
        evidence: "戊探D 2026-09-22 权限菜单屏读底料 §2.1（夹具 codex-perm-menu-idle.txt）",
    },
    // ===== codex 权限菜单：footer（本次事故的两个变体）=====
    AnchorRow {
        tool: "codex",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::FOOTER,
        text: "press enter to confirm or esc to go back",
        observed_version: "0.154.0",
        evidence: "戊探D 2026-09-22（夹具 codex-perm-menu-idle.txt；旧形态，0.156.1 已不再出现）",
    },
    AnchorRow {
        tool: "codex",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::FOOTER,
        text: "enter select · esc back",
        observed_version: "0.156.1",
        evidence: "2026-09-23 用户实机走查截图 + 自建会话探针 dump（夹具 codex-perm-menu-newfooter-escback.txt / codex-perm-menu-nonadmin-sandbox.txt）",
    },
    // ===== codex Full Access 二次确认框：标题 =====
    AnchorRow {
        tool: "codex",
        scenario: scenario::FULL_ACCESS_CONFIRM,
        slot: slot::TITLE,
        text: "enable full access?",
        observed_version: "0.154.0",
        evidence: "2026-09-23 用户实机（夹具 codex-full-access-confirm.txt）",
    },
    // ===== codex 权限切换成功回执 =====
    AnchorRow {
        tool: "codex",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::RECEIPT,
        text: "permissions updated to",
        observed_version: "0.154.0",
        evidence: "实机取证档案 §3（用户 2026-09-23 走查复现）",
    },
    // ===== kimi 权限菜单（已入档，本次一并对齐账本口径）=====
    AnchorRow {
        tool: "kimi",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::TITLE,
        text: "select permission mode",
        observed_version: "2.0.0",
        evidence: "戊探D kimi 段（夹具 kimi-perm-menu-permission.txt）",
    },
    AnchorRow {
        tool: "kimi",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::FOOTER,
        text: "enter select · esc cancel",
        observed_version: "2.0.0",
        evidence: "戊探D kimi 段（夹具 kimi-perm-menu-permission.txt；注意与 codex 新 footer 前缀相同、后缀不同）",
    },
    AnchorRow {
        tool: "kimi",
        scenario: scenario::PERMISSION_MENU,
        slot: slot::RECEIPT,
        text: "permission mode:",
        observed_version: "2.0.2",
        evidence: "键序大词典 §3（夹具 kimi-perm-receipt.txt）",
    },
    // ===== claude 计划批准框标题（dialog.rs，原 PLAN_TITLE_ANCHOR）=====
    AnchorRow {
        tool: "claude",
        scenario: scenario::PLAN_APPROVE,
        slot: slot::TITLE,
        text: "claude has written up a plan",
        observed_version: "2.1.251",
        evidence: "戊探E 2026-09-22 claude 对话框选项锚点底料（夹具 claude-approve-dialog-e1.txt）",
    },
    // ===== 回合忙态标记（confirm.rs TURN_BUSY_MARKER，2026-09-23 起入账本）=====
    AnchorRow {
        tool: "claude",
        scenario: scenario::TURN_STATE,
        slot: slot::BUSY,
        text: "esc to interrupt",
        observed_version: "2.1.251",
        evidence: "真机快照 screen-t9-after-enter-busy.txt（queue.rs tests::real_frames 同源）",
    },
    // ===== claude 队列在场提示（confirm.rs CLAUDE_QUEUE_HINT）=====
    AnchorRow {
        tool: "claude",
        scenario: scenario::QUEUE_HINT,
        slot: slot::PRESENT,
        text: "press up to edit queued messages",
        observed_version: "2.1.251",
        evidence: "戊探F 2026-09-22 打断式插队扩展底料（夹具 claude-queue-state.txt）",
    },
    // ===== claude 问答 Review 确认屏 + 终态回执（question.rs，原 REVIEW_*_ANCHOR /
    // ANSWERED_RECEIPT_ANCHOR，2026-09-23 收编——ceb2e1d 搁置项）=====
    AnchorRow {
        tool: "claude",
        scenario: scenario::QUESTION_REVIEW,
        slot: slot::TITLE,
        text: "review your answers",
        observed_version: "2.1.251",
        evidence: "2026-09-21 AskUserQuestion 按键语义探测 C-s8-submitted 截图 + claude 二进制 title:\"Review your answers\"（question.rs 模块文档判据表）",
    },
    AnchorRow {
        tool: "claude",
        scenario: scenario::QUESTION_REVIEW,
        slot: slot::PRESENT,
        text: "ready to submit",
        observed_version: "2.1.251",
        evidence: "同上副题 Ready to submit your answers?——矮窗口下标题行可能滚出可见区，副题是确认屏在场的证据行（question.rs REVIEW_SUBTITLE_ANCHOR 注）",
    },
    AnchorRow {
        tool: "claude",
        scenario: scenario::QUESTION_REVIEW,
        slot: slot::CONFIRM,
        text: "submit answers",
        observed_version: "2.1.251",
        evidence: "C-s8-submitted 截图 `1. Submit answers` + claude 二进制 confirmLabel:\"Submit answers\"",
    },
    AnchorRow {
        tool: "claude",
        scenario: scenario::QUESTION,
        slot: slot::RECEIPT,
        text: "user answered claude",
        observed_version: "2.1.251",
        evidence: "C-s8-final 截图 `User answered Claude's questions:`（question.rs ANSWERED_RECEIPT_ANCHOR）",
    },
    // ===== kimi 问答 Review 汇总屏 + 终态回执（戊探B 实机 + 用户 K-5 实录）=====
    AnchorRow {
        tool: "kimi",
        scenario: scenario::QUESTION_REVIEW,
        slot: slot::TITLE,
        text: "ready to submit your answers?",
        observed_version: "2.0.2",
        evidence: "戊探B + 用户 K-5 实录（screen-ki1-b1-after-char3.txt 原件；单题 Review 屏同含此行）",
    },
    AnchorRow {
        tool: "kimi",
        scenario: scenario::QUESTION,
        slot: slot::RECEIPT,
        text: "collected your answers",
        observed_version: "2.0.2",
        evidence: "戊探B M1/M2 提交后屏读原件 `● Collected your answers`（question.rs KIMI_ANSWERED_ANCHOR）",
    },
    // ===== codex 问答终态回执（戊探C 全链）=====
    AnchorRow {
        tool: "codex",
        scenario: scenario::QUESTION,
        slot: slot::RECEIPT,
        text: "answered",
        observed_version: "0.155.1",
        evidence: "戊探C 原件摘要头 `• Questions 1/1 answered`（question.rs CODEX_ANSWERED_ANCHOR）",
    },
    // ===== opencode 问答终态回执（戊探A 全链）=====
    AnchorRow {
        tool: "opencode",
        scenario: scenario::QUESTION,
        slot: slot::RECEIPT,
        text: "# questions",
        observed_version: "1.18.32",
        evidence: "戊探A E-A1 提交后 transcript 摘要段 `# Questions`（question.rs OPENCODE_ANSWERED_ANCHOR）",
    },
];

/// 取候选：`(tool, scenario, slot)` 下的**全部**已知文案（**不按版本过滤**）。
///
/// 返回顺序 = 账本顺序（确定性；调用方不应依赖「第一条最可能」）。
pub fn candidates(tool: &str, scenario: &str, slot: &str) -> Vec<&'static AnchorRow> {
    ANCHOR_LEDGER
        .iter()
        .filter(|r| r.tool == tool && r.scenario == scenario && r.slot == slot)
        .collect()
}

/// 账本命中（[`detect`] 的返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorHit {
    /// 命中的账本行
    pub row: &'static AnchorRow,
    /// 命中时的屏上行号（首个命中行）
    pub line_index: usize,
}

/// **认屏**：在已小写化的屏读行里找该槽位的任一条已知文案。
///
/// 语义（与模块文档一致）：
/// - 逐行扫，**首个命中**的候选即返回该行索引；同一行若多条候选命中，取 `text`
///   最长者（更具体）；
/// - 全不命中 → `None`（调用方据此拒绝动手 + 点名候选）。
///
/// 调用方传 `lowered`（`lines.iter().map(|l| l.to_lowercase())`）——本模块不做
/// 大小写转换，避免每帧重复分配（屏读在轮询里被高频调用）。
pub fn detect(lowered: &[String], tool: &str, scenario: &str, slot: &str) -> Option<AnchorHit> {
    let cands = candidates(tool, scenario, slot);
    if cands.is_empty() {
        return None;
    }
    for (idx, line) in lowered.iter().enumerate() {
        // 同行多命中 → 取最长（更具体）
        let mut best: Option<&'static AnchorRow> = None;
        for c in &cands {
            if line.contains(c.text) {
                match best {
                    Some(b) if b.text.len() >= c.text.len() => {}
                    _ => best = Some(c),
                }
            }
        }
        if let Some(row) = best {
            return Some(AnchorHit {
                row,
                line_index: idx,
            });
        }
    }
    None
}

/// **找不到时的诊断文案**（本设计最重要的一条可观测性）：点名「在找哪些文案、
/// 本机是什么版本」——上次事故若有这句，定位是 5 分钟而不是两轮走查。
///
/// `observed` = 本机版本（[`crate::inject::approve::cached_cli_version`] 的结果，
/// 探测失败时为 `None`，如实写「未知」）。
pub fn miss_report(tool: &str, scenario: &str, slot: &str, observed: Option<&str>) -> String {
    let cands = candidates(tool, scenario, slot);
    if cands.is_empty() {
        return format!("账本无 {tool}/{scenario}/{slot} 条目（内部表缺失，非终端问题）");
    }
    let list = cands
        .iter()
        .map(|c| format!("「{}」（实测于 {}）", c.text, c.observed_version))
        .collect::<Vec<_>>()
        .join(" / ");
    format!(
        "{tool} 屏上未出现 {scenario}/{slot} 的任一已知文案——已找过：{list}；本机 {tool} 版本={}。\
         多半是该工具改了文案：请用探针 dump 屏面后按账本格式追加一行（append-only，勿改旧行）",
        observed.unwrap_or("未知")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_lowercase()).collect()
    }

    /// **本次事故的回归锁**：新旧两个 codex footer 变体都必须能认出来
    /// （旧夹具 + 两个新夹具三份真机原文）。
    #[test]
    fn codex_menu_footer_both_variants_recognized() {
        let old = lower(&[
            "  1. Read Only",
            "  Press enter to confirm or esc to go back",
        ]);
        let hit = detect(&old, "codex", scenario::PERMISSION_MENU, slot::FOOTER)
            .expect("旧变体必须仍认得出（老版本工具还在用）");
        assert_eq!(hit.row.text, "press enter to confirm or esc to go back");
        assert_eq!(hit.row.observed_version, "0.154.0");

        let new = lower(&["  1. Read Only", "  enter select · esc back"]);
        let hit = detect(&new, "codex", scenario::PERMISSION_MENU, slot::FOOTER)
            .expect("新变体必须认得出（本次事故的直接修复）");
        assert_eq!(hit.row.text, "enter select · esc back");
        assert_eq!(hit.row.observed_version, "0.156.1");
        assert_eq!(hit.line_index, 1);
    }

    /// **kimi 与 codex 新 footer 前缀相同、后缀不同**——分家必须正确
    /// （`enter select` 是两者共有的前缀；靠 `· esc back` / `· esc cancel` 区分）。
    #[test]
    fn codex_and_kimi_footers_do_not_cross_match() {
        let kimi = lower(&["  ↑↓ navigate · Enter select · Esc cancel"]);
        assert!(
            detect(&kimi, "codex", scenario::PERMISSION_MENU, slot::FOOTER).is_none(),
            "kimi 的 footer 不得被 codex 的候选命中"
        );
        assert!(detect(&kimi, "kimi", scenario::PERMISSION_MENU, slot::FOOTER).is_some());

        let codex = lower(&["  enter select · esc back"]);
        assert!(
            detect(&codex, "kimi", scenario::PERMISSION_MENU, slot::FOOTER).is_none(),
            "codex 的 footer 不得被 kimi 的候选命中"
        );
        assert!(detect(&codex, "codex", scenario::PERMISSION_MENU, slot::FOOTER).is_some());
    }

    /// **不按版本过滤**：本机版本再新，旧文案照样认得出（这是「屏幕证据是主键，
    /// 版本只是备注」的代码面）。反例：若哪天有人加「按版本筛候选」的优化，本测
    /// 会红——旧版本工具的屏读将不可用。
    #[test]
    fn candidates_are_not_filtered_by_version() {
        let c = candidates("codex", scenario::PERMISSION_MENU, slot::FOOTER);
        assert_eq!(c.len(), 2, "两个 footer 变体都必须留在候选里：{c:?}");
    }

    /// **appoint-only 纪律的守卫**：账本里每行的 `evidence` 都必须非空（禁止无出处
    /// 的「手造条目」混入——与夹具红线同源）。
    #[test]
    fn every_row_carries_evidence() {
        for r in ANCHOR_LEDGER {
            assert!(
                !r.evidence.trim().is_empty(),
                "条目缺 evidence（禁止手造）：{r:?}"
            );
            assert!(!r.text.trim().is_empty(), "条目文案为空：{r:?}");
            assert_eq!(
                r.text,
                r.text.to_lowercase(),
                "账本文案必须小写（比较语义单一）：{r:?}"
            );
            assert!(
                !r.observed_version.trim().is_empty(),
                "条目缺 observed_version：{r:?}"
            );
        }
    }

    /// **报错点名候选与本机版本**（可观测性缺口修补）：文案必须含全部候选文案
    /// 与本机版本号，否则「下次一眼定位」不成立。
    #[test]
    fn miss_report_names_candidates_and_version() {
        let msg = miss_report(
            "codex",
            scenario::PERMISSION_MENU,
            slot::FOOTER,
            Some("0.156.1"),
        );
        assert!(
            msg.contains("press enter to confirm or esc to go back"),
            "{msg}"
        );
        assert!(msg.contains("enter select · esc back"), "{msg}");
        assert!(msg.contains("0.156.1"), "必须带上本机版本：{msg}");
        assert!(msg.contains("append-only"), "必须指明追加而非改旧行：{msg}");
        // 版本未知（探测失败）时如实写「未知」，不得假装
        let msg2 = miss_report("codex", scenario::PERMISSION_MENU, slot::FOOTER, None);
        assert!(msg2.contains("未知"), "{msg2}");
    }

    /// 未知工具/场景 → 报错如实说「账本无条目」（内部问题），不误导为终端问题。
    #[test]
    fn miss_report_reports_missing_ledger_entry() {
        let msg = miss_report("nonexistent-tool", "x", "y", Some("1.0.0"));
        assert!(msg.contains("账本无"), "{msg}");
        assert!(msg.contains("非终端问题"), "{msg}");
    }

    /// 同行多候选命中 → 取更长者（更具体）。构造：新 footer 文本含旧文案的
    /// **子串形态**时，应选更长的那个（保证具体性优先）。
    #[test]
    fn longest_candidate_wins_on_same_line() {
        // 构造一条同时含两个候选的行（现实里不会出现，此处验语义）
        let l = lower(&["  press enter to confirm or esc to go back  enter select · esc back"]);
        let hit = detect(&l, "codex", scenario::PERMISSION_MENU, slot::FOOTER).unwrap();
        // 旧文案 42 字符 vs 新文案 23 字符 → 取旧（更长）
        assert_eq!(hit.row.text, "press enter to confirm or esc to go back");
    }

    /// 空屏 / 无锚屏 → None（不空指针、不 panic）。
    #[test]
    fn empty_screen_yields_no_hit() {
        assert!(detect(
            &lower(&[]),
            "codex",
            scenario::PERMISSION_MENU,
            slot::FOOTER
        )
        .is_none());
        assert!(detect(
            &lower(&["普通输出", "• done"]),
            "codex",
            scenario::PERMISSION_MENU,
            slot::FOOTER
        )
        .is_none());
    }

    /// **question.rs 锚点收编的回归锁**（2026-09-23 收编 8 条，ceb2e1d 搁置项）：
    /// 每槽账本**首条**文案必须与收编前的编译期兜底常量**逐字一致**——收编不改
    /// 行为（question.rs 各锚函数 `.first()` 取账本、取不到回落同值常量），账本行
    /// 与兜底值一旦分叉本测即红。真机原文（原大小写）小写化后可认出。
    #[test]
    fn question_anchors_ledger_matches_pre_collection_constants() {
        // (tool, scenario, slot, 收编前常量值)
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "claude",
                scenario::QUESTION_REVIEW,
                slot::TITLE,
                "review your answers",
            ),
            (
                "claude",
                scenario::QUESTION_REVIEW,
                slot::PRESENT,
                "ready to submit",
            ),
            (
                "claude",
                scenario::QUESTION_REVIEW,
                slot::CONFIRM,
                "submit answers",
            ),
            (
                "claude",
                scenario::QUESTION,
                slot::RECEIPT,
                "user answered claude",
            ),
            (
                "kimi",
                scenario::QUESTION_REVIEW,
                slot::TITLE,
                "ready to submit your answers?",
            ),
            (
                "kimi",
                scenario::QUESTION,
                slot::RECEIPT,
                "collected your answers",
            ),
            ("codex", scenario::QUESTION, slot::RECEIPT, "answered"),
            ("opencode", scenario::QUESTION, slot::RECEIPT, "# questions"),
        ];
        for (tool, scenario, slot, pre_collection) in cases {
            let row = candidates(tool, scenario, slot)
                .first()
                .unwrap_or_else(|| panic!("{tool}/{scenario}/{slot} 账本缺条目"));
            assert_eq!(
                row.text, *pre_collection,
                "{tool}/{scenario}/{slot} 账本文案与收编前常量不一致（行为漂移）"
            );
        }
        // 真机原文（原大小写）小写化后可认：claude Review 确认屏三锚
        let review = lower(&[
            "  Review your answers",
            "  Ready to submit your answers?",
            "  1. Submit answers",
        ]);
        assert!(detect(&review, "claude", scenario::QUESTION_REVIEW, slot::TITLE).is_some());
        assert!(detect(&review, "claude", scenario::QUESTION_REVIEW, slot::PRESENT).is_some());
        assert!(detect(&review, "claude", scenario::QUESTION_REVIEW, slot::CONFIRM).is_some());
        // kimi Review 汇总屏副题锚
        assert!(detect(
            &lower(&[
                "  ↑↓ navigate · ↵ confirm",
                "  Ready to submit your answers?"
            ]),
            "kimi",
            scenario::QUESTION_REVIEW,
            slot::TITLE
        )
        .is_some());
        // 四家终态回执各自可认
        let receipts = lower(&[
            "  User answered Claude's questions:",
            "  ● Collected your answers",
            "  • Questions 1/1 answered",
            "  # Questions",
        ]);
        assert!(detect(&receipts, "claude", scenario::QUESTION, slot::RECEIPT).is_some());
        assert!(detect(&receipts, "kimi", scenario::QUESTION, slot::RECEIPT).is_some());
        assert!(detect(&receipts, "codex", scenario::QUESTION, slot::RECEIPT).is_some());
        assert!(detect(&receipts, "opencode", scenario::QUESTION, slot::RECEIPT).is_some());
    }
}
