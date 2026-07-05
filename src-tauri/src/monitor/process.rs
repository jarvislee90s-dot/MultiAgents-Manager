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

/// 通用进程发现：扫描指定进程名列表，过滤子 Agent 和孤儿
/// process_names[0] 是 CLI 名，后续可以是 APP 名
fn find_processes_by_names(
    system: &System,
    process_names: &[&str],
    our_app_names: &[&str],
) -> Vec<AgentProcess> {
    use std::collections::HashSet;

    // 收集所有匹配的 PID（用于子 Agent 过滤）
    let mut matched_pids: HashSet<Pid> = HashSet::new();
    for (pid, process) in system.processes() {
        let cmd = process.cmd();
        if let Some(first_arg) = cmd.first() {
            let first = first_arg.to_string_lossy().to_lowercase();
            if process_names.iter().any(|&name| {
                let name_lower = name.to_lowercase();
                first == name_lower || first.ends_with(&format!("/{}", name_lower))
            }) {
                matched_pids.insert(*pid);
            }
        }
    }

    let mut processes = Vec::new();
    for (pid, process) in system.processes() {
        let cmd = process.cmd();
        let process_name = process.name().to_string_lossy();

        let first_arg = cmd.first();
        let is_match = first_arg.map(|arg| {
            let first = arg.to_string_lossy().to_lowercase();
            process_names.iter().any(|&name| {
                let name_lower = name.to_lowercase();
                first == name_lower || first.ends_with(&format!("/{}", name_lower))
            })
        }).unwrap_or(false);

        if !is_match {
            continue;
        }

        // 排除自身应用
        if our_app_names.iter().any(|&app| process_name.contains(app)) {
            trace!("Skipping our own app: pid={}, name={}", pid.as_u32(), process_name);
            continue;
        }

        // 判断进程形态（CLI 还是 APP）
        // APP 形态：可执行文件首字母大写（如 "Codex"），CLI 首字母小写（如 "codex"）
        // 例：CLI 路径 ~/.cargo/bin/codex → base "codex"；APP 路径 /Applications/Codex.app/Contents/MacOS/Codex → base "Codex"
        let form = if process_names.len() > 1 {
            // 多形态工具（如 Codex 有 CLI "codex" 和 APP "Codex"）
            let first = first_arg.unwrap().to_string_lossy();
            // 取可执行文件的 basename（去掉路径和扩展名）
            let exe_base = std::path::Path::new(first.as_ref())
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            // exe_base 第一个字符大写 → APP
            let is_app = exe_base.chars().next().map(|c| c.is_uppercase()).unwrap_or(false);
            if is_app { ProcessForm::App } else { ProcessForm::Cli }
        } else {
            ProcessForm::Cli
        };

        let cwd = process.cwd().map(|p| p.to_path_buf());

        // 跳过子 Agent（父进程也是同工具进程）
        if let Some(parent_pid) = process.parent() {
            if matched_pids.contains(&parent_pid) {
                debug!("Skipping sub-agent: pid={}, parent={}", pid.as_u32(), parent_pid.as_u32());
                continue;
            }
        }

        // 跳过孤儿进程（仅 CLI 形态检查 — APP 形态由 launchd 启动是正常的）
        if matches!(form, ProcessForm::Cli) && is_orphaned_process(system, process) {
            warn!("Skipping orphaned CLI: pid={}, cwd={:?}", pid.as_u32(), cwd);
            continue;
        }

        debug!(
            "Found process: name={:?}, pid={}, cwd={:?}, cpu={:.1}%, form={:?}",
            process_name, pid.as_u32(), cwd, process.cpu_usage(), form
        );

        processes.push(AgentProcess {
            pid: pid.as_u32(),
            cpu_usage: process.cpu_usage(),
            cwd,
            form,
        });
    }

    processes
}

/// 发现 Claude Code 进程
pub fn find_claude_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["claude"], &["multi-agents-manager", "agent-sessions"])
}

/// 发现 Codex CLI + 桌面 APP 进程
pub fn find_codex_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["codex", "Codex"], &["multi-agents-manager"])
}

/// 发现 OpenCode 进程
pub fn find_opencode_processes(system: &System) -> Vec<AgentProcess> {
    find_processes_by_names(system, &["opencode"], &["multi-agents-manager"])
}
