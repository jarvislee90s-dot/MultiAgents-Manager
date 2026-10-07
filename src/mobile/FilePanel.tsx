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
import { fetchChannel, type ChannelInfo, type SessionFileEntry } from "./api";

/** 带宽耗时预估的参考附件大小（§C5，MB）：面板只知路径不知大小，按线稿口径用
 *  20 MB 参考量给出「量级感受」（20MB×8bit / 1.8Mbps ≈ 1.5 分钟 / ÷0.8 ≈ 3.3 分钟） */
const BW_REFERENCE_MB = 20;

/** 带宽受限横幅（§C5：如实告知 + 满速升级指引）。**文件面板与附件区共用同一份
 *  判据与文案**（评审 I-2 后半：附件区也要出横幅——只上传、不浏览文件的用户
 *  不然永远看不到「这是通道限制，不是故障」；抽成一个组件而不是各写一份，是为了
 *  杜绝两处文案漂移，改口径只改这里）。
 *  - 判据：仅受限通道（隧道）就地出现；局域网直连与拉取失败都不渲染；
 *  - 耗时式 = 20MB×8 / 实测速率 / 60（分钟，一位小数）；
 *  - **有限正数守卫**（评审 M-3 / I-2）：畸形载荷（limited=true 但 est=0/NaN/
 *    Infinity）时耗时式会得「Infinity 分钟」这类假数字——速率非有限正数一律
 *    不渲染（无数据不如不提示）；
 *  - testId 由调用方给（panel-bw-note / composer-bw-note），className 供落点间距。 */
export function ChannelBwNote({
  channel,
  testId,
  className = "",
}: {
  channel: ChannelInfo | null;
  testId: string;
  className?: string;
}) {
  if (!channel?.limited) return null;
  // 速率要能算出有意义的耗时：非有限正数（0 / NaN / Infinity）一律不渲染横幅
  const isUsableRate = (mbps: number) => Number.isFinite(mbps) && mbps > 0;
  if (!isUsableRate(channel.estMbpsDown) || !isUsableRate(channel.estMbpsUp)) return null;
  // 参考量按实测速率折算的耗时（分钟，一位小数）
  const minutesFor = (mbps: number) => ((BW_REFERENCE_MB * 8) / mbps / 60).toFixed(1);
  return (
    <div
      data-testid={testId}
      className={`rounded-lg bg-[var(--cbg)] px-2 py-1.5 text-[11px] leading-4 text-[var(--mut)] ${className}`}
    >
      当前通道带宽受限（实测下行 ~{channel.estMbpsDown} Mbps / 上行 ~{channel.estMbpsUp}{" "}
      Mbps——这是通道限制，不是故障）：{BW_REFERENCE_MB} MB 附件下载约需{" "}
      {minutesFor(channel.estMbpsDown)} 分钟、上传约需 {minutesFor(channel.estMbpsUp)} 分钟。
      手机装上 Tailscale 并加入尾网后可直连本机，带宽可提升 1–2 个数量级（满速）。
    </div>
  );
}

/** 追溯档位（用户裁决 3）：顶档 = SessionDetail.MAX_LIMIT（1000 到顶） */
export const FILE_SCOPES = [200, 500, 1000] as const;

/** 类型过滤档（用户裁决 5）：全部 / 文档 / 图片；代码文件仅在「全部」可见 */
export type FileKindFilter = "all" | "doc" | "image";

/** 来源过滤档（M5 决策 10 / 线稿三池）：全部来源 / 我上传的 / 工具读取 / 工具读写。
 *  值与后端 FileEntry.origin（snake_case 序列化）一致 */
