export const POLL_INTERVAL = 3000;
export const EVENT_FRESHNESS_THRESHOLD = 30;
export const SESSION_STATUS = { RUNNING: "running", WAITING: "waiting", IDLE: "idle", COMPLETED: "completed", UNKNOWN: "unknown" } as const;
export const EXTENSION_KIND = { SKILL: "skill", MCP: "mcp", PLUGIN: "plugin" } as const;
export const SUPPORTED_TOOLS = ["claude", "codex", "opencode", "openclaw"] as const;
