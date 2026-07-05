# Claude Code vs Codex CLI 接口对比文档

> 信息来源：参考项目源码（agent-sessions / HarnessKit / cc-switch / claude-control）+ 本机实际数据验证

## 一、总览

| 维度 | Claude Code | Codex CLI |
|------|-------------|-----------|
| 厂商 | Anthropic | OpenAI |
| 进程标识 | `claude` | `codex` |
| 配置目录 | `~/.claude/` | `~/.codex/` |
| 会话日志 | `~/.claude/projects/<encoded-cwd>/*.jsonl` | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` |
| Hook 系统 | ✅ `~/.claude/settings.json` | ✅ `~/.codex/hooks.json` |
| MCP 配置 | JSON (`~/.claude.json`) | TOML (`~/.codex/config.toml`) |
| Skill 目录 | `~/.claude/skills/` | `~/.agents/skills/` (规范) / `~/.codex/skills/` (废弃) |
| Plugin 目录 | `~/.claude/plugins/` | `~/.codex/plugins/cache/` |
| 子 Agent | `~/.claude/agents/*.md` | `~/.codex/agents/*.toml` |
| Rules 文件 | `CLAUDE.md` | `AGENTS.md` / `AGENTS.override.md` |
| 项目标记 | `.claude/` 目录 | `.codex/` 目录 |

## 二、会话发现与状态监控

### 2.1 进程发现

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 进程名 | `claude` | `codex` |
| 发现方式 | sysinfo 扫描 `cmd[0] == "claude"` 或以 `/claude` 结尾 | sysinfo 扫描 `cmd[0] == "codex"` 或以 `/codex` 结尾 |
| 子 Agent 过滤 | parent 也是 claude 进程 → 跳过 | 需确认（可能类似） |
| 孤儿进程检测 | parent chain 到 PID 1 → 跳过 | 需确认 |
| CWD 获取 | `process.cwd()` | `process.cwd()` |
| CPU/内存 | sysinfo 提供 | sysinfo 提供 |

### 2.2 会话日志格式

#### Claude Code JSONL

位置：`~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`
- 路径编码：`/Users/jarvis/Projects/my-project` → `-Users-jarvis-Projects-my-project`
- 子 Agent 文件：`agent-*.jsonl`（需排除/单独计数）

```json
{
  "session_id": "abc-123",
  "cwd": "/Users/jarvis/Projects/my-project",
  "git_branch": "main",
  "timestamp": "2026-04-30T15:33:39.572Z",
  "type": "user",                    // "user" | "assistant"
  "message": {
    "role": "user",                  // "user" | "assistant"
    "content": [
      {"type": "text", "text": "你好"},
      {"type": "tool_use", "name": "Bash", "input": {...}},
      {"type": "tool_result", "tool_use_id": "..."}
    ]
  },
  "isCompactSummary": false,
  "subtype": null                    // "compact_boundary" 表示正在压缩
}
```

#### Codex CLI JSONL

位置：`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`

```json
// 第一行：session_meta
{
  "timestamp": "2026-04-30T15:33:39.572Z",
  "type": "session_meta",
  "payload": {
    "id": "019ddf06-3665-7142-9813-378226ab0c0f",
    "timestamp": "2026-04-30T15:33:39.557Z",
    "cwd": "/Users/jarvis",
    "originator": "codex_cli_rs",
    "cli_version": "0.80.0",
    "instructions": "...",
    "source": "cli",
    "model_provider": "custom"
  }
}

// 后续行：response_item 或 event_msg
{
  "timestamp": "2026-04-30T15:33:44.459Z",
  "type": "response_item",
  "payload": {
    "type": "message",
    "role": "user",                  // "user" | "assistant"
    "content": [
      {"type": "input_text", "text": "你好"}
    ]
  }
}

{
  "timestamp": "2026-04-30T15:33:44.469Z",
  "type": "event_msg",
  "payload": {
    "type": "user_message",          // "user_message" | 其他事件类型
    "message": "你好",
    "images": []
  }
}
```

### 2.3 状态判断逻辑

| 状态 | Claude Code 判断 | Codex CLI 判断 |
|------|-----------------|----------------|
| **Thinking** (生成中) | 最后消息 role=user 且非本地命令 | 最后 response_item role=user |
| **Processing** (工具执行) | 最后消息 role=assistant 且有 tool_use | 需确认：event_msg 中的工具调用事件 |
| **Waiting** (等待输入) | 最后消息 role=assistant 纯文本 | 最后 response_item role=assistant |
| **Compacting** (压缩中) | subtype == "compact_boundary" | 需确认 |
| **Idle** (空闲) | 进程存活但无活动 | 进程存活但无活动 |
| **Finished** (结束) | 进程不存在 | 进程不存在 |

**关键差异：** Claude Code 的状态可直接从最后一条消息的 role + content 推断；Codex CLI 的格式不同（`type` + `payload` 结构），需要适配解析器，但核心逻辑相同。

### 2.4 Hook 事件系统

**好消息：Claude Code 和 Codex CLI 的 Hook 系统几乎完全一致！**

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 配置文件 | `~/.claude/settings.json` | `~/.codex/hooks.json` |
| 格式 | JSON | JSON（相同结构） |
| 项目级 | `<repo>/.claude/settings.json` | `<repo>/.codex/hooks.json` |
| 事件名 | PascalCase | PascalCase（**完全相同**） |
| stdin 输入 | JSON（hook_event_name, session_id, cwd, transcript_path, ts） | 需确认（应该相同） |

**共享事件列表：**
```
Stop, PreToolUse, PostToolUse, PostToolUseFailure,
UserPromptSubmit, SessionStart, SessionEnd,
Notification, PreCompact, PostCompact,
SubagentStart, SubagentStop, PermissionRequest
```

**Hook 配置格式（两者相同）：**
```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {"type": "command", "command": "/path/to/hook-script.sh"}
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "matcher": "",
        "hooks": [{"type": "command", "command": "/path/to/hook-script.sh"}]
      }
    ]
  }
}
```

## 三、Skill / MCP / Plugin 管理

### 3.1 Skill 目录

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 全局目录 | `~/.claude/skills/` | `~/.agents/skills/` (规范) |
| 废弃目录 | — | `~/.codex/skills/` (向后兼容) |
| 项目级 | `<repo>/.claude/skills/` | `<repo>/.agents/skills/` (从 cwd 向上扫描) |
| Skill 格式 | `SKILL.md` | `SKILL.md`（相同） |
| 发现方式 | 扫描目录下的 SKILL.md | 扫描目录下的 SKILL.md（相同） |

### 3.2 MCP 配置

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 全局配置 | `~/.claude.json` → `mcpServers` 字段 | `~/.codex/config.toml` → `[mcp_servers.<name>]` |
| 项目级 | `<repo>/.mcp.json` | `<repo>/.codex/config.toml` |
| 格式 | JSON | TOML |
| 字段 | command, args, env | command, args, env, url |
| 项目级 MCP | `.mcp.json` | `.codex/config.toml` |

**Claude Code MCP 格式（JSON）：**
```json
{
  "mcpServers": {
    "my-server": {
      "command": "npx",
      "args": ["-y", "@some/mcp-server"],
      "env": {"API_KEY": "..."}
    }
  }
}
```

**Codex CLI MCP 格式（TOML）：**
```toml
[mcp_servers.my-server]
command = "npx"
args = ["-y", "@some/mcp-server"]

[mcp_servers.my-server.env]
API_KEY = "..."
```

### 3.3 Plugin 管理

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 目录 | `~/.claude/plugins/` | `~/.codex/plugins/cache/{marketplace}/{plugin}/{version}/` |
| Registry | `installed_plugins.json` | config.toml 中 `[plugins."name@source"] enabled = false` |
| Manifest | `.claude-plugin/plugin.json` | `.codex-plugin/plugin.json` |

### 3.4 子 Agent

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| 全局目录 | `~/.claude/agents/*.md` | `~/.codex/agents/*.toml` |
| 项目级 | `<repo>/.claude/agents/*.md` | `<repo>/.codex/agents/*.toml` |
| 格式 | Markdown | TOML |

### 3.5 配置/Rules/Memory 文件

| | Claude Code | Codex CLI |
|---|-------------|-----------|
| Rules | `CLAUDE.md` | `AGENTS.md` / `AGENTS.override.md` / `TEAM_GUIDE.md` / `.agents.md` |
| Memory | `~/.claude/projects/<cwd>/memory/` | `~/.codex/memories/*.md` |
| Settings | `~/.claude/settings.json` | `~/.codex/config.toml` |

## 四、需要查官方文档确认的点

以下信息从参考项目和本机数据中**未能完全确认**，建议查阅官方文档：

### Claude Code（信息基本完整，少量确认）

| 待确认项 | 当前状态 | 文档链接 |
|---------|---------|---------|
| Hook stdin JSON 完整字段 | ✅ 已确认（hook_event_name, session_id, cwd, transcript_path, ts） | https://code.claude.com/docs/en/hooks |
| PermissionRequest 事件行为 | ⚠️ claude-control 注释说会误触发，需确认 | 同上 |
| settings.json 完整 schema | ✅ 本机已验证 keys | https://code.claude.com/docs/en/settings |

### Codex CLI（需确认项较多）

| 待确认项 | 当前状态 | 文档链接 |
|---------|---------|---------|
| Hook stdin JSON 格式 | ⚠️ 假设与 Claude 相同，需确认 | https://developers.openai.com/codex/hooks |
| 会话日志中的工具调用事件 | ⚠️ 需确认 event_msg 中的工具调用类型 | https://developers.openai.com/codex/logging |
| 进程名确认 | ⚠️ 假设为 `codex`，需确认 | — |
| Compacting 状态检测 | ⚠️ 需确认 Codex 是否有类似机制 | — |
| Hook 项目级 trust grant | ⚠️ HarnessKit 提到需 trust grant | https://developers.openai.com/codex/hooks |
| config.toml 完整 schema | ⚠️ 部分已知 | https://developers.openai.com/codex/config |

## 五、统一 AgentAdapter 设计

基于以上对比，建议设计统一的 adapter trait：

```rust
pub trait AgentAdapter: Send + Sync {
    // === 基础信息 ===
    fn name(&self) -> &str;
    fn base_dir(&self) -> PathBuf;
    fn detect(&self) -> bool;
    fn process_name(&self) -> &str;  // "claude" | "codex" | ...

    // === 会话监控 ===
    fn session_log_dir(&self) -> PathBuf;
    fn parse_session(&self, jsonl_path: &Path) -> Option<Session>;
    fn determine_status(&self, last_messages: &[JsonValue]) -> SessionStatus;
    fn hook_config_path(&self) -> PathBuf;
    fn hook_events(&self) -> &[&str];  // 共享事件列表

    // === Skill 管理 ===
    fn skill_dirs(&self) -> Vec<PathBuf>;

    // === MCP 管理 ===
    fn mcp_config_path(&self) -> PathBuf;
    fn mcp_format(&self) -> McpFormat;  // Json | Toml
    fn read_mcp_servers(&self) -> Vec<McpServerEntry>;
    fn write_mcp_server(&self, name: &str, entry: &McpServerEntry) -> Result<()>;

    // === 子 Agent ===
    fn subagent_dir(&self) -> Option<PathBuf>;
    fn subagent_format(&self) -> SubagentFormat;  // Markdown | Toml
}
```

**ClaudeCodeAdapter 和 CodexAdapter 的差异主要在：**
1. 会话日志路径和格式（JSONL 字段结构不同）
2. MCP 格式（JSON vs TOML）
3. Skill 目录路径
4. 子 Agent 格式（MD vs TOML）
5. Rules 文件名

**但 Hook 系统几乎完全一致**，这意味着 Hook 事件捕获逻辑可以共享。

## 六、实施策略确认

### 第一步：Claude Code（信息 100% 就绪）

直接移植 agent-sessions 的代码：
- `process/claude.rs` → 进程发现
- `session/parser.rs` → JSONL 解析
- `session/status.rs` → 状态判断
- `terminal/` → 终端跳转
- `agent/claude.rs` → AgentDetector 实现

### 第二步：Codex CLI（信息 90% 就绪）

需要适配的部分：
- 会话日志解析器：适配 `type` + `payload` 结构（vs Claude 的 `message` + `content`）
- MCP 读写：TOML 格式（用 toml crate）
- Skill 目录：`~/.agents/skills/`
- 子 Agent：TOML 格式

需要查文档确认的：
- Hook stdin 格式（假设与 Claude 相同）
- 会话日志中的工具调用/等待批准事件类型
- Compacting 检测机制

### 后续扩展

其他工具（Cursor / Kimi Code / OpenCode / OpenClaw / Hermes）按相同 adapter 模式实现。
APP 形态工具通过 MCP Server 插件暴露状态，不需进程/日志解析。

---

## 七、OpenCode 接口

> 信息来源：HarnessKit `adapter/opencode.rs` + agent-sessions `agent/opencode.rs`

### 7.1 总览

| 维度 | OpenCode |
|------|----------|
| 厂商 | 开源社区 (sst/opencode) |
| 进程标识 | `opencode` |
| 配置目录 | `~/.config/opencode/` |
| 数据目录 | `~/.local/share/opencode/storage/` |
| 会话存储 | JSON 文件（**非 JSONL**，分散存储） |
| Hook 系统 | ❌ 不支持 JSON hook 配置（hooks 是 JS/TS 插件代码） |
| MCP 配置 | JSONC (`opencode.json` / `opencode.jsonc`) |
| Skill 目录 | `~/.config/opencode/skills/` + `~/.agents/skills/` |
| Plugin 目录 | `~/.config/opencode/plugins/` (JS/TS 文件) |
| 子 Agent | `~/.config/opencode/agents/*.md` |
| Rules 文件 | `AGENTS.md` |
| 项目标记 | `.opencode/` 目录 / `opencode.json` / `opencode.jsonc` |

### 7.2 会话发现与状态监控

#### 进程发现
```
进程名: "opencode"
发现方式: sysinfo 扫描 name == "opencode"
CWD: process.cwd()
```

#### 会话数据结构（与 Claude/Codex 完全不同）

OpenCode 不使用 JSONL，而是将数据分散存储为独立 JSON 文件：

```
~/.local/share/opencode/storage/
├── project/
│   └── <project_id>.json          # 项目定义
├── session/
│   └── <project_id>/
│       └── <session_id>.json       # 会话元数据
├── messages/
│   └── <session_id>/
│       └── <message_id>.json       # 消息元数据
└── part/
    └── <message_id>/
        └── <part_id>.json          # 消息内容部分
```

**Project JSON:**
```json
{
  "id": "project-uuid",
  "worktree": "/path/to/project",
  "sandboxes": ["/path/to/worktree/branch-1"],
  "time": {"created": 1714499600000, "updated": 1714500000000}
}
```

**Session JSON:**
```json
{
  "id": "session-uuid",
  "projectID": "project-uuid",
  "directory": "/path/to/cwd",
  "title": "Session title",
  "time": {"created": 1714499600000, "updated": 1714500000000}
}
```

**Message JSON:**
```json
{
  "id": "message-uuid",
  "sessionID": "session-uuid",
  "role": "user",          // "user" | "assistant"
  "time": {"created": 1714499600000}
}
```

**Part JSON:**
```json
{
  "type": "text",          // "text" | "reasoning"
  "text": "消息内容"
}
```

#### 状态判断逻辑

```
1. 加载所有 project JSON，匹配 process.cwd → project.worktree 或 sandboxes
2. 加载 project 下最新的 session
3. 加载 session 下所有 messages，按 time.created 排序
4. 取最后一条 message 的 role
5. 状态判断:
   - CPU > 5% → Processing
   - last role == "assistant" → Waiting
   - last role == "user" → Processing
   - else → Idle
```

**注意：** OpenCode 的状态判断比 Claude/Codex 粗糙 — 没有 tool_use/tool_result 检测，依赖 CPU + role 启发式。

### 7.3 Hook 系统

**OpenCode 不支持 JSON 格式的 hook 配置。** HarnessKit 的 `hook_format()` 返回 `HookFormat::None`。

OpenCode 的 "hooks" 是 JS/TS 插件代码（在 `~/.config/opencode/plugins/` 中），不是声明式配置。这意味着：
- 无法像 Claude/Codex 那样注册 shell 脚本 hook
- 状态监控只能依赖进程扫描 + 数据文件解析（agent-sessions 的方式）
- 或者通过 MCP Server 插件暴露状态

### 7.4 MCP 配置

| | OpenCode |
|---|----------|
| 全局配置 | `~/.config/opencode/opencode.json` 或 `opencode.jsonc` |
| 项目级 | `<repo>/opencode.json` 或 `opencode.jsonc` |
| 格式 | JSON/JSONC（支持注释和尾逗号） |
| MCP 字段 | `mcp` (注意不是 `mcpServers`) |
| 支持类型 | `local`（命令行）和 `remote`（URL） |
| 原生 enabled 字段 | ✅ 支持 `enabled: true/false` |

**OpenCode MCP 格式：**
```json
{
  "mcp": {
    "my-server": {
      "type": "local",
      "command": ["bun", "x", "tool"],
      "environment": {"TOKEN": "abc"},
      "enabled": true
    },
    "remote-server": {
      "type": "remote",
      "url": "https://example.com/mcp"
    }
  }
}
```

**关键差异：**
- 字段名是 `mcp` 而非 `mcpServers`（Claude）或 `mcp_servers`（Codex TOML）
- `command` 是数组（`["bun", "x", "tool"]`）而非 `command` + `args` 分开
- `environment` 而非 `env`
- 支持 JSONC（注释 + 尾逗号）
- 原生支持 `enabled: false` 禁用

### 7.5 Skill / Plugin / 子 Agent

| | OpenCode |
|---|----------|
| 全局 Skill | `~/.config/opencode/skills/` + `~/.agents/skills/` |
| 项目 Skill | `<repo>/.opencode/skills/` |
| Plugin | `~/.config/opencode/plugins/*.js` / `*.ts` / `*.mjs` / `*.cjs` |
| Plugin 禁用 | 文件加 `.disabled` 后缀 |
| 子 Agent | `~/.config/opencode/agents/*.md` |
| 项目子 Agent | `<repo>/.opencode/agents/*.md` |
| Modes | `~/.config/opencode/modes/*.md` |
| Themes | `~/.config/opencode/themes/*.json` |
| Commands | `~/.config/opencode/commands/*.md` |

### 7.6 三工具接口对比总表

| 维度 | Claude Code | Codex CLI | OpenCode |
|------|-------------|-----------|----------|
| 进程名 | `claude` | `codex` | `opencode` |
| 配置目录 | `~/.claude/` | `~/.codex/` | `~/.config/opencode/` |
| 会话日志 | JSONL (projects/) | JSONL (sessions/) | JSON (storage/) |
| 日志格式 | `message.role` + `content[]` | `type` + `payload{}` | 分散 JSON 文件 |
| Hook 系统 | ✅ JSON | ✅ JSON（相同） | ❌ JS/TS 插件 |
| Hook 事件名 | PascalCase | PascalCase（相同） | — |
| MCP 格式 | JSON `mcpServers` | TOML `mcp_servers` | JSONC `mcp` |
| MCP command | `command` + `args` | `command` + `args` | `command[]` 数组 |
| MCP env 字段 | `env` | `env` | `environment` |
| Skill 目录 | `~/.claude/skills/` | `~/.agents/skills/` | `~/.config/opencode/skills/` + `~/.agents/skills/` |
| 子 Agent | `*.md` | `*.toml` | `*.md` |
| Rules | `CLAUDE.md` | `AGENTS.md` | `AGENTS.md` |
| 项目标记 | `.claude/` | `.codex/` | `.opencode/` / `opencode.json` |
| 原生 enabled | ❌ | ❌ | ✅ `enabled: false` |

