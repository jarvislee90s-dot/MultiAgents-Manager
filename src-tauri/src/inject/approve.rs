//! 审批映射表 + 选项解析纯核（W6）：红卡一键批准/拒绝的按键映射（W6 首批只做
//! 批准/拒绝两键位，「不要再问」等扩展键位待 Task 13 实测取证后回填追加）。
//!
//! ## 数据形态与来源
//! - 映射表是 `Vec<ToolMapping>` 的 serde JSON，SSOT 在 settings KV 键
//!   `inject.approve_map`（允许覆盖定制），缺省/损坏回退内置默认表
//!   [`DEFAULT_MAPPINGS_JSON`]（**不写回**——保持默认表可随版本升级）；
//! - 默认表 `verified_with` 一律 `"probe-pending"`：键位未经真实审批提示实测取证，
//!   Task 13/14 实测后回填真实版本号；[`is_version_drift`] 对 probe-pending 恒判
//!   漂移 → UI 提示「映射待实测确认，若提示不符请用普通发送」。
//!
//! ## 锁纪律（M4，调用点必须遵守）
//! [`load_mappings`] 内部读全局 DB KV（settings 表，`get_setting` 自取 `DB.lock()`），
//! **绝不在任何 DB 锁临界区内调用**——持 conn 引用的临界区内调用即自锁死锁。
//! 测试策略（零接触真实 ~/.mam）：KV 读取抽为纯内核 [`load_mappings_from`]，
//! 测试只驱动内核三态（None / 损坏 / 合法），不调 `get_setting` 不触真实 KV。

use serde::{Deserialize, Serialize};

/// 单个审批选项：`id` 语义标识（approve/reject，端点协议用）、`label` 展示文案、
/// `key` 按键（如 "1"/"esc"/"y"，经 Injector::locate_and_send_key 投递）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveOption {
    pub id: String,
    pub label: String,
    pub key: String,
}

/// 单工具映射：`tool` 小写工具标识（对齐 AgentType serde lowercase 形态）、
/// `verified_with` 映射取证时工具版本（"probe-pending" = 未取证）、
/// `prompt_markers` 审批提示词（小写 contains 任一命中即视为审批中）、`options` 键位表
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMapping {
    pub tool: String,
    pub verified_with: String,
    pub prompt_markers: Vec<String>,
    pub options: Vec<ApproveOption>,
}

/// settings KV 键（inject 域前缀，与其他 inject.* 键同域）
const KV_KEY: &str = "inject.approve_map";

/// probe-pending 哨兵：默认表键位未经实测取证（Task 13/14 回填真实版本号）
pub const PROBE_PENDING: &str = "probe-pending";

/// 默认映射（首批 Claude/Codex，裁决 14 由简到繁）。
/// **取证状态（Task 13，2026-09-18 Windows 本机实测）**：
/// - claude ✅ 已取证：真实审批提示原文「Do you want to create t13.txt? / 1. Yes /
///   2. Yes, and switch to accept edits …(shift+tab) / 3. No / Esc to cancel」
///   （claude-code 2.1.251，Write 工具触发；注入「1」实测批准生效——文件真实落盘，
///   证据 mam-probe evidence\T13-claude-*）。markers 命中验证通过，键位 approve="1"
///   （数字直选）、reject="esc"（Esc to cancel）。
/// - codex ⚠️ 未取证（probe-pending）：0.154.0 默认 auto 审批模式不产生原生审批框
///   （工作区写自动放行、越界写转对话式确认），原生键位无法触发；保持 pending，
///   drift 提示常驻，待 Mac 侧或后续版本取证。
const DEFAULT_MAPPINGS_JSON: &str = r#"[
 {"tool":"claude","verified_with":"2.1.251",
  "prompt_markers":["do you want","would you like","allow this","permission"],
  "options":[{"id":"approve","label":"允许","key":"1"},
             {"id":"reject","label":"拒绝","key":"esc"}]},
 {"tool":"codex","verified_with":"probe-pending",
  "prompt_markers":["approve","allow","run this command"],
  "options":[{"id":"approve","label":"允许","key":"y"},
             {"id":"reject","label":"拒绝","key":"n"}]}
]"#;
// 「不要再问」（claude 选项 2 等）不入首批表——Task 13 实测取证确认键位后追加

