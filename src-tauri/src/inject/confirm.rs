//! A1 写入确认层（M9R Task 5，裁决 A1 分层写入确认）：**注入成功 ≠ 已送达**。
//!
//! - **直发**（可输入态）：以**会话文件命中**定「已送达」（提交即时发生、秒级
//!   命中）——轮询 `confirm_probe` 缝（500ms 步距、首轮立即查）查 24 字符尾戳；
//!   超时未中走**屏读回查 + 三态分诊**（D7/T3 确认判据收紧，全程为恢复动作）：
//!   滞留输入行判定 → 补按回车 → 复查 3s；**分诊口径**——非滞留（屏读无滞留
//!   草稿）= 字已被 TUI 收进内部队列 → 中性「已投递未确认」（Submitted，不重试）；
//!   滞留 + 补回车 + 命中 → 已送达；滞留 + 补回车失败 / 复查仍未中 → 真失败
//!   （防重警示文案保留；E10 尾字符丢失 1/91 即本层必要性实证）。
//! - **插队**（busy 态）：以**占用排空**确认（Windows `wait_input_drained` ≤2s）；
//!   屏读草稿尾为 best-effort 诊断（busy TUI 可能在屏读前已把草稿消费进自身
//!   缓冲，不 Gate 结果）；排空超时 = 投递超时。
//!
//! ## 结构（纯核 / 执行侧分离）
//! - **纯核（零 cfg，跨平台可测）**：[`stamp_of`]（尾戳）/ [`stamp_in_messages`]
//!   （列表含戳）/ [`stamp_hit_in_page`]（user 侧过滤 + 含戳）/
//!   [`direct_confirm_fail_copy`]（族 × 平台感知失败文案，Mac 报告 §四-C）/
//!   [`triage_screen_recovery`]（D7/T3 分诊纯核：屏读回查结果 → 直发确认三态，
//!   判定因果见该函数注）；
//! - **契约/测试面 API**：[`session_stamp_hit`]（复用会话消息读路径；flush_one 不直接
//!   用它——生产确认调用全部经 `RemoteState.confirm_probe` 缝，本函数不参加生产
//!   调用链，当前唯一消费者是 queue 测试，零接触真实文件）；
//! - **执行侧**：[`await_direct_receipt`] / [`await_jump_receipt`]（flush_one 内嵌，
//!   调用方均在 spawn_blocking——轮询用线程睡眠的阻塞语义，DB 锁外）；屏读/占用
//!   API 走 `windows_console`（cfg windows），macOS 无对应 API → 降级注释于各分支。
//!
//! 确认器「可插拔」的实现方式：**不建 per-tool 确认器**——JSONL 家（claude/codex/
//! kimi/workbuddy/dsh）直接命中会话文件；opencode/zcode 等 SQLite 家走同一读路径
//! （`read_session_messages` 的工具派发天然覆盖），确认判定只有一份（戳 + user 侧
//! 过滤）。

use std::thread::sleep;
use std::time::{Duration, Instant};

/// 尾戳长度（字符）：composed 消息的尾部片段最具区分度（正文结尾），24 字符在
/// 「截断防超长」与「防撞车」间取平（跨任务接口契约，Task 6 直接消费）。
const STAMP_CHARS: usize = 24;

