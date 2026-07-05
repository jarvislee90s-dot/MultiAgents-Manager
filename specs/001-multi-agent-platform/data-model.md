# 数据模型：多 Agent 编程工具统一管理平台

**创建日期**：2026-07-05
**规格引用**：[spec.md](./spec.md) | [plan.md](./plan.md)

## 实体关系图

```
AgentTool 1───* Session
    │
    │ 1
    │
    * │
SubAgent       Extension (Skill/MCP/Plugin)
    │               │
    │ *             │ *
    │               │
    *               *
ExtensionAssignment  Preset
    │                   │
    │ *                 │ *
    │                   │
    *                   *
PresetApplication
```

## 实体定义

### Session（会话）

运行中的 AI 编程工具实例，通过进程扫描和 Hook 事件发现。

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 会话 ID（从 JSONL 日志提取） |
| agent_type | Enum | 工具类型：Claude / Codex / OpenCode（Codex 同时覆盖 CLI 与桌面 APP 两种形态，由 `form` 字段区分） |
| project_name | String | 项目名称（从 cwd 路径提取） |
| project_path | String | 项目路径（进程 cwd） |
| git_branch | Option<String> | Git 分支名（从 JSONL 提取） |
| status | Enum | 状态：Waiting / Processing / Thinking / Compacting / Idle / Finished |
| last_message | Option<String> | 最后一条消息预览（截断 100 字符） |
| last_message_role | Option<String> | 最后消息角色：user / assistant |
| last_activity_at | String | 最后活动时间（ISO 8601） |
| pid | u32 | 进程 PID |
| cpu_usage | f32 | CPU 使用率（%） |
| active_subagent_count | usize | 活跃子 Agent 数量 |
| form | Enum | 进程形态：Cli / App（桌面 APP 形态，如 Codex 桌面 APP；APP 不可跳转） |
| jump_supported | bool | 是否支持终端跳转（CLI=true，App=false） |

**状态转换图**：
```
         UserPromptSubmit          tool_use 完成
    ┌──────────────────┐    ┌──────────────────┐
    │                  ▼    │                  ▼
 ┌──┴──┐          ┌──────────┐          ┌──────────┐
 │Idle │          │ Thinking │          │Processing│
 └──┬──┘          └────┬─────┘          └─────┬────┘
    │                  │                       │
    │ 用户输入          │ Claude 回复            │ tool_result
    │                  ▼                       ▼
    │            ┌──────────┐            ┌──────────┐
    │            │ Waiting  │◄───────────│ Thinking │
    │            └──────────┘            └──────────┘
    │                  │
    │ SessionEnd       │
    ▼                  │
 ┌──────────┐          │
 │ Finished │          │
 └──────────┘          │
                       │ compact_boundary
                       ▼
                 ┌──────────┐
                 │Compacting│
                 └──────────┘
```

**存储**：不持久化到 SQLite，每次轮询从进程表和 JSONL 文件实时构建。状态转换历史可选缓存到 SQLite 用于通知去重。

### AgentTool（工具适配器配置）

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 工具标识：`claude` / `codex` / `opencode` |
| name | String | 显示名称：`Claude Code` / `Codex CLI` |
| process_name | String | 进程名：`claude` / `codex` / `opencode` |
| base_dir | PathBuf | 配置目录：`~/.claude` / `~/.codex` / `~/.config/opencode` |
| hook_supported | bool | 是否支持 Hook 系统 |
| hook_event_case | Enum | PascalCase / camelCase |
| mcp_format | Enum | JSON(mcpServers) / TOML(mcp_servers) / JSONC(mcp) |
| skill_dirs | Vec<PathBuf> | Skill 目录列表 |
| detected | bool | 是否检测到已安装 |
| enabled | bool | 用户是否启用此工具的监控 |

### Extension（扩展资源）

Skill、MCP 服务器、插件的统一抽象。

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 资源唯一 ID（UUID） |
| kind | Enum | Skill / Mcp / Plugin |
| name | String | 资源名称 |
| description | String | 资源描述 |
| source_path | String | 统一仓库中的原始路径 |
| source_url | Option<String> | 来源 URL（GitHub repo 等） |
| version | Option<String> | 版本号 |
| tags | Vec<String> | 标签 |
| installed_at | DateTime | 安装时间 |
| updated_at | DateTime | 最后更新时间 |

### ExtensionAssignment（资源分配）

资源与工具（及子 Agent）的映射关系。

| 字段 | 类型 | 说明 |
|------|------|------|
| extension_id | String | 关联 Extension.id |
| agent_tool_id | String | 关联 AgentTool.id |
| sub_agent_id | Option<String> | 关联 SubAgent.id（None=工具级） |
| enabled | bool | 是否启用 |
| link_status | Enum | Valid / Broken / WrongTarget / Missing |
| assigned_at | DateTime | 分配时间 |

