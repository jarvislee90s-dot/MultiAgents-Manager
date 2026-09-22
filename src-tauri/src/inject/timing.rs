//! 注入时序常量族（**单一事实源**）——落地宪法 D20（`docs/MASTER-PLAN.md` §5.(c) 第 8 条）
//! 与批次丁计划 §2.9 的「常量收成单一事实源 + 每条自裁值附 `#[ignore]` 实测项」。
//!
//! # 宪法 D20 原文要点（本模块的存在理由，逐条）
//!
//! 任何**通过输入改变终端状态**的操作——斜杠命令（`/plan`、`/permissions`、
//! `/permission`、三期切换模型的命令等）、shift+tab 切档、上下键在菜单/对话框中选择、
//! 以及**任何后续步骤依赖终端新状态的操作**——一律不得「输入完直接操作下一步」：
//!
//! - **(a) 动态轮询**：屏读直至**读到判据本身**（不是「屏幕变了」），命中即刻停止；
//!   **禁止**用固定睡眠替代轮询；
//! - **(b) 有界**：设步长与总窗上限，超时**如实回执**且区分「未及确认」与「不符」；
//! - **(c) 例外**：注入**前**的决策依据（对话框是否在场、高亮在哪一项）为**瞬时快照**，
//!   单次读即返回、不轮询——守卫语义是「此刻的快照」，给守卫加轮询会把「拒绝注入」
//!   变成「等 N 秒再拒绝」，语义相反。
//!
//! **实例化（本仓的合规映射，改代码前先读这张表）**：
//!
//! | 注入路径 | D20 归属 | 实现点 |
//! |---|---|---|
//! | 模式回读（切档后读回当前档） | (a)(b) | [`crate::inject::mode::poll_mode_readback`] |
//! | 权限菜单 / Full Access 确认框 / 成功回执 | (a)(b) | `remote::api::poll_menu_stage` / `poll_receipt` |
//! | 问答阶段机各段 | (a)(b) | `remote::api::poll_question_stage`（同族 `poll_review_stage` / `poll_receipt_stage`） |
//! | 对话框在场守卫（T3）/ 高亮快照 / 切档前的 `before` / GET 当前档 | **(c) 例外** | 单次读，不轮询——**不要**给它们加窗 |
//! | 消息注入确认（`confirm.rs`） | 已是轮询（等的是**会话文件落盘**，不是屏幕） | 步距 500ms，`confirm::PROBE_INTERVAL_MS` |
//!
//! # 为什么这些窗都按「注入路径的真实开销」定，而不是按人手的观感
//!
//! **注入路径的真实开销 = 文本分块 + `SUBMIT_DELAY_MS` + TUI 重绘**：
//! `injector.locate_and_inject_spec` 把正文按 [`CHUNK_CHARS`] 分块写、块间
//! [`CHUNK_GAP_MS`] 睡眠（长文/慢消费者另走背压），正文写完再固定睡
//! [`SUBMIT_DELAY_MS`] 才单独发提交回车，回车之后目标 TUI 才排到绘制那一帧。
//! 因此「按完键到屏上出现新状态」的延迟**必然**大于人手敲同一命令的观感
//! ——这正是 2026-09-22 两条实机观察的机理：
//!
//! - **观察①**：claude 切档后回读**太快** → 报「回读与预期不符」，而目视终端**已切到**
//!   预期档、刷新浏览器即一致（屏幕重绘未及，**不是切换失败**）；
//! - **观察②**：codex `/permissions` 菜单**屏读没时间** → 须等屏幕更新再发后续键。
//!
//! # 取值的证据等级（**如实申报：本模块现有值全部是自裁值**）
//!
//! 「自裁」= 由已知时序量级推算的保守值，**不是**本机实测。每个自裁值都指向一条
//! `#[ignore]` 实测项（见本模块 `d20_live_probe_tests`，跑法见那里的模块文档）；
//! 实测回填时必须同时改值与 `tests` 里的钉值断言（钉值过不了就是没回填）。
//!
//! # 边界（与 [`super::families`] 的分工，防「同一判据两处实现」）
//!
//! - **本模块**：注入**节拍**（分块/块间隔/提交延迟）与**屏读轮询窗**（步长 + 各阶段总窗）；
//! - [`super::families`]：族规格表（TUI 族/版本指纹/消费速率）与**预算/阈值**
//!   （总预算、背压斜率、排空阈值、判冻窗）——那些约束的是**单次注入的时长上限**，
//!   不是「等屏幕更新」的窗。
//!
//! 分块三常量**定义在本模块**，[`super::families`] 只 `pub use` 转出（既有
//! `families::CHUNK_CHARS` 等路径不变，但**禁止**在那边重新定义）。

