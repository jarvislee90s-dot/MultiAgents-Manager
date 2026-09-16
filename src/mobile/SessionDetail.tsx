// ZCode 式会话详情页（M3 Task 8，P9 前端）：
// - 运行中：thinking / tool-call 默认折叠可展开（wire collapsed 字段语义），
//   assistant 正文直接渲染（markdown + 代码高亮）；
// - 会话 status ∈ idle/finished（总结模式）：过程消息（thinking / tool-call /
//   tool-result）与非最终 assistant 一律自动折叠，只显最后一条 assistant 总结，
//   均可手动展开；
// - 消息正文中的已知文件路径（/session-files 提取结果）渲染为可点链接 → 文件预览；
// - 「加载更早消息」按钮以更大 limit 整页重拉（Task 8 裁决：M3 用按钮替代无限
//   滚动，YAGNI——避免滚动位置管理复杂度）；
// - 不自动轮询（M3 范围裁决：SSE transition 不驱动详情页），页头刷新按钮手动重拉。
import { useCallback, useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { ArrowLeft, ChevronDown, ChevronRight, Columns2, Maximize2, RotateCw } from "lucide-react";
import FilePreview from "./FilePreview";
import { ApiError, fetchSessionFiles, fetchSessionMessages, type SessionMessage } from "./api";
import { STATUS_DOT_COLOR, TOOL_LABELS } from "./board-logic";
import type { Session } from "@/types/session";

/** 单次拉取条数（与后端 session-messages 默认 limit 一致） */
const PAGE_LIMIT = 200;
/** 后端 limit clamp 上限：到达后不再提供「加载更早消息」 */
const MAX_LIMIT = 1000;

interface SessionDetailProps {
  /** 完整会话对象（Task 8 裁决：详情页需要 status 判定自动折叠、projectName 页头、
   *  agentType；比传 id+agentType 再反查简单） */
  session: Session;
  /** 返回看板 */
  onBack: () => void;
}

interface PreviewState {
  path: string;
  mode: "split" | "fullscreen";
}

/** 拉取失败态：status=null 表示网络层异常（无 HTTP 状态可读） */
interface LoadError {
  status: number | null;
}

// ============================================================
// 纯函数小件（组件外，独立可测）
// ============================================================

/** 折叠行的摘要标签 */
function collapsedLabel(m: SessionMessage): string {
  switch (m.kind) {
    case "thinking":
      return "思考过程";
    case "tool-call":
      return m.toolName ? `调用 ${m.toolName}` : "工具调用";
    case "tool-result":
      return "工具结果";
    case "assistant":
      return "更早的回复";
    default:
      return "已折叠消息";
  }
}

/** 已知路径按长度降序（最长优先替换：路径互为前缀时不被短路径截断） */
function sortedPaths(files: Set<string>): string[] {
  return [...files].sort((a, b) => b.length - a.length);
}

/** markdown 正文链接化预处理：把出现的已知路径替换为 `#file:` 内链，
 *  再由 components.a 拦截渲染成可点按钮。路径含 markdown 特殊字符（[]()）时
 *  该处替换可能不成链（保持原样文本，M3 接受） */
function linkifyMarkdown(text: string, files: string[]): string {
  let out = text;
  for (const p of files) {
    if (!p) continue;
    out = out.split(p).join(`[${p}](#file:${encodeURIComponent(p)})`);
  }
  return out;
}

/** 纯文本分段链接化：按已知路径把正文切成文本段与文件段（thinking / tool-result 用） */
function linkifySegments(
  text: string,
  files: string[]
): Array<{ type: "text" | "file"; value: string }> {
  let segments: Array<{ type: "text" | "file"; value: string }> = [{ type: "text", value: text }];
  for (const p of files) {
    if (!p) continue;
    const next: Array<{ type: "text" | "file"; value: string }> = [];
    for (const seg of segments) {
      if (seg.type !== "text" || !seg.value.includes(p)) {
        next.push(seg);
        continue;
      }
      const parts = seg.value.split(p);
      parts.forEach((part, i) => {
        if (part) next.push({ type: "text", value: part });
        if (i < parts.length - 1) next.push({ type: "file", value: p });
      });
    }
    segments = next;
  }
  return segments;
}

// ============================================================
// 组件
// ============================================================

export default function SessionDetail({ session, onBack }: SessionDetailProps) {
  const [messages, setMessages] = useState<SessionMessage[] | null>(null);
  // 头部截断标记（Bug 1，M3 验收）：后端字节窗切掉文件头时为 true——
  // 即使本页条数 < limit 也存在更早内容，按钮必须可见
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<LoadError | null>(null);
  const [loading, setLoading] = useState(true);
  // 「加载更早消息」：limit 递增整页重拉（后端取文件序尾部 limit 条）
  const [limit, setLimit] = useState(PAGE_LIMIT);
  // 手动刷新信号（不自动轮询，见文件头注释）
  const [refreshTick, setRefreshTick] = useState(0);
  // 折叠覆盖表：seq → 强制折叠/展开；缺省走默认折叠语义（见 isCollapsed）
  const [expandedOverride, setExpandedOverride] = useState<Map<number, boolean>>(new Map());
  // 该会话涉及的文件路径（/session-files 提取结果，链接化数据源）
  const [files, setFiles] = useState<Set<string>>(new Set());
  const [preview, setPreview] = useState<PreviewState | null>(null);

  // 总结模式（P9）：会话已结束/空闲 → 只显最后 assistant 总结，过程消息自动折叠
  const isSummary = session.status === "idle" || session.status === "finished";

  // 消息流拉取：挂载 / limit 变化 / 手动刷新时重拉（整页替换；M3 不做增量追加
  // 与滚动位置保持——「加载更多」按钮替代无限滚动的裁决即含此简化）
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError(null);
    fetchSessionMessages(session.agentType, session.id, limit)
      .then((pg) => {
        if (!alive) return;
        setMessages(pg.messages);
        setTruncated(pg.truncated);
      })
      .catch((e: unknown) => {
        if (alive) setError({ status: e instanceof ApiError ? e.status : null });
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.id, limit, refreshTick]);

  // 文件路径表：挂载拉一次（提取要读一遍会话消息流，属重活；失败已在 api 层
  // 静默降级为空表——正文照常渲染，只是没有文件链接）
  useEffect(() => {
    let alive = true;
    fetchSessionFiles(session.agentType, session.id).then((list) => {
      if (alive) setFiles(new Set(list));
    });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.id]);

  // 总结模式下的「最后 assistant 总结」：倒数第一条 assistant
  const lastAssistantSeq = useMemo(() => {
    if (!messages) return null;
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      if (messages[i].kind === "assistant") return messages[i].seq;
    }
    return null;
  }, [messages]);

  const isCollapsed = useCallback(
    (m: SessionMessage): boolean => {
      const forced = expandedOverride.get(m.seq);
      if (forced !== undefined) return forced; // 手动展开/再折叠优先
      if (isSummary) {
        // 总结模式：user 直显；assistant 只显最后总结；过程消息全折叠
        if (m.kind === "user") return false;
        if (m.kind === "assistant") return m.seq !== lastAssistantSeq;
        return true;
      }
      // 运行中：保持 wire collapsed 字段语义（thinking / tool-call 恒折叠）
      return m.collapsed;
    },
    [expandedOverride, isSummary, lastAssistantSeq]
  );

  const toggleCollapsed = useCallback(
    (m: SessionMessage) => {
      setExpandedOverride((prev) => {
        const next = new Map(prev);
        next.set(m.seq, !isCollapsed(m));
        return next;
      });
    },
    [isCollapsed]
  );

  // 点文件链接 → 预览。Bug 2 顺手项（spec P9「按屏幕宽度自适应」）：≥768px 分屏
  // （上对话下文件）、<768px 全屏；手动切换（页头 / 预览头控件）随时覆盖该默认值。
  // matchMedia 防御式访问（jsdom / 隐私模式可能缺失，theme.ts 同款口径）
  const openFile = useCallback((path: string) => {
    let wide = false;
    try {
      wide = window.matchMedia?.("(min-width: 768px)")?.matches ?? false;
    } catch {
      wide = false; // matchMedia 缺失/异常 → 窄屏默认全屏（Task 8 原裁决）
    }
    setPreview({ path, mode: wide ? "split" : "fullscreen" });
  }, []);

  const closePreview = useCallback(() => setPreview(null), []);

  // 布局切换（页头切换器与 FilePreview 页头控件共用同一状态出口）
  const changePreviewMode = useCallback((mode: "split" | "fullscreen") => {
    setPreview((p) => (p ? { ...p, mode } : p));
  }, []);

  const sortedFiles = useMemo(() => sortedPaths(files), [files]);

  // markdown 渲染（assistant / user 正文）：remark-gfm 表格/删除线 + rehype-highlight
  // 代码块高亮（主题色由 mobile.css 双态内联，见其注释）；#file: 内链拦截为文件按钮
  const renderMarkdown = useCallback(
    (text: string) => (
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={{
          a: ({ href, children }) => {
            if (href?.startsWith("#file:")) {
              const p = decodeURIComponent(href.slice("#file:".length));
              return (
                <button
                  type="button"
                  data-testid="file-link"
                  className="inline text-left break-all text-sky-700 underline underline-offset-2 dark:text-sky-400"
                  onClick={() => openFile(p)}
                >
                  {children}
                </button>
              );
            }
            return (
              <a href={href} target="_blank" rel="noreferrer">
                {children}
              </a>
            );
          },
        }}
      >
        {linkifyMarkdown(text, sortedFiles)}
      </ReactMarkdown>
    ),
    [openFile, sortedFiles]
  );

  // 纯文本链接化渲染（thinking / tool-result：正文常含路径，值得可点）
  const renderLinkifiedText = useCallback(
    (text: string) =>
      linkifySegments(text, sortedFiles).map((seg, i) =>
        seg.type === "text" ? (
          <span key={i}>{seg.value}</span>
        ) : (
          <button
            key={i}
            type="button"
            data-testid="file-link"
            className="break-all text-sky-700 underline underline-offset-2 dark:text-sky-400"
            onClick={() => openFile(seg.value)}
          >
            {seg.value}
          </button>
        )
      ),
    [openFile, sortedFiles]
  );

  const renderBody = useCallback(
    (m: SessionMessage) => {
      switch (m.kind) {
        case "assistant":
        case "user":
          return <div className="text-sm">{renderMarkdown(m.content)}</div>;
        case "thinking":
          return (
            <div className="text-xs break-words whitespace-pre-wrap text-slate-500 dark:text-slate-400">
              {renderLinkifiedText(m.content)}
            </div>
          );
        case "tool-result":
          return (
            <pre className="overflow-x-auto rounded-lg bg-slate-100 p-2 text-xs break-words whitespace-pre-wrap text-slate-700 dark:bg-slate-900 dark:text-slate-300">
              {renderLinkifiedText(m.content)}
            </pre>
          );
        case "tool-call":
          return (
            <div className="space-y-1">
              <p className="text-xs font-medium text-slate-700 dark:text-slate-300">
                {m.toolName ? `调用 ${m.toolName}` : "工具调用"}
              </p>
              {m.toolArgs && (
                <pre
                  data-testid={`tool-args-${m.seq}`}
                  className="overflow-x-auto rounded-lg bg-slate-100 p-2 text-xs text-slate-700 dark:bg-slate-900 dark:text-slate-300"
                >
                  {m.toolArgs}
                </pre>
              )}
            </div>
          );
        default:
          return <div className="text-sm">{m.content}</div>;
      }
    },
    [renderLinkifiedText, renderMarkdown]
  );

  const retry = useCallback(() => {
    setError(null);
    setRefreshTick((t) => t + 1);
  }, []);

  // 是否渲染折叠切换头：过程消息 + 总结模式下的「更早 assistant」；
  // 最终 assistant 总结直显正文（不给「更早的回复」头）
  const isToggleable = (m: SessionMessage) =>
    m.kind === "thinking" ||
    m.kind === "tool-call" ||
    m.kind === "tool-result" ||
    (isSummary && m.kind === "assistant" && m.seq !== lastAssistantSeq);

  // Bug 1（M3 验收）：条数到顶（length >= limit）**或**后端报告头部截断（truncated，
  // 胖单行吃满字节窗的会话条数恒小于 limit）任一成立即提供「加载更早消息」
  const hasLoadMore =
    messages !== null && !error && (messages.length >= limit || truncated) && limit < MAX_LIMIT;

  // 消息区（split 布局复用同一份 JSX）
  const messageArea = (
    <>
      {loading && messages === null && (
        <p className="py-16 text-center text-sm text-slate-500">加载中…</p>
      )}
      {error && (
        <div className="py-16 text-center">
          <p data-testid="detail-error" className="mb-3 text-sm text-rose-600 dark:text-rose-400">
            {error.status === 404 ? "无法读取该会话内容" : "加载失败，请检查网络后重试"}
          </p>
          <button
            type="button"
            data-testid="detail-retry"
            onClick={retry}
            className="rounded-full bg-slate-200 px-4 py-1.5 text-sm text-slate-700 dark:bg-slate-800 dark:text-slate-300"
          >
            重试
          </button>
        </div>
      )}
      {messages !== null && !error && messages.length === 0 && (
        <p className="py-16 text-center text-sm text-slate-500">暂无消息</p>
      )}
      {messages !== null && messages.length > 0 && (
        <ul className="space-y-2 pb-4">
          {messages.map((m) => {
            const collapsed = isCollapsed(m);
            const toggleable = isToggleable(m);
            const isUser = m.kind === "user";
            return (
              <li
                key={m.seq}
                data-testid={`msg-${m.seq}`}
                data-kind={m.kind}
                className={isUser ? "flex flex-col items-end" : "flex flex-col items-start"}
              >
                <div
                  className={
                    isUser
                      ? "max-w-[85%] rounded-2xl rounded-br-sm bg-sky-500/10 px-3 py-2"
                      : "w-full rounded-2xl rounded-bl-sm bg-slate-100 px-3 py-2 dark:bg-slate-900"
                  }
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
                      {!collapsed && <div className="mt-1">{renderBody(m)}</div>}
                    </>
                  ) : (
                    renderBody(m)
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {hasLoadMore && (
        <div className="pb-4 text-center">
          <button
            type="button"
            data-testid="load-more"
            onClick={() => setLimit((l) => Math.min(l + PAGE_LIMIT, MAX_LIMIT))}
            className="rounded-full bg-slate-200 px-4 py-1.5 text-xs text-slate-700 dark:bg-slate-800 dark:text-slate-300"
          >
            {loading ? "加载中…" : "加载更早消息"}
          </button>
        </div>
      )}
    </>
  );

  return (
    <div className="flex h-dvh flex-col bg-white text-slate-800 dark:bg-slate-950 dark:text-slate-200">
      <header className="flex shrink-0 items-center gap-2 border-b border-slate-200 px-3 py-2 dark:border-slate-800">
        <button
          type="button"
          data-testid="detail-back"
          aria-label="返回看板"
          onClick={onBack}
          className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
        >
          <ArrowLeft size={18} />
        </button>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold text-slate-900 dark:text-slate-100">
            {session.projectName}
          </span>
          <span className="block truncate text-xs text-slate-500 dark:text-slate-400">
            {TOOL_LABELS[session.agentType]}
            {session.title ? ` · ${session.title}` : ""}
          </span>
        </span>
        {/* 状态点（与看板卡片同语义：waiting 附加呼吸动画） */}
        <span
          className={`inline-block h-2.5 w-2.5 shrink-0 rounded-full ${STATUS_DOT_COLOR[session.status]} ${
            session.status === "waiting" ? "animate-pulse" : ""
          }`}
        />
        {/* 预览布局切换（仅预览打开时出现；Task 8 裁决：切换控件在 SessionDetail） */}
        {preview && (
          <span
            role="group"
            aria-label="预览布局"
            className="flex shrink-0 items-center rounded-full bg-slate-200 p-0.5 dark:bg-slate-800"
          >
            <button
              type="button"
              data-testid="preview-mode-split"
              aria-pressed={preview.mode === "split"}
              aria-label="分屏预览"
              onClick={() => setPreview((p) => (p ? { ...p, mode: "split" } : p))}
              className={`rounded-full p-1 ${
                preview.mode === "split"
                  ? "bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100"
                  : "text-slate-500 dark:text-slate-400"
              }`}
            >
              <Columns2 size={14} />
            </button>
            <button
              type="button"
              data-testid="preview-mode-fullscreen"
              aria-pressed={preview.mode === "fullscreen"}
              aria-label="全屏预览"
              onClick={() => setPreview((p) => (p ? { ...p, mode: "fullscreen" } : p))}
              className={`rounded-full p-1 ${
                preview.mode === "fullscreen"
                  ? "bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100"
                  : "text-slate-500 dark:text-slate-400"
              }`}
            >
              <Maximize2 size={14} />
            </button>
          </span>
        )}
        <button
          type="button"
          data-testid="detail-refresh"
          aria-label="刷新消息"
          onClick={retry}
          className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
        >
          <RotateCw size={15} />
        </button>
      </header>

      {/* split = 上对话下文件（纵向分屏，Task 8 裁决的手机现实） */}
      {preview?.mode === "split" ? (
        <div data-testid="split-container" className="flex min-h-0 flex-1 flex-col">
          <div className="min-h-0 flex-1 overflow-y-auto px-3 pt-3">{messageArea}</div>
          <div className="h-1/2 shrink-0 border-t border-slate-200 dark:border-slate-800">
            <FilePreview
              session={session}
              filePath={preview.path}
              mode="split"
              onModeChange={changePreviewMode}
              onClose={closePreview}
            />
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 pt-3">{messageArea}</div>
      )}

      {/* fullscreen = 全屏浮层（覆盖对话，关闭回到原位——列表状态由本组件持有） */}
      {preview?.mode === "fullscreen" && (
        <div className="fixed inset-0 z-50 bg-white dark:bg-slate-950">
          <FilePreview
            session={session}
            filePath={preview.path}
            mode="fullscreen"
            onModeChange={changePreviewMode}
            onClose={closePreview}
          />
        </div>
      )}
    </div>
  );
}
