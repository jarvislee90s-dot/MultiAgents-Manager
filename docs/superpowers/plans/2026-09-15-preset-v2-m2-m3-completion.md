# 预设组 v2 · M2+M3 完结 实施计划（前端 + 对账体系 + 托盘/frontmatter）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 `docs/superpowers/specs/2026-09-14-preset-groups-v2-design.md` 的全部剩余内容（28 项缺口表的 A-E 组）：预设区 v2 UI（双分区/滑块开关/编辑弹窗/确认弹窗 v2）、资源卡片专属/常驻、账本-磁盘对账体系（§13）、托盘接线、frontmatter 预填、用户手册——做完后用户可完成全部手工测试。

**Architecture:** 后端补少量命令（对账扫描/处置、激活状态批量查询、frontmatter 解析、托盘刷新），前端在既有 React Query + radix Dialog + sonner 惯例上重写 PresetList 与确认弹窗、新增编辑弹窗/体检卡片/资源卡片增强。所有 M1 后端语义（独占/快照/暂存/绑定/常驻命令）已就绪，本计划只消费不改动。

**Tech Stack:** React 19 + TS + Tailwind v4 + radix(shadcn 风格 ui/dialog、ui/switch) + React Query + sonner + i18next；Rust/Tauri 2（少量命令 + reconcile 模块 + frontmatter 解析 + 托盘）；vitest + @testing-library/react。

**Spec:** `docs/superpowers/specs/2026-09-14-preset-groups-v2-design.md`（§5.5 开关 / §6 专属 / §7 UI / §13 对账；裁决见 §12 表末四行 2026-09-15）

## Global Constraints

- 注释中文、标识符英文；commit message 可英文；i18n 新 key 必须 zh/en **成对**添加（无 parity 测试，靠自觉 + 任务内自检步骤）
- 前端惯例（已核实）：queryKey 用导出的 const 元组（如 `export const PRESETS_KEY = ["presets"] as const`）；toast 用 `import { toast } from "sonner"` + `formatInvokeError(e, t)`；弹窗用 `@/components/ui/dialog`（无 Select/Textarea 原语，用原生 `<textarea>/<select>`，参照 ResourceByKindView）；组件测试用 `vi.hoisted` + `vi.mock("@tauri-apps/api/core")` 惯例（参照 `tests/legacySkillMigrationDialog.test.tsx`）
- 后端惯例：命令 `#[tauri::command]` + serde camelCase，注册进 `lib.rs` invoke_handler；测试资源名 `v2m2-` 前缀；集成测试断言用 `contains`；Rust 新测试进 `src-tauri/tests/preset_v2_test.rs` 或新文件 `reconcile_test.rs`
- **托盘坑（必读）**：菜单 id 是 `preset-{preset.id}` 而 `preset.id` 本身以 `preset-` 开头 → 实际 id 双前缀 `preset-preset-...`，解析时 `strip_prefix("preset-")` 恰好一次；`update_tray_menu`（语言/宠物路径）会**抹掉**预设菜单项——接线时必须收敛为统一重建入口
- 每任务收尾门禁：`cd src-tauri && cargo test`（涉 Rust 时）+ 仓库根 `pnpm lint && pnpm build`（涉前端时）；最终门禁含 `cargo clippy -- -D warnings && cargo fmt --check && pnpm format:check`
- M1 后端已实现的命令清单（消费不重造）：`get_preset/update_preset/restore_preset/get_active_preset/preview_apply_preset/set_resource_binding/list_resource_bindings/delete_resource_binding/set_tool_resident/list_tool_residents`（`commands/preset.rs`），`create_preset` 已收 `description/scope/boundTool` 可选参数

---

## Phase 0 · Rust 前置

### Task 1: MCP / plugin 的 sweep→restore 端到端测试（裁决 2 开工前置）

**Files:**
- Test: `src-tauri/tests/preset_v2_test.rs`（追加）

**Interfaces:**
- Consumes: M1 全部服务（`apply_preset`/`restore_tool`/`toggle_mcp`/`toggle_plugin`）
- Produces: 无新接口——纯回归锁

- [ ] **Step 1: 写 MCP 生命周期测试（先跑确认覆盖缺口，无 RED 阶段——这是补覆盖不是修 bug；若暴露真缺陷停下报告）**

```rust
/// MCP 端到端（spec §3.1 矩阵第 2 行）：启用态 MCP 被独占清扫移除配置段，恢复默认后重写
#[test]
fn mcp_sweep_restore_roundtrip() {
    support::setup();
    use multi_agents_manager_lib::database;
    use multi_agents_manager_lib::services::preset::{apply_preset, restore_tool};

    let home = dirs::home_dir().unwrap();
    // SSOT MCP 配置 + 注册
    let repo = home.join(".mam/mcp");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("v2m2-mcp-a.json"), r#"{"command":"x","args":[]}"#).unwrap();
    database::ensure_extension(&database::ExtensionRecord {
        id: "mcp-v2m2-mcp-a".into(), kind: "mcp".into(), name: "v2m2-mcp-a".into(),
        description: None, source_path: repo.join("v2m2-mcp-a.json").to_string_lossy().to_string(),
        source_url: None, version: None, tags: None, suite: None,
        source_tool: None, is_native: false,
    })
    .unwrap();
    // 基底：claude 启用该 MCP
    multi_agents_manager_lib::services::toggle_mcp("v2m2-mcp-a", "claude", true).unwrap();

    // 预设不含它 → 应用 → 配置段应被清扫（remove_mcp），assignment disabled
    let pid = database::create_preset("v2m2-mcp-p", &[]).unwrap(); // 空预设=白板（裁决 2026-09-15）
    let r = apply_preset(&pid, "claude").unwrap();
    assert!(r.disabled.contains(&"mcp-v2m2-mcp-a".to_string()), "{:?}", r.disabled);
    assert!(!multi_agents_manager_lib::services::is_skill_in_tool_range("v2m2-mcp-a", "claude") || {
        // assignment 断言走 list_assignments
        let a = database::list_assignments("claude")
            .into_iter()
            .find(|a| a.extension_id == "mcp-v2m2-mcp-a");
        a.map(|x| !x.enabled).unwrap_or(true)
    });

    // 恢复默认 → 配置段重写、assignment enabled
    let rr = restore_tool("claude").unwrap();
    assert!(rr.restored_mam.contains(&"mcp-v2m2-mcp-a".to_string()), "{:?}", rr.restored_mam);
    let a = database::list_assignments("claude")
        .into_iter()
        .find(|a| a.extension_id == "mcp-v2m2-mcp-a")
        .expect("assignment 应存在");
    assert!(a.enabled);
}
```

