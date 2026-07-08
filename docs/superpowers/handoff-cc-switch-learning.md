# 学习 cc-switch 项目结构 — 改进规划 Handoff

**生成时间**: 2026-07-08
**上下文**: MultiAgents Manager 刚完成资源看板重构（18 个任务），已合并到 main 分支，DMG 已打包（5.4MB）
**当前分支**: `main`
**最新 Commit**: `ac7d1ec` (fix: resolve TypeScript unused variable errors in tauri-mock.ts)

---

## 一、项目背景

MultiAgents Manager 是一个 **Tauri 2 桌面应用**（Rust + React 19 + TypeScript），用于统一管理多个 AI 编程工具（Claude Code / Codex CLI / OpenCode / OpenClaw）。

**技术栈**:
| 层 | 技术 |
|----|------|
| 桌面框架 | Tauri 2 (Rust) |
| 前端 | React 19 + TypeScript |
| UI 组件 | shadcn/ui (Radix UI) + Tailwind CSS v4 |
| 状态管理 | Zustand |
| 数据库 | SQLite (rusqlite) |
| 进程监控 | sysinfo |
| i18n | i18next (中文/英文) |
| 包管理 | pnpm |

## 二、参考项目：cc-switch

**仓库**: https://github.com/farion1231/cc-switch (114k stars)
**技术栈**: Tauri 2.8 + React 18 + TypeScript + Rust + Vite 7 + pnpm
**规模**: 1075 文件，119 目录
**license**: MIT

cc-switch 和我们的项目同为 **Tauri 2 桌面应用**，同样支持多 AI 工具（Claude/Codex/OpenCode/OpenClaw/Gemini/Hermes/Copilot），其项目组织方式值得学习。

### cc-switch 核心目录结构（关键差异点）

```
cc-switch/
├── .github/
│   ├── ISSUE_TEMPLATE/          # 4 个 YAML 模板 (bug/feature/doc/question)
│   └── workflows/               # ci.yml + release.yml + claude.yml + stale.yml + labeler.yml
├── assets/screenshots/          # 多语言应用截图
├── docs/
│   ├── guides/                  # 5 组三语指南 (en/ja/zh)
│   ├── release-notes/           # 每版本三语文档 (v3.6.0 ~ v3.16.5)
│   └── user-manual/{en,ja,zh}/  # 分章节用户手册 (31页/语言)
├── src/                         # --- 前端 ---
│   ├── components/
│   │   ├── providers/           # 67 文件 — 按工具拆分的 Provider 表单
│   │   ├── sessions/            # SessionManagerPage, SessionItem
│   │   ├── skills/              # SkillsPage, SkillCard, RepoManager
│   │   ├── mcp/                 # McpFormModal, UnifiedMcpPanel
│   │   ├── settings/            # 21 个设置组件
│   │   ├── proxy/               # ProxyPanel, FailoverToggle
│   │   ├── common/              # 通用组件
│   │   └── ui/                  # 23 个 shadcn/ui 组件
│   ├── config/                  # 15 个预设/常量文件
│   ├── hooks/                   # 27 个自定义 hooks
│   ├── lib/
│   │   ├── api/                 # 26 个 Tauri invoke API 封装（核心模式！）
│   │   ├── query/               # TanStack Query (queries, mutations, queryClient)
│   │   └── schemas/             # 4 个 Zod 验证 schemas
│   └── utils/                   # 12 个工具模块
├── src-tauri/src/               # --- 后端 ---
│   ├── commands/                # 34 个命令文件 (mod.rs + 33 子模块)
│   ├── database/                # mod.rs, schema.rs, migration.rs, tests.rs
│   │   └── dao/                 # 13 个 DAO 模块（核心模式！）
│   ├── services/                # 39 个服务文件 + provider/ 子目录
│   └── proxy/                   # 63 文件 — 本地代理服务器
├── src-tauri/tests/             # 12 个 Rust 集成测试
├── tests/                       # 70 个前端测试 (vitest)
│   ├── components/              # 28 个组件测试
│   ├── hooks/                   # 16 个 hook 测试
│   ├── msw/                     # Mock Service Worker (tauriMocks.ts)
│   └── utils/                   # 6 个工具测试
├── CHANGELOG.md
├── CONTRIBUTING.md
├── SECURITY.md
├── SUPPORT.md
└── README.md + README_ZH.md + README_JA.md + README_DE.md
```

## 三、我们当前的核心文件结构（需要改进的现状）

