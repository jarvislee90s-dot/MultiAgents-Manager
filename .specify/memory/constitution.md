<!--
=== 同步影响报告 ===
版本变更: 1.2.0 -> 1.3.0
修改原则:
  - III: 阶段一由三工具改为四工具（Claude Code + Codex CLI + OpenCode + OpenClaw）
  - IV: 补充 notify 双策略落地到 monitor 的说明
  - V: 原子更新措辞精确化（单资源原子化 + 预设组按单位分别原子化）
  - 技术栈: 性能目标调整（启动 ≤3s 与 spec 001 对齐）
  - 开发流程: 代码组织小节增加 PATCH 占位说明
新增章节: 无
删除章节: 无
技术栈变更: 性能目标调整
需更新的模板:
  - .specify/templates/plan-template.md ✅ 兼容
  - .specify/templates/spec-template.md ✅ 兼容
  - .specify/templates/tasks-template.md ✅ 兼容
后续待办: Spec 002 完成后同步代码组织 PATCH
===
-->

# MultiAgents Manager 项目宪法

## 核心原则

### I. 统一技术栈（不可协商）

所有代码必须使用单一、统一的技术栈，该栈经 5 个以上参考项目验证（cc-switch、HarnessKit、skills-manager-jw、agent-sessions、tauri-app-template）：

- 桌面框架：Tauri 2
- 后端语言：Rust
- 前端：React 19 + TypeScript + Vite 7
- UI 组件：shadcn/ui (Radix UI) + Tailwind CSS v4
- 图标：Lucide Icons
- 状态管理：Zustand
- 数据存储：SQLite (rusqlite)
- 文件监听：notify (notify-rs)
- 包管理：pnpm

核心应用的后端运行时禁止混用 Electron、Swift、Python 或 Go。前端 npm 依赖不受此限制。CLI 工具可以是独立的 Rust 二进制文件。理由：单一技术栈降低认知负担，实现跨模块代码复用，且在我们调研的每个同类项目中都获得了成功验证。Tailwind v4 与 shadcn/ui 的兼容性已确认（shadcn/ui #6668 已关闭）。

### II. Adapter 模式实现多工具支持

每个 AI 编程工具（Claude Code、Codex CLI、OpenCode、Cursor、Kimi Code、Hermes 等）必须抽象到统一的 `AgentAdapter` trait 之后。新增工具不得修改核心逻辑 — 只需实现新的 adapter。

Adapter trait 只有 `name()` 和 `detect()` 是必须实现的，其余方法提供默认实现（返回 None/空），adapter 按需覆写：
- 进程发现（进程名、PID、CWD）— required
- 会话日志解析（路径、格式、状态提取）— optional，默认空
- Hook 事件注册（如支持）— optional，默认 None
- MCP 配置读写（格式转换）— optional，默认 JSON
- Skill 目录映射 — optional，默认空
- 子 Agent 配置（如适用）— optional，默认 None

参考实现：HarnessKit 的 `AgentAdapter` trait（9 个适配器）、agent-sessions 的 `AgentDetector` trait（2 个适配器）、ccmanager 的可配置状态检测策略（8 个工具）、agent-sessions 的 OpenCode 检测器（489行）。

### III. 渐进式交付（不可协商）

开发必须按阶段推进，每个阶段交付一个可用的产品：

- 阶段一（MVP）：Claude Code + Codex CLI + OpenCode + OpenClaw 四工具监控 — 红绿灯状态、桌面通知、声音提醒、终端跳转、skill/MCP/插件统一管理、预设组。Claude Code 和 Codex CLI 的 Hook 系统语义一致（仅事件名大小写不同），可共享 Hook 注册逻辑；OpenCode 无 Hook 系统、OpenClaw 无 Hook 系统，两者均通过进程扫描 + 数据文件解析。必须可独立使用。
- 阶段二：增加更多工具（Kimi Code、Hermes 等）+ 高级安全审计。
- 阶段三：增加 Cursor 等 APP 形态工具（进程扫描 + 输出解析，MCP 桥接作为后续增强）+ 多 Agent 编排、移动端、自愈机制。

