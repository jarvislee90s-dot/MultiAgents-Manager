// 消息窗口书签（M3+，2026-09-16 用户裁决）——纯函数 + 模块级内存单例。
//
// 用途：长会话里用户上下反复翻找，把「待会要回来看」的位置打上书签（≤10 个、
// 每色一个），点书签即跳回。
//
// 两条硬约束决定了本模块形态：
// 1. **锚点用内容指纹而非 seq**：后端 finalize 把 seq 赋为「返回数组内下标」，
//    点「加载更早消息」（limit 200→400）或刷新后同一条消息 seq 整体位移，
//    按 seq 存会指向错行。指纹 = kind|ts|长度|前 120 字符，跨重拉稳定。
// 2. **状态放模块级单例而非组件 state**：SessionDetail 是条件挂载（App.tsx），
//    返回看板即卸载、state 全丢；而用户要求「出会话窗口再切回来保留」。
//    单例随 JS 上下文存活 → 关 MAM 自然消失（正是用户要的生命周期）。
//
// 与 theme.ts 的 localStorage 惯例刻意不同：书签**不落盘**（用户明确要求
// 关掉即失效），故无 try/catch 降级分支。

/** 调色板（10 色，颜色即书签唯一键——每色至多一个） */
export const BOOKMARK_COLORS: readonly string[] = [
  "#ef4444", // 红
  "#f59e0b", // 橙
  "#eab308", // 黄
  "#22c55e", // 绿
  "#14b8a6", // 青
  "#3b82f6", // 蓝
  "#6366f1", // 靛
  "#a855f7", // 紫
  "#ec4899", // 粉
  "#78716c", // 灰
];

/** 上限 = 调色板色数（颜色唯一 + 10 色 = 天然上限 10，用户裁决） */
export const BOOKMARK_LIMIT = BOOKMARK_COLORS.length;

/** 单条书签 */
export interface Bookmark {
  /** 调色板色值（唯一键：每色至多一个） */
  color: string;
  /** 打标时的 seq——仅作冗余提示，**不作锚**（seq 会随 limit 位移） */
  seq: number;
  /** 主锚：内容指纹（见 messageAnchor） */
  anchor: string;
  /** 消息摘要（tooltip / 管理态回显） */
  preview: string;
}

/** 内容指纹（跨 limit 重拉 / 刷新稳定）：kind + ts + 长度 + 前 120 字符。
 *  ts null 占位空串（不产出 "undefined"）。已知边界：同会话内两条
 *  kind/ts/长度/前缀全同的消息会命中同指纹（概率极低，跳转到第一条）。 */
export function messageAnchor(m: { kind: string; ts: number | null; content: string }): string {
  return `${m.kind}|${m.ts ?? ""}|${m.content.length}|${m.content.slice(0, 120)}`;
}

/** 消息摘要（书签 preview / 管理态标签）：单行化 + 截断 */
export function bookmarkPreview(content: string, max = 40): string {
  const oneLine = content.replace(/\s+/g, " ").trim();
  return oneLine.length > max ? `${oneLine.slice(0, max)}…` : oneLine;
}

/** sessionId → 书签表（模块级单例；随 JS 上下文存活，关 MAM 消失） */
const store = new Map<string, Bookmark[]>();

/** 该会话的书签表（副本——外部改写不污染单例） */
export function listBookmarks(sessionId: string): Bookmark[] {
  return [...(store.get(sessionId) ?? [])];
}

/** 加书签：颜色已占用 或 已达上限 → 拒绝（no-op）。
 *  两种情况都不可静默替换语义（用户裁决：颜色即唯一键、硬上限 10） */
export function addBookmark(sessionId: string, b: Bookmark): void {
  const list = store.get(sessionId) ?? [];
  if (list.length >= BOOKMARK_LIMIT) return;
  if (list.some((x) => x.color === b.color)) return;
  store.set(sessionId, [...list, b]);
}

/** 删单条（按颜色）：不存在则 no-op */
export function removeBookmark(sessionId: string, color: string): void {
  const list = store.get(sessionId);
  if (!list) return;
  store.set(
    sessionId,
    list.filter((x) => x.color !== color)
  );
}

/** 清空该会话全部书签（不影响其它会话） */
export function clearBookmarks(sessionId: string): void {
  store.delete(sessionId);
}
