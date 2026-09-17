// 文件预览浮窗（M3 Task 8，P9）：会话项目目录内文件的安全只读预览。
// - 图片 mime → <img>（同源 fetch blob → object URL，卸载/换文件时 revoke）；
// - markdown → ReactMarkdown 渲染；其他文本 → <pre><code> 语法高亮
//   （highlight.js/lib/common，与 rehype-highlight 共享同一份 lowlight/highlight.js
//   模块，不产生重复打包）；
// - 源码/渲染双态（M5 B4，线稿 seg）：仅 .md 与 .html 出现切换器——.md 源码态
//   回落源码高亮；.html 渲染态 = 沙箱 iframe（sandbox="allow-scripts"，无
//   same-origin，与看板数据隔离）；其余类型与图片不显示 seg；
// - 越界 / 超限 / 不存在对外一律 403（后端探测面最小化）→「无法预览该文件」错误态；
// - 手机现实（Task 8 裁决）：split = 上对话下文件（纵向分屏，由 SessionDetail 排版），
//   fullscreen = 全屏浮层；默认 fullscreen，切换控件在 SessionDetail（mode 是受控 prop）。
import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/common";
import { ArrowLeft, X } from "lucide-react";
import PreviewModeSwitcher, { type PreviewMode } from "./PreviewModeSwitcher";
import { fileKindOf } from "./FilePanel";
import { ApiError, fetchFile, type FilePayload } from "./api";
import type { Session } from "@/types/session";

interface FilePreviewProps {
  /** 所属会话（file 端点按 session_id 找项目目录，数据同源铁律 3） */
  session: Session;
  /** 待预览的文件路径（绝对或相对项目目录，端点侧 read_file_safe 解析） */
  filePath: string;
  /** 呈现形态：split=上对话下文件 / split-h=左对话右文件 / fullscreen=全屏浮层
   *  （只影响容器布局语义，布局本身由 SessionDetail 承担） */
  mode: PreviewMode;
  /** 布局切换回调（Bug 2，M3 验收）：提供后页头出现三态切换控件——
   *  全屏浮层 fixed inset-0 盖住 SessionDetail 页头，此控件是全屏态唯一可达入口；
   *  缺省不渲染（既有直接用法/测试不受影响），切换仍由 SessionDetail 持有 mode */
  onModeChange?: (mode: PreviewMode) => void;
  /** 返回文件列表（M3+）：从面板进入时提供 → 页头显示返回按钮；
   *  从消息正文链接进入（backToList=false）时缺省不渲染，既有行为不变 */
  onBack?: () => void;
  /** 字号档位（2026-09-16 用户裁决）：只作用于预览内容区（页头不受影响）。
   *  缺省 1（100%，不覆写 CSS 变量） */
  fontScale?: number;
  onClose: () => void;
}

type LoadState =
  | { phase: "loading" }
  | {
      phase: "error";
      status: number | null;
      /** 403 响应体的结构化原因码（M5 P2-a：sensitive/too_large/not_found/not_file/io） */
      errorData: Record<string, unknown> | null;
    }
  | { phase: "ok"; payload: FilePayload };

/** 扩展名 → highlight.js 语言（lib/common 子集内的常用项；未命中走自动检测） */
function hljsLanguage(filePath: string): string | undefined {
  const ext = filePath.split(".").pop()?.toLowerCase() ?? "";
  const byExt: Record<string, string> = {
    rs: "rust",
    ts: "typescript",
    tsx: "typescript",
    js: "javascript",
    mjs: "javascript",
    cjs: "javascript",
    jsx: "javascript",
    py: "python",
    go: "go",
    java: "java",
    c: "c",
    h: "c",
    cpp: "cpp",
    cc: "cpp",
    hpp: "cpp",
    css: "css",
    json: "json",
    toml: "ini",
    sh: "bash",
    yaml: "yaml",
    yml: "yaml",
    xml: "xml",
    svg: "xml",
  };
  return byExt[ext];
}

/** 按扩展名高亮纯文本（highlight.js 输出自带 HTML 转义；失败回落纯文本） */
function highlightText(content: string, filePath: string): string {
  try {
    const lang = hljsLanguage(filePath);
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(content, { language: lang, ignoreIllegals: true }).value;
    }
    return hljs.highlightAuto(content).value;
  } catch {
    return ""; // 由调用方回落到 textContent 渲染
  }
}

