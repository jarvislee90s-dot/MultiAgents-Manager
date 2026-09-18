//! 注入族规格层（M9R Task 1，纯核）：**脆弱常量集中落点（宪法横切 6）**——分块/间隔/
//! 预算/阈值/版本指纹全部在此，大版本升级复验走项目技能 win-console-inject-probe 快路径。
//!
//! 族表为 M6R 探测定案（2026-09-18/19，证据 `research/refs/phase2-消息注入/`）：
//! 四家 CLI 按 TUI 家族分 A/B 两族——
//! - **A 族 [`TuiFamily::RawVt`]**（ReadConsoleInput 原生 VT 流）：claude / kimi /
//!   opencode，方向键走 vk=0 字符流（单批原子写）；
//! - **B 族 [`TuiFamily::Crossterm`]**（crossterm event 体系）：codex，键须带
//!   VK+scan，vk=0 控制字符会被丢弃。
//!
//! `verified_with` 是版本指纹：仅对该版本验证过注入规格；CLI 大版本升级后必须
//! 先跑 win-console-inject-probe 复验，再改此表（不得凭直觉改数值）。
//!
//! 本模块零平台 cfg：族表/背压判定/预算/分块计划全部是纯函数，Windows 上全绿可测。

/// TUI 家族（A/B 两族，M6R 定案）：决定事件构造形态（VT 字符流 vs VK+scan）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiFamily {
    /// A 族：原生 VT 流（ReadConsoleInput 消费 vk=0 的 Unicode 字符事件）。
    RawVt,
    /// B 族：crossterm 事件体系（须 VK+scan 键形态，vk=0 会被丢）。
    Crossterm,
}

/// 单工具的注入族规格（M6R 探测定案表的一行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FamilySpec {
    /// 所属 TUI 家族（A/B 族按家分支的依据）。
    pub family: TuiFamily,
    /// 版本指纹：注入规格实测验证时的 CLI 版本（大版本升级须复验）。
    pub verified_with: &'static str,
    /// 慢消费者（opencode ~70 事件/秒）：任意长度都走背压节流（R2-1）。
    pub slow_consumer: bool,
    /// A1 直发确认超时（毫秒，Task 5 消费）。
    pub confirm_timeout_ms: u64,
}

/// 每块字符数（80 字符 = 160 个 keydown+keyup 事件，单次写入单元）。
pub const CHUNK_CHARS: usize = 80;
/// 块间隔毫秒（给目标 TUI 留输入缓冲消费窗口）。
pub const CHUNK_GAP_MS: u64 = 50;
/// 正文写完后到提交回车的延迟毫秒（回车独立块不进 [`chunk_plan`]——执行层固定
/// 此延迟后单批发回车）。
pub const SUBMIT_DELAY_MS: u64 = 150;
/// 长文阈值（字符数）：快消费者超过才切背压（恰好 2000 不背压）。
pub const LONG_MSG_CHARS: usize = 2000;
/// 背压模式占用回落阈值（事件数）：写停到占用 ≤ 此值再继续。
pub const DRAIN_TO: u32 = 40;
/// 占用 >0 持续此时长（毫秒）判异常（死锁/挂起，放弃等待）。
pub const OCC_ABNORMAL_MS: u64 = 5_000;
/// 基础注入总预算（毫秒）：非背压路径的硬上限。
pub const BASE_BUDGET_MS: u64 = 10_000;
/// 背压斜率：每字符放宽毫秒数（opencode 实测 10k 字符 ≈110–183s，45ms/字符
/// 放宽到 460s 留足余量）。
pub const BACKPRESSURE_MS_PER_CHAR: u64 = 45;

/// 工具 → 族规格（小写精确匹配，对齐 `AgentType` serde lowercase 形态）。
/// M6R 探测定案表；其余工具（workbuddy/dsh/zcode/openclaw 等黑盒/无头家）
/// 返回 None——路由层已拦，此处纵深防御。
pub fn family_for(tool: &str) -> Option<FamilySpec> {
    let (family, verified_with, slow_consumer) = match tool {
        "claude" => (TuiFamily::RawVt, "2.1.251", false),
        "kimi" => (TuiFamily::RawVt, "2.0.0", false),
        "opencode" => (TuiFamily::RawVt, "1.18.31", true),
        "codex" => (TuiFamily::Crossterm, "0.154.0", false),
        _ => return None,
    };
    Some(FamilySpec {
        family,
        verified_with,
        slow_consumer,
        // 自裁值（测试未钉）：直发确认秒级命中口径下留双倍余量；慢消费者含
        // SQLite 副本查询开销，再加倍。
        confirm_timeout_ms: if slow_consumer { 10_000 } else { 5_000 },
    })
}

