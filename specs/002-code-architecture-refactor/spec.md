# 功能规格说明：代码架构重构

**功能分支**：`002-code-architecture-refactor`

**创建日期**：2026-07-08

**状态**：草稿

**输入**：参考 cc-switch 项目（114k stars）的代码组织方式，对 MultiAgents Manager 当前代码结构进行重构，提升可维护性、可测试性和开发效率。

## 用户场景与测试

### 用户故事 1 — 开发者快速定位代码（优先级: P1）

新加入的开发者需要理解项目结构并快速定位到需要修改的文件。当前 `commands.rs` 单文件 542 行、`store.rs` 单文件 561 行、components 扁平结构 20+ 文件，导致代码定位困难。

**优先级理由**：代码组织是开发效率的基础，直接影响后续功能迭代速度和新人上手成本。

**独立测试**：新开发者能在 5 分钟内找到任意功能的代码位置。

**验收场景**：

1. **给定** 开发者需要修改"会话状态查询"的 IPC 命令，**当** 查看 `src-tauri/src/commands/` 目录，**则** 能在 `session.rs` 中找到对应实现，而非在 542 行的 `commands.rs` 中搜索
2. **给定** 开发者需要修改"数据库会话表"的 schema，**当** 查看 `src-tauri/src/database/` 目录，**则** 能在 `schema.rs` 中找到表结构定义，在 `dao/session.rs` 中找到 CRUD 实现
3. **给定** 开发者需要修改"会话卡片"组件，**当** 查看 `src/components/sessions/` 目录，**则** 能在 `SessionCard.tsx` 和 `SessionGrid.tsx` 中找到对应组件

### 用户故事 2 — 避免组件间耦合（优先级: P2）

当前前端组件直接调用 `invoke()`，API 调用分散在各组件中。当后端命令签名变更时，需要逐个文件修改。

**优先级理由**：API 层封装是前后端解耦的关键，参考 cc-switch 的 `src/lib/api/` 模式。

**独立测试**：修改一个后端命令参数后，只需修改对应 API 模块一处。

**验收场景**：

1. **给定** 后端 `get_all_sessions` 命令新增了一个可选参数，**当** 修改 `src/lib/api/session.ts` 中的函数签名，**则** 所有调用该 API 的组件自动获得类型提示，无需逐个修改
2. **给定** 需要为某个 API 添加统一的错误处理（如重试逻辑），**当** 在对应 API 模块中修改，**则** 所有调用方自动获得该行为

### 用户故事 3 — 后端业务逻辑分层（优先级: P3）

当前 `manager/` 目录命名不清，包含 MCP/预设/插件管理，但缺少清晰的分层。参考 cc-switch 的 `services/` 模式，将业务逻辑按功能域拆分。

**优先级理由**：manager 层是后端核心，分层清晰后便于单元测试和后续扩展。

**独立测试**：每个 service 模块能独立编译和测试。

**验收场景**：

1. **给定** 需要测试"预设组应用"逻辑，**当** 查看 `src-tauri/src/services/preset/` 目录，**则** 能找到独立的 preset service 模块及其测试文件
2. **给定** 需要新增一个"资源同步"功能，**当** 在 `src-tauri/src/services/resource/` 中创建新模块，**则** 不与其他 service 模块产生循环依赖

### 用户故事 4 — 数据获取与状态管理现代化（优先级: P4）

当前前端使用手动 `setInterval` 轮询（`useSessions` hook），存在竞态条件、无缓存、无自动刷新控制等问题。React Query 能解决这些问题，同时减少样板代码。

**优先级理由**：数据获取是前端核心 loop，React Query 的缓存/去重/后台刷新机制能显著提升用户体验和代码质量。cc-switch 已在全项目范围使用该模式。

**独立测试**：同一会话数据在多个组件中渲染，验证网络请求只发送一次（缓存命中），且状态变更时自动刷新。

**验收场景**：

1. **给定** 用户打开看板页面，**当** `useQuery` 首次获取会话列表，**则** 加载状态正确显示（骨架屏），数据到达后渲染会话卡片
2. **给定** 看板已加载数据，**当** 用户切换到设置页面再切回看板，**则** 看板先展示缓存数据（瞬时渲染），后台静默刷新
3. **给定** 后端返回错误，**当** `useQuery` 捕获异常，**则** UI 显示错误状态而非空白页面或崩溃

### 用户故事 5 — 错误边界与优雅降级（优先级: P5）

