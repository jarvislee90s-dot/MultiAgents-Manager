# Spec 复核补丁清单 R1-R12

**生成时间**：2026-07-09
**用途**：修复首轮修改后复核发现的 12 项遗留问题。R1-R3 为阻塞性，R4-R8 为一致性遗漏，R9-R12 为排版/措辞。

---

## 执行 Agent 提示词

```
你是一个 spec 与项目治理文档修订 Agent。这是第二轮补丁，修复首轮修改后复核发现的 12 项问题。

工作约束：
1. 只做清单中明确列出的修改，不做需求外的操作。
2. 每个修改项后标 [编号]（如 [R1]），便于追溯。
3. 修改前先完整读取目标文件，找到对应位置再改。
4. 全程中文，代码标识符用英文。
5. [R1] 涉及全文重新编号，改完后逐条核对无重复、无跳号。
6. 修改完成后，输出「修改完成核对表」。

涉及文件：
  1. specs/001-multi-agent-platform/spec.md        （R1, R2, R8）
  2. .specify/memory/constitution.md               （R4, R5）
  3. specs/002-code-architecture-refactor/spec.md  （R6, R7, R11）
  4. specs/003-test-infrastructure/spec.md         （R3）
  5. specs/004-docs-and-process/spec.md            （R10）
  6. specs/005-extension-manifest/spec.md          （R9, R12）
```

---

## 补丁清单

### [R1] 阻塞性 - Spec 001 FR 编号全文重排

**文件**：`specs/001-multi-agent-platform/spec.md`
**问题**：新增条目插入后未重新编号，出现三处重复编号（两个 5、两个 9、两个 16），FR 引用无法唯一定位。
**操作**：将「功能需求」章节下所有条目从 1 开始连续重新编号，最终应为 1-40。注意 [R2] 会先移动 notify 条目到 FR-1，移动后再统一编号。

目标结构（编号 + 所属 FR）：

```
FR-1 会话发现与状态检测     -> 条目 1-5   （第 5 条为 notify，从 FR-2 移入）
FR-2 统一监控看板           -> 条目 6-10  （第 10 条为可视化看板归属）
FR-3 通知与提醒             -> 条目 11-14
FR-4 快速跳转               -> 条目 15-18 （第 18 条为 WindowManager trait）
FR-5 扩展资源统一仓库       -> 条目 19-27 （第 27 条为原子更新）
FR-6 预设组管理             -> 条目 28-32
FR-7 子 Agent 级分配        -> 条目 33-36
FR-8 安全与透明             -> 条目 37-40
```

改完后自检：`rg -n "^\d+\." specs/001-multi-agent-platform/spec.md` 输出中功能需求区域无重复数字。

---

### [R2] 阻塞性 - Spec 001 notify 条目移回 FR-1

**文件**：`specs/001-multi-agent-platform/spec.md`
**问题**：[C3] 要求在 FR-1 列表末尾新增 notify 条目，但实际放在了 `### FR-2: 统一监控看板` 下（当前编号 5 的位置）。
**操作**：将 notify 条目（「有 Hook 的工具采用 Hook 事件 + notify 文件监听 + 定时轮询三重策略...」整段）从 FR-2 移到 FR-1 的最后一条（「状态检测失败时不得崩溃...」之后）。移动后配合 [R1] 统一编号。

---

### [R3] 阻塞性 - Spec 003 残留 get_sessions 三处统一

**文件**：`specs/003-test-infrastructure/spec.md`
**问题**：首轮 [B6]/[D1] 要求全局统一命令名为 `get_all_sessions`，但 spec 003 有三处遗漏。
**操作**：

1. **FR-1.3 Rust 测试示例**（当前 `commands_test.rs` 代码块内）：
   - `tauri::test::invoke(&app, "get_sessions", ())` -> `tauri::test::invoke(&app, "get_all_sessions", ())`
   - `let sessions: Vec<Session>` -> `let response: SessionsResponse`（匹配实际返回类型）
   - 对应断言调整为 `response.sessions`

2. **FR-3 tauriMocks.ts 示例**：
   - `http.post("/tauri/get_sessions", ...)` -> `http.post("/tauri/get_all_sessions", ...)`

3. **用户故事 3 验收场景 1**：
   - `api.session.getSessions()` -> `api.session.getAllSessions()`（匹配 spec 002 FR-4 的函数命名）

---

### [R4] 宪法原则 IV 补 OpenClaw 到无 Hook 工具列表

**文件**：`.specify/memory/constitution.md`
**问题**：[A1] 把 OpenClaw 加入 MVP 并声明其无 Hook，但原则 IV 正文仍写「无 Hook 的工具（OpenCode、Cursor）」，遗漏了 OpenClaw。
**操作**：将原则 IV 中「无 Hook 的工具（OpenCode、Cursor）必须回退到」改为「无 Hook 的工具（OpenCode、OpenClaw、Cursor）必须回退到」。

---

### [R5] 宪法规模/范围工具数 3 -> 4

**文件**：`.specify/memory/constitution.md`
**问题**：技术栈约束段仍写「管理 100+ skill 跨 3 个工具（MVP）」，MVP 已改为四工具。
**操作**：将「跨 3 个工具（MVP）」改为「跨 4 个工具（MVP）」。

---

### [R6] Spec 002 假设 5 与假设 8 重构顺序矛盾