// ===== 第一组：注入后的屏读轮询（步长 + 各阶段总窗）=====

/// 屏读轮询步长（毫秒）——D20(a)(b) 的「步长」，四路共用一处。
///
/// **原 `remote::api::MENU_POLL_STEP_MS`**：D20 起它同时是菜单/确认框/回执/模式回读/
/// 问答阶段机五路的步长，名字里的 `MENU_` 已不足以描述用途，故收进本模块并改名
/// （**值不变，仍是 100ms**；改名只为让「谁在用」一眼可查）。
///
/// **依据（自裁）**：100ms 是「一次屏读的开销（阻塞 FFI，本机毫秒级）之上再加一点余量」
/// ——比它更密只是空转读同一帧，比它更疏会在最坏情况下多等一整拍。
pub const POLL_STEP_MS: u64 = 100;

/// **权限菜单**轮询窗（毫秒）：第一段 `/permissions` 回车后，轮询等菜单画出；
/// 窗尽仍读不到 → **中止第二段并如实回执**（不盲发方向键）。
///
/// **依据（自裁，2026-09-22 D20 放大）**：旧值 600ms 的依据是「M9R 人工在终端里敲
/// `/permissions` 后手按 ↑+Enter 的时序」，**不代表 MAM 注入路径**——注入路径多了
/// 文本分块 + [`SUBMIT_DELAY_MS`] + TUI 重绘（见模块文档）。用户实机观察②「屏读根本
/// 没时间」就是低估的表现。1500ms ≈ 10 × [`SUBMIT_DELAY_MS`]，给重绘与慢机器留足余量，
/// 又远小于用户可感知的「卡住」阈值。**1500ms 同样是自裁值**，实测项见
/// `d20_live_probe_tests::d20_menu_paint_latency_live_probe`。
pub const MENU_POLL_TOTAL_MS: u64 = 1_500;

/// **Full Access 二次确认框**轮询窗（毫秒）：第二段提交后等确认框画出。
///
/// **依据（自裁）**：与 [`MENU_POLL_TOTAL_MS`] 同属「提交后的下一次重绘」，同量级故取同值。
/// **与菜单窗的关键差别是超时的语义**：菜单读不到 = 中止（功能不可用）；确认框读不到
/// = **不当作失败**（用户可能此前关过该警告，codex 二进制有 `Continue and don't warn
/// again.`）→ 继续走成功回执核验。实测项见
/// `d20_live_probe_tests::d20_confirm_and_receipt_latency_live_probe`。
pub const CONFIRM_POLL_TOTAL_MS: u64 = 1_500;

/// **成功回执核验**轮询窗（毫秒）：提交后等工具打印 `Permissions updated to …` /
/// `Permission mode: …`。
///
/// **依据（自裁）**：回执行是提交后 TUI 打印的一行，比菜单/确认框更快——但**同样**要
/// 等提交回车那一帧之后，故给同量级的窗（旧值 600ms 的「更快所以更短」的推理在本仓
/// 没有实测支撑，D20 起与其他两窗取齐）。窗末仍未见 → `receipt_seen=false` →
/// 回执 `verified:false` + 「请人工核对」，**不当作失败**。实测项同
/// `d20_live_probe_tests::d20_confirm_and_receipt_latency_live_probe`。
pub const RECEIPT_POLL_TOTAL_MS: u64 = 1_500;

/// **模式回读**轮询窗（毫秒）：任意切档注入（shift+tab / 斜杠命令 / 菜单三段式）之后，
/// 轮询屏读当前档直到**读到判据本身**（D20(a)），命中即停。
///
/// **依据（自裁，新增值的理由）**：修复用户实机观察①——旧实现是「固定睡 150ms 后
/// **单次**读」，屏幕尚未重绘就读 → 误报「回读与预期不符」。回读等的东西与菜单/回执
/// 同一类（注入后的下一次重绘），故与三窗取齐 1500ms。实测项见
/// `d20_live_probe_tests::d20_mode_readback_repaint_latency_live_probe`（该探针直接量
/// 「动作 → 首次读到目标档」的耗时，用于标定本值）。
pub const MODE_READBACK_POLL_TOTAL_MS: u64 = 1_500;

/// **问答阶段机**单段轮询窗（毫秒）：多选提交（提交屏 → Review 屏 → 终态）与自由作答
/// （定位行 → 文本 → 终态）的每一段各给一个窗。
///
/// **依据（自裁，D20 起保持不变——理由逐条）**：本值的原始依据是**实测档**（2026-09-21
/// 探测档案：「数字直答延迟实测 <1s（截图即见 UI 关闭）」，本机 `s8-submit` →
/// `s8-submitted` 实测间隔 ≈1s 内），比三窗的「人手敲命令」依据更接近注入路径；
/// 且 2000ms **已经大于** D20 放大的三窗值（1500ms），不存在观察②那种「窗小于真实开销」
/// 的证据。**不跟着调**：没有新证据时改动既有无实测支撑的窗，只会把一次无依据的变更
/// 记在账上（结论不得超过证据）。若 T5 实机探针（`inject::question::live_probe_t5`）
/// 量出超窗，再按实测回填。
pub const QUESTION_STAGE_POLL_TOTAL_MS: u64 = 2_000;

