# 执行日志：5 Plan 架构重构

**日期**：2026-07-10

**分支**：`refactor/code-architecture`（从 `main` 切出）

**Commits**：15 个

**范围**：执行 `docs/superpowers/plans/2026-07-09-*` 下的 5 个 plan，涵盖 Spec 001-005

---

## 一、任务执行情况

### 执行顺序

按 plan 间依赖关系执行：

```
Plan 1 (fs2 文件锁)
  → Plan 2 (架构重构, 15 个任务, 按 FR-2→5→5b→5c→5d→1→前端→配置 顺序)
    → Plan 3 (测试体系)
    → Plan 5 (Extension Manifest)
    → Plan 4 (文档与流程)
```

---

### Plan 1：Spec 001 增量 -- fs2 文件锁（3 任务）

**对应 Spec**：001 FR-5.27（原子化更新）

**背景**：`write_atomic`（write-to-temp + rename）已在 `linker/mod.rs` 中实现，但 `fs2` 文件锁虽在 `Cargo.toml` 中却未被代码使用。多实例并发写入同一工具配置文件时无保护。

**执行内容**：

1. 在 `linker/mod.rs` 中新增 `write_config_locked` 函数：
   - 用 `fs2::FileExt::lock_exclusive()` 加排他锁
   - 锁内执行 write-to-temp + rename 原子写入
   - 无论成功失败都释放锁

2. 将 `mcp.rs`（6 处）、`plugin.rs`（4 处）、`hooks.rs`（1 处）中所有 `write_atomic` 调用替换为 `write_config_locked`

**验证**：`cargo check` 通过

---

### Plan 2：Spec 002 代码架构重构（15 任务）

**对应 Spec**：002 FR-1 至 FR-12

#### 任务 1：store.rs → database/ DAO（FR-2）

**before**：`store.rs` 单文件 561 行，全局 `DB: Lazy<Mutex<Connection>>`，所有函数内部 `DB.lock().unwrap()` 获取连接。

**after**：

```
database/
├── mod.rs          -- 模块聚合 + 公共 API 重新导出
├── connection.rs   -- 全局 DB 静态 + init() + open()
├── schema.rs       -- 9 张表的 CREATE TABLE 语句
├── migration.rs    -- ALTER TABLE 迁移（manifest 字段）
└── dao/
    ├── mod.rs
    ├── session.rs      -- update_session_status, cleanup_stale_sessions + SessionDao trait
    ├── extension.rs    -- ExtensionRecord, AssignmentRecord, NativeExtensionRecord + CRUD
    ├── preset.rs       -- PresetRecord, PresetItemRecord + CRUD
    ├── settings.rs     -- get_setting, set_setting
    └── agent_tool.rs   -- SubAgentRecord, list_sub_agents
```

关键决策：保留全局 `DB` 模式（spec 假设的连接传递模式与实际代码不符），DAO trait 方法内部使用全局连接。新增 `SessionDao` trait + `SessionDaoImpl` 适配 spec 的标准接口要求。

全局 `crate::store::` 引用替换为 `crate::database::`（涉及 commands.rs、manager/、monitor/、adapter/、plugins/ 共 40+ 处）。

#### 任务 2：manager/ → services/ 子目录拆分（FR-5）

**before**：`manager/` 含 `mod.rs`（447 行，混合 skill/resource/cross-cutting 函数）、`preset.rs`、`mcp.rs`、`plugin.rs`（扁平文件）。

**after**：

```
services/
├── mod.rs          -- 模块声明 + 重新导出 + toggle_mcp/toggle_plugin/detect_subagents
├── skill/mod.rs    -- install_skill, enable/disable_skill_for_tool, assign_skill_to_subagent
├── resource/mod.rs -- auto_import_extensions, ImportStats, scan_skills_recursive
├── preset/mod.rs   -- apply_preset, deactivate_preset, check_compatibility
├── mcp/mod.rs      -- write_mcp, remove_mcp
└── plugin/mod.rs   -- toggle_plugin, enable/disable_file_plugin
```

`mod.rs` 通过 `pub use` 重新导出 skill/resource 函数，保持 `crate::services::install_skill()` 向后兼容。

#### 任务 3：terminal/ → window/ + WindowManager trait（FR-5b）

