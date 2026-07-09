import { SessionCard } from "@/components/sessions/SessionCard";
import type { Session } from "@/types/session";
import { Monitor } from "lucide-react";

export function SessionGrid({ sessions }: { sessions: Session[] }) {
  if (sessions.length === 0) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-20 text-center">
        <Monitor className="text-muted-foreground/40 h-12 w-12" />
        <div>
          <p className="text-muted-foreground text-sm">暂无活跃会话</p>
          <p className="text-muted-foreground/60 mt-1 text-xs">
            在终端中运行 Claude Code、Codex CLI 或 OpenCode 即可在此监控
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
      {sessions.map((session) => (
        <SessionCard key={`${session.agentType}-${session.id}`} session={session} />
      ))}
    </div>
  );
}
