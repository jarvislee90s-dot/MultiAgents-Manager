# Spec 审查修改清单

**生成时间**：2026-07-08
**审查范围**：specs/001 ~ 005 + constitution.md + AGENTS.md
**用途**：供执行 Agent 按本清单逐条修订 spec 与治理文档，不直接改本文档。

---

## 执行 Agent 提示词

```
你是一个 spec 与项目治理文档修订 Agent。你的任务是根据下面的「修改清单」，逐文件执行修订。

工作约束：
1. 只做清单中明确列出的修改，不做需求外的操作，不改清单未提及的文件。
2. 业务逻辑不变——本次只改文档措辞、FR 编号、示例代码、路径术语、依赖顺序声明，不改任何 .rs / .ts 源码文件。
3. 每个修改项后标 [编号]（如 [B1]），便于追溯。
4. 修改前先完整读取目标文件，找到对应的 FR/行号/章节再改，避免破坏上下文。
5. 全程中文，代码标识符用英文。
6. 涉及宪法修订的，按宪法治理规则走版本递增：原则实质改动=MINOR，澄清/措辞/结构同步=PATCH。同步检查 .specify/templates/ 下是否有模板需要同步传播。
7. 涉及前端示例代码的，保持现有项目的 import 风格（@/ 路径别名、@tauri-apps/api/core）。
8. 修改完成后，逐条核对清单，自检是否全部落地，输出一份「修改完成核对表」。

输入文件读取顺序（先读后改）：
  1. .specify/memory/constitution.md（宪法，最高权威，最后改）
  2. AGENTS.md（项目指令）
  3. specs/001-multi-agent-platform/spec.md
  4. specs/002-code-architecture-refactor/spec.md
  5. specs/003-test-infrastructure/spec.md
  6. specs/004-docs-and-process/spec.md
  7. specs/005-extension-manifest/spec.md
```

---

## 修改清单

### 文件 A：`.specify/memory/constitution.md`

[A1] [B1] 原则 III 第一段 — 阶段一工具数量
- 当前：阶段一（MVP）：Claude Code + Codex CLI + OpenCode **三工具**监控
- 改为：阶段一（MVP）：Claude Code + Codex CLI + OpenCode + **OpenClaw** **四工具**监控
- 同步调整紧随该段的描述：把「OpenCode 无 Hook 系统，通过进程扫描 + 数据文件解析」后的工具展开为「OpenCode 无 Hook 系统、OpenClaw 无 Hook 系统，两者均通过进程扫描 + 数据文件解析」。
- 这是原则实质改动 → **MINOR** 版本递增 1.2.0 → 1.3.0；同步更新顶部同步影响报告的「修改原则 III」描述与版本号；更新文件末尾「版本 | 批准日期 | 最后修订」。

[A2] [S9] 技术栈 → 性能目标
- 当前：「启动 <300ms（懒加载会话）」
- 改为：「启动 ≤3s（含 20 会话冷启动场景）；懒加载单页 <300ms」
- 与 spec 001 成功标准 #6（≤3s）对齐，消除 10 倍偏差。

[A3] [S3] 技术栈/原则 IV — notify 双策略落地到 monitor
- 在原则 IV「Hook 失败时必须有回退策略」段后补一句（或在其内补充）：「monitor 模块必须同时集成 `notify` (notify-rs) 文件监听与轮询：Hook/进程事件有变化时优先用 notify 事件，定期轮询作为兜底。」
- 理由：宪法技术栈已列 `notify` 为核心依赖，但 IV 正文只提轮询。补这一句让 IV 与技术栈约束自洽。

[A4] [S3] 原则 V — 补原子更新措辞的精确化（不新增原则，只澄清）
- 当前 V 末句：「原子更新（write-to-temp + rename，或 fs2 文件锁）」
- 改为：「原子更新：单资源映射采用 write-to-temp + rename 或 fs2 文件锁；预设组的批量操作按资源单位分别原子化，单位间失败不自动回滚已成功项。」
- 与 spec 001 FR-6.28「失败保留已成功项」对齐，消除两者隐含矛盾。属澄清 → 可并入 [A1] 的同一 MINOR 修订。

[A5] [S2][S3] 开发流程 → 代码组织小节（PATCH 修订预条目）
- 在代码组织小节加一行「**待同步（PATCH）**：Spec 002 完成后，本小节将同步更新为 `commands/`（按功能拆分）、`database/`（含 dao/）、`services/`（原 manager/）、`window/`（原 terminal/）结构。其余模块 `adapter/`、`monitor/`、`linker/`、`plugins/` 保持。」
- 不立即改结构块，只加这条占位说明，待重构落地后由执行方同步 PATCH。