```
MultiAgents-Manager/
├── .gitignore                # 需添加 TEST/ .playwright-mcp/
├── AGENTS.md                 # 存在，简短
├── CLAUDE.md                 # ✅ 刚创建
├── README.md                 # ✅ 刚更新
├── src-tauri/src/
│   ├── commands.rs           # ❌ 单文件 800+ 行
│   ├── store.rs              # ❌ 单文件 600+ 行 (SQLite)
│   ├── manager/              # ❌ 命名不清，含 mod/mcp/preset/plugin
│   ├── adapter/              # ✅ 良好
│   ├── monitor/              # ✅ 良好
│   ├── linker/               # ✅ 良好
│   └── terminal/             # ✅ 良好
├── src/
│   ├── components/           # ❌ 扁平结构 ~20 文件
│   ├── hooks/                # 4 个 hooks
│   ├── lib/                  # 零散工具函数
│   │   ├── audio.ts
│   │   ├── screenshot.ts
│   │   ├── shortcut.ts
│   │   ├── updater.ts
│   │   ├── window.ts
│   │   └── utils.ts
│   ├── stores/               # sessionStore.ts
│   ├── i18n/                 # en.json, zh.json
│   ├── types/                # session.ts, extension.ts, preset.ts
│   └── tauri-mock.ts         # ✅ 浏览器渲染 mock
├── TEST/                     # 临时截图目录（需迁移到 assets/screenshots/）
├── specs/001-multi-agent-platform/  # 已有 spec 1
├── docs/
│   ├── AUTO_UPDATE.md        # 中英版本
│   ├── GLOBAL_SHORTCUT.md
│   └── I18N.md
└── .specify/memory/constitution.md  # 项目宪法
```

---

## 四、任务：编写 3 个 Spec 文件

### Spec 1: 代码架构重构 → `specs/002-code-architecture-refactor/spec.md`

**优先级**: 第一（先重构再写测试）
**包含 7 个改进项**:

#### 1. commands.rs 拆分
- **当前**: `src-tauri/src/commands.rs` 单文件 800+ 行，所有 IPC 命令混在一起
- **cc-switch 做法**: `src-tauri/src/commands/` 目录含 mod.rs + 34 个独立文件
- **目标**: 拆为 `commands/{mod,session,resource,preset,skill,mcp,plugin,screenshot}.rs`
- **验证标准**: `cargo check` 通过，所有 `invoke` 调用功能不变

#### 2. store.rs → database/ DAO
- **当前**: `store.rs` 单文件 600+ 行，schema + CRUD + migration 混在一起
- **cc-switch 做法**: `database/` 含 mod.rs + schema.rs + migration.rs + backup.rs + `dao/` 子目录(13个)
- **目标**: 拆为 `database/{mod,schema,migration}.rs` + `database/dao/{session,extension,preset,settings,agent_tool}.rs`
- **验证标准**: 所有引入 `store` 的地方编译通过

#### 3. components/ 子目录
- **当前**: `src/components/` 扁平结构 ~20 文件
- **cc-switch 做法**: 按功能域分组 (providers/, sessions/, skills/, settings/, mcp/, ui/)
- **目标**: 拆为 `components/{sessions,resources,presets,settings,mcp,ui}/`
- **验证标准**: `tsc --noEmit` 通过

#### 4. src/lib/api/ 层
- **当前**: 组件/hooks 直接调用 `invoke()`，无统一 API 层
- **cc-switch 做法**: `src/lib/api/` 含 26 个按功能拆分的 API 模块
- **目标**: 创建 `src/lib/api/{session,resource,preset,mcp,skill,settings}.ts`
- **关键设计**: 每个模块导出类型安全的 async 函数，组件只调用这些函数

#### 5. manager/ → services/ 层
- **当前**: `src-tauri/src/manager/` 含 mod.rs + mcp.rs + preset.rs + plugin.rs
- **cc-switch 做法**: `src-tauri/src/services/` 含 39 个服务文件 + provider/ 子目录
- **目标**: 重命名 + 拆分为 `services/{resource,preset,skill,mcp,plugin}/`
- **注意**: 只改命名和文件组织，不改公共 API。linker/, terminal/ 保持不变

#### 6. src/config/ 预设集中
- **当前**: 常量和预设分散在各组件中
- **cc-switch 做法**: `src/config/` 含 15 个预设/常量文件
- **目标**: 创建 `src/config/{constants,tool-presets,skill-presets,mcp-presets}.ts`

#### 7. src/lib/schemas/ 验证
- **当前**: API 响应无运行时验证
- **cc-switch 做法**: `src/lib/schemas/` 含 Zod schemas (common.ts, mcp.ts, provider.ts, settings.ts)
- **目标**: 创建 `src/lib/schemas/{session,extension,preset,settings}.ts`
- **依赖**: 需要先装 `zod` 包

### Spec 2: 测试体系 → `specs/003-test-infrastructure/spec.md`

**优先级**: 第二（架构重构后才写测试）
**包含 3 个改进项**:

