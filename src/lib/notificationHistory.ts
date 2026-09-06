// 通知历史 — localStorage 持久化（最新在前，容量 50）
export interface HistoryEntry {
  agentType: string;
  form?: string;
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

// #5b：清空全部（含未读），照 markAllRead 模式落盘 + 派发事件
export function clearHistory() {
  localStorage.setItem(KEY, JSON.stringify([]));
  window.dispatchEvent(new CustomEvent("mam-history-updated"));
}

// #5b：仅删除已读条目，未读保留（角标计数不变）
export function clearRead() {
  localStorage.setItem(KEY, JSON.stringify(getHistory().filter((e) => !e.read)));
  window.dispatchEvent(new CustomEvent("mam-history-updated"));
}

export function getUnreadCount(): number {
  return getHistory().filter((e) => !e.read).length;
}
