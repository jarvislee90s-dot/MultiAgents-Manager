import { useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Terminal, Cpu, Clock, Bot, FolderGit2, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { Card } from "@/components/ui/card";
import { StatusLight } from "@/components/sessions/StatusLight";
import type { Session, AgentType } from "@/types/session";

const AGENT_BADGE: Record<AgentType, { label: string; className: string; icon: typeof Bot }> = {
  claude: {
    label: "Claude",
    className: "bg-purple-500/15 text-purple-400 border-purple-500/30",
    icon: Bot,
  },
  codex: {
    label: "Codex",
    className: "bg-green-500/15 text-green-400 border-green-500/30",
    icon: Terminal,
  },
  opencode: {
    label: "OpenCode",
    className: "bg-orange-500/15 text-orange-400 border-orange-500/30",
    icon: FolderGit2,
  },
};

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
  const Icon = badge.icon;
  // 歧义候选窗口（跳转命中多个窗口时弹出选择器）
  const [pendingWindows, setPendingWindows] = useState<
    { hwnd: number; title: string; process: string }[] | null
  >(null);

  const handleClick = async () => {
    if (!session.jumpSupported) {
      toast.info(t("sessions.jumpUnsupported"));
      return;
    }
    try {
      const result = await invoke<{
        type: string;
        windows?: { hwnd: number; title: string; process: string }[];
      }>("focus_session", {
        pid: session.pid,
        sessionId: session.id,
        agentType: session.agentType,
        projectName: session.projectName,
      });
      if (result.type === "ambiguous" && result.windows && result.windows.length > 0) {
        setPendingWindows(result.windows);
      }
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
              {badge.label}
              {session.form === "app" && (
                <span className="text-[9px] opacity-60" title={t("sessions.appBadge")}>
                  APP
                </span>
              )}
            </span>
            <span className="truncate text-sm font-medium">{session.projectName}</span>
            {(session.title || session.id) && (
              <span className="text-muted-foreground/60 truncate font-mono text-[10px]">
                {session.title || session.id.slice(0, 12)}
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
      {pendingWindows && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
          onClick={() => setPendingWindows(null)}
        >
          <div
            className="bg-card w-96 rounded-lg border p-4 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <p className="mb-3 text-sm font-medium">{t("sessions.pickWindow")}</p>
            <div className="flex flex-col gap-2">
              {pendingWindows.map((w) => (
                <button
                  key={w.hwnd}
                  className="hover:bg-accent truncate rounded border px-3 py-2 text-left text-xs"
                  onClick={async () => {
                    setPendingWindows(null);
                    try {
                      await invoke("focus_hwnd", { hwnd: w.hwnd });
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