[A6] [B5] 原则 V 数据目录部分 — MCP 路径复数化统一
- 若宪法 V 中出现 `~/.mam/mcp/` 之外的路径写法，统一为 `~/.mam/mcp/`；同时补 `~/.mam/plugins/` 作为插件仓库路径（如有提及）。
- 检查宪法其它段是否提了 `~/.mam/mcps/`，全部改 `~/.mam/mcp/`。

---

### 文件 B：`AGENTS.md`

[B-A] [B5] 数据目录表
- 当前 `~/.mam/mcp/` 行保持不变（此即为统一基准）。
- 新增一行：`| ~/.mam/plugins/ | 全局 Plugin 仓库 |`
- 检查 AGENTS.md 其它段（如架构概览）是否出现 `~/.mam/mcps/` 或 `~/.mam/plugins/` 缺失的情况，补齐。

---

### 文件 C：`specs/001-multi-agent-platform/spec.md`

[C1] [B2] FR-5.21 — Skill 启用/禁用模式重写
- 当前：「MCP 服务器和插件可为每个工具独立启用/禁用（每个 资源×工具 组合有独立状态）；Skill **不支持**单独为工具启用/禁用，只能通过预设组整体应用到目标工具（见 FR-6），从而避免手动单独分配与预设组移除之间的冲突」
- 改为：「Skill、MCP 服务器和插件均可为每个工具独立启用/禁用（每个 资源×工具 组合有独立状态）。同时支持预设组一键批量启用/禁用（见 FR-6）：应用预设组时，组内所有资源批量应用到目标工具；移除预设组时，组内资源批量从目标工具移除。用户也可将现有某工具上已启用的 skill 组合保存为新的预设组，供后续一键复用。」
- 同步删除该段尾「从而避免…冲突」整句理由（已被新模式取消）。

[C2] [B1] 假设 5 — MVP 工具清单已与宪法对齐
- 检查假设 5：「阶段一（MVP）聚焦 Claude Code + Codex CLI + OpenCode + OpenClaw **四工具**」。
- 当前是否已是四工具。若有「三工具」残留改为四工具；保持与 [A1] 宪法修订后一致。

[C3] [S3] FR-1 — 新增 notify 双策略条目
- 在 FR-1 列表末尾新增条目：「有 Hook 的工具采用 Hook 事件 + notify 文件监听 + 定时轮询三重策略：notify 捕获文件变化优先触发，轮询作为兜底；无 Hook 的工具采用 notify + 进程扫描双策略。Hook/进程事件文件新鲜（<30s）时以事件为准，过期回退轮询。」
- 对应宪法 IV [A3] 与技术栈 notify 依赖。

[C4] [S3] FR-4 — 显化 WindowManager trait
- 在 FR-4 列表内补一条：「跳转机制抽象到 `WindowManager` trait 之后；macOS 覆盖 iTerm2、Terminal.app、kitty、WezTerm、tmux（AppleScript）；Linux X11 用 xdotool；Linux Wayland 检测 `$WAYLAND_DISPLAY` 后降级为『此环境不支持跳转』提示；Windows 用 SetForegroundWindow。」
- 同时把 FR-4 第 13 条「点击会话卡片必须激活对应的终端窗口或标签页」中「终端窗口」泛化为「对应终端窗口或 APP（APP 形态不支持跳转，仅展示状态与通知，见 FR-1.1）」。

[C5] [S3] FR-5 — 新增原子更新条目
- 在 FR-5 列表内补一条：「单资源的映射更新必须原子化：Skill symlink/Junction 创建采用 write-to-temp + rename；MCP/插件配置写入采用 fs2 文件锁防止并发写。预设组批量操作按资源单位分别原子化，单位间失败保留已成功项（见 FR-6.28）。」
- 对应宪法 V [A4]。

[C6] [S3] FR-2 或新增条目 — 可视化配置看板归属
- 在 FR-2 或单独新增条目：「资源映射看板必须可视化展示每个 资源×工具/子Agent 的启用状态，覆盖 Skill/MCP/插件三类，支持按类型视图与按工具视图切换。」
- 明确现有 `ResourceByKindView`/`ResourceByToolView` 即宪法 V 所述「可视化配置看板」，消除 V 的悬空要求。