/// 尾戳（跨任务接口契约）：**先剥尾部签名 → `trim_end` → 截尾 24 字符**。
///
/// # 为什么必须先剥签名（丁T3 裁2，任务书点名的真 bug）
///
/// 裁2 把来源签名从消息**头部**移到了**尾部**（`{正文} [mobile {设备名}]`）。签名
/// 一旦落在尾部，取 composed 尾部的旧口径就退化成**设备级**判据：同一设备的每条
/// 消息尾部都相同（` [mobile iPhone]`）→ 第二条消息注入后，查询第一条的戳也会命中
/// 第二条 → 确认层谎报「已送达」（跨消息假命中，`stamp_never_false_hits_across_
/// same_device_messages` 即该不变式的锁）。
///
/// # 为什么剥签名放在**本函数内部**（而不是让调用方传正文）
///
/// `stamp_of` 是跨任务接口契约（`inject::queue` 的直发确认、`tests/m9r_e2e.rs` 的
/// 独立复核、`remote/mod.rs` 生产装配的 probe 闭包多处消费）。若把「传正文而非
/// composed」的责任交给调用方：① 每个调用点都要记得先调 `strip_mobile_signature`，
/// 漏一处就静默退回假命中形态；② 未来新增调用方无从知晓这条隐性契约。放在函数内部
/// 则**不变式与数据形态绑定**：无论喂 composed 还是裸正文，取到的都是正文尾部
/// （对裸正文是恒等变换——`strip_mobile_signature` 只见形态完整的尾签名才剥）。
///
/// **注入到终端的文本仍然含签名**（正文本身当然含戳所指的那段字符，签名在其后不影响
/// 会话文件里的子串命中——`stamp_in_messages` 是 `contains` 语义）。
///
/// 其余口径不变：`trim_end` 先做（F8 尾空格修剪——终端输入行尾部空格不可见且易被
/// TUI/会话文件丢弃，戳含尾空格会系统性失配）；短于 24 字符取全串；char 边界安全
/// 截取（多字节字符不可按字节切）。**斜杠命令裸注入**（裁2）本就无签名，剥签名恒等。
pub fn stamp_of(content: &str) -> &str {
    let trimmed = super::normalize::strip_mobile_signature(content);
    let total = trimmed.chars().count();
    if total <= STAMP_CHARS {
        return trimmed;
    }
    // 倒数第 24 字符的字节起点：跳过前 total-24 个字符
    let start = trimmed
        .char_indices()
        .nth(total - STAMP_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(0);
    &trimmed[start..]
}

/// 消息列表含戳判定（纯核）：子串包含而非全等（会话文件正文可能带序号/包装，
/// 逐条 `contains`）。
pub fn stamp_in_messages<T: AsRef<str>>(msgs: &[T], stamp: &str) -> bool {
    // 空戳恒不中（防「空串 contains 恒真」的假阳性——空内容本就不该有确认语义）
    !stamp.is_empty() && msgs.iter().any(|m| m.as_ref().contains(stamp))
}

/// 消息页含戳判定（纯核）：**user 侧过滤**——只有用户消息是注入产物的落点。
/// role 归一化口径（`remote/content.rs` 的 `SessionMessage::text` / `tool_call`）：
/// kind == "user" → role == "user"；thinking / tool-call / tool-result / plan 一律
/// 归 "assistant"（agent 侧工作产物）。故 `role == "user"` 恰好等价于「用户正文
/// 消息」，不含工具结果回显（工具回显可能恰好引用注入原文，过滤防误判命中）。
/// **plan 消息不进 user 侧确认比对（T1 升格后的语义锁）**：计划一等消息
/// role=assistant，正文是 agent 产出的计划 markdown——即使其中恰好包含与注入
/// 正文相同的尾串，也不得判「已送达」（与 tool-result 回显同一条防误判线）。
pub fn stamp_hit_in_page(pg: &crate::remote::content::MessagesPage, stamp: &str) -> bool {
    let user_texts: Vec<&str> = pg
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.content.as_str())
        .collect();
    stamp_in_messages(&user_texts, stamp)
}

/// 戳探测取数上限（条）：注入命中必然落在最近几条，20 条兼顾覆盖与读开销。
/// **读量口径（质量评审 Minor 4 纠正）**：SQLite 家（zcode/opencode 等）是 SQL
/// `LIMIT 20`，便宜；JSONL 家并非逐条读取——`read_session_messages` 经
/// `read_recent_lines_with_budget` 有界尾窗（`line_budget(20)`=500 行 /
/// `byte_budget(20)`=512KB）一次截尾读入后整页解析，20 条只是页内再截取。
/// `pub(crate)`：remote/mod.rs 生产装配的 confirm_probe 闭包同源引用（单一来源）。
pub(crate) const PROBE_MESSAGE_LIMIT: usize = 20;

/// 契约/测试面 API（跨任务接口命名保留）：复用会话消息读路径
/// （`remote/api::read_session_messages_core`，与 /session-messages 端点数据同源）
/// 取最近 20 条查戳。**读失败 = 未命中**（诚实口径：确认不足不伪装成功）。
/// **不参加生产调用链（M9R 评审 F7 核实口径，2026-09-19 grep 调用点）**：生产确认
/// 路径走 `RemoteState.confirm_probe` 缝（remote/mod.rs 生产装配直调
/// `content::read_session_messages` + `stamp_hit_in_page`，不经本函数）；当前唯一
/// 消费者是 queue 测试（读路径失败语义回归）。
pub fn session_stamp_hit(
    st: &crate::remote::server::RemoteState,
    tool: &str,
    sid: &str,
    stamp: &str,
) -> bool {
    match crate::remote::api::read_session_messages_core(st, tool, sid, PROBE_MESSAGE_LIMIT) {
        Ok(pg) => stamp_hit_in_page(&pg, stamp),
        Err(_) => false,
    }
}

// ============================================================
// 执行侧（flush_one 内嵌；调用方均在 spawn_blocking，阻塞轮询安全）
// ============================================================

/// 直发确认轮询间隔（毫秒）：提交即时发生、秒级命中口径下的步距
const PROBE_INTERVAL_MS: Duration = Duration::from_millis(500);
/// 补按回车后的复查窗（毫秒）：回车提交后会话文件落盘的宽限
const RECHECK_MS: u64 = 3_000;
/// 直发确认失败回执（裁决 A1 文案）：注入成功但会话文件未见戳——提交未发生，
/// 重试由用户判断（重试语义 = 用户先检查终端再重试，不自动重发防重复正文）
const DIRECT_CONFIRM_FAIL: &str = "已注入未确认（未见会话记录），请检查终端后重试";
/// 直发确认失败回执——macOS 回车吞没特例（Mac 报告 §四-C，M3B 裁决；F2 更正
/// 平台归属，mac-reverify-b9a501c §四-B）：Mac 实测 codex **与 kimi** 注入后自动
/// 回车被 TUI 吞（疑 bracketed-paste 把尾随 \n 当 paste 内容），文本滞留 composer
/// 未提交、确认层 3s 窗找不到落盘——原文案没告诉 macOS 用户「按一次回车即好」；
/// Windows 不受此困（M6R 探测定案）。**工具集由 [`super::families::macos_enter_swallowed`]
/// 平台投影表给出**（codex/kimi=true）——族表平台无关，kimi 在 Windows 是 A 族，
/// 按族判定会漏 kimi（F2 根因），故改按「工具 × 平台」实测投影判定。
const DIRECT_CONFIRM_FAIL_MACOS_SWALLOWED: &str =
    "已注入未确认：该类工具在 macOS 注入后可能需在终端按一次回车提交，请检查后重试";
