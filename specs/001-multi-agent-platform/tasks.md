# 任务列表：多 Agent 编程工具统一管理平台

**输入**：设计文档 `/specs/001-multi-agent-platform/`（spec.md、plan.md、research.md、data-model.md、contracts/adapter-trait.md、quickstart.md）

**组织**：按用户故事分组，支持独立实现和测试。`[P]` 标记可并行任务。

## 路径约定

- Rust 后端：`src-tauri/src/`
- React 前端：`src/`
- 单项目结构（非 workspace）

---

## US1: 多工具会话监控看板

### 基础设施

- [x] T001 [US1] 以 tauri-app-template 为基础搭建项目脚手架，重命名为 multi-agents-manager，合并 Cargo.toml 依赖（sysinfo、serde、rusqlite、dashmap、tokio、dirs、fs2、chrono、notify、once_cell、log）和 package.json 依赖
  - 文件：`src-tauri/Cargo.toml`、`package.json`、`src-tauri/src/lib.rs`、`src/main.tsx`

- [x] T002 [P] [US1] 定义 AgentAdapter trait 和相关枚举（HookEventCase、McpFormat、SubagentFormat、ProjectMarker）
  - 文件：`src-tauri/src/adapter/mod.rs`
  - 参考：contracts/adapter-trait.md

- [x] T003 [P] [US1] 定义 Session 数据模型（Session、SessionStatus、AgentType、SessionsResponse、JsonlMessage）
  - 文件：`src-tauri/src/session/model.rs`
  - 参考：data-model.md

### Claude Code 适配器

- [x] T004 [US1] 实现 Claude Code 进程发现（sysinfo 扫描 "claude" 进程，过滤子 Agent 和孤儿进程）
  - 文件：`src-tauri/src/adapter/claude.rs`、`src-tauri/src/monitor/process.rs`
  - 参考：agent-sessions `process/claude.rs`

- [x] T005 [US1] 实现 Claude Code JSONL 解析器（读取 ~/.claude/projects/ 目录，解析 message.role + content[]，提取 session_id/cwd/git_branch）
  - 文件：`src-tauri/src/monitor/parser.rs`
  - 参考：agent-sessions `session/parser.rs`

- [x] T006 [P] [US1] 实现纯消息状态判断（assistant+tool_use→Processing，assistant纯文本→Waiting，user→Thinking，compact_boundary→Compacting）
  - 文件：`src-tauri/src/monitor/status.rs`
  - 参考：agent-sessions `session/status.rs`

### Codex CLI 适配器

- [x] T007 [US1] 实现 Codex CLI 进程发现（sysinfo 扫描 "codex" 进程 + "Codex" 桌面 APP 进程，区分 form=Cli/App，复用孤儿/子 Agent 过滤逻辑；APP 标记 jump_supported=false）
  - 文件：`src-tauri/src/adapter/codex.rs`
  - 依赖：T004（复用进程过滤逻辑）

- [x] T008 [US1] 实现 Codex CLI JSONL 解析器（读取 ~/.codex/sessions/YYYY/MM/DD/，解析 type+payload 结构，session_meta 提取 cwd，response_item 提取 role）
  - 文件：`src-tauri/src/monitor/parser.rs`（新增 Codex 分支）
  - 依赖：T005

### OpenCode 适配器

- [x] T009 [US1] 实现 OpenCode 进程发现（sysinfo 扫描 "opencode" 进程）
  - 文件：`src-tauri/src/adapter/opencode.rs`
  - 参考：agent-sessions `agent/opencode.rs`

- [x] T010 [US1] 实现 OpenCode 分散 JSON 解析器（四级目录：project/ → session/ → messages/ → part/，匹配 cwd → worktree/sandboxes）
  - 文件：`src-tauri/src/monitor/opencode_parser.rs`
  - 参考：agent-sessions `agent/opencode.rs`

- [x] T011 [P] [US1] 实现 OpenCode 状态判断（CPU > 5% → Processing，last role == assistant → Waiting，last role == user → Processing，else → Idle）
  - 文件：`src-tauri/src/monitor/opencode_status.rs`

### 会话发现调度器

