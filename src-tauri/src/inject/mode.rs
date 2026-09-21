//! 模式切换内核（批次丙 T6）：统一模式枚举 + 各工具切换机制映射。
//!
//! # 要解决的问题（图1 场景）
//!
//! 无任何模式切换 UI——opencode/kimi 卡在 plan 只读无法退出；claude 计划批准后
//! 切档不可见；codex 走 `/plan`、`/permissions` 斜杠命令。用户要「远程一键退出
//! plan mode」。
//!
//! # 统一模式枚举（对齐 happy 的 8 值，收敛为 MAM 5 值）
//!
//! happy 的 `PermissionMode = 'auto'|'default'|'acceptEdits'|'bypassPermissions'|
//! 'plan'|'read-only'|'safe-yolo'|'yolo'`（调研档案 §3）。MAM 收敛为 5 值：把
//! happy 的 auto/safe-yolo/yolo 三档合并为「完全信任」（对 TUI 侧而言都是「别再
//! 问我」），其余一一对应：
//!
//! | MAM 统一档 | happy 对应 | 语义 |
//! |---|---|---|
//! | Plan | plan | 只读规划，不改文件 |
//! | Default | default | 每步询问（默认档） |
//! | AcceptEdits | acceptEdits | 自动接受文件编辑 |
//! | Bypass | bypassPermissions / auto / yolo | 完全信任，不再询问 |
//! | ReadOnly | read-only | 只读（不含规划语义） |
//!
//! # 各工具切换机制（实测/档案，勿凭想象扩面）
//!
//! - **claude / opencode / kimi：shift+tab 循环**（A 族实测共性）。opencode 状态栏
//!   明示模式（如 "Plan · DeepSeek…"）→ 可屏读确认；claude/kimi 的状态栏回显**未
//!   实测** → 只能盲切 + 人工核对提示（红线 4）。
//! - **codex：斜杠命令注入**（`/plan`、`/permissions`）——矩阵 §2.1 实测该命令
//!   存在于 0.155.1 命令清单（"switch to Plan mode"）。
//!
//! **shift+tab 的循环语义**：三家的档位顺序、以及「切几次到目标档」**未实测**。
//! 故本模块**只提供单次 shift+tab 的按键构造**（切一档），不实现「切 N 次到目标档」
//! 的自动推算——那需要知道各家档位环序，属未验面（不出手）。
//!
//! # 边界（计划书 §2.5）
//!
//! 不做各工具像素级复刻；不做自定义主题。macOS 屏读缺失 → 盲切 + 人工核对提示
//! （红线 4：不假装成功）。

/// 该工具是否支持**打断式插队**（批次丙 T9：运行中会话先 Esc 中断再投递）。
///
/// **只有 claude**：K2 实机取证（探测档案 2026-09-21-claude-askuserquestion）——
/// Esc 落入 busy 回合 = 中断该回合（"Interrupted · What should Claude do instead?"），
/// 这正是插队想要的语义。
///
/// 其他工具**未实测**（codex 的 Esc 实测同为「中断整个回合」，但其插队路径未经
/// 端到端验证；opencode/kimi 未测）→ 一律 false（结论不得超过证据；未验不出手）。
pub fn supports_interrupt(tool: &str) -> bool {
    matches!(tool, "claude")
}

/// MAM 统一模式档（5 值，对齐 happy 8 值收敛——见模块文档表）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MamMode {
    /// 只读规划（plan mode）
    Plan,
    /// 默认：每步询问
    Default,
    /// 自动接受文件编辑
    AcceptEdits,
    /// 完全信任：不再询问（happy 的 bypassPermissions/auto/yolo 收敛）
    Bypass,
    /// 只读（不含规划语义）
    ReadOnly,
}

impl MamMode {
    /// wire 字符串 ↔ 档（camelCase 请求体用小写词）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(Self::Plan),
            "default" => Some(Self::Default),
            "acceptEdits" | "accept_edits" => Some(Self::AcceptEdits),
            "bypass" => Some(Self::Bypass),
            "readOnly" | "read_only" => Some(Self::ReadOnly),
            _ => None,
        }
    }

    /// wire 字符串（响应体与审计用）
    pub fn wire(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Bypass => "bypass",
            Self::ReadOnly => "readOnly",
        }
    }

    /// 中文显示名（前端卡头与提示文案）
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "计划",
            Self::Default => "默认",
            Self::AcceptEdits => "接受编辑",
            Self::Bypass => "完全信任",
            Self::ReadOnly => "只读",
        }
    }

    /// 全部档位（前端选择器渲染序）
    pub const ALL: [MamMode; 5] = [
        MamMode::Plan,
        MamMode::Default,
        MamMode::AcceptEdits,
        MamMode::Bypass,
        MamMode::ReadOnly,
    ];
}

