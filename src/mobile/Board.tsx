import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { Moon, Sun, Volume2, VolumeX } from "lucide-react";
import { connectEvents, fetchHost, fetchSessions, type HostPayload } from "./api";
import { getInitialTheme, toggleTheme, type Theme } from "./theme";
import { getSoundEnabled, playCompletionChime, toggleSoundEnabled } from "./sound";
import {
  CHIP_LIGHT_TEXT_FACTOR,
  STATUS_COLOR_KIND,
  STATUS_DOT_COLOR,
  TOOL_BRAND_COLORS,
  TOOL_BRAND_COLORS_DARK,
  TOOL_LABELS,
  applyTransition,
  darkenHex,
  filterByAgent,
  filterEnabledTools,
  formatRelativeTime,
  formatTransition,
  sortChipsByActivity,
  sortSessions,
  type ToolFilter,
} from "./board-logic";
import { ToolIcon } from "@/components/common/ToolIcon";
import type { AgentType, Session, SessionsResponse, TransitionEvent } from "@/types/session";

const POLL_MS = 3000;
/** SSE 模式低频对账周期（评审修复 R1）：transition 只更新已存在卡，新会话成员资格
 *  靠本周期一次全量拉取兜底。30s = 成员资格变化的最大可见延迟，远低于实时性要求，
 *  又不会对服务端构成轮询压力 */
const RECONCILE_MS = 30_000;
/** 相对时长的基准时钟刷新间隔：SSE 模式下无每拍拉取，时钟仍须走动
 *  （否则卡片上的「3 分钟前」会冻结在挂载时刻）。纯前端重算，零网络开销 */
const CLOCK_MS = 30_000;
/** 跃迁横幅存活时长（毫秒）：几秒后自动消失，不长期占据看板顶部 */
const BANNER_TTL_MS = 4000;
/** 同屏横幅上限：突发跃迁（批量会话同时变化）时不淹没会话列表 */
const MAX_BANNERS = 3;

// P8e 折叠高度上限（溢出判定与裁剪样式的单一来源，fix round 1）：
// 36px = 单行 chips（24px）+ 行纵距（12px）——恰容纳一行、第二行起点恰在 36px 被完全裁掉
// （不留残影）。折叠态以此为固定 max-height（brief 明确要求）：auto-height 容器下
// scrollHeight === clientHeight 恒等，溢出判定会恒 false（折叠死功能）。
// 不变式（chips 行须用 gap-y-3）：chipHeight + rowGap ≥ 上限 ≥ chipHeight + paddingBottom，
// 下界防宽屏单行被误裁，上界防折叠态露出第二行残影。字号增大时两边界同向放宽，仍成立
const CHIPS_COLLAPSED_HEIGHT = 36;

/** 跃迁横幅条目：key = `工具-会话id`（展示层防叠键，见 pushBanner 注释） */
interface TransitionBanner {
  key: string;
  text: string;
}

// 提醒音（M3 Task 6 提醒三件套之二）：合成音实现与开关已抽到 ./sound.ts
// （两音上行 + 包络；复用桌面 12 音效资产不可行——资产不在移动产物内，见该文件注释）。
// 本组件只负责**何时响**：见 handleTransition 的转绿过滤 + 5 秒同色去重，
// 口径照抄桌面 hooks/useNotification（currColor === "green" + lastNotified）。
//
// 跃迁横幅条目：key = `工具-会话id`（展示层防叠键，见 pushBanner 注释）

interface BoardProps {
  /** 首次成功拉到数据时回调（一次）：探测成功信号，App 由此把 paired null→true（已配对设备免重配） */
  onPaired: () => void;
  /** 收到 403（设备失效）时回调：App 切回配对页 */
  onUnpaired: () => void;
  /** 卡片点击回调（M3 Task 8）：进入会话详情；缺省时卡片不可点（既有测试/用法不受影响） */
  onOpenSession?: (session: Session) => void;
  /** 历史入口点击回调（历史会话区 spec §7.1）：进入归档历史页 */
  onOpenHistory: () => void;
}