/// 屏读门槛探针长度（字符）：正文末尾至多 16 字符（屏读窗口 64 unit 的安全子集；
/// content ≤ 2 字符时取全串——「至多」语义天然覆盖）。**启发式性质（诚实口径）**：
/// 提示符文本与超短消息理论上可撞车（探针恰为终端提示符片段）；折行长文只能读
/// 到光标所在尾视觉行（`windows_console::read_input_tail` 契约），长文首部滞留
/// 不可见——判定成立才补回车，不成立不动作（保守方向安全）。
const SCREEN_PROBE_CHARS: usize = 16;
/// 插队等待占用排空上限（毫秒，§8.1）：busy TUI 消费写入缓冲的宽限
const JUMP_DRAIN_TIMEOUT_MS: u64 = 2_000;

/// T9 打断式插队：Esc 中断后等待输入行排空的预算（ms）。中断是异步生效的（模型
/// 收尾 + TUI 重绘），预算比普通插队的 2s 宽一档；超时即继续投递（best-effort，
/// 正文注入另有 backpressure 兜底）
pub const INTERRUPT_DRAIN_TIMEOUT_MS: u64 = 3_000;
/// 排空超时回执（对齐 PARTIAL_WARN 防重纪律，质量评审 Minor 3）：目标可能仍在
/// 消费，盲目重试会叠加正文——先引导人工检查终端
const DELIVERY_TIMEOUT_MSG: &str = "投递超时（目标可能仍在消费，重试前请检查终端）";

/// 直发确认失败文案选择（纯函数，F2 更正：文案按「工具 × 平台」感知，跨平台
/// 可测）：`os == "macos"` 且 [`super::families::macos_enter_swallowed`]（codex/kimi）
/// → [`DIRECT_CONFIRM_FAIL_MACOS_SWALLOWED`]（Mac 报告 §四-C——crossterm 疑
/// bracketed-paste 吞尾随回车，文本滞留 composer 未提交，须补「按一次回车」指引）；
/// 其余（claude/opencode 全平台、四工具的 Windows 形态、未实测工具默认）→ 既有
/// [`DIRECT_CONFIRM_FAIL`]。os 作参数而非函数内硬取 `std::env::consts::OS`，
/// 使全象限在任一平台可钉（Windows 上跑全绿）——生产调用点
/// （[`await_direct_receipt`] 失败臂）传 `std::env::consts::OS`。
/// **F2 根因注记**：旧签名按 `TuiFamily` 判定（family_for 平台无关单表），kimi 在
/// Windows 探测定案为 A 族（RawVt）→ macOS 上 kimi 漏新文案（Mac 复验 §四-B
/// 实测：kimi 滞留 composer 走旧文案），故签名从 (family, os) 改 (tool, os)，
/// 投影表在 families.rs 与 known-families.md 平台差异表互链。
pub(crate) fn direct_confirm_fail_copy(tool: &str, os: &str) -> &'static str {
    if os == "macos" && super::families::macos_enter_swallowed(tool) {
        DIRECT_CONFIRM_FAIL_MACOS_SWALLOWED
    } else {
        DIRECT_CONFIRM_FAIL
    }
}

/// 戳命中查询（经 `confirm_probe` 缝——queue 测试装恒真/恒假假体，零接触真实文件）
fn probe_hits(st: &crate::remote::server::RemoteState, tool: &str, sid: &str, stamp: &str) -> bool {
    (st.confirm_probe)(tool, sid, stamp)
}

/// 屏读探针（**签名之前的**正文末尾至多 16 字符，char 边界安全）。
/// 丁T3 裁2 适配：与 [`stamp_of`] 同源的尾部判据——探针若含尾部签名，则同一设备的
/// 任意消息在输入行上都能判「滞留」（滞留判定退化成设备级），补回车的恢复动作会在
/// 消息其实已被消费时凭空多发一颗回车（那会误激活对话框的默认项）。
fn screen_probe(content: &str) -> String {
    let trimmed = super::normalize::strip_mobile_signature(content);
    let skip = trimmed.chars().count().saturating_sub(SCREEN_PROBE_CHARS);
    trimmed.chars().skip(skip).collect()
}