export default function FilePreview({
  session,
  filePath,
  mode,
  onModeChange,
  onBack,
  fontScale = 1,
  onClose,
}: FilePreviewProps) {
  const [state, setState] = useState<LoadState>({ phase: "loading" }); // 手动重试信号（403 时文件可能已被 agent 补写回限内 / 网络恢复后再试）
  const [retryTick, setRetryTick] = useState(0);

  // 源码/渲染双态（M5 B4，线稿 .pv-head seg）：仅 .md 与 .html 出现切换器。
  // 默认口径 = 各自现状延续：.md 默认渲染（现有 markdown 排版显式化）、
  // .html 默认源码（渲染为本次新增能力——沙箱 iframe，见正文分支）
  const lowerPath = filePath.toLowerCase();
  const segKind: "markdown" | "html" | null = lowerPath.endsWith(".md")
    ? "markdown"
    : lowerPath.endsWith(".markdown")
      ? "markdown"
      : lowerPath.endsWith(".html") || lowerPath.endsWith(".htm")
        ? "html"
        : null;
  const [view, setView] = useState<"source" | "render">(
    segKind === "markdown" ? "render" : "source"
  );
  // 换文件回到该扩展名的默认态，避免上一份文件的切换态串场
  useEffect(() => {
    setView(
      filePath.toLowerCase().endsWith(".md") || filePath.toLowerCase().endsWith(".markdown")
        ? "render"
        : "source"
    );
  }, [filePath]);

  // HTML 渲染态缩放（M5 P2-b，虚拟浏览器放缩）：步进 25%、范围 50%–200%。
  // 缩放经注入 srcDoc 的样式生效（html{zoom}），跨文件保留——观感偏好不随文件重置
  const [zoom, setZoom] = useState(1);
  const ZOOM_MIN = 0.5;
  const ZOOM_MAX = 2;
  const ZOOM_STEP = 0.25;
  const clampZoom = (z: number) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z));
  const bumpZoom = (dir: 1 | -1) =>
    setZoom((z) => clampZoom(Math.round((z + dir * ZOOM_STEP) * 100) / 100));

  // 源码档软换行（M5 P2-b）：文档类（md/markdown/txt）源码自动换行——不影响
  // 阅读与实质；代码类保持不换行（横向滚动，保逻辑关系）。业界惯例
  // （GitHub / VSCode 默认口径）
  const docSource = segKind === "markdown" || fileKindOf(filePath) === "doc";

  // 拉取文件：挂载与 filePath 变化时各一次；图片 object URL 在清理函数里 revoke，
  // 防浮窗反复开关泄漏 blob。alive 标记防卸载后 setState 与迟到响应的 revoke 竞态
  useEffect(() => {
    let alive = true;
    let objectUrl: string | null = null;
    setState({ phase: "loading" });
    fetchFile(session.id, filePath)
      .then((p) => {
        if (!alive) {
          if (p.kind === "image") URL.revokeObjectURL(p.url); // 迟到的图片响应直接释放
          return;
        }
        if (p.kind === "image") objectUrl = p.url;
        setState({ phase: "ok", payload: p });
      })
      .catch((e: unknown) => {
        if (alive) {
          setState({
            phase: "error",
            status: e instanceof ApiError ? e.status : null,
            errorData: e instanceof ApiError ? e.data : null,
          });
        }
      });
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [session.id, filePath, retryTick]);

  // 路径末段作标题（分隔符双态：win 反斜杠 / unix 斜杠）
  const baseName = filePath.split(/[\\/]/).pop() || filePath;

  // M5 P2-a：403 原因分診——后端响应体 error ∈ sensitive/too_large/not_found/
  // not_file/io（snake_case），已过闸设备可见原因便于排障；404 会话级与网络异常
  // 保留原兜底文案
  const errorReason =
    state.phase === "error"
      ? typeof (state.errorData ?? {})?.error === "string"
        ? ((state.errorData as { error: string }).error as string)
        : null
      : null;
  const errorText = (() => {
    if (state.phase !== "error") return "";
    if (errorReason === "sensitive") return "该路径受安全策略保护，无法预览";
    if (errorReason === "too_large")
      return "文件超过预览上限（文本 500KB / 图片 5MB），请用工具导出小结后查看";
    if (errorReason === "not_found") return "文件不存在或已被移动";
    if (errorReason === "not_file") return "该路径不是文件";
    if (state.status === 403 || state.status === 404) return "无法预览该文件";
    return "预览加载失败";
  })();

  // 正文三分支（M5 B4）：markdown 渲染 / html 沙箱 iframe / 源码高亮。
  // md 在 seg 源码态回落 code 分支；html 渲染态 = 取文本 → 沙箱 iframe
  // （sandbox 仅 allow-scripts、**无** allow-same-origin——脚本可跑但与看板
  // 数据/cookie 完全隔离，线稿既定安全口径）
  const showMarkdown =
    state.phase === "ok" &&
    state.payload.kind === "text" &&
    state.payload.mime === "text/markdown" &&
    !(segKind === "markdown" && view === "source");
  const showHtmlFrame =
    state.phase === "ok" &&
    state.payload.kind === "text" &&
    segKind === "html" &&
    view === "render";

  // 高亮 HTML 只在内容变化时重算（500KB 文本的高亮不是零成本，勿放渲染期）；
  // markdown 渲染态不需要高亮产物，跳过（既有优化，随分支条件同步）
  const highlighted = useMemo(() => {
    if (state.phase !== "ok" || state.payload.kind !== "text") return null;
    if (showMarkdown) return null;
    return highlightText(state.payload.content, filePath);
  }, [state, filePath, showMarkdown]);

  return (
    <section
      data-testid="file-preview"
      data-mode={mode}
      aria-label={`文件预览 ${baseName}`}
      className="flex h-full min-h-0 flex-col bg-white dark:bg-slate-950"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-slate-200 px-3 py-2 dark:border-slate-800">
        {/* 返回文件列表（M3+）：仅从面板进入时出现 */}
        {onBack && (
          <button
            type="button"
            data-testid="preview-back-list"
            aria-label="返回文件列表"
            onClick={onBack}
            className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            <ArrowLeft size={16} />
          </button>
        )}
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-slate-800 dark:text-slate-200">
          {baseName}
        </span>
        {state.phase === "error" && (
          <button
            type="button"
            data-testid="preview-retry"
            onClick={() => setRetryTick((t) => t + 1)}
            className="shrink-0 rounded-md bg-slate-200 px-2 py-1 text-xs text-slate-700 dark:bg-slate-800 dark:text-slate-300"
          >
            重试
          </button>
        )}
        {/* 布局切换控件（Bug 2，M3 验收）：全屏浮层盖住 SessionDetail 页头时，
            此处是三态切换的唯一可达入口 */}
        {onModeChange && (
          <PreviewModeSwitcher mode={mode} onChange={onModeChange} testIdPrefix="preview-toggle" />
        )}
        {/* 源码/渲染 seg（M5 B4，线稿 .pv-head）：仅 .md 与 .html 出现（文本加载后）。
            渲染态（iframe）下保持在场——它是切回源码的唯一入口 */}
        {segKind !== null && state.phase === "ok" && state.payload.kind === "text" && (
          <span
            role="group"
            aria-label="源码/渲染切换"
            data-testid="preview-seg"
            className="flex shrink-0 overflow-hidden rounded-lg border border-slate-300 dark:border-slate-700"
          >
            <button
              type="button"
              data-testid="preview-seg-source"
              aria-pressed={view === "source"}
              onClick={() => setView("source")}
              className={`px-2.5 py-1 text-xs ${
                view === "source"
                  ? "bg-slate-800 text-white dark:bg-slate-100 dark:text-slate-900"
                  : "text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
              }`}
            >
              源码
            </button>
            <button
              type="button"
              data-testid="preview-seg-render"
              aria-pressed={view === "render"}
              onClick={() => setView("render")}
              className={`px-2.5 py-1 text-xs ${
                view === "render"
                  ? "bg-slate-800 text-white dark:bg-slate-100 dark:text-slate-900"
                  : "text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
              }`}
            >
              渲染
            </button>
          </span>
        )}
        {/* 缩放控件（M5 P2-b）：仅 HTML 渲染态出现——虚拟浏览器放缩 */}
        {showHtmlFrame && (
          <span
            role="group"
            aria-label="渲染缩放"
            data-testid="preview-zoom"
            className="flex shrink-0 items-center overflow-hidden rounded-lg border border-slate-300 dark:border-slate-700"
          >
            <button
              type="button"
              data-testid="preview-zoom-out"
              aria-label="缩小"
              onClick={() => bumpZoom(-1)}
              className="px-2 py-1 text-xs text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
            >
              −
            </button>
            <button
              type="button"
              data-testid="preview-zoom-reset"
              aria-label="重置缩放"
              onClick={() => setZoom(1)}
              className="border-x border-slate-300 px-1.5 py-1 text-[10px] text-slate-500 hover:bg-slate-100 dark:border-slate-700 dark:text-slate-400 dark:hover:bg-slate-800"
            >
              {Math.round(zoom * 100)}%
            </button>
            <button
              type="button"
              data-testid="preview-zoom-in"
              aria-label="放大"
              onClick={() => bumpZoom(1)}
              className="px-2 py-1 text-xs text-slate-500 hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
            >
              ＋
            </button>
          </span>
        )}
        <button
          type="button"
          data-testid="preview-close"
          aria-label="关闭预览"
          onClick={onClose}
          className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
        >
          <X size={16} />
        </button>
      </header>
      <div
        data-testid="preview-content"
        data-font-scale={fontScale}
        className="min-h-0 flex-1 overflow-auto p-3"
      >
        {state.phase === "loading" && (
          <p className="text-sm text-slate-500 dark:text-slate-400">加载中…</p>
        )}
        {state.phase === "error" && (
          <p data-testid="preview-error" className="text-sm text-rose-600 dark:text-rose-400">
            {/* M5 P2-a：403 带后端结构化原因码，按原因分診排障文案（已过闸设备可见） */}
            {errorText}
          </p>
        )}
        {state.phase === "ok" && state.payload.kind === "image" && (
          <img
            data-testid="preview-image"
            src={state.payload.url}
            alt={baseName}
            className="max-w-full"
          />
        )}
        {showHtmlFrame && (
          <iframe
            data-testid="preview-html-frame"
            title={`渲染 ${baseName}`}
            /* 沙箱仅 allow-scripts（无 allow-same-origin）：渲染态可跑脚本，
             * 但与看板数据、cookie、storage 完全隔离（M5 线稿既定安全口径）。
             * 缩放（M5 P2-b）：注入 html{zoom} 样式实现放缩——内容自带样式表的
             * body 级规则不会覆盖 html 层 zoom */
            sandbox="allow-scripts"
            srcDoc={`<style>html{zoom:${zoom}}</style>${
              state.phase === "ok" && state.payload.kind === "text" ? state.payload.content : ""
            }`}
            className="h-full min-h-[320px] w-full rounded-lg border border-slate-200 bg-white dark:border-slate-700"
          />
        )}
        {showMarkdown && (
          <div data-testid="preview-markdown" className="md-body text-sm dark:text-slate-200">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>
              {state.phase === "ok" && state.payload.kind === "text" ? state.payload.content : ""}
            </ReactMarkdown>
          </div>
        )}
        {!showMarkdown &&
          !showHtmlFrame &&
          state.phase === "ok" &&
          state.payload.kind === "text" && (
            <pre
              data-testid="preview-code"
              className={
                // 源码档软换行（M5 P2-b）：文档类（md/txt）自动换行不影响阅读；
                // 代码类保持不换行（横向滚动，保逻辑关系与缩进层级）
                docSource
                  ? "rounded-lg bg-slate-100 p-3 text-xs break-words whitespace-pre-wrap dark:bg-slate-900"
                  : "overflow-auto rounded-lg bg-slate-100 p-3 text-xs dark:bg-slate-900"
              }
            >
              {highlighted ? (
                // highlight.js 输出已转义；.hljs 供 mobile.css 双态主题着色
                <code className="hljs" dangerouslySetInnerHTML={{ __html: highlighted }} />
              ) : (
                // 高亮失败 / 空内容 → 纯文本回落（textContent 渲染，零注入面）
                <code className="hljs">{state.payload.content}</code>
              )}
            </pre>
          )}
      </div>
    </section>
  );
}