/// 总窗（毫秒）→ **轮询轮数**（D20(b)：窗 = 步长 × 轮数）。
///
/// 取整向上且至少 1 轮——`0 轮` 会让「窗」退化成「不读」，那不是有界轮询而是放弃。
/// **为什么用轮数表达窗**：轮数是一个可在门禁内**确定性驱动**的量（测试的「睡眠」
/// 是空操作，墙钟不前进），而墙钟 deadline 在门禁里要么真等满窗、要么得注入时钟缝。
/// 两者语义等价：生产侧每轮固定 [`POLL_STEP_MS`]，故轮数 × 步长 = 墙钟上限。
pub const fn poll_rounds(total_ms: u64) -> u32 {
    let rounds = total_ms.div_ceil(POLL_STEP_MS);
    if rounds == 0 {
        1
    } else {
        rounds as u32
    }
}

/// **有界轮询内核**（D20(a)(b) 的最小实现）：最多 `rounds` 拍，每拍先 `attempt`
/// （`Some` = **读到判据本身** → 立即返回，命中即停），未命中则 `settle` 一拍照步长等重绘。
///
/// # 为什么把它抽出来（本批的教训）
///
/// 「窗」此前是各调用点自带的**墙钟 deadline**（`Instant::now() + total_ms`）——那种写法
/// 在门禁里要么真等满窗（测试变慢），要么得再缝一个时钟，于是「窗到底读了几拍」这条
/// 判据只有实机能覆盖（本批已两次栽在「真机路径没有自动化替代」上）。
/// 用**拍数**表达窗（`窗 = 步长 × 轮数`）后，`attempt`/`settle` 都是注入的闭包：
/// 测试喂一个计数闭包就能断言「读了几拍、命中后还读不读、窗尽是什么结果」，零睡眠。
///
/// # 命中即刻停止（D20(a)）
///
/// `attempt` 返回 `Some` 的那一拍**不再 settle**（命中即停，不浪费固定延迟）；
/// 末拍也不再 settle（等下去没有下一拍可读）。
pub fn bounded_poll<P, S, T>(rounds: u32, mut attempt: P, mut settle: S) -> Option<T>
where
    P: FnMut() -> Option<T>,
    S: FnMut(),
{
    let effective = rounds.max(1); // 0 拍 = 不读 = 放弃，不是有界轮询
    for i in 0..effective {
        if let Some(v) = attempt() {
            return Some(v);
        }
        if i + 1 < effective {
            settle();
        }
    }
    None
}

// ===== 第二组：文本分块与提交延迟（既有值，**数值不变**，D20 起只挪位置）=====

/// 每块字符数（80 字符 = 160 个 keydown+keyup 事件，单次写入单元）。
///
/// 原 `families::CHUNK_CHARS`——**值不变**，D20 起归本模块（时序族的单一事实源）；
/// [`super::families`] 仅 `pub use` 转出以保持既有路径。
pub const CHUNK_CHARS: usize = 80;

/// 块间隔毫秒（给目标 TUI 留输入缓冲消费窗口）。原 `families::CHUNK_GAP_MS`，值不变。
pub const CHUNK_GAP_MS: u64 = 50;

