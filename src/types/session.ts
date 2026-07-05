// 会话类型定义 — 与 Rust Session 结构（camelCase 序列化）对应

export type AgentType = "claude" | "codex" | "opencode";

export type SessionStatus =
  | "waiting"
  | "processing"
  | "thinking"
  | "compacting"
  | "idle"
  | "finished";

export type ProcessForm = "cli" | "app";

export interface Session {
  id: string;
  agentType: AgentType;
  projectName: string;
  projectPath: string;
  title: string | null;
  gitBranch: string | null;
  githubUrl: string | null;
  status: SessionStatus;
  lastMessage: string | null;
  lastMessageRole: string | null;
  lastActivityAt: string;
  pid: number;
  cpuUsage: number;
  activeSubagentCount: number;
  form: ProcessForm;
  jumpSupported: boolean;
}

export interface SessionsResponse {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
}
