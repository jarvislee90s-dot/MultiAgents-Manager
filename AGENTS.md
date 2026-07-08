# 项目指令

## 语言规范
- 本项目所有设计文档、规格说明、计划文档、任务列表均使用中文撰写
- 代码标识符使用英文，代码注释尽量使用中文
- Git commit message 可使用英文

## 技术栈
- Tauri 2 + Rust + React 19 + TypeScript + shadcn/ui + Tailwind CSS v4

## 构建与开发命令

```bash
pnpm install            # 安装前端依赖
pnpm tauri:dev          # 启动开发模式（Rust + Vite HMR）
pnpm build              # TypeScript 编译 + Vite 打包
pnpm check              # 完整检查：format + lint + build
pnpm format             # Prettier 自动格式化
pnpm format:check       # Prettier 只读检查
pnpm lint               # ESLint 检查
pnpm lint:fix           # ESLint 自动修复

# Rust 专用检查（在 src-tauri/ 目录下）
cd src-tauri && cargo check    # 编译检查（快）
cd src-tauri && cargo test     # 运行 Rust 单元测试
cd src-tauri && cargo clippy   # Rust 代码 lint
```

## 项目结构

```
# 项目根目录
├── src/                    # 前端源码（React + TypeScript）
│   ├── pages/             #   页面路由：首页、设置、关于
│   ├── components/        #   UI 组件（会话卡片、资源管理、预设组等）
│   ├── hooks/             #   React Hooks（会话轮询、通知、更新器）
│   ├── stores/            #   Zustand 状态管理
│   ├── lib/               #   工具函数（音频、快捷键、截图）
│   ├── i18n/              #   国际化（中文 / English）
│   └── types/             #   TypeScript 类型定义
├── src-tauri/             # Tauri Rust 后端
│   └── src/
│       ├── adapter/       #   Agent 适配器（Claude / Codex / OpenCode / OpenClaw）
│       ├── monitor/       #   进程扫描、会话解析、状态判定
│       ├── manager/       #   Skill/MCP/Plugin 管理、预设组、兼容性检查
│       ├── linker/        #   三层符号链接映射（SSOT → Tool → SubAgent）
│       ├── terminal/      #   终端聚焦（iTerm2 / Terminal.app / tmux）
│       ├── plugins/       #   系统托盘
│       ├── store.rs       #   SQLite 数据层
│       └── commands.rs    #   Tauri IPC 命令
├── docs/                  # 开发文档
├── scripts/               # 构建/发布脚本
├── research/              # 早期调研文档（本地参考，不入库）
├── package.json           # 项目配置
├── vite.config.ts         # Vite 构建配置
├── tsconfig.json          # TypeScript 配置
└── README.md              # 项目说明
```

## 架构概览

**Tauri 2 桌面应用**，Rust 后端 + React 19 前端，通过 Tauri IPC (`invoke`) 通信。

### 后端（Rust）— `src-tauri/src/`

- **`adapter/`** — 工具适配器，实现 `AgentAdapter` trait。每个适配器负责发现进程、解析会话、读写配置、管理 skill 目录。
- **`monitor/`** — 进程扫描、JSONL 会话解析、Hook 事件读取、状态判定。
- **`manager/`** — Skill 安装/启用/禁用、MCP 配置写入（JSON/TOML/JSONC）、预设组应用/取消、插件管理。
- **`linker/`** — 符号链接/交接点管理，三层映射：
  - **Layer 1**: SSOT 全局仓库 `~/.mam/skills/`
  - **Layer 2**: 工具级激活目录 `~/.mam/active/<tool>/skills/`
  - **Layer 3**: 子 Agent 级激活目录 `~/.mam/active/<tool>/<sub-agent>/skills/`
- **`store.rs`** — SQLite 数据层（`~/.mam/mam.db`）。
- **`commands.rs`** — 所有 `#[tauri::command]` IPC 处理器。
- **`terminal/`** — 通过 AppleScript 聚焦终端（iTerm2/Terminal.app）和 tmux。
- **`plugins/system_tray.rs`** — 系统托盘（状态指示 + 预设菜单）。