- [x] T012 [US1] 实现会话发现调度器（共享 System 实例，按 AgentType 注册检测器，active 50ms / idle 5s 双频轮询 + notify 文件监听）
  - 文件：`src-tauri/src/monitor/mod.rs`
  - 依赖：T004、T007、T009

- [x] T013 [P] [US1] 实现 SQLite 数据层（session_status_cache + settings 两张表，通知去重 + 配置读写）
  - 文件：`src-tauri/src/store.rs`
  - 参考：data-model.md

### 前端看板

- [x] T014 [P] [US1] 实现 useSessions hook（轮询 invoke("get_all_sessions")，Zustand 状态管理，通知去重）
  - 文件：`src/hooks/useSessions.ts`、`src/stores/sessionStore.ts`

- [x] T015 [P] [US1] 实现 SessionCard 组件（工具图标 + 项目名 + 红绿灯状态 + 最后消息预览 + 运行时长 + CPU）
  - 文件：`src/components/SessionCard.tsx`

- [x] T016 [P] [US1] 实现 StatusLight 红绿灯指示器组件（红/黄/绿/橙/灰五色 + 脉冲/旋转/静态动画）
  - 文件：`src/components/StatusLight.tsx`

- [x] T017 [US1] 实现 SessionGrid 会话网格（按状态优先级排序：Waiting > Processing > Idle）
  - 文件：`src/components/SessionGrid.tsx`
  - 依赖：T015、T016

- [x] T018 [US1] 实现 Dashboard 页面（SessionGrid 布局 + 空状态提示）
  - 文件：`src/pages/Dashboard.tsx`
  - 依赖：T014、T017

- [x] T019 [US1] 实现 Tauri IPC 命令（get_all_sessions、focus_session、kill_session）
  - 文件：`src-tauri/src/commands.rs`
  - 依赖：T012

### 系统托盘

- [x] T020 [US1] 实现系统托盘（TrayIconBuilder + 菜单 + 左键点击显示窗口 + 聚合红/黄/绿状态图标）
  - 文件：`src-tauri/src/lib.rs`
  - 参考：agent-sessions `lib.rs`、tauri-app-template 托盘逻辑
  - 依赖：T019

### US1 检查点

- [x] T021 [US1] 验证：打开 3 个终端分别运行 claude/codex/opencode，看板同时显示 3 个会话卡片，状态正确；另启动 Codex 桌面 APP 验证显示为 form=App 且不可跳转的独立会话

---

## US2: 状态变更通知与提醒

### Hook 系统

- [x] T022 [US2] 实现 Hook 事件注册器（写入 ~/.claude/settings.json 的 hooks 字段 + ~/.codex/hooks.json，事件名自动转换 PascalCase/camelCase）
  - 文件：`src-tauri/src/monitor/hooks.rs`
  - 参考：plan.md Hook 注册实现方案

- [x] T023 [US2] 编写共享 Hook 脚本（~/.mam/hooks/status-hook.sh，从 stdin 读取 JSON 写入 ~/.mam/events/<ppid>.json）
  - 文件：脚本模板内嵌到 hooks.rs
  - 参考：plan.md Hook 脚本设计

- [x] T024 [US2] 实现 Hook 事件文件读取（轮询 ~/.mam/events/ 目录，按 PPID 关联进程，过期文件 >30s 回退进程扫描）
  - 文件：`src-tauri/src/monitor/hooks.rs`
  - 依赖：T022

### 前端通知

- [x] T025 [P] [US2] 实现声音通知（Web Audio API 双音 chime 880Hz+1174.66Hz，可自定义频率）
  - 文件：`src/lib/audio.ts`
  - 参考：claude-control `useNotificationSound.ts`

- [x] T026 [P] [US2] 实现桌面通知（tauri-plugin-notification，含工具名+项目名+状态+颜色图标，通知可点击跳转）
  - 文件：`src/hooks/useNotification.ts`
  - 依赖：T019（focus_session 命令）

- [x] T027 [US2] 实现通知去重逻辑（同一会话同一状态变更 5 秒内不重复，状态转换记录到 SQLite session_status_cache）
  - 文件：`src/hooks/useNotification.ts`、`src/stores/sessionStore.ts`
  - 依赖：T013、T025、T026

### 设置页面

