//! 新建会话状态机内核（spec §4）：C4 进程锚定段——起窗后按「目标目录 cwd + 新进程 +
//! start_time 新鲜度」发现 TUI 进程并记 pid（spec §4 第 3 步），后续屏读/按键/注入
//! 全部锚定该 pid，不受同项目多实例干扰（陈旧实例按 start_time 下界滤除）；C5 弹窗
//! 处置状态机段——信任框/更新框红线键序处置 + idle 门首句注入（spec §4 第 4 步），
//! 纯内核零 IO（屏读/按键/注入经 [`CreateDeps`] 缝，无真实 sleep），物化等待归 C6。

use crate::inject::anchor_ledger::{detect, scenario, slot};

/// TUI 进程名集合（m9r_e2e launch_cli 同口径：四家统一——原生 exe / node / bun；
/// kimi 无特判，`{tool}.exe` 即其原生进程名 kimi.exe）
fn tui_names(tool: &str) -> Vec<String> {
    vec![
        format!("{tool}.exe"),
        "node.exe".to_string(),
        "bun.exe".to_string(),
    ]
}

/// 候选判据（纯函数）：进程 cwd 与目标目录归一化后相等，且进程名命中该工具的 TUI
/// 进程名集合（大小写不敏感——Windows 进程名大小写随启动方式漂移；中间壳
/// cmd.exe / powershell.exe 天然不命中）。
fn is_tui_candidate(tool: &str, proc_cwd: &str, proc_name: &str, want_cwd: &str) -> bool {
    proc_cwd == want_cwd
        && tui_names(tool)
            .iter()
            .any(|n| n.eq_ignore_ascii_case(proc_name))
}

/// 陈旧过滤后取最新候选（C4 评审 Important 1 的纯核）：`start_time + 1 >=
/// not_before`（+1s 松量吸收 sysinfo 秒级分辨率——恰在入口前 1s 内启动的进程仍算
/// 新起）中 start_time 最大者；start_time 并列取更小 pid（HashMap 迭代序不定，
/// 结论必须确定）。无合格候选 → None（调用方继续轮询至超时）。
fn freshest_candidate(candidates: &[(u32, u64)], not_before: u64) -> Option<u32> {
    candidates
        .iter()
        .filter(|(_, st)| *st + 1 >= not_before)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(pid, _)| *pid)
}