当前应用无 React 错误边界，任意组件渲染错误会导致整个页面白屏。需要错误边界实现局部降级。

**优先级理由**：对于面向用户的桌面应用，白屏是不可接受的。React 官方推荐错误边界作为健壮性基线。

**独立测试**：模拟 `SessionCard` 组件渲染崩溃，验证错误边界捕获后看板其他区域正常显示。

**验收场景**：

1. **给定** 某个 `SessionCard` 组件因数据异常而崩溃，**当** 错误边界捕获，**则** 该卡片区域显示错误占位（"会话加载失败"），其他会话卡片和页面布局不受影响
2. **给定** 页面级组件崩溃，**当** 页面级错误边界捕获，**则** 显示"出错了"页面并提供"重试"和"回到首页"按钮

## 功能需求

### FR-1: commands.rs 拆分

1. 将 `src-tauri/src/commands.rs`（542 行）拆分为 `src-tauri/src/commands/` 目录结构：
   - `mod.rs` — 模块聚合，注册所有命令到 Tauri
   - `session.rs` — 会话相关命令（`get_all_sessions`, `get_session_detail`, `focus_session` 等）
   - `resource.rs` — 资源管理命令（`get_resources`, `enable_resource`, `disable_resource` 等）
   - `preset.rs` — 预设组命令（`get_presets`, `apply_preset`, `remove_preset` 等）
   - `skill.rs` — Skill 管理命令（`get_skills`, `install_skill`, `uninstall_skill` 等）
   - `mcp.rs` — MCP 管理命令（`get_mcp_servers`, `add_mcp_server`, `remove_mcp_server` 等）
   - `plugin.rs` — 插件管理命令（`get_plugins`, `install_plugin`, `uninstall_plugin` 等）
   - `screenshot.rs` — 截图工具命令（`capture_screenshot` 等）
   - `settings.rs` — 设置相关命令（`get_settings`, `update_settings` 等）

2. 每个子模块使用 `pub fn add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R>` 模式注册命令
3. `mod.rs` 中统一调用各子模块的 `add_commands` 完成注册
4. 保持所有命令的函数签名不变，确保前端 `invoke` 调用无需修改
5. 命令名不变：保留现有 `get_all_sessions`，不重命名为 `get_sessions`。

**cc-switch 参考**：`src-tauri/src/commands/` 含 `mod.rs` + 33 个子模块，每个模块对应一个功能域

### FR-2: store.rs → database/ DAO

1. 将 `src-tauri/src/store.rs`（561 行）拆分为 `src-tauri/src/database/` 目录结构：
   - `mod.rs` — 模块聚合，导出 database 公共接口
   - `schema.rs` — 数据库 schema 定义（所有 `CREATE TABLE` 语句）
   - `migration.rs` — 数据库迁移逻辑（版本升级脚本）
   - `connection.rs` — 数据库连接池管理（从 `store.rs` 中提取）
   - `dao/mod.rs` — DAO 模块聚合
   - `dao/session.rs` — Session 实体的 CRUD
   - `dao/extension.rs` — Extension（Skill/MCP/Plugin）实体的 CRUD
   - `dao/preset.rs` — Preset 实体的 CRUD
   - `dao/settings.rs` — Settings 实体的 CRUD
   - `dao/agent_tool.rs` — AgentTool 实体的 CRUD

2. 每个 DAO 模块提供标准接口：
   ```rust
   pub trait SessionDao {
       fn find_all(&self) -> Result<Vec<Session>>;
       fn find_by_id(&self, id: i64) -> Result<Option<Session>>;
       fn create(&self, session: &Session) -> Result<i64>;
       fn update(&self, id: i64, session: &Session) -> Result<()>;
       fn delete(&self, id: i64) -> Result<()>;
   }
   ```

3. 保持数据库文件路径不变（`~/.mam/mam.db`），保持表结构不变
4. 所有现有 `use crate::store::Store` 的引用改为 `use crate::database::{Database, SessionDao}`

**cc-switch 参考**：`src-tauri/src/database/` 含 `mod.rs` + `schema.rs` + `migration.rs` + `backup.rs` + `dao/`（13 个 DAO 模块）

### FR-3: components/ 子目录化

