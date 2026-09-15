import { useCallback, useEffect, useRef, useState } from "react";
import { fetchHost, fetchSessions, type HostPayload } from "./api";
import {
  STATUS_DOT_COLOR,
  TOOL_FILTERS,
  filterByAgent,
  formatRelativeTime,
  sortSessions,
  type ToolFilter,
} from "./board-logic";
import type { SessionsResponse } from "@/types/session";

const POLL_MS = 3000;

interface BoardProps {
  /** 首次成功拉到数据时回调（一次）：探测成功信号，App 由此把 paired null→true（已配对设备免重配） */
  onPaired: () => void;
  /** 轮询收到 403（设备失效）时回调：App 切回配对页 */
  onUnpaired: () => void;
}

// 移动看板：自持 3s 轮询（含挂载后首拍 = 探测），App 不再持有任何拉取逻辑。
// 失败口径：403 → 回配对页；网络异常（fetch reject，如服务器关闭）→ 保留上次数据 +
// 错误横幅继续重试（不白屏、不误踢回配对页，支撑"重启免重配"自愈）。
export default function Board({ onPaired, onUnpaired }: BoardProps) {
  const [data, setData] = useState<SessionsResponse | null>(null);
  const [loadError, setLoadError] = useState(false);
  // 页头品牌行（P8a/P8b）：host 信息运行期不变，挂载时拉一次即可，不随轮询重复。
  // 失败口径（与轮询不同）：拉取失败 / 403 一律静默降级为不显示——设备有效性只以
  // 会话轮询的 403 为准，品牌行只是展示层，不参与配对状态机
  const [host, setHost] = useState<HostPayload["host"] | null>(null);
  const [filter, setFilter] = useState<ToolFilter>("all");
  // 相对时长的基准时钟：随每拍轮询刷新（react-hooks/purity 禁止渲染期直接调 Date.now）
  const [now, setNow] = useState(() => Date.now());
  // 首拍成功通知只发一次：防每拍回调导致父级无谓重渲染；重挂载（403 后重配）时随组件自然复位
  const aliveRef = useRef(false);
  // in-flight 守卫：慢网下上一拍未返回时跳过新拍，防早发慢到的旧响应覆盖新数据
  const inFlightRef = useRef(false);

  const tick = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    try {
      const s = await fetchSessions<SessionsResponse>();
      if (s === null) {
        onUnpaired(); // 设备失效 → 回配对页
        return;
      }
      if (!aliveRef.current) {
        aliveRef.current = true;
        onPaired(); // 首拍成功（首拍 = 探测）：通知 App 配对仍有效，只发一次
      }
      setData(s);
      setNow(Date.now());
      setLoadError(false);
    } catch {
      // 网络异常：保持上次数据与上次时钟，仅提示重试中
      setLoadError(true);
    } finally {
      inFlightRef.current = false; // 无论成败都放行下一拍
    }
  }, [onPaired, onUnpaired]);

  useEffect(() => {
    void tick();
    const id = setInterval(() => void tick(), POLL_MS);
    // 卸载清理：停掉轮询，防止离页后继续请求
    return () => clearInterval(id);
  }, [tick]);

  // 品牌行数据：挂载时拉一次（host 信息不变，无需轮询；失败静默，见 state 注释）
  useEffect(() => {
    let alive = true;
    void fetchHost<HostPayload>()
      .then((h) => {
        if (alive && h !== null) setHost(h.host);
      })
      .catch(() => {
        /* 网络异常：品牌行静默不显示 */
      });
    return () => {
      alive = false;
    };
  }, []);

  const sessions = data ? sortSessions(filterByAgent(data.sessions, filter)) : [];
  // 过滤后无卡但总量不为 0 时，提示归因于过滤条件而非"真的没会话"
  const filteredOut = data !== null && data.totalCount > 0 && sessions.length === 0;

  return (
    <div className="min-h-screen bg-slate-950 px-4 py-4 text-slate-200">
      {/* 品牌行（P8a+P8b）：MAM + 版本号 + 本机名（右侧，双机双子域辨识）；
          host 未拉到时整行隐藏（静默降级，见上方 state 注释） */}
      {host && (
        <header className="flex items-center gap-2 px-4 pt-4 pb-2">
          <span className="text-lg font-bold">MAM</span>
          <span className="text-xs text-slate-400">v{host.version}</span>
          <span className="ml-auto text-sm">{host.name}</span>
        </header>
      )}
      <header className="mb-3 flex items-baseline justify-between">
        <h1 className="text-lg font-semibold text-slate-100">会话看板</h1>
        <span className="text-xs text-slate-500">
          {data ? `${data.totalCount} 个会话` : "加载中…"}
        </span>
      </header>

      {loadError && (
        <p className="mb-3 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-400">
          网络连接失败，正在自动重试…（当前展示上次数据）
        </p>
      )}

      {/* 工具过滤 chips：全部 + 八工具，横向可滚动 */}
      <div className="mb-3 flex gap-2 overflow-x-auto pb-1">
        {TOOL_FILTERS.map((f) => (
          <button
            key={f}
            type="button"
            onClick={() => setFilter(f)}
            className={
              filter === f
                ? "shrink-0 rounded-full bg-slate-100 px-3 py-1 text-xs font-medium text-slate-900"
                : "shrink-0 rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-400"
            }
          >
            {f === "all" ? "全部" : f}
          </button>
        ))}
      </div>

      {sessions.length === 0 ? (
        <p className="py-16 text-center text-sm text-slate-500">
          {filteredOut ? "该工具暂无会话" : "暂无会话"}
        </p>
      ) : (
        <ul className="space-y-2">
          {sessions.map((s) => (
            <li key={`${s.agentType}-${s.id}`} className="rounded-xl bg-slate-900 p-3">
              <div className="flex items-center gap-2">
                {/* 三色圆点：与桌面 StatusLight 同语义（waiting 附加呼吸动画） */}
                <span
                  className={`inline-block h-2.5 w-2.5 shrink-0 rounded-full ${STATUS_DOT_COLOR[s.status]} ${
                    s.status === "waiting" ? "animate-pulse" : ""
                  }`}
                />
                <span className="truncate text-sm font-medium text-slate-100">{s.projectName}</span>
                <span className="ml-auto shrink-0 text-xs text-slate-500">
                  {formatRelativeTime(s.lastActivityAt, now)}
                </span>
              </div>
              <p className="mt-1 truncate text-sm text-slate-300">{s.title ?? "（无标题）"}</p>
              {s.lastMessage && (
                <p className="mt-0.5 truncate text-xs text-slate-500">{s.lastMessage}</p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
