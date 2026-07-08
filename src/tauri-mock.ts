/**
 * Tauri API Mock for browser/Playwright environment
 *
 * When the app runs outside of Tauri WebView (e.g., in a browser via Vite dev server),
 * the @tauri-apps/api modules throw errors because __TAURI_INTERNALS__ is undefined.
 * This mock provides safe fallbacks so the UI can render for screenshot capture.
 */

// Check if running in Tauri
const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

if (!isTauri) {
  console.log('[tauri-mock] Running outside Tauri WebView — injecting API mocks');

  // Mock __TAURI_INTERNALS__
  (window as any).__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: 'main' },
      currentWebview: { label: 'main' },
    },
    invoke: (cmd: string, args?: any) => {
      console.log(`[tauri-mock] invoke("${cmd}", ${JSON.stringify(args)}) — returning mock data`);
      return mockInvoke(cmd, args);
    },
    convertFileSrc: (path: string) => path,
    transformCallback: (_callback: Function, _once?: boolean) => {
      return Math.random().toString(36).slice(2);
    },
    unregisterCallback: (_id: string) => {},
    postMessage: () => {},
  };

  // Mock invoke responses for different commands
  function mockInvoke(cmd: string, args?: any): Promise<any> {
    switch (cmd) {
      case 'get_all_sessions':
        return Promise.resolve({
          sessions: [
            {
              id: 'mock-claude-1',
              agentType: 'claude',
              projectName: 'MultiAgents-Manager',
              projectPath: '/Users/jarvis/Documents/MultiAgents-Manager',
              title: '实现资源看板重构',
              gitBranch: 'feat/resource-dashboard-redesign',
              githubUrl: 'https://github.com/user/MultiAgents-Manager',
              status: 'processing',
              lastMessage: '继续实现 Layer 2 目录管理...',
              lastMessageRole: 'assistant',
              lastActivityAt: new Date().toISOString(),
              pid: 12345,
              cpuUsage: 2.3,
              activeSubagentCount: 1,
              form: 'cli',
              jumpSupported: true,
            },
            {
              id: 'mock-codex-1',
              agentType: 'codex',
              projectName: 'MultiAgents-Manager',
              projectPath: '/Users/jarvis/Documents/MultiAgents-Manager',
              title: '修复编译错误',
              gitBranch: 'feat/resource-dashboard-redesign',
              githubUrl: null,
              status: 'idle',
              lastMessage: '编译成功，0 errors',
              lastMessageRole: 'assistant',
              lastActivityAt: new Date(Date.now() - 60000).toISOString(),
              pid: 12346,
              cpuUsage: 0.1,
              activeSubagentCount: 0,
              form: 'cli',
              jumpSupported: true,
            },
            {
              id: 'mock-openclaw-1',
              agentType: 'codex',
              projectName: 'OpenClaw',
              projectPath: '/Users/jarvis/.openclaw',
              title: 'SkyComputing Agent',
              gitBranch: 'main',
              githubUrl: null,
              status: 'waiting',
              lastMessage: null,
              lastMessageRole: null,
              lastActivityAt: new Date(Date.now() - 120000).toISOString(),
              pid: 12347,
              cpuUsage: 0.0,
              activeSubagentCount: 0,
              form: 'app',
              jumpSupported: false,
            },
          ],
          totalCount: 3,
          waitingCount: 1,
        });

      case 'list_extensions_with_assignments':
        return Promise.resolve([
          { id: '1', kind: 'skill', name: 'brainstorming', description: '将想法转化为设计', sourcePath: '~/.mam/skills/brainstorming', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }, { agentToolId: 'openclaw', enabled: true, linkStatus: 'linked' }] },
          { id: '2', kind: 'skill', name: 'systematic-debugging', description: '系统化调试', sourcePath: '~/.mam/skills/systematic-debugging', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }] },
          { id: '3', kind: 'skill', name: 'test-driven-development', description: '测试驱动开发', sourcePath: '~/.mam/skills/test-driven-development', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '4', kind: 'skill', name: 'writing-plans', description: '编写实现计划', sourcePath: '~/.mam/skills/writing-plans', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: false, linkStatus: 'unlinked' }] },
          { id: '5', kind: 'skill', name: 'verification-before-completion', description: '完成前验证', sourcePath: '~/.mam/skills/verification-before-completion', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '6', kind: 'skill', name: 'requesting-code-review', description: '请求代码审查', sourcePath: '~/.mam/skills/requesting-code-review', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }] },
          { id: '7', kind: 'skill', name: 'subagent-driven-development', description: '子智能体驱动开发', sourcePath: '~/.mam/skills/subagent-driven-development', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '8', kind: 'skill', name: 'using-superpowers', description: '使用技能系统', sourcePath: '~/.mam/skills/using-superpowers', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }, { agentToolId: 'openclaw', enabled: true, linkStatus: 'linked' }] },
          { id: '9', kind: 'skill', name: 'using-git-worktrees', description: '使用 Git 工作树', sourcePath: '~/.mam/skills/using-git-worktrees', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }] },
          { id: '10', kind: 'skill', name: 'chinese-code-review', description: '中文代码审查', sourcePath: '~/.mam/skills/chinese-code-review', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '11', kind: 'skill', name: 'chinese-commit-conventions', description: '中文提交规范', sourcePath: '~/.mam/skills/chinese-commit-conventions', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '12', kind: 'skill', name: 'mcp-builder', description: '构建 MCP 服务器', sourcePath: '~/.mam/skills/mcp-builder', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: false, linkStatus: 'unlinked' }, { agentToolId: 'codex', enabled: false, linkStatus: 'unlinked' }] },
          { id: '13', kind: 'mcp', name: 'context7', description: 'Library documentation lookup', sourcePath: '~/.mam/mcp/context7', sourceTool: 'claude', suite: null, tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }, { agentToolId: 'codex', enabled: true, linkStatus: 'linked' }] },
          { id: '14', kind: 'mcp', name: 'firecrawl-mcp', description: 'Web search and scraping', sourcePath: '~/.mam/mcp/firecrawl', sourceTool: 'claude', suite: null, tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '15', kind: 'mcp', name: 'playwright', description: 'Browser automation', sourcePath: '~/.mam/mcp/playwright', sourceTool: 'claude', suite: null, tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '16', kind: 'mcp', name: 'supabase', description: 'Database management', sourcePath: '~/.mam/mcp/supabase', sourceTool: 'claude', suite: null, tags: null, assignments: [{ agentToolId: 'claude', enabled: false, linkStatus: 'unlinked' }] },
          { id: '17', kind: 'plugin', name: 'statusline-setup', description: 'Configure status line', sourcePath: '~/.mam/plugins/statusline-setup', sourceTool: 'claude', suite: 'superpowers', tags: null, assignments: [{ agentToolId: 'claude', enabled: true, linkStatus: 'linked' }] },
          { id: '18', kind: 'plugin', name: 'file-secretary', description: 'Personal file management', sourcePath: '~/.mam/plugins/file-secretary', sourceTool: 'claude', suite: null, tags: null, assignments: [{ agentToolId: 'claude', enabled: false, linkStatus: 'unlinked' }] },
        ]);

      case 'list_presets':
        return Promise.resolve([
          {
            id: 'preset-1',
            name: 'Full Stack Dev',
            items: [
              { type: 'skill', name: 'brainstorming', tool_id: 'claude' },
              { type: 'skill', name: 'systematic-debugging', tool_id: 'claude' },
              { type: 'mcp', name: 'context7', tool_id: 'claude' },
              { type: 'mcp', name: 'firecrawl-mcp', tool_id: 'claude' },
            ],
            active_for: ['claude'],
          },
          {
            id: 'preset-2',
            name: 'Code Review',
            items: [
              { type: 'skill', name: 'requesting-code-review', tool_id: 'claude' },
              { type: 'skill', name: 'chinese-code-review', tool_id: 'claude' },
            ],
            active_for: [],
          },
        ]);

      case 'detect_tools':
        return Promise.resolve([
          { id: 'claude', name: 'Claude Code', available: true, path: '/usr/local/bin/claude' },
          { id: 'codex', name: 'Codex CLI', available: true, path: '/usr/local/bin/codex' },
          { id: 'opencode', name: 'OpenCode', available: false, path: '' },
          { id: 'openclaw', name: 'OpenClaw', available: true, path: '/usr/local/bin/openclaw' },
        ]);

      case 'list_sub_agents':
        return Promise.resolve([
          { id: 'sub-1', name: 'frontend-dev', tool_id: 'claude', skills: ['brainstorming', 'test-driven-development'] },
          { id: 'sub-2', name: 'backend-dev', tool_id: 'claude', skills: ['systematic-debugging'] },
        ]);

      case 'get_setting':
        if (args?.key === 'notifications_enabled') return Promise.resolve(true);
        if (args?.key === 'notification_sound') return Promise.resolve('default');
        if (args?.key === 'global_shortcut') return Promise.resolve('Cmd+Shift+M');
        return Promise.resolve(null);

      case 'read_mcp_servers':
        return Promise.resolve([
          { name: 'context7', command: 'npx', args: ['-y', '@upstash/context7-mcp@latest'], tools: ['claude', 'codex'] },
          { name: 'firecrawl-mcp', command: 'npx', args: ['-y', 'firecrawl-mcp'], tools: ['claude'] },
          { name: 'playwright', command: 'npx', args: ['-y', '@playwright/mcp@latest'], tools: ['claude'] },
        ]);

      case 'scan_native_resources':
        return Promise.resolve({
          skills: [
            { name: 'brainstorming', path: '/Users/jarvis/.claude/skills/brainstorming/SKILL.md', tool_id: 'claude' },
            { name: 'systematic-debugging', path: '/Users/jarvis/.claude/skills/systematic-debugging/SKILL.md', tool_id: 'claude' },
          ],
          mcps: [
            { name: 'context7', config_path: '/Users/jarvis/.claude/settings.json', tool_id: 'claude' },
          ],
          plugins: [],
        });

      case 'list_repo_skills':
        return Promise.resolve([
          { name: 'brainstorming', path: '/Users/jarvis/.mam/skills/brainstorming' },
          { name: 'systematic-debugging', path: '/Users/jarvis/.mam/skills/systematic-debugging' },
        ]);

      case 'check_preset_compatibility':
        return Promise.resolve({
          compatible: [
            { type: 'skill', name: 'brainstorming', reason: 'Available in tool scope' },
          ],
          incompatible: [
            { type: 'mcp', name: 'supabase', reason: 'Not installed for this tool' },
          ],
        });

      case 'capture_window_screenshot':
        return Promise.resolve({ success: true, path: '/tmp/mock-screenshot.png' });

      case 'list_screenshots':
        return Promise.resolve([]);

      // Event/Notification plugin commands — return safe mock values
      case 'plugin:event|listen':
      case 'plugin:event|once':
      case 'plugin:notification|register_listener':
      case 'plugin:notification|register_action_types':
        return Promise.resolve(() => {});  // unlisten function

      case 'plugin:window|is_maximized':
        return Promise.resolve(false);

      case 'plugin:window|show':
        return Promise.resolve(undefined);

      case 'plugin:updater|check':
        return Promise.resolve(null);

      case 'plugin:notification|is_permission_granted':
        return Promise.resolve(false);

      case 'update_tray_menu':
        return Promise.resolve(undefined);

      // Default: return empty success
      default:
        console.log(`[tauri-mock] Unhandled command: ${cmd}`);
        return Promise.resolve(null);
    }
  }
}

export {};