**约束**：
- sub_agent_id 非空时，对应的工具级 ExtensionAssignment 必须存在且 enabled=true（子 Agent 只能使用工具级范围内的资源）。
- kind=Skill 的 ExtensionAssignment 只能由预设组应用/取消激活创建和移除，不提供单独启用/禁用 skill 的接口（避免手动分配与预设组移除冲突）。
- kind=Mcp/Plugin 的 ExtensionAssignment 支持单独启用/禁用，也支持预设组批量操作。

### Preset（预设组）

扩展资源的命名组合，可包含 skill + MCP + 插件。

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 预设组 ID（UUID） |
| name | String | 预设组名称（如"前端开发"） |
| items | Vec<PresetItem> | 包含的资源列表 |
| created_at | DateTime | 创建时间 |

### PresetItem（预设组成员）

| 字段 | 类型 | 说明 |
|------|------|------|
| extension_id | String | 关联 Extension.id |
| kind | Enum | Skill / Mcp / Plugin |

### PresetApplication（预设应用记录）

预设组应用到工具/子 Agent 的记录。

| 字段 | 类型 | 说明 |
|------|------|------|
| preset_id | String | 关联 Preset.id |
| agent_tool_id | String | 关联 AgentTool.id |
| sub_agent_id | Option<String> | 关联 SubAgent.id（None=工具级） |
| applied_at | DateTime | 应用时间 |
| active | bool | 是否当前激活 |

### SubAgent（子 Agent）

多 Agent 工具内部的子角色。

| 字段 | 类型 | 说明 |
|------|------|------|
| id | String | 子 Agent ID |
| name | String | 显示名称（如 researcher、coder） |
| agent_tool_id | String | 所属工具 ID |
| config_path | PathBuf | 配置文件路径 |
| format | Enum | Markdown / Toml |

## SQLite 表结构

```sql
-- 会话状态缓存（用于通知去重，不存储完整会话数据）
CREATE TABLE session_status_cache (
    session_id TEXT PRIMARY KEY,
    agent_type TEXT NOT NULL,
    status TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    previous_status TEXT
);

-- 扩展资源
CREATE TABLE extensions (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,         -- skill / mcp / plugin
    name TEXT NOT NULL,
    description TEXT,
    source_path TEXT NOT NULL,
    source_url TEXT,
    version TEXT,
    tags TEXT,                   -- JSON array
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 资源分配
CREATE TABLE extension_assignments (
    id TEXT PRIMARY KEY,
    extension_id TEXT NOT NULL REFERENCES extensions(id),
    agent_tool_id TEXT NOT NULL,
    sub_agent_id TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    link_status TEXT NOT NULL DEFAULT 'missing',
    assigned_at TEXT NOT NULL,
    UNIQUE(extension_id, agent_tool_id, sub_agent_id)
);

-- 预设组
CREATE TABLE presets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- 预设组成员
CREATE TABLE preset_items (
    id TEXT PRIMARY KEY,
    preset_id TEXT NOT NULL REFERENCES presets(id),
    extension_id TEXT NOT NULL REFERENCES extensions(id),
    kind TEXT NOT NULL
);

-- 预设应用记录
CREATE TABLE preset_applications (
    id TEXT PRIMARY KEY,
    preset_id TEXT NOT NULL REFERENCES presets(id),
    agent_tool_id TEXT NOT NULL,
    sub_agent_id TEXT,
    applied_at TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1
);

-- 工具配置
CREATE TABLE agent_tools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    process_name TEXT NOT NULL,
    base_dir TEXT NOT NULL,
    hook_supported INTEGER NOT NULL,
    hook_event_case TEXT NOT NULL,
    mcp_format TEXT NOT NULL,
    detected INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1
);

-- 子 Agent
CREATE TABLE sub_agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    agent_tool_id TEXT NOT NULL REFERENCES agent_tools(id),
    config_path TEXT NOT NULL,
    format TEXT NOT NULL
);

-- 设置
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

## MCP 配置格式转换

```
统一仓库中的 MCP 配置（内部格式）:
{
    "name": "my-server",
    "command": "npx",
    "args": ["-y", "@some/mcp-server"],
    "env": {"API_KEY": "..."}
}

写入 Claude Code (JSON ~/.claude.json):
    "mcpServers": { "my-server": { "command": "npx", "args": [...], "env": {...} } }

写入 Codex CLI (TOML ~/.codex/config.toml):
    [mcp_servers.my-server]
    command = "npx"
    args = ["-y", "@some/mcp-server"]
    [mcp_servers.my-server.env]
    API_KEY = "..."

写入 OpenCode (JSONC opencode.json):
    "mcp": { "my-server": { "type": "local", "command": ["npx", "-y", "@some/mcp-server"], "environment": {...} } }
```