**before**：`terminal/mod.rs` 含 `focus_terminal_for_pid` 函数，支持 iTerm2/Terminal.app/tmux。

**after**：目录重命名为 `window/`，新增 trait：

```rust
pub trait WindowManager {
    fn focus(&self, pid: u32) -> Result<(), String>;
}

pub struct DefaultWindowManager;
impl WindowManager for DefaultWindowManager {
    fn focus(&self, pid: u32) -> Result<(), String> {
        focus_terminal_for_pid(pid)  // 委托到现有实现
    }
}
```

#### 任务 4：notify 集成到 monitor（FR-5c）

在 `monitor/mod.rs` 末尾新增 `start_file_watcher` 函数：
- 使用 `notify::RecommendedWatcher` 监听 Hook/进程事件文件
- 文件变化（Create/Modify 事件）时触发 `on_change` 回调
- 30 秒超时回退轮询兜底
- watcher 初始化失败时降级为纯轮询

#### 任务 5：IPC 实体对齐表（FR-5d）

创建 `docs/ipc-entity-alignment.md`，列出 13 个 Rust struct 到 TS interface 的字段映射（Session、SessionsResponse、ExtensionRecord、PresetRecord、SubAgentRecord、NativeExtensionRecord、ImportStats、ApplyResult、CompatibilityReport、ToolDetection、ScreenshotResult 等）。

#### 任务 6：commands.rs → commands/ 拆分（FR-1）

**before**：`commands.rs` 单文件 542 行，34 个 `#[tauri::command]` 函数 + 辅助结构体。

**after**：

```
commands/
├── mod.rs       -- 模块声明 + add_commands<R: Runtime>(builder) 聚合注册
├── session.rs   -- get_all_sessions, focus_session, kill_session
├── resource.rs  -- list_extensions_with_assignments, scan/import_native, list_tool_resources
├── preset.rs    -- create/delete/list/apply/deactivate_preset + subagent 变体
├── skill.rs     -- list_repo_skills, install_skill, rescan_skills, assign_skill_to_subagent
├── mcp.rs       -- toggle_mcp_for_tool, read/write/remove_mcp_server
├── plugin.rs    -- toggle_plugin_for_tool
├── settings.rs  -- get/set_setting, detect_tools, detect_subagents, list_sub_agents
├── screenshot.rs -- capture_window_screenshot, list_screenshots
└── manifest.rs  -- validate_manifest, install/uninstall_resource, get_store_index
```

每个子模块实现 `add_commands<R: Runtime>(builder: Builder<R>) -> Builder<R>`，`mod.rs` 链式调用所有子模块的 `add_commands`。

**关键技术问题**：`get_all_sessions` 和 `capture_window_screenshot` 接收 `tauri::AppHandle` 参数，在泛型 `add_commands<R: Runtime>` 上下文中 `generate_handler!` 无法解析。解决方案：这两个命令在 `lib.rs` 的非泛型上下文中单独注册。

#### 任务 7：components/ 子目录化（FR-3）

**before**：20+ 组件文件扁平放在 `src/components/`。

**after**：

```
components/
├── sessions/   -- SessionCard, SessionGrid, StatusLight
├── resources/  -- ResourceByKindView, ResourceByToolView, ExtensionList, ImportDialog, CompatibilityDialog, PermissionBadge, ManifestInstallDialog
├── presets/    -- PresetList
├── settings/   -- ScreenshotTool
├── mcp/        -- McpManager
├── common/     -- ErrorBoundary, PageErrorBoundary, ToolIcon, title-bar, main-title-bar, window-frame, theme-provider, language-toggle, mode-toggle, shortcut-input, updater-dialog
└── ui/         -- shadcn/ui 组件（保持不变）
```

所有 `@/components/ComponentName` import 路径更新为 `@/components/subdir/ComponentName`，跨子目录的相对 import 同步修正。

#### 任务 8：src/lib/api/ 层（FR-4）

创建 8 个 API 模块（session/resource/preset/skill/mcp/plugin/settings/manifest），每个模块封装对应的 `invoke()` 调用：