#### 8. Rust 集成测试目录
- **当前**: 内联 `#[cfg(test)]` 仅 3 个模块 (adapter/mod.rs, linker/layer2.rs, linker/layer3.rs)
- **cc-switch 做法**: `src-tauri/tests/` 含 12 个独立测试文件 + support.rs
- **目标**: 创建 `src-tauri/tests/{commands,dao,linker,services}_test.rs` + support.rs
- **验证标准**: `cargo test` 全部通过

#### 9. 前端 vitest
- **当前**: 无前端测试
- **cc-switch 做法**: `tests/` 含 70 文件，分 components(28)/hooks(16)/integration(2)/config(10)/utils(6)
- **依赖**: 需要先装 vitest, @testing-library/react, jsdom 等包
- **目标**: 创建 `vitest.config.ts` + `tests/{components,hooks,integration,msw}/`

#### 10. MSW + Tauri Mock
- **当前**: 有 `src/tauri-mock.ts`（浏览器渲染用），但无测试 mock
- **cc-switch 做法**: `tests/msw/tauriMocks.ts` 统一 mock invoke() + handlers.ts + server.ts
- **目标**: 扩展 tauri-mock.ts 支持 vitest，创建 `tests/msw/` 目录

### Spec 3: 文档与流程 → `specs/004-docs-and-process/spec.md`

**优先级**: 第三（功能稳定后才补文档）
**包含 9 个改进项**:

| # | 改进项 | cc-switch 参考 | 目标 |
|---|--------|---------------|------|
| 11 | GitHub CI/CD | ci.yml(前端+后端并行检查) + release.yml(5平台构建) + stale.yml | `.github/workflows/{ci,release,stale}.yml` |
| 12 | Issue 模板 | 4 个 YAML 模板 (bug/feature/doc/question) + config.yml | `.github/ISSUE_TEMPLATE/` |
| 13 | 多语言指南 | 5 组三语指南 (en/ja/zh) | 覆盖 session/resource/preset 主题 |
| 14 | Release notes | 每版本三语文档 | `docs/release-notes/` |
| 15 | 用户手册 | 分章节手册 (31页/语言) | `docs/user-manual/{en,zh}/` |
| 16 | CHANGELOG.md | 完整版本变更记录 | 创建 CHANGELOG.md |
| 17 | CONTRIBUTING.md | 独立贡献指南 | 创建 CONTRIBUTING.md |
| 18 | SECURITY.md | 独立安全策略 | 创建 SECURITY.md |
| 19 | Screenshots | `assets/screenshots/` 多语言截图 | 从 TEST/ 迁移到 `assets/screenshots/` |

---

## 五、cc-switch 的关键架构模式（写 spec 时需要注意）

1. **Commands 按功能拆分** — 我们 commands.rs 800+ 行全在一个文件，cc-switch 拆为 34 个独立文件。每个文件对应一个功能域（auth, mcp, provider, skill, session 等）
2. **前端 API 层** — 所有 invoke() 调用在 `src/lib/api/` 统一封装，组件不直接调用 invoke。这是**最大的架构改进点**
3. **React Query 替代手动轮询** — 用 `@tanstack/react-query` 的 `useQuery` + `useMutation` 替代 `setInterval` 轮询。需要评估是否值得引入该依赖
4. **数据库 DAO 模式** — `database/dao/` 按实体拆分（providers, mcp, skills, profiles 等），每个 DAO 处理一个实体的 CRUD
5. **前端测试 MSW mock** — `tests/msw/tauriMocks.ts` 系统化模拟 Tauri API，比我们的 `tauri-mock.ts` 更完整

## 六、产出格式要求

每个 spec 文件需要包含以下章节（遵循 speckit 规范）：

1. **功能概要** - 概述本 spec 包含哪些改进项
2. **功能需求** - 每项的详细拆分方案、文件映射、迁移策略
3. **用户场景** - 虽然主要是重构，但应描述对用户和开发者体验的影响
4. **验收标准** - 怎么验证重构成功（如 `cargo check` + `tsc --noEmit` + 功能不变）
5. **风险评估** - 哪些重构可能破坏现有功能，回退策略
6. **假设与约束** - 如"不改公共 API"、"保持向后兼容"
7. **cc-switch 参考** - cc-switch 对应实现的路径和做法

**语言规范**（来自 AGENTS.md）:
- 所有设计文档、规格说明使用**中文撰写**
- 代码标识符使用英文
- 代码注释尽量使用中文

## 七、参考文件

- 项目宪法: `.specify/memory/constitution.md`
- 项目指令: `AGENTS.md`
- 项目说明: `CLAUDE.md`
- 已有 Spec 模板: `specs/001-multi-agent-platform/spec.md`
- cc-switch 仓库: https://github.com/farion1231/cc-switch
- cc-switch GitHub API 目录树: `https://api.github.com/repos/farion1231/cc-switch/git/trees/main?recursive=1`