- [x] T028 [P] [US2] 实现通知设置页面（全局开关 + 按状态配置提示音 + 测试按钮）
  - 文件：`src/pages/Settings.tsx`
  - 依赖：T027

### US2 检查点

- [x] T029 [US2] 验证：Claude Code 会话完成任务时收到桌面通知 + 提示音，Codex CLI 同理，OpenCode 通过进程扫描触发通知

---

## US3: 快速跳转到终端

### 窗口管理

- [x] T030 [US3] 实现 WindowManager trait（macOS AppleScript / Linux xdotool / Windows Win32 API / Wayland 降级检测）
  - 文件：`src-tauri/src/window/mod.rs`

- [x] T031 [P] [US3] 实现 macOS AppleScript 终端跳转（iTerm2 / Terminal.app，通过 ps 获取 TTY，AppleScript 匹配激活）
  - 文件：`src-tauri/src/terminal/applescript.rs`、`src-tauri/src/terminal/iterm.rs`、`src-tauri/src/terminal/terminal_app.rs`
  - 参考：agent-sessions `terminal/`

- [x] T032 [P] [US3] 实现 tmux pane 跳转（focus_tmux_pane_by_tty）
  - 文件：`src-tauri/src/terminal/tmux.rs`
  - 参考：agent-sessions `terminal/tmux.rs`

- [x] T033 [US3] 实现 Wayland 降级（检测 $WAYLAND_DISPLAY，降级为"仅通知不跳转" + UI 提示）
  - 文件：`src-tauri/src/window/mod.rs`
  - 依赖：T030

### 前端跳转

- [x] T034 [US3] 实现 SessionCard 点击跳转（调用 invoke("focus_session", {pid})，不支持时显示提示）
  - 文件：`src/components/SessionCard.tsx`（修改）
  - 依赖：T015、T030、T019

### US3 检查点

- [x] T035 [US3] 验证：点击 Claude Code 会话卡片跳转到 iTerm2 对应标签页；点击 tmux 中的 Codex 会话跳转到对应 pane

---

## US4: Skill/MCP/插件统一仓库管理

### 全局仓库 + Linker

- [x] T036 [US4] 实现全局仓库目录结构（~/.mam/skills/ 原始文件 + ~/.mam/hooks/ Hook 脚本 + ~/.mam/events/ 事件文件）
  - 文件：`src-tauri/src/linker/mod.rs`

- [x] T037 [US4] 移植 LinkerService（symlink Unix / Junction Windows / copy 模式 + LinkStatus 健康检查 + 原子写入 write-to-temp + rename）
  - 文件：`src-tauri/src/linker/mod.rs`
  - 参考：skills-manager-jw `linker.rs`

- [x] T038 [P] [US4] 移植工具检测器（rayon 并行检测 3 个工具，检测目录存在 + CLI 可用性）
  - 文件：`src-tauri/src/linker/detector.rs`
  - 参考：skills-manager-jw `detector.rs`

### SQLite 扩展

- [x] T039 [US4] 扩展 SQLite 数据层（新增 extensions、extension_assignments、agent_tools、sub_agents 四张表）
  - 文件：`src-tauri/src/store.rs`（修改）
  - 依赖：T013
  - 参考：data-model.md

### MCP 配置管理

- [x] T040 [US4] 实现 MCP 格式转换器（内部格式 {command, args, env} → JSON mcpServers / TOML mcp_servers / JSONC mcp 三格式写入 + 读取）
  - 文件：`src-tauri/src/manager/mcp.rs`
  - 参考：HarnessKit `deployer.rs` deploy_mcp_server_json/toml/opencode

- [x] T041 [US4] 实现 MCP 配置读写集成到 adapter（Claude: ~/.claude.json JSON, Codex: ~/.codex/config.toml TOML, OpenCode: opencode.jsonc JSONC）
  - 文件：`src-tauri/src/adapter/claude.rs`、`src-tauri/src/adapter/codex.rs`、`src-tauri/src/adapter/opencode.rs`（修改）
  - 依赖：T040

### 插件管理

- [x] T042 [P] [US4] 实现插件管理（文件型 symlink 映射 + 配置型写入工具配置，Windows .disabled 后缀检测）
  - 文件：`src-tauri/src/manager/plugin.rs`