/// 屏读回查结果（D7/T3 分诊输入，平台执行侧产出、纯核消费）：
/// 「戳超时未中」之后屏读回查（滞留判定 → 补按回车 → 复查）观察到的四种结局，
/// 外加非 Windows 平台「无屏读能力」的降级格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScreenRecovery {
    /// 屏读**无滞留草稿**：注入的字不在输入行上 = 已被 TUI 收进内部队列
    /// （busy TUI 消费草稿的常态；屏读 Err 同归本格——无滞留证据，保守不补键）。
    /// **残差风险对称披露**：真滞留 + 屏读失败会误归本格 → 误回 Submitted 劝退
    /// 重试、消息可能滞留输入行——但方向上仍优于旧口径（Failed + 重试的不可撤
    /// 双发）；两害取其轻，判据锚定「无滞留证据不补键、不诬失败」
    NotStuck,
    /// 滞留 + 补回车成功 + 复查窗内戳命中：补键提交成功，已确认落盘
    Recovered,
    /// 滞留 + 补回车成功 + 3s 复查仍未中：真失败（防重警示）
    RecheckMissed,
    /// 滞留但补按回车失败（携带原始错误，文案拼接交纯核统一处理）
    EnterFailed(String),
    /// 平台无屏读/占用 API（macOS 等）：分诊不可达——维持「未中即失败」既有口径。
    /// 构造点仅非 Windows 执行侧（`direct_recovery` 降级臂）与 cfg(test) 分诊表
    /// 测试——Windows 非测试构建下永不构造，按 resume.rs 先例条件化 allow
    #[cfg_attr(all(windows, not(test)), allow(dead_code))]
    Unavailable,
}

/// flush 内核三态回执（D7/T3：**注入失败 / 插队 / 直发确认三路共用**，
/// `queue::try_flush_with` 按此映射 [`super::queue::FlushOutcome`]：Confirmed→Sent /
/// Submitted→Submitted / Failed→Failed）。`Submitted` 仅直发确认分诊产出；另两态
/// 的构造点与载荷随路径而异（见各变体注）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectReceipt {
    /// 确认通过（→ Sent）。**跨路径载荷差异**：直发 = 戳命中（轮询窗内，或屏读
    /// 回查补回车后 3s 窗内）= 已确认落盘；插队 = 占用排空 / best-effort（无戳
    /// 可查，屏读只作诊断不 Gate）。
    Confirmed,
    /// 注入 Ok + 戳未中 + 屏读无滞留草稿 = **已投递未确认**（中性，仅直发分诊
    /// 产出）：消息已被 TUI 收进内部队列，agent 空闲后处理——不失败、不提供重试
    /// （重试 = 双发，且 TUI 那份无法撤回；验收问题 #5 的「假失败诱导重试」根因
    /// 即此态被误判）
    Submitted,
    /// 失败（载荷随路径而异）：直发确认 = 防重警示文案 ± 补按回车失败原因（真
    /// 失败，重试由用户判断）；注入失败 = 注入器错误原文（裸注入错误，短路确认）。
    Failed(String),
}

/// D7/T3 分诊纯核：屏读回查结果 → 直发确认三态结论。三条判定与因果（防后人
/// 改回「非滞留即失败」的旧口径——那会把「已被 TUI 收进内部队列」误判成假失败，
/// 诱导用户重试造成双发，验收问题 #5）：
/// ① **非滞留**（[`ScreenRecovery::NotStuck`]）→ [`DirectReceipt::Submitted`]：
///    无滞留草稿 = 字已被 TUI 收进内部队列，中性「已投递未确认」，**不失败不重试**；
/// ② **滞留 + 补回车 + 命中**（[`ScreenRecovery::Recovered`]）→
///    [`DirectReceipt::Confirmed`]（= Sent）：补键提交成功、会话文件见戳；
/// ③ **滞留 + 补回车失败**（[`ScreenRecovery::EnterFailed`]）→
///    [`DirectReceipt::Failed`]：`{防重警示}；补按回车失败：{e}`（两段文案保持）；
/// ④ **滞留 + 补回车 + 3s 复查仍未中**（[`ScreenRecovery::RecheckMissed`]）→
///    [`DirectReceipt::Failed`]：防重警示文案（[`direct_confirm_fail_copy`]），
///    真失败、重试由用户判断；
/// ⑤ **无屏读能力**（[`ScreenRecovery::Unavailable`]，macOS）→
///    [`DirectReceipt::Failed`]：分诊不可达，行为与 T3 前一致（任务书明确不扩
///    macOS 分诊；吞回车专用文案路径保持）。
/// `tool` × `os` 参数化（同 [`direct_confirm_fail_copy`]）：任一平台可钉全表。
pub(crate) fn triage_screen_recovery(
    recovery: ScreenRecovery,
    tool: &str,
    os: &str,
) -> DirectReceipt {
    match recovery {
        ScreenRecovery::NotStuck => DirectReceipt::Submitted,
        ScreenRecovery::Recovered => DirectReceipt::Confirmed,
        ScreenRecovery::EnterFailed(e) => DirectReceipt::Failed(format!(
            "{}；补按回车失败：{e}",
            direct_confirm_fail_copy(tool, os)
        )),
        ScreenRecovery::RecheckMissed | ScreenRecovery::Unavailable => {
            DirectReceipt::Failed(direct_confirm_fail_copy(tool, os).to_string())
        }
    }
}

