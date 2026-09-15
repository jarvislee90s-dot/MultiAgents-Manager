// 移动看板纯函数层：排序 / 过滤 / 状态配色 / 相对时长 —— 零 React、零 Tauri、零时钟依赖
// （相对时长接收 now 参数，组件侧传 Date.now()，保证可测性）
import type { AgentType, Session, SessionStatus } from "@/types/session";

// 穷尽守卫（类型级，以 src/types/session.ts 的 AgentType 为源）：
// Record 键集必须与 AgentType 完全一致——未来 AgentType 增/删第 9 值时，
// 此处缺失或多余的键会直接编译报错，chips 不会静默缺失。
// 字面量键序即 chips 展示顺序（Object.values 对字符串键保证按插入序返回）。
const AGENT_TYPE_RECORD: Record<AgentType, AgentType> = {
  claude: "claude",
  codex: "codex",
  opencode: "opencode",
  openclaw: "openclaw",
  kimi: "kimi",
  workbuddy: "workbuddy",
  zcode: "zcode",
  dsh: "dsh",
};

/** 八工具 chips 顺序（键集穷尽自 AgentType，顺序与其在 session.ts 的声明顺序一致） */
export const AGENT_TYPES: readonly AgentType[] = Object.values(AGENT_TYPE_RECORD);

export type ToolFilter = AgentType | "all";

/** 过滤 chips：全部在最前，其后按 AgentType 八值顺序 */
export const TOOL_FILTERS: readonly ToolFilter[] = ["all", ...AGENT_TYPES] as const;

// 排序优先级：等待(0) → 运行(1) → 空闲(2)，数字越小越靠前
// 移动看板口径：等待人工介入的卡置顶（桌面 Rust status_sort_priority 为运行优先，
// 两端口径刻意不同——移动端强调"哪些卡需要我处理"）
const STATUS_PRIORITY: Record<SessionStatus, number> = {
  waiting: 0,
  processing: 1,
  thinking: 1,
  compacting: 1,
  idle: 2,
  finished: 2,
};

/** 等待 → 运行 → 空闲；同级内按 lastActivityAt 降序（最近活跃在前）。不改入参。 */
export function sortSessions(sessions: Session[]): Session[] {
  return [...sessions].sort((a, b) => {
    const byPriority = STATUS_PRIORITY[a.status] - STATUS_PRIORITY[b.status];
    if (byPriority !== 0) return byPriority;
    // Date.parse 比较（比字符串比较更能容忍时区偏移差异）；不可解析的时间戳排同级末尾
    const ta = Date.parse(a.lastActivityAt);
    const tb = Date.parse(b.lastActivityAt);
    return (Number.isNaN(tb) ? -Infinity : tb) - (Number.isNaN(ta) ? -Infinity : ta);
  });
}

/** 按 agentType 过滤；"all" 原样返回（不过滤） */
export function filterByAgent(sessions: Session[], filter: ToolFilter): Session[] {
  if (filter === "all") return sessions;
  return sessions.filter((s) => s.agentType === filter);
}

// 状态 → 圆点颜色，三色语义与桌面 StatusLight 一致：
// 红=待处理（waiting）/ 黄=正在运行（processing/thinking/compacting）/ 绿=已完成（idle/finished）
export const STATUS_DOT_COLOR: Record<SessionStatus, string> = {
  waiting: "bg-red-500",
  processing: "bg-yellow-500",
  thinking: "bg-yellow-500",
  compacting: "bg-yellow-500",
  idle: "bg-green-500",
  finished: "bg-green-500",
};

/** lastActivityAt 距 now 的相对时长（中文文案，移动页 i18n 随 M3 完善） */
export function formatRelativeTime(lastActivityAt: string, now: number): string {
  const t = Date.parse(lastActivityAt);
  if (Number.isNaN(t)) return "--";
  const diff = now - t;
  // 未来时间戳（设备时钟偏差）按刚发生处理
  if (diff < 60_000) return "刚刚";
  const mins = Math.floor(diff / 60_000);
  if (mins < 60) return `${mins} 分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} 小时前`;
  return `${Math.floor(hours / 24)} 天前`;
}