export type FileOriginFilter = "all" | "user" | "tool_read" | "tool_write";

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
  // 来源筛选（M5 B3）：默认全部来源；undefined origin 的旧载荷条目只在「全部」可见
  const [origin, setOrigin] = useState<FileOriginFilter>("all");
  // 文件名搜索（M5 决策 10）：**点「搜索」（或回车）才执行**——输入框只是草稿态，
  // activeSearch 才参与过滤。两态分离是裁决的执行点，勿合并成受控即时过滤
  const [searchDraft, setSearchDraft] = useState("");
  const [activeSearch, setActiveSearch] = useState("");
  // 目录路径浮窗（2026-09-16 用户裁决）：次行目录被截断时点击查看全路径
  // （手机与电脑逻辑一致——同一组件两板共用）
  const [pathPopover, setPathPopover] = useState<string | null>(null);
  // 相对时间的基准时钟（Board 同款模式：渲染期不得调 Date.now——react-hooks/purity）
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, []);

  // 通道能力（Task 10 §C5，装饰）：挂载拉一次，任何失败静默 → 无提示不阻塞面板。
  // **纯装饰**——读 Host 推断、可被伪造、允许不准、不进任何安全判定（G1 推论③）
  const [channel, setChannel] = useState<ChannelInfo | null>(null);
  useEffect(() => {
    let alive = true;
    void fetchChannel().then((c) => {
      if (alive) setChannel(c);
    });
    return () => {
      alive = false;
    };
  }, []);

  /** 执行搜索：草稿 trim 后生效（空串 = 清除搜索） */
  const runSearch = () => setActiveSearch(searchDraft.trim());

  // 视图层三层叠加过滤（决策 10：与追溯范围、类型、来源叠加；不再排序：
  // 顺序即后端 lastSeq 降序，用户裁决 1）
  const visible = useMemo(() => {
    let list = kind === "all" ? entries : entries.filter((e) => fileKindOf(e.path) === kind);
    if (origin !== "all") {
      list = list.filter((e) => e.origin === origin);
    }
    if (activeSearch) {
      const q = activeSearch.toLowerCase();
      list = list.filter((e) => fileBaseName(e.path).toLowerCase().includes(q));
    }
    return list;
  }, [entries, kind, origin, activeSearch]);

  // 到顶判定（用户裁决 3）：1000 = MAX_LIMIT，到顶后不再提示"还有更早文件"
  const atTop = scope >= FILE_SCOPES[FILE_SCOPES.length - 1];
  const chips: Array<{ key: FileKindFilter; label: string }> = [
    { key: "all", label: "全部" },
    { key: "doc", label: "文档" },
    { key: "image", label: "图片" },
  ];
  // 来源 chips（M5 线稿三池 + 全部；键 = 后端 origin 值域）
  const originChips: Array<{ key: FileOriginFilter; label: string }> = [
    { key: "all", label: "全部来源" },
    { key: "user", label: "我上传的" },
    { key: "tool_read", label: "工具读取" },
    { key: "tool_write", label: "工具读写" },
  ];
  // 行内来源徽标文案（undefined = 旧载荷，不渲染）
  const originLabel: Record<string, string> = {
    user: "我上传的",
    tool_read: "工具读取",
    tool_write: "工具读写",
  };

  return (
    <section
      data-testid="file-panel"
      aria-label="文件面板"
      className="flex h-full min-h-0 flex-col bg-[var(--cbg)]"
    >
      <header className="flex shrink-0 items-center gap-2 border-b border-[var(--cb)] px-3 py-2">
        <span className="shrink-0 text-sm font-medium text-[var(--tx)]">文件</span>
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
                  ? "bg-[var(--btnp)] text-[var(--btnpt)]"
                  : "bg-[var(--cb)] text-[var(--mut)]"
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
          className="shrink-0 rounded-full p-1 text-[var(--mut)] hover:bg-[var(--cb)] dark:hover:bg-[var(--btnp)]"
        >
          <X size={16} />
        </button>
      </header>

      {/* 档位卡片（用户裁决 3）：三档常显、当前高亮 */}
      <div className="flex shrink-0 items-center gap-1 border-b border-[var(--cb)] px-3 py-2">
        {/* 单位注释（2026-09-16 用户裁决）：200/500/1000 指**消息条数** */}
        <span className="text-xs text-[var(--mut)]">追溯范围</span>
        <span
          data-testid="file-scope-hint"
          className="text-[10px] text-[var(--mut)]"
          title="按最近的消息条数统计：user / assistant / 思考 / 工具调用 / 工具结果 各算 1 条"
        >
          （消息条数）
        </span>
        {FILE_SCOPES.map((s) => (
          <button
            key={s}
            type="button"
            data-testid={`file-scope-${s}`}
            aria-pressed={scope === s}
            onClick={() => onScopeChange(s)}
            className={`rounded-full px-2 py-0.5 text-xs ${
              scope === s
                ? "bg-[var(--btnp)]/20 text-[var(--tx)]"
                : "bg-[var(--cb)] text-[var(--mut)]"
            }`}
          >
            {s}
          </button>
        ))}
        {/* 来源筛选（M5 决策 10 / 线稿三池）：全部来源 / 我上传的 / 工具读取 / 工具读写 */}
        <span className="text-[var(--mut)]">|</span>
        <span role="group" aria-label="文件来源过滤" className="flex items-center gap-1">
          {originChips.map((c) => (
            <button
              key={c.key}
              type="button"
              data-testid={`file-origin-${c.key}`}
              aria-pressed={origin === c.key}
              onClick={() => setOrigin(c.key)}
              className={`rounded-full px-2 py-0.5 text-xs ${
                origin === c.key
                  ? "bg-violet-600 text-white dark:bg-violet-400"
                  : "bg-[var(--cb)] text-[var(--mut)]"
              }`}
            >
              {c.label}
            </button>
          ))}
        </span>
        {/* 文件名搜索（M5 决策 10）：点「搜索」（或回车）才执行 */}
        <form
          role="search"
          aria-label="文件名搜索"
          className="ml-auto flex items-center"
          onSubmit={(e) => {
            e.preventDefault();
            runSearch();
          }}
        >
          <input
            value={searchDraft}
            onChange={(e) => setSearchDraft(e.target.value)}
            placeholder="按文件名搜索"
            data-testid="file-search-input"
            aria-label="按文件名搜索"
            className="w-28 rounded-l-lg border border-r-0 border-[var(--cb)] bg-[var(--cbg)] px-2 py-1 text-xs outline-none focus:border-[var(--cb)]"
          />
          <button
            type="submit"
            data-testid="file-search-run"
            className="rounded-r-lg border border-[var(--cb)] bg-[var(--cbg)] px-2 py-1 text-xs text-[var(--mut)] hover:bg-[var(--cb)]"
          >
            搜索
          </button>
        </form>
        {loading && <span className="text-xs text-[var(--mut)]">加载中…</span>}
      </div>

      <div
        data-testid="panel-content"
        data-font-scale={fontScale}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      >
        {visible.length === 0 && !loading && (
          <p data-testid="panel-empty" className="py-12 text-center text-sm text-[var(--mut)]">
            {activeSearch || origin !== "all" ? "无匹配文件" : "该范围内未发现文件"}
          </p>
        )}
        <ul className={`space-y-1 ${loading && entries.length > 0 ? "opacity-60" : ""}`}>
          {visible.map((e, i) => (
            <li
              key={`${e.path}-${i}`}
              data-testid={`file-row-${i}`}
              className="rounded-lg px-2 py-1.5 hover:bg-[var(--cbg)] dark:hover:bg-[var(--btnp)]"
            >
              {/* 主行 = 末段文件名（加粗，即超链接）；次行 = 目录 + 时间 */}
              <button
                type="button"
                data-testid={`file-row-${i}-open`}
                onClick={() => onOpenFile(e.path)}
                className={`block w-full truncate text-left text-sm hover:underline ${
                  e.modified ? "font-semibold text-[var(--tx)]" : "font-medium text-[var(--mut)]"
                }`}
              >
                {fileKindOf(e.path) === "image" ? (
                  <ImageIcon size={12} className="mr-1 inline shrink-0" />
                ) : (
                  <FileText size={12} className="mr-1 inline shrink-0" />
                )}
                {fileBaseName(e.path)}
              </button>
              <div className="flex items-center gap-2 text-xs text-[var(--mut)]">
                {/* 来源徽标（M5 线稿 .from pill）：undefined（旧载荷）不渲染 */}
                {e.origin && (
                  <span
                    data-testid={`file-row-${i}-origin`}
                    className="flex-none rounded-full border border-[var(--cb)] px-1.5 py-px text-[10px] text-[var(--mut)]"
                  >
                    {originLabel[e.origin] ?? e.origin}
                  </span>
                )}
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
                    className="min-w-0 flex-1 truncate text-left hover:text-[var(--tx)] hover:underline dark:hover:text-[var(--mut)]"
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
            className="sticky bottom-0 mt-2 flex items-start gap-2 rounded-lg border border-[var(--cb)] bg-[var(--cbg)] p-2 shadow-lg"
          >
            <code className="min-w-0 flex-1 text-xs break-all text-[var(--tx)]">{pathPopover}</code>
            <button
              type="button"
              data-testid="path-popover-close"
              aria-label="关闭路径浮窗"
              onClick={() => setPathPopover(null)}
              className="shrink-0 rounded-full p-0.5 text-[var(--mut)] hover:bg-[var(--cb)] dark:hover:bg-[var(--btnp)]"
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
        {/* 可预览说明（M5 P2-a：类型与上限前置告知，配合 403 原因细分排障） */}
        <p
          data-testid="panel-preview-help"
          className="mt-3 border-t border-[var(--cb)] pt-2 text-[10px] leading-relaxed text-[var(--mut)]"
        >
          可预览：文本 / 代码 ≤500KB（md/html 支持渲染切换），图片 ≤5MB
          （png/jpg/jpeg/gif/webp/svg/bmp）；敏感目录不可预览，其余原因见报错提示。
        </p>
        {/* 带宽受限提示 + 满速升级指引（Task 10 §C5）：仅受限通道（隧道）就地出现，
            局域网直连与拉取失败都不渲染——升级指引不占通道卡片位（§C4 4 卡不变），
            只在用户真会遇到大文件传输时就地告知。判据/文案/有限正数守卫见
            ChannelBwNote（附件区复用同一份，评审 I-2） */}
        <ChannelBwNote channel={channel} testId="panel-bw-note" className="mt-3" />
      </div>
    </section>
  );
}