[C7] [B5] FR-5.17 — 路径复数化统一
- 当前：`~/.mam/skills/` + `~/.mam/mcps/` + `~/.mam/plugins/`
- 改为：`~/.mam/skills/` + `~/.mam/mcp/` + `~/.mam/plugins/`
- 全文搜索 `~/.mam/mcps/` 全部改为 `~/.mam/mcp/`。确保与 AGENTS.md [B-A] 和宪法 [A6] 一致。

[C8] [S9] 成功标准 #6
- 检查是否为「启动不超过 3 秒」。保持不变；本条仅确认无需修改。若写的是 <300ms，改为 ≤3s。

---

### 文件 D：`specs/002-code-architecture-refactor/spec.md`

[D1] [B6] FR-1.4 / FR-4 示例 / FR-8.3 — 命令名与返回类型统一
- 全文搜索 `get_sessions` 全部改为 `get_all_sessions`；`Session[]`（作为该命令返回类型时）改为 `SessionsResponse`。
- 具体位置：
  - FR-1.1 注册说明段：把示例命令名同步。
  - FR-4 示例代码：`invoke<Session[]>("get_sessions")` → `invoke<SessionsResponse>("get_all_sessions")`；对应 import 改 `import type { SessionsResponse } from "@/types/session"`。
  - FR-8.3 重构示例：`queryFn: getSessions` 内部 `invoke("get_sessions")` → `invoke("get_all_sessions")`。
- 在 FR-1.4「保持命令函数签名不变」后补一行：「命令名不变：保留现有 `get_all_sessions`，不重命名为 `get_sessions`。」

[D2] [S3] 新增 FR — terminal/ → window/ + WindowManager trait
- 在 FR-5 之后新增 FR-5b（或顺延编号）。内容：
  1. 将 `src-tauri/src/terminal/` 改名为 `src-tauri/src/window/`。
  2. 把跳转抽象升格为 `WindowManager` trait，含 `focus(session)` 方法；
     - macOS 实现：iTerm2/Terminal.app/kitty/WezTerm/tmux（AppleScript）
     - Linux-X11：xdotool
     - Linux-Wayland：返回不支持，UI 提示
     - Windows：SetForegroundWindow
  3. 公共 API 不变。
  4. 验证：`cargo check` 通过 + 手动 iTerm2/tmux 跳转验证。
- 对接宪法代码组织 [A5] 的 PATCH 修订。

[D3] [S3] 新增 FR — notify 集成到 monitor
- 在 FR-1 或独立条目补：「`monitor/` 模块集成 notify-rs 文件监听：Hook/进程事件文件变化时优先触发 notify 事件，定期轮询作为兜底。保持 monitor 对外接口不变。」
- 注意：这是「重构中的双策略接入点」，而非新功能——拆 monitor 时顺带补 notify 接入位。

[D4] [实体对齐建议] 新增 FR — IPC 实体对齐表
- 在 FR-1 或 FR-4 后新增一条 FR：「重构期间建立 Rust ↔ TypeScript 实体对齐表，覆盖 Session / SessionsResponse / Extension / Preset / AgentTool / SubAgent 共 6 个实体，列出每个实体在 Rust struct 和 TS interface 间的字段对应。重构 PR 验收时附该表作为静态检查文档。」
- 示例放在该 FR 内，给一张两列对照表骨架。

[D5] [轮询间隔常量建议] FR-8.3 补常量来源
- 在 FR-8.3 的 React Query 配置示例中，把硬编码 `refetchInterval: 1500` 改为从常量取（如 `import { POLL_INTERVAL } from "@/config/constants"`），并补 `refetchIntervalInBackground: false`。
- 与 FR-6 的 `constants.ts` 关联。

[D6] [重构顺序建议] 假设 8 — 顺序微调
- 当前假设 8（最后一条）：「完整重构顺序：FR-1 → FR-2 → FR-5（后端）→ ...」
- 改为：后端从底层往上层拆——「FR-2（database）→ FR-5（services）→ FR-5b（window）→ FR-1（commands 聚合层，此时改 use 引用）→ 前端 FR-3/4/8/9 → FR-6/7 → FR-10/11」。
- 理由：commands 引用 store/services 改名，先拆底层再拆命令层可少一道来回。

[D7] [S2] 新增 FR — 宪法代码组织 PATCH 修订
- 在 spec 末尾新增一条 FR（编号顺延）：「Spec 002 重构完成且 `cargo check` + `tsc --noEmit` 通过后，提交宪法 `.specify/memory/constitution.md` 的『开发流程 → 代码组织』小节 PATCH 修订，反映 `commands/`、`database/`、`services/`、`window/` 新结构，并同步传播到 plan/tasks 模板。」

