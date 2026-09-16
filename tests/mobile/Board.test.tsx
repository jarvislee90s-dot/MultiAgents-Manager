import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Board from "@/mobile/Board";
import { applyInitialTheme } from "@/mobile/theme";
import { MockEventSource } from "./eventSourceMock";
import type { Session, SessionsResponse, TransitionEvent } from "@/types/session";

// M3 Task 6 后，Board 的主数据通道是 SSE（首帧 snapshot + transition 增量），
// 3s 轮询只在连续 2 次失败降级后启用。因此本文件的夹具统一走「脚本化 EventSource」：
// 连接建立后自动送达首帧快照（setTimeout 0，fake timers 下由 advance(0) 驱动），
// 后续快照 / 跃迁由用例显式 emit 推送——与真实服务端行为同形。
const POLL_MS = 3000;
const BANNER_TTL_MS = 4000;
/** SSE 模式低频对账周期（评审修复 R1）：新会话成员资格兜底上卡的周期上限 */
const RECONCILE_MS = 30_000;

function okSessions(totalCount = 0): SessionsResponse {
  return { sessions: [], totalCount, waitingCount: 0 };
}

// 带会话卡的成功载荷（M3 Task 3 chips 测试用）
function sessionsWith(sessions: Session[]): SessionsResponse {
  return {
    sessions,
    totalCount: sessions.length,
    waitingCount: sessions.filter((s) => s.status === "waiting").length,
  };
}

// 最小会话夹具：仅 chips / 卡片链路消费的字段有语义，其余给合法默认值
function chipSession(
  overrides: Partial<Session> & Pick<Session, "id" | "agentType" | "lastActivityAt">
): Session {
  return {
    projectName: "proj",
    projectPath: "/tmp/proj",
    title: null,
    gitBranch: null,
    githubUrl: null,
    status: "idle",
    lastMessage: null,
    lastMessageRole: null,
    pid: 1,
    cpuUsage: 0,
    activeSubagentCount: 0,
    form: "cli",
    jumpSupported: false,
    unread: false,
    ...overrides,
  };
}

function okHost(enabledTools: string[]): Response {
  return new Response(
    JSON.stringify({
      host: { name: "JARVIS-Mac", platform: "macos", version: "0.4.1" },
      enabledTools,
    }),
    { status: 200 }
  );
}

/** 安装脚本化 EventSource：首次连接自动送达给定快照（模拟服务端首帧） */
function installSse(snapshot: SessionsResponse) {
  class ScriptedEventSource extends MockEventSource {
    constructor(url: string) {
      super(url);
      setTimeout(() => this.emit("snapshot", snapshot), 0);
    }
  }
  vi.stubGlobal("EventSource", ScriptedEventSource);
}

/** 当前连接（不存在时抛错——用例顺序写错立即失败） */
function sse(): MockEventSource {
  return MockEventSource.latest();
}

/** 投递一帧（act 包裹：事件分发触发 React 状态更新） */
function emitFrame(type: string, data: unknown) {
  act(() => {
    sse().emit(type, data);
  });
}

/** 推一条跃迁事件（与后端 TransitionEvent camelCase 形状一致） */
function transitionEvent(overrides: Partial<TransitionEvent> = {}): TransitionEvent {
  return {
    sessionId: "s1",
    agentType: "claude",
    from: "processing",
    to: "waiting",
    projectName: "proj",
    lastMessage: "needs approval",
    ts: 42,
    ...overrides,
  };
}

/** 让当前 SSE 连接连续失败 N 次（跨退避重连），返回实际失败次数 */
async function failSse(times: number) {
  for (let i = 0; i < times; i += 1) {
    act(() => {
      sse().fail(); // 失败分发触发重连/降级状态更新，须在 act 内
    });
    await advance(1000 * (i + 1)); // 退避 1s×第几次失败：推进到重连完成
  }
}