// 移动看板：主通道为 SSE（快照首帧 + 跃迁增量），断流 2 次降级为 3s 轮询。
// 数据流：connectEvents 的 snapshot → setData（含挂载探测）；transition → 卡片实时刷新
// + 横幅/提示音/振动提醒；降级 → 交给下方轮询 effect（复用 tick 的 in-flight 守卫）。
// 失败口径：403 → 回配对页（只由 fetchSessions 的 null 触发，SSE 断流不算）；
// 网络异常 → 保留上次数据 + 错误横幅继续重试（不白屏、不误踢回配对页）。
export default function Board({ onPaired, onUnpaired, onOpenSession, onOpenHistory }: BoardProps) {
  const [data, setData] = useState<SessionsResponse | null>(null);
  const [loadError, setLoadError] = useState(false);
  // SSE 已降级（连续 2 次失败）：单向闩——置位后由轮询 effect 接管数据拉取；
  // 不做「轮询期间试回 SSE」（YAGNI，M3 不要求；服务端恢复后刷新页面即重连）
  const [degraded, setDegraded] = useState(false);
  // 跃迁横幅（提醒三件套之一）：SSE transition 边沿驱动，见 pushBanner
  const [banners, setBanners] = useState<TransitionBanner[]>([]);
  // 页头品牌行（P8a/P8b）：host 信息运行期不变，挂载时拉一次即可，不随轮询重复。
  // 失败口径（与轮询不同）：拉取失败 / 403 一律静默降级为不显示——设备有效性只以
  // 会话拉取的 403 为准，品牌行只是展示层，不参与配对状态机
  const [host, setHost] = useState<HostPayload["host"] | null>(null);
  // P8d 受管工具名单：与 host 同源一次拉取（enabledTools 来自后端 dao::agent_tool）。
  // null = host 未到（竞态窗口）：chips 只显示「全部」，不猜全量八工具
  const [enabledTools, setEnabledTools] = useState<Set<string> | null>(null);
  const [filter, setFilter] = useState<ToolFilter>("all");
  // P8e 多行折叠：chips 内容超一行时折叠为一行 + 展开/收起按钮（无溢出无按钮）。
  // chipsExpanded 跨溢出周期保留（Minor ③ 行为与注释对齐）：溢出消失时按钮隐藏但展开态
  // 不回写——内容本就放得下，保持展开无副作用；下次溢出时沿用用户上次的选择
  const [chipsOverflow, setChipsOverflow] = useState(false);
  const [chipsExpanded, setChipsExpanded] = useState(false);
  // 相对时长的基准时钟：数据变化时刷新 + CLOCK_MS 定时走动（react-hooks/purity
  // 禁止渲染期直接调 Date.now）
  const [now, setNow] = useState(() => Date.now());
  // P8f 日/夜双皮肤：初值取 getInitialTheme（localStorage > 系统偏好 > 默认 dark），
  // 与 mobile.html 防闪白脚本、main.tsx applyInitialTheme 三处同源同优先级。
  // 真实 DOM 类由 toggleTheme 直接切（非渲染派生），本 state 仅驱动按钮图标/可达名
  const [theme, setTheme] = useState<Theme>(() => getInitialTheme());
  // 提示音总开关（2026-09-19）：初值从 localStorage 读；只控提示音，不影响横幅/振动
  const [soundOn, setSoundOn] = useState<boolean>(() => getSoundEnabled());
  // 上次实际响铃记录：同会话 5 秒内重复翻转到绿色只响一次（防状态抖动连响）。
  // 口径照抄桌面 useNotification 的 lastNotified——消费者侧 UI 层防抖兜底，
  // 与 SessionWatcher 层的跃迁去重（铁律 4）不冲突：那层去的是「状态边沿」，
  // 这层去的是「同一会话在短时间内反复回到绿」的重复提醒
  const lastChimed = useRef<Map<string, number>>(new Map());
  // 首个成功快照/首拍只发一次 onPaired：防重复回调导致父级无谓重渲染；
  // 重挂载（403 后重配）时随组件自然复位
  const aliveRef = useRef(false);
  // in-flight 守卫：慢网下上一拍未返回时跳过新拍，防早发慢到的旧响应覆盖新数据
  const inFlightRef = useRef(false);
  // 横幅自动消失定时器（键 = 横幅 key）：同会话新跃迁覆盖旧横幅时须撤销旧定时器，
  // 否则旧定时器会提前清掉新横幅
  const bannerTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  /** 首个成功快照/首拍：探测信号只发一次（SSE 快照与降级轮询两路共用，防语义分叉） */
  const notifyPairedOnce = useCallback(() => {
    if (aliveRef.current) return;
    aliveRef.current = true;
    onPaired(); // 首拍成功（首拍 = 探测）：通知 App 配对仍有效，只发一次
  }, [onPaired]);

  /** SSE 首帧快照（也是断线重连后的基线校正）：全量替换看板数据 */
  const handleSnapshot = useCallback(
    (s: SessionsResponse) => {
      notifyPairedOnce();
      setData(s);
      setNow(Date.now());
      setLoadError(false);
    },
    [notifyPairedOnce]
  );

  /** 跃迁横幅入列（提醒三件套之一：横幅）。
   *  「同 sessionId 覆盖」是**展示层防叠**，不是数据去重——数据去重的唯一来源是服务端
   *  watcher（铁律 4），事件来一条我们显一条、音也响一次；这里只是保证同一会话的
   *  连续跃迁不会堆出多条横幅（后到的替换先到的，各自计时独立重置） */
  const pushBanner = useCallback((ev: TransitionEvent) => {
    // key 取 (工具, 会话 id)：与 watcher diff 同一唯一性口径——会话 id 只在工具内唯一，
    // 跨工具撞 id 不得互相顶掉横幅
    const key = `${ev.agentType}-${ev.sessionId}`;
    const text = formatTransition(ev);
    setBanners((prev) =>
      [{ key, text }, ...prev.filter((b) => b.key !== key)].slice(0, MAX_BANNERS)
    );
    const old = bannerTimers.current.get(key);
    if (old) clearTimeout(old);
    bannerTimers.current.set(
      key,
      setTimeout(() => {
        bannerTimers.current.delete(key);
        setBanners((prev) => prev.filter((b) => b.key !== key));
      }, BANNER_TTL_MS)
    );
  }, []);

  /** 状态跃迁（watcher 已去重的边沿，铁律 4：本层不独立去重，来一条处理一条）：
   *  卡片实时刷新（F1.3「SSE 实时刷新」）+ 提醒三件套（横幅 / 提示音 / 振动）。
   *  **提示音只在「变为绿」（任务完成）时响**（2026-09-19 用户裁决，与桌面端统一）：
   *  按**目标颜色**判定——red→green 与 yellow→green 均响（口径同桌面
   *  useNotification 的 currColor === "green"）；黄态细分跃迁
   *  （processing↔thinking↔compacting）不响。修正前对任意状态值变化无条件响，
   *  一轮回合内多次黄态细分跃迁会连响数次（用户实测为噪声）。
   *  横幅与振动保持「每条跃迁都提醒」不变——它们是最低打扰的通道。 */
  const maybeChime = useCallback(
    (ev: TransitionEvent) => {
      if (!soundOn) return;
      if (STATUS_COLOR_KIND[ev.to] !== "green") return;
      const last = lastChimed.current.get(ev.sessionId);
      const nowMs = Date.now();
      if (last !== undefined && nowMs - last < 5000) return; // 5 秒同会话去重
      lastChimed.current.set(ev.sessionId, nowMs);
      playCompletionChime();
    },
    [soundOn]
  );

  const handleTransition = useCallback(
    (ev: TransitionEvent) => {
      // 命中会话才换数组引用；未命中（新会话 / 已消失 / 坏状态串）返回原引用，
      // setData 走引用相等短路零重渲染
      setData((prev) => (prev ? { ...prev, sessions: applyTransition(prev.sessions, ev) } : prev));
      setNow(Date.now());
      pushBanner(ev);
      maybeChime(ev);
      // 振动（能力检测）：桌面浏览器与 iOS Safari 均无此 API，缺失即跳过
      navigator.vibrate?.(200);
    },
    [pushBanner, maybeChime]
  );

  const tick = useCallback(async () => {
    if (inFlightRef.current) return;
    inFlightRef.current = true;
    try {
      const s = await fetchSessions<SessionsResponse>();
      if (s === null) {
        onUnpaired(); // 设备失效 → 回配对页（**唯一判废通道**：SSE 断流不触发）
        return;
      }
      notifyPairedOnce();
      setData(s);
      setNow(Date.now());
      setLoadError(false);
    } catch {
      // 网络异常：保持上次数据与上次时钟，仅提示重试中
      setLoadError(true);
    } finally {
      inFlightRef.current = false; // 无论成败都放行下一拍
    }
  }, [notifyPairedOnce, onUnpaired]);

  // SSE 主通道（M3 Task 6）+ 30s 低频对账（评审修复 R1）：挂载即连，卸载即断（→ 服务端
  // Receiver 归零 → watcher 停扫，Task 5 守卫闭环）。connectEvents 的回调全部是稳定引用
  // （useCallback 空依赖链路 + setXxx），tick 亦稳定（App 侧 onPaired/onUnpaired 为
  // useCallback 常量），本 effect 只随组件生命周期跑一次
  useEffect(() => {
    const stopEvents = connectEvents(handleSnapshot, handleTransition, () => setDegraded(true));
    // 30s 低频对账（Critical 评审修复）：transition 只更新已存在卡（watcher 对**新增**
    // 会话不发事件），SSE 模式又无周期快照——新会话的成员资格（上卡/下卡）在事件流里
    // 是冻结的。低频 tick 全量拉取兜底；in-flight 守卫 / 403→onUnpaired / 错误横幅语义
    // 由 tick 单点保留。降级后本 interval 与 3s 轮询并存无害（tick 幂等 + in-flight 守卫
    // 防重叠），不为此引入 degraded 分支——那会让本 effect 重跑、SSE 重连，得不偿失
    const reconcile = setInterval(() => void tick(), RECONCILE_MS);
    return () => {
      clearInterval(reconcile);
      stopEvents(); // 既有 close 逻辑原样保留（stopped 闩 + 关连接 + 清重连定时器）
    };
  }, [handleSnapshot, handleTransition, tick]);

  // 降级轮询（仅在 SSE 连续 2 次失败后启用）：复用既有 tick（in-flight 守卫、
  // 403→onUnpaired、错误横幅语义单点保留）。SSE 模式（未降级）下本 effect 不装定时器，
  // 数据全由事件流驱动
  useEffect(() => {
    if (!degraded) return;
    void tick(); // 降级即刻补一拍：断流期间可能已有状态变化，不等 3s
    const id = setInterval(() => void tick(), POLL_MS);
    return () => clearInterval(id);
  }, [degraded, tick]);

  // 基准时钟走动：SSE 模式下没有每拍拉取，相对时长仍须随时间前进
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), CLOCK_MS);
    return () => clearInterval(id);
  }, []);

  // 横幅定时器清理（卸载）：防离页后 setState 警告与定时器泄漏
  useEffect(() => {
    const timers = bannerTimers.current;
    return () => {
      for (const t of timers.values()) clearTimeout(t);
      timers.clear();
    };
  }, []);

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

  // P8d filter 残留回落（Minor ⑤）：chips 集合随会话收敛，先前选中的工具可能整体消失
  // （卡全结束）。残留 filter 会让看板停在空列表且无对应 chip 可取消高亮 ⇒ 回落「全部」。
  // 依赖签名串做成员判断（工具 id 无逗号，split 为其精确逆运算），避免依赖数组身份
  // 导致每拍空转；用 effect 而非渲染期 setState（react-hooks/purity）
  const chipsSignature = sortedCardTools.join(",");
  useEffect(() => {
    if (filter === "all") return;
    if (!chipsSignature.split(",").includes(filter)) setFilter("all");
  }, [chipsSignature, filter]);

  // P8e 溢出测量（fix round 1 重写，Critical）：判据 = 内容自然高度 > 折叠高度上限。
  // 关键机理：容器在「未裁剪」时 scrollHeight 即内容自然高度；折叠后虽有 max-height 裁剪，
  // scrollHeight 仍报告内容自然高度（真实浏览器实测：自然 96 / 裁剪后仍 96，clientHeight 才降为
  // 36）——因此该判据在两种状态下给出同一答案，无循环依赖，折叠→展开→再折叠稳定。
  // 旧实现用 nowrap+overflow-hidden 无固定高度：scrollHeight === clientHeight 恒等，
  // 判定恒 false（评审实测 101/101），折叠加按钮形同虚设。
  // 测量时机：useLayoutEffect（首帧布局后同步，免闪烁）+ resize + chips 集合签名变化
  // （host 晚到使集合从 1 收敛为 N；签名而非裸数量——同数量不同集合也要重测，Minor ④）
  const chipsRowRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const measure = () => {
      const el = chipsRowRef.current;
      if (!el) return;
      setChipsOverflow(el.scrollHeight > CHIPS_COLLAPSED_HEIGHT);
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [chipsSignature]);

  return (
    <div className="min-h-screen bg-white px-4 py-4 text-slate-800 dark:bg-slate-950 dark:text-slate-200">
      {/* 品牌行（P8a+P8b）：MAM + 版本号 + 本机名（右侧，双机双子域辨识）；
          host 未拉到时整行隐藏（静默降级，见上方 state 注释）。
          内层不再加 px-4（M3 Task 2 顺手修）：容器已有 px-4，双层内边距导致品牌行偏右 */}
      {host && (
        <header className="flex items-center gap-2 pt-4 pb-2">
          <span className="text-lg font-bold">MAM</span>
          <span className="text-xs text-slate-500 dark:text-slate-400">v{host.version}</span>
          <span className="ml-auto text-sm">{host.name}</span>
        </header>
      )}
      {/* 标题行右端常驻 P8f 主题切换 + 提示音开关（不随 host 拉取成败进退）：
          图标 + 可达名描述「点下去切到什么」，aria-label 供测试与无障碍精确定位。
          会话计数在窄屏会让位（主题/音效两个按钮优先）——计数是信息、按钮是操作 */}
      <header className="mb-3 flex items-baseline justify-between">
        <h1 className="text-lg font-semibold text-slate-900 dark:text-slate-100">会话看板</h1>
        <span className="flex items-center gap-2">
          <button
            type="button"
            onClick={onOpenHistory}
            className="rounded-lg border border-slate-200 px-2 py-1 text-xs text-slate-600 enabled:hover:bg-slate-100 dark:border-slate-800 dark:text-slate-300"
            aria-label="历史会话"
          >
            🕘 历史
          </button>
          <span className="hidden text-xs text-slate-500 sm:inline">
            {data ? `${data.totalCount} 个会话` : "加载中…"}
          </span>
          <button
            type="button"
            data-testid="sound-toggle"
            onClick={() => setSoundOn(toggleSoundEnabled())}
            aria-label={soundOn ? "关闭完成提示音" : "开启完成提示音"}
            aria-pressed={soundOn}
            title={soundOn ? "完成提示音：开" : "完成提示音：关"}
            className="rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            {soundOn ? <Volume2 size={16} /> : <VolumeX size={16} />}
          </button>
          <button
            type="button"
            onClick={() => setTheme(toggleTheme())}
            aria-label={theme === "dark" ? "切换到浅色模式" : "切换到深色模式"}
            className="rounded-full p-1 text-slate-500 hover:bg-slate-200 dark:text-slate-400 dark:hover:bg-slate-800"
          >
            {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
          </button>
        </span>
      </header>

      {loadError && (
        <p className="mb-3 rounded-lg bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
          网络连接失败，正在自动重试…（当前展示上次数据）
        </p>
      )}

      {/* 跃迁横幅（M3 Task 6 提醒三件套之一）：SSE transition 边沿驱动，展示
          「工具 · 项目 · 前态 → 后态 · 消息预览」，BANNER_TTL_MS 后自动消失。
          与 loadError 横幅并列而非互斥——断流恢复期的跃迁提醒仍有意义；
          aria-live=polite 供读屏器播报（看板是信息类界面，不用 assertive 打断）*/}
      {banners.length > 0 && (
        <ul aria-live="polite" data-testid="transition-banners" className="mb-3 space-y-1">
          {banners.map((b) => (
            <li
              key={b.key}
              className="truncate rounded-lg bg-sky-500/10 px-3 py-2 text-xs text-sky-700 dark:text-sky-400"
            >
              {b.text}
            </li>
          ))}
        </ul>
      )}

      {/* 工具过滤 chips（P8d/P8e）：受管∩有卡 + 全部，按活跃排序；溢出时折叠为一行。
          结构（fix round 1，Important）：展开/收起按钮是裁剪行的**兄弟节点**——
          旧实现把按钮放在裁剪行内，折叠态横向溢出时按钮整体被裁到屏外不可达。
          折叠只作用于 chips 行自身（max-height 纵向裁「第二行起」，保留 flex-wrap，
          不再用 nowrap 横向裁行尾）。data-testid 供测试精确定位 */}
      <div className="mb-3 flex items-start gap-2">
        <div
          ref={chipsRowRef}
          data-testid="tool-chips"
          className="flex min-w-0 flex-1 flex-wrap content-start items-center gap-x-2 gap-y-3"
          style={
            chipsOverflow && !chipsExpanded
              ? { maxHeight: CHIPS_COLLAPSED_HEIGHT, overflow: "hidden" }
              : undefined
          }
        >
          <button
            type="button"
            aria-pressed={filter === "all"}
            onClick={() => setFilter("all")}
            className={
              filter === "all"
                ? "shrink-0 rounded-full bg-slate-800 px-3 py-1 text-xs font-medium text-white dark:bg-slate-100 dark:text-slate-900"
                : "shrink-0 rounded-full bg-slate-200 px-3 py-1 text-xs text-slate-600 dark:bg-slate-800 dark:text-slate-400"
            }
          >
            全部
          </button>
          {sortedCardTools.map((tool) => {
            // 品牌色 chip：选中态 = 品牌色实底 + 白字；未选中态 = 品牌色 12% 透明度
            // 淡化底（8 位 hex 追加 1F alpha）+ 品牌色字。
            // P8f 浅色态修正：品牌原色当字在浅底上仅 1.96–3.84（不足 WCAG AA 4.5），
            // 故浅色态文字色压暗（darkenHex 系数 0.6 → 4.93–8.00 全达标）。
            // Bug 4（M3 验收）：暗色态原样用原色实测 1.08–5.41（kimi/claude/zcode/
            // openclaw/dsh 融底），文字色改查暗色表 TOOL_BRAND_COLORS_DARK；选中态
            // chip 在暗色下加白色轮廓——kimi 原色 #0B0E1A 作选中实底与深色卡底
            // #0f172a 几乎同色（1.08），八色统一轮廓解决深底融边
            const brand = TOOL_BRAND_COLORS[tool];
            const selected = filter === tool;
            const chipTextColor =
              theme === "dark"
                ? TOOL_BRAND_COLORS_DARK[tool]
                : darkenHex(brand, CHIP_LIGHT_TEXT_FACTOR);
            return (
              <button
                key={tool}
                type="button"
                aria-pressed={selected}
                onClick={() => setFilter(tool)}
                className="shrink-0 rounded-full px-3 py-1 text-xs"
                style={
                  selected
                    ? {
                        backgroundColor: brand,
                        color: "#ffffff",
                        ...(theme === "dark"
                          ? { boxShadow: "0 0 0 1px rgba(255,255,255,0.35)" }
                          : {}),
                      }
                    : { backgroundColor: `${brand}1F`, color: chipTextColor }
                }
              >
                {TOOL_LABELS[tool]}
              </button>
            );
          })}
        </div>
        {chipsOverflow && (
          <button
            type="button"
            onClick={() => setChipsExpanded((v) => !v)}
            className="shrink-0 rounded-full bg-slate-200 px-3 py-1 text-xs text-slate-600 dark:bg-slate-800 dark:text-slate-400"
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
            <li
              key={`${s.agentType}-${s.id}`}
              onClick={onOpenSession ? () => onOpenSession(s) : undefined}
              className={`rounded-xl border border-slate-200 bg-slate-100 p-3 dark:border-white/15 dark:bg-slate-900 ${
                onOpenSession ? "cursor-pointer" : ""
              }`}
            >
              {/* 主行（P8c 定稿）：工具图标+工具名+项目名 … 相对时长+状态点（右端）。
                  相对时长保留在主行右端（状态点左边）：时间/状态属卡片级元数据，
                  一眼可读，且沿用旧版"时长在右"的视觉惯性 */}
              <div className="flex items-center gap-2">
                {/* 桌面组件但零 Tauri 依赖，移动 bundle 可直接 import（控制者裁决） */}
                <ToolIcon toolId={s.agentType} size={16} className="shrink-0" />
                <span className="shrink-0 text-sm font-medium text-slate-900 dark:text-slate-100">
                  {TOOL_LABELS[s.agentType]}
                </span>
                <span className="truncate text-sm text-slate-600 dark:text-slate-400">
                  {s.projectName}
                </span>
                <span className="ml-auto shrink-0 text-xs text-slate-600 dark:text-slate-500">
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
              <p className="mt-1 truncate text-sm text-slate-700 dark:text-slate-300">
                {s.title ?? "（无标题）"}
              </p>
              {s.lastMessage && (
                <p className="mt-0.5 truncate text-xs text-slate-600 dark:text-slate-500">
                  {s.lastMessage}
                </p>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
