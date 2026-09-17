import { vi } from "vitest";

export const mockSessions = {
  sessions: [
    {
      id: "session-1",
      agentType: "claude",
      projectName: "project1",
      projectPath: "/tmp/project1",
      title: null,
      gitBranch: "main",
      githubUrl: null,
      status: "processing",
      lastMessage: "正在处理...",
      lastMessageRole: "assistant",
      lastActivityAt: new Date().toISOString(),
      pid: 12345,
      cpuUsage: 12.5,
      activeSubagentCount: 0,
      form: "cli",
      jumpSupported: true,
    },
    {
      id: "session-2",
      agentType: "codex",
      projectName: "project2",
      projectPath: "/tmp/project2",
      title: null,
      gitBranch: "develop",
      githubUrl: null,
      status: "waiting",
      lastMessage: "等待用户输入",
      lastMessageRole: "assistant",
      lastActivityAt: new Date().toISOString(),
      pid: 12346,
      cpuUsage: 0,
      activeSubagentCount: 0,
      form: "cli",
      jumpSupported: true,
    },
  ],
  totalCount: 2,
  waitingCount: 1,
};

export const mockExtensions = [
  {
    id: "brainstorming",
    kind: "skill",
    name: "Brainstorming",
    description: "头脑风暴 skill",
    sourceTool: "claude",
    // isNative:true + sourceTool 已设：与 src/tauri-mock.ts 同构，可目验「原生技能」分组（T7）
    isNative: true,
    assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
  },
];

// 预设组 v2 形状（1 通用 + 1 tool 私有）：与 src/tauri-mock.ts 的 list_presets 样例同构，
// 与 Rust PresetRecord（serde camelCase）一致（mock parity 收口门禁）
export const mockPresets = [
  {
    id: "preset-1",
    name: "前端开发",
    description: "前端开发常用组合",
    scope: "universal",
    boundTool: null,
    items: [{ extensionId: "brainstorming", kind: "skill", extensionName: "Brainstorming" }],
  },
  {
    id: "preset-2",
    name: "Claude 专属",
    description: "仅 Claude Code 可见的工具私有预设样例",
    scope: "tool",
    boundTool: "claude",
    items: [],
  },
];

// convertFileSrc：asset 协议路径转换（petRuntime/向导预览用）
export const convertFileSrcMock = (path: string) => `asset://mock/${path}`;

// —— 一致性体检 fixture（T15，与 src/tauri-mock.ts 同构）：三类异常样例 + 空健康两态。
// setMockHealthMode("empty") 切空健康（组件折叠态用例），默认异常样例 ——
export const mockLedgerDrift = [
  {
    toolId: "claude",
    kind: "L1",
    extensionId: "skill-brainstorming",
    path: "/Users/jarvis/.claude/skills/brainstorming",
  },
  {
    toolId: "claude",
    kind: "L3",
    extensionId: "skill-systematic-debugging",
    path: "/Users/jarvis/.claude/skills/systematic-debugging",
  },
  {
    toolId: "codex",
    kind: "L2",
    extensionId: "skill-chinese-code-review",
    path: "/Users/jarvis/.codex/skills/chinese-code-review",
  },
  {
    toolId: "codex",
    kind: "L4",
    extensionId: "skill-using-superpowers",
    path: "/Users/jarvis/.codex/skills/using-superpowers",
  },
];

export const mockPresetHealthIssues = {
  invariants: ["codex：快照存在但无激活预设"],
  stashPending: [
    {
      id: 1,
      toolId: "claude",
      skillName: "legacy-native-skill",
      stashedPath: "/Users/jarvis/.mam/stash/claude/skills/legacy-native-skill",
      originalPath: "/Users/jarvis/.claude/skills/legacy-native-skill",
      createdAt: new Date().toISOString(),
      restoredAt: null,
    },
  ],
  drift: mockLedgerDrift,
  // 空目录（wave33 Item D，与 src/tauri-mock.ts 同构）：mam 仓库 + 工具目录各一
  emptyDirs: [
    { owner: "mam", path: "/Users/jarvis/.mam/skills/empty-suite-dir" },
    { owner: "tool:claude", path: "/Users/jarvis/.claude/skills/empty-dir" },
  ],
};

export const mockPresetHealthEmpty = { invariants: [], stashPending: [], drift: [], emptyDirs: [] };