beforeEach(() => {
  vi.useFakeTimers();
  MockEventSource.reset();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// fake timers 下推进定时器并让微任务落地（act 包裹消状态更新警告）
async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

// Board 对外契约（App 状态机依赖这两个回调的方向与次数）：
describe("Board 数据通道契约", () => {
  it("SSE 首帧快照：onPaired 恰好回调一次；后续快照（含重连）不重复回调（幂等）", async () => {
    installSse(okSessions(1));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 403 }))
    );
    const onPaired = vi.fn();
    const onUnpaired = vi.fn();

    render(<Board onPaired={onPaired} onUnpaired={onUnpaired} />);
    await advance(0); // 首帧快照送达（= 探测）
    expect(onPaired).toHaveBeenCalledTimes(1);
    expect(onUnpaired).not.toHaveBeenCalled();
    expect(screen.getByText("1 个会话")).toBeInTheDocument();

    // 后续快照（服务端再次推送）：数据更新但成功通知不再重复
    emitFrame("snapshot", okSessions(2));
    await advance(0);
    expect(screen.getByText("2 个会话")).toBeInTheDocument();
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("SSE 模式下新会话 30s 对账内上卡（成员资格不再冻结）", async () => {
    // 背景（评审 Critical）：transition 只更新已存在卡（watcher 对新增会话不发事件），
    // SSE 模式又无周期快照——新会话的「上卡/下卡」成员资格在 SSE 主通道里是冻结的。
    // 兜底 = SSE effect 内 30s 低频对账 tick（全量拉取）。本用例锁该机制：
    // 无任何 transition / snapshot 推送，仅桌面侧新增会话，advance 30s 后必须上卡
    installSse(okSessions(0));
    let remoteCount = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost([]);
        return new Response(JSON.stringify(okSessions(remoteCount)), { status: 200 });
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(screen.getByText("0 个会话")).toBeInTheDocument();

    remoteCount = 1; // 桌面侧出现新会话（SSE 不推送任何帧）
    await advance(RECONCILE_MS); // 30s 对账 tick → fetchSessions 全量拉取
    expect(screen.getByText("1 个会话")).toBeInTheDocument();
  });

  it("SSE 断线不触发 onUnpaired（断线≠403，不误踢回配对页）", async () => {
    installSse(okSessions(1));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(okSessions(1)), { status: 200 }))
    );
    const onPaired = vi.fn();
    const onUnpaired = vi.fn();
    render(<Board onPaired={onPaired} onUnpaired={onUnpaired} />);
    await advance(0);

    await failSse(2); // 连断两次 → 降级轮询
    expect(onUnpaired).not.toHaveBeenCalled();
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("连续 2 次失败降级：3s 轮询接管数据（fetch 收到会话），403 才回配对页", async () => {
    installSse(okSessions(1));
    const fetchMock = vi.fn(async () => new Response("", { status: 403 }));
    vi.stubGlobal("fetch", fetchMock);
    const onUnpaired = vi.fn();
    render(<Board onPaired={vi.fn()} onUnpaired={onUnpaired} />);
    await advance(0);

    await failSse(2);
    // 降级即刻补一拍：403 → 设备失效 → 回配对页（唯一判废通道）
    expect(onUnpaired).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith("/m/api/v1/sessions");

    // 降级后无 SSE 重建：轮询接管期间不再新建连接
    const created = MockEventSource.instances.length;
    await advance(POLL_MS * 3);
    expect(MockEventSource.instances).toHaveLength(created);
  });

  it("降级轮询防拍重叠：上一拍未返回时跳过新拍；返回后恢复轮询", async () => {
    installSse(okSessions(0));
    let resolveFirst!: (r: Response) => void;
    const pendingFirst = new Promise<Response>((resolve) => {
      resolveFirst = resolve;
    });
    // 按 URL 分流：首个 /sessions 请求挂起（慢网），其余（含 /host 品牌行）即时应答——
    // 若用 mockReturnValueOnce，挂起会被挂载时的 /host 请求抢走（调用序不再等于数据序）
    let sessionCalls = 0;
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "/m/api/v1/host") return okHost(["claude"]);
      sessionCalls += 1;
      if (sessionCalls === 1) return pendingFirst;
      return new Response(JSON.stringify(okSessions(0)), { status: 200 });
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);

    await failSse(2); // 降级 → 立即起一拍（挂起）
    expect(sessionCalls).toBe(1);
    await advance(POLL_MS * 2 + 100); // 挂起期间到点的两拍全部跳过
    expect(sessionCalls).toBe(1);

    resolveFirst(new Response(JSON.stringify(okSessions(3)), { status: 200 }));
    await advance(0); // 首拍落地
    expect(screen.getByText("3 个会话")).toBeInTheDocument();

    await advance(POLL_MS + 100); // 恢复后正常走下一拍
    expect(sessionCalls).toBe(2);
  });
});