1. 将 `src/components/` 中 20+ 个文件按功能域拆分为子目录：
   ```
   src/components/
   ├── sessions/           # 会话相关组件
   │   ├── SessionCard.tsx
   │   ├── SessionGrid.tsx
   │   └── StatusLight.tsx
   ├── resources/          # 资源管理组件
   │   ├── ResourceByKindView.tsx
   │   ├── ResourceByToolView.tsx
   │   └── ExtensionList.tsx
   ├── presets/            # 预设组组件
   │   └── PresetList.tsx
   ├── settings/           # 设置相关组件
   │   ├── CompatibilityDialog.tsx
   │   ├── ImportDialog.tsx
   │   └── ScreenshotTool.tsx
   ├── mcp/                # MCP 管理组件
   │   └── McpManager.tsx
   ├── ui/                 # shadcn/ui 组件（已有，保持不变）
   └── common/             # 通用/布局组件
       ├── language-toggle.tsx
       ├── mode-toggle.tsx
       ├── shortcut-input.tsx
       ├── theme-provider.tsx
       ├── title-bar.tsx
       ├── main-title-bar.tsx
       ├── window-frame.tsx
       └── updater-dialog.tsx
   ```

2. 每个子目录创建 `index.ts` 统一导出，简化引用路径
3. 更新所有引用这些组件的文件路径

**cc-switch 参考**：`src/components/` 含 `providers/`, `sessions/`, `skills/`, `settings/`, `mcp/`, `proxy/`, `common/`, `ui/` 等子目录

### FR-4: src/lib/api/ 层

1. 创建 `src/lib/api/` 目录，按功能域封装 Tauri `invoke()` 调用：
   - `session.ts` — 会话相关 API（`getAllSessions()`, `focusSession()`, `getSessionDetail()` 等）
   - `resource.ts` — 资源管理 API（`getResources()`, `enableResource()`, `disableResource()` 等）
   - `preset.ts` — 预设组 API（`getPresets()`, `applyPreset()`, `removePreset()` 等）
   - `mcp.ts` — MCP 管理 API（`getMcpServers()`, `addMcpServer()`, `removeMcpServer()` 等）
   - `skill.ts` — Skill 管理 API（`getSkills()`, `installSkill()`, `uninstallSkill()` 等）
   - `settings.ts` — 设置 API（`getSettings()`, `updateSettings()` 等）

2. 每个 API 模块导出类型安全的 async 函数：
   ```typescript
   // src/lib/api/session.ts
   import { invoke } from "@tauri-apps/api/core";
   import type { SessionsResponse } from "@/types/session";

   export async function getAllSessions(): Promise<SessionsResponse> {
     return await invoke<SessionsResponse>("get_all_sessions");
   }

   export async function focusSession(sessionId: number): Promise<void> {
     return await invoke("focus_session", { sessionId });
   }
   ```

3. 组件和 hooks 不再直接调用 `invoke()`，而是通过 API 层间接调用
4. 保持 `tauri-mock.ts` 作为浏览器渲染时的 fallback

**cc-switch 参考**：`src/lib/api/` 含 26 个按功能拆分的 API 模块

### FR-5: manager/ → services/ 层

1. 将 `src-tauri/src/manager/` 重命名为 `src-tauri/src/services/`，并拆分为：
   ```
   src-tauri/src/services/
   ├── mod.rs              # 模块聚合
   ├── resource/           # 资源管理服务
   │   └── mod.rs
   ├── preset/             # 预设组服务
   │   └── mod.rs
   ├── skill/              # Skill 管理服务
   │   └── mod.rs
   ├── mcp/                # MCP 管理服务
   │   └── mod.rs
   └── plugin/             # 插件管理服务
       └── mod.rs
   ```

2. 每个 service 模块封装对应业务逻辑，对外暴露清晰的 trait 或函数接口
3. 保持公共 API 不变（commands.rs 调用的函数签名不变），只改内部文件组织
4. `linker/`、`adapter/`、`monitor/` 保持不变（`terminal/` 由 FR-5b 处理）

**cc-switch 参考**：`src-tauri/src/services/` 含 39 个服务文件 + `provider/` 子目录



### FR-5b: terminal/ -> window/ + WindowManager trait

1. 将 `src-tauri/src/terminal/` 改名为 `src-tauri/src/window/`
2. 把跳转抽象升格为 `WindowManager` trait，含 `focus(session)` 方法：
   - macOS 实现：iTerm2/Terminal.app/kitty/WezTerm/tmux（AppleScript）
   - Linux-X11：xdotool
   - Linux-Wayland：返回不支持，UI 提示
   - Windows：SetForegroundWindow
3. 公共 API 不变
4. 验证：`cargo check` 通过 + 手动 iTerm2/tmux 跳转验证