/// 直发确认（裁决 A1 直发语义 + D7/T3 三态分诊）：轮询会话文件戳 → 超时未中走
/// 屏读回查（恢复动作：滞留判定 → 补按回车 → 复查 3s）并按屏读结果**分诊三态**
/// （[`triage_screen_recovery`]，判定因果见其注）：非滞留 = 已投递未确认（中性
/// Submitted）；滞留补回车后命中 = 已送达；滞留补回车失败 / 3s 仍未中 = 真失败。
/// `timeout_ms` 由调用方按族规格下发（`families::FamilySpec::confirm_timeout_ms`，
/// 无族回退快消费者默认 5000——见 `families::FALLBACK_SPEC`；测试经
/// `queue::flush_one_with` 小超时覆盖，保持套件无 5s 级慢测）。失败文案按
/// 「工具 × 平台」感知（[`direct_confirm_fail_copy`]），os 在本函数取
/// `std::env::consts::OS`。
pub(crate) fn await_direct_receipt(
    st: &crate::remote::server::RemoteState,
    session: &crate::session::Session,
    content: &str,
    timeout_ms: u64,
) -> DirectReceipt {
    let stamp = stamp_of(content);
    let tool = session.agent_type.tool_id();
    let sid = session.id.as_str();
    // ① 轮询会话文件戳：首轮立即查（提交即时发生口径），不预睡
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if probe_hits(st, tool, sid, stamp) {
            return DirectReceipt::Confirmed;
        }
        if Instant::now() >= deadline {
            break;
        }
        sleep(PROBE_INTERVAL_MS);
    }
    // ② 屏读回查（恢复动作）+ D7/T3 分诊：戳超时未中**不是失败的充分证据**——
    //    屏读无滞留草稿 = 消息已被 TUI 收进内部队列（中性 Submitted，不重试）；
    //    滞留 + 补回车 + 命中 = 已送达；滞留 + 补回车失败 / 3s 仍未中 = 真失败
    //    （防重警示保留）。macOS 无屏读 API → Unavailable → Failed（分诊不可达，
    //    行为与 T3 前一致，吞回车专用文案路径保持）
    triage_screen_recovery(
        direct_recovery(st, session, content, stamp),
        tool,
        std::env::consts::OS,
    )
}

/// 屏读回查恢复动作（Windows）：滞留输入行判定 → 补按回车（跨平台缝经
/// `st.injector.locate_and_send_key`——测试假体可观测）→ 复查 3s。产出
/// [`ScreenRecovery`] 四种结局（分诊结论交 [`triage_screen_recovery`] 纯核）：
/// 无滞留 → `NotStuck`（**非滞留≠失败**——D7/T3 判据收紧的落点，此处只如实
/// 上报观察，不定结论）；补键失败 → `EnterFailed`；复查命中 → `Recovered`；
/// 复查未中 → `RecheckMissed`。
///
/// **双投后果披露（质量评审 Minor 2）**：补按回车的窗口内（屏读判定+补键数十 ms
/// 级）若用户焦点恰落在该会话的审批对话框/选择菜单上，这颗空回车会激活其默认
/// 项——概率低（要求屏读滞留判定成立且焦点恰好重叠），属既有注入面的边际扩大；
/// 焦点行为实机验证归 Task 12 清单。
#[cfg(windows)]
fn direct_recovery(
    st: &crate::remote::server::RemoteState,
    session: &crate::session::Session,
    content: &str,
    stamp: &str,
) -> ScreenRecovery {
    if !stuck_on_input_line(session.pid, content) {
        // D7/T3：屏读未见滞留草稿（含屏读 Err——无滞留证据，保守不补键）。
        // 字已被 TUI 收进内部队列属常态，非滞留≠失败；结论由纯核分诊
        // （NotStuck → Submitted 中性回执），本函数只如实上报观察
        return ScreenRecovery::NotStuck;
    }
    // 补按回车（提交滞留行）：失败即无法恢复，原始错误上抛（文案拼接在纯核）
    if let Err(e) = st.injector.locate_and_send_key(session.pid, "enter") {
        return ScreenRecovery::EnterFailed(e);
    }
    let tool = session.agent_type.tool_id();
    let sid = session.id.as_str();
    let recheck = Instant::now() + Duration::from_millis(RECHECK_MS);
    loop {
        if probe_hits(st, tool, sid, stamp) {
            return ScreenRecovery::Recovered;
        }
        if Instant::now() >= recheck {
            return ScreenRecovery::RecheckMissed;
        }
        sleep(PROBE_INTERVAL_MS);
    }
}

/// 屏读回查降级（macOS 等非 Windows）：无屏读/占用 API——不回查、分诊不可达
/// （`Unavailable` → 纯核判 Failed），维持「未中即失败」既有口径（屏读门槛语义
/// 的字面执行 + T3 任务书明确不扩 macOS 分诊；Mac 回传清单已有确认机制复验项）。
#[cfg(not(windows))]
fn direct_recovery(
    _st: &crate::remote::server::RemoteState,
    _session: &crate::session::Session,
    _content: &str,
    _stamp: &str,
) -> ScreenRecovery {
    ScreenRecovery::Unavailable
}

/// 滞留输入行判定（屏读门槛，Windows）：`read_input_tail(pid, 64)` 返回串含
/// 屏读探针 → 判「草稿在场」（启发式，性质见 [`SCREEN_PROBE_CHARS`] 注）；
/// 屏读 Err 或不含 → 门槛不成立（保守：不动作、不补键）。
#[cfg(windows)]
fn stuck_on_input_line(pid: u32, content: &str) -> bool {
    let probe = screen_probe(content);
    if probe.is_empty() {
        return false;
    }
    match super::windows_console::read_input_tail(pid, 64) {
        Ok(tail) => tail.contains(&probe),
        Err(_) => false,
    }
}

