# 功能规格说明：测试体系

**功能分支**：`003-test-infrastructure`

**创建日期**：2026-07-08

**状态**：草稿

**输入**：参考 cc-switch 项目的测试体系（12 个 Rust 集成测试 + 70 个前端 vitest 测试），为 MultiAgents Manager 建立完整的测试基础设施，覆盖后端业务逻辑和前端组件。

## 用户场景与测试

### 用户故事 1 — 后端逻辑回归保护（优先级: P1）

开发者在重构 `commands.rs` 或 `store.rs` 时，需要确保现有功能不被破坏。当前缺乏系统化的后端测试，重构风险高。

**优先级理由**：测试是重构的安全网，没有测试覆盖的重构等于盲飞。

**独立测试**：运行 `cargo test`，验证所有 Rust 测试通过。

**验收场景**：

1. **给定** 开发者修改了 `database/dao/session.rs` 中的查询逻辑，**当** 运行 `cargo test`，**则** DAO 层的单元测试能立即发现 SQL 语法错误或逻辑回归
2. **给定** 开发者重构了 `commands/session.rs` 中的命令处理函数，**当** 运行 `cargo test`，**则** 集成测试验证命令的输入输出契约不变
3. **给定** CI 流水线执行 `cargo test`，**当** 有测试失败，**则** 流水线自动阻止合并

### 用户故事 2 — 前端组件行为验证（优先级: P2）

开发者在修改 React 组件（如 `SessionCard` 或 `ResourceByKindView`）时，需要验证组件的渲染逻辑和交互行为。

**优先级理由**：前端组件测试能捕获 UI 回归（如状态灯颜色错误、按钮点击无响应）。

**独立测试**：运行 `pnpm test`（vitest），验证所有前端测试通过。

**验收场景**：

1. **给定** 开发者修改了 `SessionCard` 组件的状态显示逻辑，**当** 运行 vitest，**则** 组件测试验证不同状态下渲染正确的颜色和文本
2. **给定** 开发者修改了 `PresetList` 的预设应用逻辑，**当** 运行 vitest，**则** 交互测试验证点击预设按钮后调用了正确的 API
3. **给定** 开发者修改了 `ResourceByKindView` 的资源过滤逻辑，**当** 运行 vitest，**则** 测试验证过滤后的列表包含预期项

### 用户故事 3 — Tauri API Mock 测试（优先级: P3）

前端测试需要模拟 Tauri 的 `invoke()` 调用，避免测试依赖后端运行时。

**优先级理由**：Tauri 桌面应用的测试必须在无后端环境下运行，需要系统化的 Mock 机制。

**独立测试**：运行 `pnpm test`，验证 MSW mock 能正确拦截 Tauri API 调用。

**验收场景**：

1. **给定** 前端测试调用 `api.session.getAllSessions()`，**当** 后端未运行时，**则** MSW mock 返回预定义的测试数据
2. **给定** 测试需要模拟后端错误响应，**当** 配置 mock handler 返回 500，**则** 前端组件正确显示错误状态

## 功能需求

### FR-1: Rust 集成测试目录

1. 创建 `src-tauri/tests/` 目录结构：
   ```
   src-tauri/tests/
   ├── support.rs          # 测试辅助函数（数据库初始化、临时目录创建）
   ├── commands_test.rs    # IPC 命令集成测试
   ├── dao_test.rs         # DAO 层单元测试
   ├── linker_test.rs      # 符号链接三层映射测试
   └── services_test.rs    # Service 层业务逻辑测试
   ```

2. `support.rs` 提供测试基础设施：
   ```rust
   pub fn setup_test_db() -> Connection {
       let conn = Connection::open_in_memory().unwrap();
       database::schema::init(&conn).unwrap();
       conn
   }

   pub fn temp_dir() -> TempDir {
       tempfile::tempdir().unwrap()
   }
   ```

3. `commands_test.rs` 测试每个 IPC 命令的输入输出契约：
   ```rust
   #[test]
   fn test_get_sessions_returns_valid_list() {
       let app = setup_test_app();
       let response: SessionsResponse = tauri::test::invoke(&app, "get_all_sessions", ());
       assert!(response.sessions.is_empty() || response.sessions[0].tool.is_some());
   }
   ```

4. `dao_test.rs` 测试每个 DAO 的 CRUD 操作：
   ```rust
   #[test]
   fn test_session_dao_crud() {
       let conn = setup_test_db();
       let dao = SessionDao::new(&conn);
       let id = dao.create(&Session::new("claude", "/tmp/test")).unwrap();
       let session = dao.find_by_id(id).unwrap();
       assert_eq!(session.tool, "claude");
   }
   ```

