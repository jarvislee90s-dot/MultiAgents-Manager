import { useTranslation } from "react-i18next";
import { Cpu, Clock, Bot, ChevronRight, X, ArrowLeftRight } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import { toast } from "sonner";
import { Card } from "@/components/ui/card";
import { StatusLight } from "@/components/sessions/StatusLight";
import { useSessionJump } from "@/hooks/useSessionJump";
import { AGENT_BADGE, getAgentLabel } from "@/lib/agentBadge";
import { sessionTitleOrUndefined } from "@/lib/sessionTitle";
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

// 全角/CJK 字符按 1.5 单位计入显示预算（10 个中文 = 15 单位）
const CJK_CHAR =
  /[\u2e80-\u303f\u31c0-\u31ef\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;

/**
 * 项目名显示预算（卡片顶行）：中文封顶 10 字、英文封顶 15 字符（按 15 单位宽度
 * 预算折算，CJK 每字 1.5 单位），超出截断加省略号；完整名走 title 悬停提示。
 * CSS truncate 仅作兜底——预算保证不同卡宽下截断点可预期
 */
export function truncateProjectName(name: string, budget = 15): string {
  const chars = [...name];
  let cost = 0;
  for (let i = 0; i < chars.length; i++) {
    cost += CJK_CHAR.test(chars[i]) ? 1.5 : 1;
    if (cost > budget) return chars.slice(0, i).join("") + "…";
  }
  return name;
}

export function SessionCard({
  session,
  pairingAmbiguous = false,
}: {
  session: Session;
  /** 同工具同项目双开：卡片信息（状态/最后消息）可能互换的提示角标（issue #48） */
  pairingAmbiguous?: boolean;
}) {
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
        title: sessionTitleOrUndefined(session),
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
          "group hover:bg-accent/50 @container relative cursor-pointer border p-3 transition-colors",
          session.status === "waiting" && "border-red-500/40",
          !session.jumpSupported && "cursor-default opacity-80"
        )}
        onClick={handleClick}
        title={session.jumpSupported ? t("sessions.jumpToTerminal") : t("sessions.jumpUnsupported")}
      >
        {/* 顶部：工具标签 + 项目目录名（自然宽度优先完整显示）+ 会话标题尾巴 + 分支 | 状态指示区 | 关闭 X 最右。
            会话标题主要用于跳转匹配定位而非阅读，压缩为灰色尾巴（悬停 tooltip 看全文）；
            空间不足时尾巴先截断（flex-shrink 5 倍让路），项目名最后才截——
            kimi 长任务标题曾把项目名挤成单字（窄卡 + DPI 缩放下定宽尾巴同样会饿死项目名） */}
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <span
              className={cn(
                "inline-flex shrink-0 items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-semibold",
                badge.className
              )}
            >
              <Icon className="h-3 w-3" />
              {getAgentLabel(session.agentType, session.form)}
            </span>
            <span className="min-w-0 truncate text-sm font-medium" title={session.projectName}>
              {truncateProjectName(session.projectName)}
            </span>
            {pairingAmbiguous && (
              <span
                className="shrink-0 text-amber-500/80"
                title={t("sessions.pairingAmbiguous")}
                aria-label={t("sessions.pairingAmbiguous")}
              >
                <ArrowLeftRight className="h-3 w-3" />
              </span>
            )}
            {/* issue #51：容器查询阈值隐藏——卡宽低于 25rem（400px）时整段收起，
                避免尾巴被项目名挤压到只剩孤立省略号（实测窄卡 ≈366px 时发生）；
                跳转匹配消费的是 session.title 数据而非可见文本，隐藏无功能影响 */}
            {(session.title || session.id) && (
              <span
                className="text-muted-foreground/60 hidden max-w-[14em] min-w-0 [flex-shrink:5] truncate font-mono text-[10px] @min-[25rem]:block"
                title={session.title || session.id.slice(0, 8)}
              >
                {session.title || session.id.slice(0, 8)}
              </span>
            )}
            {session.gitBranch && (
              <span className="text-muted-foreground shrink-0 font-mono text-[10px]">
                {session.gitBranch}
              </span>
            )}
          </div>
          {/* 状态指示区：状态灯与未读合并为单一指示（未读时绿点加光环，不再渲染独立小绿点） */}
          <div className="flex shrink-0 items-center gap-1.5">
            <div className="relative" title={session.unread ? t("sessions.unread") : undefined}>
              <StatusLight status={session.status} size="sm" />
              {session.unread && (
                <span
                  className="pointer-events-none absolute -inset-1 rounded-full ring-2 ring-emerald-400/80"
                  aria-label={t("sessions.unread")}
                />
              )}
            </div>
            {(session.unread || session.form === "app") && (
              <button
                onClick={handleClose}
                className="text-muted-foreground hover:bg-muted hover:text-foreground rounded p-0.5"
                title={session.unread ? t("sessions.markRead") : t("sessions.dismissCard")}
                aria-label={session.unread ? t("sessions.markRead") : t("sessions.dismissCard")}
              >
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
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
