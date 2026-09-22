//! 模式切换内核（批次丙 T6 起；批次丁 T4 扩为「二维结构 + 回读全开 + 两段式注入」）。
//!
//! # 要解决的问题
//!
//! 图1（无法退出 plan mode）+ 问题 9（**切换后不知道切到了哪**）。裁5 原文：
//! 「切换后不知道是什么模式 = 白做」——二维工具（codex/kimi）必须出**模式组 + 权限组**
//! 两组按钮，单轴工具（claude/opencode）出**切换钮 + 当前档回显**；**回读失败显示
//! 「请人工核对」，不假装成功**（红线 4 的同一口径）。
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
//! # 二维结构（裁5 / §2.6 规格表逐格落地）
//!
//! codex 与 kimi 的「模式」与「权限」是**两个正交轴**（模式 = 计划/默认；权限 =
//! 要不要问你），故各自出两组按钮；claude 与 opencode 只有一个轴，出切换钮 + 回显。
//! 每组档位带**屏显标签**——**标签用工具自己的词**（§2.6「档位（屏显标签）」列），
//! 不是 MAM 通用名：kimi 权限组是「总是询问/按需询问/永不询问」，不能显示成 MAM 的
//! 「默认/接受编辑/完全信任」（用户看到的是终端上的词，对不上号等于没回显）。
//!
//! ## kimi 权限三档 → MAM 5 值的映射（**判断依据，逐条写清**）
//!
//! kimi 官方三档原文（`kimi.exe` 内嵌 i18n 与 `/permission` 选择器，2.0.2 实测抽取）：
//!
//! | kimi 档 | 官方原文 | 语义 | 映射到 MAM | 依据 |
//! |---|---|---|---|---|
//! | manual | `Always Ask` — `Auto-read only; everything else needs your approval first.` | 只自动读，其余逐一向你确认 | [`MamMode::Default`] | 与 MAM「默认：每步询问」同义（happy 的 `default` 同格） |
//! | yolo | `Ask When Needed` — `Routine edits and commands run automatically; risky actions, questions, and plans still ask.` | 常规改动/命令自动跑，高危与提问仍问 | [`MamMode::AcceptEdits`] | MAM 5 值里**唯一**的「常规改动自动放行、高危仍问」档就是 AcceptEdits（happy 把 yolo/safe-yolo 也并进 Bypass，但那是 happy 的收敛口径；本仓 AcceptEdits 的语义「自动接受文件编辑」正是 kimi yolo 描述的前半句，且**不会**把「仍会问你」说成「完全信任」——映射到 Bypass 会说假话） |
//! | auto | `Never Ask` — `Never interrupts you; everything runs and is decided automatically.` | 完全不打断 | [`MamMode::Bypass`] | 与 MAM「完全信任：不再询问」逐字同义 |
//!
//! **不加枚举值的理由**：加第 6 值要动 `MamMode`（wire 词表、前端 `MAM_MODE_LABELS`、
//! happy 收敛表、GET/POST 契约）——这是一处**结构性改动**，而 kimi 的三档在 5 值里
//! 有语义等价的落点（见上表）。映射只影响**按钮背后的 wire 词**，**屏显标签仍用
//! kimi 的词**（§2.6），故用户看到的永远是终端上的说法，不存在术语失真。
//!
//! # 回读全开（批次丙台账「开放项 3」的收口）
//!
//! 批次丙 T6 只对 opencode 放开回读（其余三家「盲切 + 人工核对」）。T6 实机探测
//! （`research/refs/phase2-消息注入/2026-09-21-四家模式切换shift-tab实机探测.md`）
//! 证实**四家底栏都有可解析的档位文本**，故 T4 全部放开。**分族解析**（本模块的
//! 判断，理由见 [`parse_mode_from_screen`] 文档）：四家词表互斥性存疑（claude 屏上
//! 出现 `plan` 字样、codex 的项目路径可能含 `plan`），共用一个关键词匹配器必然互相
//! 误判；分族后每家的判据都能贴着**该家真机屏幕原文**写死。
//!
//! **缺席推断**（§2.6 表末「Default 靠缺席推断的口径处理」）：
//! - codex：底栏**只**在 Plan 档写 `Plan mode`，默认档没有任何模式字样 → 认出底栏
//!   且无 `plan mode` 即 Default；
//! - kimi：底栏只在 Plan 档有 `plan` 前缀 → 认出底栏且无前缀即 Default。
//!
//! 认不出底栏（屏读失败 / 形态漂移）→ `None` = **档未知** → 前端「请人工核对」，
//! 绝不把「没读到」说成 Default（红线 4）。
//!
//! # 两段式注入（§2.6 codex/kimi 权限组）
//!
//! `命令 → 菜单 → 屏读定位 → 导航确认`：第一段投递 `/permissions`（codex）或
//! `/permission`（kimi）打开菜单，第二段屏读菜单、定位目标档、按**当前高亮位**算
//! 循环步进后导航确认。第二段与丁T3 对话框守卫的关系是本任务最需要想清楚的冲突——
//! 详见 [`crate::remote::api::session_mode_switch`] 的文档（那里是两段式唯一实现点）。
//!
//! # 边界（计划书 §2.5）
//!
//! 不做各工具像素级复刻；不做自定义主题。未实测的机制一律**不出手**（结论不得
//! 超过证据）。
//!
//! # 脆弱常量
//!
//! 菜单轮询/回读等待等注入节奏常量集中在 [`super::families`]（宪法横切 6：脆弱常量
//! 集中落点），本模块只放**纯判定与纯构造**。

use crate::inject::dialog::DialogOption;

/// 该工具是否支持**打断式插队**（批次丙 T9：运行中会话先 Esc 中断再投递）。
///
/// **只有 claude**：K2 实机取证（探测档案 2026-09-21-claude-askuserquestion）——
/// Esc 落入 busy 回合 = 中断该回合（"Interrupted · What should Claude do instead?"），
/// 这正是插队想要的语义。
///
/// 其他工具**未实测**（codex 的 Esc 实测同为「中断整个回合」，但其插队路径未经
/// 端到端验证；opencode/kimi 未测）→ 一律 false（结论不得超过证据；未验不出手）。
///
/// **不要与 [`mode_readback_supported`] 混**（两条不同的轴）：回读是「能不能看见
/// 当前档」，插队是「能不能打断运行中的回合」。
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
    ///
    /// **退役档永不可解析**（裁7）：codex 的 `untrusted`（0.149.0 起配置即拒启）与
    /// `on-failure`（deprecated）在这里**没有 wire 词**——任何客户端把这两个词发进
    /// POST 都会落到 `None` → 400。这是「UI 与代码不得作为可选档」在**输入面**的锁：
    /// 不是靠前端不渲染，而是后端根本不认（见 `mode_legacy_enum_is_not_selectable`）。
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

    /// 中文显示名（**通用**档名——二维组的按钮标签用工具自己的词，见 [`ModeTier::label`]）
    pub fn label(self) -> &'static str {
        match self {
            Self::Plan => "计划",
            Self::Default => "默认",
            Self::AcceptEdits => "接受编辑",
            Self::Bypass => "完全信任",
            Self::ReadOnly => "只读",
        }
    }

    /// 全部档位（无顺序语义的集合；**环序**见 [`cycle_next`]）
    pub const ALL: [MamMode; 5] = [
        MamMode::Plan,
        MamMode::Default,
        MamMode::AcceptEdits,
        MamMode::Bypass,
        MamMode::ReadOnly,
    ];
}

/// 模式组标识（裁5 的二维结构：codex/kimi 两组正交；claude/opencode 单轴一组）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeGroupId {
    /// 模式组（计划 / 默认）——所有家都有
    Mode,
    /// 权限组（要不要问你）——只有 codex/kimi 有（claude 的接受编辑/信任档单轴内）
    Permission,
}

impl ModeGroupId {
    /// wire 词（GET 载荷与 POST 请求体）
    pub fn wire(self) -> &'static str {
        match self {
            Self::Mode => "mode",
            Self::Permission => "permission",
        }
    }

    /// wire 词 → 组（未知词 → None，调用方 400）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "mode" => Some(Self::Mode),
            "permission" => Some(Self::Permission),
            _ => None,
        }
    }

    /// 组名（前端分组标题）
    pub fn label(self) -> &'static str {
        match self {
            Self::Mode => "模式",
            Self::Permission => "权限",
        }
    }
}

/// 单档：MAM 档 + **屏显标签**（工具自身词表）+ 可选性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeTier {
    /// MAM 统一档（wire 词见 [`MamMode::wire`]）
    pub mode: MamMode,
    /// 屏显标签（§2.6「档位（屏显标签）」列**逐字**；不等于 [`MamMode::label`] 时
    /// 以本字段为准——kimi 权限组即此例）
    pub label: &'static str,
    /// 是否可作为切换目标（false = 前端不渲染可点按钮，POST 也会被拒）
    pub selectable: bool,
    /// 不可选原因（`selectable=false` 时必填——**如实回执**，不静默禁用）
    pub reason: Option<&'static str>,
}

/// 官方已退役/弃用的旧档位（裁7）。
///
/// **刻意不放进 [`ModeGroupSpec::tiers`]**：tiers 是「可作为切换目标」的清单，而
/// legacy 档**不得作为可选档**——放进去就必须给它们一个 [`MamMode`] 才能表达，
/// 而 MAM 5 值里并没有对应值（发明一个值正是裁7 禁止的「作为可选档」）。故以独立
/// 清单承载，供 GET 载荷**如实展示**「这两个档官方已退役」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTier {
    /// 屏上/官方文档里的旧档名原文
    pub label: &'static str,
    /// 退役依据（一句话，给用户看）
    pub note: &'static str,
}

/// 单个组（模式组或权限组）的完整描述（纯静态表，可测）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeGroupSpec {
    /// 组标识
    pub id: ModeGroupId,
    /// 组名（前端分组标题）
    pub label: &'static str,
    /// 是否**步进**组（一步一档，目标档不参与按键构造——claude/opencode 的
    /// shift+tab 循环）。false = 逐档按钮（有直达机制）
    pub step: bool,
    /// 该组是否有屏读回读源（false → 前端只显示「请人工核对」，不假装知道当前档）
    pub readback: bool,
    /// 档位（**顺序即环序**——单轴组用 [`cycle_next`] 按此推进；二维组顺序即
    /// §2.6 表的档位列序）
    pub tiers: &'static [ModeTier],
    /// 已退役旧档（**不可选**，只作如实展示；见 [`LegacyTier`]）
    pub legacy: &'static [LegacyTier],
}

/// 工具的模式栏结构（裁5：二维 / 单轴 / 未实测）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeStructure {
    /// 两组正交（codex / kimi）
    TwoAxis {
        /// 模式组
        mode: ModeGroupSpec,
        /// 权限组
        permission: ModeGroupSpec,
    },
    /// 单轴（claude 四档环序 / opencode 两档）
    SingleAxis {
        /// 唯一轴（组标识恒为 [`ModeGroupId::Mode`]）
        axis: ModeGroupSpec,
    },
    /// 未实测工具（无结构 → 前端不渲染切换入口）
    Unsupported,
}

impl ModeStructure {
    /// 全部组（二维两家；单轴一组；未实测空）
    pub fn groups(&self) -> Vec<ModeGroupSpec> {
        match self {
            Self::TwoAxis { mode, permission } => vec![*mode, *permission],
            Self::SingleAxis { axis } => vec![*axis],
            Self::Unsupported => Vec::new(),
        }
    }

    /// 结构 wire 词（GET 载荷 `structure` 字段；前端据此选渲染分支）
    pub fn wire(&self) -> &'static str {
        match self {
            Self::TwoAxis { .. } => "twoAxis",
            Self::SingleAxis { .. } => "singleAxis",
            Self::Unsupported => "none",
        }
    }

    /// 取指定组（结构里没有该组 → None：POST 的 `group` 校验点）
    pub fn group(&self, id: ModeGroupId) -> Option<ModeGroupSpec> {
        self.groups().into_iter().find(|g| g.id == id)
    }
}

// ===== 档位表（§2.6 逐格；屏显标签 = 该格「档位（屏显标签）」列原文）=====

