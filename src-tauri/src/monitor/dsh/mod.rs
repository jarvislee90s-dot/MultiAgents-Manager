// dsh（DeepSeek harness）监控解析 — M1 只读底座
// 会话存储/事件语义全部依据 M0 探测报告（research/dsh-probe-2026-09-13-report.md）
// 双数据源：storages/session_projcache（首选）+ sessions/<项目>/<会话>/session.vN.jsonl.zstd（兜底）

pub mod decode;
pub mod log;
pub mod preview;
pub mod projcache;
pub mod status;

use crate::adapter::AgentProcess;
use crate::session::Session;

/// dsh 数据根：$DSH_HOME 覆盖（M0 F14 优先级：env > ~/.dsh），测试注入用
pub fn dsh_home() -> std::path::PathBuf {
    std::env::var("DSH_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".dsh"))
}

/// 进程发现：node 进程且 cmdline 令牌含 "dsh" 与 "web"（M0：进程名是 node，必须按 cmdline 判定）
pub fn find_dsh_processes(_system: &sysinfo::System) -> Vec<AgentProcess> {
    Vec::new() // Task 8 实装
}

/// 会话聚合：宿主进程在位时扫描全部会话目录出卡（Task 8 实装）
pub fn get_dsh_sessions(_processes: &[AgentProcess]) -> Vec<Session> {
    Vec::new()
}
