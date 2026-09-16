// 预览形态切换器（需求 1，2026-09-16 用户裁决）：三态互切——
// 纵向分屏（上对话下文件）/ 横向分屏（左对话右文件）/ 全屏浮层。
// 页头（SessionDetail）与预览页头（FilePreview）复用同一组件：全屏浮层
// fixed inset-0 盖住详情页头，预览页头那份是全屏态唯一可达入口。
import { Columns2, Maximize2, Rows2 } from "lucide-react";

/** 预览形态三态：与 SessionDetail.PreviewMode 同源（此文件为避免循环依赖
 *  独立声明；两者由 usage 类型检查保持一致） */
export type PreviewMode = "split" | "split-h" | "fullscreen";

interface PreviewModeSwitcherProps {
  mode: PreviewMode;
  onChange: (mode: PreviewMode) => void;
  /** testid 前缀：页头用 preview-mode、预览页头用 preview-toggle（互不冲突） */
  testIdPrefix: string;
}

export default function PreviewModeSwitcher({
  mode,
  onChange,
  testIdPrefix,
}: PreviewModeSwitcherProps) {
  const cls = (active: boolean) =>
    `rounded-full p-1 ${
      active
        ? "bg-white text-slate-900 dark:bg-slate-950 dark:text-slate-100"
        : "text-slate-500 dark:text-slate-400"
    }`;
  return (
    <span
      role="group"
      aria-label="预览布局"
      className="flex shrink-0 items-center rounded-full bg-slate-200 p-0.5 dark:bg-slate-800"
    >
      <button
        type="button"
        data-testid={`${testIdPrefix}-split`}
        aria-pressed={mode === "split"}
        aria-label="上下分屏预览"
        onClick={() => onChange("split")}
        className={cls(mode === "split")}
      >
        <Columns2 size={14} />
      </button>
      <button
        type="button"
        data-testid={`${testIdPrefix}-split-h`}
        aria-pressed={mode === "split-h"}
        aria-label="左右分屏预览"
        onClick={() => onChange("split-h")}
        className={cls(mode === "split-h")}
      >
        <Rows2 size={14} />
      </button>
      <button
        type="button"
        data-testid={`${testIdPrefix}-fullscreen`}
        aria-pressed={mode === "fullscreen"}
        aria-label="全屏预览"
        onClick={() => onChange("fullscreen")}
        className={cls(mode === "fullscreen")}
      >
        <Maximize2 size={14} />
      </button>
    </span>
  );
}
