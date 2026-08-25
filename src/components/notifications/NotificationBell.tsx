// 通知历史铃铛 — 未读角标 + 历史面板（点击条目跳转对应会话）
import { useEffect, useState } from "react";
import { Bell } from "lucide-react";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import { AGENT_BADGE } from "@/lib/agentBadge";
import { useSessionJump } from "@/hooks/useSessionJump";
import {
  getHistory,
  getUnreadCount,
  markAllRead,
  type HistoryEntry,
} from "@/lib/notificationHistory";

function timeAgo(at: number, t: (k: string) => string): string {
  const mins = Math.floor((Date.now() - at) / 60000);
  if (mins < 1) return t("sessions.justNow");
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h`;
}

export function NotificationBell() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [unread, setUnread] = useState(0);
  const { focus } = useSessionJump();

  useEffect(() => {
    const refresh = () => {
      setEntries(getHistory());
      setUnread(getUnreadCount());
    };
    refresh();
    window.addEventListener("mam-history-updated", refresh);
    return () => window.removeEventListener("mam-history-updated", refresh);
  }, []);

  const toggle = () => {
    const next = !open;
    setOpen(next);
    if (next) markAllRead();
  };

  const jumpTo = async (e: HistoryEntry) => {
    try {
      await focus({
        pid: e.pid,
        id: e.sessionId,
        agentType: e.agentType,
        projectName: e.projectName,
        lastMessage: e.lastMessage,
      });
    } catch {
      toast.error(t("notifications.jumpFailed"));
    }
  };

  return (
    <div className="relative">
      <button
        className="hover:bg-accent relative rounded p-1.5"
        onClick={toggle}
        title={t("notifications.historyTitle")}
      >
        <Bell className="h-4 w-4" />
        {unread > 0 && (
          <span className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-red-500 px-1 text-[9px] font-bold text-white">
            {unread > 99 ? "99+" : unread}
          </span>
        )}
      </button>
      {open && (
        <div className="bg-card absolute right-0 z-50 mt-2 w-96 rounded-lg border p-2 shadow-xl">
          <p className="mb-2 px-1 text-xs font-semibold">{t("notifications.historyTitle")}</p>
          <div className="max-h-80 overflow-y-auto">
            {entries.length === 0 && (
              <p className="text-muted-foreground p-4 text-center text-xs">
                {t("notifications.historyEmpty")}
              </p>
            )}
            {entries.map((e) => {
              const badge = AGENT_BADGE[e.agentType];
              return (
                <button
                  key={e.at + e.sessionId}
                  className="hover:bg-accent/50 flex w-full items-center gap-2 rounded px-2 py-1.5 text-left"
                  onClick={() => jumpTo(e)}
                >
                  {badge && (
                    <span
                      className={`inline-flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[9px] ${badge.className}`}
                    >
                      <badge.Icon className="h-2.5 w-2.5" />
                      {badge.label}
                    </span>
                  )}
                  <span className="min-w-0 flex-1 truncate text-[11px]">
                    {e.projectName} — {e.lastMessage || t("sessions.noMessage")}
                  </span>
                  <span className="text-muted-foreground shrink-0 text-[10px]">
                    {timeAgo(e.at, t)}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
