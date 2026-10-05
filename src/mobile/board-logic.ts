// 移动看板纯函数层：排序 / 过滤 / 状态配色 / 相对时长 —— 零 React、零 Tauri、零时钟依赖
// （相对时长接收 now 参数，组件侧传 Date.now()，保证可测性）
import type { AgentType, Session, SessionStatus, TransitionEvent } from "@/types/session";

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
// 注：仅测试消费（board-logic.test 穷尽断言），生产代码用 host.enabledTools 驱动
export type ToolFilter = AgentType | "all";

/** 工具显示名（P8c 卡片主行）：八值与桌面 agentBadge.tsx 的 label 口径一致，但**独立定义**
 *  （桌面组件与移动 bundle 隔离，不从桌面 import）。Record<AgentType, string> 穷尽守卫
 *  （对齐 AGENT_TYPE_RECORD 模式）：AgentType 增删值时此处编译报错，主行不会静默缺名。
 *  注意与桌面 getAgentLabel 的差异：桌面 codex 按 form 区分 APP/CLI，移动主行统一 "Codex" */
export const TOOL_LABELS: Record<AgentType, string> = {
  claude: "Claude",
  codex: "Codex",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  kimi: "Kimi Code",
  workbuddy: "WorkBuddy",
  zcode: "ZCode",
  dsh: "DSH",
};

/** 过滤 chips：全部在最前，其后按 AgentType 八值顺序 */
/** 工具品牌色（P8e chips 底色，逐字对齐 Task 3 brief；色值仅作品牌识别，与桌面 ToolIcon 的
 *  图标底色系不同源——brief 拍板的口径优先）。Record<AgentType, string> 穷尽守卫
 *  （对齐 AGENT_TYPE_RECORD 模式）：AgentType 增删值时此处编译报错，chips 不会静默缺色 */
export const TOOL_BRAND_COLORS: Record<AgentType, string> = {
  // 2026-10-05 UI 改版：以配色台账 docs/design/tool-color-ledger.md「颜色矩阵」
  // 的「识别檐色」列为准（官方经典配色重映射，用户逐项裁决）
  claude: "#D97757", // 台账：官方 Crail 橙
  codex: "#10A37F", // 台账：OpenAI 黑白系的唯一点缀绿
  opencode: "#6B6B6B", // 台账：终端灰度美学 → 墨灰檐
  openclaw: "#D2453C", // 台账：龙虾暖系回归 → 猩红（黄 #E5A83B 备选）
  kimi: "#0B0E1A", // 台账：蓝黑底白月牙（不变）
  workbuddy: "#4AD06A", // 台账：猫绿渐变首色（不变）
  zcode: "#141413", // 台账：纯黑底白 Z（蓝紫渐变退役）
  dsh: "#4D6BFE", // 台账：DeepSeek 蓝（不变）
};

// Bug 4（M3 验收）：暗色态 chip 文字色。P8f 的对比度结论做在 P8e 改色之前无人
// 复算——原色在深色卡底 #0f172a 上：kimi 1.08 / claude 2.48 / zcode 3.49 /
// openclaw 4.00 / dsh 4.12 均低于文字线 4.5（claude/kimi 同时低于图形线 3.0）。
// 下表为逐工具暗色文字色（vs #0f172a 实测对比度入注释），保留蓝紫色调、
// 不机械提白成灰；codex(5.41)/opencode(5.01)/workbuddy(8.94) 原色达标沿用
export const TOOL_BRAND_COLORS_DARK: Record<AgentType, string> = {
  // 2026-10-05 台账「夜间皮肤处理规则」：黑系檐色夜间提亮为灰阶（kimi/zcode/
  // opencode），彩色系提亮至夜卡底对比达标；数值与台账矩阵一致
  claude: "#E08A66", // 橙提亮（原紫提亮值 #8B74B9 退役）
  codex: "#10A37F", // 点缀绿夜卡底达标
  opencode: "#8C8C88", // 台账：墨灰 → 中灰
  openclaw: "#E06A5E", // 台账：猩红夜间提亮
  kimi: "#8A94A6", // 台账：蓝黑 → 蓝灰
  workbuddy: "#4AD06A", // 原色达标
  zcode: "#A39C8C", // 台账：纯黑 → 暖灰
  dsh: "#4D6BFE", // 原色达标
};

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

/** chips 活跃排序（P8e）：按各工具最新会话的 lastActivityAt 降序，最近活跃的工具在前。
 *  泛型透传入参元素类型（调用侧传 AgentType[] 时返回保持 AgentType[]，chips 渲染处
 *  无需类型断言；brief 的 string[] 签名调用方式完全兼容）。
 *  入参 sessions 只消费 agentType / lastActivityAt 两个字段（Board 侧直接传全量会话）。
 *  边界口径：无会话工具与不可解析时间戳一律按 0 处理，排有活跃时间的工具之后
 *  （NaN → 0 与 sortSessions 的 -Infinity 语义对齐：坏数据不冒泡到前排）。
 *  不改入参。零时钟依赖：时间全部来自会话自身的时间戳，无 now 注入需求 */