注意：上面第二个断言块写法别扭，直接用最终形态：

```rust
    let enabled = |ext: &str| {
        database::list_assignments("claude")
            .into_iter()
            .any(|a| &a.extension_id == ext && a.enabled)
    };
    assert!(!enabled("mcp-v2m2-mcp-a"), "清扫后应 disabled");
    // ... restore 后
    assert!(enabled("mcp-v2m2-mcp-a"));
```

- [ ] **Step 2: 写 plugin（file 型）往返测试**

同构：`~/.mam/plugins/v2m2-plug-a/` 目录 + `plugin-v2m2-plug-a` 注册（kind=plugin、tags=Some("file")）→ `toggle_plugin("v2m2-plug-a","claude",true,"file")` 建链 → 空预设 apply → 断言 `r.disabled.contains("plugin-v2m2-plug-a")` 且工具插件目录链接消失（`adapter.plugin_dirs()[0].join("v2m2-plug-a")` 不存在）→ restore → 断言链接回来（`exists()`）+ assignment enabled。config 型 plugin 的配置段路径与 MCP 同构（`toggle_plugin(...,"config")` 写工具配置 plugins 段），若工具配置格式导致无法在 claude 上构造，允许仅在 codex 或跳过并注明原因——**file 型为必测**。

- [ ] **Step 3: 跑测试**

Run: `cd src-tauri && cargo test v2m2_`
Expected: PASS。若 FAIL 暴露 M1 真缺陷 → 停下报告，不自行改 services。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/preset_v2_test.rs
git commit -m "test(preset-v2): MCP/plugin sweep-restore roundtrips (M2 precondition, ruling #2)"
```

---

## Phase A · 前端基建

### Task 2: 类型 / API 封装 / Query / Mock / 小命令 / 死代码

**Files:**
- Modify: `src/types/preset.ts`、`src/types/extension.ts`（加绑定类型）
- Modify: `src/lib/api/preset.ts`、`src/lib/query/queries/presets.ts`
- Create: `src/lib/query/queries/bindings.ts`
- Modify: `src-tauri/src/commands/preset.rs` + `src-tauri/src/lib.rs`（两个小命令）
- Delete: `src/lib/schemas/preset.ts`
- Modify: `src/tauri-mock.ts`、`tests/msw/tauriMocks.ts`

**Interfaces:**
- Produces（前端全量类型与 API，后续所有任务消费）:

```ts
// src/types/preset.ts（全文替换）
export interface PresetItem { extensionId: string; kind: string; extensionName: string; }
export interface PresetRecord { id: string; name: string; description: string; scope: "universal" | "tool"; boundTool: string | null; items: PresetItem[]; }
export interface PresetApplyResult { successCount: number; failures: string[]; conflicts: string[]; stashed: string[]; disabled: string[]; restoredNative: string[]; }
export interface RestoreResult { restoredMam: string[]; restoredNative: string[]; conflicts: string[]; }
export interface ApplyPreview { toEnable: string[]; filtered: string[]; toDisable: string[]; toStash: string[]; residentExempt: string[]; }
```

```ts
// src/types/extension.ts 追加
export interface ResourceBinding { extensionId: string; exclusiveTools: string; reason: string | null; updatedAt: string; }
export interface ActivePreset { toolId: string; presetId: string; }
```

```ts
// src/lib/api/preset.ts 追加/修改（保留现有 7 个函数名，扩展签名）
export async function createPreset(name: string, items: [string, string][],
  description?: string, scope?: string, boundTool?: string): Promise<string>
export async function getPreset(presetId: string): Promise<PresetRecord | null>
export async function updatePreset(id: string, name: string, description: string,
  scope: string, boundTool: string | null, items: [string, string][]): Promise<void>
export async function restorePreset(toolId: string): Promise<RestoreResult>
export async function getActivePreset(toolId: string): Promise<string | null>
export async function listActivePresets(): Promise<ActivePreset[]>   // 新 Rust 命令
export async function getToolActiveResources(toolId: string): Promise<[string, string, string][]>  // 新 Rust 命令 (extId, kind, origin)
export async function previewApplyPreset(presetId: string, toolId: string): Promise<ApplyPreview>
export async function setResourceBinding(extensionId: string, exclusiveTools: string[], reason?: string): Promise<void>
export async function listResourceBindings(): Promise<ResourceBinding[]>
export async function deleteResourceBinding(extensionId: string): Promise<void>
export async function setToolResident(toolId: string, extensionId: string, resident: boolean): Promise<void>
export async function listToolResidents(toolId: string): Promise<string[]>
```

- Produces（Rust 两个小命令，`commands/preset.rs` 追加 + `lib.rs` 注册）:

```rust
/// 全工具激活预设（开关状态批量数据源，避免前端 N 次 invoke）
#[tauri::command]
pub fn list_active_presets() -> Vec<serde_json::Value> {
    crate::adapter::TOOL_IDS
        .iter()
        .filter_map(|tool| {
            crate::database::get_base_snapshot(tool)
                .and_then(|(active, _)| active)
                .map(|p| serde_json::json!({ "toolId": tool, "presetId": p }))
        })
        .collect()
}