### Manager 统一入口

- [x] T043 [US4] 实现 Manager 统一入口（按 ExtensionKind 分发到 linker/mcp.rs/plugin.rs）。toggle_extension 仅支持 MCP/Plugin；Skill 不提供单独启用接口，由 preset.rs 调用 linker 创建/移除链接
  - 文件：`src-tauri/src/manager/mod.rs`
  - 依赖：T037、T040、T042

- [x] T044 [US4] 实现 Tauri IPC 命令（list_extensions、install_extension、toggle_extension（仅 MCP/Plugin）、read_mcp_servers、write_mcp_server）
  - 文件：`src-tauri/src/commands.rs`（修改）
  - 依赖：T043

### 前端资源管理

- [x] T045 [P] [US4] 实现 ExtensionList 组件（Skill/MCP/Plugin 三分类展示；MCP/Plugin 显示资源×工具启用矩阵可切换，Skill 行只读并标注"通过预设组分配"+ 来源/版本/路径显示）
  - 文件：`src/components/ExtensionList.tsx`

- [x] T046 [US4] 实现 Extensions 页面（ExtensionList + 安装/卸载操作；MCP/Plugin 启用/禁用 + 工具开关；Skill 跳转至 Presets 页面管理）
  - 文件：`src/pages/Extensions.tsx`
  - 依赖：T045

### US4 检查点

- [x] T047 [US4] 验证：安装一个 skill 到全局仓库，为 Claude Code 和 Codex CLI 启用，两个工具的 skill 目录出现 symlink；为 Codex 启用 MCP，config.toml 出现 [mcp_servers] 段

---

## US5: 预设组一键切换

### 预设组数据模型

- [x] T048 [US5] 扩展 SQLite 数据层（新增 presets、preset_items、preset_applications 三张表）
  - 文件：`src-tauri/src/store.rs`（修改）
  - 依赖：T039

### 预设组应用逻辑

- [x] T049 [US5] 实现预设组应用逻辑（读取预设项 → 检查全局仓库存在性 → 逐项 toggle_extension(enabled=true) → 记录 preset_applications）
  - 文件：`src-tauri/src/manager/preset.rs`
  - 依赖：T043

- [x] T050 [US5] 实现预设组取消激活（读取预设项 → 逐项 toggle_extension(enabled=false) → 移除 symlink/配置 → 更新 preset_applications active=false）
  - 文件：`src-tauri/src/manager/preset.rs`
  - 依赖：T049

- [x] T051 [US5] 实现冲突处理（已有同名 MCP 保留并跳过 + 已有 Valid symlink 跳过 + UI 提示冲突项）+ 部分成功处理（失败项收集报告用户，不回滚已成功项）
  - 文件：`src-tauri/src/manager/preset.rs`
  - 依赖：T050

### 前端预设组

- [x] T052 [P] [US5] 实现 PresetBar 组件（pill 按钮列表 + 激活/停用状态 + 点击切换）
  - 文件：`src/components/PresetBar.tsx`

- [x] T053 [US5] 实现 Presets 页面（创建/编辑/删除预设组 + 资源选择器 + 应用到工具/子Agent）
  - 文件：`src/pages/Presets.tsx`
  - 依赖：T052

- [x] T054 [US5] 实现系统托盘预设操作（托盘菜单列出预设组 + 点击切换）
  - 文件：`src-tauri/src/lib.rs`（修改）
  - 依赖：T049、T050、T020

- [x] T055 [US5] 实现 Tauri IPC 命令（list_presets、create_preset、apply_preset、remove_preset）
  - 文件：`src-tauri/src/commands.rs`（修改）
  - 依赖：T049

### US5 检查点

- [x] T056 [US5] 验证：创建"前端开发"预设组（2 skill + 1 MCP），为 Codex CLI 一键应用，验证 skill 目录出现 2 个链接 + config.toml 出现 1 个 MCP 段；取消激活后三项同时移除

---

## US6: 子 Agent 级资源分配

### 子 Agent 检测