// M3 Task 6：跃迁实时提醒（横幅 + 提示音 + 振动）与看板卡实时刷新
describe("Board 跃迁提醒（SSE transition）", () => {
  it("transition → 横幅出现（工具 · 项目 · 变化方向 · 消息预览），并自动消失", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "s1", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
      ])
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(screen.queryByTestId("transition-banners")).not.toBeInTheDocument();

    emitFrame("transition", transitionEvent({ projectName: "mam", lastMessage: "需要批准" }));
    await advance(0);
    const banners = screen.getByTestId("transition-banners");
    expect(banners.textContent).toContain("Claude");
    expect(banners.textContent).toContain("mam");
    expect(banners.textContent).toContain("运行中 → 等待操作");
    expect(banners.textContent).toContain("需要批准");

    // 几秒后自动消失（不长期占据看板顶部）
    await advance(BANNER_TTL_MS + 100);
    expect(screen.queryByTestId("transition-banners")).not.toBeInTheDocument();
  });

  it("transition → 对应会话卡状态实时刷新（数据去重在服务端，前端来一条应用一条）", async () => {
    installSse(
      sessionsWith([
        chipSession({
          id: "s1",
          agentType: "claude",
          status: "processing",
          projectName: "mam",
          lastActivityAt: "2026-09-15T10:00:00Z",
        }),
      ])
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 初始：processing → 状态点为黄
    const dot = () => {
      const card = screen.getByText("mam").closest("li") as HTMLElement;
      return card.querySelector("span.rounded-full:last-of-type") as HTMLElement;
    };
    expect(dot().className).toContain("bg-yellow-500");

    emitFrame(
      "transition",
      transitionEvent({ sessionId: "s1", from: "processing", to: "waiting", projectName: "mam" })
    );
    await advance(0);
    // 跃迁后：waiting → 状态点转红且挂呼吸动画（卡片随 SSE 实时变化，无需等下一拍）
    expect(dot().className).toContain("bg-red-500");
    expect(dot().className).toContain("animate-pulse");
  });

  it("同 sessionId 的连续跃迁：横幅覆盖为最新一条（展示层防叠），不同会话可并存", async () => {
    installSse(okSessions(0));
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);

    emitFrame("transition", transitionEvent({ sessionId: "a", projectName: "first" }));
    await advance(0);
    emitFrame("transition", transitionEvent({ sessionId: "a", projectName: "second" }));
    await advance(0);
    let items = within(screen.getByTestId("transition-banners")).getAllByRole("listitem");
    expect(items).toHaveLength(1); // 同会话覆盖，不叠加
    expect(items[0].textContent).toContain("second");

    emitFrame("transition", transitionEvent({ sessionId: "b", projectName: "other" }));
    await advance(0);
    items = within(screen.getByTestId("transition-banners")).getAllByRole("listitem");
    expect(items).toHaveLength(2); // 不同会话各自一条
    expect(items.map((i) => i.textContent).join("|")).toContain("other");
  });

  it("振动与提示音为能力检测路径：有 navigator.vibrate 时触发一次 200ms", async () => {
    installSse(okSessions(0));
    const vibrate = vi.fn();
    Object.defineProperty(navigator, "vibrate", { value: vibrate, configurable: true });
    try {
      render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
      await advance(0);
      emitFrame("transition", transitionEvent());
      await advance(0);
      expect(vibrate).toHaveBeenCalledTimes(1);
      expect(vibrate).toHaveBeenCalledWith(200);
    } finally {
      // 还原：jsdom 的 navigator 无 vibrate，删除本次注入
      delete (navigator as unknown as Record<string, unknown>).vibrate;
    }
  });

  it("无 navigator.vibrate（桌面浏览器 / iOS Safari）与无 AudioContext 时静默跳过，提醒链路不中断", async () => {
    installSse(okSessions(0));
    // jsdom 默认即无 vibrate / AudioContext——此用例锁「缺失不抛错且横幅照常」
    expect((navigator as unknown as { vibrate?: unknown }).vibrate).toBeUndefined();
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(() => emitFrame("transition", transitionEvent({ projectName: "no-cap" }))).not.toThrow();
    await advance(0);
    expect(screen.getByTestId("transition-banners").textContent).toContain("no-cap");
  });
});

