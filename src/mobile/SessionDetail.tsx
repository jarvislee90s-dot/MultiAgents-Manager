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
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { ArrowLeft, ChevronDown, ChevronRight, PanelLeft, RotateCw } from "lucide-react";
import BookmarkBar from "./BookmarkBar";
import FilePanel from "./FilePanel";
import FilePreview from "./FilePreview";
import { type PreviewMode } from "./PreviewModeSwitcher";
import SplitHandle from "./SplitHandle";
import {
  ApiError,
  fetchSessionFiles,
  fetchSessionMessages,
  type SessionFileEntry,
  type SessionMessage,
} from "./api";
import { STATUS_DOT_COLOR, TOOL_LABELS } from "./board-logic";
import {
  addBookmark,
  bookmarkPreview,
  clearBookmarks,
  ensureBootId,
  listBookmarks,
  messageAnchor,
  removeBookmark,
  restoreBookmarks,
  BOOKMARK_LIMIT,
  type Bookmark,
} from "./bookmarks";
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

/** 预览侧栏状态（M3+ 辨识联合）：
 *  - list：文件面板（聚合列表，用户裁决 4/6）；
 *  - file：单文件预览；backToList 标记来源（从面板进入 → 显示返回按钮，
 *    从消息正文链接进入 → 无返回按钮，行为不变） */
type PreviewState =
  | { view: "list"; mode: PreviewMode }
  | { view: "file"; path: string; mode: PreviewMode; backToList: boolean };

/** 面板默认追溯档位（首屏数据源，与详情页默认 limit 同标尺） */
const FILE_DEFAULT_SCOPE = 200;

/** 字号档位（2026-09-16 用户裁决）：只作用于消息正文与文件预览内容，
 *  UI 与页头不受影响（mobile.css 的 [data-font-scale] 变量覆写） */