5. `linker_test.rs` 测试三层符号链接映射：
   ```rust
   #[test]
   fn test_layer2_symlink_creation() {
       let temp = temp_dir();
       let layer1 = temp.path().join("skills/brainstorming");
       let layer2 = temp.path().join("active/claude/skills/brainstorming");
       linker::create_symlink(&layer1, &layer2).unwrap();
       assert!(layer2.exists());
   }
   ```

6. **依赖**：需要安装 `tempfile` crate（dev-dependencies）

**cc-switch 参考**：`src-tauri/tests/` 含 12 个测试文件 + `support.rs`

### FR-2: 前端 vitest 测试

1. 安装 vitest 及相关依赖：
   ```bash
   pnpm add -D vitest @testing-library/react @testing-library/jest-dom @testing-library/user-event jsdom @vitest/coverage-v8
   ```

2. 创建 `vitest.config.ts`：
   ```typescript
   import { defineConfig } from "vitest/config";
   import react from "@vitejs/plugin-react";
   import path from "path";

   export default defineConfig({
     plugins: [react()],
     test: {
       environment: "jsdom",
       globals: true,
       setupFiles: ["./tests/setup.ts"],
       coverage: {
         provider: "v8",
         reporter: ["text", "html"],
         exclude: ["node_modules/", "tests/", "src/lib/api/"],
       },
     },
     resolve: {
       alias: {
         "@": path.resolve(__dirname, "./src"),
       },
     },
   });
   ```

3. 创建 `tests/setup.ts` 测试初始化文件：
   ```typescript
   import "@testing-library/jest-dom";
   import { server } from "./msw/server";

   beforeAll(() => server.listen());
   afterEach(() => server.resetHandlers());
   afterAll(() => server.close());
   ```

4. 创建前端测试目录结构：
   ```
   tests/
   ├── setup.ts              # 全局测试初始化
   ├── components/           # 组件测试
   │   ├── SessionCard.test.tsx
   │   ├── SessionGrid.test.tsx
   │   ├── ResourceByKindView.test.tsx
   │   └── PresetList.test.tsx
   ├── hooks/                # Hook 测试
   │   ├── useSessions.test.ts
   │   └── useNotification.test.ts
   ├── integration/            # 集成测试
   │   └── dashboard-flow.test.tsx
   └── msw/                  # Mock Service Worker
       ├── server.ts
       ├── handlers.ts
       └── tauriMocks.ts
   ```

5. 组件测试示例（`SessionCard.test.tsx`）：
   ```typescript
   import { render, screen } from "@testing-library/react";
   import { SessionCard } from "@/components/sessions/SessionCard";
   import { describe, it, expect } from "vitest";

   describe("SessionCard", () => {
     it("renders running status with yellow light", () => {
       render(<SessionCard session={mockRunningSession} />);
       expect(screen.getByText("运行中")).toBeInTheDocument();
       expect(screen.getByTestId("status-light")).toHaveClass("bg-yellow-500");
     });

     it("renders waiting status with red light", () => {
       render(<SessionCard session={mockWaitingSession} />);
       expect(screen.getByText("等待输入")).toBeInTheDocument();
       expect(screen.getByTestId("status-light")).toHaveClass("bg-red-500");
     });
   });
   ```

6. Hook 测试示例（`useSessions.test.ts`）：
   ```typescript
   import { renderHook, waitFor } from "@testing-library/react";
   import { useSessions } from "@/hooks/useSessions";
   import { describe, it, expect } from "vitest";

   describe("useSessions", () => {
     it("returns session list on mount", async () => {
       const { result } = renderHook(() => useSessions());
       await waitFor(() => {
         expect(result.current.data?.sessions).toHaveLength(2);
       });
     });
   });
   ```

**cc-switch 参考**：`tests/` 含 70 个文件，分 `components/`(28)、`hooks/`(16)、`integration/`(2)、`config/`(10)、`utils/`(6)

### FR-3: MSW + Tauri Mock

1. 扩展现有 `src/tauri-mock.ts`，增加 vitest 支持：
   ```typescript
   // src/tauri-mock.ts（新增 vitest 分支）
   import { vi } from "vitest";

   export function mockTauriApi() {
     if (import.meta.env.VITEST) {
       // vitest 环境下由 MSW 处理
       return;
     }
     // 原有浏览器渲染 mock 逻辑...
   }
   ```

