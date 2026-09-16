// 文件面板（M3+ 计划 Task 3）：会话工具调用涉及文件的聚合列表。
// 纯展示（用户裁决 4）——排序/去重/计数全部后端完成（files.rs FileEntry），
// 本组件的 chips 过滤只是**视图层筛选**已有结果。
//
// 布局由父组件（SessionDetail）决定：split/split-h 内联分屏、fullscreen 全屏，
// 与单文件预览共用同一侧栏空间与三态切换器。
import { useEffect, useMemo, useState } from "react";
import { FileText, Image as ImageIcon, X } from "lucide-react";
import PreviewModeSwitcher, { type PreviewMode } from "./PreviewModeSwitcher";
import { formatRelativeTime } from "./board-logic";
import type { SessionFileEntry } from "./api";

/** 追溯档位（用户裁决 3）：顶档 = SessionDetail.MAX_LIMIT（1000 到顶） */
export const FILE_SCOPES = [200, 500, 1000] as const;

/** 类型过滤档（用户裁决 5）：全部 / 文档 / 图片；代码文件仅在「全部」可见 */
export type FileKindFilter = "all" | "doc" | "image";

/** 文档扩展名（用户裁决 5：md/markdown/txt） */
const DOC_EXTS = ["md", "markdown", "txt"];
/** 图片扩展名（用户裁决 5：png/jpg/jpeg/gif/webp/svg/bmp） */
const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"];

/** 扩展名 → 类型档（纯函数，小写判定；无扩展名/未知 → other） */
export function fileKindOf(path: string): "doc" | "image" | "other" {
  const ext = path.split(/[\\/]/).pop()?.split(".").pop()?.toLowerCase() ?? "";
  if (DOC_EXTS.includes(ext)) return "doc";
  if (IMAGE_EXTS.includes(ext)) return "image";
  return "other";
}

/** 路径末段文件名（列表主行） */
export function fileBaseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** 路径的目录前缀（列表次行；无目录时为空串） */
export function fileDirPrefix(path: string): string {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join("/");
}

interface FilePanelProps {
  /** 已按 lastSeq 降序（后端契约，本组件不再排序） */
  entries: SessionFileEntry[];
  /** 该档位下还有更早文件未纳入（档位提示依据） */
  truncated: boolean;
  scope: number;
  loading: boolean;
  /** 当前布局（切换器高亮当前态；布局本身由父组件承担） */
  mode: PreviewMode;
  onScopeChange: (n: number) => void;
  /** 主行文件名点击 → 进入单文件预览 */
  onOpenFile: (path: string) => void;
  /** 布局切换（与单文件预览共用三态） */
  onModeChange: (mode: PreviewMode) => void;
  /** 字号档位（2026-09-16 用户裁决）：作用于列表内容区（头部/档位卡不受影响） */
  fontScale?: number;
  onClose: () => void;
}