const FONT_SCALES = [0.5, 0.75, 1, 1.25] as const;

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
  // 该会话涉及的文件（/session-files 提取结果，M3+）：一份数据两用——
  // fileEntries 驱动文件面板列表，派生 Set 驱动正文路径链接化
  const [fileEntries, setFileEntries] = useState<SessionFileEntry[]>([]);
  const [fileTruncated, setFileTruncated] = useState(false);
  // 追溯档位（M3+ 用户裁决 3）：面板三档 200/500/1000，切档重拉
  const [fileScope, setFileScope] = useState<number>(FILE_DEFAULT_SCOPE);
  const [fileLoading, setFileLoading] = useState(false);
  // 字号档位（默认 100%）
  const [fontScale, setFontScale] = useState<number>(1);
  // 书签表（M3+）：初值从模块级 store 读——SessionDetail 卸载重挂后仍恢复
  const [bookmarks, setBookmarks] = useState<Bookmark[]>(() => listBookmarks(session.id));
  // 跳转失败提示（书签指向更早范围，当前窗口内找不到）
  const [bookmarkJumpMiss, setBookmarkJumpMiss] = useState(false);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  // 文件栏占比（可拖分隔条，需求 2026-09-16）：两形态各自保留用户拖出的比例，
  // 初值 0.5（对半分，与旧版 h-1/2 / w-1/2 观感一致）
  const [fileRatioV, setFileRatioV] = useState(0.5);
  const [fileRatioH, setFileRatioH] = useState(0.5);
  // 分屏容器 ref：拖动换算的尺寸来源（SplitHandle 内取 getBoundingClientRect）
  const splitRef = useRef<HTMLDivElement>(null);
  // 消息区 ref（滚动到底 / 加载更早的位置锚定）
  const messageAreaRef = useRef<HTMLDivElement>(null);
  // 待回补的滚动锚（加载更早前记录；非 null 表示下次数据落地要做位置补偿）
  const pendingScrollRef = useRef<{ prevHeight: number; prevTop: number } | null>(null);
  // 当前形态对应的占比与写回口（横向/纵向各记一份，来回切换不丢用户拖出的比例）
  const fileRatio = preview?.mode === "split-h" ? fileRatioH : fileRatioV;
  const setFileRatio = preview?.mode === "split-h" ? setFileRatioH : setFileRatioV;

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

  // 文件表拉取（M3+）：挂载 / 切档时重拉。挂载那次（scope=200）即面板首次打开
  // 复用的数据（一次拉取两用，用户裁决 3 的附带口径）；失败已由 api 层静默降级
  useEffect(() => {
    let alive = true;
    setFileLoading(true);
    fetchSessionFiles(session.agentType, session.id, fileScope)
      .then((pg) => {
        if (!alive) return;
        setFileEntries(pg.files);
        setFileTruncated(pg.truncated);
      })
      .finally(() => {
        if (alive) setFileLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.id, fileScope]);

  // 链接化 Set（派生自面板条目，单一数据源）：正文路径匹配用
  const files = useMemo(() => new Set(fileEntries.map((e) => e.path)), [fileEntries]);

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

  /** 宽屏判定（matchMedia 防御式访问；jsdom / 隐私模式可能缺失，theme.ts 同款口径） */
  const isWideViewport = useCallback((): boolean => {
    try {
      return window.matchMedia?.("(min-width: 768px)")?.matches ?? false;
    } catch {
      return false; // matchMedia 缺失/异常 → 窄屏语义（Task 8 原裁决）
    }
  }, []);

  // 点文件链接（消息正文）→ 预览。Bug 2 顺手项（spec P9「按屏幕宽度自适应」）：
  // ≥768px 分屏（上对话下文件）、<768px 全屏；手动切换随时覆盖该默认值。
  // backToList=false（从正文进入，无返回列表按钮，既有行为不变）
  const openFile = useCallback(
    (path: string) => {
      setPreview({
        view: "file",
        path,
        mode: isWideViewport() ? "split" : "fullscreen",
        backToList: false,
      });
    },
    [isWideViewport]
  );

  // 从文件面板进入单文件预览（M3+）：backToList=true → 预览页头显示返回按钮；
  // 布局沿用当前 mode（往返保持，用户裁决「布局保持」）
  const openFileFromList = useCallback((path: string) => {
    setPreview((p) => ({
      view: "file",
      path,
      mode: p?.mode ?? "fullscreen",
      backToList: true,
    }));
  }, []);

  // 页头面板入口（M3+）：宽屏默认 split-h（列表是行集，右侧整列纵向空间大）、
  // 窄屏 fullscreen（用户裁决：面板默认布局口径）。
  // 面板开启态 = 当前是 list 视图（页头按钮高亮依据；文件预览态不算——那是
  // 从列表或正文进入的下一层）；再点收回（ZCode 式排版，2026-09-16 用户裁决）
  const panelOpen = preview?.view === "list";
  const togglePanel = useCallback(() => {
    setPreview((p) =>
      p?.view === "list"
        ? null
        : { view: "list", mode: isWideViewport() ? "split-h" : "fullscreen" }
    );
  }, [isWideViewport]);

  const closePreview = useCallback(() => setPreview(null), []);

  // 从单文件预览返回列表（保持当前布局 mode）
  const backToList = useCallback(() => {
    setPreview((p) => ({ view: "list", mode: p?.mode ?? "fullscreen" }));
  }, []);

  // 布局切换（预览页头控件与面板共用同一状态出口）
  const changePreviewMode = useCallback((mode: PreviewMode) => {
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
          // md-body：markdown 排版层（Bug 5——preflight 拍平标题/列表的修复锚点）
          return <div className="md-body text-sm">{renderMarkdown(m.content)}</div>;
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

  // ---- 书签（M3+，2026-09-16 用户裁决）----
  // 恢复：拿到 MAM 进程 bootId 后从 localStorage 种回内存单例（刷新页面/
  // 卸载重挂均走此路径）；bootId 不一致（MAM 已重启）由 restore 内部清空
  useEffect(() => {
    let alive = true;
    void ensureBootId().then((boot) => {
      if (!alive || boot === null) return;
      restoreBookmarks(boot);
      setBookmarks(listBookmarks(session.id));
    });
    return () => {
      alive = false;
    };
  }, [session.id]);

  // 取锚：当前视口顶部可见的那条消息。消息 li 挂 data-seq，取第一个
  // 「底边越过容器顶边」的条目即视口首条
  const topVisibleMessage = useCallback((): SessionMessage | null => {
    const el = messageAreaRef.current;
    if (!el || messages === null) return null;
    const areaTop = el.getBoundingClientRect().top;
    const items = el.querySelectorAll<HTMLElement>("[data-seq]");
    for (const it of items) {
      if (it.getBoundingClientRect().bottom > areaTop) {
        const seq = Number(it.dataset.seq);
        return messages.find((m) => m.seq === seq) ?? null;
      }
    }
    return null;
  }, [messages]);

  const handleAddBookmark = useCallback(
    (color: string) => {
      const m = topVisibleMessage();
      if (m === null) return;
      addBookmark(session.id, {
        color,
        seq: m.seq,
        anchor: messageAnchor(m),
        preview: bookmarkPreview(m.content),
      });
      setBookmarks(listBookmarks(session.id));
      setBookmarkJumpMiss(false);
    },
    [session.id, topVisibleMessage]
  );

  const handleRemoveBookmark = useCallback(
    (color: string) => {
      removeBookmark(session.id, color);
      setBookmarks(listBookmarks(session.id));
    },
    [session.id]
  );

  const handleClearBookmarks = useCallback(() => {
    clearBookmarks(session.id);
    setBookmarks([]);
  }, [session.id]);

  // 跳转：按指纹在当前窗口查回消息 → seq → scrollIntoView（block:start 落在视口顶部）。
  // 查不到（书签指向已滑出窗口的更早消息）→ 提示先加载更早消息
  const handleJumpBookmark = useCallback(
    (anchor: string) => {
      if (messages === null) return;
      const target = messages.find((m) => messageAnchor(m) === anchor);
      if (!target) {
        setBookmarkJumpMiss(true);
        return;
      }
      setBookmarkJumpMiss(false);
      const el = messageAreaRef.current?.querySelector<HTMLElement>(`[data-seq="${target.seq}"]`);
      el?.scrollIntoView({ block: "start" });
    },
    [messages]
  );

  // 进入详情默认滚到最底部（最新消息在下方，用户裁决 2026-09-16）；且
  // 「加载更早消息」重拉后**保持原阅读位置**——记录重拉前的滚动高度差，
  // 新内容（更早消息）插在顶部后把差值补回去，视线不跳。
  // 依赖 messages（而非 limit）：只在数据真正落地后执行一次对齐
  useEffect(() => {
    const el = messageAreaRef.current;
    if (!el || messages === null) return;
    const pending = pendingScrollRef.current;
    if (pending !== null) {
      // 回补：新高度 - 旧高度 = 顶部插入量，往下偏移同等距离保持视线
      const inserted = el.scrollHeight - pending.prevHeight;
      el.scrollTop = pending.prevTop + Math.max(0, inserted);
      pendingScrollRef.current = null;
      return;
    }
    el.scrollTop = el.scrollHeight; // 首次（或手动刷新后）落到底部
  }, [messages]);

  // 是否渲染折叠切换头：过程消息 + 总结模式下的「更早 assistant」；
  // 最终 assistant 总结直显正文（不给「更早的回复」头）
  const isToggleable = (m: SessionMessage) =>
    m.kind === "thinking" ||
    m.kind === "tool-call" ||
    m.kind === "tool-result" ||
    (isSummary && m.kind === "assistant" && m.seq !== lastAssistantSeq);

  // Bug 8（M3 验收）：总结模式折叠提示。折叠数 = 当前被折叠的可折叠条数——
  // 70 条过程消息被静默折叠会被误读为「内容被截」，顶部提示行 + 展开/收起全部
  // 消除歧义（折叠本身是正确行为，不改动折叠语义）。
  // useMemo 保持引用稳定（expandAll 依赖它，逐渲染新建会让 useCallback 每拍失效）
  const toggleableMessages = useMemo(
    () => (messages !== null ? messages.filter(isToggleable) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- isToggleable 是渲染期纯函数（依赖 isSummary/lastAssistantSeq，已在下行列出）
    [messages, isSummary, lastAssistantSeq]
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

  const collapseAll = useCallback(() => {
    // 清空覆盖表 = 回到默认折叠语义（总结模式折叠规则的单点来源仍是 isCollapsed）
    setExpandedOverride(new Map());
  }, []);

  // Bug 1（M3 验收）：条数到顶（length >= limit）**或**后端报告头部截断（truncated，
  // 胖单行吃满字节窗的会话条数恒小于 limit）任一成立即提供「加载更早消息」
  const hasLoadMore =
    messages !== null && !error && (messages.length >= limit || truncated) && limit < MAX_LIMIT;

  // 锚 → 书签查找表（消息角标用；渲染期 O(1) 直读，避免每条消息 find）
  const bookmarkByAnchor = useMemo(() => new Map(bookmarks.map((b) => [b.anchor, b])), [bookmarks]);

  // 书签条（两个布局分支共用同一份 JSX）
  const bookmarkBar = (
    <BookmarkBar
      bookmarks={bookmarks}
      atLimit={bookmarks.length >= BOOKMARK_LIMIT}
      onAdd={handleAddBookmark}
      onJump={handleJumpBookmark}
      onRemove={handleRemoveBookmark}
      onClear={handleClearBookmarks}
    />
  );

  // 消息区（split 布局复用同一份 JSX）
  const messageArea = (
    <>
      {/* 总结模式提示行（Bug 8）：仅总结模式且有可折叠过程消息时出现 */}
      {isSummary && toggleableMessages.length > 0 && (
        <div
          data-testid="summary-banner"
          className="mb-2 flex items-center gap-2 rounded-lg bg-sky-500/10 px-3 py-2 text-xs text-sky-700 dark:text-sky-400"
        >
          <span>总结模式 · 已折叠 {collapsedCount} 条过程消息</span>
          <span className="ml-auto flex shrink-0 gap-1">
            {collapsedCount > 0 && (
              <button
                type="button"
                data-testid="expand-all"
                onClick={expandAll}
                className="rounded-full bg-sky-500/20 px-2 py-0.5 text-xs text-sky-700 dark:bg-sky-400/20 dark:text-sky-300"
              >
                展开全部
              </button>
            )}
            {collapsedCount < toggleableMessages.length && (
              <button
                type="button"
                data-testid="collapse-all"
                onClick={collapseAll}
                className="rounded-full bg-slate-200 px-2 py-0.5 text-xs text-slate-600 dark:bg-slate-800 dark:text-slate-300"
              >
                收起全部
              </button>
            )}
          </span>
        </div>
      )}
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
      {/* 书签跳转失败提示（M3+）：书签指向的消息不在当前窗口 */}
      {bookmarkJumpMiss && (
        <p
          data-testid="bookmark-jump-miss"
          className="mb-2 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400"
        >
          该书签在更早的范围内，请先点上方「加载更早消息」
        </p>
      )}
      {/* 「加载更早消息」置于列表**最上方**（2026-09-16 用户裁决）：语义是
          「往前翻到头再加一段更早的」，与阅读方向一致；此前放在列表末尾，
          与「更早」的空间直觉相反 */}
      {hasLoadMore && (
        <div className="pt-1 pb-3 text-center">
          <button
            type="button"
            data-testid="load-more"
            onClick={() => {
              const el = messageAreaRef.current;
              if (el) {
                pendingScrollRef.current = { prevHeight: el.scrollHeight, prevTop: el.scrollTop };
              }
              setLimit((l) => Math.min(l + PAGE_LIMIT, MAX_LIMIT));
            }}
            className="rounded-full bg-slate-200 px-4 py-1.5 text-xs text-slate-700 dark:bg-slate-800 dark:text-slate-300"
          >
            {loading ? "加载中…" : "加载更早消息"}
          </button>
        </div>
      )}
      {messages !== null && messages.length > 0 && (
        <ul className="space-y-2 pb-4">
          {messages.map((m) => {
            const collapsed = isCollapsed(m);
            const toggleable = isToggleable(m);
            const isUser = m.kind === "user";
            const msgBookmark = bookmarkByAnchor.get(messageAnchor(m));
            return (
              <li
                key={m.seq}
                data-testid={`msg-${m.seq}`}
                data-kind={m.kind}
                data-seq={m.seq}
                className={
                  isUser ? "relative flex flex-col items-end" : "relative flex flex-col items-start"
                }
              >
                {/* 书签角标（M3+）：该消息命中书签时在气泡左上显示色点 */}
                {msgBookmark && (
                  <span
                    data-testid={`msg-bookmark-${m.seq}`}
                    title={`书签：${msgBookmark.preview}`}
                    className="absolute -top-1 -left-1 h-2.5 w-2.5 rounded-full ring-2 ring-white dark:ring-slate-950"
                    style={{ backgroundColor: msgBookmark.color }}
                  />
                )}
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
        {/* 文件面板入口（M3+；ZCode 式排版，2026-09-16 用户裁决）：页头恒可见
            （不依赖预览是否打开）；开启态高亮 + tooltip 随状态切换 + 再点收回 */}
        <button
          type="button"
          data-testid="file-panel-button"
          aria-label="文件面板"
          aria-pressed={panelOpen}
          title={panelOpen ? "收起文件面板" : "打开文件面板"}
          onClick={togglePanel}
          className={`shrink-0 rounded-full p-1 ${
            panelOpen
              ? "bg-slate-200 text-slate-900 dark:bg-slate-800 dark:text-slate-100"
              : "text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
          }`}
        >
          <PanelLeft size={16} />
        </button>
        {/* 字号档位（2026-09-16 用户裁决）：面板按钮旁，作用于正文与预览内容 */}
        <select
          data-testid="font-scale-select"
          aria-label="正文字号"
          title="正文字号"
          value={String(fontScale)}
          onChange={(e) => setFontScale(Number(e.target.value))}
          className="shrink-0 rounded-md border border-slate-200 bg-transparent px-1 py-0.5 text-xs text-slate-600 dark:border-slate-700 dark:text-slate-300"
        >
          {FONT_SCALES.map((v) => (
            <option key={v} value={String(v)}>
              {Math.round(v * 100)}%
            </option>
          ))}
        </select>
        {/* 布局切换器唯一实例在预览侧栏页头（FilePreview / FilePanel 内，
            2026-09-16 用户裁决：两处重复出现占用页面空间，只保留贴近文件的那份） */}
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

      {/* 预览侧栏（M3+ 两视图共用）：split/split-h 内联分屏（可拖分隔条），
          fullscreen 全屏浮层；list 视图渲染 FilePanel，file 视图渲染 FilePreview */}
      {preview?.mode === "split" || preview?.mode === "split-h" ? (
        <div
          ref={splitRef}
          data-testid="split-container"
          data-split={preview.mode}
          className={`flex min-h-0 flex-1 ${preview.mode === "split-h" ? "flex-row" : "flex-col"}`}
        >
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {bookmarkBar}
            <div
              ref={messageAreaRef}
              data-testid="message-area"
              data-font-scale={fontScale}
              className="min-h-0 flex-1 overflow-y-auto px-3 pt-3"
            >
              {messageArea}
            </div>
          </div>
          <SplitHandle
            orientation={preview.mode === "split-h" ? "horizontal" : "vertical"}
            ratio={fileRatio}
            onRatioChange={setFileRatio}
            containerRef={splitRef}
          />
          <div
            data-testid="split-file-pane"
            className="shrink-0 overflow-hidden"
            style={
              preview.mode === "split-h"
                ? { width: `${fileRatio * 100}%` }
                : { height: `${fileRatio * 100}%` }
            }
          >
            {preview.view === "list" ? (
              <div
                data-testid="preview-shell"
                data-view="list"
                data-mode={preview.mode}
                className="h-full"
              >
                <FilePanel
                  entries={fileEntries}
                  truncated={fileTruncated}
                  scope={fileScope}
                  loading={fileLoading}
                  mode={preview.mode}
                  onScopeChange={setFileScope}
                  onOpenFile={openFileFromList}
                  onModeChange={changePreviewMode}
                  fontScale={fontScale}
                  onClose={closePreview}
                />
              </div>
            ) : (
              <div
                data-testid="preview-shell"
                data-view="file"
                data-mode={preview.mode}
                className="h-full"
              >
                <FilePreview
                  session={session}
                  filePath={preview.path}
                  mode={preview.mode}
                  onModeChange={changePreviewMode}
                  fontScale={fontScale}
                  onBack={preview.backToList ? backToList : undefined}
                  onClose={closePreview}
                />
              </div>
            )}
          </div>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          {bookmarkBar}
          <div
            ref={messageAreaRef}
            data-testid="message-area"
            data-font-scale={fontScale}
            className="min-h-0 flex-1 overflow-y-auto px-3 pt-3"
          >
            {messageArea}
          </div>
        </div>
      )}

      {/* fullscreen = 全屏浮层（覆盖对话，关闭回到原位——列表状态由本组件持有） */}
      {preview?.mode === "fullscreen" && (
        <div className="fixed inset-0 z-50 bg-white dark:bg-slate-950">
          <div
            data-testid="preview-shell"
            data-view={preview.view}
            data-mode="fullscreen"
            className="h-full"
          >
            {preview.view === "list" ? (
              <FilePanel
                entries={fileEntries}
                truncated={fileTruncated}
                scope={fileScope}
                loading={fileLoading}
                mode="fullscreen"
                onScopeChange={setFileScope}
                onOpenFile={openFileFromList}
                onModeChange={changePreviewMode}
                fontScale={fontScale}
                onClose={closePreview}
              />
            ) : (
              <FilePreview
                session={session}
                filePath={preview.path}
                mode="fullscreen"
                onModeChange={changePreviewMode}
                fontScale={fontScale}
                onBack={preview.backToList ? backToList : undefined}
                onClose={closePreview}
              />
            )}
          </div>
        </div>
      )}
    </div>
  );
}