**对接宪法代码组织 [A5] 的 PATCH 修订。**

### FR-5c: notify 集成到 monitor

1. `monitor/` 模块集成 notify-rs 文件监听：Hook/进程事件文件变化时优先触发 notify 事件，定期轮询作为兜底
2. 保持 monitor 对外接口不变
3. 注意：这是「重构中的双策略接入点」，而非新功能--拆 monitor 时顺带补 notify 接入位

### FR-5d: IPC 实体对齐表

1. 重构期间建立 Rust <-> TypeScript 实体对齐表，覆盖 Session / SessionsResponse / Extension / Preset / AgentTool / SubAgent 共 6 个实体
2. 列出每个实体在 Rust struct 和 TS interface 间的字段对应
3. 重构 PR 验收时附该表作为静态检查文档
4. 示例骨架：

| Rust struct | TS interface | 字段映射说明 |
|-------------|-------------|-------------|
| `Session` | `Session` | `pid: u32` -> `pid: number` |
| `SessionsResponse` | `SessionsResponse` | `sessions: Vec<Session>` -> `sessions: Session[]` |
| `Extension` | `Extension` | `kind: String` -> `kind: "skill" \| "mcp" \| "plugin"` |
| `Preset` | `Preset` | ... |
| `AgentTool` | `AgentTool` | ... |
| `SubAgent` | `SubAgent` | ... |




### FR-6: src/config/ 预设集中

1. 创建 `src/config/` 目录，存放前端预设和常量：
   - `constants.ts` — 全局常量（轮询间隔、状态枚举、工具列表等）
   - `tool-presets.ts` — 各工具的预设配置（名称、图标、进程标识等）
   - `skill-presets.ts` — 常用 Skill 的预设列表
   - `mcp-presets.ts` — 常用 MCP 服务器的预设配置

2. 将当前分散在各组件中的硬编码常量迁移到 `constants.ts`

**cc-switch 参考**：`src/config/` 含 15 个预设/常量文件

### FR-7: src/lib/schemas/ 验证

1. 创建 `src/lib/schemas/` 目录，使用 Zod 定义运行时验证 schemas：
   - `session.ts` — Session 实体验证 schema
   - `extension.ts` — Extension 实体验证 schema
   - `preset.ts` — Preset 实体验证 schema
   - `settings.ts` — Settings 实体验证 schema

2. 每个 schema 定义对应的数据结构验证规则：
   ```typescript
   import { z } from "zod";

   export const SessionSchema = z.object({
     id: z.number(),
     tool: z.string(),
     project_path: z.string(),
     status: z.enum(["running", "waiting", "idle", "completed"]),
     pid: z.number().optional(),
     cpu_usage: z.number().optional(),
   });

   export type Session = z.infer<typeof SessionSchema>;
   ```

3. 在 API 层使用 schema 验证后端返回数据，确保类型安全
4. **依赖**：需要先安装 `zod` 包（`pnpm add zod`）

**cc-switch 参考**：`src/lib/schemas/` 含 `common.ts`, `mcp.ts`, `provider.ts`, `settings.ts`

### FR-8: React Query 替代手动轮询

1. 安装 `@tanstack/react-query`：`pnpm add @tanstack/react-query`
2. 创建 `src/lib/query/` 目录（参考 cc-switch）：
   - `queryClient.ts` — QueryClient 配置（staleTime、retry、refetchInterval）
   - `queries/sessions.ts` — 会话查询 hooks（`useSessionsQuery`、`useSessionDetailQuery`）
   - `queries/resources.ts` — 资源查询 hooks（`useResourcesQuery`、`useExtensionsQuery`）
   - `queries/presets.ts` — 预设组查询 hooks（`usePresetsQuery`）
   - `mutations/sessions.ts` — 会话变更 hooks（`useFocusSessionMutation`）
   - `mutations/resources.ts` — 资源变更 hooks（`useEnableResourceMutation`）
3. 重构 `useSessions` hook：
   ```typescript
   // src/hooks/useSessions.ts → 使用 React Query 替代 setInterval
   import { useQuery } from "@tanstack/react-query";
   import { getAllSessions } from "@/lib/api/session";
   import { POLL_INTERVAL } from "@/config/constants";

   export function useSessions() {
     return useQuery({
       queryKey: ["sessions"],
       queryFn: getAllSessions,
       refetchInterval: POLL_INTERVAL,  // 从常量取，替代手动 setInterval
      refetchIntervalInBackground: false,
       staleTime: 1000,
     });
   }
   ```