/// 起窗后轮询发现 TUI pid（sysinfo cwd + 进程名 + start_time 新鲜度；超时 Err
/// 中文回执）。刷新口径与 resume 效果回查同源（with_cmd + with_cwd Always）：
/// sysinfo 0.32 两参 `refresh_processes` 的默认 `ProcessRefreshKind` 不刷 cwd/cmd，
/// 必须 specifics 显式刷（见 resume.rs `refresh_cmd_snapshot` 的实测登记；cwd 部分
/// 与 adapter 主扫描 / m9r_e2e find_wt_target 同款）。
///
/// 陈旧实例防误锚（C4 评审 Important 1）：入口记 `not_before`（UNIX epoch 秒），
/// 仅认 `start_time + 1 >= not_before` 的候选（见 [`freshest_candidate`]）——同目录
/// 陈旧同工具实例、起窗前已在跑的同名 node/bun 脚本一律滤除；无合格候选继续
/// 轮询至超时。
///
/// **调用方契约（C5 评审 I2，C6 必须遵守）**：必须在 resume_spawner 返回后**立即**
/// 调用——新鲜度下界以本函数入口为基准，起窗与轮询之间不得插入 >1s 的耗时步骤，
/// 否则新 TUI 自身会被误滤为陈旧实例（假超时）；若装配时序确实收不进 1s，调宽
/// [`freshest_candidate`] 的松量（单点可调，测试同步改）。
pub fn find_tui_pid(
    tool: &str,
    dir: &std::path::Path,
    timeout: std::time::Duration,
) -> Result<u32, String> {
    let want = crate::monitor::cwd::normalize_cwd_for_match(&dir.to_string_lossy());
    // 时钟早于纪元（异常态）→ 0：fail-open 不过滤，仅失去防误锚保护
    let not_before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let t0 = std::time::Instant::now();
    let mut cwd_self_checked = false;
    loop {
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new()
                .with_cmd(sysinfo::UpdateKind::Always)
                .with_cwd(sysinfo::UpdateKind::Always),
        );
        // 首轮一次性自检（C4 评审 Minor 3，resume.rs 先例）：自身进程 cwd 读不到 =
        // 刷新配置在本机失效，cwd 判据将恒假阴性——log::error 点名诊断，不改控制流
        if !cwd_self_checked {
            cwd_self_checked = true;
            let self_cwd_empty = sysinfo::get_current_pid()
                .ok()
                .and_then(|pid| system.process(pid))
                .and_then(|self_proc| self_proc.cwd())
                .map(|c| c.to_string_lossy().trim().is_empty())
                .unwrap_or(true);
            if self_cwd_empty {
                log::error!(
                    "进程 cwd 刷新为空（sysinfo 未读到自身进程 cwd），find_tui_pid 的 \
                     cwd 判据将恒假阴性——超时回执将不反映真实原因，请检查刷新配置"
                );
            }
        }
        // 收集全部判据命中者再取新鲜者——首个命中即返回会撞 HashMap 迭代序不定
        let mut candidates: Vec<(u32, u64)> = Vec::new();
        for (pid, process) in system.processes() {
            if let Some(cwd) = process.cwd() {
                if is_tui_candidate(
                    tool,
                    &crate::monitor::cwd::normalize_cwd_for_match(&cwd.to_string_lossy()),
                    &process.name().to_string_lossy(),
                    &want,
                ) {
                    candidates.push((pid.as_u32(), process.start_time()));
                }
            }
        }
        if let Some(pid) = freshest_candidate(&candidates, not_before) {
            return Ok(pid);
        }
        if t0.elapsed() >= timeout {
            return Err(format!(
                "起窗后 {timeout:?} 内未发现 {tool} 进程（目录 {}）；\
                 若终端窗口确已弹出且进程在跑，疑似被 start_time 新鲜度过滤\
                 （见函数 doc 调用方契约）",
                dir.display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1200));
    }
}

// ===== C5 弹窗处置状态机（spec §4 第 4 步；探测定案 §4-§7）=====

/// 新建会话状态机的状态列（C5 内核只产出 [`CreateStatus::WaitingMaterialize`] 与
/// [`CreateStatus::Failed`] 两态；`OpeningTerminal` / `DialogHandling` /
/// `InjectingFirst` / `Done` 由 C6 的起窗前置段与物化等待段产出——枚举一次列全，
/// C6 直接复用，不再扩列）。
#[derive(Debug, PartialEq, Eq)]
pub enum CreateStatus {
    /// 终端已起、TUI 尚未锚定（C6 起窗前置段）
    OpeningTerminal,
    /// 弹窗处置中（C6 任务 phase 归类用；内核失败前不单独暴露中间态）
    DialogHandling,
    /// 首句注入动作进行中（C6 消费）
    InjectingFirst,
    /// 首句已注入，等待会话物化落盘（C5 内核成功终态；物化等待归 C6）
    WaitingMaterialize,
    /// 会话已物化，携带回读到的 session_id（C6 物化等待产出）
    Done {
        /// 物化后的会话 id
        session_id: String,
    },
    /// 失败：`phase` = 失败所处阶段（dialog_handling / injecting_first），`code` =
    /// 失败码（机器可判），`message` = 中文回执（人可读）
    Failed {
        /// 失败所处阶段
        phase: &'static str,
        /// 失败码（network_wall / login_wall / onboard_wall / unrecognized_screen /
        /// key_failed / inject_failed）
        code: String,
        /// 中文回执
        message: String,
    },
}

