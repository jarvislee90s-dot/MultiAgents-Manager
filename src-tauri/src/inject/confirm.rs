//! A1 写入确认层（M9R Task 5，裁决 A1 分层写入确认）：**注入成功 ≠ 已送达**。
//!
//! - **直发**（可输入态）：以**会话文件命中**定「已送达」（提交即时发生、秒级
//!   命中）——轮询 `confirm_probe` 缝（500ms 步距、首轮立即查）查 24 字符尾戳；
//!   超时未中走**屏读回查**（全程为恢复动作）：滞留输入行判定 → 补按回车 →
//!   复查 3s；仍未中才回失败回执（重试由用户判断；E10 尾字符丢失 1/91 即本层
//!   必要性实证）。
//! - **插队**（busy 态）：以**占用排空**确认（Windows `wait_input_drained` ≤2s）；
//!   屏读草稿尾为 best-effort 诊断（busy TUI 可能在屏读前已把草稿消费进自身
//!   缓冲，不 Gate 结果）；排空超时 = 投递超时。
//!
//! ## 结构（纯核 / 执行侧分离）
//! - **纯核（零 cfg，跨平台可测）**：[`stamp_of`]（尾戳）/ [`stamp_in_messages`]
//!   （列表含戳）/ [`stamp_hit_in_page`]（user 侧过滤 + 含戳）/
//!   [`direct_confirm_fail_copy`]（族 × 平台感知失败文案，Mac 报告 §四-C）；
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

/// 尾戳（跨任务接口契约）：**先 `trim_end` 再截尾 24 字符**（F8 尾空格修剪——
/// 终端输入行尾部空格不可见且易被 TUI/会话文件丢弃，戳含尾空格会系统性失配）；
/// 短于 24 字符取全串。char 边界安全截取（多字节字符不可按字节切）。
pub fn stamp_of(content: &str) -> &str {
    let trimmed = content.trim_end();
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
/// kind == "user" → role == "user"；thinking / tool-call / tool-result 一律归
/// "assistant"（agent 侧工作产物）。故 `role == "user"` 恰好等价于「用户正文
/// 消息」，不含工具结果回显（工具回显可能恰好引用注入原文，过滤防误判命中）。
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

/// 屏读探针（trim 后正文末尾至多 16 字符，char 边界安全）
fn screen_probe(content: &str) -> String {
    let trimmed = content.trim_end();
    let skip = trimmed.chars().count().saturating_sub(SCREEN_PROBE_CHARS);
    trimmed.chars().skip(skip).collect()
}

/// 直发确认（裁决 A1 直发语义）：轮询会话文件戳 → 超时未中走屏读回查（恢复动作：
/// 滞留输入行判定 → 补按回车 → 复查 3s）→ 仍未中 = 确认失败（失败回执）。
/// `timeout_ms` 由调用方按族规格下发（`families::FamilySpec::confirm_timeout_ms`，
/// 无族回退快消费者默认 5000——见 `families::FALLBACK_SPEC`；测试经
/// `queue::flush_one_with` 小超时覆盖，保持套件无 5s 级慢测）。确认失败文案按
/// 「工具 × 平台」感知（[`direct_confirm_fail_copy`]，F2：macOS 回车吞没投影表
/// families::macos_enter_swallowed——kimi 在 Windows 是 A 族，按族判定会漏），
/// os 在本函数取 `std::env::consts::OS`。
pub(crate) fn await_direct_receipt(
    st: &crate::remote::server::RemoteState,
    session: &crate::session::Session,
    content: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    let stamp = stamp_of(content);
    let tool = session.agent_type.tool_id();
    let sid = session.id.as_str();
    // ① 轮询会话文件戳：首轮立即查（提交即时发生口径），不预睡
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if probe_hits(st, tool, sid, stamp) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        sleep(PROBE_INTERVAL_MS);
    }
    // ② 屏读回查（恢复动作）：Windows 滞留判定 → 补按回车 → 复查；
    //    macOS 无屏读 API → Ok(false)（直发未中直接 Failed——屏读门槛语义的
    //    字面执行，报告已申报；Mac 回传清单已有确认机制复验项）
    let fail_copy = direct_confirm_fail_copy(tool, std::env::consts::OS);
    match direct_recovery(st, session, content, stamp) {
        Ok(true) => Ok(()),
        Ok(false) => Err(fail_copy.to_string()),
        Err(e) => Err(format!("{fail_copy}；{e}")),
    }
}

/// 屏读回查恢复动作（Windows）：滞留输入行判定 → 补按回车（跨平台缝经
/// `st.injector.locate_and_send_key`——测试假体可观测）→ 复查 3s。
/// 返回 `Ok(true)` = 复查命中（已送达）；`Ok(false)` = 门槛不成立或复查仍未中；
/// `Err` = 补按回车失败（原因上抛，由调用方拼接进失败回执）。
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
) -> Result<bool, String> {
    if !stuck_on_input_line(session.pid, content) {
        return Ok(false);
    }
    // 补按回车（提交滞留行）：失败即无法恢复，原因拼接进最终回执
    st.injector
        .locate_and_send_key(session.pid, "enter")
        .map_err(|e| format!("补按回车失败：{e}"))?;
    let tool = session.agent_type.tool_id();
    let sid = session.id.as_str();
    let recheck = Instant::now() + Duration::from_millis(RECHECK_MS);
    loop {
        if probe_hits(st, tool, sid, stamp) {
            return Ok(true);
        }
        if Instant::now() >= recheck {
            return Ok(false);
        }
        sleep(PROBE_INTERVAL_MS);
    }
}

/// 屏读回查降级（macOS 等非 Windows）：无屏读/占用 API——不回查，直发未中直接
/// Failed（屏读门槛语义的字面执行，报告已申报；Mac 回传清单已有确认机制复验项）。
#[cfg(not(windows))]
fn direct_recovery(
    _st: &crate::remote::server::RemoteState,
    _session: &crate::session::Session,
    _content: &str,
    _stamp: &str,
) -> Result<bool, String> {
    Ok(false)
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
}
