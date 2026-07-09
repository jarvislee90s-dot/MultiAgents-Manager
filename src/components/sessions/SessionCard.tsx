import { invoke } from "@tauri-apps/api/core";
import { Terminal, Cpu, Clock, Bot, FolderGit2, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { Card } from "@/components/ui/card";
import { StatusLight } from "@/components/sessions/StatusLight";
import type { Session, AgentType } from "@/types/session";

const AGENT_BADGE: Record<AgentType, { label: string; className: string; icon: typeof Bot }> = {
  claude: { label: "Claude", className: "bg-purple-500/15 text-purple-400 border-purple-500/30", icon: Bot },
  codex: { label: "Codex", className: "bg-green-500/15 text-green-400 border-green-500/30", icon: Terminal },
  opencode: { label: "OpenCode", className: "bg-orange-500/15 text-orange-400 border-orange-500/30", icon: FolderGit2 },
};

function formatRuntime(lastActivityAt: string): string {
  if (!lastActivityAt || lastActivityAt === "Unknown") return "--";
  // 尝试解析 ISO 时间戳或 Claude 的时间格式
  const date = new Date(lastActivityAt);
  if (isNaN(date.getTime())) return lastActivityAt.slice(0, 19);
  const diff = Date.now() - date.getTime();
  if (diff < 0) return "刚刚";
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  return `${hours}h${mins % 60}m`;
}

export function SessionCard({ session }: { session: Session }) {
  const badge = AGENT_BADGE[session.agentType];
  const Icon = badge.icon;

  const handleClick = async () => {
    if (!session.jumpSupported) {
      toast.info("桌面 APP 形态不支持终端跳转");
      return;
    }
    try {
      await invoke("focus_session", { pid: session.pid });
    } catch (e) {
      toast.error(`跳转失败: ${e}`);
    }
  };

  return (
    <Card
      className={cn(
        "group relative cursor-pointer border p-3 transition-colors hover:bg-accent/50",
        session.status === "waiting" && "border-red-500/40",
        !session.jumpSupported && "cursor-default opacity-80"
      )}
      onClick={handleClick}
      title={session.jumpSupported ? "点击跳转到终端" : "桌面 APP 形态：不可跳转终端"}
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
              <span
                className="text-[9px] opacity-60"
                title="桌面 APP 形态：不可跳转终端，仅可监控"
              >
                APP
              </span>
            )}
          </span>
          <span className="truncate text-sm font-medium">
            {session.projectName}
          </span>
          {(session.title || session.id) && (
            <span className="text-muted-foreground/60 truncate text-[10px] font-mono">
              {session.title || session.id.slice(0, 12)}
            </span>
          )}
          {session.gitBranch && (
            <span className="text-muted-foreground shrink-0 text-[10px] font-mono">
              {session.gitBranch}
            </span>
          )}
        </div>
        <StatusLight status={session.status} size="sm" />
      </div>

      {/* 中间：最后消息预览 */}
      <p className="text-muted-foreground mb-2 line-clamp-2 min-h-[2.5rem] text-xs">
        {session.lastMessage || "（无消息）"}
      </p>

      {/* 底部：CPU + PID + 运行时长 */}
      <div className="text-muted-foreground flex items-center gap-3 text-[10px]">
        <span className="flex items-center gap-1">
          <Cpu className="h-3 w-3" />
          {session.cpuUsage.toFixed(1)}%
        </span>
        <span className="flex items-center gap-1">
          <Clock className="h-3 w-3" />
          {formatRuntime(session.lastActivityAt)}
        </span>
        {session.activeSubagentCount > 0 && (
          <span className="flex items-center gap-1">
            <Bot className="h-3 w-3" />
            {session.activeSubagentCount} 子Agent
          </span>
        )}
        {session.jumpSupported && (
          <ChevronRight className="ml-auto h-3 w-3 opacity-0 transition-opacity group-hover:opacity-50" />
        )}
      </div>
    </Card>
  );
}