/// codex 模式组：默认 / 计划。
///
/// **默认档不可选**：§2.6 只给了 `/plan`（进 Plan）——**退出** Plan 在本仓没有任何
/// 实测命令（实测可用的另一条路是 `Implement this plan?` 对话框选项 1
/// `Switch to Default and start coding`，那属审批流，不是模式组的下行通路）。
/// 用户实测底栏写着 `shift+tab to cycle`，但 §2.6 是刚性表格（模式组机制点名 `/plan`），
/// 故**不把 shift+tab 加为等价入口**——改为把「默认」标为不可选 + 写明原因（如实回执，
/// 而不是点下去没反应）。
const CODEX_MODE_TIERS: [ModeTier; 2] = [
    ModeTier {
        mode: MamMode::Default,
        label: "默认",
        selectable: false,
        reason: Some(
            "codex 退出计划模式无实测命令（可在终端按 shift+tab，或在计划批准框选第一项）",
        ),
    },
    ModeTier {
        mode: MamMode::Plan,
        label: "计划",
        selectable: true,
        reason: None,
    },
];

/// codex 权限组：只读 / 默认 / 完全信任（§2.6）。
///
/// 屏显标签对应实测菜单项（`/permissions` 弹窗 `Update Model Permissions`，M9R
/// 实机档位序 Read Only / Ask for approval / Approve for me / Full Access）：
/// 只读=`Read Only`、默认=`Ask for approval`、完全信任=`Full Access`。
/// **`Approve for me` 不在 §2.6 的三档里**（Guardian 开启时才出现），故不作目标档。
const CODEX_PERMISSION_TIERS: [ModeTier; 3] = [
    ModeTier {
        mode: MamMode::ReadOnly,
        label: "只读",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Default,
        label: "默认",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Bypass,
        label: "完全信任",
        selectable: true,
        reason: None,
    },
];

/// codex 退役旧档（裁7：官方已退役/deprecated → UI 与代码不得作为可选档）
const CODEX_PERMISSION_LEGACY: [LegacyTier; 2] = [
    LegacyTier {
        label: "untrusted",
        note: "官方 0.149.0 起退役（配置即拒启）——不作为可选档",
    },
    LegacyTier {
        label: "on-failure",
        note: "官方已弃用（deprecated）——不作为可选档",
    },
];

/// kimi 模式组：默认 / 计划（`/plan off|on` 带参直达）
const KIMI_MODE_TIERS: [ModeTier; 2] = [
    ModeTier {
        mode: MamMode::Default,
        label: "默认",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Plan,
        label: "计划",
        selectable: true,
        reason: None,
    },
];

/// kimi 权限组：总是询问 / 按需询问 / 永不询问（§2.6 屏显标签；映射依据见模块文档表）
const KIMI_PERMISSION_TIERS: [ModeTier; 3] = [
    ModeTier {
        mode: MamMode::Default,
        label: "总是询问",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::AcceptEdits,
        label: "按需询问",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Bypass,
        label: "永不询问",
        selectable: true,
        reason: None,
    },
];

/// claude 单轴四档——**顺序 = 实测环序**（逐次 shift+tab 屏读：
/// `⏵⏵ accept edits on` → `⏸ plan mode on` → `⏵⏵ auto mode on` → `⏸ manual mode on`，
/// 探测档案 §1.2 + 复盘 `screen-cl1-cycA-st{1..4}.txt` 同序）。注意与
/// [`MamMode::ALL`] 的枚举序**不同**——本数组的顺序有环序语义，不可按 ALL 排列。
const CLAUDE_AXIS_TIERS: [ModeTier; 4] = [
    ModeTier {
        mode: MamMode::AcceptEdits,
        label: "接受编辑",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Plan,
        label: "计划",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Bypass,
        label: "完全信任",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Default,
        label: "默认",
        selectable: true,
        reason: None,
    },
];

/// opencode 单轴两档：默认（原 `Build`，裁6 术语对齐）/ 计划
const OPENCODE_AXIS_TIERS: [ModeTier; 2] = [
    ModeTier {
        mode: MamMode::Default,
        label: "默认",
        selectable: true,
        reason: None,
    },
    ModeTier {
        mode: MamMode::Plan,
        label: "计划",
        selectable: true,
        reason: None,
    },
];

/// 工具 → 模式栏结构（纯函数，可测）。
///
/// **只对四家出手**（§2.6 表的四行）；其余工具 [`ModeStructure::Unsupported`]
/// ——前端不渲染任何切换入口（未验不出手）。
pub fn mode_structure(tool: &str) -> ModeStructure {
    match tool {
        "codex" => ModeStructure::TwoAxis {
            mode: ModeGroupSpec {
                id: ModeGroupId::Mode,
                label: ModeGroupId::Mode.label(),
                step: false,
                // 底栏 `Plan mode (shift+tab to cycle)`；缺席即 Default（§2.6 表末口径）
                readback: true,
                tiers: &CODEX_MODE_TIERS,
                legacy: &[],
            },
            permission: ModeGroupSpec {
                id: ModeGroupId::Permission,
                label: ModeGroupId::Permission.label(),
                step: false,
                // 权限档**没有**底栏回读源（实测底栏只有模式文本）→ 前端「请人工核对」
                readback: false,
                tiers: &CODEX_PERMISSION_TIERS,
                legacy: &CODEX_PERMISSION_LEGACY,
            },
        },
        "kimi" => ModeStructure::TwoAxis {
            mode: ModeGroupSpec {
                id: ModeGroupId::Mode,
                label: ModeGroupId::Mode.label(),
                step: false,
                // 底栏 `plan` 前缀；缺席即 Default（§2.6 表末口径）
                readback: true,
                tiers: &KIMI_MODE_TIERS,
                legacy: &[],
            },
            permission: ModeGroupSpec {
                id: ModeGroupId::Permission,
                label: ModeGroupId::Permission.label(),
                step: false,
                // kimi 底栏**不含**权限档文本（实测：`plan  <模型> thinking: high  <cwd>`）
                readback: false,
                tiers: &KIMI_PERMISSION_TIERS,
                legacy: &[],
            },
        },
        "claude" => ModeStructure::SingleAxis {
            axis: ModeGroupSpec {
                id: ModeGroupId::Mode,
                label: ModeGroupId::Mode.label(),
                // shift+tab 一步一档：目标档不参与按键构造（§2.6「切换钮=shift+tab 一步+回读确认」）
                step: true,
                readback: true,
                tiers: &CLAUDE_AXIS_TIERS,
                legacy: &[],
            },
        },
        "opencode" => ModeStructure::SingleAxis {
            axis: ModeGroupSpec {
                id: ModeGroupId::Mode,
                label: ModeGroupId::Mode.label(),
                step: true,
                readback: true,
                tiers: &OPENCODE_AXIS_TIERS,
                legacy: &[],
            },
        },
        _ => ModeStructure::Unsupported,
    }
}

/// 单轴组的**环序推进**（当前档 → 下一步应到档）。当前档不在环内 / 非单轴组 → None。
///
/// claude 环序 `[AcceptEdits, Plan, Bypass, Default]` 是**实测**（探测档案 §1.2）；
/// **已知未验面**：claude 还有 `bypassPermissions` 档（官方 `--permission-mode` 旗标
/// 列有），某些配置下环序可能多一档——那样本函数的预测会与实测不符，端点侧按
/// `Mismatch` 如实回执「回读与预期不符」而不是假装成功（见 [`verify_mode_switch`]）。
pub fn cycle_next(tool: &str, current: MamMode) -> Option<MamMode> {
    let ModeStructure::SingleAxis { axis } = mode_structure(tool) else {
        return None;
    };
    let idx = axis.tiers.iter().position(|t| t.mode == current)?;
    let next = axis.tiers[(idx + 1) % axis.tiers.len()];
    Some(next.mode)
}

/// 模式切换机制（各工具一族；**顶部兼容字段**——真实机制按组描述在
/// [`ModeGroupSpec::step`]，本函数只服务 GET 载荷的 `switchKind` 旧字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSwitchKind {
    /// shift+tab 循环（claude / opencode）：一次按键切一档
    ShiftTabCycle,
    /// 斜杠命令注入（codex / kimi）：`/plan`、`/permissions` 等
    SlashCommand,
    /// 无机制（未实测/不支持的工具）→ 端点拒绝，前端只显示当前档（若可屏读）
    Unsupported,
}

/// 会话工具 → 旧 `switchKind` 字段值（纯函数，可测）。
///
/// **T4 起 kimi 归 `SlashCommand`**：§2.6 给 kimi 的模式组是 `/plan on|off`（带参
/// 直达），不再是「盲切一档」；权限组是 `/permission` + `/yolo`/`/auto`。旧值
/// `ShiftTabCycle` 会让前端渲染「循环一步」钮（新结构下已由 `groups[].step` 驱动），
/// 故此处与事实对齐——本字段现在是**旧客户端的降级路径**，不是机制的唯一来源。
pub fn switch_kind(tool: &str) -> ModeSwitchKind {
    match tool {
        "claude" | "opencode" => ModeSwitchKind::ShiftTabCycle,
        "codex" | "kimi" => ModeSwitchKind::SlashCommand,
        _ => ModeSwitchKind::Unsupported,
    }
}

/// 该工具是否支持**屏读回显当前模式**（T4：四家全开）。
///
/// 批次丙 T6 只放开 opencode（保守）；T6 实机探测已证**四家底栏都有可解析的档位
/// 文本**（opencode `Build`/`Plan`、claude `⏸ plan mode on` 等、codex `Plan mode`、
/// kimi `plan` 前缀）→ T4 按台账收口全开。「有回读源」不等于「一定读得到」：屏读
/// 失败 / 形态漂移仍返回 `None` → 前端「请人工核对」（红线 4）。
pub fn mode_readback_supported(tool: &str) -> bool {
    matches!(tool, "claude" | "opencode" | "codex" | "kimi")
}