/// 生产薄壳：读 settings KV 后交内核解析。
/// 锁纪律（M4）：内部会取全局 DB 锁（get_setting），**绝不在任何 DB 锁临界区内调用**。
pub fn load_mappings() -> Vec<ToolMapping> {
    load_mappings_from(crate::database::dao::settings::get_setting(KV_KEY).as_deref())
}

/// 解析内核（纯函数，测试零接触 DB）：kv=None（缺省）/ 损坏 JSON → 默认表
/// （损坏 log::warn 一条，不写回——保持默认表可升级；缺省是首跑常态不告警）；
/// 合法 JSON → 原样解析结果（用户定制覆盖优先）
pub fn load_mappings_from(kv: Option<&str>) -> Vec<ToolMapping> {
    match kv {
        Some(s) => {
            match serde_json::from_str(s) {
                Ok(ms) => ms,
                Err(e) => {
                    log::warn!("inject.approve_map KV 损坏（{e}），回退内置默认表（不写回，保持默认表可升级）");
                    default_mappings()
                }
            }
        }
        None => default_mappings(),
    }
}

/// 内置默认表（常量反序列化，常量本身是测试夹具同源 SSOT，损坏属编程错误 panic）
fn default_mappings() -> Vec<ToolMapping> {
    serde_json::from_str(DEFAULT_MAPPINGS_JSON).expect("内置默认表必须是合法 JSON")
}

/// 审批提示检测（纯核）：last_message 小写化后 contains 任一 marker（marker 同步
/// 小写化，容忍 KV 定制表里的大小写混写）
pub fn detect(m: &ToolMapping, last_message: &str) -> bool {
    let lower = last_message.to_lowercase();
    m.prompt_markers
        .iter()
        .any(|mk| lower.contains(&mk.to_lowercase()))
}

/// 按 id 查选项（端点协议：optionId → 键位；未命中 = no_mapping 404）
pub fn option_by_id<'a>(m: &'a ToolMapping, id: &str) -> Option<&'a ApproveOption> {
    m.options.iter().find(|o| o.id == id)
}

/// 版本漂移判定（纯核）：verified_with 为 [`PROBE_PENDING`] 恒判漂移（未取证 →
/// UI 提示复核）；其余按 major.minor 比较（缺失段按 0）——patch 漂移不告警（保守，
/// 键位映射只随大版本/次版本语义变化的可能性低，压误报）
pub fn is_version_drift(verified_with: &str, current: &str) -> bool {
    if verified_with == PROBE_PENDING {
        return true;
    }
    let major_minor = |s: &str| -> (u64, u64) {
        let mut it = s.split('.');
        let major = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = it.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor)
    };
    major_minor(verified_with) != major_minor(current)
}

/// CLI 版本缓存（进程级单例）：cli → 探测结果（含 None，失败也缓存防每次请求刷进程）。
/// 锁内持有期间完成探测（首调代价一次性，双检都在锁内 = 不会对同一 cli 重复 spawn）；
/// 这是本模块自有锁，与全局 DB 锁无关。
static VERSION_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
> = std::sync::OnceLock::new();

/// CLI 版本探测（进程级缓存）：首次 spawn `<cli> --version` 取 stdout 首行首个含
/// 数字的 token（如 "2.1.251"）；失败缓存 None 不重试（避免每次探测刷进程）；
/// 后续命中缓存直接返回
pub fn cached_cli_version(cli: &str) -> Option<String> {
    let cache =
        VERSION_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock().unwrap();
    if let Some(hit) = map.get(cli) {
        return hit.clone();
    }
    let probed = probe_cli_version(cli);
    map.insert(cli.to_string(), probed.clone());
    probed
}