/// 插队确认（裁决 A1 插队语义）：写后等占用排空 ≤2s（Windows
/// `wait_input_drained`）——排空成功 → `Ok`（已送达；屏读草稿尾为 best-effort
/// 诊断只进日志，不 Gate 结果）；排空超时 → `Err`（[`DELIVERY_TIMEOUT_MSG`]，
/// 防重口径）；排空查询基础设施失败（假 pid / 控制台失效）→ best-effort 以
/// 「写入成功」为准返回 Ok（诊断通道不可用不得误报投递超时，错误进日志）。
/// macOS 无占用/屏读 API → 直接 Ok（保持既有行为，插队无 drain 可等）。
pub(crate) fn await_jump_receipt(
    st: &crate::remote::server::RemoteState,
    session: &crate::session::Session,
    content: &str,
) -> Result<(), String> {
    // st 缝在插队路径暂无消费（屏读/占用直走 windows_console；参数形状保留供
    // Task 6/后续诊断扩展），显式弃用防跨平台未用告警
    let _ = st;
    jump_receipt(session, content)
}

/// 插队确认平台实现分派（见 [`await_jump_receipt`] 语义注）。
#[cfg(windows)]
fn jump_receipt(session: &crate::session::Session, content: &str) -> Result<(), String> {
    match super::windows_console::wait_input_drained(session.pid, JUMP_DRAIN_TIMEOUT_MS) {
        Ok(true) => {
            // 屏读草稿尾：best-effort 诊断（裁决「屏读失败则以写入成功+排空为准」；
            // busy TUI 消费后草稿离开输入行亦属正常，故不含也只记日志）
            let probe = screen_probe(content);
            match super::windows_console::read_input_tail(session.pid, 64) {
                Ok(tail) if !probe.is_empty() && !tail.contains(&probe) => {
                    log::debug!("插队屏读未见草稿尾（best-effort 不 Gate）：tail={tail:?}");
                }
                Ok(_) => {}
                Err(e) => log::debug!("插队屏读失败（best-effort 不 Gate）：{e}"),
            }
            Ok(())
        }
        Ok(false) => Err(DELIVERY_TIMEOUT_MSG.to_string()),
        Err(e) => {
            log::debug!("插队排空查询失败（best-effort 以写入成功为准）：{e}");
            Ok(())
        }
    }
}