/// 正文写完后到提交回车的延迟毫秒（回车独立块不进 [`super::families::chunk_plan`]
/// ——执行层固定此延迟后单批发回车）。原 `families::SUBMIT_DELAY_MS`，值不变。
///
/// **它的意义在 D20 里被放大了**：本值是「注入路径真实开销」的固定项之一（见模块文档），
/// 所有屏读窗的下限都必须 ≥ 它 + 一次重绘；把屏读窗写到比它还小，就是观察②的形态。
pub const SUBMIT_DELAY_MS: u64 = 150;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inject::families;

    /// **全部时序常量的钉值关**（脆弱常量：改任一数值必须过此关并同步实测回填）。
    ///
    /// 还原动作（变异③）：把 `MENU_POLL_TOTAL_MS` / `CONFIRM_POLL_TOTAL_MS` /
    /// `RECEIPT_POLL_TOTAL_MS` 改回 600 → 本测试先红（这正是「不许悄悄缩窗」的锁）。
    #[test]
    fn timing_constants_are_pinned() {
        assert_eq!(POLL_STEP_MS, 100);
        assert_eq!(MENU_POLL_TOTAL_MS, 1_500, "D20：观察②的低估窗已放大");
        assert_eq!(CONFIRM_POLL_TOTAL_MS, 1_500);
        assert_eq!(RECEIPT_POLL_TOTAL_MS, 1_500);
        assert_eq!(MODE_READBACK_POLL_TOTAL_MS, 1_500, "D20：观察①的修复窗");
        assert_eq!(
            QUESTION_STAGE_POLL_TOTAL_MS, 2_000,
            "未随三窗调整（理由见其文档）"
        );
        assert_eq!(CHUNK_CHARS, 80);
        assert_eq!(CHUNK_GAP_MS, 50);
        assert_eq!(SUBMIT_DELAY_MS, 150);
    }

    /// 窗 → 轮数（D20(b) 的「步长 × 轮数」表达）：三窗与回读窗各 15 拍、问答 20 拍；
    /// 非整除向上取整、0 也至少 1 拍（**不读**不是有界轮询）。
    #[test]
    fn poll_rounds_are_bounded_and_nonzero() {
        assert_eq!(poll_rounds(MENU_POLL_TOTAL_MS), 15);
        assert_eq!(poll_rounds(CONFIRM_POLL_TOTAL_MS), 15);
        assert_eq!(poll_rounds(RECEIPT_POLL_TOTAL_MS), 15);
        assert_eq!(poll_rounds(MODE_READBACK_POLL_TOTAL_MS), 15);
        assert_eq!(poll_rounds(QUESTION_STAGE_POLL_TOTAL_MS), 20);
        assert_eq!(
            poll_rounds(150),
            2,
            "非整除向上取整（宁可多等一拍，不少给）"
        );
        assert_eq!(
            poll_rounds(0),
            1,
            "窗为 0 也要读一次（0 轮 = 放弃，不是有界轮询）"
        );
    }

    /// **单一事实源的对账**：分块三常量在本模块定义，[`families`] 侧是**转出**而非
    /// 另一份定义——两处取值必须恒等（若有人在 families.rs 里重新定义一份，
    /// 数值一旦分叉本测试即红）。
    #[test]
    fn chunk_constants_have_one_definition() {
        assert_eq!(families::CHUNK_CHARS, CHUNK_CHARS);
        assert_eq!(families::CHUNK_GAP_MS, CHUNK_GAP_MS);
        assert_eq!(families::SUBMIT_DELAY_MS, SUBMIT_DELAY_MS);
        // 分块计划仍按同一常量算（回归：挪位置不得改动行为）
        let spec = families::family_for("claude").unwrap();
        assert_eq!(families::chunk_plan(&"x".repeat(161), &spec).text_chunks, 3);
    }

    /// **窗必须显著大于注入路径的固定开销**（D20 的机理，做成机器可验的下限）：
    /// 提交延迟之后才轮到重绘，故每个屏读窗都应 ≥ [`SUBMIT_DELAY_MS`] 的若干倍——
    /// 这正是观察②「窗比真实开销还小」的判据化。三窗至少 5×（≥750ms），
    /// 回读窗同规；问答阶段机窗更大。
    #[test]
    fn windows_dominate_submit_delay() {
        for (name, total) in [
            ("菜单窗", MENU_POLL_TOTAL_MS),
            ("确认框窗", CONFIRM_POLL_TOTAL_MS),
            ("回执窗", RECEIPT_POLL_TOTAL_MS),
            ("模式回读窗", MODE_READBACK_POLL_TOTAL_MS),
        ] {
            assert!(
                total >= SUBMIT_DELAY_MS * 5,
                "{name}（{total}ms）必须显著大于注入路径的提交延迟（{SUBMIT_DELAY_MS}ms）\
                 ——否则就是用户实机观察②的形态（屏读没时间）"
            );
        }
        // 两常量之间的比较 → 编译期断言（const 块；clippy::assertions_on_constants 的
        // 要求，与 families.rs 的 FALLBACK_SPEC 钉值同款）。**问答窗不得小于三窗**：
        // 它的依据是实测档（见其文档），若有人把它调到比自裁窗还小，本关先红。
        const _: () = assert!(QUESTION_STAGE_POLL_TOTAL_MS >= MENU_POLL_TOTAL_MS);
    }
}