/// 工具当前生效资源集合（FR-24「存为预设」预填数据源）
#[tauri::command]
pub fn get_tool_active_resources(tool_id: String) -> Vec<(String, String, String)> {
    crate::services::preset::snapshot::scan_tool_state(&tool_id)
        .into_iter()
        .map(|i| (i.extension_id, i.kind, i.origin))
        .collect()
}
```

注意 `snapshot` 模块需 pub 可达（M1 已 `pub mod snapshot;`）。serde camelCase 由 tuple 自动序列化为数组。

- Produces（query）:

```ts
// src/lib/query/queries/presets.ts（全文替换，启用起来）
import { useQuery } from "@tanstack/react-query";
import { listPresets, listActivePresets } from "@/lib/api/preset";
export const PRESETS_KEY = ["presets"] as const;
export const ACTIVE_PRESETS_KEY = ["active-presets"] as const;
export const usePresetsQuery = () => useQuery({ queryKey: PRESETS_KEY, queryFn: listPresets, staleTime: 5000 });
export const useActivePresetsQuery = () => useQuery({ queryKey: ACTIVE_PRESETS_KEY, queryFn: listActivePresets, staleTime: 5000 });
```

```ts
// src/lib/query/queries/bindings.ts（新建）
export const BINDINGS_KEY = ["resource-bindings"] as const;
export const useResourceBindingsQuery = () => useQuery({ queryKey: BINDINGS_KEY, queryFn: listResourceBindings, staleTime: 10000 });
```

- [ ] **Step 1: Rust 两命令 + 注册 + `cargo test`**（命令无逻辑分支，测试走 Task 9/15 的组件测试间接覆盖；本步只验编译与注册）
- [ ] **Step 2: 类型/api/query/删死代码落地**
- [ ] **Step 3: Mock 更新**——`src/tauri-mock.ts`：修正 `list_presets` case 至 v2 形状（`{id,name,description,scope,boundTool,items:[{extensionId,kind,extensionName}]}`，给 1 通用 + 1 tool 私有样例）；`check_preset_compatibility` 修正为 `{id,name,kind}` 形状；新增 `get_preset/update_preset/restore_preset/get_active_preset/list_active_presets/get_tool_active_resources/preview_apply_preset/set_resource_binding/list_resource_bindings/delete_resource_binding/set_tool_resident/list_tool_residents` 各 case（读命令返回 fixture，写命令 resolve undefined）。`tests/msw/tauriMocks.ts`：`mockPresets` 升 v2 形状；新写命令加进 `:117-123` 分组 fall-through；读命令加 fixture case
- [ ] **Step 4: 门禁**：`cd src-tauri && cargo test`；`pnpm lint && pnpm build`
- [ ] **Step 5: Commit** `feat(preset-v2): frontend plumbing — types/api/queries/mocks + list_active_presets & get_tool_active_resources commands`

### Task 3: i18n 全量 key（zh/en 成对）

**Files:** `src/i18n/locales/zh.json`、`en.json`

**Interfaces:** Produces 后续任务消费的 key 树（嵌套组惯例，参照 `resources.migration.*`）：

- [ ] **Step 1: zh.json 写入以下 key 组**（文案自拟中文，简洁准确）：
  - `presets` 扩展：`universalSection`（「通用预设组」）、`toolScopedSection`（「工具私有预设组」）、`description`、`descriptionPlaceholder`、`edit`、`editTitle`、`type`、`typeUniversal`、`typeTool`、`boundTool`、`addItem`、`removeItem`、`nativeSkills`（「原生技能」）、`exclusiveBadge`（「专属」）、`residentBadge`（「常驻」）、`switchToToolScopedHint`（「所选内容含专属/原生资源，建议转为工具私有」）、`activeOn`（「激活于 {{tool}}」）、`saveFailedActive`（「该预设正在激活中，请先恢复默认」）、`cannotDeleteActive`、`whiteboardWarning`（「⚠️ 该预设对此工具可用项为 0，应用后将只保留常驻资源（白板模式）」）、`applyResult`（「成功 {{n}} · 停用 {{d}} · 暂存 {{s}}」）、`restoreDone`（「已恢复默认：重建 {{m}} · 回移 {{n}}」）、`restoreConflicts`、`filteredCount`、`toStashCount`、`toDisableCount`、`residentExemptCount`、`saveAsPreset`（「将当前组合存为预设」）
  - `resources` 扩展：`binding.editTitle`、`binding.tools`、`binding.reason`、`binding.reasonPlaceholder`、`binding.save`、`binding.clear`、`resident.toggle`、`health.title`（「一致性体检」）、`health.runNow`（「立即体检」）、`health.ok`（「账本与磁盘一致」）、`health.L1`/`L2`/`L3`/`L4`（分类名）、`health.fixA`（「按名单修磁盘」）、`health.fixB`（「按磁盘回写名单」）、`health.fixC`（「暂不处理」）、`health.needsManual`（「需人工处理」）、`health.batchA`、`health.batchB`、`health.invariantBroken`、`health.stashPending`、`health.suggestion`（「frontmatter 声明待确认」）
  - `tray` 扩展：`presetsLabel`（「预设」）、`on`、`off`
- [ ] **Step 2: en.json 逐 key 英文成对**
- [ ] **Step 3: 自检**：`node -e` 脚本比对两文件叶子路径集合一致（写一次性命令，不留测试文件）
- [ ] **Step 4: Commit** `chore(preset-v2): i18n keys zh/en for M2+M3`

---

## Phase B · 预设区 v2

### Task 4: PresetList v2——双分区 + 预设卡片 + 数据层迁移

**Files:**
- Rewrite: `src/components/presets/PresetList.tsx`
- Unchanged: `src/components/resources/ExtensionList.tsx`（继续传 `extensions` prop——编辑弹窗需要；或改为弹窗自取 query，实现者择一，推荐后者则同步删 prop）

**Interfaces:**
- Consumes: Task 2 全部（`usePresetsQuery`/`useActivePresetsQuery`/`useEnabledToolsQuery`）、Task 3 keys
- Produces: `PresetList`（默认导出组件，无 props）；内部结构：

```
<Card>
  标题行：presets.title + 「+ 创建」按钮（开编辑弹窗，空表单）
  通用预设组：scope==="universal" 的预设卡片列表
  工具私有预设组：按 boundTool 分组（组头 = 工具名），scope==="tool" 的卡片
