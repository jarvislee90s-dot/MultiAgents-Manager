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

export const tauriInvokeMock = vi.fn((cmd: string, _args?: unknown) => {
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
        mockPresets.find((p) => p.id === (_args as { presetId?: string })?.presetId) ?? null
      );
    case "get_active_preset":
      return Promise.resolve((_args as { toolId?: string })?.toolId === "claude" ? "preset-1" : null);
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
      return Promise.resolve({ imported: 0, newlyAdded: 0, skippedDup: 0, sourceCounts: [] });
    case "scan_native_resources":
      return Promise.resolve([]);
    case "import_native_resources":
      return Promise.resolve({ imported: 0, newlyAdded: 0, skippedDup: 0, sourceCounts: [] });
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
    case "install_skill":
    case "assign_skill_to_subagent":
    case "create_preset":
    case "update_preset":
    case "delete_preset":
    case "deactivate_preset":
    case "apply_preset_to_subagent":
    case "deactivate_preset_from_subagent":
    case "set_resource_binding":
    case "delete_resource_binding":
    case "set_tool_resident":
      return Promise.resolve();
    default:
      return Promise.resolve(undefined);
  }
});
