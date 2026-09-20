import { useCallback, useEffect, useState } from "react";
import {
  deleteArchivedSession,
  fetchArchivedSessions,
  type ArchivedPayload,
  type ArchivedSession,
} from "./api";
import { chipLabel, filterArchivedByProject, filterArchivedByTool } from "./archive-logic";
// 复用既有导出（勿自造）：formatRelativeTime 相对时间
import { formatRelativeTime } from "./board-logic";

type Days = 1 | 3 | 7;

/** 历史会话页（spec §7.2）：懒加载（进页 days=1，切天数重拉，页内不轮询——
 *  死数据静态）；双维筛选（工具 chips × 项目下拉）独立于活板选择；卡片无按钮
 *  （裁决 6——打开动作只在详情页）。 */
export default function ArchiveBoard({
  onBack,
  onOpenCard,
  onUnpaired,
}: {
  onBack: () => void;
  onOpenCard: (s: ArchivedSession) => void;
  /** 403（配对态在历史页内过期）→ 透传 App 置 paired=false 回配对页（Board 同款） */
  onUnpaired: () => void;
}) {
  const [days, setDays] = useState<Days>(1);
  const [data, setData] = useState<ArchivedPayload | null>(null);
  const [error, setError] = useState(false);
  // 清空归档失败的行内反馈（终审 Finding 3：Promise 拒绝无 .catch → 按钮无反应 +
  // unhandled rejection）。不复用 error（那是加载失败语义，会带出 retry 按钮误导）；
  // 下次操作或刷新时清除
  const [manageError, setManageError] = useState(false);
  const [tool, setTool] = useState<string>("all");
  const [project, setProject] = useState<string>("all");
  // 相对时长基准时钟（react-hooks/purity 禁渲染期调 Date.now，同 Board 惯例）：
  // 挂载时取一次快照。历史页是死数据静态页（页内不轮询），时长冻结在进页时刻即可
  const [now] = useState(() => Date.now());

  const load = useCallback(
    async (d: Days) => {
      setError(false);
      try {
        const p = await fetchArchivedSessions(d);
        if (p === null) {
          // 403：配对态在本页过期 → 透传 App 回配对页（评审 Minor：原静默吞掉，
          // 用户停在历史页假活状态）
          onUnpaired();
          return;
        }
        setData(p);
      } catch {
        setError(true);
      }
    },
    [onUnpaired],
  );

  useEffect(() => {
    void load(days);
  }, [days, load]);

  const rows = data
    ? filterArchivedByProject(filterArchivedByTool(data.archived, tool), project)
    : [];
  // chips = 「全部」+ 当前窗口出现过的工具（动态——与活板 chips 的受管∩有卡不同，
  // 归档以结果集为准）
  const tools = ["all", ...new Set((data?.archived ?? []).map((s) => s.agentType))];

  return (
    <div className="mx-auto max-w-3xl px-4">
      <header className="mb-3 flex items-center justify-between pt-4">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onBack}
            className="text-sm text-slate-500"
            aria-label="返回看板"
          >
            ‹ 返回
          </button>
          <h1 className="text-lg font-semibold">历史会话</h1>
        </div>
        <span className="flex items-center gap-2 text-xs text-slate-400">
          <button
            type="button"
            data-testid="archive-refresh"
            className="underline"
            onClick={() => {
              setManageError(false);
              void load(days);
            }}
          >
            刷新
          </button>
          <button
            type="button"
            data-testid="archive-clear"
            className="underline"
            onClick={() => {
              if (!window.confirm("清空全部归档记录？")) return;
              setManageError(false);
              deleteArchivedSession()
                .then(() => void load(days))
                .catch(() => setManageError(true));
            }}
          >
            清空归档
          </button>
        </span>
      </header>

      {/* 清空归档失败反馈（终审 Finding 3）：小号红字贴近操作点，不触发 retry */}
      {manageError && (
        <p
          data-testid="archive-manage-error"
          className="mb-2 text-right text-xs text-red-600"
        >
          操作失败，请重试
        </p>
      )}

      {/* 第一行：工具 chips（文案复用 TOOL_LABELS；选中态样式从简，品牌色后续按需） */}
      <div className="mb-2 flex gap-2 overflow-x-auto">
        {tools.map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTool(t)}
            className={`rounded-full px-3 py-1 text-xs ${t === tool ? "bg-blue-600 text-white" : "bg-slate-200 text-slate-700 dark:bg-slate-800 dark:text-slate-300"}`}
          >
            {chipLabel(t)}
          </button>
        ))}
      </div>

      {/* 第二行：项目下拉 + 天数分段 */}
      <div className="mb-3 flex gap-2">
        <select
          value={project}
          onChange={(e) => setProject(e.target.value)}
          className="min-w-0 flex-1 rounded-lg border border-slate-200 px-2 py-1.5 text-sm dark:border-slate-800"
          aria-label="按项目筛选"
        >
          <option value="all">项目：全部</option>
          {(data?.projects ?? []).map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <span className="flex gap-1">
          {([1, 3, 7] as Days[]).map((d) => (
            <button
              key={d}
              type="button"
              data-testid={`archive-days-${d}`}
              onClick={() => setDays(d)}
              className={`rounded-lg px-2.5 py-1.5 text-xs ${d === days ? "bg-blue-600 text-white" : "bg-slate-200 text-slate-700 dark:bg-slate-800 dark:text-slate-300"}`}
            >
              {d}天
            </button>
          ))}
        </span>
      </div>

      {error && (
        <div className="py-8 text-center text-sm text-slate-500">
          加载失败
          <button
            type="button"
            data-testid="archive-retry"
            className="ml-2 underline"
            onClick={() => void load(days)}
          >
            重试
          </button>
        </div>
      )}
      {!error && data === null && (
        <p className="py-8 text-center text-sm text-slate-400">加载中…</p>
      )}
      {!error && data && rows.length === 0 && (
        <p className="py-8 text-center text-sm text-slate-400">
          {tool === "all" && project === "all"
            ? days === 7
              ? "暂无归档记录"
              : days === 3
                ? "最近 3 天没有非活跃会话，可试 7 天"
                : "最近 1 天没有非活跃会话，可试 3 天 / 7 天"
            : // 有筛选残留（常见：选定项目后切小窗，新窗口不含该项目）——扩窗文案
              // 会误导（扩了也没有），改显筛选语义（评审 Minor）
              "当前筛选无匹配会话"}
        </p>
      )}
      <div className="flex flex-col gap-2 pb-8">
        {rows.map((s) => (
          <button
            key={s.sessionId}
            type="button"
            onClick={() => onOpenCard(s)}
            className="rounded-xl border border-slate-200 px-3 py-2.5 text-left enabled:hover:bg-slate-50 dark:border-slate-800"
          >
            <div className="flex items-baseline justify-between">
              <span className="flex min-w-0 items-center gap-1.5">
                {/* 软归档活会话徽标（体验批二）：未结束、看板隐藏中，详情页可移回 */}
                {s.hiddenAlive && (
                  <span
                    data-testid="alive-badge"
                    className="shrink-0 rounded-full bg-green-100 px-1.5 py-0.5 text-[10px] text-green-700 dark:bg-green-900/40 dark:text-green-300"
                  >
                    未结束
                  </span>
                )}
                <span className="truncate text-sm font-medium">{s.projectName}</span>
              </span>
              <span className="shrink-0 text-xs text-slate-400">
                {formatRelativeTime(s.lastSeenAt, now)}
                {s.hiddenAlive ? "活跃" : "结束"}
              </span>
            </div>
            <div className="mt-0.5 text-xs text-slate-500">
              {chipLabel(s.agentType)} · {s.title ?? "（无标题）"}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
