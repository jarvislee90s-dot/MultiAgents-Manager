// 会话类型定义 — 与 Rust Session 结构（camelCase 序列化）对应

export type AgentType =
  "claude" | "codex" | "opencode" | "openclaw" | "kimi" | "workbuddy" | "zcode" | "dsh";

export type SessionStatus =
  "waiting" | "processing" | "thinking" | "compacting" | "idle" | "finished";

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
  /** 未读标记（W4）：绿色已完成且用户未查看的持久未读卡（APP 类专用） */
  unread: boolean;
}

export interface SessionsResponse {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
}

/** 状态跃迁事件（SSE `transition` 帧的 data 载荷，M3 Task 6）— 与 Rust
 * `remote::watcher::TransitionEvent`（camelCase 序列化）逐字段对应。
 * 语义：**只含边沿**（watcher 是唯一去重点，铁律 4），消费者收到一条即处理一条，
 * 不得在客户端再做 diff/去重。key 口径同后端 diff：id 只在工具内唯一，
 * 跨工具可撞 id，故唯一键必须取 (agentType, sessionId) 二元组 */
export interface TransitionEvent {
  sessionId: string;
  agentType: AgentType;
  /** 变化前的状态（wire 字符串，与 Session.status 同一联合） */
  from: SessionStatus;
  /** 变化后的状态 */
  to: SessionStatus;
  projectName: string;
  /** 变化时刻该会话的最新消息预览（可为 null：无消息） */
  lastMessage: string | null;
  /** 事件产生时刻（服务端毫秒时间戳） */
  ts: number;
}
