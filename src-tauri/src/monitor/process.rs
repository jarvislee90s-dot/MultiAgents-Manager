// 进程发现 — sysinfo 扫描 + 孤儿/子 Agent 过滤
// 移植自 agent-sessions process/claude.rs，扩展支持 Codex CLI/APP 和 OpenCode

use crate::adapter::AgentProcess;
use crate::session::ProcessForm;
use log::{debug, trace, warn};
use sysinfo::{Pid, System};

/// 检查进程是否为孤儿（终端已关闭，shell 被 reparent 到 PID 1）
pub fn is_orphaned_process(system: &System, process: &sysinfo::Process) -> bool {
    let parent_pid = match process.parent() {
        Some(pid) => pid,
        None => return true,
    };
    if parent_pid.as_u32() == 1 {
        return true;
    }
    if let Some(parent_process) = system.process(parent_pid) {
        if let Some(grandparent_pid) = parent_process.parent() {
            if grandparent_pid.as_u32() == 1 {
                return true;
            }
        }
    } else {
        return true;
    }
    false
}

/// 归一化候选字符串：统一为 / 分隔、转小写，取 basename，去 Windows .exe 扩展名
/// （先小写再 strip，保证 "CLAUDE.EXE" 这类大写扩展名也能剥掉）
fn normalized_base(candidate: &str) -> String {
    let normalized = candidate.replace('\\', "/").to_lowercase();
    let base = normalized.rsplit('/').next().unwrap_or("");
    base.strip_suffix(".exe").unwrap_or(base).to_string()
}

/// 判断可执行文件路径 / 进程名 / argv[0] 是否匹配工具名列表（跨平台）
/// - Windows: "C:\\...\\codex.exe"、"codex.exe" 均匹配 "codex"
/// - Unix:    "/Applications/ChatGPT.app/Contents/Resources/codex"、"codex" 均匹配 "codex"
fn exe_matches(candidate: &str, process_names: &[&str]) -> bool {
    let base = normalized_base(candidate);
    !base.is_empty() && process_names.iter().any(|name| name.to_lowercase() == base)
}

/// 判断进程形态（CLI 还是 APP），依据命中候选（exe 路径 / 进程名 / argv[0]）的特征
/// - basename 首字母大写（如独立 Codex.app 的 "Codex"）→ APP
/// - 位于 macOS .app 包内（如 ChatGPT.app 内嵌的 codex app-server）→ APP
/// - 位于 Windows MSIX 安装目录（ChatGPT 合并版 Codex 桌面端）→ APP。
///   P2-6：`WindowsApps/<publisher>.<app>_<version>_...` 的版本段随升级变化，硬编码
///   `openai.codex_` 前缀会在 MSIX 版本升级后静默失效（误判 CLI 而非 APP）——
///   放宽为「路径位于 WindowsApps/ 且路径/basename 含 codex」双条件
fn classify_form(candidate: &str) -> ProcessForm {
    let normalized = candidate.replace('\\', "/");
    let base_stem = normalized.rsplit('/').next().unwrap_or("");
    let base_stem = base_stem.strip_suffix(".exe").unwrap_or(base_stem);
    // basename 首字母大写 → APP（保留原始大小写判断，不能先 lowercase）
    let exe_upper = base_stem
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    let lower = normalized.to_lowercase();
    let in_app_bundle = lower.contains(".app/contents");
    // MSIX：路径位于 WindowsApps 且含 codex（P2-6 去版本耦化）
    let in_msix = lower.contains("windowsapps/") && lower.contains("codex");
    if exe_upper || in_app_bundle || in_msix {
        ProcessForm::App
    } else {
        ProcessForm::Cli
    }
}

