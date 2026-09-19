//! 审批映射表 + 选项解析纯核（W6）：红卡一键批准/拒绝的按键映射（W6 首批只做
//! 批准/拒绝两键位，「不要再问」等扩展键位待 Task 13 实测取证后回填追加）。
//!
//! ## 数据形态与来源
//! - 映射表是 `Vec<ToolMapping>` 的 serde JSON，SSOT 在 settings KV 键
//!   [`KV_KEY`]（允许覆盖定制），缺省/损坏回退内置默认表
//!   [`DEFAULT_MAPPINGS_JSON`]（**不写回**——保持默认表可随版本升级）；
//! - 默认表 `verified_with` 均为实机取证版本（claude 2.1.251 Task 13/M8R 双场景、
//!   codex 0.154.0 M8R，Windows 本机）；KV 定制表若回填 `"probe-pending"` 触发
//!   [`is_version_drift`] 恒判漂移，且远端端点按**严格档**处理（M9R Task 10 裁决：
//!   未取证不出键——approve-options 不下发选项只给 [`PROBE_PENDING_REASON`] 提示，
//!   session-approve 按映射缺失 404，见 remote/api.rs）。边界：KV 定制映射填**非
//!   probe-pending 的任意版本号 = 用户自证**（键位自行担责），严格档不拦（W6
//!   「用户定制覆盖优先」既定口径，端点不做版本真伪校验）。
//!
//! ## 锁纪律（M4，调用点必须遵守）
//! KV 读取经调用方 `DeviceStore.with` 短临界区：[`load_mappings_conn`] 直用传入
//! conn（不自取 DB 锁）——临界区内只做这一条 SQL + 纯解析；**内部自取全局 DB 锁的
//! 调用（如 session_source）必须在 with 之外**（同线程嵌套加锁即自锁死锁，std Mutex
//! 非重入）。测试策略（零接触真实 ~/.mam）：解析抽为纯内核 [`load_mappings_from`]
//! （单测直驱三态 None/损坏/合法）；端点测试经 `DeviceStore::memory()` 自建库 seed
//! 定制 KV，不触真实 settings 表。

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
pub(crate) const KV_KEY: &str = "inject.approve_map";

/// probe-pending 哨兵：键位未经真实审批提示实测取证（实机取证后回填真实版本号）
pub const PROBE_PENDING: &str = "probe-pending";

/// 严格档降级原因（M9R Task 10 裁决：未取证不出键）：verified_with 为
/// [`PROBE_PENDING`] 时 approve-options 端点下发本原因 + available=false（前端
/// ApproveCard 契约：available=false 且带 reason → 只渲染提示条不渲染按键）
pub const PROBE_PENDING_REASON: &str = "键位待实测确认，请用普通发送";

/// 默认映射（首批 Claude/Codex，裁决 14 由简到繁）。
/// **取证状态（Task 13 + M8R Task 10，Windows 本机实机取证）**：
/// - claude ✅ 已取证（2.1.251，2026-09-18/09-19 双场景）：Write 工具审批（「Do you want
///   to create t13.txt? / 1. Yes / …」）注入「1」批准生效，文件真实落盘（Task 13，证据
///   mam-probe evidence\T13-claude-*）；plan 模式计划批准（「Claude has written up a plan
///   and is ready to execute. Would you like to proceed?」）注入「1」**数字直选单键即执行**
///   （plan-test.txt 落盘 + 状态栏转 auto mode on，M8R，证据 mam-probe-m6r
///   evidence\M8R-approve-claude-*）→ approve="1" 双场景通用，reject="esc"（Write 审批
///   "Esc to cancel"）；markers 增补 plan 批准标题短语 "would you like to proceed"。
/// - codex ✅ 已取证（0.154.0，2026-09-19，`/permissions` 切 Read Only 档后工作区写必弹
///   补丁审批）：弹框原文与源码快照逐字一致（"Would you like to make the following
///   edits? / › 1. Yes, proceed (y) / 2. … (a) / 3. No, and tell Codex what to do
///   differently (esc) / Press enter to confirm or esc to cancel"）；注入 y → hello.txt
///   落盘；注入 VK Esc → world.txt 未落盘 + 对话内拒绝记录（证据 mam-probe-m6r
///   evidence\M8R-approve-codex-*）→ approve="y"、reject="esc"；markers 换为源码
///   approval_overlay.rs 弹窗标题短语（窄匹配，替换旧 "approve"/"allow" 宽词）。
///
/// 实测差异照实记录：`/permissions` 实机档位序为 Read Only / Ask for approval /
/// Approve for me / Full Access（与手册 A1 快照序不同），当前高亮为
/// Ask for approval（非首项，选 Read Only 实注 ↑+Enter 而非 ↓+Enter）。
const DEFAULT_MAPPINGS_JSON: &str = r#"[
 {"tool":"claude","verified_with":"2.1.251",
  "prompt_markers":["do you want","would you like","would you like to proceed","allow this","permission"],
  "options":[{"id":"approve","label":"允许","key":"1"},
             {"id":"reject","label":"拒绝","key":"esc"}]},
 {"tool":"codex","verified_with":"0.154.0",
  "prompt_markers":["would you like to run the following","would you like to make the following","do you want to approve network"],
  "options":[{"id":"approve","label":"允许","key":"y"},
             {"id":"reject","label":"拒绝","key":"esc"}]}
]"#;
// 「不要再问」（codex 选项 2 "(a)"、claude shift+tab 放行等）不入首批表——扩展键位待
// 后续任务实测取证后追加；claude plan 批准框 reject 路径（Esc/选项3）未单独触发，
// reject="esc" 以 Task 13 Write 审批 "Esc to cancel" 实证为准（结论不超出证据）

