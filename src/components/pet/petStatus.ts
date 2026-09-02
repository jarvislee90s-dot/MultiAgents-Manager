// 六态 → 桌宠灯色差分推导（纯函数，spec §5）。卡片=状态展示，事件/未读=差分。
import type { Session, SessionStatus } from "@/types/session";

export type PetLight = "waiting" | "running" | "done";
export type StatusColor = "red" | "yellow" | "green";

export interface PetCard {
  id: string;
  title: string;
  lines: string[];
  light: PetLight;
  unread: boolean;
}

export interface PetEntry {
  light: PetLight | null;
  prevColor: StatusColor | null;
  unread: boolean;
  vanishedAt: number | null;
  title: string;
  lines: string[];
}

export type PetStatusState = Record<string, PetEntry>;

const RUNNING_SET: ReadonlySet<SessionStatus> = new Set(["processing", "thinking", "compacting"]);
const MAX_CARDS = 6;
const VANISH_TTL_MS = 60_000;

export function statusColor(s: SessionStatus): StatusColor {
  if (s === "waiting") return "red";
  if (RUNNING_SET.has(s)) return "yellow";
  return "green"; // idle / finished
}

// 词元感知截断（移植原版 estimateTokens/truncate，CJK 每字 1 词元）
function estimateTokens(text: string): number {
  let n = 0;
  let inWord = false;
  for (const ch of text) {
    if (/[\u4e00-\u9fff]/.test(ch)) {
      n += 1;
      inWord = false;
    } else if (/\s/.test(ch)) inWord = false;
    else if (!inWord) {
      n += 1;
      inWord = true;
    }
  }
  return n;
}

export function truncate(s: string, maxTokens = 24): string {
  const t = (s || "").replace(/\s+/g, " ").trim();
  const maxChars = maxTokens * 2; // 字符兜底：超长无空格连续串（如 URL/重复字符）按词元只算 1 个词，需按字符数截断
  if (!t || (estimateTokens(t) <= maxTokens && t.length <= maxChars)) return t;
  let n = 0;
  let inWord = false;
  let cut = Math.min(t.length, maxChars);
  for (let i = 0; i < t.length; i++) {
    const ch = t[i];
    if (/[\u4e00-\u9fff]/.test(ch)) {
      n += 1;
      inWord = false;
    } else if (/\s/.test(ch)) inWord = false;
    else if (!inWord) {
      n += 1;
      inWord = true;
    }
    if (n >= maxTokens || i + 1 >= maxChars) {
      cut = i + 1;
      break;
    }
  }
  return t.slice(0, cut).trim() + "…";
}

function cardLines(color: StatusColor, session: Session): string[] {
  if (color === "red")
    return ["等待操作", ...(session.lastMessage ? [truncate(session.lastMessage)] : [])];
  if (color === "green") return ["已完成"];
  return [session.lastMessage ? truncate(session.lastMessage) : "运行中"];
}

export function computePetStatus(
  sessions: Session[],
  prev: PetStatusState | null,
  now: number
): {
  cards: PetCard[];
  moreCount: number;
  events: { newWaiting: string[]; newCompletion: string[] };
  state: PetStatusState;
} {
  const first = prev === null;
  const state: PetStatusState = {};
  const events = { newWaiting: [] as string[], newCompletion: [] as string[] };

  for (const s of sessions) {
    const color = statusColor(s.status);
    const p = prev?.[s.id];
    const completion = !first && !!p?.prevColor && p.prevColor !== "green" && color === "green";
    const unread = first ? false : completion || (!!p && p.light === "done" && p.unread);
    const light: PetLight | null =
      color === "red" ? "waiting" : color === "yellow" ? "running" : unread ? "done" : null;
    const title = s.title || s.projectName || s.id;
    // vanishedAt 记录「最后见到该会话」的时间，消失 TTL 以此起算（spec D9/H4）
    state[s.id] = {
      light,
      prevColor: color,
      unread,
      vanishedAt: now,
      title,
      lines: cardLines(color, s),
    };
    if (completion) events.newCompletion.push(s.id);
    if (!first && color === "red" && p?.prevColor !== "red") events.newWaiting.push(s.id);
  }

  // 消失会话：自最后见到起保留 60s（未读绿卡不瞬间消失），不亮断联灯（spec D9/H4）
  for (const [id, p] of Object.entries(prev ?? {})) {
    if (state[id]) continue;
    if (p.vanishedAt !== null && now - p.vanishedAt > VANISH_TTL_MS) continue; // 清理
    state[id] = { ...p, vanishedAt: p.vanishedAt ?? now };
  }

  const all = cardsFromState(state);
  return {
    cards: all.slice(0, MAX_CARDS),
    moreCount: Math.max(0, all.length - MAX_CARDS),
    events,
    state,
  };
}

const LIGHT_RANK: Record<PetLight, number> = { waiting: 0, running: 1, done: 2 };

export function cardsFromState(state: PetStatusState): PetCard[] {
  return Object.entries(state)
    .filter(([, e]) => e.light !== null)
    .map(([id, e]) => ({
      id,
      title: e.title,
      lines: e.lines,
      light: e.light as PetLight,
      unread: e.unread,
    }))
    .sort(
      (a, b) => LIGHT_RANK[a.light] - LIGHT_RANK[b.light] || a.title.localeCompare(b.title, "zh")
    );
}

/** 绿卡点击已读即消（spec C2） */
export function ackDone(state: PetStatusState, id: string): void {
  const e = state[id];
  if (!e || e.light !== "done") return;
  e.unread = false;
  e.light = null;
}
