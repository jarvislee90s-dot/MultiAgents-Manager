export const POLL_INTERVAL = 3000;
export const EVENT_FRESHNESS_THRESHOLD = 30;
export const SESSION_STATUS = {
  RUNNING: "running",
  WAITING: "waiting",
  IDLE: "idle",
  COMPLETED: "completed",
  UNKNOWN: "unknown",
} as const;
export const EXTENSION_KIND = { SKILL: "skill", MCP: "mcp", PLUGIN: "plugin" } as const;
// 债修复（ZCode 接入轮）：原清单缺 workbuddy（agentBadge 遍历测试的覆盖面漏了
// 第六工具）；与后端 TOOL_IDS 对齐，新工具接入须同步此清单
export const SUPPORTED_TOOLS = [
  "claude",
  "codex",
  "opencode",
  "openclaw",
  "kimi",
  "workbuddy",
  "zcode",
] as const;