/// 新建会话入参（起窗前置在 C6；本内核只消费 `tool` 与 `composed`）
pub struct Params {
    /// 工具 id（claude / codex / kimi / opencode，与 adapter 的 tool_id 同域）
    pub tool: String,
    /// 目标目录（C6 起窗前置段消费；本内核不读——pub 字段无 dead_code 告警，
    /// 同 C2 PathReject 先例）
    pub dir: std::path::PathBuf,
    /// 未加移动端标签的原始首句（C6 物化段消费；本内核不读）
    pub first_message: String,
    /// 实际注入输入框的首句（已带移动端标签）
    pub composed: String,
}

/// 管线结果：终态 + 全程实发键流水 + 弹窗处置留痕（`(场景, 实发键序)`；onboard
/// 失败留 `(create_onboard, [])` 空键序，未识别轮不留痕）
#[derive(Debug, PartialEq, Eq)]
pub struct CreateOutcome {
    /// 终态
    pub status: CreateStatus,
    /// 全程实发键的顺序流水（处置键 + 无他）
    pub keys_sent: Vec<String>,
    /// 弹窗处置留痕
    pub dialog_log: Vec<(String, Vec<String>)>,
}

/// 弹窗处置缝（C6 生产装配真屏读/真注入；测试用记录型假体）。pid 实参由缝闭包
/// 绑定真 pid（C4 [`find_tui_pid`] 的产物），内核一律传占位 0——pid 无关。
pub struct CreateDeps<'a> {
    /// 屏读：返回当前屏各行（原大小写）；None = 读屏失败（按未识别轮计）
    pub screen: &'a dyn Fn(u32) -> Option<Vec<String>>,
    /// 发键（键名 wire 词，如 "down" / "enter" / "2"）
    pub send_key: &'a dyn Fn(u32, &str) -> Result<(), String>,
    /// 注入文本（首句）
    pub send_text: &'a dyn Fn(u32, &str) -> Result<(), String>,
}

/// 屏读步距（spec §4.4 探测同口径 2s；**消费契约：C6 真缝闭包在每次屏读前 sleep
/// 本值**——内核零真实睡眠，步距语义由缝承载：主会话裁决，C5 报告登记；处置→idle
/// 的沉降亦由本步距近似覆盖）
pub const SCREEN_POLL_STEP_MS: u64 = 2000;
/// 处置键间隔（探测 P3a；**消费契约：C6 send_key 真缝闭包在发键前 sleep 本值**）
pub const KEY_GAP_MS: u64 = 1500;
/// 处置后沉降（探测 P3a 处置→idle 1–4.5s；**消费契约：由 C6 屏读闭包步距近似
/// 承载**——内核无独立沉降点）
pub const POST_DISPOSAL_SETTLE_MS: u64 = 2500;
/// 连续未识别轮上限（15 轮 × 2s = 30s 单阶段窗）
pub const MAX_MISSED_ROUNDS: usize = 15;
/// 单场景处置尝试上限（C5 评审 I1）：同屏反复在场 = 键可能未生效，无上限会导致
/// 无界发键且 C6 单飞额度被永久占用；超限按 `dialog_stuck` 失败终止
pub const MAX_DISPOSAL_ATTEMPTS_PER_SCENARIO: usize = 3;

/// 处置键序（2026-09-27 探测定案 §4/§5 红线；账本管「认屏」，键序是引擎域常量）：
/// - claude 信任框默认 ❯ No, exit（危险默认）——直按 Enter=退出，必须 ↓+Enter；
/// - codex 更新框默认 Update now（动全局 npm）——'2'=Skip（esc 无效、Enter 禁用）；
/// - codex/kimi 信任框默认即信任项，Enter 直通；其余（opencode 无信任框，占位不达）。
fn disposal_keys(tool: &str, sc: &str) -> Vec<&'static str> {
    match (tool, sc) {
        ("claude", s) if s == scenario::CREATE_TRUST => vec!["down", "enter"],
        ("codex", s) if s == scenario::CREATE_UPDATE => vec!["2"],
        _ => vec!["enter"],
    }
}