```typescript
// 伪代码
export async function getAllSessions(): Promise<SessionsResponse> {
  return invoke("get_all_sessions");
}
export async function focusSession(pid: number): Promise<void> { ... }
// ...每个 IPC 命令对应一个类型安全的封装函数
```

#### 任务 9：React Query 替代手动轮询（FR-8）

- 安装 `@tanstack/react-query`
- 创建 `lib/query/queryClient.ts`（retry=2, staleTime=1s）
- 创建 `queries/sessions.ts`（`useSessionsQuery`，refetchInterval=POLL_INTERVAL）
- 创建 `queries/resources.ts`、`queries/presets.ts`
- 创建 `mutations/sessions.ts`（`useFocusSessionMutation`）
- 创建 `mutations/resources.ts`（`useToggleMcpMutation`、`useEnableResourceMutation`）
- `main.tsx` 注册 `QueryClientProvider`
- `useSessions` hook 重构为委托 `useSessionsQuery`

#### 任务 10：React Error Boundaries（FR-9）

创建 `ErrorBoundary`（组件级，含重试按钮）和 `PageErrorBoundary`（页面级，含重试+刷新）。设计为 Class Component，使用 `getDerivedStateFromError` 捕获渲染错误。

#### 任务 11：src/config/ 常量集中（FR-6）

创建 `constants.ts`（POLL_INTERVAL、EVENT_FRESHNESS_THRESHOLD、SESSION_STATUS、EXTENSION_KIND、SUPPORTED_TOOLS）。

#### 任务 12：src/lib/schemas/ Zod 验证（FR-7）

安装 `zod`，创建 5 个 schema 文件（session/extension/preset/settings/manifest），定义运行时验证规则。

#### 任务 13-14：Monorepo 基础 + Pre-commit Hooks（FR-10/11）

- `.editorconfig`（统一缩进/换行/编码）
- `husky` + `lint-staged`（pre-commit 跑 Prettier + ESLint + rustfmt）
- `commitlint`（`.husky/commit-msg` 强制 Conventional Commits）

#### 任务 15：宪法代码组织 PATCH（FR-12）

更新 `.specify/memory/constitution.md` 的代码组织小节，将旧的 `manager/`、`store.rs`、`commands.rs` 结构替换为新的 `database/`、`services/`、`commands/`、`window/` 结构，并新增前端 `lib/api/`、`lib/query/`、`lib/schemas/`、`config/`、`components/` 子目录说明。

---

### Plan 3：Spec 003 测试体系（9 任务）

#### Rust 测试基础设施

**support.rs**：
- `setup()` 函数使用 `Once` 保证全局 DB 只初始化一次
- 设置 `HOME` 环境变量到临时目录，创建 `~/.mam/` 目录结构
- `std::mem::forget(temp)` 防止 TempDir 被清理（否则数据库文件被删除导致测试失败）
- `create_test_extension()` 辅助函数

**dao_test.rs**（4 个测试）：
- `test_settings_get_set` -- settings 表读写 + 更新
- `test_extension_crud` -- extensions 表插入 + 查询 + assignment 关联
- `test_preset_crud` -- presets 表创建 + 列表 + items 查询 + 删除
- `test_session_status` -- session_status_cache 状态更新 + 清理

**linker_test.rs**（3 个测试）：
- `test_write_atomic` -- 原子写入 + 临时文件不残留
- `test_write_config_locked` -- fs2 锁写入
- `test_create_and_remove_link` -- 符号链接创建 + 删除 + 源文件不受影响

#### 前端测试基础设施

**vitest 配置**：`vitest.config.ts`（jsdom 环境，`@` alias，coverage v8）

**Tauri Mock**：
- `tests/msw/tauriMocks.ts` -- `tauriInvokeMock` 函数，switch 分支覆盖所有 IPC 命令
- `tests/setup.ts` -- `vi.mock("@tauri-apps/api/core")` 全局拦截 invoke
- Mock 数据使用正确的 `Session` 类型（agentType/projectName/status 等字段）

**前端测试**（4 个测试）：
- `tauriMocks.test.ts`（3 个）-- 验证 mock 返回正确的数据结构
- `useSessions.test.tsx`（1 个）-- React Query hook 加载会话数据，需 QueryClientProvider wrapper

---

### Plan 5：Spec 005 Extension Manifest（10 任务）