// M3 Task 1：页头品牌行（P8a 版本号 + P8b 本机名）
describe("Board 页头品牌行", () => {
  it("挂载时拉一次 /host，品牌行显示 MAM + v{version} + 本机名；host 403 不踢回配对页", async () => {
    installSse(okSessions(3));
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "/m/api/v1/host") {
        return new Response(
          JSON.stringify({
            host: { name: "JARVIS-Win", platform: "windows", version: "0.4.1" },
            enabledTools: ["claude"],
          }),
          { status: 200 }
        );
      }
      return new Response("", { status: 403 });
    });
    vi.stubGlobal("fetch", fetchMock);
    const onUnpaired = vi.fn();
    const { container } = render(<Board onPaired={vi.fn()} onUnpaired={onUnpaired} />);

    await advance(0);
    // host 只在挂载时拉一次（不随数据通道重复）
    expect(fetchMock).toHaveBeenCalledWith("/m/api/v1/host");
    const hostCalls = fetchMock.mock.calls.filter(([u]) => u === "/m/api/v1/host").length;
    await advance(POLL_MS + 100);
    expect(fetchMock.mock.calls.filter(([u]) => u === "/m/api/v1/host").length).toBe(hostCalls);

    expect(screen.getByText("MAM")).toBeInTheDocument();
    expect(screen.getByText("v0.4.1")).toBeInTheDocument();
    expect(screen.getByText("JARVIS-Win")).toBeInTheDocument();
    // 看板标题行保留（品牌行在其上一行）
    expect(container.textContent).toContain("会话看板");
    expect(onUnpaired).not.toHaveBeenCalled();
  });

  it("host 拉取失败（网络异常）：静默降级为隐藏品牌行，不影响会话数据通道", async () => {
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") throw new TypeError("network down");
        return new Response(JSON.stringify(okSessions(0)), { status: 200 });
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(screen.queryByText("MAM")).not.toBeInTheDocument();
    // 会话数据照常到达（SSE 快照）
    expect(screen.getByText("0 个会话")).toBeInTheDocument();
  });

  it("host 403（设备失效）：不回调 onUnpaired——设备有效性只以会话数据通道的 403 为准", async () => {
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/m/api/v1/host"
          ? new Response("", { status: 403 })
          : new Response(JSON.stringify(okSessions(0)), { status: 200 })
      )
    );
    const onUnpaired = vi.fn();
    render(<Board onPaired={vi.fn()} onUnpaired={onUnpaired} />);
    await advance(0);
    expect(onUnpaired).not.toHaveBeenCalled();
  });
});