/// KV 读取（store 缝版本，远端端点用）：直用调用方 `DeviceStore.with` 短临界区传入的
/// conn（不自取任何锁，锁内只做这一条 SQL + 纯解析）。生产 `DeviceStore::Global` 即
/// 全局 DB 同锁同连接，语义与直读 settings KV 完全一致；测试经 `DeviceStore::memory()`
/// 自建库 seed 定制 KV——零接触真实 ~/.mam（审计词表/严格档等端点测试的注入缝）。
pub fn load_mappings_conn(conn: &rusqlite::Connection) -> Vec<ToolMapping> {
    load_mappings_from(crate::database::dao::settings::get_setting_conn(conn, KV_KEY).as_deref())
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

/// CLI 版本缓存（进程级单例）：cli → 探测结果（含 None——失败与超时也缓存，防每次
/// 请求重刷进程/挂死 CLI）。锁内持有期间完成探测（首调代价一次性，双检都在锁内 =
/// 不会对同一 cli 重复 spawn）；这是本模块自有锁，与全局 DB 锁无关。披露：探测在锁内
/// 最多等 3s——并发首探**不同** cli 时在锁上串行排队（各至多 3s，仅首调；命中缓存后
/// 不再持锁探测），并发首探同一 cli 只 spawn 一次
static VERSION_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
> = std::sync::OnceLock::new();

/// CLI 版本探测（进程级缓存）：首次 spawn `<cli> --version` 取 stdout 首行首个含
/// 数字的 token（如 "2.1.251"）；任何失败（双路 spawn 失败 / 3s 超时 / 无 stdout /
/// 首行无数字 token）一律缓存 None 不重试（避免每次探测刷进程）；后续命中缓存直接返回
pub fn cached_cli_version(cli: &str) -> Option<String> {
    let cache =
        VERSION_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = map.get(cli) {
        return hit.clone();
    }
    let probed = probe_cli_version(cli);
    map.insert(cli.to_string(), probed.clone());
    probed
}

/// spawn `<cli> --version`（灰1 双路）：先裸名直 spawn（PATH 上的 .exe 命中即走最快
/// 路）；spawn 失败（NotFound 等）回退 `cmd /c <cli> --version`——npm 全局包在
/// Windows 的可执行是 `.cmd` 垫片，裸名直 spawn 必失败，须交由 cmd 按 PATH+PATHEXT
/// 解析（灰1 定案，批次设计 §3.R3-⑥）。stdin/stderr 一律 null（版本探测是单问一答：
/// 防 CLI 等 stdin 白耗超时窗，错误输出不读）
fn spawn_version_probe(cli: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new(cli)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .or_else(|_| {
            std::process::Command::new("cmd")
                .args(["/c", cli, "--version"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
        })
}

/// 单次探测（无缓存直查，灰1 双保护）：
/// 1. **cmd 垫片回退**（见 [`spawn_version_probe`]）；
/// 2. **有界等待**：`wait_timeout` 3s——CLI 启动挂死不再无限等待；超时 `kill()` +
///    `wait()` 收尸后返回 None（[`cached_cli_version`] 把 None 一并入缓存：超时结果
///    不重试，防每次请求再刷一个挂死进程刷屏）。
///
/// 解析：stdout 首行首个含数字的 token（"claude 2.1.251" → "2.1.251"）；双路 spawn
/// 均失败 / 无 stdout / 首行无数字 token 一律 None。stdout 管道在拿到退出态后才读
/// （`--version` 输出远小于管道缓冲，先等后读无死锁风险；披露：输出超管道缓冲
/// ~64KB 的 CLI 会在我们 wait 时被写满阻塞而假性超时——结果有界（3s 杀掉）非死锁）
fn probe_cli_version(cli: &str) -> Option<String> {
    use wait_timeout::ChildExt;
    let mut child = spawn_version_probe(cli).ok()?;
    use std::io::Read;
    match child.wait_timeout(std::time::Duration::from_secs(3)).ok()? {
        // 正常退出：收 stdout 解析（子进程已退出，管道缓冲可安全排空）。字节级读取 +
        // from_utf8_lossy：中文 Windows ANSI/GBK 码页 CLI 输出可含非法 UTF-8 字节，
        // read_to_string 会 Err → 探测 None 且永久入缓存——lossy 与旧行为对齐（能提就提）
        Some(_) => {
            let mut raw = Vec::new();
            if let Some(pipe) = child.stdout.as_mut() {
                pipe.read_to_end(&mut raw).ok()?;
            }
            String::from_utf8_lossy(&raw)
                .lines()
                .next()?
                .split_whitespace()
                .find(|t| t.chars().any(|c| c.is_ascii_digit()))
                .map(str::to_string)
        }
        // 超时：杀进程 + 收尸（防僵尸），返回 None（缓存层负责「超时也入缓存」）
        None => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
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
        // Task 13 + M8R 取证状态：claude/codex 均已实测回填（codex 见
        // codex_markers_are_source_phrases 的 M8R 专项回归）
        assert_eq!(claude.verified_with, "2.1.251");
        let codex = ms.iter().find(|m| m.tool == "codex").unwrap();
        assert_eq!(codex.verified_with, "0.154.0");
        assert!(!is_version_drift(&claude.verified_with, "2.1.251"));
        assert!(!is_version_drift(&claude.verified_with, "2.1.252")); // patch 漂移不告警
        assert!(is_version_drift(&claude.verified_with, "2.2.0")); // minor 漂移告警
    }

    /// M8R 取证回归（Task 10，2026-09-19 Windows 本机实机）：codex 默认表 markers 必须是
    /// 源码 approval_overlay.rs 标题短语（源码 B2 快照 + 实机弹框抄录一致），verified_with
    /// 回填实机版本（probe-pending 解除），detect 命中实机抄录原文；键位 y/esc 为实机
    /// 落盘/未落盘双验证（证据 %USERPROFILE%\mam-probe-m6r\evidence\M8R-approve-codex-*）。
    #[test]
    fn codex_markers_are_source_phrases() {
        let ms = load_mappings_from(None);
        let codex = ms.iter().find(|m| m.tool == "codex").unwrap();
        assert_eq!(
            codex.prompt_markers,
            vec![
                "would you like to run the following",
                "would you like to make the following",
                "do you want to approve network",
            ],
            "codex markers 必须恰为源码弹窗标题三短语（窄匹配，替换旧宽词）"
        );
        // M8R 实测版本回填（codex-cli 0.154.0，Read Only 档补丁审批取证）；minor 漂移照告警
        assert_eq!(codex.verified_with, "0.154.0");
        assert!(!is_version_drift(&codex.verified_with, "0.154.0"));
        assert!(is_version_drift(&codex.verified_with, "0.155.0"));
        // detect 命中 M8R 实机抄录原文（补丁/命令/网络三形态标题）
        assert!(detect(codex, "Would you like to make the following edits?"));
        assert!(detect(
            codex,
            "Would you like to run the following command?"
        ));
        assert!(detect(
            codex,
            "Do you want to approve network access to \"example.com\"?"
        ));
        assert!(!detect(codex, "无关文本"));
        // M8R 键位验证：y 批准（hello.txt 落盘）/ esc 拒绝（world.txt 未落盘+对话内拒绝记录）
        assert_eq!(option_by_id(codex, "approve").unwrap().key, "y");
        assert_eq!(option_by_id(codex, "reject").unwrap().key, "esc");
    }

    /// M8R 取证回归（Task 10，2026-09-19）：claude markers 含 plan 模式计划批准标题短语
    /// （实机抄录「Claude has written up a plan and is ready to execute. Would you like to
    /// proceed?」）；数字直选「1」单键即执行批准（plan-test.txt 落盘实证，证据
    /// %USERPROFILE%\mam-probe-m6r\evidence\M8R-approve-claude-*）→ approve="1" 既有映射
    /// 双场景通用，无需增补独立 plan 选项。
    #[test]
    fn claude_plan_marker_added() {
        let ms = load_mappings_from(None);
        let claude = ms.iter().find(|m| m.tool == "claude").unwrap();
        assert!(
            claude
                .prompt_markers
                .iter()
                .any(|mk| mk == "would you like to proceed"),
            "claude markers 必须含 plan 模式批准标题短语"
        );
        // 命中 M8R 实机标题原文（完整句）；既有 Write 审批命中不回归（Task 13 原文）
        assert!(detect(
            claude,
            "Claude has written up a plan and is ready to execute. Would you like to proceed?"
        ));
        assert!(detect(claude, "Do you want to create t13.txt?"));
        // plan 批准键位与 Write 审批同为「1」（数字直选单键执行），reject 仍 esc
        assert_eq!(option_by_id(claude, "approve").unwrap().key, "1");
        assert_eq!(option_by_id(claude, "reject").unwrap().key, "esc");
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

    // ==== CLI 版本探测（灰1：cmd 垫片回退 + 3s 有界等待） ====
    // Windows-only：假 CLI 走 .cmd 垫片 + cmd.exe 解析（npm 全局包形态），POSIX 无此
    // 语义，Linux CI（cargo test 门禁）不编译不执行

    /// 探测测试专用串行锁：`std::env::set_var("PATH", …)` 是进程级突变，两条探测测试
    /// （以及任何并发解析 PATH 的代码）互染——两条测试全程持锁强制串行（对齐
    /// queue.rs LOOP_HANDLE_TEST_LOCK 先例）；每测收尾恢复原 PATH，断言失败也不泄漏
    /// 突变。锁自愈取锁（unwrap_or_else(into_inner)）：一测失败毒锁不得以 PoisonError
    /// 连坐另一测（假红噪声），让其照常跑出真实结论
    #[cfg(windows)]
    static PATH_TEST_LOCK: once_cell::sync::Lazy<std::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

    /// 把 dir 前插进程 PATH，返回原 PATH（供收尾恢复）。调用方持 [`PATH_TEST_LOCK`]。
    #[cfg(windows)]
    fn prepend_path(dir: &std::path::Path) -> std::ffi::OsString {
        let old = std::env::var_os("PATH").expect("Windows 下 PATH 必存在");
        let mut paths: Vec<_> = std::env::split_paths(&old).collect();
        paths.insert(0, dir.to_path_buf());
        std::env::set_var("PATH", std::env::join_paths(&paths).unwrap());
        old
    }

    /// 灰1 超时臂：PATH 上的 .cmd 假 CLI（ping 挂 29s）→ 探测必须 ≤5s 返回 None
    /// （内核 3s 有界等待 + kill 收尸，不再无限等待）；删文件复调仍 None = 不再 spawn
    /// （超时结果入缓存防重试刷屏；缓存命中的铁证见 probe_version_via_cmd_shim——
    /// 删文件复调仍返回版本号只有缓存能给）
    #[test]
    #[cfg(windows)]
    fn probe_version_times_out_and_caches_none() {
        let _serial = PATH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("mam-fake-hang-cli.cmd");
        std::fs::write(&shim, "@ping -n 30 127.0.0.1 >nul\r\n").unwrap();
        let old_path = prepend_path(dir.path());
        let started = std::time::Instant::now();
        let v = cached_cli_version("mam-fake-hang-cli");
        let elapsed = started.elapsed();
        // 收尾先恢复 PATH（断言失败也不把突变泄漏给并行测试）
        std::env::set_var("PATH", old_path);
        assert_eq!(v, None, "挂死 CLI 必须超时回 None");
        assert!(
            elapsed >= std::time::Duration::from_secs(2),
            "必须真的等满超时窗而非秒退（实际 {elapsed:?}）"
        );
        assert!(
            elapsed <= std::time::Duration::from_secs(5),
            "≤5s 返回（3s 超时 + 收尸余量），不得逼近无限等待（实际 {elapsed:?}）"
        );
        std::fs::remove_file(&shim).ok(); // 删文件；tempdir 收尾也会清，这里显式表达语义
        assert_eq!(
            cached_cli_version("mam-fake-hang-cli"),
            None,
            "删文件复调仍 None（缓存命中，不再 spawn）"
        );
    }

    /// 灰1 垫片臂：PATH 上的 .cmd 假 CLI → 裸名直 spawn 必失败 → 回退 cmd /c 垫片
    /// 解析出 9.9.9；删文件 + 还原 PATH 后复调仍 9.9.9（缓存命中铁证：重探只可能
    /// spawn 失败得 None，9.9.9 只能来自缓存）
    #[test]
    #[cfg(windows)]
    fn probe_version_via_cmd_shim() {
        let _serial = PATH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("mam-fake-fast-cli.cmd");
        std::fs::write(&shim, "@echo mam-fake-fast-cli 9.9.9\r\n").unwrap();
        let old_path = prepend_path(dir.path());
        let v = cached_cli_version("mam-fake-fast-cli");
        // 收尾：还原 PATH + 删文件，再做缓存断言（重探必败 → 9.9.9 只能来自缓存）
        std::env::set_var("PATH", old_path);
        std::fs::remove_file(&shim).ok();
        assert_eq!(v.as_deref(), Some("9.9.9"), "cmd 垫片回退必须解析出版本号");
        assert_eq!(
            cached_cli_version("mam-fake-fast-cli").as_deref(),
            Some("9.9.9"),
            "删文件复调仍 9.9.9 = 缓存命中（不再 spawn）"
        );
    }
}