/// 模式切换机制（各工具一族）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSwitchKind {
    /// shift+tab 循环（claude / opencode / kimi，A 族实测共性）：一次按键切一档
    ShiftTabCycle,
    /// 斜杠命令注入（codex）：`/plan`、`/permissions` 等
    SlashCommand,
    /// 无机制（未实测/不支持的工具）→ 端点拒绝，前端只显示当前档（若可屏读）
    Unsupported,
}

/// 会话工具 → 切换机制（纯函数，可测）
pub fn switch_kind(tool: &str) -> ModeSwitchKind {
    match tool {
        // A 族三家实测共性：shift+tab 循环（opencode 状态栏可见模式文本）
        "claude" | "opencode" | "kimi" => ModeSwitchKind::ShiftTabCycle,
        // codex：矩阵 §2.1 实测 `/plan` 在 0.155.1 命令清单里（"switch to Plan mode"）
        "codex" => ModeSwitchKind::SlashCommand,
        _ => ModeSwitchKind::Unsupported,
    }
}

/// 该工具是否支持**屏读回显当前模式**（T6 红线 4 的判据）。
///
/// 只有 **opencode** 有实测的状态栏模式文本（矩阵 §2.2：「opencode 状态栏明示
/// 『Plan · DeepSeek…』，模式可屏读确认」）。claude/kimi/codex 的模式回显**未实测**
/// ——故这三家走盲切 + 「请人工核对终端模式」提示（不假装成功）。
pub fn mode_readback_supported(tool: &str) -> bool {
    matches!(tool, "opencode")
}

/// 斜杠命令注入文本（codex；非该机制 → None）。
///
/// **未验面申报**：目标档 → 具体命令的**完整**映射只有 `/plan` 有实测支撑
/// （矩阵 §2.1 命令清单）。`/permissions` 是计划书点名的第二命令（用户实测见过），
/// 但它**接受什么参数、是否切档**未经本机实测——故本函数对 `Bypass`/`Default` 返回
/// `/permissions`（可打开权限面板，用户可在其中选择），**不承诺直达某档**；
/// AcceptEdits/ReadOnly 在 codex 侧无对应命令证据 → None（不出手）。
pub fn slash_command_for(tool: &str, target: MamMode) -> Option<&'static str> {
    if switch_kind(tool) != ModeSwitchKind::SlashCommand {
        return None;
    }
    match target {
        // 实测支撑：命令清单含 /plan（"switch to Plan mode"）
        MamMode::Plan => Some("/plan"),
        // 打开权限面板（用户在其中选择档位）——不承诺直达
        MamMode::Default | MamMode::Bypass => Some("/permissions"),
        // 无命令证据 → 不出手
        MamMode::AcceptEdits | MamMode::ReadOnly => None,
    }
}

/// 模式切换的注入序列构造（纯函数）：
///
/// - `ShiftTabCycle` → `vec!["shift+tab"]`（一次按键切一档；键域见 engine 的
///   `shift_tab_records`）；
/// - `SlashCommand` → 斜杠命令文本注入 + 提交（`vec![文本]`，由调用方走文本通道
///   ——斜杠命令需回车提交，故这里返回命令文本，提交回车由调用方按既有文本注入
///   管线补）；
/// - `Unsupported` → Err（端点据此拒绝，前端提示）。
///
/// 返回 `Vec<String>` 供调用方按「按键」或「文本」分派（与 approve/question 的
/// 序列形态一致——调用方已按族规格分发）。
pub fn mode_switch_sequence(tool: &str, target: MamMode) -> Result<ModeSwitchPlan, String> {
    match switch_kind(tool) {
        ModeSwitchKind::ShiftTabCycle => Ok(ModeSwitchPlan::Key("shift+tab".to_string())),
        ModeSwitchKind::SlashCommand => slash_command_for(tool, target)
            .map(|cmd| ModeSwitchPlan::Text(cmd.to_string()))
            .ok_or_else(|| format!("{tool} 无到「{}」档的实测命令", target.label())),
        ModeSwitchKind::Unsupported => Err(format!("{tool} 的模式切换未实测，不出手")),
    }
}