</Card>
预设卡片：名称 + description 摘要（1 行截断）+ items 计数徽标；整卡可点 → onEdit(preset)
        开关区（Task 5 接入）：通用 = enabledTools.map(Switch per tool)；私有 = 绑定工具 1 枚
        删除按钮（带确认 Dialog）；激活中删除 → catch PRESET_ACTIVE 错误 → toast presets.cannotDeleteActive
```

- [ ] **Step 1: 实现骨架**（不含开关交互——Task 5 填充）。数据层从 `useState + invoke` 迁到 `usePresetsQuery`；变更后 `qc.invalidateQueries({queryKey: PRESETS_KEY})`。`SubAgentPresetActions` 原样保留迁移（其 `apply_preset_to_subagent/deactivate_preset_from_subagent` 语义本期不变）
- [ ] **Step 2: 手动编译门禁** `pnpm lint && pnpm build`
- [ ] **Step 3: Commit** `feat(preset-v2): PresetList v2 dual-zone skeleton on React Query`

### Task 5: 滑块开关——开=确认弹窗、关=直接恢复 + toast

**Files:**
- Modify: `src/components/presets/PresetList.tsx`
- Create: `src/components/presets/ApplyConfirmDialog.tsx`（Task 6 完成前先用最小版：标题 + 确认按钮）

**Interfaces:**
- Produces: 开关行为契约（spec §5.5）：

```tsx
// 伪代码契约
const onSwitch = async (presetId: string, toolId: string, next: boolean) => {
  if (next) { setConfirm({presetId, toolId}); return; }  // 开 → 确认弹窗（Task 6 v2）
  // 关 → 直接执行 + toast（恢复是安全动作）
  const rr = await restorePreset(toolId);
  toast.success(t("presets.restoreDone", { m: rr.restoredMam.length, n: rr.restoredNative.length }));
  if (rr.conflicts.length) toast.warning(t("presets.restoreConflicts"), { description: rr.conflicts.join("\n") });
  qc.invalidateQueries({ queryKey: ACTIVE_PRESETS_KEY });
};
// 确认弹窗 onConfirm → applyPreset(presetId, toolId) → toast 结果（presets.applyResult 模板）
// 开关 checked = activePresets.find(a => a.toolId===tool.id)?.presetId === preset.id
// 同工具开新预设：apply 成功后 invalidate ACTIVE_PRESETS_KEY，旧开关自动翻关
// 失败：catch → toast.error(t("presets.applyFailed", {error: formatInvokeError(e, t)}))
```

能力门控（§7.3）：`kindSupported(tool, kind)`（照抄 `ResourceByKindView.tsx:152-158` 的写法）——预设 items 含某类资源而目标工具不支持时，该工具的 Switch `disabled` + title 说明。工具私有预设的绑定工具未启用时不渲染开关。

- [ ] **Step 1: 实现开关交互 + 最小确认弹窗**（`ui/switch` 原语已有）
- [ ] **Step 2: `pnpm lint && pnpm build` + Commit** `feat(preset-v2): preset×tool switch — on=confirm dialog, off=direct restore+toast`

### Task 6: 应用确认弹窗 v2（preview 数据 + 白板强警示）

**Files:**
- Rewrite: `src/components/presets/ApplyConfirmDialog.tsx`（替代旧 `CompatibilityDialog.tsx`；完成后删除旧文件并清理引用）
- Modify: `src/lib/api/resource.ts`（`checkPresetCompatibility` 若无他用则删）

**Interfaces:**
- Consumes: `previewApplyPreset(presetId, toolId) -> ApplyPreview`
- Produces: `ApplyConfirmDialog({open, presetId, toolId, toolName, onClose, onConfirm})`，渲染（spec §7.3 + 裁决）：

```
标题 presets.applyTo
五段清单（空段隐藏）：将启用 toEnable / 被过滤 filtered（逐条含原因文案）/ 将停用 toDisable /
  将暂存 toStash（点名）/ 常驻豁免 residentExempt（计数）
白板警示：toEnable.length === 0 && (toDisable.length || toStash.length) →
  顶部红/橙色 Alert 块 presets.whiteboardWarning，确认按钮改 destructive variant（强警示，允许继续——裁决 2026-09-15）
确认按钮 loading 态（应用执行中禁用）；取消恒可用
```

- [ ] **Step 1: 实现**（数据在 `open` 时 useEffect 拉 preview，参照旧 CompatibilityDialog 的 loading 处理）
- [ ] **Step 2: 删旧文件**：确认 `grep -r CompatibilityDialog src/` 仅 PresetList 引用后删除
- [ ] **Step 3: `pnpm lint && pnpm build` + Commit** `feat(preset-v2): apply confirm dialog v2 — preview-driven with whiteboard warning`

### Task 7: 预设编辑弹窗（新建/编辑共用）

**Files:**
- Create: `src/components/presets/PresetEditDialog.tsx`
- Modify: `src/components/presets/PresetList.tsx`（卡片点击 → 编辑；创建按钮 → 空表单）

**Interfaces:**
- Consumes: `createPreset/updatePreset/getPreset/listExtensionsWithAssignments(经 prop 或 query)/listResourceBindings/useEnabledToolsQuery/listToolResidents`
- Produces: `PresetEditDialog({open, preset: PresetRecord | null /*null=新建*/, onClose})`，表单（spec §7.2）：

```
类型 Radio：presets.typeUniversal / typeTool + boundTool 原生 <select>（仅启用工具；typeTool 必选）
名称 Input（必填，presets.nameRequired 校验）+ 描述 <textarea>（选填备忘录）
套件列表：三组（skill/mcp/plugin）复选，数据源 = extensions query；
  - 通用：仅非 is_native 资源；绑定为专属的项显示 presets.exclusiveBadge 徽标（不禁选）
  - 私有：不适配（binding.exclusiveTools 不含 boundTool）→ checkbox disabled + 原因 title；
        追加「原生技能」组 = extensions 中 is_native && sourceTool === boundTool
转类型建议：通用模式下勾选了（专属资源 || 无——通用不可能选原生）专属资源时，
  显示提示条 presets.switchToToolScopedHint + 「转工具私有」快捷按钮（切 Radio + 要求选工具）
