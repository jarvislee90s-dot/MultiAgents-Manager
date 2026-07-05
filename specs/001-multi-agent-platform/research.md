# 调研报告：多 Agent 编程工具统一管理平台

**创建日期**：2026-07-05
**规格引用**：[spec.md](./spec.md)

## 1. 技术栈选型

**决策**：Tauri 2 + Rust + React 19 + TypeScript + shadcn/ui + Tailwind CSS v4

**理由**：
- 5 个参考项目验证了这条路径（cc-switch ⭐113k、HarnessKit ⭐345、skills-manager-jw ⭐870、agent-sessions ⭐80、tauri-app-template ⭐86）
- agent-sessions 与目标技术栈完全一致，代码可直接移植
- tauri-app-template 提供完整脚手架（i18n、暗色主题、自动更新、标题栏、全局快捷键、单实例）
- shadcn/ui 已支持 Tailwind v4（issue #6668 于 2026-04-05 关闭）

**备选方案**：
- Electron + React：Pane ⭐265 和 skiller ⭐46 使用，但二进制约 100MB+，不满足性能目标
- Swift/SwiftUI：agent-manager-x ⭐68 使用，但仅 macOS，不满足跨平台需求
- Rust 纯 TUI：ccboard ⭐80 使用，但无 GUI，不满足红绿灯悬浮窗需求

## 2. Claude Code 接口

**进程标识**：`claude`（sysinfo 扫描 cmd[0]）

**会话日志**：`~/.claude/projects/<encoded-cwd>/*.jsonl`
- 路径编码：`/Users/jarvis/Projects/my-project` → `-Users-jarvis-Projects-my-project`
- 子 Agent 文件：`agent-*.jsonl`（需排除或单独计数）
- JSONL 格式：每行一个 JSON，含 `sessionId`、`cwd`、`gitBranch`、`timestamp`、`type`（user/assistant）、`message.role` + `message.content[]`
- content 数组项类型：`text`、`tool_use`、`tool_result`

**Hook 系统**：
- 配置文件：`~/.claude/settings.json`
- 格式：JSON，`hooks` → event → `[{matcher, hooks: [{type: "command", command: "..."}]}]`
- 事件名：**PascalCase**（`PreToolUse`、`Stop`、`SessionStart`、`UserPromptSubmit`、`PermissionRequest`、`PostToolUse`、`PostToolUseFailure`、`SessionEnd`、`Notification`、`PreCompact`、`PostCompact`、`SubagentStart`、`SubagentStop`）
- Hook 脚本从 stdin 接收 JSON：`{hook_event_name, session_id, cwd, transcript_path, ts}`

**MCP 配置**：`~/.claude.json` → `mcpServers` 字段（JSON，`{command, args, env}`）
**Skill 目录**：`~/.claude/skills/`
**Plugin 目录**：`~/.claude/plugins/`（registry at `installed_plugins.json`）
**子 Agent**：`~/.claude/agents/*.md`
**Rules**：`CLAUDE.md`
**项目标记**：`.claude/` 目录

**参考实现**：agent-sessions `process/claude.rs`（164行）、`session/parser.rs`（690行）、`session/status.rs`（192行）

## 3. Codex CLI 接口

**进程标识**：`codex`（预计，需验证）

