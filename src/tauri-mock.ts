/**
 * Tauri API Mock for browser/Playwright environment
 *
 * When the app runs outside of Tauri WebView (e.g., in a browser via Vite dev server),
 * the @tauri-apps/api modules throw errors because __TAURI_INTERNALS__ is undefined.
 * This mock provides safe fallbacks so the UI can render for screenshot capture.
 */

// Check if running in Tauri
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

interface TauriInternalsMock {
  metadata: {
    currentWindow: { label: string };
    currentWebview: { label: string };
  };
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  convertFileSrc: (path: string) => string;
  transformCallback: (callback: (payload: unknown) => void, once?: boolean) => string;
  unregisterCallback: (id: string) => void;
  postMessage: () => void;
}

if (!isTauri) {
  console.log("[tauri-mock] Running outside Tauri WebView — injecting API mocks");

  // 预设组 v2 fixture（1 通用 + 1 tool 私有；extensionId 对应上方资源样例的 id）
  const mockPresetsV2 = [
    {
      id: "preset-1",
      name: "Full Stack Dev",
      description: "全栈开发常用组合：需求梳理 + 系统化调试 + 文档检索",
      scope: "universal",
      boundTool: null,
      items: [
        { extensionId: "1", kind: "skill", extensionName: "brainstorming" },
        { extensionId: "2", kind: "skill", extensionName: "systematic-debugging" },
        { extensionId: "13", kind: "mcp", extensionName: "context7" },
        { extensionId: "14", kind: "mcp", extensionName: "firecrawl-mcp" },
      ],
    },
    {
      id: "preset-2",
      name: "Code Review",
      description: "Claude 专属中文审查流（工具私有预设样例）",
      scope: "tool",
      boundTool: "claude",
      items: [
        { extensionId: "10", kind: "skill", extensionName: "chinese-code-review" },
        { extensionId: "11", kind: "skill", extensionName: "chinese-commit-conventions" },
      ],
    },
  ];

  // Mock __TAURI_INTERNALS__
  (window as unknown as { __TAURI_INTERNALS__: TauriInternalsMock }).__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
    invoke: (cmd: string, args?: Record<string, unknown>) => {
      console.log(`[tauri-mock] invoke("${cmd}", ${JSON.stringify(args)}) — returning mock data`);
      return mockInvoke(cmd, args);
    },
    convertFileSrc: (path: string) => path,
    transformCallback: (_callback, _once) => {
      return Math.random().toString(36).slice(2);
    },
    unregisterCallback: (_id) => {},
    postMessage: () => {},
  };

  // Mock invoke responses for different commands
  function mockInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
    switch (cmd) {
      // P2-2：settings 页窗口关闭拦截改用 getCurrentWindow().onCloseRequested。
      // 该 API 依赖上方 __TAURI_INTERNALS__ 的 metadata/transformCallback/postMessage
      //（本 mock 已提供），浏览器渲染下注册为 no-op 监听、永不触发，页面不崩溃——
      // 无需额外 mock 模块；真实关闭拦截仅在 Tauri WebView 内生效
      case "list_enabled_tools":
        return Promise.resolve([]);
      case "get_all_sessions":
        return Promise.resolve({
          sessions: [
            {
              id: "mock-claude-1",
              agentType: "claude",
              projectName: "MultiAgents-Manager",
              projectPath: "/Users/jarvis/Documents/MultiAgents-Manager",
              title: "实现资源看板重构",
              gitBranch: "feat/resource-dashboard-redesign",
              githubUrl: "https://github.com/user/MultiAgents-Manager",
              status: "processing",
              lastMessage: "继续实现 Layer 2 目录管理...",
              lastMessageRole: "assistant",
              lastActivityAt: new Date().toISOString(),
              pid: 12345,
              cpuUsage: 2.3,
              activeSubagentCount: 1,
              form: "cli",
              jumpSupported: true,
            },
            {
              id: "mock-codex-1",
              agentType: "codex",
              projectName: "MultiAgents-Manager",
              projectPath: "/Users/jarvis/Documents/MultiAgents-Manager",
              title: "修复编译错误",
              gitBranch: "feat/resource-dashboard-redesign",
              githubUrl: null,
              status: "idle",
              lastMessage: "编译成功，0 errors",
              lastMessageRole: "assistant",
              lastActivityAt: new Date(Date.now() - 60000).toISOString(),
              pid: 12346,
              cpuUsage: 0.1,
              activeSubagentCount: 0,
              form: "cli",
              jumpSupported: true,
            },
            {
              id: "mock-openclaw-1",
              agentType: "codex",
              projectName: "OpenClaw",
              projectPath: "/Users/jarvis/.openclaw",
              title: "SkyComputing Agent",
              gitBranch: "main",
              githubUrl: null,
              status: "waiting",
              lastMessage: null,
              lastMessageRole: null,
              lastActivityAt: new Date(Date.now() - 120000).toISOString(),
              pid: 12347,
              cpuUsage: 0.0,
              activeSubagentCount: 0,
              form: "app",
              jumpSupported: false,
            },
          ],
          totalCount: 3,
          waitingCount: 1,
        });

      case "list_extensions_with_assignments":
        return Promise.resolve([
          {
            id: "1",
            kind: "skill",
            name: "brainstorming",
            description: "将想法转化为设计",
            sourcePath: "~/.mam/skills/brainstorming",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
              { agentToolId: "openclaw", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "2",
            kind: "skill",
            name: "systematic-debugging",
            description: "系统化调试",
            sourcePath: "~/.mam/skills/systematic-debugging",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "3",
            kind: "skill",
            name: "test-driven-development",
            description: "测试驱动开发",
            sourcePath: "~/.mam/skills/test-driven-development",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "4",
            kind: "skill",
            name: "writing-plans",
            description: "编写实现计划",
            sourcePath: "~/.mam/skills/writing-plans",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: false, linkStatus: "unlinked" }],
          },
          {
            id: "5",
            kind: "skill",
            name: "verification-before-completion",
            description: "完成前验证",
            sourcePath: "~/.mam/skills/verification-before-completion",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "6",
            kind: "skill",
            name: "requesting-code-review",
            description: "请求代码审查",
            sourcePath: "~/.mam/skills/requesting-code-review",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "7",
            kind: "skill",
            name: "subagent-driven-development",
            description: "子智能体驱动开发",
            sourcePath: "~/.mam/skills/subagent-driven-development",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "8",
            kind: "skill",
            name: "using-superpowers",
            description: "使用技能系统",
            sourcePath: "~/.mam/skills/using-superpowers",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
              { agentToolId: "openclaw", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "9",
            kind: "skill",
            name: "using-git-worktrees",
            description: "使用 Git 工作树",
            sourcePath: "~/.mam/skills/using-git-worktrees",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "10",
            kind: "skill",
            name: "chinese-code-review",
            description: "中文代码审查",
            sourcePath: "~/.mam/skills/chinese-code-review",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "11",
            kind: "skill",
            name: "chinese-commit-conventions",
            description: "中文提交规范",
            sourcePath: "~/.mam/skills/chinese-commit-conventions",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "12",
            kind: "skill",
            name: "mcp-builder",
            description: "构建 MCP 服务器",
            sourcePath: "~/.mam/skills/mcp-builder",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: false, linkStatus: "unlinked" },
              { agentToolId: "codex", enabled: false, linkStatus: "unlinked" },
            ],
          },
          {
            id: "13",
            kind: "mcp",
            name: "context7",
            description: "Library documentation lookup",
            sourcePath: "~/.mam/mcp/context7",
            sourceTool: "claude",
            suite: null,
            tags: null,
            assignments: [
              { agentToolId: "claude", enabled: true, linkStatus: "linked" },
              { agentToolId: "codex", enabled: true, linkStatus: "linked" },
            ],
          },
          {
            id: "14",
            kind: "mcp",
            name: "firecrawl-mcp",
            description: "Web search and scraping",
            sourcePath: "~/.mam/mcp/firecrawl",
            sourceTool: "claude",
            suite: null,
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "15",
            kind: "mcp",
            name: "playwright",
            description: "Browser automation",
            sourcePath: "~/.mam/mcp/playwright",
            sourceTool: "claude",
            suite: null,
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "16",
            kind: "mcp",
            name: "supabase",
            description: "Database management",
            sourcePath: "~/.mam/mcp/supabase",
            sourceTool: "claude",
            suite: null,
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: false, linkStatus: "unlinked" }],
          },
          {
            id: "17",
            kind: "plugin",
            name: "statusline-setup",
            description: "Configure status line",
            sourcePath: "~/.mam/plugins/statusline-setup",
            sourceTool: "claude",
            suite: "superpowers",
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: true, linkStatus: "linked" }],
          },
          {
            id: "18",
            kind: "plugin",
            name: "file-secretary",
            description: "Personal file management",
            sourcePath: "~/.mam/plugins/file-secretary",
            sourceTool: "claude",
            suite: null,
            tags: null,
            assignments: [{ agentToolId: "claude", enabled: false, linkStatus: "unlinked" }],
          },
        ]);

      // 预设组 v2 样例（1 通用 + 1 tool 私有）：与 Rust PresetRecord（serde camelCase）同构，
      // 并与 tests/msw/tauriMocks.ts 的 mockPresets 保持形状一致（mock parity 收口门禁）
      case "list_presets":
        return Promise.resolve(mockPresetsV2);

      case "get_preset":
        return Promise.resolve(
          mockPresetsV2.find((p) => p.id === (args?.presetId as string)) ?? null
        );

      case "get_active_preset":
        return Promise.resolve(args?.toolId === "claude" ? "preset-1" : null);

      case "list_active_presets":
        return Promise.resolve([{ toolId: "claude", presetId: "preset-1" }]);

      // (extensionId, kind, origin) 三元组，origin = "mam" | "native"（scan_tool_state 口径）
      case "get_tool_active_resources":
        return Promise.resolve([
          ["1", "skill", "mam"],
          ["2", "skill", "mam"],
          ["13", "mcp", "mam"],
          ["skill-brainstorming", "skill", "native"],
        ]);

      case "list_resource_bindings":
        return Promise.resolve([
          {
            extensionId: "13",
            exclusiveTools: "claude,codex",
            reason: "需要 Node 运行时，仅绑定前端工具链",
            updatedAt: new Date().toISOString(),
          },
        ]);

      case "list_tool_residents":
        return Promise.resolve(args?.toolId === "claude" ? ["17"] : []);

      case "preview_apply_preset":
        return Promise.resolve({
          toEnable: ["1", "2"],
          filtered: [],
          toDisable: [],
          toStash: [],
          residentExempt: [],
        });

      // 预设组写命令（浏览器 mock 一律视为成功；与 tests/msw/tauriMocks.ts 写分组对齐）
      case "create_preset":
      case "update_preset":
      case "delete_preset":
      case "apply_preset":
      case "deactivate_preset":
      case "restore_preset":
      case "apply_preset_to_subagent":
      case "deactivate_preset_from_subagent":
      case "set_resource_binding":
      case "delete_resource_binding":
      case "set_tool_resident":
        return Promise.resolve(undefined);

      case "detect_tools":
        return Promise.resolve([
          { id: "claude", name: "Claude Code", available: true, path: "/usr/local/bin/claude" },
          { id: "codex", name: "Codex CLI", available: true, path: "/usr/local/bin/codex" },
          { id: "opencode", name: "OpenCode", available: false, path: "" },
          { id: "openclaw", name: "OpenClaw", available: true, path: "/usr/local/bin/openclaw" },
          { id: "kimi", name: "Kimi Code", available: false, path: "" },
          // 债修复（ZCode 接入轮）：原 mock 缺 workbuddy 行（与后端 6 工具不对齐）
          { id: "workbuddy", name: "WorkBuddy", available: true, path: "" },
          { id: "zcode", name: "ZCode", available: true, path: "" },
        ]);

      case "list_sub_agents":
        return Promise.resolve([
          {
            id: "sub-1",
            name: "frontend-dev",
            tool_id: "claude",
            skills: ["brainstorming", "test-driven-development"],
          },
          { id: "sub-2", name: "backend-dev", tool_id: "claude", skills: ["systematic-debugging"] },
        ]);

      case "get_setting":
        if (args?.key === "notifications_enabled") return Promise.resolve(true);
        if (args?.key === "notification_sound") return Promise.resolve("default");
        if (args?.key === "global_shortcut") return Promise.resolve("Cmd+Shift+M");
        if (args?.key === "ui_theme") return Promise.resolve(null);
        return Promise.resolve(null);

      case "set_theme":
        return Promise.resolve(undefined);

      case "read_mcp_servers":
        return Promise.resolve([
          {
            name: "context7",
            command: "npx",
            args: ["-y", "@upstash/context7-mcp@latest"],
            tools: ["claude", "codex"],
          },
          {
            name: "firecrawl-mcp",
            command: "npx",
            args: ["-y", "firecrawl-mcp"],
            tools: ["claude"],
          },
          {
            name: "playwright",
            command: "npx",
            args: ["-y", "@playwright/mcp@latest"],
            tools: ["claude"],
          },
        ]);

      // 真实命令返回 NativeExtensionRecord 数组（serde camelCase，resource.rs）
      case "scan_native_resources":
        return Promise.resolve([
          {
            id: "skill-brainstorming",
            kind: "skill",
            name: "brainstorming",
            sourcePath: "/Users/jarvis/.claude/skills/brainstorming",
            sourceTool: "claude",
            description: null,
            detectedAt: new Date().toISOString(),
            imported: false,
          },
          {
            id: "skill-systematic-debugging",
            kind: "skill",
            name: "systematic-debugging",
            sourcePath: "/Users/jarvis/.claude/skills/systematic-debugging",
            sourceTool: "claude",
            description: null,
            detectedAt: new Date().toISOString(),
            imported: false,
          },
        ]);

      // ApplyResult 形状（tool_settings.rs，camelCase）；null 会让设置页
      // result.rebuildFailed.length 抛 TypeError（issue #36 review Minor）
      case "update_tool_settings":
        return Promise.resolve({
          restored: [],
          restoredMcps: [],
          rebuildFailed: [],
          skippedKept: [],
          skippedLost: [],
        });

      // 工具管理行（ToolSetting，camelCase）：七工具全量行，浏览器渲染下
      // 设置页「工具管理」分区可出数据
      case "get_tool_settings":
        return Promise.resolve([
          { toolId: "claude", name: "Claude Code", enabled: true, installed: true, managed: true },
          { toolId: "codex", name: "Codex CLI", enabled: true, installed: true, managed: true },
          { toolId: "workbuddy", name: "WorkBuddy", enabled: true, installed: true, managed: true },
          { toolId: "kimi", name: "Kimi Code", enabled: true, installed: true, managed: true },
          { toolId: "opencode", name: "OpenCode", enabled: true, installed: true, managed: false },
          { toolId: "openclaw", name: "OpenClaw", enabled: true, installed: false, managed: false },
          { toolId: "zcode", name: "ZCode", enabled: true, installed: true, managed: false },
        ]);

      case "list_repo_skills":
        return Promise.resolve([
          { name: "brainstorming", path: "/Users/jarvis/.mam/skills/brainstorming" },
          { name: "systematic-debugging", path: "/Users/jarvis/.mam/skills/systematic-debugging" },
        ]);

      // CompatibilityReport（serde camelCase）：条目为 {id, name, kind} / {id, name, kind, reason}
      case "check_preset_compatibility":
        return Promise.resolve({
          compatible: [
            { id: "1", name: "brainstorming", kind: "skill" },
            { id: "13", name: "context7", kind: "mcp" },
          ],
          incompatible: [
            { id: "16", name: "supabase", kind: "mcp", reason: "Not installed for this tool" },
          ],
        });

      // 遗留 codex 技能链接检测/迁移（spec §4.3）：浏览器模式视为无遗留、零报告
      case "detect_legacy_agents_links":
        return Promise.resolve([]);
      case "migrate_legacy_agents_links":
        return Promise.resolve([]);

      case "capture_window_screenshot":
        return Promise.resolve({ success: true, path: "/tmp/mock-screenshot.png" });

      case "list_screenshots":
        return Promise.resolve([]);

      // Event/Notification plugin commands — return safe mock values
      case "plugin:event|listen":
      case "plugin:event|once":
      case "plugin:notification|register_listener":
      case "plugin:notification|register_action_types":
        return Promise.resolve(() => {}); // unlisten function

      case "plugin:window|is_maximized":
        return Promise.resolve(false);

      case "plugin:window|show":
        return Promise.resolve(undefined);

      case "plugin:updater|check":
        return Promise.resolve(null);

      case "plugin:notification|is_permission_granted":
        return Promise.resolve(false);

      case "update_tray_menu":
        return Promise.resolve(undefined);

      // Default: return empty success
      default:
        console.log(`[tauri-mock] Unhandled command: ${cmd}`);
        return Promise.resolve(null);
    }
  }
}

export {};