/// 切换动作形态（按键 vs 文本注入）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeSwitchPlan {
    /// 单键（键域内名称，走 `inject_key_spec`）
    Key(String),
    /// 文本（斜杠命令，走文本注入 + 回车提交）
    Text(String),
}

/// 屏读文本 → 当前模式（批次丙 T6 回显解析，纯函数可测）。
///
/// **只对 [`mode_readback_supported`] 为真的工具调用**（当前仅 opencode）。
///
/// 判据来自矩阵 §2.2 实测：**opencode 状态栏明示模式文本**（实录
/// `Plan · DeepSeek…`——模式名在状态栏最前，以 `·` 分隔模型名）。本函数按
/// **关键词**匹配（大小写不敏感）：
///
/// | 屏上文本 | 判定档 |
/// |---|---|
/// | `Plan` | [`MamMode::Plan`] |
/// | `Build` / `Default` | [`MamMode::Default`] |
/// | `Accept` / `Accept Edits` | [`MamMode::AcceptEdits`] |
/// | `Bypass` / `YOLO` | [`MamMode::Bypass`] |
///
/// 返回 None = 未识别（调用方按「屏读失败」降级——盲切 + 人工核对提示，不假装成功）。
///
/// **关键词口径的保守性**：只认上表这些词，且**优先匹配更具体的**（`Accept Edits`
/// 先于 `Default`；`Plan` 与其他不冲突）。不把「任意含 mode 字样」当命中——避免
/// 把模型名/项目名里的词误判成模式。
pub fn parse_mode_from_screen(lines: &[String]) -> Option<MamMode> {
    for line in lines {
        let lower = line.to_lowercase();
        // 具体档优先（Accept Edits 含 "accept"；Bypass/YOLO 是信任档别名）
        if lower.contains("accept edits") || lower.contains("accept-edits") {
            return Some(MamMode::AcceptEdits);
        }
        if lower.contains("bypass") || lower.contains("yolo") {
            return Some(MamMode::Bypass);
        }
        if lower.contains("read-only") || lower.contains("read only") {
            return Some(MamMode::ReadOnly);
        }
        // Plan 用词边界判据（避免 "Planner"/"planning..." 误命中）：
        // 行内独立词 "plan"（前后非字母）
        if contains_word(&lower, "plan") {
            return Some(MamMode::Plan);
        }
        if contains_word(&lower, "build") || contains_word(&lower, "default") {
            return Some(MamMode::Default);
        }
    }
    None
}