/// **D20 实机实测项**（`#[ignore]`——常规门禁只编译不跑）。
///
/// # 把自裁值变成实测值的方法（三个探针共用一条测法）
///
/// 三个探针都在**真实 conhost 窗口**上按 [`POLL_STEP_MS`] 轮询屏读，逐拍打印
/// `(elapsed_ms, 判据状态)`，把「人工/手机端动作 → 判据在屏上出现」的耗时量出来。
/// 它们**不发起注入**（注入需要受控会话与人工观察窗口，与丁T3/T4/T5 的实机占位同口径）：
/// 动作由人工在终端里做（手敲路径），或由手机端点一次按钮（**MAM 注入路径**）。
///
/// **两条读数要分开看**（这是本探针最有用的一点）：手敲路径与 MAM 注入路径的读数之差
/// ≈ 注入路径的固定开销（文本分块 + [`SUBMIT_DELAY_MS`] + 本探针自身的轮询量化误差
/// ≤ [`POLL_STEP_MS`]）。回填常量时用**较大**的那个读数。
///
/// # 跑法
///
/// ```text
/// # 前置：Windows + 真 conhost 窗口；目标 CLI 已在跑（`MAM_D20_PROBE_PID` = 它的 pid）
/// set MAM_D20_PROBE_PID=12345
/// set MAM_D20_PROBE_TOOL=claude        # 模式回读探针用：claude/opencode/codex/kimi
/// cargo test --lib d20_live_probe -- --ignored --nocapture --test-threads=1
/// ```
///
/// **前置缺失时的行为（写死在代码里，不靠注释承诺）**：
///
/// - 未设 `MAM_D20_PROBE_PID` → 打印前置清单与步骤后**直接返回**（不是通过、也不是失败
///   ——只是没测）；
/// - 设了 pid 但**一次成功的屏读都没发生**（全程 `None`）→ **panic**，带明确原因
///   （pid 错 / 非 conhost 宿主 / AttachConsole 全路径失败）。
///
/// **为什么「拍数 > 0」不能当成功判据**（实测教训）：屏读失败时本探针**照样会拍**
/// （每拍试一次）——只数拍数的话，一个完全不存在的 pid 也能让探针「跑完不报错」。
/// 故判据是**成功读数**的次数：`read_once` 返回 `Some` 才算数。**这条不是纸面要求**：
/// 本探针在实现期用真实 claude pid 跑过（读到 `Some(Some(Plan))`），也用不存在的 pid
/// 跑过（确认它会 panic 而不是静默通过）。
///
/// # 各探针去测哪一条自裁值
///
/// | 探针 | 观测点 | 对应常量（现自裁值） |
/// |---|---|---|
/// | [`d20_mode_readback_repaint_latency_live_probe`] | 切档动作 → 首次读**到目标档**的耗时 | [`MODE_READBACK_POLL_TOTAL_MS`]（1500ms）、[`POLL_STEP_MS`]（100ms） |
/// | [`d20_menu_paint_latency_live_probe`] | `/permissions` 回车 → 菜单可屏读的耗时 | [`MENU_POLL_TOTAL_MS`]（1500ms） |
/// | [`d20_confirm_and_receipt_latency_live_probe`] | Full Access 提交 → 确认框可读 / 确认后 → 成功回执行 | [`CONFIRM_POLL_TOTAL_MS`]（1500ms）、[`RECEIPT_POLL_TOTAL_MS`]（1500ms） |
///
/// 回填落点：改本模块常量 + `tests::timing_constants_are_pinned` 的钉值 + 在探针文档里
/// 记下实测日期与读数（**三处一起改**，只改常量等于没回填）。
#[cfg(test)]
mod d20_live_probe_tests {
    use super::*;
    use std::cell::Cell;

    /// 探针取目标 pid（`MAM_D20_PROBE_PID`）；缺省 None（探针打印清单后返回）。
    fn probe_pid() -> Option<u32> {
        match std::env::var("MAM_D20_PROBE_PID") {
            Ok(v) => v.trim().parse::<u32>().ok(),
            Err(_) => None,
        }
    }