/// 通用进程发现：扫描指定进程名列表，过滤子 Agent 和孤儿
/// process_names[0] 是 CLI 名，后续可以是 APP 名
fn find_processes_by_names(
    system: &System,
    process_names: &[&str],
    our_app_names: &[&str],
) -> Vec<AgentProcess> {
    use std::collections::HashSet;

    // 对单个进程做匹配：依次尝试 exe 路径 > 进程名 > 命令行首参数，返回命中候选
    // （提权进程的命令行可能读不到，exe/name 仍可读，因此 exe/name 优先）
    fn match_candidate(process: &sysinfo::Process, names: &[&str]) -> Option<String> {
        if let Some(exe) = process.exe() {
            let exe_str = exe.to_string_lossy().to_string();
            if exe_matches(&exe_str, names) {
                return Some(exe_str);
            }
        }
        let name = process.name().to_string_lossy().to_string();
        if exe_matches(&name, names) {
            return Some(name);
        }
        if let Some(first) = process.cmd().first() {
            let first = first.to_string_lossy().to_string();
            if exe_matches(&first, names) {
                return Some(first);
            }
        }
        None
    }

    // 收集所有匹配的 PID（用于子 Agent 过滤）
    let matched_pids: HashSet<Pid> = system
        .processes()
        .iter()
        .filter(|(_, p)| match_candidate(p, process_names).is_some())
        .map(|(pid, _)| *pid)
        .collect();

    let mut processes = Vec::new();
    for (pid, process) in system.processes() {
        let Some(candidate) = match_candidate(process, process_names) else {
            continue;
        };

        // 排除自身应用
        let process_name = process.name().to_string_lossy();
        if our_app_names.iter().any(|&app| process_name.contains(app)) {
            trace!(
                "Skipping our own app: pid={}, name={}",
                pid.as_u32(),
                process_name
            );
            continue;
        }

        // 判断进程形态（CLI 还是 APP），依据命中候选的路径特征（Task 3 实现 classify_form）
        let form = if process_names.len() > 1 {
            classify_form(&candidate)
        } else {
            ProcessForm::Cli
        };

        let cwd = process.cwd().map(|p| p.to_path_buf());

        // 跳过子 Agent（父进程也是同工具进程）
        if let Some(parent_pid) = process.parent() {
            if matched_pids.contains(&parent_pid) {
                debug!(
                    "Skipping sub-agent: pid={}, parent={}",
                    pid.as_u32(),
                    parent_pid.as_u32()
                );
                continue;
            }
        }

        // 跳过孤儿进程（仅 CLI 形态检查 — APP 形态由 launchd / 系统启动是正常的）
        if matches!(form, ProcessForm::Cli) && is_orphaned_process(system, process) {
            warn!("Skipping orphaned CLI: pid={}, cwd={:?}", pid.as_u32(), cwd);
            continue;
        }

        debug!(
            "Found process: name={:?}, pid={}, cwd={:?}, cpu={:.1}%, form={:?}",
            process_name,
            pid.as_u32(),
            cwd,
            process.cpu_usage(),
            form
        );

        processes.push(AgentProcess {
            pid: pid.as_u32(),
            cpu_usage: process.cpu_usage(),
            cwd,
            exe: process.exe().map(|e| e.to_path_buf()),
            form,
        });
    }

    processes
}

/// 发现 Claude Code 进程
pub fn find_claude_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(
        system,
        &["claude"],
        &["multi-agents-manager", "agent-sessions"],
    )
}

/// 发现 Codex CLI + 桌面 APP 进程
pub fn find_codex_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["codex", "Codex"], &["multi-agents-manager"])
}

/// 发现 OpenCode 进程
pub fn find_opencode_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["opencode"], &["multi-agents-manager"])
}

/// 发现 OpenClaw 进程
pub fn find_openclaw_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["openclaw"], &["multi-agents-manager"])
}

/// 发现 Kimi Code 进程（主进程 kimi；kimi-code-worker 等子进程经父链过滤剔除）
pub fn find_kimi_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["kimi"], &["multi-agents-manager"])
}
// WorkBuddy 不在此处做进程名发现（P0-1）：会话进程发现已改为心跳目录驱动，
// 见 workbuddy_parser::discover_workbuddy_processes——Windows 上会话宿主与主进程同名
// WorkBuddy.exe，进程名匹配不可用，且父进程同名会被通用子代理过滤误杀

#[cfg(test)]
mod tests {
    mod exe_matches {
        use super::super::exe_matches;

        #[test]
        fn matches_bare_unix_name() {
            // 旧行为兼容：裸名（argv[0] 恰好是命令名）
            assert!(exe_matches("codex", &["codex", "Codex"]));
            assert!(exe_matches("claude", &["claude"]));
        }

        #[test]
        fn matches_unix_path_without_extension() {
            // 旧行为兼容：macOS 内嵌 codex app-server
            assert!(exe_matches(
                "/Applications/ChatGPT.app/Contents/Resources/codex",
                &["codex", "Codex"]
            ));
            assert!(exe_matches(
                "/Users/x/.cargo/bin/codex",
                &["codex", "Codex"]
            ));
        }

