# 接口契约：AgentAdapter Trait

**创建日期**：2026-07-05
**规格引用**：[spec.md](../spec.md) | [data-model.md](../data-model.md)

## AgentAdapter Trait

统一所有 AI 编程工具的适配器接口。只有 `name()` 和 `detect()` 是必须实现的，其余方法提供默认实现。

```rust
/// AI 编程工具适配器 trait
/// 新增工具只需实现此 trait，不修改核心逻辑
pub trait AgentAdapter: Send + Sync {
    // === 必须实现 ===

    /// 工具标识（如 "claude"、"codex"）
    fn name(&self) -> &str;

    /// 检测工具是否已安装
    fn detect(&self) -> bool;

    // === 可选覆写（有默认实现）===

    /// 进程名（用于 sysinfo 扫描）
    fn process_name(&self) -> &str { "" }

    /// 配置目录（如 ~/.claude）
    fn base_dir(&self) -> PathBuf { PathBuf::new() }

    // --- 会话监控 ---

    /// 会话日志目录
    fn session_log_dir(&self) -> Option<PathBuf> { None }

    /// 解析会话日志文件，返回会话信息
    fn parse_session(&self, log_path: &Path, pid: u32, cpu: f32) -> Option<Session> { None }

    // --- Hook 系统 ---

    /// Hook 配置文件路径
    fn hook_config_path(&self) -> Option<PathBuf> { None }

    /// Hook 事件名大小写格式
    fn hook_event_case(&self) -> HookEventCase { HookEventCase::PascalCase }

    /// 支持的 Hook 事件列表
    fn hook_events(&self) -> Vec<&str> { vec![] }

    // --- Skill 管理 ---

    /// Skill 目录列表
    fn skill_dirs(&self) -> Vec<PathBuf> { vec![] }

    // --- MCP 管理 ---

    /// MCP 配置文件路径
    fn mcp_config_path(&self) -> Option<PathBuf> { None }

    /// MCP 配置格式
    fn mcp_format(&self) -> McpFormat { McpFormat::JsonMcpServers }

    /// 读取 MCP 服务器列表
    fn read_mcp_servers(&self) -> Vec<McpServerEntry> { vec![] }

    /// 写入一个 MCP 服务器配置
    fn write_mcp_server(&self, name: &str, entry: &McpServerEntry) -> Result<(), String> {
        Err("MCP write not supported".into())
    }

    // --- 子 Agent ---

    /// 子 Agent 配置目录
    fn subagent_dir(&self) -> Option<PathBuf> { None }

    /// 子 Agent 配置格式
    fn subagent_format(&self) -> SubagentFormat { SubagentFormat::Markdown }

    // --- 项目级配置 ---

    /// 项目标记（如 .claude/ 目录）
    fn project_markers(&self) -> Vec<ProjectMarker> { vec![] }
}

/// Hook 事件名大小写格式
pub enum HookEventCase {
    PascalCase,  // Claude Code: PreToolUse, Stop, SessionStart
    CamelCase,   // Codex CLI: preToolUse, stop, sessionStart
}

/// MCP 配置格式
pub enum McpFormat {
    JsonMcpServers,  // Claude Code: JSON, mcpServers 字段, command+args+env
    TomlMcpServers,  // Codex CLI: TOML, [mcp_servers.<name>], command+args+env
    JsoncMcp,        // OpenCode: JSONC, mcp 字段, command[]数组+environment
}

/// 子 Agent 配置格式
pub enum SubagentFormat {
    Markdown,  // Claude Code: ~/.claude/agents/*.md
    Toml,      // Codex CLI: ~/.codex/agents/*.toml
}

/// 项目标记
pub enum ProjectMarker {
    Dir(&str),   // 如 .claude, .codex
    File(&str),  // 如 opencode.json
}
```

## Tauri IPC 命令

前端通过 `@tauri-apps/api` 的 `invoke()` 调用以下命令：

```typescript
// 会话监控
invoke("get_all_sessions"): Promise<SessionsResponse>
invoke("focus_session", { pid: number }): Promise<void>
invoke("kill_session", { pid: number }): Promise<void>

// 系统托盘
invoke("update_tray_title", { title: string }): Promise<void>

// 全局快捷键
invoke("register_shortcut", { shortcut: string }): Promise<void>
invoke("unregister_shortcut", { shortcut: string }): Promise<void>

// 通知设置（阶段一）
invoke("get_notification_settings"): Promise<NotificationSettings>
invoke("set_notification_settings", { settings: NotificationSettings }): Promise<void>

// 扩展资源管理（阶段二）
invoke("list_extensions", { kind: ExtensionKind }): Promise<Extension[]>
invoke("install_extension", { source: string, kind: ExtensionKind }): Promise<Extension>
invoke("toggle_extension", { extension_id: string, agent_tool_id: string, enabled: boolean }): Promise<void>

// 预设组管理（阶段二）
invoke("list_presets"): Promise<Preset[]>
invoke("create_preset", { name: string, items: PresetItem[] }): Promise<Preset>
invoke("apply_preset", { preset_id: string, agent_tool_id: string }): Promise<void>
invoke("remove_preset", { preset_id: string }): Promise<void>
```

## TypeScript 类型定义

```typescript
type AgentType = "claude" | "codex" | "opencode";
type SessionStatus = "waiting" | "processing" | "thinking" | "compacting" | "idle";
type ExtensionKind = "skill" | "mcp" | "plugin";

interface Session {
    id: string;
    agentType: AgentType;
    projectName: string;
    projectPath: string;
    gitBranch: string | null;
    status: SessionStatus;
    lastMessage: string | null;
    lastMessageRole: string | null;
    lastActivityAt: string;
    pid: number;
    cpuUsage: number;
    activeSubagentCount: number;
}

interface SessionsResponse {
    sessions: Session[];
    totalCount: number;
    waitingCount: number;
}
```

## Hook 事件名映射表

| 语义 | Claude Code (PascalCase) | Codex CLI (camelCase) |
|------|--------------------------|----------------------|
| 会话开始 | SessionStart | sessionStart |
| 会话结束 | SessionEnd | — |
| 用户提交 | UserPromptSubmit | userPromptSubmit |
| 停止 | Stop | stop |
| 工具使用前 | PreToolUse | preToolUse |
| 工具使用后 | PostToolUse | postToolUse |
| 工具使用失败 | PostToolUseFailure | — |
| 权限请求 | PermissionRequest | permissionRequest |
| 通知 | Notification | — |
| 压缩前 | PreCompact | preCompact |
| 压缩后 | PostCompact | postCompact |
| 子Agent开始 | SubagentStart | subagentStart |
| 子Agent停止 | SubagentStop | subagentStop |