保存：新建 → createPreset(表单)；编辑 → updatePreset；catch 激活守卫错误 → toast presets.saveFailedActive
至少选 1 项校验（presets.selectAtLeastOne）——白板预设指「应用时可用项为 0」，编辑期仍要求 ≥1 项
```

- [ ] **Step 1: 实现弹窗**（编辑态 `getPreset` 拉详情预填；`extensions` 的 `isNative` 字段若 `ExtensionWithAssignments` 类型缺失则补——Rust 侧 `ExtensionWithAssignments` 已含 `suite/source_tool` 但**无 is_native**，需在 `commands/resource.rs:82-111` 的结构体与映射补 `is_native` 一行，前端类型同步）
- [ ] **Step 2: `cd src-tauri && cargo test` + `pnpm lint && pnpm build`**
- [ ] **Step 3: Commit** `feat(preset-v2): preset edit dialog — type/description/suite list with native skills & binding-aware gating`

### Task 8: FR-24 存为预设 + 收尾清理

**Files:**
- Modify: `src/components/resources/ResourceByKindView.tsx`（或 ExtensionList 层——放在按工具视图 `ResourceByToolView.tsx` 的工具组头部更贴语义，实现者按现有布局择一）
- Modify: `src/components/presets/PresetList.tsx`（移除 deactivate 旧路径残留）
- Modify: `tests/msw/tauriMocks.ts`、`src/tauri-mock.ts`（`deactivate_preset` case 删除）

**Interfaces:**
- Produces:
  1. 工具组头部「presets.saveAsPreset」按钮 → `getToolActiveResources(toolId)` 预填 → 打开 `PresetEditDialog`（preset=null，scope 预选 tool、boundTool 预选该工具，items 预勾 = 返回的 (extId, kind)）
  2. 前端全部 `deactivatePreset` 调用已由 Task 5 的 `restorePreset` 替代；本任务删除 `src/lib/api/preset.ts` 中的 `deactivatePreset` 导出与 mock case。**Rust 命令 `deactivate_preset` 保留**（托盘/外部兼容，M3 后评估移除）——只清前端

- [ ] **Step 1: 实现**；**Step 2: `pnpm lint && pnpm build` + Commit** `feat(preset-v2): save-active-combo-as-preset entry (FR-24) + retire frontend deactivate path`

### Task 9: 预设前端组件测试

**Files:**
- Create: `tests/presetList.test.tsx`、`tests/presetEditDialog.test.tsx`、`tests/applyConfirmDialog.test.tsx`

**Interfaces:** 照 `tests/legacySkillMigrationDialog.test.tsx` 惯例（`vi.hoisted` + `vi.mock("@tauri-apps/api/core")` + i18n `changeLanguage("zh")`）。

- [ ] **Step 1: 写三类断言（每文件至少 3 条用例）**：
  1. `presetList`：mock `list_presets` 返回 1 通用 + 1 私有（codex 绑定）→ 断言两分区标题渲染、私有卡出现在 codex 组；mock `list_active_presets` 返回 `[{toolId:"codex",presetId:私有id}]` → 断言 codex 开关 checked、其他工具 unchecked
  2. `presetEditDialog`：空名称提交 → 不得 invoke `create_preset`；勾 1 项 + 填名 → 断言 `create_preset` args 含 `description/scope/boundTool`；私有模式 mock bindings 含不适配资源 → 断言该 checkbox `disabled`
  3. `applyConfirmDialog`：mock `preview_apply_preset` 返回 `toEnable:[]` + `toStash:["x"]` → 断言白板警示文案出现且确认按钮为 destructive；正常 preview → 断言五段清单渲染
- [ ] **Step 2: `pnpm vitest run tests/preset*`（或 `npx vitest run`）全绿 + `pnpm lint`**
- [ ] **Step 3: Commit** `test(preset-v2): frontend component tests — dual zone, edit form gating, whiteboard warning`

---

## Phase C · 资源卡片增强

### Task 10: 专属徽标 + 编辑弹层 + 不适配置灰

**Files:**
- Modify: `src/components/resources/ResourceByKindView.tsx`（三行渲染处）

**Interfaces:**
- Consumes: `useResourceBindingsQuery`、`setResourceBinding/deleteResourceBinding`
- Produces: 每资源行名称旁徽标（`presets.exclusiveBadge`，有 binding 且 exclusiveTools 非空时显示）→ 点击开小 Dialog：原生多选 `<select multiple>` 或复选列表选允许工具（数据 = `useEnabledToolsQuery`）+ 原因 `<textarea>` + 保存/清除；行内对不适配工具的启用按钮 `disabled`（复用 `kindSupported` 同款 title 模式，文案 `presets.exclusiveBadge` + binding 工具列表）

- [ ] **Step 1: 实现**；**Step 2: `pnpm lint && pnpm build` + Commit** `feat(preset-v2): exclusive badge + binding editor + per-tool gating on resource cards`

### Task 11: 常驻锁开关

**Files:**
- Modify: `src/components/resources/ResourceByKindView.tsx`（三行渲染处，紧邻启停按钮）

**Interfaces:**
- Consumes: `setToolResident/listToolResidents`
- Produces: 每资源行 × 每工具一枚锁形开关（`ui/switch` + 🔒 图标，title=`resources.resident.toggle`）。`listToolResidents(toolId)` 按 query（新 key `["tool-residents", toolId]`，写在 bindings.ts）。常驻态视觉：开关 on + 徽标 `presets.residentBadge`

- [ ] **Step 1: 实现**；**Step 2: 门禁 + Commit** `feat(preset-v2): per-tool resident lock switch on resource cards`

---

## Phase D · 账本-磁盘对账（spec §13）

### Task 12: reconcile 扫描引擎 + scan 命令

**Files:**
- Create: `src-tauri/src/services/resource/reconcile.rs`（`services/resource/mod.rs` 加 `pub mod reconcile;`）
- Modify: `src-tauri/src/commands/resource.rs` + `lib.rs`（注册）
- Test: `src-tauri/tests/reconcile_test.rs`（新建，`mod support;` 同 preset_v2_test）

**Interfaces:**
- Produces:

```rust
// services/resource/reconcile.rs
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftItem {
    pub tool_id: String,
    pub kind: String,          // "L1"|"L2"|"L3"|"L4"
    pub extension_id: String,  // L1/L2 = 账本 skill-<name>；L3/L4 推导同名
    pub path: String,          // 工具目录下的路径
}