let healthMode: "issues" | "empty" = "issues";
export function setMockHealthMode(mode: "issues" | "empty") {
  healthMode = mode;
}

// frontmatter 存量建议 fixture（Task 17，spec §6）：与 src/tauri-mock.ts 同构
// （extensionId 不在 list_resource_bindings 样例 → 可出建议）。
// setMockFmMode("empty") 切空建议，默认有建议 ——
const mockFmSuggestion = {
  extensionId: "skill-systematic-debugging",
  tools: ["claude", "codex"],
};
let fmMode: "suggestion" | "empty" = "suggestion";
export function setMockFmMode(mode: "suggestion" | "empty") {
  fmMode = mode;
}

export const tauriInvokeMock = vi.fn((cmd: string, args?: unknown) => {
  switch (cmd) {
    case "pet_list_pets":
      return Promise.resolve([]);
    case "pet_list_codex_pets":
      return Promise.resolve([]);
    case "pet_scan":
      return Promise.resolve({
        id: "x",
        dir: "/home/u/.mam/pets/x",
        spritesheet: { rel: "spritesheet.webp", exists: true, size: 1 },
        voiceFiles: [],
      });
    case "pet_read_manifest":
      return Promise.resolve(null);
    case "get_all_sessions":
      return Promise.resolve(mockSessions);
    case "list_extensions_with_assignments":
      return Promise.resolve(mockExtensions);
    case "list_presets":
      return Promise.resolve(mockPresets);
    // —— 预设组 v2 读命令 fixture（与 src/tauri-mock.ts 同构）——
    case "get_preset":
      return Promise.resolve(
        mockPresets.find((p) => p.id === (args as { presetId?: string })?.presetId) ?? null
      );
    case "get_active_preset":
      return Promise.resolve((args as { toolId?: string })?.toolId === "claude" ? "preset-1" : null);
    case "list_active_presets":
      return Promise.resolve([{ toolId: "claude", presetId: "preset-1" }]);
    // (extensionId, kind, origin) 三元组，origin = "mam" | "native"（scan_tool_state 口径）
    case "get_tool_active_resources":
      return Promise.resolve([
        ["brainstorming", "skill", "mam"],
        ["skill-native-brainstorming", "skill", "native"],
      ]);
    case "list_resource_bindings":
      return Promise.resolve([
        {
          extensionId: "brainstorming",
          exclusiveTools: "claude,codex",
          reason: null,
          updatedAt: new Date().toISOString(),
        },
      ]);
    case "list_tool_residents":
      return Promise.resolve([]);
    // —— 一致性体检读命令（T15，与 src/tauri-mock.ts 形状一致）——
    case "get_preset_health":
      return Promise.resolve(healthMode === "empty" ? mockPresetHealthEmpty : mockPresetHealthIssues);
    case "scan_ledger_drift":
      return Promise.resolve(mockLedgerDrift);
    // 空目录扫描（wave33 Item D）：读 fixture（与 get_preset_health 的 emptyDirs 同源）
    case "scan_empty_dirs":
      return Promise.resolve(mockPresetHealthIssues.emptyDirs);
    case "preview_apply_preset":
      return Promise.resolve({
        toEnable: ["brainstorming"],
        filtered: [],
        toDisable: [],
        toStash: [],
        residentExempt: [],
      });
    // T6 开关 fixture：写命令须返回与真实命令同形的对象（undefined 会让 r.successCount / rr.restoredMam.length 崩）
    case "apply_preset":
      // PresetApplyResult（serde camelCase）：计数与上方 preview_apply_preset 的 toEnable 对齐
      return Promise.resolve({
        successCount: 1,
        failures: [],
        conflicts: [],
        stashed: [],
        disabled: [],
        restoredNative: [],
      });
    case "restore_preset":
      return Promise.resolve({ restoredMam: [], restoredNative: [], conflicts: [] });
    case "kill_session":
      return Promise.resolve();
    case "focus_session":
      // 与 Rust focus_session 返回契约一致（focused | ambiguous）；undefined 会让调用方读 result.type 抛错
      return Promise.resolve({ type: "focused" });
    case "get_setting":
      return Promise.resolve(null);
    case "set_setting":
      return Promise.resolve();
    case "detect_tools":
      return Promise.resolve([]);
    case "detect_subagents":
      return Promise.resolve([]);
    case "list_sub_agents":
      return Promise.resolve([]);
    case "list_repo_skills":
      return Promise.resolve([]);
    case "rescan_skills":
      // 自动导入恒无建议（spec §6：只建议不强制；存量建议走体检卡片）
      return Promise.resolve({
        imported: 0,
        newlyAdded: 0,
        skippedDup: 0,
        sourceCounts: [],
        suggestion: null,
      });
    case "scan_native_resources":
      return Promise.resolve([]);
    // ImportStats + suggestion（Task 17）：手动导入路径带首个命中建议 fixture
    case "import_native_resources":
      return Promise.resolve({
        imported: 0,
        newlyAdded: 0,
        skippedDup: 0,
        sourceCounts: [],
        suggestion: fmMode === "empty" ? null : mockFmSuggestion,
      });
    // ImportOutcome（commands/skill.rs，serde camelCase）：undefined 会让调用方读 .success 抛错
    case "install_skill":
      return Promise.resolve({
        success: true,
        suggestion: fmMode === "empty" ? null : mockFmSuggestion,
      });
    case "list_frontmatter_suggestions":
      return Promise.resolve(fmMode === "empty" ? [] : [mockFmSuggestion]);
    case "list_tool_resources":
      return Promise.resolve({ global: [], native: [] });
    // CompatibilityReport（serde camelCase）：条目为 {id, name, kind} / {id, name, kind, reason}
    case "check_preset_compatibility":
      return Promise.resolve({
        compatible: [{ id: "brainstorming", name: "Brainstorming", kind: "skill" }],
        incompatible: [
          { id: "supabase", name: "Supabase", kind: "mcp", reason: "Not installed for this tool" },
        ],
      });
    case "toggle_mcp_for_tool":
    case "toggle_plugin_for_tool":
    case "write_mcp_server":
    case "remove_mcp_server":
    case "assign_skill_to_subagent":
    case "create_preset":
    case "update_preset":
    case "delete_preset":
    // deactivate_preset 前端路径已退役（T8），case 移除；子 Agent 变体保留
    case "apply_preset_to_subagent":
    case "deactivate_preset_from_subagent":
    case "set_resource_binding":
    case "delete_resource_binding":
    case "set_tool_resident":
    // 暂存回移（T15 体检卡片 ③）：真实命令返回 Result<(), String>，mock 视为成功
    case "restore_stash_entry":
      return Promise.resolve();
    // 空目录清理（wave33 Item D）：真实命令返回 Result<usize>，mock 删无可删返回 0
    case "clean_empty_dirs":
      return Promise.resolve(0);
    // 快捷跳转（wave33 Item B）：homeDir()（@tauri-apps/api/path）mock 下提供
    // 固定 home；reveal_dir 浏览器 mock 语义，静默成功（与 src/tauri-mock.ts 对齐）
    case "plugin:path|resolve_directory":
      return Promise.resolve("/Users/jarvis");
    case "reveal_dir":
      return Promise.resolve(undefined);
    // 体检对账写命令（T15）：须返回与 Rust ReconcileOutcome 同形对象
    //（undefined 会让 o.fixed / o.needsManual 读崩）；extensionId/toolId 结构化回带
    //（终审 Minor #4 起前端批量行映射靠结构化字段，message 只供人读不承载可解析格式）——
    // 与 src/tauri-mock.ts 双 mock 形状一致
    case "reconcile_item": {
      const item = (args as { item?: { extensionId?: string; toolId?: string } } | undefined)?.item;
      return Promise.resolve({
        fixed: true,
        needsManual: false,
        extensionId: item?.extensionId ?? "",
        toolId: item?.toolId ?? "",
        message: "mock: 已按账本重建链接",
      });
    }
    case "reconcile_tool_batch": {
      const batchToolId = (args as { toolId?: string } | undefined)?.toolId ?? "claude";
      return Promise.resolve(
        mockLedgerDrift
          .filter((d) => d.toolId === batchToolId)
          .map((d) => ({
            fixed: true,
            needsManual: false,
            extensionId: d.extensionId,
            toolId: d.toolId,
            message: "mock 已按账本重建链接",
          }))
      );
    }
    // 托盘统一重建（T16）：fire-and-forget，无返回值消费，显式 no-op 以闭合双 mock parity
    case "refresh_tray":
      return Promise.resolve();
    default:
      return Promise.resolve(undefined);
  }
});