/// 切换动作形态（按键 vs 文本 vs 两段式菜单）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSwitchPlan {
    /// 单键（键域内名称，走 `inject_key_spec`）——一步一档的循环切换
    Key(&'static str),
    /// 单段文本（斜杠命令）——文本注入 + 回车提交
    Text(&'static str),
    /// **两段式**（§2.6 codex/kimi 权限组）：先投递 `open` 命令打开菜单，再屏读
    /// 定位 + 导航确认（第二段的实现点与守卫关系见 `remote::api::session_mode_switch`）
    Menu {
        /// 打开菜单的命令（`/permissions` / `/permission`）
        open: &'static str,
        /// 菜单里的目标档
        target: MamMode,
    },
}

impl ModeSwitchPlan {
    /// 本次注入是否**两段式菜单**（第一段开菜单、第二段屏读定位 + 导航确认）。
    ///
    /// 抽成方法而不是在端点写 `matches!(plan, ..Menu..)`：这个布尔决定**回执文案**
    /// （`remote::api::mode_verify_receipt` 的「屏读推算、未回读确认」限定，T4 复评
    /// I2）——判据若散在调用点，将来加变体（如第三种两段式）时回执会静默漏掉限定，
    /// 而那正是「不假装成功」要覆盖的态。放在内核里，**加变体时编译器会提醒改这里**
    /// （endpoint 用的就是本方法，不是自己的 matches）。
    pub fn is_two_stage(self) -> bool {
        matches!(self, Self::Menu { .. })
    }
}

/// 拒绝切换的原因（**如实回执**，不是静默失败）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchRefusal {
    /// 该工具的模式切换未实测（`Unsupported`）→ 端点 409 `no_mechanism`
    Unsupported,
    /// 该工具+组+档无实测机制，附**具体原因**（端点 409 `no_mechanism` + `reason`：
    /// 例如 codex 退出计划模式无命令、codex 权限组以外的档）
    NoMechanism(&'static str),
}

/// 模式切换的注入序列构造（纯函数，§2.6 逐格）。
///
/// 传 `group` 必须是**该工具结构里存在的组**（调用方先经 [`ModeStructure::group`] 校验）。
pub fn mode_switch_plan(
    tool: &str,
    group: ModeGroupId,
    target: MamMode,
) -> Result<ModeSwitchPlan, SwitchRefusal> {
    match (tool, group, target) {
        // shift+tab 一步（claude/opencode 单轴；目标档不参与按键构造）
        ("claude", ModeGroupId::Mode, _) | ("opencode", ModeGroupId::Mode, _) => {
            Ok(ModeSwitchPlan::Key("shift+tab"))
        }
        // codex 模式组：`/plan` 进 Plan（实测命令清单原文 "switch to Plan mode"）
        ("codex", ModeGroupId::Mode, MamMode::Plan) => Ok(ModeSwitchPlan::Text("/plan")),
        ("codex", ModeGroupId::Mode, _) => Err(SwitchRefusal::NoMechanism(
            "codex 退出计划模式无实测命令（可在终端按 shift+tab，或在计划批准框选第一项）",
        )),
        // codex 权限组：`/permissions` 两段式（命令→菜单→屏读定位→导航确认）
        ("codex", ModeGroupId::Permission, MamMode::ReadOnly)
        | ("codex", ModeGroupId::Permission, MamMode::Default)
        | ("codex", ModeGroupId::Permission, MamMode::Bypass) => Ok(ModeSwitchPlan::Menu {
            open: "/permissions",
            target,
        }),
        ("codex", ModeGroupId::Permission, _) => Err(SwitchRefusal::NoMechanism(
            "codex 权限菜单无该档（实测档位：只读 / Ask for approval / Full Access）",
        )),
        // kimi 模式组：`/plan on|off` 带参直达（TUI 内嵌实现：subcmd on/off/clear）
        ("kimi", ModeGroupId::Mode, MamMode::Plan) => Ok(ModeSwitchPlan::Text("/plan on")),
        ("kimi", ModeGroupId::Mode, MamMode::Default) => Ok(ModeSwitchPlan::Text("/plan off")),
        // kimi 权限组：`/yolo`、`/auto` 预选直达变体（§2.6）；「总是询问」无直达命令
        // → `/permission` 两段式
        ("kimi", ModeGroupId::Permission, MamMode::Default) => Ok(ModeSwitchPlan::Menu {
            open: "/permission",
            target: MamMode::Default,
        }),
        ("kimi", ModeGroupId::Permission, MamMode::AcceptEdits) => {
            Ok(ModeSwitchPlan::Text("/yolo"))
        }
        ("kimi", ModeGroupId::Permission, MamMode::Bypass) => Ok(ModeSwitchPlan::Text("/auto")),
        ("kimi", ModeGroupId::Permission, _) => Err(SwitchRefusal::NoMechanism(
            "kimi 权限档无实测机制（实测三档：总是询问 / 按需询问 / 永不询问）",
        )),
        _ => Err(SwitchRefusal::Unsupported),
    }
}

/// **POST 的组推断**（纯函数，可测；旧客户端不带 `group` 字段时的兼容口径）。
///
/// 规则（三条，按序）：
/// 1. 请求带 `group` → 必须存在于该工具结构（否则 [`SwitchRefusal::Unsupported`]
///    ——端点映射 400/409）；
/// 2. 不带 `group`：目标档**只在一个组里** → 取该组；
/// 3. 两组都有（codex/kimi 的 `default` 即此例）→ **取可选的那一组**；两组都可选
///    → 取**模式组**。
///
/// 规则 3 的依据（逐条）：旧客户端（批次丙 T6 前端的调用面）在 codex 上只会发
/// `plan`（模式组）与 `bypass`（权限组）——两者都不歧义；在 kimi 上发的是
/// `default`（那是对 `ShiftTabCycle` 的「切一档」语义，实际效果 = 切plan 档），
/// 因此 `default` 归**模式组**才与旧行为同义。codex 的 `default` 在模式组里
/// **不可选**（无退出命令）而权限组里可选 → 落到权限组（= 旧实现 `/permissions`
/// 开面板的同义升级：现在是完整两段式）。两条都是「旧意图 → 最接近的可行动作」。
pub fn resolve_group(
    tool: &str,
    requested: Option<ModeGroupId>,
    target: MamMode,
) -> Result<ModeGroupId, SwitchRefusal> {
    let structure = mode_structure(tool);
    // 组必须先存在，且**目标档要在该组的 tiers 里**——否则「组对了但档不在组里」
    // 会被下游当作机制缺失，而调用方无法预判（判据只在这一处，不散两处）
    let in_group = |g: &ModeGroupSpec| g.tiers.iter().any(|t| t.mode == target);
    if let Some(id) = requested {
        let spec = structure.group(id).filter(in_group);
        return spec.map(|g| g.id).ok_or(SwitchRefusal::Unsupported);
    }
    let candidates: Vec<ModeGroupSpec> = structure.groups().into_iter().filter(in_group).collect();
    match candidates.as_slice() {
        [] => Err(SwitchRefusal::Unsupported),
        [only] => Ok(only.id),
        _ => {
            // 歧义：优先可取的那一组（档位在组内可选）；两组都可选 → 模式组（顺序即
            // groups() 的序：mode 在前）
            let selectable = candidates
                .iter()
                .find(|g| g.tiers.iter().any(|t| t.mode == target && t.selectable));
            Ok(selectable.map(|g| g.id).unwrap_or(candidates[0].id))
        }
    }
}

/// **codex `/plan` 运行中不可用**（§2.6 表末「运行中不可用→如实回执」）——纯判据。
///
/// 判据来源：codex 0.155.1 二进制内嵌文案 `Plan mode unavailable right now.`
/// （`slash_dispatch` 分支，与 `/plan` 的 in-progress 门同源）——即 codex 自己就会
/// 拒；MAM 侧**在投递前**判，才能给用户一份**如实回执**而不是「已发送」后无变化。
///
/// 「运行中」的口径复用 [`crate::inject::queue::is_running`]（Processing / Thinking /
/// Compacting 三态——队列层既有单一判据，不另立一份）。只对 **codex × 模式组 × Plan**
/// 生效：其余家其余档**未实测**是否有同类限制，不扩张（结论不得超过证据）。
pub fn codex_plan_busy(tool: &str, group: ModeGroupId, target: MamMode, is_running: bool) -> bool {
    tool == "codex" && group == ModeGroupId::Mode && target == MamMode::Plan && is_running
}

/// 屏读文本 → 当前模式（**按工具分族解析**，纯函数可测）。
///
/// # 为什么分族（本模块的判断，理由）
///
/// 批次丙 T6 的实现是**工具无关**的关键词匹配（`accept edits`/`bypass`/`plan`/
/// `build` 谁先命中谁算）。T4 放开三家回读后这条路走不通：
/// - **误判是必然的，不是概率问题**：claude 屏上 `⏸ plan mode on` 与 codex 屏上
///   `Plan mode (shift+tab to cycle)` 都含 `plan`，而它们**不是同一档**（codex 的
///   `Plan mode` 是模式档、claude 的也是模式档，但 codex 底栏的 `Plan mode` 与
///   claude 底栏的 `plan mode on` 判据不同源）；更危险的是**正文**：claude 计划批准
///   框里就有 `No, stay in Plan mode`，工具无关匹配会把「对话框在场」读成「当前是
///   Plan 档」；
/// - **每家的缺席语义不同**（codex/kimi 的 Default 靠**缺席**推断，claude/opencode
///   靠**在场词**）——工具无关的匹配器无法表达「认出这个工具的底栏了，但上面没有
///   档位词」这层结论，只能返回 None，等于白放开；
/// - 分族后每家都能贴着自己的**真机屏幕原文**写死（本模块测试用的就是 T6 档案里的
///   逐字快照），可回归、可追责。
///
/// # 各家族判据（夹具 = `research/refs/phase2-消息注入/2026-09-21-四家模式切换shift+tab实机探测.md` 与
/// `%TEMP%\mam-probe-c3-20260921-150000\evidence\screen-*` 的**逐字原文**）
///
/// 一律**自底向上**扫描（底栏在屏幕最下方，取最靠下的一条命中——正文里引用模式词
/// 的段落总在底栏之上）：
///
/// | 工具 | 判据 | 实录 |
/// |---|---|---|
/// | claude | `accept edits on`→AcceptEdits；`plan mode on`→Plan；`auto mode on`→Bypass；`manual mode on`→Default | `⏵⏵ accept edits on (shift+tab to cycle) · ← for agents` / `⏸ plan mode on …` / `⏵⏵ auto mode on …` / `⏸ manual mode on · ? for shortuts ·←for agents` |
/// | opencode | 独立词 `build`→Default；独立词 `plan`→Plan | `Build   DeepSeek V4.1 Flash Ollama Cloud ·high` / `Plan   …   high` |
/// | kimi | 底栏（最后一条含 `thinking:` 的行）首词 `plan`→Plan；**认出底栏但无前缀→Default** | `plan  GLM-5.3-Flash thinking: high  C:\…\proj-kimi` / 默认态 `GLM-5.3-Flash thinking: high  C:\…` |
/// | codex | 行含独立短语 `plan mode`→Plan；行含 ` · `（状态栏分隔符）且无 `plan mode`→**Default（缺席推断）** | `glm-5.3-flash medium · ~\…\proj-codex  Plan mode (shift+tab to cycle)` / 默认档 `glm-5.3-flash medium · ~\…\proj-codex` |
///
/// # 保守面（如实申报）
///
/// - codex 的 Default 依赖 ` · ` 状态栏形态（实测两态都有）；若某版本改版式 → 返回
///   None（未知）而不是乱猜；
/// - kimi 的底栏识别依赖 `thinking:`（实测两台模型皆有；无思考能力的模型可能不印
///   这段 → 返回 None = 未知，方向保守）；
/// - **claude / codex** 的正文若出现与底栏**逐字相同**的短句且位于更下方，可能误判
///   ——两家的底栏判据是「含某子串」（`plan mode on` / `plan mode` + ` · `），
///   自底向上扫描只能缓解不能消除「正文里也有同样一行」的情形。实机由 `#[ignore]`
///   全量回读用例逐例核对（四个真机快照已在门禁内锁住；对话框遮挡时自然返回 None，
///   见 `claude_dialog_does_not_fake_plan_mode`）。
pub fn parse_mode_from_screen(tool: &str, lines: &[String]) -> Option<MamMode> {
    match tool {
        "claude" => parse_claude_footer(lines),
        "opencode" => parse_opencode_footer(lines),
        "kimi" => parse_kimi_footer(lines),
        "codex" => parse_codex_footer(lines),
        // 未实测工具：不猜（旧实现会对任意工具跑关键词表——T4 起按族收敛）
        _ => None,
    }
}

/// claude 底栏（自底向上；四档词表 = 实测环序的四条原文）
fn parse_claude_footer(lines: &[String]) -> Option<MamMode> {
    for line in lines.iter().rev() {
        let lower = line.to_lowercase();
        if lower.contains("accept edits on") {
            return Some(MamMode::AcceptEdits);
        }
        if lower.contains("plan mode on") {
            return Some(MamMode::Plan);
        }
        if lower.contains("auto mode on") {
            // happy 的 auto 归 MAM 的 Bypass（调研档案 §3 的收敛表）
            return Some(MamMode::Bypass);
        }
        if lower.contains("manual mode on") {
            return Some(MamMode::Default);
        }
    }
    None
}

/// opencode 底栏（自底向上；`Build`=Default 是裁6 的术语来源，`Plan`=Plan）
fn parse_opencode_footer(lines: &[String]) -> Option<MamMode> {
    for line in lines.iter().rev() {
        let lower = line.to_lowercase();
        if contains_word(&lower, "plan") {
            return Some(MamMode::Plan);
        }
        if contains_word(&lower, "build") || contains_word(&lower, "default") {
            return Some(MamMode::Default);
        }
    }
    None
}

/// kimi 底栏（最后一条含 `thinking:` 的行；首词 `plan` → Plan，否则 Default）
fn parse_kimi_footer(lines: &[String]) -> Option<MamMode> {
    // 识别标记：底栏形如 `<[plan]  ><模型> thinking: <effort>  <工作目录>`。用
    // `thinking:` 作锚——它是四份真机快照里共同的、与模式无关的稳定段；认不出锚
    // 即返回 None（宁可「未知」，不把没读到的屏说成 Default）
    let footer = lines
        .iter()
        .rev()
        .find(|l| l.to_lowercase().contains("thinking:"))?;
    let first = footer.split_whitespace().next()?;
    if first.eq_ignore_ascii_case("plan") {
        Some(MamMode::Plan)
    } else {
        // 缺席推断（§2.6 表末）：底栏在、没写 plan → 默认档
        Some(MamMode::Default)
    }
}

/// codex 底栏（自底向上；`Plan mode` 短语 → Plan，` · ` 状态栏在场 → Default）
///
/// **与 claude 的区分点（分族的关键）**：claude 的底栏逐字是 `plan mode on`
/// （带 `on`），codex 的是裸 `Plan mode`。两者的 `(shift+tab to cycle)` 后缀相同，
/// 分隔符 ` · ` 也相同——只有这个 `on` 能分开它们。故本族的 Plan 判据是
/// 「含 `plan mode` 且**不含** `plan mode on`」。
fn parse_codex_footer(lines: &[String]) -> Option<MamMode> {
    for line in lines.iter().rev() {
        let lower = line.to_lowercase();
        // claude 的形态（`… mode on`）不是 codex 的判据 → 跳过
        if lower.contains("plan mode on") {
            continue;
        }
        // `Plan mode` 是**独立短语**（词边界：`Planner`/`planning` 不命中）
        if lower.contains("plan mode") {
            return Some(MamMode::Plan);
        }
        // 状态栏形态：`<模型> · <工作目录>`（默认档没有模式字样 → 缺席即 Default）。
        // 用**带空格的中点分隔符**做锚，避免正文里的裸 `·` 误命中
        if line.contains(" · ") {
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

// ===== 回读确认（裁5「回读确认」/ 红线 4「不假装成功」）=====

/// 一次切档的**回读结论**（端点回执据此下发 `verified`/`hint`）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeVerify {
    /// 回读命中目标档（`verified=true`）
    Confirmed,
    /// 回读成功但**不是**目标档——如实报出两边（不假装成功）
    Mismatch {
        /// 按机制推算的应到档
        expected: MamMode,
        /// 屏读到的实际档
        observed: MamMode,
    },
    /// 无法判定（无回读源 / 屏读失败 / 环序未知）——前端「请人工核对终端」
    Unverifiable,
}

/// 切完一轮后**应到的档**（纯函数）。
///
/// - 步进组（claude/opencode）：环序推算 [`cycle_next`]——**当前档未知时返回 None**
///   （不能凭空预测，那样只会造出假 Mismatch）；
/// - 直达组：目标档本身，但**仅当该组有回读源**（如 codex 权限组无底栏回读 ⇒ None，
///   端点回执如实说「该组无回读源，请人工核对」）。
pub fn expected_mode_after(
    tool: &str,
    group: ModeGroupId,
    target: MamMode,
    current: Option<MamMode>,
) -> Option<MamMode> {
    let spec = mode_structure(tool).group(group)?;
    if spec.step {
        return current.and_then(|c| cycle_next(tool, c));
    }
    if spec.readback {
        Some(target)
    } else {
        None
    }
}

/// 回读确认（纯函数）：`expected` 与屏读结果对账。
pub fn verify_mode_switch(expected: Option<MamMode>, observed: Option<MamMode>) -> ModeVerify {
    match (expected, observed) {
        (Some(e), Some(o)) if e == o => ModeVerify::Confirmed,
        (Some(e), Some(o)) => ModeVerify::Mismatch {
            expected: e,
            observed: o,
        },
        _ => ModeVerify::Unverifiable,
    }
}

// ===== 两段式的第二段：菜单定位与导航（§2.6 codex/kimi 权限组）=====

/// 权限菜单的**已知档位标签**（工具官方词，`/permissions` / `/permission` 弹窗原文）。
///
/// codex 序按 M9R 实机记录（`inject::approve` 表注：Read Only / Ask for approval /
/// Approve for me / Full Access，与手册快照序不同）；kimi 按 TUI 内嵌 i18n
/// （`PERMISSION_OPTIONS`：Always Ask / Ask When Needed / Never Ask）。
///
/// **注意：本表顺序不参与导航计算**——导航从**屏上实际顺序**（[`locate_menu_items`]
/// 的产出）算，本表只回答「哪些词算这一档」。弹窗里缺失的项（如 Guardian 关闭时的
/// `Approve for me`）因此天然不参与步进。
pub fn menu_labels(tool: &str) -> &'static [&'static str] {
    match tool {
        "codex" => &[
            "Read Only",
            "Ask for approval",
            "Approve for me",
            "Full Access",
        ],
        "kimi" => &["Always Ask", "Ask When Needed", "Never Ask"],
        _ => &[],
    }
}

/// 菜单里**该目标档**的标签（找不到 → None：该菜单里没有这一档，不出手）
pub fn menu_target_label(tool: &str, target: MamMode) -> Option<&'static str> {
    match (tool, target) {
        ("codex", MamMode::ReadOnly) => Some("Read Only"),
        ("codex", MamMode::Default) => Some("Ask for approval"),
        ("codex", MamMode::Bypass) => Some("Full Access"),
        ("kimi", MamMode::Default) => Some("Always Ask"),
        ("kimi", MamMode::AcceptEdits) => Some("Ask When Needed"),
        ("kimi", MamMode::Bypass) => Some("Never Ask"),
        _ => None,
    }
}

/// **菜单定位**（纯函数，第二段的第一步）：屏读行集 → 编号化的菜单项表。
///
/// 与 [`crate::inject::dialog::parse_dialog_options`] 的关键差别：**权限菜单不带编号**
/// （arrow-key 选择器），所以判据是**标签前缀**：行首（剥掉前导空白与光标标记后）
/// 以某个已知档位标签开头即为菜单项，序号按**屏上出现顺序**编（1 起）。
///
/// **为什么必须行首匹配**：codex 的 Full Access 档带一段警示语，其中就有
/// `We strongly recommend selecting "Ask for approval" instead.`——若用「包含」判据，
/// 这行会被当成一个菜单项，菜单项数凭空 +1 → 循环步进算错 → **选错档**（正是
/// `dialog::navigation_sequence` 文档里「想拒绝却批准」同类事故）。行首匹配把
/// 引文行排除在外。
///
/// 返回 `None`（端点据此**中止第二段并如实回执**，绝不盲发方向键）：菜单项 < 2
/// （不成菜单）。**其余保守面在 [`menu_navigation_sequence`]**（那里知道是哪个工具，
/// 才能判「这一家的菜单应当长什么样」）——本函数只做**原始定位**，其输出**不可直接
/// 行动**（见 [`menu_items_coherent`] 的文档）。
///
/// # 已知窄面（T4 复评 I1；本批**选择如实申报 + 补一道可证据化的闸**，不做块簇加固）
///
/// **反例（评审抓获，成立）**：行首匹配**不能定位「菜单块」**。同屏若有别的**正文/
/// 状态行以某个档位标签开头**（评审给的真机例子：codex `/status` 输出回显
/// `Read Only (sandbox: read-only)`），它会被计为一个菜单项 → 项数凭空 +1 → 目标档的
/// 步进**可能多一步**（评审的算例：目标 `Full Access` 从正确的 `↓,enter` 变成
/// `↓↓,enter`；若菜单不回卷，多一步会把高亮停在**别的档**——用户点 Full Access
/// 而实际切到 Read Only）。
///
/// **为什么选「如实申报 + 可证据化的闸」而不是「块簇 + 紧邻标题加固」（依据 = 证据量）**：
/// 块簇加固需要「菜单块长什么样」的**实机屏幕逐行原文**——菜单标题文本（codex
/// `Update Model Permissions` / kimi 的对应标题）、档位行之间是否夹描述行、光标标记
/// 用哪个字符，这些**全部来自二进制内嵌文案，本仓没有一份实机菜单快照**（T4 已如实
/// 申报；`t4_permission_menu_screen_shape_live_probe` 就是去取这个证据的用例）。
/// 在无原文的前提下写「紧邻标题 + 连续成簇」的判据 = 拿推演当规范，正是本批反复栽
/// 跟头的模式（T2 的 Processing、T3 的空断言、C1 的全 ASCII 夹具）。按「结论不得
/// 超过证据」：**不加固块结构判据**，改为在 [`menu_items_coherent`] 上加一道
/// **只依赖「菜单应含哪些档」这一事实**的闸（该事实有 §2.6 与 M9R 档位序记录支撑），
/// 并把块簇加固列入下批（前置条件 = 实机菜单快照）。
pub fn locate_menu_items(lines: &[String], labels: &[&str]) -> Option<Vec<DialogOption>> {
    let mut items: Vec<DialogOption> = Vec::new();
    for line in lines {
        let (rest, highlighted) = crate::inject::dialog::strip_cursor_marker(line);
        let text = rest.trim();
        if text.is_empty() {
            continue;
        }
        let Some(label) = labels.iter().find(|l| starts_with_ignore_case(text, l)) else {
            continue;
        };
        items.push(DialogOption {
            number: items.len() as u32 + 1,
            // label 存屏上原文（前导已剥），便于回执/日志引用
            label: format!("{label}|{text}"),
            highlighted,
        });
    }
    if items.len() < 2 {
        return None;
    }
    Some(items)
}

/// **菜单项表的一致性判据**（纯函数，可测）——[`locate_menu_items`] 的原始输出
/// **必须过这道闸才能拿去算步进**。
///
/// # 两条不变式（都由「菜单应含哪些档」这一**有据事实**导出，不依赖菜单块形态）
///
/// 1. **每个档位标签恰好出现一次**：权限菜单每一档只渲染一行，故同一标签出现两次
///    = 屏上混进了以该标签开头的正文/状态行（评审的反例：`/status` 回显
///    `Read Only (sandbox: read-only)` 与真菜单项 `Read Only` **重复**）。这一条
///    **正好命中评审的算例**——原始 4 项里 `Read Only` 出现两次 → 拒绝。
/// 2. **目标档全集齐备**：该工具的三个用户可切档都得在场（§2.6 codex/kimi 各三档；
///    M9R 记录 codex 实机档位序含 `Approve for me`，但它只在 Guardian 开启时出现，
///    故**不算进**目标全集——把它算进会让 Guardian 关闭的正常菜单被判不完整而永久
///    无法切档）。缺档 = 菜单被对话框/正文挤掉一部分 → 步进不可信 → 拒绝。
///
/// # 残留风险（如实申报，不假装已消除）
///
/// 若同屏混入的行**恰好补齐了缺席的目标档**（菜单只画出 2 档、正文里正好有一行以
/// 第 3 个目标标签开头），两条不变式都会通过。这需要两个巧合同时发生（菜单不完整
/// **且**正文给出恰好缺的那个标签），概率远低于评审的单行混入反例。**本条窄面留在
/// 台账**，块簇加固在下批做（前置 = 实机菜单快照）。
pub fn menu_items_coherent(items: &[DialogOption], tool: &str) -> bool {
    let seen: Vec<&str> = items
        .iter()
        .filter_map(|o| o.label.split_once('|').map(|(l, _)| l))
        .collect();
    if seen.len() != items.len() {
        return false; // 形态异常（label 未按 `标签|原文` 构造）
    }
    // 不变式 1：无重复
    for (i, a) in seen.iter().enumerate() {
        if seen[i + 1..].iter().any(|b| a.eq_ignore_ascii_case(b)) {
            log::debug!("权限菜单定位：标签重复（{a}）→ 屏上混入正文行，中止（不盲发方向键）");
            return false;
        }
    }
    // 不变式 2：目标档全集齐备
    let want = menu_target_labels(tool);
    if want.is_empty() {
        return false; // 未实测工具
    }
    let missing: Vec<&&str> = want
        .iter()
        .filter(|w| !seen.iter().any(|s| s.eq_ignore_ascii_case(w)))
        .collect();
    if !missing.is_empty() {
        log::debug!("权限菜单定位：目标档不全（缺 {missing:?}）→ 中止（不盲发方向键）");
        return false;
    }
    true
}

/// 工具 → **目标档标签全集**（用户可切的三档；codex/kimi 各三条，见 §2.6）。
///
/// **不含** codex 的 `Approve for me`：它只在 Guardian 开启时出现（M9R 表注），
/// 且 §2.6 未把它列为目标档——算进完整集会误伤正常菜单。
fn menu_target_labels(tool: &str) -> &'static [&'static str] {
    match tool {
        "codex" => &CODEX_TARGET_LABELS,
        "kimi" => &KIMI_TARGET_LABELS,
        _ => &[],
    }
}

