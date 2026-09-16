// 文件面板（M3+）：会话工具调用涉及文件的聚合列表。
// Task 2 骨架：仅提供容器与接口（列表/过滤/档位在 Task 3 实现）。
import { X } from "lucide-react";
import PreviewModeSwitcher, { type PreviewMode } from "./PreviewModeSwitcher";
import type { SessionFileEntry } from "./api";

/** 追溯档位（用户裁决 3）：顶档 = SessionDetail.MAX_LIMIT（1000 到顶） */
export const FILE_SCOPES = [200, 500, 1000] as const;

/** 类型过滤档（用户裁决 5）：全部 / 文档 / 图片；代码文件仅在「全部」可见 */
export type FileKindFilter = "all" | "doc" | "image";

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
  onClose: () => void;
}

export default function FilePanel({ onModeChange, onClose }: FilePanelProps) {
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
        <PreviewModeSwitcher
          mode="fullscreen"
          onChange={onModeChange}
          testIdPrefix="preview-toggle"
        />
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
      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2" />
    </section>
  );
}
