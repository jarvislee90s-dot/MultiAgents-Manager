import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { fetchHost, fetchSessions, type HostPayload } from "./api";
import {
  STATUS_DOT_COLOR,
  TOOL_BRAND_COLORS,
  TOOL_LABELS,
  filterByAgent,
  filterEnabledTools,
  formatRelativeTime,
  sortChipsByActivity,
  sortSessions,
  type ToolFilter,
} from "./board-logic";
import { ToolIcon } from "@/components/common/ToolIcon";
import type { AgentType, SessionsResponse } from "@/types/session";

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
  // P8d 受管工具名单：与 host 同源一次拉取（enabledTools 来自后端 dao::agent_tool）。
  // null = host 未到（竞态窗口）：chips 只显示「全部」，不猜全量八工具
  const [enabledTools, setEnabledTools] = useState<Set<string> | null>(null);
  const [filter, setFilter] = useState<ToolFilter>("all");
  // P8e 多行折叠：chips 内容超一行时折叠为一行 + 展开/收起按钮（无溢出无按钮）
  const [chipsOverflow, setChipsOverflow] = useState(false);
  const [chipsExpanded, setChipsExpanded] = useState(false);
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

  // 品牌行数据：挂载时拉一次（host 信息不变，无需轮询；失败静默，见 state 注释）。
  // enabledTools（P8d 受管名单）随同一载荷更新——host 拉取失败时保持 null，
  // chips 收敛为「全部」，与品牌行同口径静默降级
  useEffect(() => {
    let alive = true;
    void fetchHost<HostPayload>()
      .then((h) => {
        if (alive && h !== null) {
          setHost(h.host);
          setEnabledTools(new Set(h.enabledTools));
        }
      })
      .catch(() => {
        /* 网络异常：品牌行静默不显示，chips 保持「全部」 */
      });
    return () => {
      alive = false;
    };
  }, []);

  const sessions = data ? sortSessions(filterByAgent(data.sessions, filter)) : [];
  // 过滤后无卡但总量不为 0 时，提示归因于过滤条件而非"真的没会话"
  const filteredOut = data !== null && data.totalCount > 0 && sessions.length === 0;

  // P8d/P8e chips 集合（竞态收敛）：
  // - host 未到（enabledTools=null）→ 只有「全部」（不猜全量八工具）
  // - host 已到 → 「全部」 + filterEnabledTools(有卡工具 ∩ 受管)，
  //   再按各工具最新会话活跃时间降序（「全部」恒首位不参与排序）
  // 有卡工具集合从当前 sessions 推导（Set 去重；交集元素本就来自 AgentType 会话字段，
  // filterEnabledTools 的 string[] 签名在此收窄回 AgentType）；再走活跃排序
  const cardTools: AgentType[] = enabledTools
    ? (filterEnabledTools(
        [...new Set(data?.sessions.map((s) => s.agentType) ?? [])],
        enabledTools
      ) as AgentType[])
    : [];
  const sortedCardTools = sortChipsByActivity(cardTools, data?.sessions ?? []);

  // P8e 溢出测量：chips 行实际渲染高度 > 单行高度 ⇒ 折叠为可展开态。
  // useLayoutEffect 在首帧布局后同步测量，避免「先折叠后闪烁」；依赖 chips 数量：
  // host 晚到使 chips 从 1 个收敛为 N 个时重测（resize 之外的第二个测量时机，
  // 否则竞态窗口内测得的「未溢出」会一直滞后到下次 resize——移动 PWA 上 resize 罕见）。
  // jsdom 恒 0 → 不折叠（测试需 mock scrollHeight/clientHeight 后派发 resize 验证折叠分支）
  const chipsRowRef = useRef<HTMLDivElement>(null);
  const chipCount = sortedCardTools.length + 1; // +1 = 「全部」
  useLayoutEffect(() => {
    const measure = () => {
      const el = chipsRowRef.current;
      if (!el) return;
      setChipsOverflow(el.scrollHeight > el.clientHeight);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [chipCount]);

  return (
    <div className="min-h-screen bg-slate-950 px-4 py-4 text-slate-200">
      {/* 品牌行（P8a+P8b）：MAM + 版本号 + 本机名（右侧，双机双子域辨识）；
          host 未拉到时整行隐藏（静默降级，见上方 state 注释）。
          内层不再加 px-4（M3 Task 2 顺手修）：容器已有 px-4，双层内边距导致品牌行偏右 */}
      {host && (
        <header className="flex items-center gap-2 pt-4 pb-2">
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

      {/* 工具过滤 chips（P8d/P8e）：受管∩有卡 + 全部，按活跃排序；溢出时折叠为一行。
          data-testid 供测试精确定位（卡片主行也渲染工具名，全局文本匹配有歧义） */}
      <div
        ref={chipsRowRef}
        data-testid="tool-chips"
        className={`mb-3 flex items-center gap-2 pb-1 ${
          chipsOverflow && !chipsExpanded
            ? "flex-nowrap overflow-hidden" // 折叠态：压回一行，溢出部分隐藏
            : "flex-wrap" // 展开态 / 无溢出：自然换行
        }`}
      >
        <button
          type="button"
          onClick={() => setFilter("all")}
          className={
            filter === "all"
              ? "shrink-0 rounded-full bg-slate-100 px-3 py-1 text-xs font-medium text-slate-900"
              : "shrink-0 rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-400"
          }
        >
          全部
        </button>
        {sortedCardTools.map((tool) => {
          // 品牌色 chip：选中态 = 品牌色实底 + 白字（八色均够深，白字对比度足够）；
          // 未选中态 = 品牌色 12% 透明度淡化底（8 位 hex 追加 1F alpha，免 color-mix 的
          // Tailwind v4 注册环节，选 style 内联为最简实现）+ 品牌色字
          const brand = TOOL_BRAND_COLORS[tool];
          const selected = filter === tool;
          return (
            <button
              key={tool}
              type="button"
              onClick={() => setFilter(tool)}
              className="shrink-0 rounded-full px-3 py-1 text-xs"
              style={
                selected
                  ? { backgroundColor: brand, color: "#ffffff" }
                  : { backgroundColor: `${brand}1F`, color: brand }
              }
            >
              {TOOL_LABELS[tool]}
            </button>
          );
        })}
        {chipsOverflow && (
          <button
            type="button"
            onClick={() => setChipsExpanded((v) => !v)}
            className="shrink-0 rounded-full bg-slate-800 px-3 py-1 text-xs text-slate-400"
          >
            {chipsExpanded ? "收起" : "展开"}
          </button>
        )}
      </div>

      {sessions.length === 0 ? (
        <p className="py-16 text-center text-sm text-slate-500">
          {filteredOut ? "该工具暂无会话" : "暂无会话"}
        </p>
      ) : (
        <ul className="space-y-2">
          {sessions.map((s) => (
            <li key={`${s.agentType}-${s.id}`} className="rounded-xl bg-slate-900 p-3">
              {/* 主行（P8c 定稿）：工具图标+工具名+项目名 … 相对时长+状态点（右端）。
                  相对时长保留在主行右端（状态点左边）：时间/状态属卡片级元数据，
                  一眼可读，且沿用旧版"时长在右"的视觉惯性 */}
              <div className="flex items-center gap-2">
                {/* 桌面组件但零 Tauri 依赖，移动 bundle 可直接 import（控制者裁决） */}
                <ToolIcon toolId={s.agentType} size={16} className="shrink-0" />
                <span className="shrink-0 text-sm font-medium text-slate-100">
                  {TOOL_LABELS[s.agentType]}
                </span>
                <span className="truncate text-sm text-slate-400">{s.projectName}</span>
                <span className="ml-auto shrink-0 text-xs text-slate-500">
                  {formatRelativeTime(s.lastActivityAt, now)}
                </span>
                {/* 三色圆点：与桌面 StatusLight 同语义（waiting 附加呼吸动画） */}
                <span
                  className={`inline-block h-2.5 w-2.5 shrink-0 rounded-full ${STATUS_DOT_COLOR[s.status]} ${
                    s.status === "waiting" ? "animate-pulse" : ""
                  }`}
                />
              </div>
              {/* 副行（P8c 定稿）：标题+最新消息预览（现有数据重新排布，无新 API） */}
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
