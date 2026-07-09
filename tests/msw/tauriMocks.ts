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

export const mockPresets = [
  { id: "preset-1", name: "前端开发", items: [["brainstorming", "skill"]] },
];

export const tauriInvokeMock = vi.fn((cmd: string, _args?: unknown) => {
  switch (cmd) {
    case "get_all_sessions":
      return Promise.resolve(mockSessions);
    case "list_extensions_with_assignments":
      return Promise.resolve(mockExtensions);
    case "list_presets":
      return Promise.resolve(mockPresets);
    case "focus_session":
    case "kill_session":
      return Promise.resolve();
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
    case "check_preset_compatibility":
      return Promise.resolve({ compatible: [], incompatible: [] });
    case "toggle_mcp_for_tool":
    case "toggle_plugin_for_tool":
    case "write_mcp_server":
    case "remove_mcp_server":
    case "install_skill":
    case "assign_skill_to_subagent":
    case "create_preset":
    case "delete_preset":
    case "apply_preset":
    case "deactivate_preset":
    case "apply_preset_to_subagent":
    case "deactivate_preset_from_subagent":
      return Promise.resolve();
    default:
      return Promise.resolve(undefined);
  }
});