/// 单次探测（无缓存直查）：stdout 首行首个含数字的 token（"claude 2.1.251" →
/// "2.1.251"）；spawn 失败 / 无 stdout / 首行无数字 token 一律 None
fn probe_cli_version(cli: &str) -> Option<String> {
    let output = std::process::Command::new(cli)
        .arg("--version")
        .output()
        .ok()?;
    let first_line = String::from_utf8_lossy(&output.stdout);
    first_line
        .lines()
        .next()?
        .split_whitespace()
        .find(|t| t.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认表覆盖 claude/codex，各两项；W6 首批定死只做批准/拒绝
    #[test]
    fn default_mappings_cover_claude_codex() {
        // 零接触契约：走内核 None 路径（= 无 KV → 默认表），不调 get_setting 触真实 ~/.mam
        let ms = load_mappings_from(None);
        let claude = ms.iter().find(|m| m.tool == "claude").unwrap();
        assert_eq!(claude.options.len(), 2);
        assert!(ms.iter().any(|m| m.tool == "codex" && m.options.len() == 2));
        // 裁决 14/W6：首批只有 批准/拒绝 两键位
        for o in &claude.options {
            assert!(o.id == "approve" || o.id == "reject");
        }
        // Task 13 取证状态：claude 已实测回填（2.1.251，Windows 本机真实审批提示 +
        // 「1」键批准生效）；codex 原生审批框未触发保持 pending（drift 提示常驻）
        assert_eq!(claude.verified_with, "2.1.251");
        let codex = ms.iter().find(|m| m.tool == "codex").unwrap();
        assert_eq!(codex.verified_with, PROBE_PENDING);
        assert!(is_version_drift(&codex.verified_with, "0.154.0"));
        assert!(!is_version_drift(&claude.verified_with, "2.1.251"));
        assert!(!is_version_drift(&claude.verified_with, "2.1.252")); // patch 漂移不告警
        assert!(is_version_drift(&claude.verified_with, "2.2.0")); // minor 漂移告警
    }

    /// 内核三态：None → 默认表；损坏 JSON → 默认表（不 panic 不写回）；合法 → 原样解析
    #[test]
    fn load_mappings_from_three_states() {
        assert_eq!(load_mappings_from(None), default_fixture(), "缺省回默认表");
        assert_eq!(
            load_mappings_from(Some("{{{not-json")),
            default_fixture(),
            "损坏 JSON 回默认表"
        );
        let custom = r#"[{"tool":"kimi","verified_with":"1.0.0",
            "prompt_markers":["allow"],"options":[]}]"#;
        let ms = load_mappings_from(Some(custom));
        assert_eq!(ms.len(), 1, "合法 JSON 原样解析（用户定制覆盖）");
        assert_eq!(ms[0].tool, "kimi");
        assert!(
            !ms[0].verified_with.starts_with("probe"),
            "取证版本号可回填"
        );
    }

    /// detect 大小写无关：last_message 小写化后 contains 任一 marker
    #[test]
    fn detect_case_insensitive_marker() {
        let m = ToolMapping {
            tool: "claude".into(),
            verified_with: "probe-pending".into(),
            prompt_markers: vec!["do you want".into()],
            options: vec![],
        };
        assert!(detect(&m, "Do you want to proceed?"));
        assert!(!detect(&m, "无关文本"));
        // Task 13 取证原文回归：Windows 本机真实审批提示（claude 2.1.251 Write 工具）
        assert!(detect(&m, "Do you want to create t13.txt?"));
    }

    /// 漂移规则：probe-pending 恒漂移（未取证 → UI 提示复核）；
    /// major 漂移告警；patch 漂移不告警（保守）
    #[test]
    fn drift_rules() {
        assert!(is_version_drift("probe-pending", "1.2.3"));
        assert!(!is_version_drift("1.2.3", "1.2.3"));
        assert!(is_version_drift("1.2.3", "2.0.0"));
        assert!(!is_version_drift("1.2.3", "1.2.4")); // patch 漂移不告警（保守）
        assert!(
            is_version_drift("1.2", "1.3.0"),
            "minor 漂移告警（缺失段按 0 补齐）"
        );
    }

    /// 选项查找：命中返回 / 未命中 None
    #[test]
    fn option_lookup() {
        let m = ToolMapping {
            tool: "claude".into(),
            verified_with: "x".into(),
            prompt_markers: vec![],
            options: vec![ApproveOption {
                id: "approve".into(),
                label: "允许".into(),
                key: "1".into(),
            }],
        };
        assert!(option_by_id(&m, "approve").is_some());
        assert!(option_by_id(&m, "nope").is_none());
    }

    /// 默认表期望夹具（直接反序列化内置常量，与内核回退产物同源比对）
    fn default_fixture() -> Vec<ToolMapping> {
        serde_json::from_str(DEFAULT_MAPPINGS_JSON).unwrap()
    }
}