2. 创建 `tests/msw/tauriMocks.ts` — 系统化模拟 Tauri API：
   ```typescript
   import { http, HttpResponse } from "msw";

   export const tauriHandlers = [
     http.post("/tauri/get_all_sessions", () => {
       return HttpResponse.json([
         { id: 1, tool: "claude", status: "running", project_path: "/tmp/test1" },
         { id: 2, tool: "codex", status: "waiting", project_path: "/tmp/test2" },
       ]);
     }),
     http.post("/tauri/get_resources", () => {
       return HttpResponse.json([
         { id: 1, name: "brainstorming", type: "skill", enabled_tools: ["claude"] },
       ]);
     }),
     http.post("/tauri/get_presets", () => {
       return HttpResponse.json([
         { id: 1, name: "前端开发", resources: [{ id: 1, type: "skill" }] },
       ]);
     }),
   ];
   ```

3. 创建 `tests/msw/handlers.ts` — 聚合所有 mock handlers：
   ```typescript
   import { tauriHandlers } from "./tauriMocks";
   export const handlers = [...tauriHandlers];
   ```

4. 创建 `tests/msw/server.ts` — MSW 服务器实例：
   ```typescript
   import { setupServer } from "msw/node";
   import { handlers } from "./handlers";
   export const server = setupServer(...handlers);
   ```

5. **依赖**：需要安装 `msw` 包（`pnpm add -D msw`）

**cc-switch 参考**：`tests/msw/tauriMocks.ts` 统一 mock invoke() + `handlers.ts` + `server.ts`

## 成功标准

1. `cargo test` 全部通过（Rust 后端测试）
2. `pnpm test` 全部通过（前端 vitest 测试）
3. 后端核心模块测试覆盖率 ≥80%（核心模块 = DAO（session/extension/preset/settings/agent_tool）+ Service（resource/preset/skill/mcp/plugin/manifest）+ Linker（layer2/layer3）+ Manifest validator。commands 集成测试、UI 组件测试、窗口跳转测试为非核心通过测试，不计入 80% 基线，但必须存在且有测试用例。）
4. 前端关键组件（SessionCard, ResourceByKindView, PresetList）有完整测试
5. MSW mock 能正确拦截所有 Tauri API 调用，测试无需后端运行
6. CI 流水线自动运行 `cargo test` + `pnpm test`，失败时阻止合并

## 关键实体

| 实体 | 说明 | 关键属性 |
|------|------|---------|
| TestSupport | 测试辅助工具 | 数据库初始化、临时目录、mock 数据 |
| MockHandler | MSW mock 处理器 | API 路径、请求方法、响应数据 |
| TestSuite | 测试套件 | 测试文件、测试用例、覆盖率 |
| CoverageReport | 覆盖率报告 | 行覆盖率、分支覆盖率、函数覆盖率 |

## 假设

1. 测试在架构重构（Spec 002，含 FR-2 database/dao、FR-5 services 拆分、FR-8 React Query 重构）完成后编写，确保代码结构稳定
2. `tempfile` crate 已添加到 `src-tauri/Cargo.toml` 的 `[dev-dependencies]`
3. `vitest` 和相关包已安装到前端依赖
4. `msw` 包已安装，且 mock 配置与现有 `tauri-mock.ts` 不冲突
5. 测试数据使用内存数据库和 mock 数据，不依赖真实文件系统或网络

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| Tauri 集成测试需要完整 App 上下文 | 中 | 使用 `tauri::test::mock_builder()` 或仅测试 commands 层的输入输出 |
| MSW 与 tauri-mock.ts 冲突 | 低 | 明确区分环境：浏览器用 tauri-mock，vitest 用 MSW |
| 测试编写耗时，延迟功能开发 | 中 | 优先覆盖核心模块（DAO + commands），UI 测试后续补充 |
| SQLite 内存测试与文件测试行为差异 | 低 | DAO 测试使用内存数据库，集成测试使用临时文件数据库 |

## cc-switch 参考

- **Rust 集成测试**：`src-tauri/tests/` 含 12 个测试文件 + `support.rs`
- **前端 vitest**：`tests/` 含 70 个文件，分 `components/`(28)、`hooks/`(16)、`integration/`(2)、`config/`(10)、`utils/`(6)
- **MSW mock**：`tests/msw/tauriMocks.ts` 统一模拟 Tauri API，比我们的 `tauri-mock.ts` 更完整
- **覆盖率**：使用 `@vitest/coverage-v8`，配置 `reporter: ["text", "html"]`