        #[test]
        fn matches_windows_path_with_backslash_and_exe() {
            // 本次修复的核心场景
            assert!(exe_matches(
                "C:\\Users\\bunny\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Anthropic.ClaudeCode_xxx\\claude.exe",
                &["claude"]
            ));
            assert!(exe_matches(
                "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.818.5229.0_x64__2p2nqsd0c76g0\\app\\codex.exe",
                &["codex", "Codex"]
            ));
        }

        #[test]
        fn matches_process_name_only() {
            // 提权进程命令行读不到时，只剩 process.name()
            assert!(exe_matches("codex.exe", &["codex", "Codex"]));
            assert!(exe_matches("CLAUDE.EXE", &["claude"]));
            assert!(exe_matches("Codex", &["codex", "Codex"]));
        }

        #[test]
        fn rejects_partial_and_unrelated_names() {
            // 防误伤：名字相近但 basename 不同的进程不得命中
            assert!(!exe_matches("codex-plus-plus.exe", &["codex", "Codex"]));
            assert!(!exe_matches("ChatGPT.exe", &["codex", "Codex"]));
            assert!(!exe_matches("node.exe", &["claude"]));
            assert!(!exe_matches("", &["codex"]));
        }
    }

    mod classify_form {
        use super::super::classify_form;
        use crate::session::ProcessForm;

        #[test]
        fn mac_standalone_capitalized_binary_is_app() {
            // 旧行为兼容：独立 Codex.app 的可执行文件首字母大写
            assert_eq!(
                classify_form("/Applications/Codex.app/Contents/MacOS/Codex"),
                ProcessForm::App
            );
        }

        #[test]
        fn mac_chatgpt_embedded_codex_is_app() {
            // 旧行为兼容（commit 41adeaa）：ChatGPT.app 内嵌 codex app-server
            assert_eq!(
                classify_form("/Applications/ChatGPT.app/Contents/Resources/codex"),
                ProcessForm::App
            );
        }

        #[test]
        fn windows_msix_codex_is_app() {
            // 本次新增：ChatGPT 合并版 Codex 桌面端（Windows MSIX 安装目录）
            assert_eq!(
                classify_form(
                    "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.818.5229.0_x64__2p2nqsd0c76g0\\app\\codex.exe"
                ),
                ProcessForm::App
            );
        }

        #[test]
        fn windows_msix_codex_any_version_is_app() {
            // P2-6 回归锁：MSIX 版本段随升级变化，不得硬编码 openai.codex_ 前缀——
            // 路径在 WindowsApps/ 且含 codex 即判 APP（版本升级后不静默失效）
            assert_eq!(
                classify_form(
                    "C:\\Program Files\\WindowsApps\\OpenAI.Codex_99.0.0.0_x64__2p2nqsd0c76g0\\app\\codex.exe"
                ),
                ProcessForm::App
            );
            assert_eq!(
                classify_form(
                    "C:\\Program Files\\WindowsApps\\openai.codex_1.2.3.4_x64__2p2nqsd0c76g0\\Codex.exe"
                ),
                ProcessForm::App
            );
        }

        #[test]
        fn windows_apps_non_codex_path_is_cli() {
            // P2-6 宽松化的边界：WindowsApps 下不含 codex 的路径（如其他应用自带的
            // 同名工具）不误判为 APP；WindowsApps 之外含 codex 的路径仍按 CLI 处理
            assert_eq!(
                classify_form(
                    "C:\\Program Files\\WindowsApps\\Microsoft.WindowsTerminal_1.0.0_x64__x\\wt.exe"
                ),
                ProcessForm::Cli
            );
            assert_eq!(
                classify_form("C:\\Users\\x\\.local\\bin\\codex.exe"),
                ProcessForm::Cli
            );
        }

        #[test]
        fn windows_and_unix_cli_paths_are_cli() {
            assert_eq!(
                classify_form("C:\\Users\\x\\.local\\bin\\claude.exe"),
                ProcessForm::Cli
            );
            assert_eq!(classify_form("/Users/x/.cargo/bin/codex"), ProcessForm::Cli);
            assert_eq!(classify_form("codex.exe"), ProcessForm::Cli);
        }
    }
}
