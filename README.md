<div align="center">

# MultiAgents Manager

**多 Agent 编程工具统一管理平台**

一站式监控、通知、跳转、管理 Claude Code / Codex CLI / OpenCode / OpenClaw 的桌面应用

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Tauri v2](https://img.shields.io/badge/Tauri-v2-blue?logo=tauri)](https://v2.tauri.app/)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?logo=react)](https://react.dev/)

[English](README.en.md) · 中文

</div>

---

## 功能概览

### 会话监控看板

实时红绿灯状态看板，一目了然掌握所有 AI 编程工具的运行状态。

| 状态 | 含义 |
|------|------|
| 🔴 红色 | 等待用户输入 |
| 🟡 黄色 | 处理中 / 思考中 |
| 🟢 绿色 | 空闲 / 已完成 |

- 自动发现运行中的 **Claude Code**、**Codex CLI/APP**、**OpenCode**、**OpenClaw** 会话
- 区分 CLI 与桌面 APP 形态（APP 仅显示状态，不支持终端跳转）
- 显示项目名称、Git 分支、最后消息预览、CPU 占用、运行时长
- 按优先级排序：等待中 → 运行中 → 空闲
- 系统托盘图标反映聚合状态（🔴/🟡/🟢）

### 桌面通知与提示音

- 状态颜色变化时发送桌面通知（红↔黄↔绿），带去重机制
- Web Audio API 提示音，无需音频文件
- 设置中可开关
- 通知可点击，附带「查看会话」动作直接跳转终端

### 快速跳转终端

点击会话卡片即可瞬间聚焦对应终端标签页：

| 终端 | 支持情况 |
|------|---------|
| iTerm2 | ✅ AppleScript |
| Terminal.app | ✅ AppleScript |
| tmux | ✅ pane 选择 + 终端聚焦 |
| Wayland | ❌ 优雅降级提示 |

### 扩展资源统一管理

Skill / MCP 服务器 / 插件的统一仓库，一键映射到各工具：

- **Skill**：符号链接（Unix）/ 交接点（Windows）映射到各工具的 skill 目录
- **MCP 服务器**：自动格式转换 —— JSON（Claude）/ TOML（Codex）/ JSONC（OpenCode）
- **插件**：文件/配置混合管理
- 首次启动自动导入已有 skill（从 `~/.claude/skills/`、`~/.agents/skills/`、`~/.config/opencode/skills/`）
- 重新扫描按钮发现新安装的 skill

### 预设组一键切换

将 Skill + MCP + 插件打包为命名预设组，一键应用/取消：

- 一键应用到任意工具 —— 自动适配各工具的配置格式
- 部分成功处理：失败项报告错误，不回滚已成功项
- 冲突检测：跳过已存在的资源
- 系统托盘菜单集成，快速切换

### 子 Agent 级资源分配

为多 Agent 工具（Hermes、OpenCode 等）的子角色分配资源子集：

- 子 Agent 分配受工具级启用范围约束
- 工具级禁用自动级联到所有子 Agent

---

## 技术栈

| 层级 | 技术 |
|------|------|
| 桌面框架 | [Tauri v2](https://v2.tauri.app/)（Rust） |
| 前端 | [React 19](https://react.dev/) + [TypeScript](https://www.typescriptlang.org/) |
| UI 组件 | [shadcn/ui](https://ui.shadcn.com/)（Radix UI） |
| 样式 | [Tailwind CSS v4](https://tailwindcss.com/) |
| 状态管理 | [Zustand](https://zustand-demo.pmnd.rs/) |
| 国际化 | [i18next](https://www.i18next.com/)（中文 / English） |
| 数据库 | [SQLite](https://www.sqlite.org/)（via [rusqlite](https://github.com/rusqlite/rusqlite)） |
| 进程监控 | [sysinfo](https://github.com/GuillaumeGomez/sysinfo) |

## 架构

```
src-tauri/src/
├── adapter/           # Agent 适配器 trait + 各工具实现
│   ├── claude.rs      #   Claude Code（JSONL + Hook）
│   ├── codex.rs       #   Codex CLI/APP（JSONL + Hook）
│   ├── opencode.rs    #   OpenCode（SQLite）
│   ├── openclaw.rs    #   OpenClaw（state.json）
│   └── mod.rs         #   AgentAdapter trait + 会话发现调度器
├── monitor/
│   ├── process.rs     #   进程发现（sysinfo 扫描）
│   ├── parser.rs      #   Claude & Codex JSONL 解析器
│   ├── opencode_parser.rs # OpenCode SQLite 解析器
│   ├── openclaw_parser.rs # OpenClaw state.json 解析器
│   ├── status.rs      #   纯消息状态判定
│   └── hooks.rs       #   Hook 注册 + 事件文件读取
├── manager/
│   ├── mod.rs         #   Skill 安装/启用/禁用 + 自动导入
│   ├── mcp.rs         #   MCP 配置写入（JSON/TOML/JSONC）
│   ├── preset.rs      #   预设应用/取消 + 兼容性检查
│   └── plugin.rs      #   插件管理
├── linker/
│   ├── mod.rs         #   符号链接/交接点管理 + 安全检查
│   ├── detector.rs    #   工具安装检测
│   ├── layer2.rs      #   Layer 2 工具级激活目录
│   └── layer3.rs      #   Layer 3 子 Agent 级激活目录
├── terminal/          #   终端聚焦（iTerm2/Terminal.app/tmux）
├── plugins/
│   └── system_tray.rs #   系统托盘（状态 + 预设菜单）
├── store.rs           #   SQLite 数据层
├── commands.rs        #   Tauri IPC 命令
├── session/           #   会话模型 + 状态枚举
└── lib.rs             #   应用入口 + 插件注册

src/
├── pages/             #   首页 / 设置 / 关于
├── components/
│   ├── SessionCard.tsx #   带状态灯的会话卡片
│   ├── SessionGrid.tsx #   看板网格
│   ├── ExtensionList.tsx # 双视图（按分类/按工具）资源管理
│   ├── ResourceByKindView.tsx # Skill/MCP/Plugin 三栏视图
│   ├── ResourceByToolView.tsx # 四工具卡片视图
│   ├── ImportDialog.tsx  #   原生资源扫描与导入
│   ├── CompatibilityDialog.tsx # 预设兼容性检查
│   ├── PresetList.tsx  #   预设组增删改查
│   └── ui/            #   shadcn/ui 基础组件
├── hooks/             #   useSessions, useNotification, useUpdater
├── stores/            #   Zustand 会话存储
├── lib/               #   音频、快捷键、更新器、窗口工具
├── i18n/              #   中文 + 英文语言包
└── types/             #   TypeScript 类型定义
```

---

## 快速开始

### 环境要求

- [Node.js](https://nodejs.org/) ≥ 18
- [pnpm](https://pnpm.io/) ≥ 8
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.77
- [Tauri v2 CLI](https://v2.tauri.app/start/prerequisites/)

### 安装与运行

```bash
# 克隆仓库
git clone https://github.com/YOUR_USERNAME/MultiAgents-Manager.git
cd MultiAgents-Manager

# 安装前端依赖
pnpm install

# 启动开发模式
pnpm tauri:dev
```

### 构建

```bash
# 构建发布版（Windows NSIS 安装包）
pnpm tauri:build
```

### 代码检查与格式化

```bash
pnpm check        # format:check + lint + build
pnpm format       # Prettier 自动格式化
pnpm lint         # ESLint 检查
pnpm lint:fix     # ESLint 自动修复
```

---

## 配置

应用数据存储在 `~/.mam/`：

| 路径 | 用途 |
|------|------|
| `~/.mam/mam.db` | SQLite 数据库（设置、扩展、预设、会话缓存） |
| `~/.mam/skills/` | 全局 Skill 仓库 |
| `~/.mam/mcp/` | 全局 MCP 服务器配置 |
| `~/.mam/hooks/status-hook.sh` | 共享 Hook 脚本（状态事件） |
| `~/.mam/events/` | Hook 事件文件（自动清理，30 秒 TTL） |

### 各工具配置支持

| 工具 | Skill 目录 | MCP 配置 | MCP 格式 | Hook 支持 |
|------|-----------|----------|----------|----------|
| Claude Code | `~/.claude/skills/` | `~/.claude.json` | JSON | ✅（PascalCase） |
| Codex CLI | `~/.agents/skills/` | `~/.codex/config.toml` | TOML | ✅（camelCase） |
| OpenCode | `~/.config/opencode/skills/` | `~/.config/opencode/opencode.json` | JSONC | ❌ |
| OpenClaw | `~/.openclaw/skills/` | N/A | N/A | ❌ |

---

## 路线图

- [x] US1 — 多工具会话监控看板
- [x] US2 — 状态变更通知与提示音
- [x] US3 — 终端快速跳转（iTerm2/Terminal.app/tmux）
- [x] US4 — Skill/MCP/Plugin 统一仓库管理
- [x] US5 — 预设组一键切换
- [x] US6 — 子 Agent 级资源分配
- [x] 资源看板重设计（双视图 + 导入 + 兼容性）
- [x] OpenClaw 支持（第四工具）
- [x] 插件管理（文件/配置混合）
- [x] i18n（中文 + English）
- [x] GitHub Releases 自动更新
- [x] 暗色/亮色主题跟随系统
- [ ] Linux & Windows 支持（当前以 macOS 为主）
- [ ] Kitty & WezTerm 终端跳转支持

---

## 参与贡献

欢迎提交 Pull Request！

1. Fork 本仓库
2. 创建功能分支（`git checkout -b feature/amazing-feature`）
3. 提交更改（`git commit -m 'feat: add amazing feature'`）
4. 推送到分支（`git push origin feature/amazing-feature`）
5. 打开 Pull Request

请阅读 [CLAUDE.md](CLAUDE.md) 了解项目架构与开发规范。

---

## 许可证

本项目采用 MIT 许可证 —— 详见 [LICENSE](LICENSE) 文件。