每个阶段在下一阶段开始前必须可通过测试和演示验证。任何阶段不得依赖后续阶段的未完成工作。

### IV. Hook 优先的状态检测

拥有 Hook 系统的工具（Claude Code、Codex CLI）必须将 Hook 作为主要状态捕获机制。Hook 事件写入本地 JSON 文件，由看板轮询。

**关键差异：** Claude Code 的 Hook 事件名是 PascalCase（`PreToolUse`、`Stop`、`SessionStart`），Codex CLI 是 camelCase（`preToolUse`、`stop`、`sessionStart`）。语义完全一致，但大小写不同。Adapter 必须通过 `hook_event_case()` 方法声明大小写格式，Hook 注册时自动转换。Codex 额外支持 `commandWindows`、`timeoutSec`、`async`、`statusMessage` 字段，adapter 可按需使用。

无 Hook 的工具（OpenCode、OpenClaw、Cursor）必须回退到：通过 `sysinfo` crate 进行进程扫描 + 会话数据文件解析。

Hook 失败时必须有回退策略：monitor 模块的轮询逻辑同时检查 Hook 事件文件和进程表 — Hook 文件新鲜则用 Hook，过期（>30s）则回退进程扫描。monitor 模块必须同时集成 `notify` (notify-rs) 文件监听与轮询：Hook/进程事件有变化时优先用 notify 事件，定期轮询作为兜底。

### V. 统一资源管理

Skill、MCP 服务器和插件必须存储在单一全局仓库中（默认：`~/.mam/skills/`），通过符号链接（Unix）或 Junction/copy 模式（Windows）映射到各工具。

**Windows 强制要求：** Windows 上目录必须使用 Junction（`mklink /J`），文件必须使用 copy 模式（带 `.skills-manager-source.json` 元数据）。不得依赖 symlink（需要管理员权限）。检测 Junction 通过 reparse point 标志（`FILE_ATTRIBUTE_REPARSE_POINT = 0x0400`）。

映射必须支持：
- 按工具启用/禁用（每个 skill × tool 组合有独立状态）
- Presets 预设组（命名的 skill 分组，一键为某工具激活/停用）
- 子 Agent 级过滤（适用于 Hermes 等多 Agent 工具）
- 可视化配置看板
- 原子更新：单资源映射采用 write-to-temp + rename 或 fs2 文件锁；预设组的批量操作按资源单位分别原子化，单位间失败不自动回滚已成功项。

### VI. 非侵入式用户体验

监控看板不得干扰用户的终端工作流。状态展示方式：
- 系统托盘图标，聚合显示红/黄/绿状态
- 悬浮侧边栏（始终置顶，通过全局热键切换）
- 桌面通知，配以状态专属提示音

点击会话卡片必须跳转到对应的终端窗口或 APP。终端聚焦方式因平台而异：
- macOS：AppleScript（iTerm2、Terminal.app、kitty、WezTerm）
- Linux X11：xdotool
- Linux Wayland：降级为"仅通知不跳转"（检测 `$WAYLAND_DISPLAY` 环境变量）
- Windows：Win32 API SetForegroundWindow

跳转机制必须抽象到 `WindowManager` trait 之后。Wayland 降级时 UI 显示"此环境不支持跳转"提示。

### VII. 安全与透明

通过本平台安装的每个 skill、MCP 服务器和插件必须对用户可见，包括：
- 完整文件路径（在每个工具中的安装位置）
- 权限摘要（文件系统、网络、Shell 命令、环境变量）
- 启用/禁用开关（绝不静默安装）
- 首次启用 MCP 服务器时弹出确认对话框

敏感路径不得读取或暴露，完整列表：`~/.ssh`、`~/.gnupg`、`~/.aws`、`~/.kube`、`~/.netrc`、`~/.npmrc`、`~/.docker`、`~/.config/gcloud`、`~/.config/gh`、`~/.env`、凭据文件。