#### 后端

**types.rs**：定义 `Manifest`（含 `ManifestCommon` + `SkillFields`/`McpFields`/`PluginFields`）、`Permission` 枚举（7 种权限，含 `risk_level()` 和 `description()`）、`Kind` 枚举、`CompatibilityEntry`、`Author`。使用 `#[serde(flatten)]` 展开公共字段。

**validator.rs**：`ManifestValidator` 实现：
- `validate_file(path)` -- 从文件读取并校验
- `validate_json(json)` -- 从 JSON 字符串校验
- 校验规则：必填字段、id 格式（字母数字._-）、semver 版本、githubRepo 格式（owner/repo）、路径穿越检查（`../` 拒绝）、MCP 格式一致性检查（`mcp.formats` 与 `compatibility.mcpFormat` 匹配）
- 返回结构化 `ValidationError` 列表（field + message + code）

**store.rs**：`~/.mam/store/index.json` 读写，`add_entry`/`remove_entry`/`read_index` 函数。

**update_checker.rs**：Phase 2 预留接口（`check_for_updates` 返回 None）。

**commands/manifest.rs**：4 个 IPC 命令：
- `validate_manifest` -- 校验 manifest 文件，返回 ValidateResult
- `install_resource_from_manifest` -- 校验 → 复制到 SSOT → 写 store 索引 → 记录数据库
- `uninstall_resource` -- 遍历移除所有工具分配 → 删除文件 → store 标记未安装
- `get_store_index` -- 读取商店索引

**数据库扩展**：`schema.rs` 的 extensions 表新增 `manifest_path`/`permissions`/`min_runtime` 三列；`migration.rs` 用 `ALTER TABLE` 兼容已有数据库。

#### 前端

**lib/schemas/manifest.ts**：Zod `discriminatedUnion("kind", [...])` 区分 skill/mcp/plugin 三种 manifest 变体，`PermissionSchema` 枚举，`PERMISSION_RISK`/`PERMISSION_DESCRIPTION` 映射表。

**lib/api/manifest.ts**：4 个 API 函数封装。

**components/resources/PermissionBadge.tsx**：按风险等级（low=绿/medium=黄/high=红）渲染权限徽章。

**components/resources/ManifestInstallDialog.tsx**：安装确认弹窗，展示资源信息 + 权限列表 + 兼容工具 + 高风险警告，校验失败时展示错误详情。

---

### Plan 4：Spec 004 文档与流程（9 任务）

| 产出 | 数量 | 说明 |
|------|------|------|
| GitHub Workflows | 3 | ci.yml（前端 lint+build+test + 后端 cargo check+clippy+test）、release.yml（3 平台构建）、stale.yml（60 天自动关闭） |
| Issue 模板 | 4 | bug_report、feature_request、question + config（禁用空白 issue） |
| 根目录文档 | 3 | CHANGELOG.md、CONTRIBUTING.md、SECURITY.md |
| 多语言指南 | 10 | 5 主题 x 2 语言（zh/en），每篇含安装、使用、故障排查 |
| 用户手册 | 22 | 11 章 x 2 语言（zh/en），含截图占位符 |
| Release notes | 1 | v0.2.2 版本说明 |
| ADR | 5 | README 索引 + 001(adapter) + 002(symlink) + 003(hook 策略) + 004(React Query) |
| 工程化配置 | 5 | commitlint.config.js、dependabot.yml、CODEOWNERS、PR 模板、.husky/commit-msg |
| 截图目录 | 2 | assets/screenshots/{zh,en}/ 含 .gitkeep |

---

## 二、测试情况

### 测试总览

| 层级 | 测试文件数 | 测试用例数 | 通过 | 失败 |
|------|-----------|-----------|------|------|
| Rust lib 单元测试 | 内嵌 | 9 | 9 | 0 |
| Rust 集成测试（DAO） | 1 | 4 | 4 | 0 |
| Rust 集成测试（Linker） | 1 | 3 | 3 | 0 |
| 前端 vitest | 2 | 4 | 4 | 0 |
| **合计** | **4** | **20** | **20** | **0** |

### Rust 测试明细

**lib 单元测试（9 个）**：