/// 背压判定（R2-1）：慢消费者或长文切背压——慢消费者任意长度都节流；快消费者
/// 仅超过 [`LONG_MSG_CHARS`] 才节流（恰好 2000 不背压）。
pub fn use_backpressure(spec: &FamilySpec, chars: usize) -> bool {
    spec.slow_consumer || chars > LONG_MSG_CHARS
}

/// 真总预算（毫秒）：背压路径按 [`BACKPRESSURE_MS_PER_CHAR`] 斜率放宽
/// （[`BASE_BUDGET_MS`] + 每字符 45ms），非背压路径固定 [`BASE_BUDGET_MS`]。
pub fn inject_budget_ms(spec: &FamilySpec, chars: usize) -> u64 {
    if use_backpressure(spec, chars) {
        BASE_BUDGET_MS + chars as u64 * BACKPRESSURE_MS_PER_CHAR
    } else {
        BASE_BUDGET_MS
    }
}

/// 分块/背压计划（纯构造，Task 3 执行层消费）：正文按 [`CHUNK_CHARS`] 向上取整
/// 的分块数 + 是否走背压。回车块不在此计划内（执行层固定 [`SUBMIT_DELAY_MS`]
/// 后单批发回车）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkPlan {
    /// 正文分块数（按 chars 计数；空文本 → 0 块）。
    pub text_chunks: usize,
    /// 是否切背压节流。
    pub backpressure: bool,
}

/// 构造 [`ChunkPlan`]：`text_chunks` = 正文按 [`CHUNK_CHARS`] 向上取整（按 chars
/// 计数）；`backpressure` = [`use_backpressure`]。
pub fn chunk_plan(text: &str, spec: &FamilySpec) -> ChunkPlan {
    let chars = text.chars().count();
    ChunkPlan {
        text_chunks: chars.div_ceil(CHUNK_CHARS),
        backpressure: use_backpressure(spec, chars),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inject::families;

    #[test]
    fn family_table_matches_probe() {
        let c = families::family_for("claude").unwrap();
        assert_eq!(c.family, TuiFamily::RawVt);
        assert!(!c.slow_consumer);
        assert_eq!(c.verified_with, "2.1.251");
        let o = families::family_for("opencode").unwrap();
        assert!(o.slow_consumer);
        assert_eq!(o.verified_with, "1.18.31");
        let x = families::family_for("codex").unwrap();
        assert_eq!(x.family, TuiFamily::Crossterm);
        assert_eq!(x.verified_with, "0.154.0");
        assert!(families::family_for("kimi").is_some());
        assert!(families::family_for("workbuddy").is_none()); // 路由层已拦，此处纵深防御
    }

    #[test]
    fn backpressure_rules() {
        // R2-1：慢消费者或长文切背压
        let fast = family_for("claude").unwrap();
        let slow = family_for("opencode").unwrap();
        assert!(!families::use_backpressure(&fast, 80));
        assert!(families::use_backpressure(&fast, 2001));
        assert!(families::use_backpressure(&slow, 39));
        assert!(!families::use_backpressure(&fast, 2000));
    }

    #[test]
    fn budget_scales_for_backpressure() {
        // 真总预算：背压按斜率放宽（opencode 实测 10k≈110-183s）
        let slow = family_for("opencode").unwrap();
        assert_eq!(families::inject_budget_ms(&slow, 80), 10_000 + 80 * 45);
        assert!(families::inject_budget_ms(&slow, 10_000) >= 183_000);
        assert_eq!(
            families::inject_budget_ms(&family_for("claude").unwrap(), 500),
            10_000
        );
    }

    #[test]
    fn chunk_plan_splits_text_and_enter() {
        // 正文块 160 事件 + 回车独立块（150ms 后单批）
        let spec = families::family_for("claude").unwrap();
        let plan = chunk_plan("[mobile test] hello", &spec); // 纯函数：分块/背压计划
        assert_eq!(plan.text_chunks, 1);
        assert!(!plan.backpressure);
        let spec_op = families::family_for("opencode").unwrap();
        assert!(chunk_plan(&"x".repeat(3000), &spec_op).backpressure);
    }
}