/// 扫描 enabled 工具的账本-磁盘漂移（§13）：disabled 工具排除（W5 名册语义）；
/// 只看 skill（mcp/plugin 后续版本）；L4 外链只报告不接管
pub fn scan_drift() -> Vec<DriftItem> {
    let home = dirs::home_dir().unwrap_or_default();
    let mam_root = home.join(".mam");
    let mut out = Vec::new();
    for tool in crate::adapter::TOOL_IDS {
        if !crate::database::get_tool_enabled(tool) {
            continue; // W5 停用 = 名册保留语义，不算漂移
        }
        // 账本侧：工具级 enabled skill 分配
        let ledger: Vec<String> = crate::database::list_assignments(tool)
            .into_iter()
            .filter(|a| a.enabled && a.sub_agent_id.is_none() && a.extension_id.starts_with("skill-"))
            .map(|a| a.extension_id)
            .collect();
        let Some(dir) = crate::adapter::primary_skill_dir(tool) else { continue };
        // 磁盘侧：目录里的 MAM 链接名集合
        let mut disk_mam_links: Vec<String> = Vec::new();
        if dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    let path = e.path();
                    let Some(name) = e.file_name().to_str() else { continue };
                    if name == "subagents" {
                        continue;
                    }
                    let ext_id = format!("skill-{}", name);
                    if path.is_symlink() {
                        let target = std::fs::read_link(&path).unwrap_or_default();
                        let is_mam = target.starts_with(&mam_root);
                        if is_mam {
                            disk_mam_links.push(ext_id);
                        } else {
                            // L4 外链：MAM 不接管，只报告
                            out.push(DriftItem {
                                tool_id: tool.to_string(),
                                kind: "L4".into(),
                                extension_id: ext_id,
                                path: path.to_string_lossy().to_string(),
                            });
                        }
                    }
                    // 真目录：若名在 ledger → L2 占位；否则是正常原生，忽略
                    else if path.is_dir() && ledger.iter().any(|l| l == &ext_id) {
                        out.push(DriftItem {
                            tool_id: tool.to_string(),
                            kind: "L2".into(),
                            extension_id: ext_id,
                            path: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
        // L1 缺链：账本有、磁盘既无链接也无目录
        for ext_id in &ledger {
            if !disk_mam_links.contains(ext_id) {
                let name = ext_id.strip_prefix("skill-").unwrap_or(ext_id);
                if !dir.join(name).exists() {
                    out.push(DriftItem {
                        tool_id: tool.to_string(),
                        kind: "L1".into(),
                        extension_id: ext_id.clone(),
                        path: dir.join(name).to_string_lossy().to_string(),
                    });
                }
            }
        }
        // L3 多链：磁盘 MAM 链接、账本无
        for ext_id in &disk_mam_links {
            if !ledger.contains(ext_id) {
                out.push(DriftItem {
                    tool_id: tool.to_string(),
                    kind: "L3".into(),
                    extension_id: ext_id.clone(),
                    path: dir.join(ext_id.strip_prefix("skill-").unwrap_or(ext_id)).to_string_lossy().to_string(),
                });
            }
        }
    }
    out
}
```

```rust
// commands/resource.rs
#[tauri::command]
pub fn scan_ledger_drift() -> Vec<crate::services::resource::reconcile::DriftItem> {
    crate::services::resource::reconcile::scan_drift()
}
```

- [ ] **Step 1: 测试先写**（`reconcile_test.rs`，资源名 `v2m2-rc-*`）：构造 L1（enabled assignment、目录删链）/ L2（enabled + 真目录）/ L3（手工 create_link 到 Layer2 目标、无账本）/ L4（symlink 指向 `/tmp` 非 mam 目标）各一 → `scan_drift()` 断言四类各命中对应 extension_id；再构造 disabled 工具的同类漂移 → 断言不出现
- [ ] **Step 2: 跑出失败 → 实现 → 全绿**
- [ ] **Step 3: `cargo test` + 注册 + Commit** `feat(preset-v2): ledger-disk drift scan (L1-L4) + scan_ledger_drift command`

### Task 13: 对账处置命令（逐条 a/b + 批量）

**Files:**
- Modify: `src-tauri/src/services/resource/reconcile.rs`、`src-tauri/src/commands/resource.rs` + `lib.rs`
- Test: `src-tauri/tests/reconcile_test.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileOutcome {
    pub fixed: bool,
    pub needs_manual: bool,   // a 模式下 L2 内容不一致 → true（绝不删用户真目录）
    pub message: String,
}

/// 单条处置（§13）。mode: "a" 账本为准修磁盘 | "b" 磁盘为准回写账本
pub fn reconcile_one(item: &DriftItem, mode: &str) -> ReconcileOutcome {
    let name = item.extension_id.strip_prefix("skill-").unwrap_or(&item.extension_id);
    match (item.kind.as_str(), mode) {
        ("L1", "a") => enable_skill_for_tool(name, &item.tool_id) → fixed / 失败 message,
        ("L1", "b") => upsert_assignment(ext_id, tool, false, "missing") → fixed,
        ("L2", "a") => enable_skill_for_tool（其守卫：内容一致才替换，不一致 Err）
                      → Err 含「不一致」→ needs_manual=true 保留原 message；否则 fixed,
        ("L2", "b") => upsert_assignment(..., false, "missing")（真目录保留原生态）→ fixed,
        ("L3", "a") => disable_skill_for_tool(name, tool)（清链含 Layer2 级联）→ fixed,
        ("L3", "b") => upsert_assignment(ext_id, tool, true, "valid") → fixed,
        ("L4", _)   => needs_manual=true（外链 MAM 不接管）,
        _ => message "未知组合",
    }
}

// 命令：
#[tauri::command] pub fn reconcile_item(item: DriftItem, mode: String) -> ReconcileOutcome
#[tauri::command] pub fn reconcile_tool_batch(tool_id: String, mode: String) -> Vec<ReconcileOutcome>  // 该工具全部 L1-L3 逐条同 mode；L4 恒 needs_manual
```

- [ ] **Step 1: 测试先写**：L1-a 重建链接（断言工具目录链接回来）；L2-b 账本 disabled + 真目录原样；L2-a 内容不一致 → `needs_manual` 且真目录**仍在**（防删回归锁，参照 Patch 6 教训）；L3-a 清链；L4 任意 mode → needs_manual
- [ ] **Step 2: 失败 → 实现 → 绿 → `cargo test` + Commit** `feat(preset-v2): reconcile actions — per-item & batch a/b with needs-manual escalation`

### Task 14: preset health 聚合命令 + 启动接线

**Files:**
- Modify: `src-tauri/src/services/preset/mod.rs`（新 `pub fn preset_health()`）、`src-tauri/src/commands/preset.rs` + `lib.rs`
- Modify: `src-tauri/src/lib.rs` 启动线程（漂移扫描，结果仅 log——UI 现算）

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetHealth {
    pub invariants: Vec<String>,   // check_snapshot_invariants() 输出
    pub stash_pending: Vec<crate::database::StashEntryRecord>, // unrestored_stash(None)
    pub drift: Vec<crate::services::resource::reconcile::DriftItem>,
}
#[tauri::command]
pub fn get_preset_health() -> PresetHealth { services::preset::preset_health() }
```

- [ ] **Step 1: 实现 + 测试**（`preset_health` 组装三源，单测断言聚合正确即可）；**Step 2: 启动线程在 backfill 后加 `for d in reconcile::scan_drift() { log::warn!("[漂移{}] {} {}", ...) }`；Step 3: cargo test + Commit** `feat(preset-v2): preset health aggregation command + startup drift logging`

### Task 15: 一致性体检卡片 + 设置页「立即体检」

**Files:**
- Create: `src/components/resources/HealthCheckCard.tsx`
- Modify: `src/components/resources/ExtensionList.tsx`（资源 tab 顶部挂载，`ResourceByKindView` 之前）+ 角标（`ExtensionList` 或 `pages/home.tsx` 资源 tab 标题处红点）
- Modify: `src/pages/settings.tsx`（`SettingSection` union 加 `"health"` + `menuItems` + 区块）
- Modify: `src/lib/api/preset.ts`/`reconcile api`（`scanLedgerDrift/reconcileItem/reconcileToolBatch/getPresetHealth` 封装）+ `src/tauri-mock.ts`/`tauriMocks.ts`

**Interfaces:**
- Produces: `HealthCheckCard`：

```
数据：useQuery(["preset-health"], getPresetHealth)
无异常 → 折叠态一行 resources.health.ok + 「立即体检」按钮（refetch）
有异常 → 展开三段：
  ① 漂移明细：按 toolId 分组表格（列：分类徽标 L1-L4 / 资源 / 路径 / 操作）
     操作 = 每行三个小按钮 health.fixA/fixB/fixC（c=无动作仅收起该行）；
     reconcileItem 返回 needsManual → 该行转橙色 health.needsManual 态（无按钮）
     组头批量：health.batchA / health.batchB
     每次处置后 invalidate ["preset-health"] + PRESETS/ACTIVE/SSOT 相关 key
  ② 不变量违背：逐条文案 + 「一键修复」= restorePreset(对应工具)（守卫：工具需启用，disabled 时提示先启用）
  ③ 残留暂存：列 skillName + 工具 + 「回移」按钮（后端复用 recover_orphans 单条？——最小实现：提示用户重启应用自动对账 + 可选加 Rust 单条命令 `restore_stash_entry(id)`，允许实现者二选一，选重启提示则文案注明）
设置页 health 区块：同数据的只读摘要 + resources.health.runNow 按钮（refetch 同 query）
```

- [ ] **Step 1: Rust 若选单条回移命令**：`#[tauri::command] pub fn restore_stash_entry(id: i64) -> Result<(), String>`（按账本查 entry → `stash::restore_stashed_skill`）；**Step 2: 前端实现 + mock**；**Step 3: `pnpm lint && pnpm build` + `cargo test`（若动 Rust）+ Commit** `feat(preset-v2): health check card — drift triage a/b/c with batch, invariant & stash panels`

---

## Phase E · M3

### Task 16: 托盘接线（选中态 + 直接执行 + 刷新收敛）

**Files:**
- Modify: `src-tauri/src/plugins/system_tray.rs`、`src-tauri/src/commands/session.rs`（刷新触发）、`src-tauri/src/lib.rs`
- Modify: `src/pages/home.tsx`、`src/components/common/language-toggle.tsx`（语言路径不再调 `update_tray_menu` 抹预设，改调新统一命令）
- Modify: `src/tauri-mock.ts`（`update_tray_menu` 已有 case，无需大改）

**Interfaces:**
- Produces:

```rust
// system_tray.rs 改造（三个点）：
// 1) on_menu_event 加 preset arm（:74-101 的 _ 之前）——点击=直接执行（裁决 2026-09-15，不弹确认）：
//    id 形如 "preset-preset-<...>"（双重前缀！），item_id.strip_prefix("preset-") 得 preset_id；
//    该 preset 为 universal → 弹主窗口并 emit 事件让用户选工具（无窗口选工具不可行）——
//    ★简化裁决（实现期定，记录进 progress.md）：universal 预设的托盘子菜单直接按工具展开（§7.5 原文），
//    子菜单项 id = "preset-tool-{preset_id}|{tool_id}"，点击 = 该工具上翻转（active==该预设 ? restore : apply）
//    tool 私有预设 = 顶层 CheckMenuItem，id 同 "preset-preset-..."，点击翻转绑定工具
// 2) update_tray_with_presets：preset 项改 CheckMenuItem（tauri 2 有）/子菜单，checked = get_active_preset(tool)==preset_id；
//    label 参数化（去掉硬编码中文，从命令入参传 t("tray.presetsLabel")）
// 3) 收敛重建入口：新增 #[tauri::command] refresh_tray(app, presets_label: String)
//    内部 = update_tray_menu 的基础项 + update_tray_with_presets 的预设项合并重建；
//    home.tsx / language-toggle.tsx / 预设变更后统一调 refresh_tray（替代原 update_tray_menu 调用点，
//    旧 update_tray_menu 命令保留兼容但前端不再用）
// 4) 刷新触发：session.rs 的计数启发式保留兜底；前端在 PRESETS/ACTIVE_PRESETS invalidate 处追加 invoke("refresh_tray", {presetsLabel: t(...)})
```

- [ ] **Step 1: Rust 改造 + `cargo test`**（托盘逻辑抽出的翻转判定纯函数 `preset_toggle_action(preset_id, tool_id) -> "apply"|"restore"` 写单测：active 匹配→restore，否则→apply）
- [ ] **Step 2: 前端调用点替换 + i18n 传参**；**Step 3: `pnpm lint && pnpm build` + Commit** `feat(preset-v2): tray wiring — check state, direct toggle, unified rebuild, i18n label`
- [ ] **Step 4: 启动竞态复评（Minor#10）**：托盘直接执行与启动 recover 线程的窗口已由 Patch 5 的 `is_some()` 守卫关闭——在 progress.md 记一行复核结论即可

### Task 17: frontmatter 解析 + 导入提示 + 存量建议入体检

**Files:**
- Create: `src-tauri/src/services/resource/frontmatter.rs`（`resource/mod.rs` 加 `pub mod frontmatter;`）
- Modify: `src-tauri/src/commands/skill.rs`（install_skill 返回值带建议）或 `commands/resource.rs`（import_native_resources）——挂在**手动导入**路径
- Modify: `commands/resource.rs` + `lib.rs`：`list_frontmatter_suggestions()` 命令
- Modify: `src/components/resources/HealthCheckCard.tsx`（建议段）+ `PresetEditDialog` 无需动（绑定已有）
- Test: `src-tauri/src/services/resource/frontmatter.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Produces:

```rust
// frontmatter.rs —— 解析 SKILL.md 头部 --- 包围的 YAML（自写极简行解析或 serde_yaml 若已在依赖——
// 查 Cargo.toml，没有就用 toml_edit 同思路的轻量手写：只取顶层标量数组字段，容错坏 YAML → None）
pub fn parse_supported_agents(skill_md: &str) -> Option<Vec<String>> {
    // 识别键（宽容别名）：supported_agents / agents / compatible_tools / tools
    // 值形态：YAML 数组 ["a","b"] 或逗号串 "a, b"（宽容）；键缺失/解析失败 → None
}

// 导入路径集成：install_skill / import_native_resources 成功后，
// 若 parse 命中且该 ext 无绑定行 → 在返回结构中带 suggestion: Option<{extension_id, tools}>
// （返回类型扩展为 struct ImportOutcome { ..., suggestion }——改动两命令签名，前端接）

#[tauri::command]
pub fn list_frontmatter_suggestions() -> Vec<serde_json::Value> {
    // 扫 ~/.mam/skills/*/SKILL.md：parse 命中 && resource_bindings 无行 → {extensionId, tools}
}
```

前端：导入命令返回 `suggestion` → Dialog（非 toast，需用户选择）：「检测到该技能声明仅支持 {tools}，标记为专属？」→ 确认 = `setResourceBinding(extId, tools)`；忽略 = 关闭。体检卡片第四段 `health.suggestion`：逐条「设为专属 / 忽略本轮」。

- [ ] **Step 1: 解析器 TDD**（用例：标准数组 / 逗号串 / 别名四键 / 坏 YAML / 缺字段 / 无 frontmatter → 各断言）
- [ ] **Step 2: 导入路径 + 存量命令 + 前端两处**
- [ ] **Step 3: `cargo test` + `pnpm lint && pnpm build` + Commit** `feat(preset-v2): frontmatter exclusive prefill — import-time prompt + stock suggestions in health card`

### Task 18: 用户手册 + 终门禁

**Files:**
- Modify: `docs/user-manual/zh/06-presets.md`、`en/06-presets.md`（占位补真：概念、双分区、开关、暂存区、常驻/专属、体检、托盘；对齐最终实现措辞）
- Modify: `.superpowers/sdd/progress.md`（终轮记录）

- [ ] **Step 1: 双语手册**（结构照 `docs/user-manual/zh/` 其他章节惯例）
- [ ] **Step 2: 顺手 DRY（缺口表 #28 / 评审 Minor#7）**：`apply_preset`/`restore_tool`/`execute_sweep` 三处近似的 enable/disable 分派闭包抽 `fn toggle_ext(ext_id, kind, tool_id, on) -> Result<(), String>`（`services/preset/mod.rs`），行为不变、全量测试绿即算完成
- [ ] **Step 3: 终门禁**：`cd src-tauri && cargo test && cargo clippy -- -D warnings && cargo fmt --check`；`pnpm format:check && pnpm lint && pnpm build`；`npx vitest run`（全前端测试）
- [ ] **Step 4: Commit** `docs(preset-v2): user manual zh/en + toggle_ext DRY + final gate`

---

## 执行顺序与依赖

```
T1 (Rust 测试前置) → T2 (前端基建) → T3 (i18n) → T4 → T5 → T6 → T7 → T8 → T9 (Phase B 串行)
                                        T2/T3 ──→ T10 → T11 (Phase C，可与 Phase B 后半并行)
T12 → T13 → T14 → T15 (Phase D 串行；T12 可与 Phase B 并行启动)
T15 → T16 → T17 → T18 (Phase E)
```

风险点：(1) T7 需要给 `ExtensionWithAssignments` 补 `is_native` 透出（Rust 一行 + 类型）；(2) T16 托盘子菜单/CheckMenuItem 是 Tauri 2 API 细节，若 `CheckMenuItem` 泛型组装受阻，退化为普通 MenuItem + 「(开)/(关)」后缀文案（记录进 progress.md）；(3) T17 YAML 解析若引 `serde_yaml` 需查 Cargo.toml 依赖策略——默认手写轻量解析，避免新增依赖。