// M3 Task 3：P8d 受管过滤 + P8e 品牌色多行折叠 chips + 活跃排序
//
// 注意口径（控制者裁决）：chips 集合 = ["全部"] ∪ filterEnabledTools(有卡工具, 受管名单)，
// 即 chip 是「受管 ∩ 有卡」——受管但当前无卡的工具不上板（有卡只影响排序与准入）。
// 卡片主行也渲染工具名（如 "Claude"），与 chip 文本同名，故 chip 断言一律在
// data-testid="tool-chips" 行内做（within），避免全局 getByText 多重匹配歧义。
describe("Board 工具 chips（P8d/P8e）", () => {
  // chips 行内的按钮文本序列（DOM 顺序即展示顺序）
  function chipLabels(): Array<string | null> {
    return Array.from(screen.getByTestId("tool-chips").querySelectorAll("button")).map(
      (b) => b.textContent
    );
  }

  it("chips 随 host 与 sessions 异步到达收敛：host 未到只有「全部」，到了并入受管∩有卡", async () => {
    // host 响应挂起：模拟 host 与 sessions 异步到达的竞态窗口
    let resolveHost!: (r: Response) => void;
    const pendingHost = new Promise<Response>((resolve) => {
      resolveHost = resolve;
    });
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? pendingHost : okSessions(0)))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // sessions 已到但 host 挂起：不猜全量八工具，chips 只有「全部」
    expect(chipLabels()).toEqual(["全部"]);

    // host 到达：受管∩有卡收敛出 Claude chip；受管但无卡（zcode）不上板
    resolveHost(okHost(["claude", "zcode"]));
    await advance(0);
    expect(chipLabels()).toEqual(["全部", "Claude"]);
  });

  it("chips = 受管∩有卡；点击 chip 切换过滤（停车场：顺带锁卡片主行渲染）", async () => {
    installSse(
      sessionsWith([
        chipSession({
          id: "a",
          agentType: "claude",
          projectName: "proj-a",
          lastActivityAt: "2026-09-15T10:00:00Z",
        }),
        chipSession({
          id: "b",
          agentType: "claude",
          projectName: "proj-b",
          lastActivityAt: "2026-09-15T09:00:00Z",
        }),
        chipSession({
          id: "x",
          agentType: "codex",
          projectName: "proj-x",
          lastActivityAt: "2026-09-15T08:00:00Z",
        }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? okHost(["claude", "codex"]) : okSessions(0)))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 有卡受管工具都上板；按最新活跃排序（claude 10:00 > codex 08:00）
    expect(chipLabels()).toEqual(["全部", "Claude", "Codex"]);

    // 点击 Codex chip：只显示 codex 卡
    const codexChip = within(screen.getByTestId("tool-chips")).getByText("Codex").closest("button");
    fireEvent.click(codexChip as HTMLElement);
    expect(screen.getByText("proj-x")).toBeInTheDocument();
    expect(screen.queryByText("proj-a")).not.toBeInTheDocument();

    // 点回「全部」：卡片恢复；停车场 Minor（Task 2 遗留）：顺带锁卡片主行（工具名+项目名）渲染
    fireEvent.click(within(screen.getByTestId("tool-chips")).getByText("全部"));
    const list = within(screen.getByRole("list"));
    expect(list.getAllByText("Claude")).toHaveLength(2);
    expect(list.getByText("proj-x")).toBeInTheDocument();
  });

  it("chips 按工具最新活跃时间降序，「全部」恒在首位", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
        chipSession({ id: "z", agentType: "zcode", lastActivityAt: "2026-09-15T12:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? okHost(["claude", "zcode"]) : okSessions(0)))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // DOM 顺序即展示顺序：zcode（12:00）在 claude（10:00）之前，「全部」最前
    expect(chipLabels()).toEqual(["全部", "ZCode", "Claude"]);
  });

  it("选中 chip 用工具品牌色底色（内联 style）", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? okHost(["claude"]) : okSessions(0)))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 默认选中「全部」：Claude chip 未选中，无品牌紫实底
    const claudeChip = () =>
      within(screen.getByTestId("tool-chips")).getByText("Claude").closest("button") as HTMLElement;
    expect(getComputedStyle(claudeChip()).backgroundColor).not.toBe("rgb(100, 69, 162)");
    // 点 Claude 后底色为品牌紫 #6445A2（jsdom 可能归一化为 rgb 形式，两种都接受）
    fireEvent.click(claudeChip());
    const bg = getComputedStyle(claudeChip()).backgroundColor;
    expect(["#6445A2", "rgb(100, 69, 162)", "rgb(100,69,162)"]).toContain(bg);
  });

  it("未选中 chip 文字色随主题：浅色态用压暗色（AA 达标），暗色态查暗色表（Bug 4）", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? okHost(["claude"]) : okSessions(0)))
    );
    // 浅色态起手（系统浅色偏好）；jsdom 无 matchMedia，本用例内安装并在末尾还原，
    // 防 shim 泄漏到后续用例改变其主题初值
    const prevMatchMedia = window.matchMedia;
    window.matchMedia = ((query: string) => ({
      matches: true, // 系统浅色偏好
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })) as unknown as typeof window.matchMedia;
    try {
      render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
      await advance(0);
      const claudeChip = () =>
        within(screen.getByTestId("tool-chips"))
          .getByText("Claude")
          .closest("button") as HTMLElement;
      expect(getComputedStyle(claudeChip()).color).toMatch(/^(#3c2961|rgb\(60, 41, 97\))$/);
      // 切到暗色：文字色查暗色表 TOOL_BRAND_COLORS_DARK（Bug 4 修复——旧实现回
      // 品牌原色 #6445A2，在深色卡底 #0f172a 上对比度 2.48 融底）
      fireEvent.click(screen.getByRole("button", { name: "切换到深色模式" }));
      expect(getComputedStyle(claudeChip()).color).toMatch(/^(#8B74B9|rgb\(139, 116, 185\))$/);
    } finally {
      window.matchMedia = prevMatchMedia;
      localStorage.clear();
      document.documentElement.classList.remove("dark");
    }
  });

  it("多行折叠：无溢出时不出现展开/收起按钮，容器无折叠裁剪样式", async () => {
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/m/api/v1/host" ? okHost(["claude", "codex", "zcode"]) : okSessions(0)
      )
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    const chipsRow = screen.getByTestId("tool-chips");
    // jsdom 无布局引擎（度量恒 0），只 mock「浏览器度量层」：单行内容高 28 ≤ 折叠上限 36。
    // 阈值比较 / 状态切换 / 按钮渲染全走真实逻辑；真实浏览器下的度量语义
    // 见 task-3-report.md fix 追记（实测：折叠态 scrollHeight 96 / clientHeight 36，
    // 宽屏单行 28 / 28——mock 值取真实量级）
    vi.spyOn(chipsRow, "scrollHeight", "get").mockReturnValue(28);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(screen.queryByText("展开")).not.toBeInTheDocument();
    expect(screen.queryByText("收起")).not.toBeInTheDocument();
    expect(chipsRow.style.maxHeight).toBe(""); // 未折叠：无固定高度裁剪
  });

  it("多行折叠：溢出时容器固定高度裁剪（Critical 回归锁）且展开按钮在裁剪行外（Important 回归锁）", async () => {
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/m/api/v1/host" ? okHost(["claude", "codex", "zcode"]) : okSessions(0)
      )
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    const chipsRow = screen.getByTestId("tool-chips");
    // mock 度量层：三行内容高 96 > 折叠上限 36 ⇒ 溢出（真实浏览器实测 375 宽下为 96/36）。
    // Critical 修复点：折叠态容器必须有固定 max-height——旧实现 auto-height 下
    // scrollHeight === clientHeight 恒等，检测恒 false（评审实测 101/101）
    vi.spyOn(chipsRow, "scrollHeight", "get").mockReturnValue(96);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(chipsRow.style.maxHeight).toBe("36px");
    expect(chipsRow.style.overflow).toBe("hidden");
    // 保留 flex-wrap：折叠裁掉的是「第二行起」（纵向），不是行尾横向裁切——
    // 旧实现 nowrap+overflow-hidden 曾把行尾按钮整体裁到屏外
    expect(chipsRow.className).toContain("flex-wrap");
    expect(chipsRow.className).not.toContain("flex-nowrap");

    // Important 修复点：展开按钮是裁剪行的兄弟节点（结构上不可能被行内裁剪）且可见可点
    const toggle = screen.getByText("展开").closest("button") as HTMLElement;
    expect(chipsRow.contains(toggle)).toBe(false);
    expect(toggle.parentElement?.contains(chipsRow)).toBe(true);
    expect(toggle).toBeEnabled();

    fireEvent.click(toggle);
    // 展开态：解除裁剪、按钮变「收起」
    expect(chipsRow.style.maxHeight).toBe("");
    fireEvent.click(screen.getByText("收起"));
    expect(chipsRow.style.maxHeight).toBe("36px");
    expect(screen.getByText("展开")).toBeInTheDocument();
  });

  it("重测依赖 chip 集合签名：数量相同、内容变化时也重测（Minor ④ 回归锁）", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
        chipSession({ id: "x", agentType: "codex", lastActivityAt: "2026-09-15T09:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/m/api/v1/host" ? okHost(["claude", "codex", "dsh"]) : okSessions(0)
      )
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 首帧集合 全部/Claude/Codex（3 个）；spy 安装在首帧渲染之后，计数基线另行确定
    const chipsRow = screen.getByTestId("tool-chips");
    let reads = 0;
    vi.spyOn(chipsRow, "scrollHeight", "get").mockImplementation(() => {
      reads += 1;
      return 28;
    });
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    const baseline = reads; // 手工 resize 触发的确定读数
    // 下一帧快照：集合 全部/Codex/DSH（数量同为 3、内容不同）——旧实现依赖裸数量不重测
    emitFrame(
      "snapshot",
      sessionsWith([
        chipSession({ id: "x", agentType: "codex", lastActivityAt: "2026-09-15T09:00:00Z" }),
        chipSession({ id: "d", agentType: "dsh", lastActivityAt: "2026-09-15T08:00:00Z" }),
      ])
    );
    await advance(0);
    expect(reads).toBeGreaterThan(baseline);
  });

  it("filter 残留回落：过滤工具在集合收敛中消失时回到「全部」（Minor ⑤ 回归锁）", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
        chipSession({
          id: "x",
          agentType: "codex",
          projectName: "proj-x",
          lastActivityAt: "2026-09-15T09:00:00Z",
        }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) =>
        url === "/m/api/v1/host" ? okHost(["claude", "codex"]) : okSessions(0)
      )
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    const chipsRow = screen.getByTestId("tool-chips");
    const allChip = () => within(chipsRow).getByText("全部").closest("button") as HTMLElement;
    fireEvent.click(within(chipsRow).getByText("Claude").closest("button") as HTMLElement);
    expect(within(chipsRow).getByText("Claude").closest("button")).toHaveAttribute(
      "aria-pressed",
      "true"
    );

    // 下一帧快照：claude 卡消失 ⇒ chip 集合只剩余 codex。filter 残留会使看板停在空列表
    // 且无对应 chip 可取消高亮 ⇒ 应回落「全部」
    emitFrame(
      "snapshot",
      sessionsWith([
        chipSession({
          id: "x",
          agentType: "codex",
          projectName: "proj-x",
          lastActivityAt: "2026-09-15T09:00:00Z",
        }),
      ])
    );
    await advance(0);
    expect(screen.queryByText("Claude")).not.toBeInTheDocument();
    expect(allChip()).toHaveAttribute("aria-pressed", "true");
    // 回落生效的证据：codex 卡可见（filter 残留则显示空态提示）
    expect(screen.getByText("proj-x")).toBeInTheDocument();
  });
});

