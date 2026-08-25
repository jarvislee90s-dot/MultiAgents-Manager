// 通知历史 — localStorage 持久化（最新在前，容量 50）
export interface HistoryEntry {
  agentType: string;
  projectName: string;
  status: string;
  lastMessage: string;
  pid: number;
  sessionId: string;
  at: number;
  read: boolean;
}

const KEY = "mam-notification-history";
const CAP = 50;

export function getHistory(): HistoryEntry[] {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]");
  } catch {
    return [];
  }
}

export function addHistory(entry: Omit<HistoryEntry, "read">) {
  const list = [{ ...entry, read: false }, ...getHistory()].slice(0, CAP);
  localStorage.setItem(KEY, JSON.stringify(list));
  window.dispatchEvent(new CustomEvent("mam-history-updated"));
}

export function markAllRead() {
  localStorage.setItem(KEY, JSON.stringify(getHistory().map((e) => ({ ...e, read: true }))));
  window.dispatchEvent(new CustomEvent("mam-history-updated"));
}

export function getUnreadCount(): number {
  return getHistory().filter((e) => !e.read).length;
}
