// dsh（DeepSeek harness）adapter — 第 8 个受监控工具（宪法 D14，只读）
// 会话解析见 monitor::dsh（projcache + zstd 日志双源）
// 形态：本地 web 服务（进程形态 App）；MCP/Skill 管理非 M1 范围（写通道另评）

use super::*;
use crate::monitor;

pub struct DshAdapter;

impl AgentAdapter for DshAdapter {
    fn name(&self) -> &'static str {
        "dsh"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::Dsh
    }
    fn process_names(&self) -> &'static [&'static str] {
        // 进程名是 node，通用名匹配不可用；发现走 find_dsh_processes 的 cmdline 判定
        //（zcode 同款形态：空切片 + 专用实现，detect 入口已防御空切片）
        &[]
    }
    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        monitor::dsh::find_dsh_processes(system)
    }
    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::dsh::get_dsh_sessions(processes)
    }
    fn base_dir(&self) -> std::path::PathBuf {
        monitor::dsh::dsh_home()
    }
    // hook/MCP/plugin 均用 trait 默认值（hook_supported=false、McpFormat::Json 默认、
    // mcp_config_path=None、skill_dirs 默认空）——M1 只做监控
}