/// 首装不可处置态的专属失败码与中文回执（探测定案 §6；需真人，自动管线不发键）
fn onboard_failure(tool: &str) -> (&'static str, String) {
    match tool {
        "claude" => (
            "network_wall",
            "claude 首装网络墙（无法连接 Anthropic 服务），需真人处置——自动管线不发键终止"
                .to_string(),
        ),
        "codex" => (
            "login_wall",
            "codex 登录墙（需真人登录），需真人处置——自动管线不发键终止".to_string(),
        ),
        _ => (
            "onboard_wall",
            "新会话首装屏需真人初始化——自动管线不发键终止".to_string(),
        ),
    }
}

/// 当前屏是否「处置对象在场」：TITLE 与 CONFIRM **同屏在场**才算（缺一即未命中——
/// codex 残留更新横幅 TITLE-only、处置后残留标题正是此形态，不处置不盲打）。
/// 更新框先于信任框（探测定案：codex 启动即弹更新框，处置完才见信任框）。
/// 四家 CONFIRM 行齐备（claude 双选项 / codex 双形态 / kimi `❯ trust this folder`
/// 带 ❯ 标记锚定选项行——标题行是其超集子串，不带标记会使 TITLE 单独满足合取）。
fn disposal_scenario(lowered: &[String], tool: &str) -> Option<&'static str> {
    [scenario::CREATE_UPDATE, scenario::CREATE_TRUST]
        .iter()
        .copied()
        .find(|&sc| {
            detect(lowered, tool, sc, slot::TITLE).is_some()
                && detect(lowered, tool, sc, slot::CONFIRM).is_some()
        })
}

/// 「未识别界面」兜底失败（红线：不盲打——keys_sent 只含此前处置已发的键，
/// 未识别屏上一键未发）
fn fail_unrecognized(
    keys_sent: Vec<String>,
    dialog_log: Vec<(String, Vec<String>)>,
) -> CreateOutcome {
    CreateOutcome {
        status: CreateStatus::Failed {
            phase: "dialog_handling",
            code: "unrecognized_screen".to_string(),
            message: format!(
                "连续 {MAX_MISSED_ROUNDS} 轮未命中信任框/更新框/idle 任一已知锚\
                 （含读屏失败轮）——按「不盲打」红线终止，未对未识别界面发送任何键"
            ),
        },
        keys_sent,
        dialog_log,
    }
}

