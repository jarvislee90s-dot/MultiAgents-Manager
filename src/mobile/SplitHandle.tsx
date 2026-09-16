// 可拖动分隔条（需求，2026-09-16 用户裁决）：预览分屏的对话区 / 文件区之间
// 的尺寸可拖拽调整——横向分屏拖宽窄、纵向分屏拖高低。
//
// 实现要点：
// - 受控比值 `ratio`（文件栏占比，0..1）：父组件持有，拖动回写；切换形态时各自
//   保留（横向拖宽度、纵向拖高度互不干扰）；
// - 拖动期间用 window 级 pointermove/pointerup 监听（而非 setPointerCapture：
//   jsdom 无该方法，且 window 监听在指针滑出 handle 后仍持续跟踪）；
// - 坐标换算：横向按 clientX 增量 / 容器宽；纵向按 clientY 增量 / 容器高
//   （增量取负——向下拖 = 对话区变高 = 文件栏变矮）；
// - 钳制 [MIN_RATIO, MAX_RATIO]：任一区不被拖到消失（15%–85%）；
// - 键盘可达：handle 为 button，方向键微调 2%（无障碍，不限于指针设备）；
// - touch-action: none 关掉浏览器滚动手势，避免手机上拖动被识别为滚动。
import { useCallback, useEffect, useRef } from "react";

/** 文件栏占比下/上限：任一区至少保留 15%（防拖没） */
export const MIN_RATIO = 0.15;
export const MAX_RATIO = 0.85;
/** 键盘方向键步长（占容器比例） */
const KEY_STEP = 0.02;

interface SplitHandleProps {
  /** 分屏方向：split-h = 左右（拖 clientX）/ split = 上下（拖 clientY） */
  orientation: "horizontal" | "vertical";
  /** 当前文件栏占比（受控） */
  ratio: number;
  /** 拖动回调：传入已钳制的新占比 */
  onRatioChange: (ratio: number) => void;
  /** 容器尺寸读数（拖动换算的分母来源） */
  containerRef: React.RefObject<HTMLElement | null>;
}

export default function SplitHandle({
  orientation,
  ratio,
  onRatioChange,
  containerRef,
}: SplitHandleProps) {
  // 拖动起始快照（起点坐标 + 起点占比）：每次 pointerdown 重置
  const dragRef = useRef<{ pos: number; ratio: number } | null>(null);

  const clamp = (v: number) => Math.min(MAX_RATIO, Math.max(MIN_RATIO, v));

  const onPointerDown = useCallback(
    (e: React.PointerEvent<HTMLButtonElement>) => {
      e.preventDefault();
      dragRef.current = {
        pos: orientation === "horizontal" ? e.clientX : e.clientY,
        ratio,
      };
    },
    [orientation, ratio]
  );

  // 拖动期间的 window 级监听：仅在拖动中挂载（dragRef 非空）
  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const span = orientation === "horizontal" ? rect.width : rect.height;
      if (span <= 0) return; // 无布局引擎 / 未挂载：不换算
      const now = orientation === "horizontal" ? e.clientX : e.clientY;
      // 纵向：文件栏在下半区，向下拖 = 文件栏变矮 → 占比减，故增量取负
      const delta = orientation === "horizontal" ? now - drag.pos : drag.pos - now;
      onRatioChange(clamp(drag.ratio + delta / span));
    };
    const onUp = () => {
      dragRef.current = null;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [containerRef, onRatioChange, orientation]);

  // 键盘微调：方向键按「让文件栏变大的方向」直觉映射
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>) => {
      const bigger = orientation === "horizontal" ? "ArrowRight" : "ArrowUp";
      const smaller = orientation === "horizontal" ? "ArrowLeft" : "ArrowDown";
      if (e.key === bigger) {
        e.preventDefault();
        onRatioChange(clamp(ratio + KEY_STEP));
      } else if (e.key === smaller) {
        e.preventDefault();
        onRatioChange(clamp(ratio - KEY_STEP));
      }
    },
    [onRatioChange, orientation, ratio]
  );

  return (
    <button
      type="button"
      role="separator"
      aria-orientation={orientation === "horizontal" ? "vertical" : "horizontal"}
      aria-label="拖动调整分屏比例"
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={Math.round(MIN_RATIO * 100)}
      aria-valuemax={Math.round(MAX_RATIO * 100)}
      data-testid="split-handle"
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      style={{ touchAction: "none" }}
      className={
        orientation === "horizontal"
          ? "w-1.5 shrink-0 cursor-col-resize bg-slate-200 hover:bg-sky-400/60 dark:bg-slate-800 dark:hover:bg-sky-500/60"
          : "h-1.5 shrink-0 cursor-row-resize bg-slate-200 hover:bg-sky-400/60 dark:bg-slate-800 dark:hover:bg-sky-500/60"
      }
    />
  );
}