/// codex 权限菜单的**目标档标签全集**（与 [`menu_target_label`] 的取值一一对应）
const CODEX_TARGET_LABELS: [&str; 3] = ["Read Only", "Ask for approval", "Full Access"];
/// kimi 权限菜单的目标档标签全集（§2.6 三档）
const KIMI_TARGET_LABELS: [&str; 3] = ["Always Ask", "Ask When Needed", "Never Ask"];

/// 前缀匹配（**大小写不敏感的「以…开头」**；`text` 以 `prefix` 开头即命中）。
///
/// # 为什么按**字符**逐位比较，而不是 `text[..prefix.len()]`（T4 复评 C1 的根因）
///
/// 字节切片版本 `text[..prefix.len()]` 有一个致命的隐式前提：**`prefix.len()` 处
/// 必须是字符边界**。屏读行里只要前 `prefix.len()` 个字节跨了非 ASCII 字符（真机
/// 屏幕**每一份**都能凑出这种行——`—`(U+2014) 占 3 字节、`╭─`(U+256D)、中文都是），
/// 切片即 panic：`end byte index 9 is not a char boundary; it is inside '—'`。
///
/// 后果链（T4 复评抓获）：本函数在 `second_stage_menu` 的 spawn_blocking 闭包内被
/// 调用 → panic 让该任务 JoinError → 端点回 **500 internal**（而不是如实回执），
/// 且第一段 `/permissions` 已经投出、菜单开在用户终端上**没人收尾**。
/// **`#[cfg(windows)]` 之外的门禁永不覆盖这条路径**——正是「测试覆盖不到的实机路径」
/// 的典型（旧夹具全是纯 ASCII，等于把实机形态排除在门禁之外；现已有非 ASCII 真机
/// 夹具，见 `menu_locator_survives_real_non_ascii_screens`）。
///
/// 逐字符比较的语义与旧实现**完全一致**（`char::eq_ignore_ascii_case` 同样只折叠
/// ASCII 大小写——与 `str::eq_ignore_ascii_case` 同口径），且不再有任何字节索引。
fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    let mut t = text.chars();
    for pc in prefix.chars() {
        match t.next() {
            Some(tc) if pc.eq_ignore_ascii_case(&tc) => {}
            _ => return false,
        }
    }
    // prefix 走完即命中（空 prefix 恒真——调用方传的是非空标签表，此处不加特判）
    true
}