/// 行内是否存在**独立词** `word`（前后非 ASCII 字母——`Planner` 不算 `plan`）
fn contains_word(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0usize;
    while let Some(i) = haystack[from..].find(word) {
        let start = from + i;
        let end = start + word.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphabetic();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_alphabetic();
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 统一枚举往返（wire ↔ 档）+ 别名词容忍
    #[test]
    fn mode_parse_roundtrip() {
        for m in MamMode::ALL {
            assert_eq!(MamMode::parse(m.wire()), Some(m), "wire 往返: {m:?}");
        }
        assert_eq!(MamMode::parse("accept_edits"), Some(MamMode::AcceptEdits));
        assert_eq!(MamMode::parse("read_only"), Some(MamMode::ReadOnly));
        assert_eq!(
            MamMode::parse("yolo"),
            None,
            "happy 的 yolo 在 MAM 收敛为 Bypass 单一档"
        );
        assert_eq!(MamMode::parse("bogus"), None);
    }

    /// 机制分族：三家 shift+tab / codex 斜杠 / 其余不支持
    #[test]
    fn switch_kind_routing() {
        for t in ["claude", "opencode", "kimi"] {
            assert_eq!(switch_kind(t), ModeSwitchKind::ShiftTabCycle, "{t}");
        }
        assert_eq!(switch_kind("codex"), ModeSwitchKind::SlashCommand);
        for t in ["zcode", "dsh", "workbuddy", "openclaw", ""] {
            assert_eq!(
                switch_kind(t),
                ModeSwitchKind::Unsupported,
                "{t} 未实测 → 不出手"
            );
        }
    }

    /// 屏读回显：只有 opencode 有实测状态栏模式文本（红线 4 的判据）
    #[test]
    fn readback_only_for_opencode() {
        assert!(mode_readback_supported("opencode"));
        for t in ["claude", "kimi", "codex", "zcode"] {
            assert!(
                !mode_readback_supported(t),
                "{t} 模式回显未实测 → 必须走盲切 + 人工核对"
            );
        }
    }

    /// 序列构造：shift+tab 单键；codex 斜杠命令；不支持/无命令 → Err
    #[test]
    fn sequence_construction() {
        assert_eq!(
            mode_switch_sequence("claude", MamMode::Plan).unwrap(),
            ModeSwitchPlan::Key("shift+tab".to_string())
        );
        assert_eq!(
            mode_switch_sequence("opencode", MamMode::Default).unwrap(),
            ModeSwitchPlan::Key("shift+tab".to_string()),
            "shift+tab 是循环切换，目标档不参与单次按键构造"
        );
        assert_eq!(
            mode_switch_sequence("codex", MamMode::Plan).unwrap(),
            ModeSwitchPlan::Text("/plan".to_string()),
            "矩阵实测：命令清单含 /plan"
        );
        assert_eq!(
            mode_switch_sequence("codex", MamMode::Bypass).unwrap(),
            ModeSwitchPlan::Text("/permissions".to_string())
        );
        // codex 无命令证据的两档 → Err（不承诺直达）
        assert!(mode_switch_sequence("codex", MamMode::AcceptEdits).is_err());
        assert!(mode_switch_sequence("codex", MamMode::ReadOnly).is_err());
        // 不支持的工具
        assert!(mode_switch_sequence("zcode", MamMode::Plan).is_err());
    }

    /// 斜杠命令映射（工具门 + 目标档门）
    #[test]
    fn slash_command_mapping() {
        assert_eq!(slash_command_for("codex", MamMode::Plan), Some("/plan"));
        assert_eq!(
            slash_command_for("codex", MamMode::Default),
            Some("/permissions")
        );
        assert_eq!(slash_command_for("codex", MamMode::AcceptEdits), None);
        // 非斜杠工具恒 None
        assert_eq!(slash_command_for("claude", MamMode::Plan), None);
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 屏读回显解析：矩阵 §2.2 实录形态（`Plan · DeepSeek…`）→ Plan 档
    #[test]
    fn parse_mode_from_real_status_bar() {
        // 实测形态（矩阵 §2.2：opencode 状态栏）
        assert_eq!(
            parse_mode_from_screen(&lines(&["Plan · DeepSeek V4.1 Flash"])),
            Some(MamMode::Plan)
        );
        // 其余档位关键词
        assert_eq!(
            parse_mode_from_screen(&lines(&["Build · claude-sonnet"])),
            Some(MamMode::Default)
        );
        assert_eq!(
            parse_mode_from_screen(&lines(&["Accept Edits · gpt"])),
            Some(MamMode::AcceptEdits)
        );
        assert_eq!(
            parse_mode_from_screen(&lines(&["Bypass Permissions"])),
            Some(MamMode::Bypass)
        );
        assert_eq!(
            parse_mode_from_screen(&lines(&["Read-only mode"])),
            Some(MamMode::ReadOnly)
        );
        // 多行：状态栏常在最底一行
        assert_eq!(
            parse_mode_from_screen(&lines(&["some output", "doing work", "Plan · model"])),
            Some(MamMode::Plan)
        );
    }

    /// 词边界保守性：`Planner` / `planning` 不误判；无模式词 → None（降级）
    #[test]
    fn parse_mode_respects_word_boundaries() {
        assert_eq!(
            parse_mode_from_screen(&lines(&["Planner agent started"])),
            None,
            "Planner 不是 Plan 档"
        );
        assert_eq!(
            parse_mode_from_screen(&lines(&["planning the implementation"])),
            None,
            "planning 不是 Plan 档"
        );
        assert_eq!(
            parse_mode_from_screen(&lines(&["no mode here", "just text"])),
            None,
            "无模式词 → None（调用方降级盲切 + 人工核对）"
        );
        assert!(parse_mode_from_screen(&[]).is_none());
        // 具体档优先：同时含 accept edits 与 plan 时取更具体的
        assert_eq!(
            parse_mode_from_screen(&lines(&["Accept Edits (was Plan)"])),
            Some(MamMode::AcceptEdits)
        );
    }
}