### 前端（React/TypeScript）— `src/`

- **`pages/`** — 路由页面：首页、设置、关于。
- **`components/`** — UI 组件：会话卡片、资源管理双视图、预设组、MCP 面板等。
- **`stores/sessionStore.ts`** — Zustand 会话状态存储。
- **`hooks/`** — `useSessions`（轮询）、`useNotification`、`useUpdater`。
- **`lib/`** — 音频（Web Audio）、快捷键、截图工具。
- **`i18n/`** — i18next，支持中文和英文。
- **`tauri-mock.ts`** — 浏览器/Playwright 渲染时的 Tauri API Mock。

### IPC 数据流

```
前端 (invoke) → commands.rs → manager/linker/adapter → store.rs (SQLite)
                                      ↓
                                文件系统（符号链接、配置文件）
```

### 三层 Skill 映射

```
Layer 1 (SSOT):    ~/.mam/skills/brainstorming/SKILL.md
                         ↓ 符号链接
Layer 2 (Tool):    ~/.mam/active/claude/skills/brainstorming → Layer 1
                         ↓ 符号链接
Layer 3 (SubAgent):~/.mam/active/claude/sub-agent-1/skills/brainstorming → Layer 2
```

Layer 3 指向 Layer 2（而非 Layer 1），因此工具级禁用会自动断开所有子 Agent 链接。

### Agent Adapter 模式

每个工具实现 `AgentAdapter` trait：
- `find_processes()` — 通过 `sysinfo` 扫描运行进程
- `find_sessions()` — 解析工具特定的会话文件
- `mcp_format()` / `mcp_config_path()` — MCP 配置格式
- `skill_dirs()` — 工具读取 skill 的目录
- `hook_supported()` — 是否支持状态 Hook

添加新工具：实现 `AgentAdapter`，在 `adapter/mod.rs` 的 `get_all_adapters()` 中注册。

## 数据目录

应用数据存储在 `~/.mam/`：

| 路径 | 用途 |
|------|------|
| `~/.mam/mam.db` | SQLite 数据库 |
| `~/.mam/skills/` | 全局 Skill 仓库 |
| `~/.mam/mcp/` | 全局 MCP 服务器配置 |
| `~/.mam/hooks/status-hook.sh` | 共享 Hook 脚本 |
| `~/.mam/events/` | Hook 事件文件（自动清理，30 秒 TTL） |

## 贡献者行为准则

### 我们的承诺

为了营造一个开放和友好的环境，我们作为贡献者和维护者承诺：无论年龄、体型、残疾、种族、性别特征、性别认同和表达、经验水平、教育程度、社会经济地位、国籍、个人外貌、种族、宗教或性取向如何，参与我们项目的每个人都将受到尊重。

### 我们的标准

有助于创造积极环境的行为包括：

- 使用友好和包容的语言
- 尊重不同的观点和经验
- 优雅地接受建设性批评
- 关注对社区最有利的事情
- 对其他社区成员表示同理心

不可接受的行为包括：

- 使用性化语言或图像，以及不受欢迎的性关注或性挑逗
- 挑衅、侮辱/贬损性评论，以及个人或政治攻击
- 公开或私下的骚扰
- 未经明确许可发布他人的私人信息，如物理或电子地址
- 其他在专业环境中被合理认为不恰当的行为

### 我们的责任

项目维护者有责任明确可接受行为的标准，并针对任何不可接受行为采取适当和公平的纠正措施。

项目维护者有权利和责任删除、编辑或拒绝不符合本行为准则的评论、提交、代码、wiki 编辑、issue 和其他贡献，或暂时或永久禁止任何贡献者的其他行为被认为不适当、威胁、冒犯或有害。

### 适用范围

本行为准则适用于所有项目空间，也适用于个人在公共空间代表项目或其社区时。

### 执行

可以通过 GitHub Issue 向项目团队报告辱骂、骚扰或其他不可接受的行为。所有投诉都将被审查和调查，并将根据情况作出必要和适当的回应。

### 来源

本行为准则改编自 [Contributor Covenant](https://www.contributor-covenant.org)，版本 1.4。