/// **第二段的完整按键序列**（纯函数）：屏读行集 → 目标档的 `[↑/↓ × k, Enter]`。
///
/// 走 [`crate::inject::dialog::navigation_sequence_directional`]（**方向感知、不假设
/// 回卷**——理由见该函数文档：权限菜单的回卷行为无实测，循环前进会在「目标在高亮之上」
/// 时发出可能切错档的 ↓ 序列；codex 的 M9R 实机取证正是 ↑+Enter）。目标越界 /
/// 无高亮 / 多高亮的保守面与审批路径共用同一份判据。
///
/// **定位结果先过 [`menu_items_coherent`]**（T4 复评 I1 的闸）：同屏混入的「以档位
/// 标签开头的正文行」会破坏一致性（重复标签 / 目标档不齐）→ 本函数 Err → 端点中止
/// 第二段 + 如实回执，**不盲发方向键**。窄面与残留风险见该函数文档。
pub fn menu_navigation_sequence(
    lines: &[String],
    tool: &str,
    target: MamMode,
) -> Result<Vec<String>, String> {
    let labels = menu_labels(tool);
    let target_label = menu_target_label(tool, target)
        .ok_or_else(|| format!("{tool} 的权限菜单里没有「{}」档", target.label()))?;
    let items = locate_menu_items(lines, labels)
        .ok_or_else(|| format!("{tool} 的权限菜单未出现或读不到档位表（不盲发方向键）"))?;
    if !menu_items_coherent(&items, tool) {
        return Err(format!(
            "{tool} 的权限菜单档位表不自洽（可能混入了正文行或被遮挡）——不猜位置，请在终端选择"
        ));
    }
    let hit = items
        .iter()
        .find(|o| {
            o.label
                .split_once('|')
                .map(|(l, _)| l.eq_ignore_ascii_case(target_label))
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("屏上菜单没有「{target_label}」项（不猜位置）"))?;
    crate::inject::dialog::navigation_sequence_directional(&items, hit.number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ==== 统一枚举 ====

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

    /// **裁7 的输入面锁**：codex 退役旧档（untrusted / on-failure）**不可解析**——
    /// 不是「前端不渲染」的自律，而是后端的 POST 入口根本不认（→ 400）。
    /// 还原动作：给 `MamMode::parse` 加 `"untrusted"` 分支 → 本断言先红。
    #[test]
    fn mode_legacy_enum_is_not_selectable() {
        for legacy in ["untrusted", "on-failure", "onFailure", "untrusted_approval"] {
            assert_eq!(
                MamMode::parse(legacy),
                None,
                "退役档不得成为可选档（裁7）：{legacy}"
            );
        }
        // 且它们**不在**任何组的 tiers 里（结构面同样不得出现）
        for tool in ["codex", "kimi", "claude", "opencode"] {
            for g in mode_structure(tool).groups() {
                for t in g.tiers {
                    assert!(
                        !t.label.eq_ignore_ascii_case("untrusted")
                            && !t.label.eq_ignore_ascii_case("on-failure"),
                        "{tool} 的 {} 组 tiers 混入退役档 {}",
                        g.id.wire(),
                        t.label
                    );
                }
            }
        }
        // 只在 codex 权限组的 legacy 清单里如实登记
        let codex = mode_structure("codex");
        let perm = codex.group(ModeGroupId::Permission).unwrap();
        let names: Vec<&str> = perm.legacy.iter().map(|l| l.label).collect();
        assert_eq!(names, vec!["untrusted", "on-failure"]);
        assert!(
            perm.legacy.iter().all(|l| !l.note.is_empty()),
            "legacy 档必须带退役依据（如实申报）"
        );
        // 其余组不带 legacy
        assert!(codex.group(ModeGroupId::Mode).unwrap().legacy.is_empty());
        assert!(mode_structure("kimi")
            .groups()
            .iter()
            .all(|g| g.legacy.is_empty()));
    }

    /// 机制分族：claude/opencode 步进；codex/kimi 命令；其余不支持
    #[test]
    fn switch_kind_routing() {
        for t in ["claude", "opencode"] {
            assert_eq!(switch_kind(t), ModeSwitchKind::ShiftTabCycle, "{t}");
        }
        for t in ["codex", "kimi"] {
            assert_eq!(switch_kind(t), ModeSwitchKind::SlashCommand, "{t}");
        }
        for t in ["zcode", "dsh", "workbuddy", "openclaw", ""] {
            assert_eq!(
                switch_kind(t),
                ModeSwitchKind::Unsupported,
                "{t} 未实测 → 不出手"
            );
        }
    }

    /// 回读四家全开（批次丙台账开放项 3 的收口）；未实测工具仍 false
    #[test]
    fn readback_open_for_four_tools() {
        for t in ["claude", "opencode", "codex", "kimi"] {
            assert!(
                mode_readback_supported(t),
                "{t} 底栏有实测档位文本 → 放开回读"
            );
        }
        for t in ["zcode", "dsh", "workbuddy"] {
            assert!(!mode_readback_supported(t), "{t} 未实测 → 不回读");
        }
    }

    // ==== 结构表（§2.6 逐格）====

    /// §2.6 规格表逐格：结构 / 档位标签 / 顺序
    #[test]
    fn structure_matches_spec_table() {
        // codex：两组（模式组 默认/计划；权限组 只读/默认/完全信任）
        let codex = mode_structure("codex");
        assert_eq!(codex.wire(), "twoAxis");
        let m = codex.group(ModeGroupId::Mode).unwrap();
        assert_eq!(m.label, "模式");
        assert!(!m.step, "codex 模式组是逐档按钮（/plan）");
        let labels: Vec<&str> = m.tiers.iter().map(|t| t.label).collect();
        assert_eq!(labels, vec!["默认", "计划"]);
        let p = codex.group(ModeGroupId::Permission).unwrap();
        assert_eq!(p.label, "权限");
        let labels: Vec<&str> = p.tiers.iter().map(|t| t.label).collect();
        assert_eq!(labels, vec!["只读", "默认", "完全信任"]);
        assert_eq!(
            p.tiers.iter().map(|t| t.mode).collect::<Vec<_>>(),
            vec![MamMode::ReadOnly, MamMode::Default, MamMode::Bypass]
        );

        // kimi：两组（模式组 默认/计划；权限组 总是询问/按需询问/永不询问）
        let kimi = mode_structure("kimi");
        assert_eq!(kimi.wire(), "twoAxis");
        let m = kimi.group(ModeGroupId::Mode).unwrap();
        assert_eq!(
            m.tiers.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["默认", "计划"]
        );
        let p = kimi.group(ModeGroupId::Permission).unwrap();
        assert_eq!(
            p.tiers.iter().map(|t| t.label).collect::<Vec<_>>(),
            vec!["总是询问", "按需询问", "永不询问"],
            "§2.6 kimi 权限组屏显标签（工具自己的词，不是 MAM 通用名）"
        );
        // kimi 权限三档 → MAM 映射（模块文档表的机器可验版本）
        assert_eq!(p.tiers[0].mode, MamMode::Default, "总是询问 = 每步询问");
        assert_eq!(
            p.tiers[1].mode,
            MamMode::AcceptEdits,
            "按需询问 = 常规改动/命令自动、高危仍问 → MAM 唯一的「常规改动自动」档"
        );
        assert_eq!(p.tiers[2].mode, MamMode::Bypass, "永不询问 = 不再询问");
        assert!(
            p.tiers.iter().all(|t| t.selectable),
            "kimi 权限三档全部可选（总是询问走 /permission 两段式；其余两档有直达变体）"
        );

        // claude：单轴四档，**顺序 = 实测环序**（非 MamMode::ALL 的枚举序）
        let claude = mode_structure("claude");
        assert_eq!(claude.wire(), "singleAxis");
        let axis = claude.group(ModeGroupId::Mode).unwrap();
        assert!(axis.step, "shift+tab 一步一档");
        assert_eq!(
            axis.tiers.iter().map(|t| t.mode).collect::<Vec<_>>(),
            vec![
                MamMode::AcceptEdits,
                MamMode::Plan,
                MamMode::Bypass,
                MamMode::Default
            ],
            "实测环序 acceptEdits → plan → auto → manual"
        );

        // opencode：单轴两档（Build 已按裁6 改显「默认」）
        let oc = mode_structure("opencode");
        assert_eq!(oc.wire(), "singleAxis");
        assert_eq!(
            oc.group(ModeGroupId::Mode)
                .unwrap()
                .tiers
                .iter()
                .map(|t| t.label)
                .collect::<Vec<_>>(),
            vec!["默认", "计划"]
        );

        // 未实测工具：无结构
        assert_eq!(mode_structure("zcode").wire(), "none");
        assert!(mode_structure("zcode").groups().is_empty());
    }

    /// 二维家的**组存在性**：claude/opencode 没有权限组（单轴）；codex/kimi 有
    #[test]
    fn group_lookup_by_structure() {
        assert!(mode_structure("claude")
            .group(ModeGroupId::Permission)
            .is_none());
        assert!(mode_structure("opencode")
            .group(ModeGroupId::Permission)
            .is_none());
        assert!(mode_structure("codex")
            .group(ModeGroupId::Permission)
            .is_some());
        assert!(mode_structure("kimi")
            .group(ModeGroupId::Permission)
            .is_some());
    }

    /// 组 wire 词往返 + 标签
    #[test]
    fn group_id_wire_roundtrip() {
        for g in [ModeGroupId::Mode, ModeGroupId::Permission] {
            assert_eq!(ModeGroupId::parse(g.wire()), Some(g));
            assert!(!g.label().is_empty());
        }
        assert_eq!(ModeGroupId::parse("permissions"), None);
        assert_eq!(ModeGroupId::parse(""), None);
    }

    // ==== 切换序列（§2.6 切换机制列）====

    /// claude/opencode：一步 shift+tab（目标档不参与按键构造）
    #[test]
    fn plan_step_axis_is_one_shift_tab() {
        for t in ["claude", "opencode"] {
            for target in [MamMode::Plan, MamMode::Default, MamMode::AcceptEdits] {
                assert_eq!(
                    mode_switch_plan(t, ModeGroupId::Mode, target).unwrap(),
                    ModeSwitchPlan::Key("shift+tab"),
                    "{t} 循环一步"
                );
            }
        }
    }

    /// codex：`/plan` 进 Plan；退出 Plan **如实拒绝**（无实测命令）
    #[test]
    fn codex_mode_group_plan_and_honest_refusal() {
        assert_eq!(
            mode_switch_plan("codex", ModeGroupId::Mode, MamMode::Plan).unwrap(),
            ModeSwitchPlan::Text("/plan")
        );
        let refusal = mode_switch_plan("codex", ModeGroupId::Mode, MamMode::Default).unwrap_err();
        match refusal {
            SwitchRefusal::NoMechanism(reason) => {
                assert!(
                    reason.contains("退出计划模式"),
                    "如实回执要讲清原因：{reason}"
                )
            }
            other => panic!("期望 NoMechanism，得到 {other:?}"),
        }
    }

    /// codex 权限组：三档都走 `/permissions` 两段式；非三档 → 拒绝
    #[test]
    fn codex_permission_is_two_stage_menu() {
        for (target, _label) in [
            (MamMode::ReadOnly, "Read Only"),
            (MamMode::Default, "Ask for approval"),
            (MamMode::Bypass, "Full Access"),
        ] {
            assert_eq!(
                mode_switch_plan("codex", ModeGroupId::Permission, target).unwrap(),
                ModeSwitchPlan::Menu {
                    open: "/permissions",
                    target
                }
            );
        }
        assert!(matches!(
            mode_switch_plan("codex", ModeGroupId::Permission, MamMode::AcceptEdits),
            Err(SwitchRefusal::NoMechanism(_))
        ));
    }

    /// kimi：模式组 `/plan on|off` 直达；权限组 `/permission` 两段式 + `/yolo`、`/auto` 直达变体
    #[test]
    fn kimi_switch_mechanisms() {
        assert_eq!(
            mode_switch_plan("kimi", ModeGroupId::Mode, MamMode::Plan).unwrap(),
            ModeSwitchPlan::Text("/plan on")
        );
        assert_eq!(
            mode_switch_plan("kimi", ModeGroupId::Mode, MamMode::Default).unwrap(),
            ModeSwitchPlan::Text("/plan off")
        );
        assert_eq!(
            mode_switch_plan("kimi", ModeGroupId::Permission, MamMode::Default).unwrap(),
            ModeSwitchPlan::Menu {
                open: "/permission",
                target: MamMode::Default
            },
            "「总是询问」无直达命令 → 两段式"
        );
        assert_eq!(
            mode_switch_plan("kimi", ModeGroupId::Permission, MamMode::AcceptEdits).unwrap(),
            ModeSwitchPlan::Text("/yolo"),
            "「按需询问」= yolo 的预选直达变体（§2.6）"
        );
        assert_eq!(
            mode_switch_plan("kimi", ModeGroupId::Permission, MamMode::Bypass).unwrap(),
            ModeSwitchPlan::Text("/auto"),
            "「永不询问」= auto 的预选直达变体（§2.6）"
        );
    }

    /// **两段式判据**（T4 复评 I2 的单点）：只有 Menu 变体为真——回执的「屏读推算」
    /// 限定由它驱动（端点用本方法，不自己写 matches!）。
    /// 还原动作：把 `is_two_stage` 改成恒 true → `two_stage_receipt_*` 的成对锁先红。
    #[test]
    fn two_stage_flag_is_menu_only() {
        assert!(ModeSwitchPlan::Menu {
            open: "/permissions",
            target: MamMode::Bypass
        }
        .is_two_stage());
        assert!(!ModeSwitchPlan::Key("shift+tab").is_two_stage());
        assert!(!ModeSwitchPlan::Text("/plan on").is_two_stage());
    }

    /// 未实测工具 → Unsupported（端点 409 no_mechanism）
    #[test]
    fn unsupported_tool_refuses() {
        assert_eq!(
            mode_switch_plan("zcode", ModeGroupId::Mode, MamMode::Plan).unwrap_err(),
            SwitchRefusal::Unsupported
        );
        assert_eq!(
            mode_switch_plan("claude", ModeGroupId::Permission, MamMode::Plan).unwrap_err(),
            SwitchRefusal::Unsupported,
            "单轴工具没有权限组"
        );
    }

    /// 组推断（旧客户端不带 group）：唯一组 / 歧义取可选组 / 两组可选取模式组 / 无组
    #[test]
    fn group_inference_rules() {
        // 唯一组：claude 的 acceptEdits 只在单轴里
        assert_eq!(
            resolve_group("claude", None, MamMode::AcceptEdits),
            Ok(ModeGroupId::Mode)
        );
        // codex plan：只在模式组
        assert_eq!(
            resolve_group("codex", None, MamMode::Plan),
            Ok(ModeGroupId::Mode)
        );
        // codex bypass：只在权限组
        assert_eq!(
            resolve_group("codex", None, MamMode::Bypass),
            Ok(ModeGroupId::Permission)
        );
        // codex default：两组都有 → 模式组的该档**不可选**（无退出命令）→ 落权限组
        assert_eq!(
            resolve_group("codex", None, MamMode::Default),
            Ok(ModeGroupId::Permission),
            "歧义时取「可取的那一组」（codex 默认档在模式组不可选）"
        );
        // kimi default：两组都有且**都可选** → 模式组（旧 shift+tab 语义 = 切计划档）
        assert_eq!(
            resolve_group("kimi", None, MamMode::Default),
            Ok(ModeGroupId::Mode)
        );
        // 显式 group 优先，且必须在结构里存在
        assert_eq!(
            resolve_group("kimi", Some(ModeGroupId::Permission), MamMode::Default),
            Ok(ModeGroupId::Permission)
        );
        assert_eq!(
            resolve_group("claude", Some(ModeGroupId::Permission), MamMode::Default),
            Err(SwitchRefusal::Unsupported),
            "单轴工具没有权限组 → 拒绝"
        );
        // 结构里没有的档 → 拒绝
        assert_eq!(
            resolve_group("kimi", Some(ModeGroupId::Mode), MamMode::ReadOnly),
            Err(SwitchRefusal::Unsupported)
        );
    }

    // ==== codex 运行中门（§2.6 表末）====

    /// `/plan` 运行中不可用：只对 codex × 模式组 × Plan 生效，其余不扩张
    #[test]
    fn codex_plan_busy_predicate() {
        let busy = true;
        assert!(codex_plan_busy(
            "codex",
            ModeGroupId::Mode,
            MamMode::Plan,
            busy
        ));
        assert!(!codex_plan_busy(
            "codex",
            ModeGroupId::Mode,
            MamMode::Plan,
            false
        ));
        // 只有 Plan 档受限（权限组不受此门——未实测有同类限制，不扩张）
        assert!(!codex_plan_busy(
            "codex",
            ModeGroupId::Permission,
            MamMode::Bypass,
            busy
        ));
        // 其他家不套用 codex 的门（未实测）
        for t in ["claude", "kimi", "opencode"] {
            assert!(!codex_plan_busy(t, ModeGroupId::Mode, MamMode::Plan, busy));
        }
    }

    // ==== 环序与回读确认 ====

    /// claude 环序（实测四档循环）+ opencode 双档循环
    #[test]
    fn cycle_next_follows_measured_ring() {
        assert_eq!(
            cycle_next("claude", MamMode::AcceptEdits),
            Some(MamMode::Plan)
        );
        assert_eq!(cycle_next("claude", MamMode::Plan), Some(MamMode::Bypass));
        assert_eq!(
            cycle_next("claude", MamMode::Bypass),
            Some(MamMode::Default)
        );
        assert_eq!(
            cycle_next("claude", MamMode::Default),
            Some(MamMode::AcceptEdits),
            // 依据 = **底栏 shift+tab 的环序实测**（T6 探测档案 §1.2：逐次 shift+tab
            // 得到 acceptEdits → plan → auto → manual，第四次后又回到 acceptEdits）。
            // 注意：这与 dialog::navigation_sequence 的「↓ 到尾部回卷」**不是同一条
            // 证据**——那是 claude **编号对话框**的行选择器行为（R1 实测），此处用的是
            // 模式栏的 shift+tab 循环。两条实测结论恰好一致（都回卷），但证据来源不同。
            "环尾回到环首（依据 = T6 底栏 shift+tab 环序实测，非对话框 ↓ 回卷）"
        );
        assert_eq!(
            cycle_next("opencode", MamMode::Default),
            Some(MamMode::Plan)
        );
        assert_eq!(
            cycle_next("opencode", MamMode::Plan),
            Some(MamMode::Default)
        );
        // 非单轴组 / 未实测工具 → None
        assert_eq!(cycle_next("codex", MamMode::Plan), None);
        assert_eq!(cycle_next("zcode", MamMode::Plan), None);
    }

    /// 期望档：步进组用环序推算（当前档未知 → None）；直达组只有**有回读源**时才可预测
    #[test]
    fn expected_mode_rules() {
        // claude 步进：从 AcceptEdits 推 Plan
        assert_eq!(
            expected_mode_after(
                "claude",
                ModeGroupId::Mode,
                MamMode::Default,
                Some(MamMode::AcceptEdits)
            ),
            Some(MamMode::Plan),
            "目标档对步进组不参与预测——预测来自环序"
        );
        assert_eq!(
            expected_mode_after("claude", ModeGroupId::Mode, MamMode::Default, None),
            None,
            "当前档未知 → 无法预测（不造假 Mismatch）"
        );
        // codex `/plan`：直达 + 有回读源 → 期望 Plan
        assert_eq!(
            expected_mode_after(
                "codex",
                ModeGroupId::Mode,
                MamMode::Plan,
                Some(MamMode::Default)
            ),
            Some(MamMode::Plan)
        );
        // codex 权限组：直达但**无回读源** → None（端点如实说「请人工核对」）
        assert_eq!(
            expected_mode_after("codex", ModeGroupId::Permission, MamMode::Bypass, None),
            None
        );
        // kimi 权限组同样无回读源
        assert_eq!(
            expected_mode_after("kimi", ModeGroupId::Permission, MamMode::Bypass, None),
            None
        );
        // kimi 模式组有回读源
        assert_eq!(
            expected_mode_after(
                "kimi",
                ModeGroupId::Mode,
                MamMode::Default,
                Some(MamMode::Plan)
            ),
            Some(MamMode::Default)
        );
    }

    /// 回读对账三态（裁5「回读确认」）：命中 / 不符（如实报两边）/ 无法判定
    #[test]
    fn verify_mode_three_states() {
        assert_eq!(
            verify_mode_switch(Some(MamMode::Plan), Some(MamMode::Plan)),
            ModeVerify::Confirmed
        );
        assert_eq!(
            verify_mode_switch(Some(MamMode::Plan), Some(MamMode::Default)),
            ModeVerify::Mismatch {
                expected: MamMode::Plan,
                observed: MamMode::Default
            },
            "回读与预期不符必须如实报两边（不假装成功）"
        );
        assert_eq!(
            verify_mode_switch(None, Some(MamMode::Plan)),
            ModeVerify::Unverifiable,
            "无回读源/无法预测 → 不可判定"
        );
        assert_eq!(
            verify_mode_switch(Some(MamMode::Plan), None),
            ModeVerify::Unverifiable,
            "屏读失败 → 不可判定"
        );
    }

    // ==== 屏读解析（分族；夹具 = 真机屏幕原文）====

    /// claude：四档底栏逐字快照（T6 探测档案 §1.2 与 `screen-t6-claude-*` 原文）
    #[test]
    fn parse_claude_real_footers() {
        assert_eq!(
            parse_mode_from_screen(
                "claude",
                &lines(&["  ⏵⏵ accept edits on (shift+tab to cycle) · ← for agents"])
            ),
            Some(MamMode::AcceptEdits)
        );
        assert_eq!(
            parse_mode_from_screen(
                "claude",
                &lines(&["  ⏸ plan mode on (shift+tab to cycle) ·  for agents"])
            ),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen(
                "claude",
                &lines(&["  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents"])
            ),
            Some(MamMode::Bypass),
            "happy 的 auto 归 MAM 的 Bypass（调研档案 §3）"
        );
        assert_eq!(
            parse_mode_from_screen(
                "claude",
                &lines(&["  ⏸ manual mode on · ? for shortuts ·←for agents"])
            ),
            Some(MamMode::Default)
        );
        // 底栏在正文之下：自底向上取最靠下的命中
        assert_eq!(
            parse_mode_from_screen(
                "claude",
                &lines(&[
                    "  ⏸ plan mode on (shift+tab to cycle)", // 正文引用的旧状态
                    "  some body text",
                    "  ⏵⏵ accept edits on (shift+tab to cycle) · ← for agents",
                ])
            ),
            Some(MamMode::AcceptEdits),
        );
    }

    /// claude 的**计划批准框在场**时不得被读成「当前 Plan 档」——屏上是
    /// `No, stay in Plan mode`（无 `on`），且底栏已被对话框顶掉 → 未知（如实）
    #[test]
    fn claude_dialog_does_not_fake_plan_mode() {
        let screen = lines(&[
            " Claude has written up a plan and is ready to execute. Would you like to proceed?",
            "",
            " ❯ 1. Yes, and use auto mode",
            "   2. Yes, manually approve edits",
            "   3. No, stay in Plan mode",
            "      shift+tab to approve with this feedback",
        ]);
        assert_eq!(
            parse_mode_from_screen("claude", &screen),
            None,
            "对话框在场 = 底栏不可见 → 档未知（不得把选项文本当当前档）"
        );
    }

    /// opencode：`Build`/`Plan` 双档真机快照（5/5 校准的形态）
    #[test]
    fn parse_opencode_real_footers() {
        assert_eq!(
            parse_mode_from_screen(
                "opencode",
                &lines(&["  Build   DeepSeek V4.1 Flash Ollama Cloud ·high"])
            ),
            Some(MamMode::Default)
        );
        assert_eq!(
            parse_mode_from_screen(
                "opencode",
                &lines(&["  Plan   DeepSeek V4.1 Flash Ollama Cloud   high"])
            ),
            Some(MamMode::Plan)
        );
        // 模型名/路径不得误命中（`Planner`/`planning` 不是独立词）
        assert_eq!(
            parse_mode_from_screen("opencode", &lines(&["  Planner agent started"])),
            None
        );
        assert_eq!(
            parse_mode_from_screen("opencode", &lines(&["  planning the implementation"])),
            None
        );
    }

    /// kimi：`plan` 前缀 → Plan；**认出底栏但无前缀 → Default（缺席推断）**
    #[test]
    fn parse_kimi_real_footers() {
        assert_eq!(
            parse_mode_from_screen(
                "kimi",
                &lines(&[
                    " plan  GLM-5.3-Flash thinking: high  C:\\Users\\u\\AppData\\Local\\Temp\\proj-kimi",
                ])
            ),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen(
                "kimi",
                &lines(&[
                    " GLM-5.3-Flash thinking: high  C:\\Users\\u\\AppData\\Local\\Temp\\proj-kimi   ctrl+o expand",
                ])
            ),
            Some(MamMode::Default),
            "缺席推断：底栏在、无 plan 前缀 ⇒ 默认档（§2.6 表末）"
        );
        // 认不出底栏（无 `thinking:` 锚）→ 未知（不把没读到的屏说成 Default）
        assert_eq!(
            parse_mode_from_screen("kimi", &lines(&["  一些普通输出", "  > "])),
            None
        );
        // 计划文件路径里含 plan 字样但底栏无前缀：仍按底栏判 Default（路径不是判据）
        assert_eq!(
            parse_mode_from_screen(
                "kimi",
                &lines(&[
                    " C:/Users/u/.kimi-code/.../plans/ms-marvel.md",
                    " GLM-5.3-Flash thinking: high  C:\\proj-kimi",
                ])
            ),
            Some(MamMode::Default)
        );
    }

    /// codex：`Plan mode` 短语 → Plan；` · ` 状态栏在场无 Plan mode → Default（缺席推断）
    #[test]
    fn parse_codex_real_footers() {
        assert_eq!(
            parse_mode_from_screen(
                "codex",
                &lines(&[
                    "  glm-5.3-flash medium · ~\\AppData\\Local\\Temp\\proj-codex  Plan mode (shift+tab to cycle)",
                ])
            ),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen(
                "codex",
                &lines(&["  glm-5.3-flash medium · ~\\AppData\\Local\\Temp\\proj-codex"])
            ),
            Some(MamMode::Default),
            "缺席推断：状态栏在、无 Plan mode ⇒ 默认档"
        );
        // 底栏在正文之下：自底向上取最靠下的命中（实现期在 Plan，正文提过 default）
        assert_eq!(
            parse_mode_from_screen(
                "codex",
                &lines(&[
                    "  计划如下：",
                    "  1. 检查 main.py。",
                    "  glm-5.3-flash medium · ~\\proj-codex  Plan mode (shift+tab to cycle)",
                ])
            ),
            Some(MamMode::Plan)
        );
        // 认不出状态栏 → 未知
        assert_eq!(
            parse_mode_from_screen(
                "codex",
                &lines(&["  Press enter to confirm or esc to go back"])
            ),
            None
        );
        // `Planning` / 路径里的 plan 不算（只认 `plan mode` 短语与 ` · ` 状态栏）
        assert_eq!(
            parse_mode_from_screen("codex", &lines(&["  Planning next steps"])),
            None
        );
    }

    /// **分族的意义**（回归锁）：各家的判据只认自家的屏形态——两家的底栏都带
    /// `(shift+tab to cycle) · `，区别只在 `plan mode on`（claude）vs `Plan mode`（codex）。
    /// 还原动作：把 codex 族的「跳过 `plan mode on`」一行删掉 → 第一条断言先红
    /// （claude 的底栏会被 codex 解析器读成 Plan）。
    #[test]
    fn family_parsers_do_not_bleed_into_each_other() {
        let claude_plan_screen = lines(&["  ⏸ plan mode on (shift+tab to cycle) ·  for agents"]);
        assert_eq!(
            parse_mode_from_screen("claude", &claude_plan_screen),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen("codex", &claude_plan_screen),
            None,
            "claude 的底栏形态不是 codex 的判据（分族解析）"
        );
        let codex_screen =
            lines(&["  glm-5.3-flash medium · ~\\proj-codex  Plan mode (shift+tab to cycle)"]);
        assert_eq!(
            parse_mode_from_screen("codex", &codex_screen),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen("claude", &codex_screen),
            None,
            "codex 的底栏形态不是 claude 的判据"
        );
        // kimi 的底栏喂给 codex：`plan` 是首词（不是 `plan mode`），且 kimi 底栏无
        // ` · `（实测两态都没有）→ codex 族读不出结论
        let kimi_screen = lines(&[" plan  GLM-5.3-Flash thinking: high  C:\\proj-kimi"]);
        assert_eq!(
            parse_mode_from_screen("kimi", &kimi_screen),
            Some(MamMode::Plan)
        );
        assert_eq!(
            parse_mode_from_screen("codex", &kimi_screen),
            None,
            "kimi 的底栏形态不是 codex 的判据"
        );
        assert_eq!(
            parse_mode_from_screen("claude", &kimi_screen),
            None,
            "kimi 的底栏形态不是 claude 的判据"
        );
        // 未实测工具恒 None（旧实现会对任意工具跑词表）
        assert_eq!(parse_mode_from_screen("zcode", &claude_plan_screen), None);
        assert_eq!(parse_mode_from_screen("", &codex_screen), None);
    }

    // ==== 两段式第二段：菜单定位与导航 ====

    /// codex 权限菜单：行首标签 + 光标标记 → 编号表（缺失项天然不参与）
    #[test]
    fn locate_codex_menu_items() {
        let screen = lines(&[
            "  Update Model Permissions",
            "",
            "   Choose what Codex is allowed to do.",
            "",
            " › Read Only",
            "    Ask for approval",
            "    Approve for me",
            "    Full Access",
        ]);
        let items = locate_menu_items(&screen, menu_labels("codex")).expect("菜单必须解析");
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].number, 1);
        assert!(items[0].highlighted, "› 前缀 = 当前高亮（导航起点）");
        assert!(!items[1].highlighted);
    }

    /// **引文行不得当菜单项**（实机文案：Full Access 档带警示语，其中含
    /// `We strongly recommend selecting "Ask for approval" instead.`）——若按「包含」
    /// 匹配，菜单项数 +1 → 循环步进算错 → 选错档。
    #[test]
    fn quote_lines_are_not_menu_items() {
        let screen = lines(&[
            "  Update Model Permissions",
            " › Read Only",
            "    Ask for approval",
            "    Approve for me",
            "    Full Access",
            "    We strongly recommend selecting \"Ask for approval\" instead.",
        ]);
        let items = locate_menu_items(&screen, menu_labels("codex")).unwrap();
        assert_eq!(
            items.len(),
            4,
            "警示语（含 Ask for approval 字样但不以它开头）不得计入菜单项"
        );
    }

    /// 目标档不在菜单里（kimi 菜单没有 codex 的档）→ Err（不猜位置）
    #[test]
    fn menu_navigation_refuses_unknown_target() {
        let kimi_menu = lines(&[
            "  Select permission mode",
            " › Always Ask",
            "   Ask When Needed",
            "   Never Ask",
        ]);
        assert!(menu_navigation_sequence(&kimi_menu, "kimi", MamMode::Default).is_ok());
        assert!(
            menu_navigation_sequence(&kimi_menu, "kimi", MamMode::ReadOnly).is_err(),
            "kimi 菜单里没有只读档 → 拒绝"
        );
    }

    /// **导航从高亮位按方向走最少步**（方向感知变体，不假设菜单回卷）：
    /// codex 实机默认高亮在 `Ask for approval`（第 2 项）→ 选 Read Only 是 ↑ 一步
    /// （M9R 实机即此序列）。
    #[test]
    fn menu_navigation_uses_real_highlight() {
        let screen = lines(&[
            "  Update Model Permissions",
            "    Read Only",
            " › Ask for approval",
            "    Approve for me",
            "    Full Access",
        ]);
        assert_eq!(
            menu_navigation_sequence(&screen, "codex", MamMode::ReadOnly).unwrap(),
            vec!["up", "enter"],
            "高亮在第 2 项、目标第 1 项 → ↑×1 + Enter（M9R 实机即此序列）"
        );
        assert_eq!(
            menu_navigation_sequence(&screen, "codex", MamMode::Bypass).unwrap(),
            vec!["down", "down", "enter"],
            "高亮第 2 项、目标第 4 项 → ↓×2 + Enter"
        );
        // 高亮已在目标行 → 直接 Enter
        let on_target = lines(&[
            "  Update Model Permissions",
            "    Read Only",
            " › Ask for approval",
            "    Full Access",
        ]);
        assert_eq!(
            menu_navigation_sequence(&on_target, "codex", MamMode::Default).unwrap(),
            vec!["enter"],
            "高亮已在目标项 → 零步进直接提交"
        );
    }

    /// 无高亮 / 多高亮 / 菜单没出现 → 一律 Err（**不盲发方向键**）
    #[test]
    fn menu_navigation_refuses_without_anchor() {
        let no_hl = lines(&[
            "  Update Model Permissions",
            "    Read Only",
            "    Ask for approval",
            "    Full Access",
        ]);
        assert!(
            menu_navigation_sequence(&no_hl, "codex", MamMode::Bypass).is_err(),
            "无高亮 → 不猜起点（猜错会选错档）"
        );
        let multi_hl = lines(&[" › Read Only", " › Ask for approval", "    Full Access"]);
        assert!(menu_navigation_sequence(&multi_hl, "codex", MamMode::Bypass).is_err());
        let not_a_menu = lines(&["  › Ask Codex to do anything", "  some output"]);
        assert!(
            menu_navigation_sequence(&not_a_menu, "codex", MamMode::Bypass).is_err(),
            "菜单未出现 → 拒绝（端点如实回执）"
        );
        // 只有一项也不成菜单（与 dialog 解析器同下界）
        let one = lines(&["  Update Model Permissions", " › Full Access"]);
        assert!(menu_navigation_sequence(&one, "codex", MamMode::Bypass).is_err());
    }

    /// 未实测工具没有菜单词表 → 恒 Err（不出手）
    #[test]
    fn menu_labels_empty_for_untested_tools() {
        assert!(menu_labels("zcode").is_empty());
        assert_eq!(menu_target_label("zcode", MamMode::Bypass), None);
        assert!(menu_navigation_sequence(
            &lines(&[" › Full Access", "   Read Only"]),
            "zcode",
            MamMode::Bypass
        )
        .is_err());
    }

    // ==== T4 复评 C1：非 ASCII 真机屏不得 panic（根因防线）====

    /// **C1 根因防线**（T4 复评 Critical）：`starts_with_ignore_case` 的字节切片版本
    /// 会在「前 N 字节跨非 ASCII 字符」时 panic，而**真机屏幕每一份都能凑出这种行**。
    ///
    /// 夹具 = 批次丙 T6 探测档案的真机屏幕原文（`%TEMP%\mam-probe-c3-20260921-150000\
    /// evidence\` 的逐字抄录，去 `\r`），含三类非 ASCII：`—`(U+2014，3 字节)、
    /// `╭─`(U+256D/U+2500，3 字节)、中文（3 字节/字）。**逐行喂进两条真入口**
    /// （`locate_menu_items` 与 `menu_navigation_sequence`）——断言「不 panic」而不是
    /// 「不编译」：旧实现会在**这些行上真的 panic**（见下条测试的变异说明）。
    ///
    /// **为什么这条测试必须存在**（本批第三次因「夹具不真机」栽跟头）：旧夹具全是纯
    /// ASCII，而 C1 的触发条件恰恰是「非 ASCII 前导跨过前缀长度」——纯 ASCII 夹具把
    /// 实机形态**排除在门禁之外**，于是 `#[cfg(windows)]` 之外永不覆盖这条路径，
    /// panic 只在实机第二段注入时才现形（500 internal + 菜单开在终端没人收尾）。
    #[test]
    fn menu_locator_survives_real_non_ascii_screens() {
        // 真机屏幕原文（含三类非 ASCII；行号标注其出处，便于回溯）
        let real: Vec<&str> = vec![
            // screen-t6-claude-before.txt:14（`—` U+2014 落在第 8..11 字节——C1 的
            // 原始触发行；前缀 "Read Only" 的 9 字节切点正落在 `—` 内部）
            "  hello() —is complete and verified (python main.py printed hi).",
            // screen-codex-after-enter.txt:1（`╭─` 边框行；"Full Access" 的 11 字节切点
            // 落在第二个 `─` 内）
            "\u{feff}╭────────────────────────────────────────────────────────╮",
            // screen-codex-after-st1.txt:2（中文正文；"Read Only" 的切点落在「如」内）
            "  在 main.py 中新增一个 hello() 函数，返回 \"Hello, World!\"，不改动现有代码和其他文件。",
            // screen-kimi-after-enter-confirm.txt:23（`└─` 边框）
            "   └──────────────────────────────────────────────────────────────────────",
            // screen-codex-planmd.txt:15（`·` 分隔的状态栏）
            "  glm-5.3-flash medium · ~\\AppData\\Local\\Temp\\mam-probe-c3-20260921-150000\\proj-codex",
            // screen-kimi-after-st.txt:29（`plan` 前缀底栏——本行**应当**被解析，见下）
            " plan  GLM-5.3-Flash thinking: high  C:\\Users\\bunny\\AppData\\Local\\Temp\\proj-kimi",
            // 纯 ASCII 短行（前缀比行长的边界格）
            "hi",
            "",
        ];
        let screen: Vec<String> = real.iter().map(|s| s.to_string()).collect();
        // ① 定位器：不得 panic（旧实现在前四行上 panic）
        for (tool, target) in [
            ("codex", MamMode::ReadOnly),
            ("codex", MamMode::Default),
            ("codex", MamMode::Bypass),
            ("kimi", MamMode::Default),
            ("kimi", MamMode::AcceptEdits),
            ("kimi", MamMode::Bypass),
        ] {
            // 这些屏上没有完整菜单 → 期望 Err（**如实中止**），但**绝不能 panic**
            let r = menu_navigation_sequence(&screen, tool, target);
            assert!(
                r.is_err(),
                "真机非 ASCII 屏上没有完整菜单 → 必须如实中止（{tool}/{target:?}）：{r:?}"
            );
        }
        // ② 直接驱动前缀匹配（最小判据）：全部标签 × 全部分行都不得 panic
        for line in &screen {
            let (rest, _) = crate::inject::dialog::strip_cursor_marker(line);
            let text = rest.trim();
            for label in menu_labels("codex").iter().chain(menu_labels("kimi")) {
                let _ = starts_with_ignore_case(text, label);
            }
            // 也过一遍逐行定位（其内部就是前缀匹配的批量形态）
            let _ = locate_menu_items(&screen_of_one(text), menu_labels("codex"));
        }
    }

    fn screen_of_one(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    /// **C1 的语义回归**：改成字符比较后，「大小写不敏感的前缀匹配」语义必须**不变**
    /// ——若有人把它换成「包含」（语义放宽），本测试先红。
    #[test]
    fn prefix_match_still_means_prefix_not_contains() {
        assert!(starts_with_ignore_case("Read Only (sandbox)", "Read Only"));
        assert!(
            starts_with_ignore_case("read only", "Read Only"),
            "大小写不敏感"
        );
        assert!(
            !starts_with_ignore_case("   Read Only", "Read Only"),
            "前导空白必须由调用方 trim（本函数不做 trim——旧语义如此）"
        );
        assert!(
            !starts_with_ignore_case(
                "We strongly recommend selecting \"Ask for approval\" instead.",
                "Ask for approval"
            ),
            "引文行不是前缀命中（行首匹配的全部意义）"
        );
        // 非 ASCII 前缀（当前词表没有，但语义要成立）
        assert!(starts_with_ignore_case("只读模式", "只读"));
        assert!(!starts_with_ignore_case("模式只读", "只读"));
        // 短行 / 空行：前缀比行长 → false（不 panic）
        assert!(!starts_with_ignore_case("hi", "Read Only"));
        assert!(!starts_with_ignore_case("", "Read Only"));
    }

    // ==== T4 复评 I1：菜单一致性闸（评审反例的可执行锁）====

    /// **评审反例的可执行锁**（T4 复评 I1）：同屏残留一行以档位标签开头的正文
    /// （评审给的真机形态：codex `/status` 回显 `Read Only (sandbox: read-only)`）。
    ///
    /// **位置是判据的一部分**：混入行若在**高亮项之前或之后整体平移**，起点与目标
    /// 编号同幅移动 → 步进不变（无害）；只有落在**高亮项与目标之间**时，目标编号被
    /// 撑大而起点不动 → **步进偏大**（旧实现发出多一步的方向键：若菜单不回卷，多一步
    /// 会把高亮停在别的档 → 用户点 Full Access 而实际切到 Read Only）。
    /// 本夹具把残留行放在两行菜单之间（TUI overlay 下的背景正文行透出），正是那条
    /// 会真出错的位置。
    ///
    /// 新实现的收口：`Read Only` **标签重复** → [`menu_items_coherent`] 判不自洽 →
    /// `menu_navigation_sequence` Err → 端点中止第二段 + 如实回执（**不盲发方向键**）。
    /// **还原动作**：删掉 `menu_navigation_sequence` 里的 `menu_items_coherent` 调用
    /// → 本测试先红（得到被撑大的 `["down","down","enter"]`）。
    #[test]
    fn menu_gate_refuses_duplicated_label_from_bystander_line() {
        let screen = lines(&[
            "  Update Model Permissions",
            "    Read Only",
            " › Ask for approval",
            // 残留正文（`/status` 输出回显）：与菜单项 Read Only **重复**，
            // 且落在**高亮项与目标之间**（唯一会撑大错误步进的位置）
            "  Read Only (sandbox: read-only)",
            "    Full Access",
        ]);
        // 定位器本身仍会把它计进来（如实：定位器不做语义判断）
        let raw = locate_menu_items(&screen, menu_labels("codex")).unwrap();
        assert_eq!(
            raw.len(),
            4,
            "定位器如实报出 4 行（两道闸分工：定位 vs 判据）"
        );
        assert!(
            !menu_items_coherent(&raw, "codex"),
            "标签重复 → 判不自洽（评审反例的收口点）"
        );
        // **被撑大的错序**：手工按「不过闸」的路径算一遍（高亮在第 2 项、目标编号被
        // 残留行顶到第 4）——证明本夹具确实能证伪旧实现（正确步进是 ↓×1）
        let wrong = crate::inject::dialog::navigation_sequence_directional(&raw, 4).unwrap();
        assert_eq!(
            wrong,
            vec!["down", "down", "enter"],
            "残留行插在高亮与目标之间 → 步进被撑成 ↓×2（正确是 ↓×1）"
        );
        // 行动入口必须拒绝（而不是带着错项数算步进）
        let r = menu_navigation_sequence(&screen, "codex", MamMode::Bypass);
        assert!(
            r.is_err(),
            "混入行破坏一致性 → 必须中止（旧行为会发出被撑大的步进）：{r:?}"
        );
        // 干净菜单：同样目标得到正确的 ↓×1（证明闸没有误伤正常路径）
        let clean = lines(&[
            "  Update Model Permissions",
            "    Read Only",
            " › Ask for approval",
            "    Full Access",
        ]);
        assert_eq!(
            menu_navigation_sequence(&clean, "codex", MamMode::Bypass).unwrap(),
            vec!["down", "enter"],
            "干净菜单的步进不受闸影响（高亮在 Ask for approval、目标 Full Access）"
        );
    }

    /// 一致性闸的第二条不变式：**目标档全集齐备**——菜单被遮挡/只画出两档 → 中止
    /// （步进不可信）。同时锁住「`Approve for me` 不算进全集」（Guardian 关闭的正常
    /// 菜单不得被判不完整）。
    #[test]
    fn menu_gate_requires_all_target_tiers() {
        // 三档齐备（含 Approve for me 的 Guardian 形态）→ 通过
        let full = lines(&[
            "  Update Model Permissions",
            " › Read Only",
            "    Ask for approval",
            "    Approve for me",
            "    Full Access",
        ]);
        assert!(menu_navigation_sequence(&full, "codex", MamMode::Bypass).is_ok());
        // Guardian 关闭（无 Approve for me）→ **仍通过**（它是可选档，不算进全集）
        let no_guardian = lines(&[
            "  Update Model Permissions",
            " › Read Only",
            "    Ask for approval",
            "    Full Access",
        ]);
        assert!(
            menu_navigation_sequence(&no_guardian, "codex", MamMode::Bypass).is_ok(),
            "Approve for me 缺席不得判不完整（§2.6 未把它列为目标档）"
        );
        // 只画出两档（缺 Full Access）→ 中止
        let truncated = lines(&[
            "  Update Model Permissions",
            " › Read Only",
            "    Ask for approval",
        ]);
        assert!(
            menu_navigation_sequence(&truncated, "codex", MamMode::Bypass).is_err(),
            "目标档不全 → 不猜位置"
        );
        // kimi 三档同样要求（用 kimi 的词表）
        let kimi_ok = lines(&[
            "  Select permission mode",
            " › Always Ask",
            "    Ask When Needed",
            "    Never Ask",
        ]);
        assert_eq!(
            menu_navigation_sequence(&kimi_ok, "kimi", MamMode::Bypass).unwrap(),
            vec!["down", "down", "enter"]
        );
        let kimi_short = lines(&[
            "  Select permission mode",
            " › Always Ask",
            "    Ask When Needed",
        ]);
        assert!(
            menu_navigation_sequence(&kimi_short, "kimi", MamMode::Bypass).is_err(),
            "kimi 菜单缺永不询问 → 中止"
        );
    }

    /// 一致性闸的**残留风险**（如实申报的机器可验版本）：若混入行恰好补齐缺席的
    /// 目标档，闸门不会拦。本测试**把这个已知缺口钉在测试里**——它不是「期望行为」，
    /// 是「已知未覆盖面」的登记（下批块簇加固的验收点：本测试应从 err 变 ok 之外的
    /// 形态被替换）。
    #[test]
    fn menu_gate_known_gap_is_documented_not_hidden() {
        // 菜单只画出两档 + 正文恰好有一行以第三个目标标签开头 → 闸门通过（已知缺口）
        let screen = lines(&[
            "  Update Model Permissions",
            " › Ask for approval",
            "    Full Access",
            "  Read Only (sandbox: read-only)",
        ]);
        let raw = locate_menu_items(&screen, menu_labels("codex")).unwrap();
        assert_eq!(raw.len(), 3, "无重复标签（三个标签各一行）");
        assert!(
            menu_items_coherent(&raw, "codex"),
            "已知缺口：无重复 + 全集齐备 → 闸门放行（本测试登记该缺口，不假装已消除）"
        );
    }
}
