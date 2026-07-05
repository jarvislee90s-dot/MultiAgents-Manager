# 实现计划：多 Agent 编程工具统一管理平台

**分支**：`001-multi-agent-platform` | **日期**：2026-07-05 | **规格**：[spec.md](./spec.md)

**输入**：功能规格说明 `/specs/001-multi-agent-platform/spec.md`

## 摘要

构建一个 Tauri 2 桌面应用，解决两个核心痛点：(1) 多终端 AI 编程工具的运行状态不可见 — 通过红绿灯看板、声音通知、快速跳转解决；(2) 跨工具 skill/MCP/插件重复安装 — 通过全局仓库 + symlink 映射 + 预设组解决。MVP 支持三个工具（Claude Code + Codex CLI + OpenCode），包含完整资源管理（skill/MCP/插件统一仓库 + 预设组 + 子 Agent 分配）。

技术路径：以 tauri-app-template 为脚手架，移植 agent-sessions 的核心监控模块（AgentDetector trait + 进程发现 + JSONL/JSON 解析 + 终端跳转），移植 skills-manager-jw 的 linker 模块（symlink/Junction/copy），参考 HarnessKit 的 AgentAdapter trait 设计和 MCP 格式转换。

## 技术上下文

**语言/版本**：Rust（edition 2021）+ TypeScript 5.8

