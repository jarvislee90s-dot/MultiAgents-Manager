import { useEffect, useState } from "react";
import {
  ApiError,
  deleteArchivedSession,
  fetchSessionMessages,
  sessionOpen,
  type ArchivedSession,
  type SessionMessage,
} from "./api";
// 复用既有导出（勿自造）：STATUS_LABELS 状态中文 / formatRelativeTime 相对时间
import { STATUS_LABELS, formatRelativeTime } from "./board-logic";
import { chipLabel } from "./archive-logic";
import { RESUME_UNSUPPORTED_REASON, resumeUnavailableReason } from "./resume-gate";

/** 打开失败文案分诊（评审 Minor：归档语境哨兵引导）。哨兵串与后端 api.rs 的
 *  Err 哨兵对应（404 载荷 data.error）：no_session=活/档双未命中（归档记录可能
 *  已被移除/清空，引导刷新）；no_resume_command=工具未入命令表（复用 resume-gate
 *  门的文案）；其余透传后端原文。 */
function openFailureCopy(error: string | null | undefined): string {
  if (error === "no_session") return "归档记录已不存在，请刷新列表";
  if (error === "no_resume_command") return `打开失败：${RESUME_UNSUPPORTED_REASON}`;
  return `打开失败：${error ?? "未知错误"}`;
}

/** 归档详情页（spec §7.3）：只读消息（/session-messages 按 id 直读文件，零后端改动）
 *  + 唯一动作「在桌面端打开」（复用 sessionOpen）+ 次要动作「从归档移除」。 */
export default function ArchiveDetail({
  session,
  onBack,
  onActivated,
}: {
  session: ArchivedSession;
  onBack: () => void;
  onActivated: () => void;
}) {
  const [messages, setMessages] = useState<SessionMessage[] | null>(null);
  const [contentError, setContentError] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  // 移除失败行内反馈（终审 Finding 3：Promise 拒绝无 .catch → 按钮无反应 +
  // unhandled rejection）；不复用 contentError/openError（语义不同），下次点击清除
  const [removeError, setRemoveError] = useState(false);
  // 相对时长基准时钟（react-hooks/purity 禁渲染期调 Date.now，同 Board 惯例）：
  // 挂载时取一次快照（归档详情为静态只读页，无需定时走动）
  const [now] = useState(() => Date.now());
  const reason = resumeUnavailableReason(session);
  // 最后状态中文（spec §7.3 顶部信息行要求）：lastStatus 小写串与 STATUS_LABELS
  // 六键（waiting/processing/thinking/compacting/idle/finished）精确对齐——Rust 端
  // format!("{:?}", status).to_lowercase() 产物一致；未知串回退原文
  const statusLabel =
    (STATUS_LABELS as Record<string, string>)[session.lastStatus] ?? session.lastStatus;

  useEffect(() => {
    let alive = true;
    fetchSessionMessages(session.agentType, session.sessionId, 200)
      .then((p) => {
        if (alive) setMessages(p.messages);
      })
      .catch(() => {
        if (alive) setContentError(true);
      });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.sessionId]);

  const handleOpen = async () => {
    setOpening(true);
    setOpenError(null);
    try {
      const r = await sessionOpen(session.sessionId);
      if (r.status === "opening") {
        onActivated(); // 乐观回看板：活板 3s 内出卡，归档条目下次拉取自然消失
        return;
      }
      setOpenError(openFailureCopy(r.error));
    } catch (e) {
      // 404 哨兵在响应体 data.error（api.ts 解析进 ApiError.data）——命中哨兵走
      // 归档语境文案，其余（网络异常等）维持原 String(e) 形态
      const code = e instanceof ApiError ? (e.data?.error as string | undefined) : undefined;
      setOpenError(code !== undefined ? openFailureCopy(code) : `打开失败：${String(e)}`);
    } finally {
      setOpening(false);
    }
  };

  return (
    <div className="mx-auto flex max-w-3xl flex-col px-4">
      <header className="flex items-center gap-2 pt-4">
        <button
          type="button"
          onClick={onBack}
          className="text-sm text-slate-500"
          aria-label="返回历史页"
        >
          ‹ 返回
        </button>
        <h1 className="text-lg font-semibold">{session.projectName}</h1>
      </header>
      <p className="mt-1 text-xs text-slate-500">
        {chipLabel(session.agentType)} · {session.projectPath} · {statusLabel} ·{" "}
        {formatRelativeTime(session.lastSeenAt, now)}结束
      </p>

      <div className="shrink-0 px-1 pt-3">
        <button
          type="button"
          data-testid="session-open"
          disabled={reason !== null || opening}
          title={reason ?? undefined}
          onClick={handleOpen}
          className="w-full rounded-lg bg-green-700 px-3 py-2 text-sm text-white enabled:hover:bg-green-800 disabled:opacity-50"
        >
          {opening ? "正在电脑上打开终端…" : "在桌面端打开"}
        </button>
        {reason && <p className="mt-1 text-center text-xs text-slate-400">{reason}</p>}
        {openError && (
          <p data-testid="session-open-error" className="mt-1 text-center text-xs text-red-600">
            {openError}
          </p>
        )}
      </div>

      <div className="mt-3 flex min-h-0 flex-1 flex-col gap-2 pb-8">
        {contentError && (
          <p className="py-6 text-center text-sm text-slate-400">
            内容暂不可读（会话文件可能已被工具清理）
          </p>
        )}
        {messages === null && !contentError && (
          <p className="py-6 text-center text-sm text-slate-400">加载中…</p>
        )}
        {messages?.length === 0 && (
          <p className="py-6 text-center text-sm text-slate-400">（无历史消息）</p>
        )}
        {messages?.map((m) => (
          <div key={m.seq} className="rounded-lg bg-slate-100 px-3 py-2 text-sm dark:bg-slate-900">
            <span className="mr-2 text-xs text-slate-400">{m.role}</span>
            {m.content}
          </div>
        ))}
      </div>

      <div className="pb-6">
        {confirmRemove ? (
          <span className="flex gap-2">
            <button
              type="button"
              data-testid="archive-remove-confirm"
              className="flex-1 rounded-lg border border-red-300 px-3 py-1.5 text-xs text-red-600"
              onClick={() => {
                setRemoveError(false);
                deleteArchivedSession(session.sessionId)
                  .then(onBack)
                  .catch(() => setRemoveError(true));
              }}
            >
              确认移除
            </button>
            <button
              type="button"
              className="flex-1 rounded-lg border border-slate-200 px-3 py-1.5 text-xs"
              onClick={() => setConfirmRemove(false)}
            >
              取消
            </button>
          </span>
        ) : (
          <button
            type="button"
            data-testid="archive-remove"
            className="w-full rounded-lg px-3 py-1.5 text-xs text-slate-400"
            onClick={() => setConfirmRemove(true)}
          >
            从归档移除
          </button>
        )}
        {removeError && (
          <p
            data-testid="archive-remove-error"
            className="mt-1 text-center text-xs text-red-600"
          >
            操作失败，请重试
          </p>
        )}
      </div>
    </div>
  );
}