**会话日志**：`~/.codex/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
- 格式：JSONL，每行一个 JSON，含 `timestamp`、`type`、`payload`
- type=`session_meta`：`{id, cwd, originator, cli_version, model_provider}`
- type=`response_item`：`{type: "message", role: "user"/"assistant", content: [{type: "input_text", text}]}`
- type=`event_msg`：`{type: "user_message", message, images}`

**Hook 系统**：
- 配置文件：`~/.codex/hooks.json`
- 格式：JSON，与 Claude Code 结构相同
- 事件名：**camelCase**（`preToolUse`、`stop`、`sessionStart`、`userPromptSubmit`、`permissionRequest`、`postToolUse`、`preCompact`、`postCompact`、`subagentStart`、`subagentStop`）
- **关键差异**：事件名大小写与 Claude Code 不同（PascalCase vs camelCase），语义完全一致
- **额外字段**：Codex 的 ConfiguredHookHandler 支持 `commandWindows`（Windows 专用命令）、`timeoutSec`、`async`、`statusMessage`
- **HookHandlerType**：三种 — `command`、`prompt`、`agent`（Claude Code 只有 `command`）

**MCP 配置**：`~/.codex/config.toml` → `[mcp_servers.<name>]`（TOML，`command, args, env`）
**Skill 目录**：`~/.agents/skills/`（规范）+ `~/.codex/skills/`（废弃）
**Plugin 目录**：`~/.codex/plugins/cache/{marketplace}/{plugin}/{version}/`
**子 Agent**：`~/.codex/agents/*.toml`
**Rules**：`AGENTS.md` / `AGENTS.override.md`
**项目标记**：`.codex/` 目录

**待验证项**：
- Hook stdin JSON 格式是否与 Claude Code 完全一致（假设一致，需在编码前验证 https://developers.openai.com/codex/hooks）
- 会话日志中的工具调用事件类型（`event_msg` 的具体子类型）
- 进程名是否为 `codex`

**参考实现**：HarnessKit `adapter/codex.rs`（MCP 读写）、接口文档中已确认的 JSONL 格式

## 4. OpenCode 接口（阶段三参考）

**进程标识**：`opencode`
**会话存储**：`~/.local/share/opencode/storage/`（分散 JSON 文件，非 JSONL）
**Hook 系统**：不支持 JSON hook 配置（hooks 是 JS/TS 插件代码）
**MCP 配置**：`~/.config/opencode/opencode.json` 或 `opencode.jsonc` → `mcp` 字段（JSONC，`{type: "local", command: [...], environment: {...}, enabled: true/false}`）
**Skill 目录**：`~/.config/opencode/skills/` + `~/.agents/skills/`
**子 Agent**：`~/.config/opencode/agents/*.md`

**关键差异**：MCP 字段名是 `mcp`（非 `mcpServers`/`mcp_servers`），`command` 是数组（非 command+args 分开），`environment`（非 `env`），原生支持 `enabled: false`

## 5. 三工具接口差异对比

| 维度 | Claude Code | Codex CLI | OpenCode |
|------|-------------|-----------|----------|
| 进程名 | `claude` | `codex` | `opencode` |
| 会话日志 | JSONL (projects/) | JSONL (sessions/) | JSON (storage/) |
| 日志格式 | `message.role` + `content[]` | `type` + `payload{}` | 分散 JSON 文件 |
| Hook 事件名大小写 | PascalCase | camelCase | — |
| Hook 配置格式 | JSON | JSON（相同） | 不支持 |
| MCP 格式 | JSON `mcpServers` | TOML `mcp_servers` | JSONC `mcp` |
| MCP command | `command` + `args` | `command` + `args` | `command[]` 数组 |
| MCP env 字段 | `env` | `env` | `environment` |
| Skill 目录 | `~/.claude/skills/` | `~/.agents/skills/` | `~/.config/opencode/skills/` |
| 子 Agent 格式 | `*.md` | `*.toml` | `*.md` |

## 6. 移植方案

**决策**：以 tauri-app-template 为脚手架，移植 agent-sessions 核心模块

**移植清单**：
- tauri-app-template：直接 copy + rename（i18n、主题、更新、标题栏、快捷键、单实例）
- agent-sessions Rust 后端：移植 agent/、process/、session/、terminal/、commands/（约 2500 行，含 OpenCode 检测器 agent/opencode.rs 489 行）
- agent-sessions 前端：移植 SessionCard、SessionGrid、useSessions、类型定义（约 260 行）
- claude-control 通知系统：移植 useNotificationSound（Web Audio API）+ useDesktopNotification
- skills-manager-jw linker.rs：MVP 移植（714 行，symlink/Junction/copy 三模式 + LinkStatus 健康检查）
- skills-manager-jw detector.rs：MVP 移植（265 行，rayon 并行工具检测）
- HarnessKit：不移植代码，参考 AgentAdapter trait 设计、MCP 格式转换（deployer.rs）、Hook 事件名转换（hook_events.rs）

**新增代码**：
- Codex adapter（adapter/codex.rs + monitor/parser.rs 中 Codex 分支）
- Codex JSONL 解析器（适配 type+payload 结构，与 Claude 的 message+content 分支共存于 parser.rs）
- 通知系统（monitor/hooks.rs Hook 注册 + 前端通知组件 + audio.ts 音效）
- 红绿灯看板 UI（StatusLight + TrayStatus + NotificationToast 组件）
- MCP 格式转换（manager/mcp.rs，JSON/TOML/JSONC 三格式）
- 预设组应用逻辑（manager/preset.rs，覆盖策略 + 冲突处理）
- 窗口管理（window/mod.rs，macOS/Linux/Windows/Wayland 分发）

**MVP 预估代码量**：约 9000-10000 行（5000-5500 Rust + 4000-4500 TS/TSX），其中约 60% 为直接移植

## 7. 风险评估与解决方案

| 风险 | 严重度 | 解决方案 |
|------|--------|---------|
| Codex Hook stdin 格式未验证 | 高 | 从 openai/codex 仓库 schema 确认事件名为 camelCase，需在编码前验证 stdin payload |
| MVP 单工具验证不足 | 高 | 改为 Claude Code + Codex CLI 双工具 MVP，Hook 语义一致仅大小写不同 |
| Tailwind v4 稳定性 | 高 | shadcn/ui #6668 已关闭，v4 兼容性已解决 |
| Wayland 终端跳转失效 | 高 | 降级为"仅通知不跳转"，检测 `$WAYLAND_DISPLAY` |
| Windows symlink 权限 | 高 | Junction（目录）+ copy 模式（文件），移植 skills-manager-jw 实现 |
| Adapter trait 过早抽象 | 中 | trait 方法用默认实现，只有 name()/detect() 必须 |
| 轮询 CPU 压力 | 中 | active 50ms + idle 5s 双策略，叠加 notify crate 文件监听 |
| 安全路径不完整 | 中 | 参考 amux 源码，9 个敏感路径列入排除列表 |
| 二进制大小目标不现实 | 中 | 从 <5MB 调整为 <15MB |

## 8. 参考项目清单

| 项目 | Star | 移植内容 | 许可证 |
|------|------|---------|--------|
| tauri-app-template (kitlib) | 86 | 脚手架 | MIT |
| agent-sessions (ozankasikci) | 80 | 监控核心模块 + OpenCode 检测器 | MIT |
| HarnessKit (RealZST) | 345 | Adapter trait 设计参考 | Apache-2.0 |
| skills-manager-jw (jiweiyeah) | 870 | linker.rs（阶段二） | MIT |
| skills-manager (xingkongliang) | 2733 | Presets 概念参考 | MIT |
| claude-control (sverrirsig) | 124 | 通知系统参考 | MIT |
| ccmanager (kbwo) | 1176 | 多工具状态检测策略参考 | MIT |
| notify-rs/notify | 3407 | 文件监听 crate | MIT |