4. 在 `main.tsx` 中注册 `QueryClientProvider`：
   ```typescript
   import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
   const queryClient = new QueryClient({
     defaultOptions: { queries: { retry: 2, staleTime: 1000 } }
   });
   ```
5. **验证标准**：所有现有功能正常，且同一数据在多个组件中共享缓存（不重复请求）

**cc-switch 参考**：`src/lib/query/` — 使用 TanStack Query 的 `useQuery` + `useMutation`

### FR-9: React Error Boundaries

1. 创建 `src/components/common/ErrorBoundary.tsx`：
   ```typescript
   import { Component, type ReactNode } from "react";
   import { Button } from "@/components/ui/button";

   interface Props { children: ReactNode; fallback?: ReactNode; }
   interface State { hasError: boolean; error?: Error; }

   export class ErrorBoundary extends Component<Props, State> {
     state: State = { hasError: false };
     static getDerivedStateFromError(error: Error): State {
       return { hasError: true, error };
     }
     render() {
       if (this.state.hasError) {
         return this.props.fallback ?? (
           <div className="p-4 text-center">
             <p className="text-sm text-muted-foreground">组件加载失败</p>
             <Button onClick={() => this.setState({ hasError: false })}>重试</Button>
           </div>
         );
       }
       return this.props.children;
     }
   }
   ```
2. 创建 `src/components/common/PageErrorBoundary.tsx` — 页面级错误边界
3. 在关键页面包裹错误边界：
   - 首页看板：`<ErrorBoundary fallback={<SessionGridError />}><SessionGrid /></ErrorBoundary>`
   - 资源管理：`<ErrorBoundary><ResourceByKindView /></ErrorBoundary>`
   - 每个 `SessionCard`：`<ErrorBoundary fallback={<SessionCardError />}><SessionCard /></ErrorBoundary>`
4. **验证标准**：模拟组件崩溃，页面其他区域正常渲染，错误占位可见

### FR-10: Monorepo 工程化基础

1. 抽取共享 TypeScript 配置 `tsconfig.base.json`（参考 codex-plusplus 的 `tsconfig.base.json`）：
   ```json
   {
     "compilerOptions": {
       "target": "ESNext",
       "module": "ESNext",
       "moduleResolution": "bundler",
       "strict": true,
       "esModuleInterop": true,
       "skipLibCheck": true
     }
   }
   ```
2. 抽取共享 ESLint 配置 `eslint.config.base.js`
3. 统一 `.editorconfig`（EditorConfig 标准化编辑器行为）：
   ```ini
   root = true
   [*]
   indent_style = space
   indent_size = 2
   end_of_line = lf
   charset = utf-8
   trim_trailing_whitespace = true
   insert_final_newline = true
   ```
4. 如果未来需要拆分多包（如 SDK 包），可引入 `turborepo` 或 `nx` 管理任务编排

**codex-plusplus 参考**：`tsconfig.base.json` + `packages/` 多包 monorepo 结构

### FR-11: Pre-commit Hooks

1. 安装依赖：`pnpm add -D husky lint-staged`
2. `lint-staged` 配置（`package.json`）：
   ```json
   {
     "lint-staged": {
       "src/**/*.{ts,tsx}": ["prettier --write", "eslint --fix"],
       "src-tauri/**/*.rs": ["rustfmt --edition 2021"]
     }
   }
   ```
3. `husky` 配置 `.husky/pre-commit`：
   ```bash
   pnpm lint-staged
   pnpm build  # 快速检查 TypeScript 编译
   ```
4. 可选：`.husky/commit-msg` 配合 `commitlint` 强制执行 Conventional Commits

**cc-switch 参考**：husky + lint-staged，常规提交前自动格式化和 lint

### FR-12: 宪法代码组织 PATCH 修订

1. Spec 002 重构完成且 `cargo check` + `tsc --noEmit` 通过后，提交宪法 `.specify/memory/constitution.md` 的「开发流程 -> 代码组织」小节 PATCH 修订
2. 反映 `commands/`、`database/`、`services/`、`window/` 新结构
3. 同步传播到 plan/tasks 模板


## 成功标准