- [x] T057 [US6] 实现子 Agent 检测（扫描 ~/.claude/agents/*.md、~/.codex/agents/*.toml、~/.config/opencode/agents/*.md）
  - 文件：`src-tauri/src/adapter/claude.rs`、`src-tauri/src/adapter/codex.rs`、`src-tauri/src/adapter/opencode.rs`（修改）

- [x] T058 [US6] 扩展 extension_assignments 表支持 sub_agent_id（约束：sub_agent_id 非空时工具级必须存在且 enabled=true）
  - 文件：`src-tauri/src/store.rs`（修改）
  - 依赖：T039

### 子 Agent 分配逻辑

- [x] T059 [US6] 实现子 Agent 级分配约束（分配前检查工具级范围，超出范围拒绝并提示；工具级禁用时子 Agent 级自动移除）
  - 文件：`src-tauri/src/manager/mod.rs`（修改）
  - 依赖：T043、T058

- [x] T060 [US6] 实现子 Agent 级预设组（预设组中的资源必须在工具级范围内，否则跳过并提示）
  - 文件：`src-tauri/src/manager/preset.rs`（修改）
  - 依赖：T049、T059

### 前端子 Agent

- [x] T061 [US6] 实现子 Agent 分配 UI（Extensions 页面中工具展开显示子 Agent + 每个子 Agent 的资源分配矩阵）
  - 文件：`src/pages/Extensions.tsx`（修改）、`src/components/SubAgentPanel.tsx`
  - 依赖：T046、T059

### US6 检查点

- [x] T062 [US6] 验证：为 Codex CLI 启用 20 个 skill，为 researcher 子 Agent 分配 5 个，验证子 Agent 目录只有 5 个链接；尝试分配未在工具级启用的 skill 被拒绝

---

## 安全与全局

- [x] T063 [P] 实现敏感路径排除（9 个路径：~/.ssh、~/.gnupg、~/.aws、~/.kube、~/.netrc、~/.npmrc、~/.docker、~/.config/gcloud、~/.config/gh）+ skill 路径穿越检查（仅验证 skill 文件路径落在允许目录内，不检查内容）
  - 文件：`src-tauri/src/store.rs` 或独立模块

- [x] T064 [P] 实现全局热键切换（tauri-plugin-global-shortcut，默认 Ctrl+Space 切换看板窗口显示/隐藏）
  - 文件：`src-tauri/src/lib.rs`（修改）、`src/hooks/useTray.ts`
  - 参考：tauri-app-template 快捷键设置

- [x] T065 [P] 实现通知去重 Hook 脚本安装检查（首次启动检测 Hook 是否已注册，未注册则自动注册）
  - 文件：`src-tauri/src/monitor/hooks.rs`（修改）
  - 依赖：T022

- [x] T066 验证：MVP 全量测试（quickstart.md 场景 1-11 全部通过）

---

## 依赖关系总结

```
T001 (脚手架) → T002 (trait) → T004 (Claude进程) → T005 (Claude解析)
                                                    ↓
T007 (Codex进程) → T008 (Codex解析) → T012 (调度器) → T019 (IPC) → T020 (托盘)
T009 (OpenCode进程) → T010 (OpenCode解析)
T003 (模型) ────┘        T006 (状态) ──┘
T013 (SQLite) ───────────────────────────────→ T039 (扩展表) → T048 (预设表)
T014-T018 (前端看板) ← T019
T022-T024 (Hook) → T027 (去重) → T028 (设置页)
T030-T034 (窗口跳转)
T036-T043 (Linker/Manager) → T044 (IPC) → T045-T046 (前端资源)
T049-T051 (预设逻辑) → T052-T055 (前端预设)
T057-T061 (子Agent)
```

## 并行任务标记

可并行执行的任务组：
- T002, T003（trait 和模型定义无依赖）
- T006, T011（状态判断：Claude/Codex 共用 + OpenCode 独立）
- T013, T014, T015, T016（SQLite、hook、组件可并行）
- T025, T026（声音和桌面通知独立）
- T028（设置页面独立）
- T031, T032（AppleScript 和 tmux 跳转独立）
- T038（工具检测器独立）
- T042（插件管理独立）
- T045（ExtensionList 组件独立）
- T052（PresetBar 组件独立）
- T063, T064, T065（安全/热键/Hook检查独立）