所有从外部安装的 skill 必须经过安全扫描后才能映射到工具。平台不得向外传输任何数据 — 所有监控均在本地完成。

## 技术栈约束

**语言/版本**：Rust（edition 2021）+ TypeScript 5.8

**主要依赖**：
- Rust：tauri 2、sysinfo、serde、rusqlite、dashmap、tokio、dirs、fs2、chrono、notify
- 前端：react 19、@radix-ui/*、tailwindcss 4、zustand、lucide-react
- Tauri 插件：global-shortcut、notification、opener、process、updater

**存储**：SQLite（嵌入式，通过 rusqlite），用于会话状态、映射配置和审计数据

**测试**：`cargo test`（Rust 单元/集成测试，核心模块覆盖率 ≥80%）、`vitest`（React 组件测试）

**目标平台**：macOS（主要）、Linux、Windows（通过 Tauri 跨平台）

**项目类型**：桌面应用（Tauri 2）

**性能目标**：启动 ≤3s（含 20 会话冷启动场景）；懒加载单页 <300ms、状态轮询 <50ms（active 会话）/ <5s（idle 会话）、二进制约 15MB

**约束**：完全离线运行、无外部 API 调用、除应用本身外无后台守护进程

**规模/范围**：监控 20+ 并发会话、管理 100+ skill 跨 4 个工具（MVP），后续扩展到 10+ 工具

## 开发流程

**阶段门禁流程**：每个阶段进入下一阶段前必须通过以下检查：
1. 宪法检查 — 实现是否符合全部 7 条原则？
2. 功能测试 — 该阶段核心功能能否端到端演示？
3. 无回归 — 上一阶段的功能仍然正常？

**代码组织**：

**待同步（PATCH）**：Spec 002 完成后，本小节将同步更新为 `commands/`（按功能拆分）、`database/`（含 dao/）、`services/`（原 manager/）、`window/`（原 terminal/）结构。其余模块 `adapter/`、`monitor/`、`linker/`、`plugins/` 保持。

```
src-tauri/src/
├── adapter/          — AgentAdapter trait + 各工具实现
├── monitor/          — 会话发现、状态检测、事件轮询（notify + 轮询双策略）
├── linker/           — Skill symlink/Junction/copy 管理（MVP）
├── manager/          — MCP/插件配置读写（MVP）
├── window/           — WindowManager trait（macOS/Linux/Windows/Wayland）
├── store.rs          — SQLite 数据层
└── commands.rs       — Tauri IPC 命令
```

**审查流程**：所有代码变更必须对照 adapter trait 契约审查 — 工具特定逻辑不得泄漏到核心模块中。每个 adapter 必须实现降级策略，失败时返回 Unknown 状态而非 panic。

**参考项目**：agent-sessions（监控 + OpenCode 检测器）、HarnessKit（adapter + 扩展管理）、skills-manager-jw（symlink linker）、xingkongliang/skills-manager（预设组）、ccmanager（多工具状态检测策略）、tauri-app-template（脚手架）、notify-rs/notify（文件监听）。

## 治理

本宪法是 MultiAgents Manager 项目的最高权威文档。所有规格说明、计划和任务必须遵守这些原则。当原则与实际需求冲突时，以原则为准，除非明确修订。

修订需要：
1. 书面理由（改了什么、为什么改、影响评估）
2. 版本递增（MAJOR：原则删除/重定义，MINOR：新增原则/章节或原则实质修改，PATCH：澄清/措辞修正）
3. 一致性传播到所有依赖模板（plan、spec、tasks）
4. 更新 `Last Amended` 日期

合规验证：每个 PR 和代码审查必须检查技术栈（原则 I）、adapter 模式（原则 II）和渐进式交付（原则 III）的遵守情况。

**版本**：1.3.0 | **批准日期**：2026-07-05 | **最后修订**：2026-07-10
