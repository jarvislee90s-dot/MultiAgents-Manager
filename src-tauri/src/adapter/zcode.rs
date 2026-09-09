// ZCode（智谱桌面 AI 编程助手，Electron APP 形态，无独立 CLI 进程形态）适配器
// 进程数量与任务数无关（app-server 池化）——进程侧只做宿主判定（应用开没开），
// 会话唯一真相源是 ~/.zcode 下两个 SQLite 库；状态提取见 monitor/zcode_parser.rs

use super::*;
use crate::monitor;

pub struct ZCodeAdapter;

impl AgentAdapter for ZCodeAdapter {
    fn name(&self) -> &'static str {
        "ZCode"
    }
    fn agent_type(&self) -> AgentType {
        AgentType::ZCode
    }
    fn process_names(&self) -> &'static [&'static str] {
        // 会话发现不按进程名匹配（进程池化，数量与任务无关）；宿主判定走
        // find_processes 的专用实现。返回空切片：detect_all_tools 等入口已防御
        // 空切片（cli_available=false，与 workbuddy 同款形态）
        &[]
    }

    fn find_processes(&self, system: &System) -> Vec<AgentProcess> {
        // 宿主判定：应用开没开（ZCode 全部卡片的总开关与清理触发器）
        monitor::zcode_parser::discover_zcode_host(system)
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        monitor::zcode_parser::get_zcode_sessions(processes)
    }

    fn base_dir(&self) -> std::path::PathBuf {
        monitor::zcode_parser::zcode_home()
    }

    fn mcp_format(&self) -> McpFormat {
        // ZCode 主配置 ~/.zcode/cli/config.json 为 JSON（与 plugins 等顶层键共存），
        // MCP 服务器在 mcp.servers 子树（见 mcp_json_section）
        McpFormat::Json
    }
    fn mcp_config_path(&self) -> Option<std::path::PathBuf> {
        Some(self.base_dir().join("cli").join("config.json"))
    }
    fn mcp_json_section(&self) -> &'static [&'static str] {
        // 条目形态 {"command","args","env"}，另有可选 enable:false 停用标记；
        // 读-改-写只动该子树，未知键与原键序保留（serde_json preserve_order）
        &["mcp", "servers"]
    }

    fn skill_dirs(&self) -> Vec<std::path::PathBuf> {
        // Plan A：官方文档声明的用户级 skill 目录 ~/.zcode/skills（首装时创建，
        // 实测尚不存在——是否被真实读取未经确认，不确定性与备选方案见
        // IMPLEMENTATION_NOTES；跨工具共享目录 ~/.agents/skills 与 MAM
        // 「每工具独立激活」模型冲突，不采用）
        super::primary_skill_dir("zcode")
            .map(|dir| vec![dir])
            .unwrap_or_else(|| vec![self.base_dir().join("skills")])
    }

    // ZCode 插件为 marketplace 结构（另行立项，D3 范围外）；无 hooks（D4 纯轮询）；
    // 无独立子 agent 目录（子代理是会话级 parent_id 关联）——均沿用 trait 默认实现
}