[D8] [003 依赖边界建议] 003 之外，本 spec 增备注
- 在 spec 002 的假设或风险评估里补一句：「Spec 003 的测试体量以本 spec 拆分后的模块为基准：核心模块 = DAO + Service + Linker + Manifest validator，commands 集成测试与 UI 组件测试为非核心通过测试。」
- 供 003 的 [E3] 落地引用。

---

### 文件 E：`specs/003-test-infrastructure/spec.md`

[E1] [B4] FR-2.6 hook 测试示例 — React Query 断言
- 当前：
  ```ts
  const { result } = renderHook(() => useSessions());
  await waitFor(() => {
    expect(result.current.sessions).toHaveLength(2);
  });
  ```
- 改为（适配 002 FR-8.3 重构后的 React Query 返回结构）：
  ```ts
  const { result } = renderHook(() => useSessions());
  await waitFor(() => {
    expect(result.current.data).toBeDefined();
    expect(result.current.data!.sessions).toHaveLength(2);
  });
  ```
  或若 002 把 `useSessions` 返回 `SessionsResponse`：
  ```ts
  await waitFor(() => {
    expect(result.current.data?.sessions).toHaveLength(2);
  });
  ```
- 取与 [D1] 最终返回类型一致的写法。

[E2] [前置依赖建议] 假设 1 补 002 FR 编号
- 当前假设 1：「测试在架构重构（Spec 002）完成后编写」
- 改为：「测试在架构重构（Spec 002，含 FR-2 database/dao、FR-5 services 拆分、FR-8 React Query 重构）完成后编写。」
- 同步在 [D8] 备注里确认核心模块边界。

[E3] [测试体量建议] 成功标准 #3 与 FR 边界
- 在成功标准 #3「后端核心模块测试覆盖率 ≥80%」后加括注：「核心模块 = DAO（session/extension/preset/settings/agent_tool）+ Service（resource/preset/skill/mcp/plugin/manifest）+ Linker（layer2/layer3）+ Manifest validator。commands 集成测试、UI 组件测试、窗口跳转测试为非核心通过测试，不计入 80% 基线，但必须存在且有测试用例。」
- 对接宪法测试要求与 [D8]。

[E4] [B5] FR-3 路径示例 — 跟随 [C7]
- 若 003 示例里出现 `~/.mam/mcps/`，改为 `~/.mam/mcp/`（少见，仅检查）。

---

### 文件 F：`specs/004-docs-and-process/spec.md`

[F1] [S5/S 前置依赖] 假设 1 — 补 003 前置
- 当前假设 1：「文档在功能稳定后编写（Spec 001 和 002 完成后）」
- 改为：「文档在功能稳定后编写（Spec 001、002、003 完成后）；FR-1 的 CI 跑 `cargo test` + `pnpm test`，依赖 Spec 003 的测试基础设施。」

[F2] [S5/S commitlint 归属] FR-11 与 002 FR-11 调和
- 在 FR-11 标题或说明内补：「commitlint/`commit-msg` hook 的完整配置归本 spec（004）单一归属；Spec 002 的 FR-11 只负责 `pre-commit` 的 lint-staged，不在 `.husky/commit-msg` 上重复配置。」
- 双重归属问题闭环。

[F3] [S7] FR-11 与 002 FR-11 — pre-commit build 偏重建议
- 在 FR-11 备注（如 002 FR-11.3 走 002，此处只需在 004 加交叉说明）：「建议 `pre-commit` 仅跑 lint-staged（变更文件 format + lint），`pnpm build` 交给 CI，避免提交期跑全量 build 拖慢。」
- 建议性，不构阻塞。

[F4] [S10] FR-9.3 — .gitignore 处理
- 当前：「更新 .gitignore，移除 `TEST/`（如已不在使用）」
- 改为：「`TEST/` 目录维持 `.gitignore` 忽略状态，不删除条目，不做处理。」
- 对齐用户决定：现阶段 TEST/ 统一忽略。

---

### 文件 G：`specs/005-extension-manifest/spec.md`

[G1] [B3] 成功标准 #2 — Phase 1 可验证形式
- 当前：成功标准 #2 `mam validate` 命令能正确校验
- 改为：成功标准 #2「安装流程触发 ManifestValidator 校验，能正确拒绝缺失必填字段/非法 semver/不合法 `githubRepo` 格式/含 `../` 路径穿越的 manifest，并返回结构化 ValidationError 列表（含字段路径 + 错误信息 + 错误码），通过手动安装流程可在 Phase 1 验证。」
- CLI `mam validate` 单独在 FR-4.3 维持 Phase 2 标注。

