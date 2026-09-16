// 文件预览浮窗（M3 Task 8，P9）：会话项目目录内文件的安全只读预览。
// - 图片 mime → <img>（同源 fetch blob → object URL，卸载/换文件时 revoke）；
// - markdown → ReactMarkdown 渲染；其他文本 → <pre><code> 语法高亮
//   （highlight.js/lib/common，与 rehype-highlight 共享同一份 lowlight/highlight.js
//   模块，不产生重复打包）；
// - 越界 / 超限 / 不存在对外一律 403（后端探测面最小化）→「无法预览该文件」错误态；
// - 手机现实（Task 8 裁决）：split = 上对话下文件（纵向分屏，由 SessionDetail 排版），
//   fullscreen = 全屏浮层；默认 fullscreen，切换控件在 SessionDetail（mode 是受控 prop）。
import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/common";
import { X } from "lucide-react";
import PreviewModeSwitcher, { type PreviewMode } from "./PreviewModeSwitcher";
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
  onClose: () => void;
}

type LoadState =
  | { phase: "loading" }
  | { phase: "error"; status: number | null }
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
  onClose,
}: FilePreviewProps) {
  const [state, setState] = useState<LoadState>({ phase: "loading" });
  // 手动重试信号（403 时文件可能已被 agent 补写回限内 / 网络恢复后再试）
  const [retryTick, setRetryTick] = useState(0);

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
          setState({ phase: "error", status: e instanceof ApiError ? e.status : null });
        }
      });
    return () => {
      alive = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [session.id, filePath, retryTick]);

  // 高亮 HTML 只在内容变化时重算（500KB 文本的高亮不是零成本，勿放渲染期）
  const highlighted = useMemo(() => {
    if (state.phase !== "ok" || state.payload.kind !== "text") return null;
    if (state.payload.mime === "text/markdown") return null;
    return highlightText(state.payload.content, filePath);
  }, [state, filePath]);

  // 路径末段作标题（分隔符双态：win 反斜杠 / unix 斜杠）
  const baseName = filePath.split(/[\\/]/).pop() || filePath;
  const previewableError =
    state.phase === "error" && (state.status === 403 || state.status === 404);

  return (
    <section
      data-testid="file-preview"
      data-mode={mode}
      aria-label={`文件预览 ${baseName}`}
      className="flex h-full min-h-0 flex-col bg-white dark:bg-slate-950"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-slate-200 px-3 py-2 dark:border-slate-800">
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
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {state.phase === "loading" && (
          <p className="text-sm text-slate-500 dark:text-slate-400">加载中…</p>
        )}
        {state.phase === "error" && (
          <p data-testid="preview-error" className="text-sm text-rose-600 dark:text-rose-400">
            {/* 探测面最小化：403（越界/超限/不存在不可区分）与 404 同文案，不给预言机 */}
            {previewableError ? "无法预览该文件" : "预览加载失败"}
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
        {state.phase === "ok" &&
          state.payload.kind === "text" &&
          state.payload.mime === "text/markdown" && (
            <div data-testid="preview-markdown" className="md-body text-sm dark:text-slate-200">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{state.payload.content}</ReactMarkdown>
            </div>
          )}
        {state.phase === "ok" &&
          state.payload.kind === "text" &&
          state.payload.mime !== "text/markdown" && (
            <pre
              data-testid="preview-code"
              className="overflow-auto rounded-lg bg-slate-100 p-3 text-xs dark:bg-slate-900"
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
