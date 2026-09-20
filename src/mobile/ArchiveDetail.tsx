import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDownToLine, ArrowUpToLine, ChevronDown, ChevronRight } from "lucide-react";
import {
  ApiError,
  deleteArchivedSession,
  fetchSessionMessages,
  sessionOpen,
  unhideSession,
  type ArchivedSession,
  type SessionMessage,
} from "./api";
// 复用既有导出（勿自造）：STATUS_LABELS 状态中文 / formatRelativeTime 相对时间
import { STATUS_LABELS, formatRelativeTime } from "./board-logic";
import { chipLabel } from "./archive-logic";
import { collapsedLabel, isProcessKind } from "./message-fold";
import { JUMP_SHOW_THRESHOLD_PX } from "./SessionDetail";
import { RESUME_UNSUPPORTED_REASON, resumeUnavailableReason } from "./resume-gate";

/** 归档详情单次拉取条数（与活会话详情默认 limit 同标尺；truncated 即有更早内容未载） */
const ARCHIVE_PAGE_LIMIT = 200;
/** limit 翻倍上限（与活会话 load-more 同标尺）：到达后不再提供「加载更早消息」 */
const MAX_LIMIT = 1000;

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
 *  + 唯一动作「在桌面端打开」（复用 sessionOpen）+ 次要动作「从归档移除」。
 *  交互对齐活会话（2026-09-20 体验批）：独立滚动容器 + 进入落底 + 到顶/到底浮动钮
 *  + 总结模式折叠（归档恒为死会话：过程消息默认折叠、只直显最后 assistant 总结）
 *  + 底部动作区（打开在上、移除在下）。 */
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
  const [truncated, setTruncated] = useState(false);
  // 加载更早消息（体验批二，活会话同款）：limit 翻倍整页重拉（200→400→…→1000）
  const [limit, setLimit] = useState(ARCHIVE_PAGE_LIMIT);
  const [contentError, setContentError] = useState(false);
  const [opening, setOpening] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  // 移除失败行内反馈（终审 Finding 3：Promise 拒绝无 .catch → 按钮无反应 +
  // unhandled rejection）；不复用 contentError/openError（语义不同），下次点击清除
  const [removeError, setRemoveError] = useState(false);
  // 移回看板失败行内反馈（软归档活会话的动作）；下次点击清除
  const [unhideError, setUnhideError] = useState(false);
  // 折叠覆盖表：seq → 强制折叠/展开（活会话同款语义；「收起」= 清空回默认折叠态）
  const [expandedOverride, setExpandedOverride] = useState<Map<number, boolean>>(new Map());
  // 浮动钮显隐（onScroll 驱动；阈值与活会话「跳到最新」同源）
  const [showJumpTop, setShowJumpTop] = useState(false);
  const [showJumpBottom, setShowJumpBottom] = useState(false);
  const areaRef = useRef<HTMLDivElement>(null);
  // 加载更早消息的滚动锚：重拉后把视图锚回原顶部消息（不把用户甩到底部）
  const anchorSeqRef = useRef<number | null>(null);
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
    fetchSessionMessages(session.agentType, session.sessionId, limit)
      .then((p) => {
        if (!alive) return;
        setMessages(p.messages);
        setTruncated(p.truncated);
      })
      .catch(() => {
        if (alive) setContentError(true);
      });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.sessionId, limit]);

  // 加载更早消息（活会话同款）：limit 翻倍整页重拉，锚定原顶部消息不甩屏
  const hasLoadMore =
    messages !== null && !contentError && (messages.length >= limit || truncated) && limit < MAX_LIMIT;

  const loadMore = useCallback(() => {
    const first = messages?.[0];
    anchorSeqRef.current = first ? first.seq : null;
    setLimit((l) => Math.min(l * 2, MAX_LIMIT));
  }, [messages]);

  // 折叠判定（活会话总结模式同款，归档恒总结模式）：过程消息可折叠；
  // assistant 只直显最后一条总结（更早 assistant 折叠）
  const lastAssistantSeq = useMemo(() => {
    if (messages === null) return null;
    let last: number | null = null;
    for (const m of messages) {
      if (m.kind === "assistant") last = m.seq;
    }
    return last;
  }, [messages]);

  const isToggleable = useCallback(
    (m: SessionMessage) =>
      isProcessKind(m.kind) || (m.kind === "assistant" && m.seq !== lastAssistantSeq),
    [lastAssistantSeq],
  );

  const isCollapsed = useCallback(
    (m: SessionMessage) => {
      const forced = expandedOverride.get(m.seq);
      return forced !== undefined ? forced : isToggleable(m);
    },
    [expandedOverride, isToggleable],
  );

  const toggleCollapsed = useCallback(
    (m: SessionMessage) => {
      setExpandedOverride((prev) => {
        const next = new Map(prev);
        next.set(m.seq, !isCollapsed(m));
        return next;
      });
    },
    [isCollapsed],
  );

  const toggleableMessages = useMemo(
    () => (messages !== null ? messages.filter((m) => isToggleable(m)) : []),
    [messages, isToggleable],
  );
  const collapsedCount = toggleableMessages.filter((m) => isCollapsed(m)).length;

  const expandAll = useCallback(() => {
    setExpandedOverride((prev) => {
      const next = new Map(prev);
      // 覆盖表值语义 = 强制折叠与否（false = 强制展开，见 isCollapsed）
      for (const m of toggleableMessages) next.set(m.seq, false);
      return next;
    });
  }, [toggleableMessages]);

  const collapseAll = useCallback(() => setExpandedOverride(new Map()), []);

  // 浮动钮显隐（onScroll 驱动；活会话同款阈值——贴底/贴顶阅读时是纯噪声）
  const handleAreaScroll = useCallback(() => {
    const el = areaRef.current;
    if (!el) return;
    setShowJumpTop(el.scrollTop > JUMP_SHOW_THRESHOLD_PX);
    setShowJumpBottom(el.scrollHeight - el.scrollTop - el.clientHeight > JUMP_SHOW_THRESHOLD_PX);
  }, []);

  const jumpToTop = useCallback(() => {
    const el = areaRef.current;
    if (!el) return;
    el.scrollTop = 0;
    setShowJumpTop(false);
  }, []);

  const jumpToBottom = useCallback(() => {
    const el = areaRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setShowJumpBottom(false);
  }, []);

  // 滚动定位：有锚（加载更早后）锚回原顶部消息；无锚（首次进入）落底
  useEffect(() => {
    const el = areaRef.current;
    if (messages === null || el === null) return;
    if (anchorSeqRef.current !== null) {
      const target = el.querySelector(`[data-testid="msg-${anchorSeqRef.current}"]`);
      anchorSeqRef.current = null;
      if (target) {
        target.scrollIntoView({ block: "start" });
        return;
      }
    }
    el.scrollTop = el.scrollHeight;
  }, [messages]);

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
    <div className="mx-auto flex h-dvh max-w-3xl flex-col px-4">
      <header className="flex shrink-0 items-center gap-2 pt-4">
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
      <p className="mt-1 shrink-0 text-xs text-slate-500">
        {chipLabel(session.agentType)} · {session.projectPath} · {statusLabel} ·{" "}
        {formatRelativeTime(session.lastSeenAt, now)}
        {session.hiddenAlive ? "活跃" : "结束"}{" "}
        {session.hiddenAlive && (
          <span
            data-testid="hidden-alive-badge"
            className="rounded-full bg-green-100 px-1.5 py-0.5 text-[10px] text-green-700 dark:bg-green-900/40 dark:text-green-300"
          >
            未结束 · 看板已隐藏
          </span>
        )}
      </p>

      {/* 窗口截断诚实提示（到顶 ≠ 全会话顶：更早内容未加载） */}
      {truncated && messages !== null && (
        <p
          data-testid="archive-truncated"
          className="mt-2 shrink-0 rounded-lg bg-amber-50 px-3 py-1.5 text-center text-xs text-amber-700 dark:bg-amber-950/40 dark:text-amber-400"
        >
          内容较长，仅显示最近 {ARCHIVE_PAGE_LIMIT} 条
        </p>
      )}

      {/* 折叠提示行（活会话 Bug 8 同款）：有可折叠内容即出现（不看当前折叠数——
          全部展开后「收起」仍要可达），计数如实显示 */}
      {toggleableMessages.length > 0 && (
        <div
          data-testid="archive-fold-banner"
          className="mt-2 flex shrink-0 items-center justify-between rounded-lg bg-slate-50 px-3 py-1.5 text-xs text-slate-500 dark:bg-slate-900"
        >
          <span>{collapsedCount} 条过程内容已折叠</span>
          <span className="flex gap-2">
            <button type="button" data-testid="expand-all" className="underline" onClick={expandAll}>
              展开全部
            </button>
            <button
              type="button"
              data-testid="collapse-all"
              className="underline"
              onClick={collapseAll}
            >
              收起
            </button>
          </span>
        </div>
      )}

      {/* 消息滚动区（活会话 MessageScrollArea 同款形态）：wrapper 持 relative，
          浮动按钮放 wrapper 才不随内容滚走 */}
      <div className="relative mt-2 min-h-0 flex-1">
        <div
          ref={areaRef}
          data-testid="message-area"
          className="h-full overflow-y-auto pr-1"
          onScroll={handleAreaScroll}
        >
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
          {/* 加载更早消息（体验批二，活会话同款）：条数达 limit 或 truncated 即提供 */}
          {hasLoadMore && (
            <button
              type="button"
              data-testid="load-more"
              onClick={loadMore}
              className="mx-auto mb-2 block rounded-lg border border-slate-200 px-3 py-1.5 text-xs text-slate-600 hover:bg-slate-100 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-800"
            >
              加载更早消息
            </button>
          )}
          <ul className="flex flex-col gap-2 pb-2">
            {messages?.map((m) => {
              const toggleable = isToggleable(m);
              const collapsed = isCollapsed(m);
              return (
                <li
                  key={m.seq}
                  data-testid={`msg-${m.seq}`}
                  data-kind={m.kind}
                  className="rounded-lg bg-slate-100 px-3 py-2 text-sm dark:bg-slate-900"
                >
                  {toggleable ? (
                    <>
                      <button
                        type="button"
                        data-testid={`msg-${m.seq}-toggle`}
                        aria-expanded={!collapsed}
                        onClick={() => toggleCollapsed(m)}
                        className="-mx-1 flex w-[calc(100%+8px)] items-center gap-1 rounded-lg px-1 py-0.5 text-left text-xs text-slate-500 hover:bg-slate-200/60 dark:text-slate-400 dark:hover:bg-slate-800"
                      >
                        {collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
                        <span className="truncate">{collapsedLabel(m)}</span>
                      </button>
                      {!collapsed && (
                        <div className="mt-1 whitespace-pre-wrap break-words">{m.content}</div>
                      )}
                    </>
                  ) : (
                    <div className="whitespace-pre-wrap break-words">
                      <span className="mr-2 text-xs text-slate-400">{m.role}</span>
                      {m.content}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
        {showJumpTop && (
          <button
            type="button"
            data-testid="jump-to-top"
            aria-label="跳到顶部"
            title="跳到顶部"
            onClick={jumpToTop}
            className="absolute right-2 top-2 z-10 rounded-full border border-slate-200 bg-white p-2 text-slate-600 shadow-md hover:bg-slate-100 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            <ArrowUpToLine size={16} />
          </button>
        )}
        {showJumpBottom && (
          <button
            type="button"
            data-testid="jump-to-bottom"
            aria-label="跳到底部"
            title="跳到底部"
            onClick={jumpToBottom}
            className="absolute right-2 bottom-2 z-10 rounded-full border border-slate-200 bg-white p-2 text-slate-600 shadow-md hover:bg-slate-100 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300 dark:hover:bg-slate-800"
          >
            <ArrowDownToLine size={16} />
          </button>
        )}
      </div>

      {/* 底部动作区（体验批二）：软归档活会话=「移回看板」；真死归档=「打开」（主，上）
          +「移除」（次，下） */}
      <div className="shrink-0 px-1 pt-3">
        {session.hiddenAlive ? (
          <>
            <button
              type="button"
              data-testid="unhide-session"
              onClick={() => {
                setUnhideError(false);
                unhideSession(session.sessionId)
                  .then(onBack)
                  .catch(() => setUnhideError(true));
              }}
              className="w-full rounded-lg bg-blue-600 px-3 py-2 text-sm text-white hover:bg-blue-700"
            >
              移回看板
            </button>
            {unhideError && (
              <p className="mt-1 text-center text-xs text-red-600">操作失败，请重试</p>
            )}
          </>
        ) : (
          <>
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
          </>
        )}
      </div>

      {!session.hiddenAlive && (
        <div className="shrink-0 pb-6 pt-2">
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
            className="w-full rounded-lg border border-red-300 px-3 py-2 text-sm text-red-600 hover:bg-red-50 dark:border-red-900 dark:hover:bg-red-950/40"
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
      )}
    </div>
  );
}
