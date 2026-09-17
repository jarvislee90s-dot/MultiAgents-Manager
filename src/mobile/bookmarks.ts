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
//    单例随 JS 上下文存活 → 切换视图自然保留。
// 3. **刷新恢复（v1.1 补充）**：手机刷新页面会销毁 JS 上下文（内存单例清空），
//    故镜像到 localStorage 并打上 bootId——同进程刷新恢复、MAM 重启清空。
//
// （v1.1 修订：不再「纯内存」——用户实测刷新网页即丢，补充 localStorage
// 镜像 + bootId 守卫，语义仍是「随 MAM 消失」，只是把「消失」的判定从
// JS 上下文销毁放宽到「MAM 进程重启」。）

import { fetchHost } from "./api";

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

/** sessionId → 书签表（模块级单例；跨组件卸载/重挂载保留） */
const store = new Map<string, Bookmark[]>();

// ---- 刷新恢复（2026-09-16 用户裁决补充）：手机刷新页面会销毁 JS 上下文，
// 内存单例随之清空——书签需镜像到 localStorage 并打上 bootId（MAM 进程
// 生命周期标识，随 /host 下发）：同 bootId（同一 MAM 进程）→ 刷新后恢复；
// bootId 变化（MAM 已重启）→ 自行清空。仍是「随 MAM 消失」，且更耐用。
// 关浏览器标签页也会清（localStorage 在该域下的这一键被覆盖前一直存在，
// 但恢复依赖 bootId 匹配——MAM 重启后即使键还在也会被判空）。

const BOOKMARKS_STORAGE_KEY = "mam-bookmarks";
let activeBootId: string | null = null;

interface StoredBookmarks {
  bootId: string;
  sessions: Record<string, Bookmark[]>;
}

function readStorage(): StoredBookmarks {
  try {
    const raw = window.localStorage.getItem(BOOKMARKS_STORAGE_KEY);
    if (!raw) return { bootId: "", sessions: {} };
    const parsed = JSON.parse(raw) as StoredBookmarks;
    if (
      typeof parsed.bootId !== "string" ||
      typeof parsed.sessions !== "object" ||
      parsed.sessions === null
    ) {
      return { bootId: "", sessions: {} };
    }
    return parsed;
  } catch {
    return { bootId: "", sessions: {} }; // 隐私模式 / JSON 损坏：降级为无持久层
  }
}

function writeStorage(stored: StoredBookmarks): void {
  try {
    window.localStorage.setItem(BOOKMARKS_STORAGE_KEY, JSON.stringify(stored));
  } catch {
    /* 写失败（隐私模式/超限）：静默降级为纯内存态 */
  }
}

/** 设定当前 MAM 进程的 bootId 并恢复书签：bootId 与存储不一致（MAM 已重启）
 *  → 清空存储；一致 → 把存储的书签种回内存单例。幂等，可重复调用 */
export function restoreBookmarks(bootId: string): void {
  activeBootId = bootId;
  const stored = readStorage();
  if (stored.bootId !== bootId) {
    store.clear();
    writeStorage({ bootId, sessions: {} });
    return;
  }
  for (const [sid, list] of Object.entries(stored.sessions)) {
    if (Array.isArray(list)) store.set(sid, list);
  }
}

/** 内存单例 → localStorage（增删清空后调用；bootId 未知时跳过——恢复未完成） */
function persist(): void {
  if (activeBootId === null) return;
  const sessions: Record<string, Bookmark[]> = {};
  for (const [sid, list] of store) sessions[sid] = list;
  writeStorage({ bootId: activeBootId, sessions });
}

/** 取当前 bootId：ensureBootId 首次成功后缓存在本模块（activeBootId）；
 *  未缓存则主动拉一次 /host。null = 尚不可得（离线/未配对） */
export async function ensureBootId(): Promise<string | null> {
  if (activeBootId !== null) return activeBootId;
  try {
    const h = await fetchHost();
    activeBootId = h?.host.bootId ?? null;
  } catch {
    activeBootId = null; // 网络异常：本次恢复/持久化跳过（纯内存态）
  }
  return activeBootId;
}

/** 该会话的书签表（副本——外部改写不污染单例） */
export function listBookmarks(sessionId: string): Bookmark[] {
  return [...(store.get(sessionId) ?? [])];
}

/** 加书签：颜色已占用 或 已达上限 → 拒绝（no-op，不静默替换——
 *  用户裁决：颜色即唯一键、硬上限 10） */
export function addBookmark(sessionId: string, b: Bookmark): void {
  const list = store.get(sessionId) ?? [];
  const full = list.length >= BOOKMARK_LIMIT;
  const taken = list.some((x) => x.color === b.color);
  if (full || taken) return;
  store.set(sessionId, [...list, b]);
  persist();
}

/** 删单条（按颜色）：不存在则 no-op */
export function removeBookmark(sessionId: string, color: string): void {
  const list = store.get(sessionId) ?? [];
  store.set(
    sessionId,
    list.filter((x) => x.color !== color)
  );
  persist();
}

/** 清空该会话全部书签（不影响其它会话） */
export function clearBookmarks(sessionId: string): void {
  store.delete(sessionId);
  persist();
}