**文件**：`specs/002-code-architecture-refactor/spec.md`
**问题**：假设 5 写「FR-1 -> FR-2 -> FR-5（后端）」（先拆 commands），假设 8 写「FR-2 -> FR-5 -> FR-5b -> FR-1」（先拆 database），两条方向相反。
**操作**：删除假设 5 的旧顺序内容，改写为引用假设 8：「重构顺序见假设 8（后端从底层往上层拆）」。或直接将假设 5 删除，保留假设 8 作为唯一顺序声明。

---

### [R7] Spec 002 假设编号乱序 7, 9, 8 -> 7, 8, 9

**文件**：`specs/002-code-architecture-refactor/spec.md`
**问题**：假设列表实际排列为 7、9、8，[D8] 测试体量备注（当前 9）排在了 [D6] 重构顺序（当前 8）前面。
**操作**：调整为 7、8、9 顺序--将 [D6] 重构顺序调到 [D8] 测试体量备注之前，重新编号为 8 和 9。

---

### [R8] Spec 001 假设 6 宪法版本号 v1.2.0 -> v1.3.0

**文件**：`specs/001-multi-agent-platform/spec.md`
**问题**：假设 6 仍引用「项目宪法（`.specify/memory/constitution.md` v1.2.0）」，宪法已升至 v1.3.0。
**操作**：将 `v1.2.0` 改为 `v1.3.0`。

---

### [R9] Spec 005 假设补充合并回假设列表

**文件**：`specs/005-extension-manifest/spec.md`
**问题**：新增的依赖声明被放在独立的 `## 假设补充` 标题下（条目 7），未合并进已有的 `## 假设` 列表。
**操作**：删除 `## 假设补充` 标题，将「7. 本 spec 依赖 002 的 FR-3/FR-4/FR-5/FR-7 完成后才可落地。」接到 `## 假设` 列表第 6 条之后，作为第 7 条。

---

### [R10] Spec 004 FR-9.1 与 FR-9.3 矛盾调和

**文件**：`specs/004-docs-and-process/spec.md`
**问题**：FR-9.1 写「将 `TEST/` 目录中的临时截图迁移到 `assets/screenshots/`」，FR-9.3 写「`TEST/` 目录维持 `.gitignore` 忽略状态，不删除条目，不做处理」，两条同时存在造成混淆。
**操作**：在 FR-9.1 末尾补注「（现阶段暂不执行迁移，见 FR-9.3；待后续阶段 UI 稳定后再迁移）」，使两条关系明确--9.1 是目标，9.3 是当前阶段处置。

---

### [R11] Spec 002 FR-5b/5c/5d 移到 FR-5 之后

**文件**：`specs/002-code-architecture-refactor/spec.md`
**问题**：FR-5b/5c/5d 当前插在 FR-11 和 FR-12 之间，逻辑上它们是 FR-5 的衍生（window 改名、notify 集成、实体对齐），位置靠后不利于阅读。
**操作**：将 `### FR-5b`、`### FR-5c`、`### FR-5d` 三个小节整体移动到 `### FR-5: manager/ -> services/ 层` 之后、`### FR-6: src/config/ 预设集中` 之前。不改变各小节内部内容，仅调整位置。

---

### [R12] Spec 005 FR-7.3 句子拆分消除歧义

**文件**：`specs/005-extension-manifest/spec.md`
**问题**：FR-7.3 当前写「manifest 中的 `compatibility` 决定可在哪些工具（Layer 2，`~/.mam/mcp/` 对 MCP 不适用，MCP 直接写入工具配置）启用」，括号内把 Layer 2 和 `~/.mam/mcp/` 并列容易误解--Layer 2 是 `~/.mam/active/<tool>/`，不是 `~/.mam/mcp/`。
**操作**：拆为两句：「安装到 Layer 1（SSOT，`~/.mam/skills/<id>/`），manifest 中的 `compatibility` 决定可在哪些工具上启用。Skill 通过 Layer 2（`~/.mam/active/<tool>/`）符号链接映射；MCP 不走 Layer，直接写入工具配置文件，安装时在 `~/.mam/mcp/<id>/` 记录元数据。」

---

## 修改完成核对表

| 编号 | 文件 | 落地点 | 优先级 | 自检 |
|------|------|--------|--------|------|
| R1 | spec 001 | FR 编号 1-40 无重复 | 阻塞 | ☐ |
| R2 | spec 001 | notify 移回 FR-1 | 阻塞 | ☐ |
| R3 | spec 003 | get_sessions 三处统一 | 阻塞 | ☐ |
| R4 | constitution | IV 补 OpenClaw | 应修 | ☐ |
| R5 | constitution | 规模 3->4 工具 | 应修 | ☐ |
| R6 | spec 002 | 假设 5/8 矛盾消除 | 应修 | ☐ |
| R7 | spec 002 | 假设编号 7,8,9 | 应修 | ☐ |
| R8 | spec 001 | 假设 6 v1.3.0 | 应修 | ☐ |
| R9 | spec 005 | 假设合并 | 次要 | ☐ |
| R10 | spec 004 | FR-9.1/9.3 调和 | 次要 | ☐ |
| R11 | spec 002 | FR-5b/c/d 位置 | 次要 | ☐ |
| R12 | spec 005 | FR-7.3 拆句 | 次要 | ☐ |