1. `cargo check` 通过，无编译错误
2. `tsc --noEmit` 通过，无 TypeScript 类型错误
3. 所有现有功能正常运行（手动验证会话看板、资源管理、预设组）
4. 代码行数分布更均匀，无单文件超过 300 行（auto-generated 除外）
5. 新开发者能在 5 分钟内找到任意功能的代码位置
6. 组件间通过 API 层通信，不直接调用 `invoke()`
7. 数据获取逻辑统一使用 React Query，无手动 `setInterval` 轮询
8. 关键页面和组件包裹错误边界，单个组件崩溃不影响整个页面
9. pre-commit hook 自动运行格式化和 lint，阻止不符合规范的代码提交

## 关键实体

| 实体 | 说明 | 关键属性 |
|------|------|---------|
| CommandModule | IPC 命令子模块 | 模块名、命令列表、注册函数 |
| Dao<T> | 数据访问对象 | 实体类型、CRUD 方法 |
| ApiModule | 前端 API 封装模块 | 功能域、invoke 封装函数、类型定义 |
| Service | 后端业务服务 | 功能域、业务逻辑、依赖的 DAO |
| Schema | Zod 验证规则 | 字段、类型、约束 |
| QueryHook | React Query hook | queryKey、queryFn、refetch 策略 |
| ErrorBoundary | 错误边界组件 | 捕获范围（页面/区块/组件）、fallback UI |

## 假设

1. 重构期间不修改任何业务逻辑，只改文件组织
2. 保持所有公共 API（Tauri commands 和前端组件 props）不变
3. 数据库 schema 和表结构不变
4. `zod` 包安装后不影响现有构建流程
5. 重构顺序见假设 9（后端从底层往上层拆）。
6. React Query 引入后，现有 Zustand store 逐步迁移，初期两者并存：Zustand 管理 UI 状态（侧边栏、主题），React Query 管理服务端数据（会话、资源）
7. husky 和 lint-staged 安装后不影响现有 CI 流程
8. Spec 003 的测试体量以本 spec 拆分后的模块为基准：核心模块 = DAO + Service + Linker + Manifest validator，commands 集成测试与 UI 组件测试为非核心通过测试。
9. 完整重构顺序：FR-2（database）-> FR-5（services）-> FR-5b（window）-> FR-1（commands 聚合层，此时改 use 引用）-> 前端 FR-3/4/8/9 -> FR-6/7 -> FR-10/11。理由：commands 引用 store/services 改名，先拆底层再拆命令层可少一道来回。

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 文件拆分引入 import 路径错误 | 中 | 使用 IDE 自动重构，拆分后运行 `cargo check` + `tsc --noEmit` |
| 模块循环依赖 | 中 | 拆分前绘制依赖图，确保新模块间无循环引用 |
| Zod 引入增加 bundle 大小 | 低 | Zod 体积约 15KB gzip，影响可接受 |
| 前端组件路径变更导致引用失效 | 中 | 更新 `tsconfig.json` paths，使用 IDE 批量重构 |
| React Query 与现有 Zustand 状态冲突 | 低 | 初期共存：Zustand 管理 UI 状态（侧边栏、主题），React Query 管理服务端数据（会话、资源） |
| 错误边界覆盖不全 | 低 | 从关键页面逐步覆盖，CI 中加 lint 规则检查关键组件是否包裹错误边界 |
| pre-commit hook 性能影响提交速度 | 低 | lint-staged 只检查变更文件，耗时 <3s |

## cc-switch 参考

- **Commands 拆分**：`src-tauri/src/commands/mod.rs` + 33 个子模块（`auth.rs`, `mcp.rs`, `provider.rs`, `skill.rs`, `session.rs` 等）
- **Database DAO**：`src-tauri/src/database/dao/` 含 13 个 DAO 模块（`providers.rs`, `mcp.rs`, `skills.rs`, `profiles.rs` 等）
- **前端 API 层**：`src/lib/api/` 含 26 个 API 模块（`session.ts`, `provider.ts`, `skill.ts`, `mcp.ts` 等）
- **Services 层**：`src-tauri/src/services/` 含 39 个服务文件 + `provider/` 子目录
- **组件子目录**：`src/components/` 含 `providers/`, `sessions/`, `skills/`, `settings/`, `mcp/`, `proxy/`, `common/`, `ui/`
- **React Query**：`src/lib/query/` — TanStack Query 替代手动轮询，含 queries/ + mutations/ 子目录

## codex-plusplus 参考

- **Monorepo 结构**：`packages/` 拆分为 installer/runtime/sdk/loader
- **共享 tsconfig**：`tsconfig.base.json` 统一 TypeScript 配置
- **Pre-commit hooks**：husky + lint-staged，在提交前自动格式化
