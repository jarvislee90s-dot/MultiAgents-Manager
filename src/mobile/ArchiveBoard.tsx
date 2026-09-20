import { useCallback, useEffect, useState } from "react";
import {
  deleteArchivedSession,
  fetchArchivedSessions,
  type ArchivedPayload,
  type ArchivedSession,
} from "./api";
import { filterArchivedByProject, filterArchivedByTool } from "./archive-logic";
// 复用既有导出（勿自造）：TOOL_LABELS 工具中文 / formatRelativeTime 相对时间
import { TOOL_LABELS, formatRelativeTime } from "./board-logic";

type Days = 1 | 3 | 7;

/** 工具 chips 文案：全部 / 工具中文（TOOL_LABELS，Record<AgentType,string> 以
 *  string 索引安全读——归档 agentType 是 string） */
function chipLabel(t: string): string {
  if (t === "all") return "全部";
  return (TOOL_LABELS as Record<string, string>)[t] ?? t;
}

/** 历史会话页（spec §7.2）：懒加载（进页 days=1，切天数重拉，页内不轮询——
 *  死数据静态）；双维筛选（工具 chips × 项目下拉）独立于活板选择；卡片无按钮
 *  （裁决 6——打开动作只在详情页）。 */
export default function ArchiveBoard({
  onBack,
  onOpenCard,
}: {
  onBack: () => void;
  onOpenCard: (s: ArchivedSession) => void;
}) {
  const [days, setDays] = useState<Days>(1);
  const [data, setData] = useState<ArchivedPayload | null>(null);
  const [error, setError] = useState(false);
  const [tool, setTool] = useState<string>("all");
  const [project, setProject] = useState<string>("all");
  // 相对时长基准时钟（react-hooks/purity 禁渲染期调 Date.now，同 Board 惯例）：
  // 挂载时取一次快照。历史页是死数据静态页（页内不轮询），时长冻结在进页时刻即可
  const [now] = useState(() => Date.now());

  const load = useCallback(async (d: Days) => {
    setError(false);
    try {
      const p = await fetchArchivedSessions(d);
      if (p === null) return; // 403：App 层配对态处理，此处静默
      setData(p);
    } catch {
      setError(true);
    }
  }, []);

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
            onClick={() => void load(days)}
          >
            刷新
          </button>
          <button
            type="button"
            data-testid="archive-clear"
            className="underline"
            onClick={() => {
              if (!window.confirm("清空全部归档记录？")) return;
              void deleteArchivedSession().then(() => void load(days));
            }}
          >
            清空归档
          </button>
        </span>
      </header>

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
      {!error && data && rows.length === 0 && (
        <p className="py-8 text-center text-sm text-slate-400">
          {days === 7
            ? "暂无归档记录"
            : days === 3
              ? "最近 3 天没有非活跃会话，可试 7 天"
              : "最近 1 天没有非活跃会话，可试 3 天 / 7 天"}
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
              <span className="text-sm font-medium">{s.projectName}</span>
              <span className="text-xs text-slate-400">
                {formatRelativeTime(s.lastSeenAt, now)}结束
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