/// 插队确认平台实现分派（macOS 降级：无 drain 可等，直接 Sent 保持既有行为）。
#[cfg(not(windows))]
fn jump_receipt(session: &crate::session::Session, content: &str) -> Result<(), String> {
    let _ = session;
    let _ = content;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==== Step 1 失败测试（纯核契约） ====

    /// 丁T3 裁2 **安全项（任务书点名，不可省）**：签名后置后两条**同设备不同正文**
    /// 的消息，尾部签名完全相同（` [mobile iPhone]`）——若戳取 composed 的尾部，
    /// 第一条的戳会命中第二条（跨消息假命中 = 确认层谎报送达）。戳必须取**签名之前
    /// 的正文尾部**：本用例即该不变式的锁。
    #[test]
    fn stamp_never_false_hits_across_same_device_messages() {
        use crate::inject::normalize::compose_injection;
        let a = compose_injection("iPhone", "第一条：把 login.ts 的空指针修掉");
        let b = compose_injection("iPhone", "第二条：跑一遍回归测试并汇报");
        let sa = stamp_of(&a);
        let sb = stamp_of(&b);
        // 戳是正文的尾部（签名不在戳里）——两条消息的戳必须**互不相同**
        assert!(!sa.contains("[mobile"), "戳不得含签名：{sa:?}");
        assert!(!sb.contains("[mobile"), "戳不得含签名：{sb:?}");
        assert_ne!(sa, sb, "同设备两条不同正文的戳必须相异");
        // 跨消息假命中锁：A 的戳不得在 B 的正文里命中（B 已注入到会话文件的情形）
        assert!(
            !stamp_in_messages(std::slice::from_ref(&b), sa),
            "假命中：第一条的戳命中了第二条消息"
        );
        assert!(
            stamp_in_messages(std::slice::from_ref(&b), sb),
            "对照格：第二条自己的戳必须在第二条里命中"
        );
        // 回归锁：旧口径（取 composed 尾部）下两条的戳会是同一串 ` [mobile iPhone]`
        // ——本断言把「戳里不得含签名」钉死，还原旧口径即变红
        assert!(
            !stamp_of(&a).contains("mobile") && !stamp_of(&b).contains("mobile"),
            "戳必须取签名之前的正文尾部（旧口径回归锁）"
        );
        // 极短正文：戳=正文全量（签名同样不参与）
        let short = compose_injection("iPhone", "好");
        assert_eq!(stamp_of(&short), "好");
    }

    /// 丁T3：屏读探针同样取**签名之前的正文尾部**（同一类尾部判据，同一处适配）——
    /// 否则滞留判定退化为设备级（任何一条本设备消息都会判「滞留」）
    #[test]
    fn screen_probe_uses_body_tail_too() {
        use crate::inject::normalize::compose_injection;
        let composed = compose_injection("iPhone", "一条用于屏读滞留判定的正文");
        let probe = screen_probe(&composed);
        assert!(!probe.contains("[mobile"), "探针不得含签名：{probe:?}");
        assert!(composed.contains(&probe), "探针必须仍是 composed 的子串");
        assert!(probe.ends_with("正文"), "探针取正文尾部：{probe:?}");
    }

    /// 纯核：截尾 24 字符 + 先 trim_end（F8 尾空格修剪）
    #[test]
    fn stamp_logic() {
        let c = "[mobile iPhone] ".to_string() + &"a".repeat(40) + "  ";
        let s = stamp_of(&c);
        assert_eq!(s.chars().count(), 24);
        assert!(!s.ends_with(' '));
        assert_eq!(stamp_of("短消息"), "短消息");
    }

    #[test]
    fn stamp_found_in_user_messages() {
        let msgs = vec![
            "历史消息".to_string(),
            format!("reply just OK {}", stamp_of("[mobile t] body…")),
        ];
        assert!(stamp_in_messages(&msgs, stamp_of("[mobile t] body…")));
        assert!(!stamp_in_messages(&msgs, "不存在戳"));
    }

    /// 空戳恒不中（防「空串 contains 恒真」假阳性——诚实口径）
    #[test]
    fn empty_stamp_never_hits() {
        assert!(!stamp_in_messages(&["任意内容".to_string()], ""));
    }

    /// 多字节字符边界：截尾必须落在 char 边界（CJK 逐字符计数，不按字节切）
    #[test]
    fn stamp_of_multibyte_boundary() {
        let c = "一二三四五六七八九十甲乙丙丁戊己庚辛壬癸子丑寅卯".to_string(); // 24 字符
        assert_eq!(stamp_of(&c), c.as_str());
        let long = format!("{c}更长的一句中文消息内容");
        let s = stamp_of(&long);
        assert_eq!(s.chars().count(), 24);
        assert!(long.ends_with(s), "截尾必须是原串的尾段子串");
    }

    /// 消息页判定（user 侧过滤）：role 归一化口径（content.rs）下，注入戳只可能
    /// 落在 user 正文；assistant 侧（tool-result 回显等）含同文不得误判命中
    #[test]
    fn stamp_hit_filters_to_user_role() {
        let stamp = stamp_of("[mobile t] body…");
        let msg = |role: &str, kind: &str, content: &str| crate::remote::content::SessionMessage {
            seq: 0,
            role: role.to_string(),
            kind: kind.to_string(),
            content: content.to_string(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: kind != "user",
        };
        // user 正文命中
        let pg = crate::remote::content::MessagesPage {
            messages: vec![msg("user", "user", "[mobile t] body…")],
            truncated: false,
        };
        assert!(stamp_hit_in_page(&pg, stamp));
        // 同文只在 assistant 侧（tool-result 回显）→ 不命中
        let pg2 = crate::remote::content::MessagesPage {
            messages: vec![msg("assistant", "tool-result", "[mobile t] body…")],
            truncated: false,
        };
        assert!(!stamp_hit_in_page(&pg2, stamp));
        // 混合页：assistant 在前 user 在后 → 命中
        let pg3 = crate::remote::content::MessagesPage {
            messages: vec![
                msg("assistant", "assistant", "回复"),
                msg("user", "user", format!("前缀 {stamp}").as_str()),
            ],
            truncated: false,
        };
        assert!(stamp_hit_in_page(&pg3, stamp));
    }

    /// T1 计划一等消息不进 user 侧确认比对（语义锁）：kind="plan" 的消息
    /// role 归一化为 assistant，正文（计划 markdown）即使恰好包含注入戳全文，
    /// stamp_hit_in_page 也不得判命中——戳比对行为与升格前完全一致（升格只改
    /// 消息形态，不动确认层过滤语义）；user 正文命中路径不回归（对照格）
    #[test]
    fn plan_messages_excluded_from_stamp_hit() {
        let stamp = stamp_of("[mobile t] body…");
        let msg = |role: &str, kind: &str, content: &str| crate::remote::content::SessionMessage {
            seq: 0,
            role: role.to_string(),
            kind: kind.to_string(),
            content: content.to_string(),
            ts: None,
            tool_name: None,
            tool_args: None,
            collapsed: false,
        };
        // plan 正文含戳 → 不命中（role=assistant 过滤）
        let pg = crate::remote::content::MessagesPage {
            messages: vec![msg(
                "assistant",
                "plan",
                format!("# 计划\n\n执行步骤引用 {stamp}").as_str(),
            )],
            truncated: false,
        };
        assert!(!stamp_hit_in_page(&pg, stamp));
        // 对照格：同流加一条 user 正文命中 → 照常命中（user 过滤通道无回归）
        let pg2 = crate::remote::content::MessagesPage {
            messages: vec![
                msg("assistant", "plan", format!("计划引用 {stamp}").as_str()),
                msg("user", "user", stamp),
            ],
            truncated: false,
        };
        assert!(stamp_hit_in_page(&pg2, stamp));
    }

    /// 直发确认失败文案表驱动（F2，mac-reverify-b9a501c §四-B）：按「工具 × 平台」
    /// 全格钉——macOS 上 codex/kimi 走新文案（补「按一次回车」指引；kimi 在 Windows
    /// 族表是 RawVt，按族判定的旧实现漏它——本格即回归锁），claude/opencode 与
    /// 全部 Windows 格维持原文案；未知工具默认原文案。os 参数化 → 在 Windows 上
    /// 即可跨平台钉全表（生产 os 取 std::env::consts::OS）。
    #[test]
    fn direct_confirm_fail_copy_is_tool_platform_aware() {
        let new_copy =
            "已注入未确认：该类工具在 macOS 注入后可能需在终端按一次回车提交，请检查后重试";
        let old_copy = "已注入未确认（未见会话记录），请检查终端后重试";
        // (tool, os, 期望文案, 格说明)
        let cases: &[(&str, &str, &str, &str)] = &[
            (
                "codex",
                "macos",
                new_copy,
                "F2 前 (Crossterm, macos) 已覆盖",
            ),
            (
                "kimi",
                "macos",
                new_copy,
                "F2 修复格：kimi(RawVt) 在 macOS 也须新文案",
            ),
            ("claude", "macos", old_copy, "claude macOS 对照实测正常"),
            ("opencode", "macos", old_copy, "opencode 投影默认 false"),
            (
                "codex",
                "windows",
                old_copy,
                "Windows crossterm 有真回车形态",
            ),
            ("kimi", "windows", old_copy, "Windows kimi=A 族语义不变"),
            ("claude", "windows", old_copy, "Windows 快族"),
            ("opencode", "windows", old_copy, "Windows 快族"),
        ];
        for (tool, os, want, note) in cases {
            assert_eq!(
                direct_confirm_fail_copy(tool, os),
                *want,
                "({tool}, {os}) {note}"
            );
        }
        // 未实测/未知工具默认 false（宁可少提示不误报）
        assert_eq!(
            direct_confirm_fail_copy("workbuddy", "macos"),
            old_copy,
            "投影表外工具默认原文案"
        );
    }

    /// D7/T3 分诊纯核表驱动（判定因果与「已被 TUI 收进内部队列 = 中性非失败」
    /// 的防改回注记见 [`super::triage_screen_recovery`]）：屏读回查四结局 + 无屏读
    /// 降级格 → 三态结论全格钉；Failed 三格另钉 kimi × macos（工具 × 平台感知的
    /// 文案选择在分诊输出端保持 F2 语义）
    #[test]
    fn direct_triage_screen_recovery_table() {
        use super::ScreenRecovery;
        let old_copy = "已注入未确认（未见会话记录），请检查终端后重试";
        let mac_copy =
            "已注入未确认：该类工具在 macOS 注入后可能需在终端按一次回车提交，请检查后重试";
        let cases: Vec<(ScreenRecovery, &str, &str, DirectReceipt)> = vec![
            // ① 非滞留（无滞留草稿）→ Submitted 中性（非滞留≠失败——本格即验收
            //    问题 #5 假失败的翻案锁）
            (
                ScreenRecovery::NotStuck,
                "claude",
                "windows",
                DirectReceipt::Submitted,
            ),
            // ② 滞留 + 补回车 + 命中 → Confirmed（= Sent）
            (
                ScreenRecovery::Recovered,
                "claude",
                "windows",
                DirectReceipt::Confirmed,
            ),
            // ③ 滞留 + 补回车失败 → Failed（防重警示前缀 + 补按回车失败原因）
            (
                ScreenRecovery::EnterFailed("句柄失效".to_string()),
                "claude",
                "windows",
                DirectReceipt::Failed(format!("{old_copy}；补按回车失败：句柄失效")),
            ),
            // ④ 滞留 + 补回车 + 3s 仍未中 → Failed（防重警示，真失败）
            (
                ScreenRecovery::RecheckMissed,
                "claude",
                "windows",
                DirectReceipt::Failed(old_copy.to_string()),
            ),
            // ⑤ 无屏读能力（macOS 形态）→ Failed（分诊不可达，行为与 T3 前一致）
            (
                ScreenRecovery::Unavailable,
                "claude",
                "windows",
                DirectReceipt::Failed(old_copy.to_string()),
            ),
            // kimi × macos：Failed 三格走吞回车专用文案（工具 × 平台感知保持）
            (
                ScreenRecovery::RecheckMissed,
                "kimi",
                "macos",
                DirectReceipt::Failed(mac_copy.to_string()),
            ),
            (
                ScreenRecovery::Unavailable,
                "kimi",
                "macos",
                DirectReceipt::Failed(mac_copy.to_string()),
            ),
            (
                ScreenRecovery::EnterFailed("no console".to_string()),
                "kimi",
                "macos",
                DirectReceipt::Failed(format!("{mac_copy}；补按回车失败：no console")),
            ),
        ];
        for (recovery, tool, os, want) in cases {
            assert_eq!(
                &triage_screen_recovery(recovery.clone(), tool, os),
                &want,
                "分诊格 ({recovery:?}, {tool}, {os})"
            );
        }
    }
}
