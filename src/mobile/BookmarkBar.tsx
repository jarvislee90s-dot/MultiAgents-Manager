// 消息窗口书签条（M3+，2026-09-16 用户裁决）：消息区上方常驻横条。
//
// 三个功能区（用户裁决）：
// - 左：「+ 书签」→ 弹 10 色调色板，已占用色置灰不可点；选色即打标签
//   （落点 = 当前视口顶部消息，由 SessionDetail 取锚后回填）；
// - 中：一排色点——常态点即跳转；管理态变成 × 删除钮；
// - 右：「管理」↔「完成」切换；管理态额外出现「清空全部」。
//
// 纯展示组件（数据与 store 操作全在 SessionDetail），故无内部书签状态；
// 唯一内部状态是「调色板是否展开」「是否管理态」。
import { useState } from "react";
import { Plus, Settings2, X } from "lucide-react";
import { BOOKMARK_COLORS, type Bookmark } from "./bookmarks";

interface BookmarkBarProps {
  bookmarks: Bookmark[];
  /** 已达上限：+ 按钮 disabled（上限由 store 保证，此处只做展示） */
  atLimit: boolean;
  /** 选色打标签（color 为调色板色值） */
  onAdd: (color: string) => void;
  /** 点色点跳转（anchor 为内容指纹） */
  onJump: (anchor: string) => void;
  /** 管理态删除单条 */
  onRemove: (color: string) => void;
  /** 清空该会话全部书签 */
  onClear: () => void;
}

export default function BookmarkBar({
  bookmarks,
  atLimit,
  onAdd,
  onJump,
  onRemove,
  onClear,
}: BookmarkBarProps) {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [managing, setManaging] = useState(false);
  const usedColors = new Set(bookmarks.map((b) => b.color));

  return (
    <div className="relative shrink-0 border-b border-slate-200 px-3 py-1.5 dark:border-slate-800">
      <div className="flex items-center gap-2">
        {/* 打标签入口 */}
        <button
          type="button"
          data-testid="bookmark-add"
          aria-label="添加书签"
          title={atLimit ? "已达 10 个书签上限" : "在当前位置添加书签"}
          aria-expanded={paletteOpen}
          disabled={atLimit}
          onClick={() => setPaletteOpen((v) => !v)}
          className={`flex shrink-0 items-center gap-0.5 rounded-full px-2 py-0.5 text-xs ${
            atLimit
              ? "cursor-not-allowed text-slate-400 dark:text-slate-600"
              : "text-slate-600 hover:bg-slate-200 dark:text-slate-300 dark:hover:bg-slate-800"
          }`}
        >
          <Plus size={13} />
          书签
        </button>

        {/* 色点区：常态点即跳转；管理态变大一号的 × 删除钮（同一按钮两种态） */}
        <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
          {bookmarks.map((b) => (
            <button
              key={b.color}
              type="button"
              data-testid={`bookmark-${managing ? "remove" : "dot"}-${b.color}`}
              aria-label={`${managing ? "删除" : "跳转到"}书签 ${b.preview}`}
              title={managing ? `删除：${b.preview}` : b.preview}
              onClick={() => (managing ? onRemove(b.color) : onJump(b.anchor))}
              style={{ backgroundColor: b.color }}
              className={
                managing
                  ? "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-white"
                  : "h-3.5 w-3.5 shrink-0 rounded-full transition-transform hover:scale-125"
              }
            >
              {managing && <X size={12} />}
            </button>
          ))}
        </span>

        {/* 管理 / 完成 */}
        <button
          type="button"
          data-testid={managing ? "bookmark-manage-done" : "bookmark-manage"}
          aria-label={managing ? "完成管理" : "管理书签"}
          aria-pressed={managing}
          onClick={() => setManaging((v) => !v)}
          className={`shrink-0 rounded-full p-1 text-xs ${
            managing
              ? "bg-slate-200 text-slate-900 dark:bg-slate-800 dark:text-slate-100"
              : "text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
          }`}
        >
          {managing ? "完成" : <Settings2 size={14} />}
        </button>

        {/* 清空全部（仅管理态 + 有书签） */}
        {managing && bookmarks.length > 0 && (
          <button
            type="button"
            data-testid="bookmark-clear-all"
            aria-label="清空全部书签"
            onClick={onClear}
            className="shrink-0 rounded-full bg-rose-500/15 px-2 py-0.5 text-xs text-rose-700 dark:text-rose-400"
          >
            清空全部
          </button>
        )}
      </div>

      {/* 调色板（点 + 展开）：10 色，已占用置灰 */}
      {paletteOpen && (
        <div
          data-testid="bookmark-palette"
          role="dialog"
          aria-label="选择书签颜色"
          className="absolute top-full left-3 z-10 mt-1 flex flex-wrap gap-1.5 rounded-lg border border-slate-300 bg-white p-2 shadow-lg dark:border-slate-700 dark:bg-slate-900"
        >
          {BOOKMARK_COLORS.map((c) => {
            const used = usedColors.has(c);
            return (
              <button
                key={c}
                type="button"
                data-testid={`bookmark-color-${c}`}
                aria-label={used ? `颜色 ${c} 已占用` : `用颜色 ${c} 添加书签`}
                title={used ? "该颜色已占用" : "用此颜色添加书签"}
                disabled={used}
                onClick={() => {
                  onAdd(c);
                  setPaletteOpen(false);
                }}
                className={`h-6 w-6 rounded-full ${
                  used ? "cursor-not-allowed opacity-25" : "hover:scale-110"
                }`}
                style={{ backgroundColor: c }}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}