/// 新建会话管线主循环：读屏 → 认屏（账本 [`detect`]）→ 处置/注入。纯内核零 IO——
/// 屏读/按键/注入全部经 [`CreateDeps`] 缝调用，**无真实 sleep**（步距常量
/// [`SCREEN_POLL_STEP_MS`] 等由 C6 真缝闭包消费）。
///
/// 每屏优先级（spec §4 第 4 步）：
/// 1. `create_onboard` 命中 → 立即 [`CreateStatus::Failed`]（专属失败码，**不发键**）；
/// 2. `create_update` / `create_trust` 的 TITLE 与 CONFIRM 同屏在场 → 按
///    [`disposal_keys`] 红线键序处置，本屏消费完毕后继续下一轮读屏；
/// 3. `create_idle` PRESENT 命中 → 注入首句（**只发一次**）→
///    [`CreateStatus::WaitingMaterialize`] 返回（物化等待归 C6）；
/// 4. 都未命中（含「TITLE 在场、CONFIRM 不在场」半屏形态）→ 未识别轮 +1；
///    连续 [`MAX_MISSED_ROUNDS`] 轮（含屏读返回 None）→ unrecognized 失败。
pub fn run_pipeline(deps: &CreateDeps, p: &Params) -> CreateOutcome {
    let mut keys_sent: Vec<String> = Vec::new();
    let mut dialog_log: Vec<(String, Vec<String>)> = Vec::new();
    let mut missed = 0usize;
    let mut disposal_attempts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    loop {
        // pid 实参为占位 0：缝闭包负责绑定真 pid（C4 find_tui_pid 产物），内核 pid 无关
        let Some(screen) = (deps.screen)(0) else {
            missed += 1;
            if missed >= MAX_MISSED_ROUNDS {
                return fail_unrecognized(keys_sent, dialog_log);
            }
            continue;
        };
        let lowered: Vec<String> = screen.iter().map(|l| l.to_lowercase()).collect();
        // ① 首装不可处置态：识别即失败，不发键（dialog_log 留空键序）
        if detect(&lowered, &p.tool, scenario::CREATE_ONBOARD, slot::TITLE).is_some() {
            let (code, message) = onboard_failure(&p.tool);
            dialog_log.push((scenario::CREATE_ONBOARD.to_string(), Vec::new()));
            return CreateOutcome {
                status: CreateStatus::Failed {
                    phase: "dialog_handling",
                    code: code.to_string(),
                    message,
                },
                keys_sent,
                dialog_log,
            };
        }
        // ② 弹窗处置：TITLE 与 CONFIRM 同屏在场才动手（半屏形态 = 未命中不盲打）。
        // 单场景处置尝试上限（C5 评审 I1）：键不生效时同屏会反复出现——无上限则
        // 循环永不终止且 C6 单飞额度被永久占用，超限按 dialog_stuck 终止。
        if let Some(sc) = disposal_scenario(&lowered, &p.tool) {
            let count = disposal_attempts.entry(sc.to_string()).or_insert(0);
            *count += 1;
            if *count > MAX_DISPOSAL_ATTEMPTS_PER_SCENARIO {
                dialog_log.push((sc.to_string(), Vec::new()));
                return CreateOutcome {
                    status: CreateStatus::Failed {
                        phase: "dialog_handling",
                        code: "dialog_stuck".to_string(),
                        message: format!(
                            "同一弹窗（{sc}）处置 {MAX_DISPOSAL_ATTEMPTS_PER_SCENARIO} 次仍持续\
                             在场——按键可能未生效（失焦/被吞/屏未刷新），按红线终止（未清场）"
                        ),
                    },
                    keys_sent,
                    dialog_log,
                };
            }
            let keys: Vec<String> = disposal_keys(&p.tool, sc)
                .into_iter()
                .map(String::from)
                .collect();
            let mut round_sent: Vec<String> = Vec::new();
            for k in &keys {
                if let Err(e) = (deps.send_key)(0, k) {
                    // 留痕对称（C5 评审 M1）：失败轮也记已发前缀——
                    // 「dialog_log 键序拼接 ⊆ keys_sent」回查不变量保持
                    dialog_log.push((sc.to_string(), round_sent.clone()));
                    return CreateOutcome {
                        status: CreateStatus::Failed {
                            phase: "dialog_handling",
                            code: "key_failed".to_string(),
                            message: format!("处置键 {k} 发送失败：{e}"),
                        },
                        keys_sent,
                        dialog_log,
                    };
                }
                keys_sent.push(k.clone());
                round_sent.push(k.clone());
            }
            dialog_log.push((sc.to_string(), keys));
            missed = 0;
            continue; // 本屏消费完毕，继续下一轮读屏
        }
        // ③ idle 门：见到 idle 锚才注入首句（只发一次）→ 物化等待归 C6
        if detect(&lowered, &p.tool, scenario::CREATE_IDLE, slot::PRESENT).is_some() {
            if let Err(e) = (deps.send_text)(0, &p.composed) {
                return CreateOutcome {
                    status: CreateStatus::Failed {
                        phase: "injecting_first",
                        code: "inject_failed".to_string(),
                        message: format!("首句注入失败：{e}"),
                    },
                    keys_sent,
                    dialog_log,
                };
            }
            return CreateOutcome {
                status: CreateStatus::WaitingMaterialize,
                keys_sent,
                dialog_log,
            };
        }
        // ④ 未识别轮（含屏读 TITLE-only 半屏形态）
        missed += 1;
        if missed >= MAX_MISSED_ROUNDS {
            return fail_unrecognized(keys_sent, dialog_log);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::cwd::normalize_cwd_for_match as norm;
    #[test]
    fn tui_candidate_matches_cwd_and_name_case_insensitive() {
        assert!(is_tui_candidate(
            "claude",
            &norm(r"E:\proj\demo"),
            "claude.exe",
            &norm("e:/proj/demo")
        ));
        assert!(is_tui_candidate(
            "codex",
            &norm(r"E:\p"),
            "node.exe",
            &norm(r"E:\p")
        ));
        assert!(!is_tui_candidate(
            "claude",
            &norm(r"E:\other"),
            "claude.exe",
            &norm(r"E:\p")
        ));
        assert!(!is_tui_candidate(
            "kimi",
            &norm(r"E:\p"),
            "cmd.exe",
            &norm(r"E:\p")
        ));
        assert!(!is_tui_candidate(
            "kimi",
            &norm(r"E:\p"),
            "powershell.exe",
            &norm(r"E:\p")
        ));
        // 大小写不敏感的真锁（C4 评审 Minor 2）：Windows 进程名大小写随启动方式漂移
        assert!(is_tui_candidate(
            "claude",
            &norm(r"E:\p"),
            "CLAUDE.EXE",
            &norm(r"E:\p")
        ));
    }

    /// 陈旧实例滤除（C4 评审 Important 1 的纯核）：start_time 远早于 not_before 的
    /// 候选一律不合格 → None（调用方继续轮询至超时，不误锚旧实例）
    #[test]
    fn freshest_candidate_filters_stale_instances() {
        assert_eq!(freshest_candidate(&[(111, 100), (222, 105)], 1_000), None);
    }

    /// 最新者胜出 + +1s 松量（sysinfo 秒级分辨率）+ 并列取更小 pid（HashMap 迭代序
    /// 不定，结论必须确定）
    #[test]
    fn freshest_candidate_picks_latest_with_slack_and_stable_tiebreak() {
        // not_before=1_004：1_000 陈旧（+1=1_001<1_004）被滤；1_005 合格 → 222
        assert_eq!(
            freshest_candidate(&[(111, 1_000), (222, 1_005)], 1_004),
            Some(222)
        );
        // 恰在入口前 1s 内启动（start_time = not_before - 1）仍算新起
        assert_eq!(freshest_candidate(&[(333, 1_003)], 1_004), Some(333));
        // start_time 并列 → 更小 pid
        assert_eq!(
            freshest_candidate(&[(999, 1_005), (100, 1_005)], 1_004),
            Some(100)
        );
    }

    /// 空集 → None
    #[test]
    fn freshest_candidate_empty_yields_none() {
        assert_eq!(freshest_candidate(&[], 1_000), None);
    }
}

/// 管线主循环语义测试（断言逐字权威——键序红线 ×2、「不盲打」×2、onboard 专属码
/// ×2、idle 门、残留横幅不处置）。屏读假体按调用序弹屏，耗尽后恒 None（额外轮按
/// 「读屏失败 = 未识别」计）。
#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// 键/文假体的 fn 项桥：闭包带 `&str` 参数无法 HRTB 泛化成
    /// `dyn Fn(u32, &str)`（Box::new+leak 链上推断不出 for<'a>），fn 项天然满足。
    fn ok_key(_pid: u32, _k: &str) -> Result<(), String> {
        Ok(())
    }
    fn ok_text(_pid: u32, _t: &str) -> Result<(), String> {
        Ok(())
    }

    /// 屏读按调用序弹 screens（耗尽后恒 None）；键/文注入均成功假体——键序从
    /// outcome.keys_sent 断言。三缝 Box::leak 成 'static（测试进程一次性分配）。
    fn deps(screens: Vec<Vec<String>>) -> CreateDeps<'static> {
        let it: &'static RefCell<std::vec::IntoIter<Vec<String>>> =
            Box::leak(Box::new(RefCell::new(screens.into_iter())));
        let screen: &'static dyn Fn(u32) -> Option<Vec<String>> =
            Box::leak(Box::new(move |_pid| it.borrow_mut().next()));
        let send_key: &'static dyn Fn(u32, &str) -> Result<(), String> =
            Box::leak(Box::new(ok_key));
        let send_text: &'static dyn Fn(u32, &str) -> Result<(), String> =
            Box::leak(Box::new(ok_text));
        CreateDeps {
            screen,
            send_key,
            send_text,
        }
    }

    fn params(tool: &str) -> Params {
        Params {
            tool: tool.into(),
            dir: "C:\\t".into(),
            first_message: "hi".into(),
            composed: "hi [mobile test-dev]".into(),
        }
    }

    /// claude 信任框键序红线：默认项 ❯ No, exit（危险默认）——必须 ↓+Enter，
    /// 直按 Enter=退出 claude。
    #[test]
    fn claude_trust_requires_down_then_enter() {
        let d = deps(vec![
            vec![
                "Quick safety check: Is this a project you created or one you trust?".into(),
                "❯ No, exit".into(),
            ],
            vec!["Quick safety check".into()],
            vec!["manual mode on · ? for shortcuts".into()],
        ]);
        let out = run_pipeline(&d, &params("claude"));
        assert!(matches!(out.status, CreateStatus::WaitingMaterialize));
        assert_eq!(out.keys_sent, vec!["down", "enter"]);
        assert_eq!(
            out.dialog_log,
            vec![(
                scenario::CREATE_TRUST.to_string(),
                vec!["down".to_string(), "enter".to_string()]
            )]
        );
    }

    /// codex 更新框先于信任框：'2'=Skip（esc 无效、Enter 禁用）→ 信任框 Enter 直通。
    #[test]
    fn codex_update_then_trust_with_char2() {
        let d = deps(vec![
            vec![
                "Update available".into(),
                "1. Update now (runs npm install -g @openai/codex)".into(),
                "2. Skip".into(),
            ],
            vec!["Update available".into()],
            vec![
                "Folder access".into(),
                "Trust this folder?".into(),
                "1. Trust and continue".into(),
            ],
            vec!["Ask Codex to do anything".into()],
        ]);
        let out = run_pipeline(&d, &params("codex"));
        assert!(matches!(out.status, CreateStatus::WaitingMaterialize));
        assert_eq!(out.keys_sent, vec!["2", "enter"]);
        assert_eq!(
            out.dialog_log,
            vec![
                (scenario::CREATE_UPDATE.to_string(), vec!["2".to_string()]),
                (
                    scenario::CREATE_TRUST.to_string(),
                    vec!["enter".to_string()]
                )
            ]
        );
    }

    /// 未识别界面兜底：连续 15 轮未命中 → unrecognized_screen 失败，一键未发（不盲打）。
    #[test]
    fn unrecognized_screen_fails_without_any_key() {
        let d = deps(
            (0..16)
                .map(|_| vec!["某种未识别界面".to_string()])
                .collect(),
        );
        let out = run_pipeline(&d, &params("claude"));
        assert!(
            matches!(out.status, CreateStatus::Failed { code, .. } if code == "unrecognized_screen")
        );
        assert!(out.keys_sent.is_empty());
    }

    /// 首装不可处置态：识别即专属失败码、不发键（探测定案 §6）。
    #[test]
    fn onboard_screen_fails_with_specific_code() {
        let d = deps(
            (0..2)
                .map(|_| vec!["Unable to connect to Anthropic services".into()])
                .collect(),
        );
        let out = run_pipeline(&d, &params("claude"));
        assert!(matches!(out.status, CreateStatus::Failed { code, .. } if code == "network_wall"));

        let d2 = deps(
            (0..2)
                .map(|_| vec!["Sign in with ChatGPT".into()])
                .collect(),
        );
        let o2 = run_pipeline(&d2, &params("codex"));
        assert!(matches!(o2.status, CreateStatus::Failed { code, .. } if code == "login_wall"));
    }

    /// idle 门：卡在信任框（kimi 信任框标题在场但账本无 CONFIRM 行 → 不处置，防盲打
    /// Enter 猜测）→ 15 轮未识别 → failed，首句从未发出。
    #[test]
    fn first_message_blocked_until_idle_anchor() {
        static TEXT_SENT: AtomicBool = AtomicBool::new(false);
        // 复位观察位（C5 评审 M3）：static 为测试二进制级共享，先清零消除未来
        // 复用时的并行污染面
        TEXT_SENT.store(false, Ordering::SeqCst);
        /// send_text 假体（fn 项桥 + 函数内 static 观察位：同 HRTB 缘由）
        fn text_probe(_pid: u32, _t: &str) -> Result<(), String> {
            TEXT_SENT.store(true, Ordering::SeqCst);
            Ok(())
        }
        let it: &'static RefCell<std::vec::IntoIter<Vec<String>>> =
            Box::leak(Box::new(RefCell::new(
                (0..16)
                    .map(|_| vec!["Trust this folder?".to_string()])
                    .collect::<Vec<_>>()
                    .into_iter(),
            )));
        let screen: &'static dyn Fn(u32) -> Option<Vec<String>> =
            Box::leak(Box::new(move |_pid| it.borrow_mut().next()));
        let send_key: &'static dyn Fn(u32, &str) -> Result<(), String> =
            Box::leak(Box::new(ok_key));
        let send_text: &'static dyn Fn(u32, &str) -> Result<(), String> =
            Box::leak(Box::new(text_probe));
        let d = CreateDeps {
            screen,
            send_key,
            send_text,
        };
        let out = run_pipeline(&d, &params("kimi"));
        assert!(matches!(out.status, CreateStatus::Failed { .. }));
        assert!(
            !TEXT_SENT.load(Ordering::SeqCst),
            "idle 锚未现，首句不得发出"
        );
    }

    /// codex 残留更新横幅（TITLE-only，CONFIRM 已不在场）不构成处置对象、不挡 idle
    /// ——直接注入成功、零键（探测定案 §5：残留横幅不构成 idle 排除条件）。
    #[test]
    fn codex_residual_update_banner_does_not_block_idle() {
        let d = deps(vec![vec![
            "Update available".into(),
            "Ask Codex to do anything".into(),
        ]]);
        let out = run_pipeline(&d, &params("codex"));
        assert!(matches!(out.status, CreateStatus::WaitingMaterialize));
        assert!(out.keys_sent.is_empty());
    }

    /// 处置尝试上限（C5 评审 I1）：同一弹窗反复在场（键不生效的实机形态）最多处置
    /// 3 次 → dialog_stuck 终止——防无界发键与 C6 单飞额度永久占用。
    #[test]
    fn stuck_dialog_fails_after_max_attempts() {
        let screen = vec![
            "Update available".to_string(),
            "1. Update now (runs npm install -g @openai/codex)".to_string(),
            "2. Skip".to_string(),
        ];
        let d = deps((0..32).map(|_| screen.clone()).collect());
        let out = run_pipeline(&d, &params("codex"));
        assert!(matches!(
            out.status,
            CreateStatus::Failed { code, .. } if code == "dialog_stuck"
        ));
        assert_eq!(out.keys_sent, vec!["2", "2", "2"]); // 恰好 3 轮处置、各一发键
        assert_eq!(out.dialog_log.len(), 4); // 3 轮处置留痕 + 1 条超限空键序
    }
}