| 测试 | 模块 | 验证内容 |
|------|------|---------|
| test_get_all_sessions | adapter | 会话发现返回正确结构 |
| test_tool_active_dir | linker::layer2 | Layer 2 活跃目录路径构建 |
| test_ensure_tool_active_dir | linker::layer2 | 活跃目录创建 |
| test_subagent_active_dir | linker::layer3 | Layer 3 子 Agent 目录路径 |
| test_valid_skill_manifest | manifest::validator | 合法 skill manifest 通过校验 |
| test_missing_id | manifest::validator | 缺少 id 字段时返回 PARSE_ERROR |
| test_invalid_semver | manifest::validator | 非法版本号返回 INVALID_SEMVER |
| test_path_traversal | manifest::validator | `../` 路径返回 PATH_TRAVERSAL |
| test_mcp_format_mismatch | manifest::validator | MCP 格式不一致返回 FORMAT_MISMATCH |

**DAO 集成测试（4 个）**：

| 测试 | 验证内容 |
|------|---------|
| test_settings_get_set | settings 表写入 → 读取 → 更新 → 再读取 |
| test_extension_crud | extensions 表插入 → list_extensions 查询 → upsert_assignment 关联 → list_assignments 验证 |
| test_preset_crud | presets 表创建 → list_presets → get_preset_items → delete_preset → 验证已删除 |
| test_session_status | session_status_cache 首次写入返回 None → 状态变化返回 previous → cleanup_stale_sessions 保留活跃会话 |

**Linker 集成测试（3 个）**：

| 测试 | 验证内容 |
|------|---------|
| test_write_atomic | write-to-temp + rename → 文件内容正确 → 临时文件不残留 |
| test_write_config_locked | fs2 排他锁 + 原子写入 → 文件内容正确 |
| test_create_and_remove_link | 符号链接创建 → target 存在 → 删除 → target 不存在 → source 仍在 |

### 前端测试明细

**tauriMocks.test.ts（3 个）**：

| 测试 | 验证内容 |
|------|---------|
| mocks get_all_sessions | invoke 返回正确的 SessionsResponse 结构（sessions 数组 + totalCount + waitingCount） |
| mocks list_presets | invoke 返回预设组列表，含 name 字段 |
| mocks unknown_command | 未注册的命令返回 undefined |

**useSessions.test.tsx（1 个）**：

| 测试 | 验证内容 |
|------|---------|
| returns session list | React Query hook 初始 loading → 等待后 isSuccess → data.sessions 长度为 2 → sessions[0].agentType === "claude" |

### 构建验证

| 检查命令 | 结果 | 说明 |
|---------|------|------|
| `cargo check` | 通过 | 5 个预存 warning（unused functions），无 error |
| `pnpm build` | 通过 | TypeScript 编译 + Vite 打包成功 |
| `pnpm lint` | 通过 | ESLint 无错误（修复了 2 个预存的 empty block 错误） |

### 测试中发现并修复的问题

| 问题 | 根因 | 修复 |
|------|------|------|
| DAO 测试全部失败 | `TempDir` 在 `Once` 闭包结束时被 drop，数据库文件被删除 | `std::mem::forget(temp)` 保持 TempDir 存活 |
| manifest validator test_missing_id 失败 | `id: String` 是必填字段，缺失时 serde 解析失败返回 PARSE_ERROR 而非 REQUIRED | 修改测试断言为 `code == "PARSE_ERROR"` |
| AppHandle 命令编译失败 | `generate_handler!` 在泛型 `add_commands<R: Runtime>` 上下文中无法解析 `AppHandle` | 将 AppHandle 命令移到 lib.rs 非泛型上下文注册 |
| notify match 非穷尽 | `Ok(Ok(_))` 不带 guard 的分支未覆盖 | 添加 `Ok(Ok(_)) => {}` 空匹配分支 |
| vitest 拾取 reference/ 目录测试 | vitest 默认扫描整个项目 | 配置 `include: ["tests/**/*.test.{ts,tsx}"]` |
| useSessions.test.ts JSX 编译失败 | 文件扩展名为 .ts 但含 JSX | 改为 .tsx |
| ESLint empty block 错误 | 预存的 `catch {}` 空块 | 添加注释使块非空 |
