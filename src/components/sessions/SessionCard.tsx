import { useTranslation } from "react-i18next";
import { Cpu, Clock, Bot, ChevronRight, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { Card } from "@/components/ui/card";
import { StatusLight } from "@/components/sessions/StatusLight";
import { useSessionJump } from "@/hooks/useSessionJump";
import { AGENT_BADGE, getAgentLabel } from "@/lib/agentBadge";
import type { Session } from "@/types/session";

function formatRuntime(lastActivityAt: string, t: (key: string) => string): string {
  if (!lastActivityAt || lastActivityAt === "Unknown") return "--";
  // 尝试解析 ISO 时间戳或 Claude 的时间格式
  const date = new Date(lastActivityAt);
  if (isNaN(date.getTime())) return lastActivityAt.slice(0, 19);
  const diff = Date.now() - date.getTime();
  if (diff < 0) return t("sessions.justNow");
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return t("sessions.justNow");
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  return `${hours}h${mins % 60}m`;
}

export function SessionCard({ session }: { session: Session }) {
  const { t } = useTranslation();
  const badge = AGENT_BADGE[session.agentType];
  const Icon = badge.Icon;
  // 跳转共享逻辑（歧义候选窗口由 hook 状态承载，命中多个窗口时弹出选择器）
  const { candidates, setCandidates, focus, focusHwnd } = useSessionJump();

  // 手动关闭卡（X）：不触发卡片跳转。
  // 未读卡 = 标记已读（mark_session_read，spec W4 已读信号 2）；
  // 活跃 App 卡（黄/红）= 暂离不提示（dismiss_session_card，T2）——写入进程内
  // dismiss 集合从看板与宠物隐藏，同一会话状态变化后自然重现
  const handleClose = async (e: React.MouseEvent) => {
    e.stopPropagation(); // 不触发卡片跳转
    try {
      if (session.unread) {
        await invoke("mark_session_read", {
          agentType: session.agentType,
          sessionId: session.id,
        });
      } else {
        await invoke("dismiss_session_card", {
          agentType: session.agentType,
          sessionId: session.id,
          status: session.status,
        });
      }
    } catch (err) {
      console.error("session card close failed:", err);
    }
  };

  const handleClick = async () => {
    if (!session.jumpSupported) {
      toast.info(t("sessions.jumpUnsupported"));
      return;
    }
    try {
      await focus({
        pid: session.pid,
        id: session.id,
        agentType: session.agentType,
        projectName: session.projectName,
        lastMessage: session.lastMessage ?? undefined,
        unread: session.unread, // 歧义选择器点选成功后回标已读用（spec W4 已读信号 1）
        form: session.form, // review M3：CLI 会话 APP 级保底激活时的 UX 提示依据
      });
    } catch (e) {
      toast.error(t("sessions.jumpFailed", { error: e }));
    }
  };

  return (
    <>
      <Card
        className={cn(
          "group hover:bg-accent/50 relative cursor-pointer border p-3 transition-colors",
          session.status === "waiting" && "border-red-500/40",
          !session.jumpSupported && "cursor-default opacity-80"
        )}
        onClick={handleClick}
        title={session.jumpSupported ? t("sessions.jumpToTerminal") : t("sessions.jumpUnsupported")}
      >
        {/* 顶部：工具标签 + 项目名 + 状态灯 */}
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2">
            <span
              className={cn(
                "inline-flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-semibold",
                badge.className
              )}
            >
              <Icon className="h-3 w-3" />
              {getAgentLabel(session.agentType, session.form)}
            </span>
            <span className="truncate text-sm font-medium">{session.projectName}</span>
            {(session.unread || session.form === "app") && (
              <span className="inline-flex items-center gap-1">
                {session.unread && (
                  <span
                    className="h-1.5 w-1.5 rounded-full bg-emerald-400"
                    aria-label={t("sessions.unread")}
                  />
                )}
                <button
                  onClick={handleClose}
                  className="text-muted-foreground hover:bg-muted hover:text-foreground rounded p-0.5"
                  title={session.unread ? t("sessions.markRead") : t("sessions.dismissCard")}
                  aria-label={session.unread ? t("sessions.markRead") : t("sessions.dismissCard")}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            )}
            {(session.title || session.id) && (
              <span className="text-muted-foreground/60 truncate font-mono text-[10px]">
                {session.title || session.id.slice(0, 8)}
              </span>
            )}
            {session.gitBranch && (
              <span className="text-muted-foreground shrink-0 font-mono text-[10px]">
                {session.gitBranch}
              </span>
            )}
          </div>
          <StatusLight status={session.status} size="sm" />
        </div>

        {/* 中间：最后消息预览 */}
        <p className="text-muted-foreground mb-2 line-clamp-2 min-h-[2.5rem] text-xs">
          {session.lastMessage || t("sessions.noMessage")}
        </p>

        {/* 底部：CPU + PID + 运行时长 */}
        <div className="text-muted-foreground flex items-center gap-3 text-[10px]">
          <span className="flex items-center gap-1">
            <Cpu className="h-3 w-3" />
            {session.cpuUsage.toFixed(1)}%
          </span>
          <span className="flex items-center gap-1">
            <Clock className="h-3 w-3" />
            {formatRuntime(session.lastActivityAt, t)}
          </span>
          {session.activeSubagentCount > 0 && (
            <span className="flex items-center gap-1">
              <Bot className="h-3 w-3" />
              {t("sessions.subagents", { n: session.activeSubagentCount })}
            </span>
          )}
          {session.jumpSupported && (
            <ChevronRight className="ml-auto h-3 w-3 opacity-0 transition-opacity group-hover:opacity-50" />
          )}
        </div>
      </Card>
      {/* 窗口选择器：跳转歧义时由用户点选目标窗口 */}
      {candidates && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
          onClick={() => setCandidates(null)}
        >
          <div
            className="bg-card w-96 rounded-lg border p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium">{t("sessions.pickWindow")}</p>
            <div className="flex flex-col gap-2">
              {candidates.map((w) => (
                <button
                  key={w.hwnd}
                  className="hover:bg-accent truncate rounded border px-3 py-2 text-left text-xs"
                  onClick={async () => {
                    setCandidates(null);
                    try {
                      await focusHwnd(w.hwnd);
                    } catch (e) {
                      toast.error(t("sessions.jumpFailed", { error: e }));
                    }
                  }}
                  title={w.title}
                >
                  {w.title || t("sessions.untitledWindow")} — {w.process}
                </button>
              ))}
            </div>
          </div>
        </div>
      )}
    </>
  );
}