export function sortChipsByActivity<T extends string>(
  tools: T[],
  sessions: { agentType: string; lastActivityAt: string }[]
): T[] {
  // 每工具只保留最新活跃时间：Math.max 逐条折叠
  const latest = new Map<string, number>();
  for (const s of sessions) {
    const t = Date.parse(s.lastActivityAt);
    latest.set(s.agentType, Math.max(latest.get(s.agentType) ?? 0, Number.isNaN(t) ? 0 : t));
  }
  return [...tools].sort((a, b) => (latest.get(b) ?? 0) - (latest.get(a) ?? 0));
}

/** 受管过滤（P8d）：有卡工具 ∩ 受管（enabled）工具。enabled 集合来自
 *  remote_status() 的 enabledTools 字段（后端从 dao::agent_tool 读已启用工具 id 列表），
 *  调用侧把数组转 Set 一次后传入 */
export function filterEnabledTools(tools: string[], enabled: Set<string>): string[] {
  return tools.filter((t) => enabled.has(t));
}

// ---- chip 配色（2026-10-05 中性化）----
// chips 不再使用品牌色文字/实底（旧 P8f 压暗方案随 darkenHex 一并退役）：
// 未选中 = 卡底 + 描边 + 弱文字，选中 = 近黑主控（--btnp/--btnpt），见 Board.tsx。
// 品牌识别色只保留卡片檐用途（TOOL_BRAND_COLORS / TOOL_BRAND_COLORS_DARK）。

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

/** 三色语义（红/黄/绿）——提示音判定用（2026-09-19）。
 *  与 STATUS_DOT_COLOR 同源同口径，只是产出语义名而非 tailwind 类；
 *  **与桌面 hooks/useNotification.statusToColor 逐值一致**（两端提示音口径统一的前提）。
 *  穷尽 Record：状态增删时此处编译报错，不会静默漏判 */
export const STATUS_COLOR_KIND: Record<SessionStatus, "red" | "yellow" | "green"> = {
  waiting: "red",
  processing: "yellow",
  thinking: "yellow",
  compacting: "yellow",
  idle: "green",
  finished: "green",
};

/** 状态中文名（M3 Task 6 跃迁横幅「变化方向」用；文案对齐桌面 i18n
 *  sessionList.statusLabels：waiting=等待操作 / finished=已结束）。
 *  Record<SessionStatus, string> 穷尽守卫：状态增删时此处编译报错，横幅不会静默缺文案 */
/** 跃迁横幅文案（M3 Task 6）：`工具 · 项目 · 前态 → 后态 [· 消息预览]`。
 *  wire 值防御：agentType / from / to 理论上受后端类型约束，但 JSON.parse 结果不可信——
 *  未知值回落原样字符串（横幅仍可读），不得渲染 undefined */
export const STATUS_LABELS: Record<SessionStatus, string> = {
  waiting: "等待操作",
  processing: "运行中",
  thinking: "思考中",
  compacting: "压缩中",
  idle: "空闲",
  finished: "已完成",
};
// 注：横幅文案回归锁（测试断言六值穷尽），防格式化时漏加状态

export function formatTransition(ev: TransitionEvent): string {
  const tool = TOOL_LABELS[ev.agentType] ?? ev.agentType;
  const from = STATUS_LABELS[ev.from] ?? ev.from;
  const to = STATUS_LABELS[ev.to] ?? ev.to;
  const preview = ev.lastMessage ? ` · ${ev.lastMessage}` : "";
  return `${tool} · ${ev.projectName} · ${from} → ${to}${preview}`;
}

/** 把跃迁事件应用到会话列表（M3 Task 6 看板卡随 SSE 实时变化）。
 *  **不是去重**（去重唯一来源在服务端 watcher，铁律 4）——这是「已去重的边沿事件」的
 *  展示层应用：命中 (agentType, id) 时更新该卡的 status/lastMessage（键口径同 watcher
 *  diff 的 (工具, id)：id 只在工具内唯一）。
 *  返回新数组才触发重渲染；未命中（新会话 / 已消失 / 未知状态串）原样返回同一引用，
 *  调用方 setData 走引用相等短路，零多余渲染。不改入参 */
export function applyTransition(sessions: Session[], ev: TransitionEvent): Session[] {
  // 未知状态串（wire 损坏）不写入：否则卡片状态点取色会渲染出 undefined 类名
  if ((STATUS_PRIORITY as Record<string, number | undefined>)[ev.to] === undefined) return sessions;
  const idx = sessions.findIndex((s) => s.agentType === ev.agentType && s.id === ev.sessionId);
  if (idx === -1) return sessions;
  const next = sessions.slice();
  // lastMessage 空值保护（评审 Important 修复）：watcher 的跃迁事件可能不带消息
  // （lastMessage=null）——此时只更新 status，既有预览原样保留（null 抹掉摘要会让
  // 卡片副行凭空消失）；非 null 时正常覆盖
  next[idx] = { ...next[idx], status: ev.to, lastMessage: ev.lastMessage ?? next[idx].lastMessage };
  return next;
}

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