    /// 探针取工具名（`MAM_D20_PROBE_TOOL`；模式回读与菜单探针用，缺省 `claude`）。
    fn probe_tool() -> String {
        std::env::var("MAM_D20_PROBE_TOOL")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "claude".to_string())
    }

    /// 前置清单（三个探针共用；`steps` 是各探针自己的操作步骤）。
    fn print_prelude(title: &str, steps: &str) {
        eprintln!(
            "{title}\n\
             \n\
             前置检查：\n\
             - 平台 = {}（屏读是 Windows 能力；非 Windows 下 read_screen_window 恒 None，\
             本探针无法测任何读数）\n\
             - MAM_D20_PROBE_PID = {}\n\
             \n\
             跑法（在同一个 cmd 里先设环境变量再跑测试）：\n\
             set MAM_D20_PROBE_PID=<目标 CLI 的 pid>\n\
             set MAM_D20_PROBE_TOOL=<claude|opencode|codex|kimi>\n\
             cargo test --lib d20_live_probe -- --ignored --nocapture --test-threads=1\n\
             \n\
             步骤（人工）：\n{steps}\n\
             \n\
             读数怎么用：每条观测点都有对应常量（见本模块 d20_live_probe_tests 的表），\
             把「动作 → 判据出现」的耗时与当前自裁值对照；明显偏小/偏大就在本模块的\n\
             常量文档与 tests::timing_constants_are_pinned 三处一起回填。\n\
             注意：本探针自身有 ≤ POLL_STEP_MS（{POLL_STEP_MS}ms）的量化误差——读数宁可\
             偏大（保守），回填时按较大值取。",
            std::env::consts::OS,
            match probe_pid() {
                Some(p) => p.to_string(),
                None => "（未设置——本探针只能打印清单）".to_string(),
            },
        );
    }

    /// 屏读一拍（Windows 才有实现；非 Windows 恒 None）。
    fn read_once(pid: u32) -> Option<Vec<String>> {
        #[cfg(windows)]
        {
            crate::inject::windows_console::read_screen_window(pid).ok()
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
            None
        }
    }

    /// **收尾断言（三个探针共用）**：至少发生一次**成功的**屏读，否则 panic。
    ///
    /// 判据刻意不是「拍数 > 0」——屏读失败时探针照样会拍（每拍试一次），只数拍数的话
    /// 一个不存在的 pid 也能「跑完不报错」（实现期实测抓获：bogus pid 下读到 36 拍）。
    fn assert_some_read(pid: u32, attempts: u32, ok_reads: u32) {
        assert!(
            ok_reads > 0,
            "一次成功的屏读都没有（pid={pid}，试了 {attempts} 拍）——没有可测的读数。\
             检查：① pid 是否是目标 CLI 的进程号（任务管理器/`tasklist`）；\
             ② 目标是否跑在 **conhost** 里（Windows Terminal 宿主未验证）；\
             ③ 该进程是否有可见控制台窗口（无窗口的后台进程读不到）"
        );
        eprintln!(
            "[D20-探针] 收尾：成功屏读 {ok_reads}/{attempts} 拍（步长 {POLL_STEP_MS}ms）——\
             若全程只打印过一行状态，说明窗内没有发生动作，请重跑并在步骤 2/3 期间操作。"
        );
    }

    /// **模式回读的实测项**：真实切档后，记录「动作 → 首次读到目标档」的实际耗时。
    ///
    /// 用于标定 [`MODE_READBACK_POLL_TOTAL_MS`]（现自裁 1500ms）与核对
    /// [`POLL_STEP_MS`]（现自裁 100ms）够不够密——**这两个值就是用户实机观察①的
    /// 病根**（旧实现固定睡 150ms 后单次读 → 屏未重绘）。
    ///
    /// # 观测点（逐条抄录）
    ///
    /// 1. 动作前底栏是什么档（探针首拍会打印）；
    /// 2. **手敲路径**：人工在终端按一次 `shift+tab`（claude/opencode）或在 codex/kimi 里
    ///    手敲 `/plan`——记下「按下 → 屏上出现新档」的 elapsed_ms；
    /// 3. **MAM 注入路径**：让手机端点一次切换钮（同一目标档）——记下同一条 elapsed_ms；
    /// 4. 两条读数之差 = 注入路径固定开销（文本分块 + SUBMIT_DELAY_MS=150ms 自裁值 + 重绘）；
    /// 5. **反例（观察①的形态）**：若某一拍打印「读到旧档」而下一拍读到目标档，
    ///    那条时间差就是「单次读会误报不符」的窗口宽度——它应当远小于
    ///    [`MODE_READBACK_POLL_TOTAL_MS`]，否则说明窗不够。
    #[test]
    #[ignore = "实机验证：切档动作 → 首次读到目标档的耗时（标定 MODE_READBACK_POLL_TOTAL_MS / POLL_STEP_MS；前置=Windows + 真 conhost + MAM_D20_PROBE_PID）"]
    fn d20_mode_readback_repaint_latency_live_probe() {
        let tool = probe_tool();
        print_prelude(
            "D20 实测项①：模式回读轮询（切档 → 首次读到目标档的耗时）",
            &format!(
                "1. 确认目标终端是 {tool} 会话且**空闲可输入**（无待决对话框），抄下它底栏的当前档；\n\
                 2. 人工在终端按一次 shift+tab（claude/opencode）或手敲 `/plan`（codex/kimi），\
                 记下探针打印的「首拍读到新档」elapsed_ms；\n\
                 3. 用手机端点一次**同一目标档**的切换钮（MAM 注入路径），记下同一条读数；\n\
                 4. 对照 MODE_READBACK_POLL_TOTAL_MS = {MODE_READBACK_POLL_TOTAL_MS}ms：\
                 若步骤 3 的读数接近该窗，说明窗偏紧（回填）；\n\
                 5. 若看到「拍到旧档 → 下一拍目标档」的序列，把两拍的时间差也抄下来"
            ),
        );
        let Some(pid) = probe_pid() else {
            return;
        };
        let step = std::time::Duration::from_millis(POLL_STEP_MS);
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(MODE_READBACK_POLL_TOTAL_MS * 4);
        let start = std::time::Instant::now();
        // 逐拍打印**变化**（没变化不刷屏）：`(elapsed_ms, 解析出的档)`。
        let mut last: Option<Option<crate::inject::mode::MamMode>> = None;
        let attempts = Cell::new(0u32);
        let ok_reads = Cell::new(0u32);
        while std::time::Instant::now() < deadline {
            let lines = read_once(pid);
            attempts.set(attempts.get() + 1);
            if lines.is_some() {
                ok_reads.set(ok_reads.get() + 1);
            }
            let observed = lines
                .map(|lines| crate::inject::mode::parse_mode_from_screen(tool.as_str(), &lines));
            if observed != last {
                eprintln!(
                    "[D20-回读探针] elapsed={}ms 屏读#{} → {:?}",
                    start.elapsed().as_millis(),
                    attempts.get(),
                    observed
                );
                last = observed;
            }
            std::thread::sleep(step);
        }
        assert_some_read(pid, attempts.get(), ok_reads.get());
    }

    /// **菜单绘制延迟的实测项**：`/permissions` 回车 → 菜单可屏读的耗时。
    ///
    /// 用于标定 [`MENU_POLL_TOTAL_MS`]（现自裁 1500ms）——**用户实机观察②**
    /// （「屏读根本没时间」，旧值 600ms）的直接证据来源。
    ///
    /// # 观测点
    ///
    /// 1. 动作 → `locate_menu_items` 首次返回可行动项表的耗时（**这是主读数**）；
    /// 2. 同一动作的**手敲**与**MAM 注入**两条路径各测一次（差 = 注入固定开销）；
    /// 3. 菜单项数（Guardian 关闭时会少 `Approve for me` 一项——不影响时间读数，
    ///    但抄下来便于判读「屏上是否真的画全了」）。工具取 `MAM_D20_PROBE_TOOL`
    ///    （codex `/permissions` 或 kimi `/permission`）。
    #[test]
    #[ignore = "实机验证：/permissions 回车 → 菜单可屏读的耗时（标定 MENU_POLL_TOTAL_MS；前置=Windows + 真 conhost + MAM_D20_PROBE_PID）"]
    fn d20_menu_paint_latency_live_probe() {
        let tool = probe_tool();
        print_prelude(
            "D20 实测项②：菜单绘制延迟（回车 → 菜单可屏读）",
            &format!(
                "1. 确认目标终端是 {tool} 会话且空闲可输入；\n\
                 2. 人工在终端敲 `/permissions` 回车（kimi 是 `/permission`），\
                 记下探针打印的「首次定位到菜单」elapsed_ms；\n\
                 3. 用手机端点一次权限组按钮（MAM 注入路径），记下同一条读数；\n\
                 4. 对照 MENU_POLL_TOTAL_MS = {MENU_POLL_TOTAL_MS}ms：读数接近或超过它 → 回填；\n\
                 5. 抄下菜单项数与逐行原文（判据词表是否漂移的副产物）"
            ),
        );
        let Some(pid) = probe_pid() else {
            return;
        };
        let labels = crate::inject::mode::menu_labels(tool.as_str());
        if labels.is_empty() {
            eprintln!("[D20-菜单探针] MAM_D20_PROBE_TOOL={tool} 不在词表覆盖的四家里（codex/kimi 有权限菜单）——结束");
            return;
        }
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(MENU_POLL_TOTAL_MS * 4);
        let start = std::time::Instant::now();
        let mut reads = 0usize;
        while std::time::Instant::now() < deadline {
            let found = read_once(pid).map(|lines| {
                crate::inject::mode::locate_menu_items(&lines, labels).map(|items| items.len())
            });
            reads += 1;
            if let Some(Some(n)) = found {
                eprintln!(
                    "[D20-菜单探针] elapsed={}ms 屏读#{} → 定位到 {n} 项（共读 {reads} 拍）",
                    start.elapsed().as_millis(),
                    reads
                );
                eprintln!(
                    "[D20-菜单探针] 主读数 = {}ms（动作 → 菜单可屏读）；当前自裁窗 = {MENU_POLL_TOTAL_MS}ms",
                    start.elapsed().as_millis()
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
        assert!(
            reads > 0,
            "屏读一次都没发生（pid={pid}）——检查 pid 与宿主类型（conhost 才有屏读）"
        );
        eprintln!(
            "[D20-菜单探针] 窗内未定位到菜单：要么没发生动作（请重跑并按步骤 2/3），\
             要么菜单形态与词表不符（抄下当时的屏读原文）"
        );
    }

    /// **确认框 / 成功回执延迟的实测项**：Full Access 提交 → 确认框可读、确认后 →
    /// 成功回执行出现的耗时。
    ///
    /// 用于标定 [`CONFIRM_POLL_TOTAL_MS`] 与 [`RECEIPT_POLL_TOTAL_MS`]（现各为自裁 1500ms）。
    ///
    /// # 观测点
    ///
    /// 1. 第二段提交（选中 Full Access 回车）→ `ConfirmAffirmative` 首次可行动的耗时
    ///    （**手敲**与 **MAM 注入**各测一次）；
    /// 2. 确认框提交 → `permission_receipt_verified` 首次为真的耗时；
    /// 3. 副产物：确认框逐行原文（`FULL_ACCESS_AFFIRMATIVE_KEYWORD` 是否漂移）与
    ///    回执行逐行原文（两个回执锚是否漂移）。
    #[test]
    #[ignore = "实机验证：Full Access 确认框与成功回执行的延迟（标定 CONFIRM/RECEIPT_POLL_TOTAL_MS；前置=Windows + 真 conhost codex + MAM_D20_PROBE_PID）"]
    fn d20_confirm_and_receipt_latency_live_probe() {
        print_prelude(
            "D20 实测项③：确认框 / 成功回执延迟（提交 → 确认框可读 → 回执行）",
            "1. 确认目标终端是 **codex** 会话且空闲可输入（权限菜单的第三段只有 codex × Full Access 有）；\n\
             2. 手敲 `/permissions` 回车 → 把高亮移到第 4 项 `Full Access` 回车，\
             记下「确认框首次可读」的 elapsed_ms；\n\
             3. 在确认框里选肯定项回车，记下「成功回执行首次出现」的 elapsed_ms；\n\
             4. 用手机端点一次权限组「完全信任」（MAM 注入路径）重复 2–3 步，两条读数相减即注入固定开销；\n\
             5. 若你的 codex 此前关过该警告（确认框不出现）→ 直接跳到第 3 步测回执，\
             并在报告里注明「确认框缺席」（那是正常形态，不是失败）",
        );
        let Some(pid) = probe_pid() else {
            return;
        };
        let tool = probe_tool();
        let label =
            crate::inject::mode::menu_target_label("codex", crate::inject::mode::MamMode::Bypass)
                .unwrap_or("");
        let confirm_plan = crate::inject::mode::MenuNavPlan::ConfirmAffirmative {
            tool: "codex",
            keyword: crate::inject::mode::FULL_ACCESS_AFFIRMATIVE_KEYWORD,
        };
        let window = std::time::Duration::from_millis(CONFIRM_POLL_TOTAL_MS * 4);
        let start = std::time::Instant::now();
        let attempts = Cell::new(0u32);
        let ok_reads = Cell::new(0u32);
        let mut confirm_at: Option<u128> = None;
        let mut receipt_at: Option<u128> = None;
        while start.elapsed() < window && receipt_at.is_none() {
            let lines = read_once(pid);
            attempts.set(attempts.get() + 1);
            if lines.is_some() {
                ok_reads.set(ok_reads.get() + 1);
            }
            if let Some(lines) = lines.as_deref() {
                if confirm_at.is_none()
                    && matches!(
                        confirm_plan.probe(lines),
                        crate::inject::mode::PollStep::Ready(_)
                    )
                {
                    confirm_at = Some(start.elapsed().as_millis());
                    eprintln!(
                        "[D20-确认框探针] elapsed={}ms → 确认框可读（当前自裁窗 CONFIRM_POLL_TOTAL_MS={CONFIRM_POLL_TOTAL_MS}ms）",
                        start.elapsed().as_millis()
                    );
                }
                if crate::inject::mode::permission_receipt_verified(tool.as_str(), lines, label) {
                    receipt_at = Some(start.elapsed().as_millis());
                    eprintln!(
                        "[D20-确认框探针] elapsed={}ms → 见到成功回执行「{label}」（当前自裁窗 RECEIPT_POLL_TOTAL_MS={RECEIPT_POLL_TOTAL_MS}ms）",
                        start.elapsed().as_millis()
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_STEP_MS));
        }
        assert_some_read(pid, attempts.get(), ok_reads.get());
        eprintln!(
            "[D20-确认框探针] 结束：确认框={:?} 回执行={:?}（None = 窗内没等到——\
             若你确实走了那条路径，说明窗偏紧，回填；否则是动作没走到那一步）",
            confirm_at, receipt_at
        );
    }
}