// M3 Task 4：P8f 日/夜双皮肤（顶部切换按钮 + 容器双态底色）
//
// 测试口径：按钮的可访问名描述「点下去会切到什么」（暗色下叫「切换到浅色模式」），
// 主题初值来自 getInitialTheme（localStorage > matchMedia > 默认 dark，详见 theme.test.ts）。
describe("Board 主题切换（P8f）", () => {
  // jsdom 无 matchMedia：安装可控 shim（同 tests/pet/petSettings.test.tsx:11 模式）
  function installMatchMedia(matches: boolean) {
    window.matchMedia = ((query: string) => ({
      matches,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })) as unknown as typeof window.matchMedia;
  }

  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark", "light");
    installMatchMedia(false); // 系统非浅色偏好
    // 终审 Important 3 起镜像真实挂载序：main.tsx 在 React 挂载前调 applyInitialTheme()，
    // 生产环境 DOM 类与 theme state 恒一致；fixture 补齐这步（此前缺失时 DOM 类从未真正
    // 打上，toggleTheme 改为按当前生效态推导后必须真实建 DOM 前置）
    applyInitialTheme();
  });
  afterEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark", "light");
  });

  it("初始暗色：按钮文案为「切换到浅色模式」；点击后翻转浅色——移除 dark 类 + 持久化 mam-theme=light", async () => {
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => okHost(["claude"]))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);

    const toggle = screen.getByRole("button", { name: "切换到浅色模式" });
    fireEvent.click(toggle);
    // 翻转后：documentElement 去掉 dark 类（dark: 前缀类随之全部失效）
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(localStorage.getItem("mam-theme")).toBe("light");
    // 图标/文案同步为「切回深色」
    expect(screen.getByRole("button", { name: "切换到深色模式" })).toBeInTheDocument();

    // 再点一次回暗色：dark 类加回、持久化翻转
    fireEvent.click(screen.getByRole("button", { name: "切换到深色模式" }));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("mam-theme")).toBe("dark");
  });

  it("手动选择优先于系统：已存 mam-theme=light + 系统暗色 → 初始按钮为「切换到深色模式」", async () => {
    localStorage.setItem("mam-theme", "light");
    installSse(okSessions(0));
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => okHost(["claude"]))
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(screen.getByRole("button", { name: "切换到深色模式" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "切换到浅色模式" })).not.toBeInTheDocument();
  });

  it("容器底色双态回归锁：浅色底（bg-white）+ dark: 前缀深色底，body 同款不残留黑底", async () => {
    installSse(
      sessionsWith([
        chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
      ])
    );
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => (url === "/m/api/v1/host" ? okHost(["claude"]) : okSessions(0)))
    );
    const { container } = render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 看板根容器：双态类同时存在，浅色态不再是黑底（P8f 浅色模式整体可读性的最小锁）
    const root = container.querySelector("div.min-h-screen") as HTMLElement;
    expect(root.className).toContain("bg-white");
    expect(root.className).toContain("dark:bg-slate-950");
    // 卡片同理：浅色底 + dark 前缀深色底
    const card = container.querySelector("ul > li") as HTMLElement;
    expect(card.className).toContain("bg-slate-100");
    expect(card.className).toContain("dark:bg-slate-900");
  });
});