[G2] [B5] FR-6.1 路径统一
- 当前：`~/.mam/skills/<id>/（或 ~/.mam/mcps/<id>/）`
- 改为：`~/.mam/skills/<id>/（或 ~/.mam/mcp/<id>/）`
- 全文搜索 `~/.mam/mcps/` 改 `~/.mam/mcp/`，与 [C7] 一致。

[G3] [S4] FR-7 依赖引用补 FR 编号
- 当前 FR-7 列「对接 Spec 002 的代码架构」三类目录，无 FR 号。
- 改为显式引用：
  - 后端校验器放 `src-tauri/src/services/manifest/` → 依赖 002 FR-5（services/ 重命名拆分）
  - 前端 Zod schema 放 `src/lib/schemas/manifest.ts` → 依赖 002 FR-7（zod 安装 + schemas/ 创建）
  - 前端 API 层新增 `src/lib/api/manifest.ts` → 依赖 002 FR-4（api 层）
  - UI 放 `src/components/resources/` → 依赖 002 FR-3（components 子目录化）
- 在假设章节新增一条：本 spec 依赖 002 的 FR-3/FR-4/FR-5/FR-7 完成后才可落地。

[G4] [S8 不改确认] FR-5/FR-6 商店体系维持 Phase 1
- 不修改。本条仅在执行完成后核对项确认。

[G5] [S3] FR-7 对接 001 三层映射
- 在 FR-7 第 3 项「对接 Spec 001 的三层映射」补一句：「安装到 Layer 1（SSOT，`~/.mam/skills/<id>/`），manifest 中的 `compatibility` 决定可在哪些工具（Layer 2，`~/.mam/mcp/` 对 MCP 不适用，MCP 直接写入工具配置）启用。MCP 路径在安装时落地为 `~/.mam/mcp/<id>/` 记录，不分 Layer。」

---

## 修改完成核对表（执行 Agent 完成后须输出）

| 编号 | 文件 | 落地点 | 自检 |
|------|------|--------|------|
| A1 | constitution.md | 原则 III 四工具 | ☐ |
| A2 | constitution.md | 性能目标 ≤3s | ☐ |
| A3 | constitution.md | notify 双策略 | ☐ |
| A4 | constitution.md | V 原子更新精确化 | ☐ |
| A5 | constitution.md | 代码组织 PATCH 占位 | ☐ |
| A6 | constitution.md | MCP 路径统一 | ☐ |
| B-A | AGENTS.md | plugins/ 新增 | ☐ |
| C1 | spec 001 | FR-5.21 重写 | ☐ |
| C2 | spec 001 | 假设 5 四工具 | ☐ |
| C3 | spec 001 | FR-1 notify | ☐ |
| C4 | spec 001 | FR-4 WindowManager | ☐ |
| C5 | spec 001 | FR-5 原子更新 | ☐ |
| C6 | spec 001 | 可视化看板归属 | ☐ |
| C7 | spec 001 | 路径复数化 | ☐ |
| C8 | spec 001 | 成功标准 #6 确认 | ☐ |
| D1 | spec 002 | 命令名/类型统一 | ☐ |
| D2 | spec 002 | window/ FR | ☐ |
| D3 | spec 002 | notify monitor FR | ☐ |
| D4 | spec 002 | 实体对齐 FR | ☐ |
| D5 | spec 002 | refetchInterval 常量 | ☐ |
| D6 | spec 002 | 重构顺序 | ☐ |
| D7 | spec 002 | 宪法 PATCH FR | ☐ |
| D8 | spec 002 | 测试体量备注 | ☐ |
| E1 | spec 003 | hook 测试断言 | ☐ |
| E2 | spec 003 | 依赖编号 | ☐ |
| E3 | spec 003 | 覆盖率边界 | ☐ |
| E4 | spec 003 | 路径检查 | ☐ |
| F1 | spec 004 | 依赖 003 | ☐ |
| F2 | spec 004 | commitlint 归属 | ☐ |
| F3 | spec 004 | pre-commit 建议 | ☐ |
| F4 | spec 004 | TEST/ gitignore | ☐ |
| G1 | spec 005 | 成功标准 #2 | ☐ |
| G2 | spec 005 | 路径统一 | ☐ |
| G3 | spec 005 | FR-7 FR 编号 | ☐ |
| G4 | spec 005 | 商店不改确认 | ☐ |
| G5 | spec 005 | FR-7 三层补述 | ☐ |