**主要依赖**：
- Rust 后端：tauri 2、sysinfo（进程监控）、serde + serde_json（序列化）、rusqlite（SQLite）、dashmap（高并发会话管理）、tokio（异步运行时）、dirs（路径获取）、fs2（文件锁）、chrono（时间）、notify（文件监听）、once_cell（全局单例）、log + env_logger（日志）
- 前端：react 19、@radix-ui/*（shadcn/ui 底层）、tailwindcss 4、zustand（状态管理）、lucide-react（图标）、class-variance-authority + clsx + tailwind-merge（样式工具）
- Tauri 插件：global-shortcut（全局热键）、notification（桌面通知）、opener（打开 URL/APP）、process（进程管理）、updater（自动更新）、single-instance（单实例）

**存储**：SQLite（嵌入式，通过 rusqlite）— 会话状态缓存（通知去重）、资源映射配置（skill×tool×subagent×enabled 四维表）、预设组定义、子 Agent 配置、审计数据

**测试**：`cargo test`（Rust 单元/集成测试，核心模块覆盖率 ≥80%）、`vitest`（React 组件测试）

**目标平台**：macOS（主要开发和测试平台）、Linux（X11 + Wayland 降级）、Windows（通过 Tauri 跨平台）

**项目类型**：桌面应用（Tauri 2）

**性能目标**：启动 <300ms（懒加载会话，20 个活跃会话场景下）、状态轮询 <50ms（active 会话）/ <5s（idle 会话）、二进制约 15MB、CPU 占用 <5%

**约束**：完全离线运行、无外部 API 调用、除应用本身外无后台守护进程、所有监控在本地完成

**规模/范围**：监控 20+ 并发会话、管理 100+ skill 跨 3 个工具（MVP），后续扩展到 10+ 工具

## 宪法检查

*门禁：Phase 0 研究前必须通过。Phase 1 设计后复查。*

依据：`.specify/memory/constitution.md` v1.2.0

| 原则 | 检查项 | 状态 | 说明 |
|------|--------|------|------|
| I. 统一技术栈 | 所有依赖是否在宪法清单内 | ✅ 通过 | 全部使用 Tauri 2 + Rust + React 19 + shadcn/ui + Tailwind v4，无混用 |
| II. Adapter 模式 | 是否使用 AgentAdapter trait 抽象 | ✅ 通过 | 移植 agent-sessions 的 AgentDetector trait，改名为 AgentAdapter，只有 name()/detect() 必须，其余默认实现 |
| III. 渐进式交付 | 阶段一是否为三工具 MVP | ✅ 通过 | 阶段一 Claude Code + Codex CLI + OpenCode 三工具，包含完整资源管理 |
| IV. Hook 优先 | 有 Hook 的工具是否优先用 Hook | ✅ 通过 | Claude Code + Codex CLI 用 Hook（事件名大小写通过 hook_event_case() 转换），OpenCode 回退进程扫描 + 数据文件解析 |
| V. 统一资源管理 | 是否用全局仓库 + symlink/Junction | ✅ 通过 | 移植 skills-manager-jw 的 linker.rs，Windows 用 Junction + copy 模式 |
| VI. 非侵入式体验 | 是否用托盘 + 悬浮窗 + 热键 | ✅ 通过 | Tauri tray-icon + always-on-top 窗口 + global-shortcut 插件 |
| VII. 安全透明 | 敏感路径是否排除 | ✅ 通过 | 9 个敏感路径列入排除列表，所有操作本地完成 |

**门禁结论**：全部通过，无违规需解释。可进入 Phase 0。

## 项目结构

### 文档（本功能）

```text
specs/001-multi-agent-platform/
├── plan.md              # 本文件
├── spec.md              # 功能规格说明
├── research.md          # Phase 0 调研结论
├── data-model.md        # Phase 1 数据模型
├── quickstart.md        # Phase 1 验证指南
├── contracts/           # Phase 1 接口契约
│   └── adapter-trait.md # AgentAdapter trait 契约
├── checklists/
│   └── requirements.md  # 规格质量检查清单
└── tasks.md             # Phase 2 任务分解（/speckit-tasks 生成）
```

### 源码（仓库根目录）

```text
src-tauri/src/
├── lib.rs               # 应用入口 + 系统托盘 + 插件注册
├── adapter/             # AgentAdapter trait + 各工具实现
│   ├── mod.rs           # trait 定义 + 注册表 + 事件名转换
│   ├── claude.rs        # Claude Code adapter（进程: claude, Hook: PascalCase, MCP: JSON）
│   ├── codex.rs         # Codex CLI adapter（进程: codex, Hook: camelCase, MCP: TOML）
│   └── opencode.rs      # OpenCode adapter（进程: opencode, 无 Hook, MCP: JSONC）
├── monitor/             # 会话监控
│   ├── mod.rs           # 会话发现 + 轮询调度（active 50ms / idle 5s + notify 文件监听）
│   ├── process.rs       # sysinfo 进程扫描 + 孤儿/子 Agent 过滤
│   ├── status.rs        # 纯消息状态判断（Claude/Codex JSONL 共用）
│   ├── opencode_status.rs # OpenCode 状态判断（CPU + role 启发式 + event_msg 解析）
│   ├── parser.rs        # JSONL 解析器（Claude message.content 格式 + Codex type.payload 格式）
│   ├── opencode_parser.rs # OpenCode 分散 JSON 解析（storage/project/session/messages/part 四级目录）
│   └── hooks.rs         # Hook 事件注册（写入 settings.json/hooks.json）+ 事件文件读取
├── terminal/            # 终端跳转
│   ├── mod.rs           # WindowManager trait + 平台分发
│   ├── applescript.rs   # macOS AppleScript（iTerm2 / Terminal.app）
│   ├── iterm.rs         # iTerm2 专有跳转
│   ├── terminal_app.rs  # Terminal.app 跳转
│   └── tmux.rs          # tmux pane 跳转
├── linker/              # 扩展资源映射
│   ├── mod.rs           # LinkerService（symlink/Junction/copy 三模式 + LinkStatus 健康检查）
│   └── detector.rs      # 工具检测（rayon 并行检测已安装的工具）
├── manager/             # MCP/插件配置读写
│   ├── mod.rs           # Manager 统一入口（按 ExtensionKind 分发）
│   ├── mcp.rs           # MCP 配置（JSON mcpServers / TOML mcp_servers / JSONC mcp 三格式转换）
│   ├── plugin.rs        # 插件管理（文件型 symlink + 配置型写入）
│   └── preset.rs        # 预设组应用逻辑（覆盖策略 + 冲突处理）
├── window/              # 窗口管理（跨平台）
│   └── mod.rs           # WindowManager trait（macOS AppleScript / Linux xdotool / Windows Win32 / Wayland 降级）
├── store.rs             # SQLite 数据层（9 张表 CRUD）
└── commands.rs           # Tauri IPC 命令（会话监控 + 资源管理 + 预设组 + 设置）

src/                     # React 前端
├── main.tsx             # 入口
├── App.tsx              # 路由 + 布局
├── components/
│   ├── SessionCard.tsx   # 会话卡片（红绿灯 + 状态 + 工具图标）
│   ├── SessionGrid.tsx   # 会话网格（按状态排序）
│   ├── StatusLight.tsx   # 红绿灯指示器组件（红/黄/绿 + 脉冲动画）
│   ├── TrayStatus.tsx    # 托盘聚合状态（取所有会话的最高优先级状态）
│   ├── NotificationToast.tsx # 通知弹窗（状态变更 + 点击跳转）
│   ├── ExtensionList.tsx # 资源列表（Skill/MCP/插件 分类展示）
│   ├── PresetBar.tsx     # 预设组快捷栏（pill 按钮 + 激活/停用）
│   └── ui/              # shadcn/ui 组件（button, card, dialog, badge, dropdown-menu, input, progress, sonner）
├── hooks/
│   ├── useSessions.ts    # 会话轮询 hook（调用 invoke("get_all_sessions")）
│   ├── useNotification.ts # 通知 hook（Web Audio API 音效 + tauri-plugin-notification）
│   └── useTray.ts        # 托盘状态 hook（聚合红/黄/绿 + 更新图标）
├── pages/
│   ├── Dashboard.tsx     # 监控看板（SessionGrid + StatusLight）
│   ├── Extensions.tsx    # 资源管理（ExtensionList + 工具×资源矩阵）
│   ├── Presets.tsx       # 预设组管理（创建/编辑/应用）
│   └── Settings.tsx      # 设置（通知配置 + 热键配置 + 工具开关）
├── stores/
│   ├── sessionStore.ts   # Zustand: 会话状态 + 通知去重
│   └── extensionStore.ts # Zustand: 资源管理状态
├── lib/
│   ├── formatters.ts     # 时间/文本格式化
│   ├── audio.ts          # Web Audio API 音效（双音 chime + 自定义提示音）
│   └── utils.ts          # 通用工具（cn 样式合并等）
└── types/
    └── session.ts        # TypeScript 类型定义
```

**结构决策**：采用 Tauri 标准的单项目结构（src-tauri/ + src/），不使用 workspace 多 crate 拆分。理由：MVP 代码量约 8000-10000 行，单 crate 足够清晰；后续若模块膨胀可拆分。

## 会话解析器设计

### Claude Code 解析器（移植 agent-sessions）

- 日志路径：`~/.claude/projects/<encoded-cwd>/*.jsonl`
- 解析方式：读取 JSONL 最后 N 行（tail 512KB），提取 `message.role` + `message.content[]`
- 状态判断：assistant + tool_use → Processing；assistant 纯文本 → Waiting；user → Thinking
- 子 Agent 文件：`agent-*.jsonl`（排除主会话，单独计数）
- 压缩检测：`subtype == "compact_boundary"` → Compacting

### Codex CLI 解析器（新增）

- 进程形态：CLI 进程名为 `codex`，桌面 APP 进程名为 `Codex`（macOS /Applications/Codex.app），两者共享 `~/.codex/` 日志目录，通过进程名区分为不同会话；APP 形态标记为不可跳转
- 日志路径：`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`
- 解析方式：读取 JSONL 最后 N 行，按 `type` 字段分发：
  - `session_meta` → 提取 `payload.id`（会话 ID）、`payload.cwd`（项目路径）
  - `response_item` → 提取 `payload.role`（user/assistant）+ `payload.content[]`
  - `event_msg` → 提取 `payload.type`（user_message 等）
- 状态判断：与 Claude 类似但字段路径不同（`payload.role` vs `message.role`）
- CWD 匹配：从 `session_meta.payload.cwd` 获取，与进程 CWD 匹配关联会话

### OpenCode 解析器（移植 agent-sessions agent/opencode.rs）

- 数据路径：`~/.local/share/opencode/storage/`（四级目录：project/ → session/ → messages/ → part/）
- 解析方式：
  1. 加载 `project/*.json`，匹配 `process.cwd` → `project.worktree` 或 `project.sandboxes`
  2. 加载 `session/<project_id>/*.json`，取最新更新的会话
  3. 加载 `messages/<session_id>/*.json`，按 `time.created` 排序，取最后一条
  4. 加载 `part/<message_id>/*.json`，提取 `text` 或 `reasoning` 内容
- 状态判断：CPU > 5% → Processing；last role == assistant → Waiting；last role == user → Processing；else → Idle
- 无 Hook 系统，完全依赖进程扫描 + 数据文件解析

## Hook 注册实现方案

### 注册流程

1. 应用首次启动时，检查 `~/.claude/settings.json` 和 `~/.codex/hooks.json` 是否已注册 Hook
2. 如未注册，写入 Hook 脚本路径到配置文件的 `hooks` 字段
3. Hook 脚本写入 `~/.mam/events/<ppid>.json`（按 Claude/Codex 进程 PID 命名）
4. monitor 模块轮询 `~/.mam/events/` 目录，读取最新事件文件

### 事件名转换

```
Claude Code (PascalCase)          Codex CLI (camelCase)
写入 settings.json:               写入 hooks.json:
"hooks": {                        "hooks": {
  "Stop": [...],                    "stop": [...],
  "UserPromptSubmit": [...],        "userPromptSubmit": [...],
  "SessionStart": [...],            "sessionStart": [...],
  "SessionEnd": [...]               "sessionEnd": [...]
}                                 }
```

adapter 的 `hook_event_case()` 返回大小写格式，`hook_events()` 返回事件列表，注册时自动转换。

### Hook 脚本（共享）

```bash
#!/bin/bash
# ~/.mam/hooks/status-hook.sh
# 从 stdin 读取 JSON，写入事件文件
EVENTS_DIR="$HOME/.mam/events"
mkdir -p "$EVENTS_DIR"
INPUT=$(cat)
# 提取字段（无 jq 依赖）
EVENT=$(echo "$INPUT" | grep -o '"hook_event_name"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
SESSION_ID=$(echo "$INPUT" | grep -o '"session_id"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
CWD=$(echo "$INPUT" | grep -o '"cwd"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
TRANSCRIPT=$(echo "$INPUT" | grep -o '"transcript_path"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/')
TS=$(date +%s)
echo "{\"event\":\"$EVENT\",\"session_id\":\"$SESSION_ID\",\"cwd\":\"$CWD\",\"transcript_path\":\"$TRANSCRIPT\",\"ts\":$TS}" > "$EVENTS_DIR/$PPID.json"
```

### OpenCode（无 Hook）

OpenCode 不支持 Hook 系统，完全依赖进程扫描 + 数据文件解析。monitor 模块的轮询逻辑同时检查：
- Hook 事件文件（Claude/Codex）— 新鲜则用 Hook
- 进程表（所有工具）— Hook 过期（>30s）则回退进程扫描

## 预设组应用逻辑

### 应用策略

一键应用预设组到目标工具时：
1. **读取**预设组中的所有资源项（skill + MCP + 插件）
2. **检查**每项资源是否在全局仓库中存在（不存在则报错）
3. **启用**：对每项资源调用 `toggle_extension(extension_id, tool_id, enabled=true)`
   - Skill → 创建 symlink/Junction 到工具 skill 目录
   - MCP → 按工具格式写入配置文件（JSON/TOML/JSONC）
   - Plugin → 文件型 symlink，配置型写入
4. **记录**应用状态到 `preset_applications` 表

### 取消激活策略

1. **读取**预设组中所有资源项
2. **禁用**：对每项资源调用 `toggle_extension(extension_id, tool_id, enabled=false)`
   - 移除 symlink/Junction
   - 从配置文件中删除对应条目
3. **更新** `preset_applications` 表，`active = false`

### 冲突处理

- 如果目标工具已有同名 MCP 配置（非本预设组添加的），**保留已有配置**，跳过该项，在 UI 中提示"已存在同名配置，跳过"
- 如果目标工具已有同名 skill symlink，**检查 link_status**：Valid 则跳过，其他则修复
- 预设组应用是**增量操作**（只添加，不删除预设组之外的资源），取消激活是**精确移除**（只移除预设组中的资源）

### 部分成功处理

- 预设组应用过程中，若某项资源启用失败（如权限不足、路径冲突、文件被占用），**保留已成功的项**，将失败项收集后报告给用户手动处理
- 不自动回滚已成功的项（与"增量操作"语义一致）

### Skill 分配约束（预设组独占）

- Skill 只能通过预设组应用到工具或子 Agent，**不支持单独为工具启用/禁用 skill**
- MCP 服务器和插件仍可单独启用/禁用（资源×工具 矩阵中可独立切换）
- 取消激活预设组时，精确移除该预设组引入的 skill 链接；不存在"手动单独启用的 skill"，因此无 CHK054 冲突
- MCP/插件若被手动单独启用，取消激活预设组时按冲突处理（保留已有、跳过、提示）

## 通知系统前端设计

### 红绿灯指示器（StatusLight 组件）

```
状态 → 颜色 → 动画：
  Waiting (等待用户)    → 红色 → 脉冲呼吸动画（1s 周期）
  Processing/Thinking   → 黄色 → 旋转加载动画
  Compacting            → 橙色 → 进度条动画
  Idle                  → 绿色 → 静态
  Finished              → 灰色 → 静态
```

### 托盘聚合状态（TrayStatus）

- 取所有活跃会话的最高优先级状态
- Waiting > Processing > Compacting > Idle > Finished
- 托盘图标颜色跟随聚合状态
- 托盘标题显示等待数（如"3 waiting"）

### 声音通知（audio.ts）

- 使用 Web Audio API 生成提示音（不依赖音频文件）
- 默认双音 chime（880Hz A5 + 1174.66Hz D6，参考 claude-control）
- 用户可自定义不同状态的提示音频率
- 通知去重：同一会话同一状态变更在 5 秒内不重复通知

### 桌面通知（useNotification hook）

- 使用 tauri-plugin-notification 发送原生通知
- 通知包含：工具名称 + 项目名称 + 新状态 + 状态颜色图标
- 通知可点击，点击后调用 `focus_session(pid)` 跳转终端

## 复杂度追踪

> 无宪法违规，本表为空。