export default function FilePanel({
  entries,
  truncated,
  scope,
  loading,
  mode,
  onScopeChange,
  onOpenFile,
  onModeChange,
  fontScale = 1,
  onClose,
}: FilePanelProps) {
  const [kind, setKind] = useState<FileKindFilter>("all");
  // 目录路径浮窗（2026-09-16 用户裁决）：次行目录被截断时点击查看全路径
  // （手机与电脑逻辑一致——同一组件两板共用）
  const [pathPopover, setPathPopover] = useState<string | null>(null);
  // 相对时间的基准时钟（Board 同款模式：渲染期不得调 Date.now——react-hooks/purity）
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, []);

  // 视图层筛选（不再排序：顺序即后端 lastSeq 降序，用户裁决 1）
  const visible = useMemo(
    () => (kind === "all" ? entries : entries.filter((e) => fileKindOf(e.path) === kind)),
    [entries, kind]
  );

  // 到顶判定（用户裁决 3）：1000 = MAX_LIMIT，到顶后不再提示"还有更早文件"
  const atTop = scope >= FILE_SCOPES[FILE_SCOPES.length - 1];
  const chips: Array<{ key: FileKindFilter; label: string }> = [
    { key: "all", label: "全部" },
    { key: "doc", label: "文档" },
    { key: "image", label: "图片" },
  ];

  return (
    <section
      data-testid="file-panel"
      aria-label="文件面板"
      className="flex h-full min-h-0 flex-col bg-white dark:bg-slate-950"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-slate-200 px-3 py-2 dark:border-slate-800">
        <span className="shrink-0 text-sm font-medium text-slate-800 dark:text-slate-200">
          文件
        </span>
        {/* 类型过滤 chips（用户裁决 5） */}
        <span role="group" aria-label="文件类型过滤" className="flex items-center gap-1">
          {chips.map((c) => (
            <button
              key={c.key}
              type="button"
              data-testid={`file-chip-${c.key}`}
              aria-pressed={kind === c.key}
              onClick={() => setKind(c.key)}
              className={`rounded-full px-2 py-0.5 text-xs ${
                kind === c.key
                  ? "bg-slate-800 text-white dark:bg-slate-100 dark:text-slate-900"
                  : "bg-slate-200 text-slate-600 dark:bg-slate-800 dark:text-slate-400"
              }`}
            >
              {c.label}
            </button>
          ))}
        </span>
        <PreviewModeSwitcher mode={mode} onChange={onModeChange} testIdPrefix="preview-toggle" />
        <button
          type="button"
          data-testid="panel-close"
          aria-label="关闭文件面板"
          onClick={onClose}
          className="shrink-0 rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
        >
          <X size={16} />
        </button>
      </header>

      {/* 档位卡片（用户裁决 3）：三档常显、当前高亮 */}
      <div className="flex shrink-0 items-center gap-1 border-b border-slate-200 px-3 py-2 dark:border-slate-800">
        <span className="text-xs text-slate-500 dark:text-slate-400">追溯范围</span>
        {FILE_SCOPES.map((s) => (
          <button
            key={s}
            type="button"
            data-testid={`file-scope-${s}`}
            aria-pressed={scope === s}
            onClick={() => onScopeChange(s)}
            className={`rounded-full px-2 py-0.5 text-xs ${
              scope === s
                ? "bg-sky-500/20 text-sky-700 dark:text-sky-300"
                : "bg-slate-200 text-slate-600 dark:bg-slate-800 dark:text-slate-400"
            }`}
          >
            {s}
          </button>
        ))}
        {loading && <span className="ml-auto text-xs text-slate-400">加载中…</span>}
      </div>

      <div
        data-testid="panel-content"
        data-font-scale={fontScale}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      >
        {visible.length === 0 && !loading && (
          <p data-testid="panel-empty" className="py-12 text-center text-sm text-slate-500">
            该范围内未发现文件
          </p>
        )}
        <ul className={`space-y-1 ${loading && entries.length > 0 ? "opacity-60" : ""}`}>
          {visible.map((e, i) => (
            <li
              key={`${e.path}-${i}`}
              data-testid={`file-row-${i}`}
              className="rounded-lg px-2 py-1.5 hover:bg-slate-100 dark:hover:bg-slate-900"
            >
              {/* 主行 = 末段文件名（加粗，即超链接）；次行 = 目录 + 时间 + hits */}
              <button
                type="button"
                data-testid={`file-row-${i}-open`}
                onClick={() => onOpenFile(e.path)}
                className="block w-full truncate text-left text-sm font-medium text-sky-700 hover:underline dark:text-sky-400"
              >
                {fileKindOf(e.path) === "image" ? (
                  <ImageIcon size={12} className="mr-1 inline shrink-0" />
                ) : (
                  <FileText size={12} className="mr-1 inline shrink-0" />
                )}
                {fileBaseName(e.path)}
              </button>
              <div className="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
                {fileDirPrefix(e.path) ? (
                  <button
                    type="button"
                    data-testid={`file-row-${i}-dir`}
                    aria-label="查看完整路径"
                    title="点击查看完整路径"
                    onClick={() =>
                      setPathPopover((p) =>
                        p === fileDirPrefix(e.path) ? null : fileDirPrefix(e.path)
                      )
                    }
                    className="min-w-0 flex-1 truncate text-left hover:text-slate-700 hover:underline dark:hover:text-slate-200"
                  >
                    {fileDirPrefix(e.path)}
                  </button>
                ) : (
                  <span className="min-w-0 flex-1" />
                )}
                {/* 相对时间（lastTs null → 占位符） */}
                <span className="shrink-0">
                  {e.lastTs === null
                    ? "—"
                    : formatRelativeTime(new Date(e.lastTs).toISOString(), now)}
                </span>
                {/* hits 徽标（1 次不显示） */}
                {e.hits > 1 && <span className="shrink-0">{e.hits}×</span>}
              </div>
            </li>
          ))}
        </ul>
        {/* 路径浮窗（2026-09-16 用户裁决）：完整目录路径可读可复制（长按/选中） */}
        {pathPopover !== null && (
          <div
            data-testid="path-popover"
            role="dialog"
            aria-label="完整路径"
            className="sticky bottom-0 mt-2 flex items-start gap-2 rounded-lg border border-slate-300 bg-white p-2 shadow-lg dark:border-slate-700 dark:bg-slate-900"
          >
            <code className="min-w-0 flex-1 text-xs break-all text-slate-700 dark:text-slate-200">
              {pathPopover}
            </code>
            <button
              type="button"
              data-testid="path-popover-close"
              aria-label="关闭路径浮窗"
              onClick={() => setPathPopover(null)}
              className="shrink-0 rounded-full p-0.5 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
            >
              <X size={14} />
            </button>
          </div>
        )}
        {/* 档位提示：还有更早文件且未到顶（用户裁决 3） */}
        {truncated && !atTop && (
          <p
            data-testid="panel-truncated-hint"
            className="mt-2 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400"
          >
            范围内还有更早文件，可扩大追溯范围
          </p>
        )}
      </div>
    </section>
  );
}
