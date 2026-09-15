import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Board from "@/mobile/Board";
import type { Session, SessionsResponse } from "@/types/session";

const POLL_MS = 3000;

function okSessions(totalCount = 0): Response {
  const body: SessionsResponse = { sessions: [], totalCount, waitingCount: 0 };
  return new Response(JSON.stringify(body), { status: 200 });
}

// 带会话卡的成功响应（M3 Task 3 chips 测试用）
function okSessionsWith(sessions: Session[]): Response {
  const body: SessionsResponse = {
    sessions,
    totalCount: sessions.length,
    waitingCount: sessions.filter((s) => s.status === "waiting").length,
  };
  return new Response(JSON.stringify(body), { status: 200 });
}

// 最小会话夹具：仅 chips 链路消费的字段有语义，其余给合法默认值
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

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

// fake timers 下推进定时器并让 tick 的 promise 微任务落地（act 包裹消状态更新警告）
async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

// Board 对外契约（App 状态机依赖这两个回调的方向与次数）：
describe("Board 轮询契约", () => {
  it("首拍成功：onPaired 恰好回调一次，后续成功拍不重复回调（幂等）", async () => {
    const fetchMock = vi.fn(async () => okSessions(1));
    vi.stubGlobal("fetch", fetchMock);
    const onPaired = vi.fn();
    const onUnpaired = vi.fn();

    render(<Board onPaired={onPaired} onUnpaired={onUnpaired} />);
    await advance(0); // 首拍（= 探测）完成
    expect(onPaired).toHaveBeenCalledTimes(1);
    expect(onUnpaired).not.toHaveBeenCalled();

    await advance(POLL_MS * 2 + 100); // 再走两拍
    // 挂载时另有一次 /host 拉取（M3 Task 1 品牌行），轮询本身仍只走了两拍
    expect(fetchMock.mock.calls.filter(([u]) => u !== "/m/api/v1/host").length).toBe(3);
    expect(onPaired).toHaveBeenCalledTimes(1); // 但成功通知只发一次
  });

  it("403（设备失效）：回调 onUnpaired，且不误发 onPaired", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response("", { status: 403 }))
    );
    const onPaired = vi.fn();
    const onUnpaired = vi.fn();
    render(<Board onPaired={onPaired} onUnpaired={onUnpaired} />);
    await advance(0);
    expect(onUnpaired).toHaveBeenCalledTimes(1);
    expect(onPaired).not.toHaveBeenCalled();
  });

  it("防拍重叠：上一拍未返回时跳过新拍；返回后恢复轮询", async () => {
    let resolveFirst!: (r: Response) => void;
    const pendingFirst = new Promise<Response>((resolve) => {
      resolveFirst = resolve;
    });
    const fetchMock = vi
      .fn<Response[]>()
      .mockReturnValueOnce(pendingFirst)
      .mockImplementation(async () => okSessions(0));
    vi.stubGlobal("fetch", fetchMock);
    const onPaired = vi.fn();
    render(<Board onPaired={onPaired} onUnpaired={vi.fn()} />);

    await advance(0); // 首拍发出但挂起（慢网）
    // 首拍 + 挂载时的 /host 品牌行拉取（M3 Task 1），会话轮询本身只发了 1 次
    expect(fetchMock.mock.calls.filter(([u]) => u !== "/m/api/v1/host").length).toBe(1);
    await advance(POLL_MS * 2 + 100); // 挂起期间到点的两拍全部跳过
    expect(fetchMock.mock.calls.filter(([u]) => u !== "/m/api/v1/host").length).toBe(1);

    resolveFirst(okSessions(0));
    await advance(0); // 首拍落地 → 成功通知（且仅一次）
    expect(onPaired).toHaveBeenCalledTimes(1);

    await advance(POLL_MS + 100); // 恢复后正常走下一拍
    expect(fetchMock.mock.calls.filter(([u]) => u !== "/m/api/v1/host").length).toBe(2);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });
});

// M3 Task 1：页头品牌行（P8a 版本号 + P8b 本机名）
describe("Board 页头品牌行", () => {
  it("挂载时拉一次 /host，品牌行显示 MAM + v{version} + 本机名；host 403 不踢回配对页", async () => {
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
      return okSessions(3);
    });
    vi.stubGlobal("fetch", fetchMock);
    const onUnpaired = vi.fn();
    const { container } = render(<Board onPaired={vi.fn()} onUnpaired={onUnpaired} />);

    await advance(0);
    // host 只在挂载时拉一次（不随 3s 轮询重复）
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

  it("host 拉取失败（网络异常）：静默降级为隐藏品牌行，不影响会话轮询", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url === "/m/api/v1/host") throw new TypeError("network down");
      return okSessions(0);
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    expect(screen.queryByText("MAM")).not.toBeInTheDocument();
    // 会话数据照常拉到
    expect(screen.getByText("0 个会话")).toBeInTheDocument();
  });

  it("host 403（设备失效）：不回调 onUnpaired——设备有效性只以会话轮询为准", async () => {
    const fetchMock = vi.fn(async (url: string) =>
      url === "/m/api/v1/host" ? new Response("", { status: 403 }) : okSessions(0)
    );
    vi.stubGlobal("fetch", fetchMock);
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
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return pendingHost;
        return okSessionsWith([
          chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
        ]);
      })
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
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost(["claude", "codex"]);
        return okSessionsWith([
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
        ]);
      })
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
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost(["claude", "zcode"]);
        return okSessionsWith([
          chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
          chipSession({ id: "z", agentType: "zcode", lastActivityAt: "2026-09-15T12:00:00Z" }),
        ]);
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // DOM 顺序即展示顺序：zcode（12:00）在 claude（10:00）之前，「全部」最前
    expect(chipLabels()).toEqual(["全部", "ZCode", "Claude"]);
  });

  it("选中 chip 用工具品牌色底色（内联 style）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost(["claude"]);
        return okSessionsWith([
          chipSession({ id: "c", agentType: "claude", lastActivityAt: "2026-09-15T10:00:00Z" }),
        ]);
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    // 默认选中「全部」：Claude chip 未选中，无品牌橙实底
    const claudeChip = () =>
      within(screen.getByTestId("tool-chips")).getByText("Claude").closest("button") as HTMLElement;
    expect(getComputedStyle(claudeChip()).backgroundColor).not.toBe("rgb(217, 119, 87)");
    // 点 Claude 后底色为品牌橙 #D97757（jsdom 可能归一化为 rgb 形式，两种都接受）
    fireEvent.click(claudeChip());
    const bg = getComputedStyle(claudeChip()).backgroundColor;
    expect(["#D97757", "rgb(217, 119, 87)", "rgb(217,119,87)"]).toContain(bg);
  });

  it("多行折叠：无溢出时不出现展开/收起按钮（jsdom 高度恒 0 需 mock）", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost(["claude", "codex", "zcode"]);
        return okSessions(0);
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    const chipsRow = screen.getByTestId("tool-chips");
    // jsdom 无布局引擎：度量恒 0。mock 出「未溢出」度量再触发 resize 重测
    vi.spyOn(chipsRow, "scrollHeight", "get").mockReturnValue(100);
    vi.spyOn(chipsRow, "clientHeight", "get").mockReturnValue(100);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(screen.queryByText("展开")).not.toBeInTheDocument();
    expect(screen.queryByText("收起")).not.toBeInTheDocument();
  });

  it("多行折叠：溢出时折叠为一行 + 展开按钮；点击切换收起", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url === "/m/api/v1/host") return okHost(["claude", "codex", "zcode"]);
        return okSessions(0);
      })
    );
    render(<Board onPaired={vi.fn()} onUnpaired={vi.fn()} />);
    await advance(0);
    const chipsRow = screen.getByTestId("tool-chips");
    // mock 出「溢出」度量：内容两行高（200） > 可视一行高（36）
    vi.spyOn(chipsRow, "scrollHeight", "get").mockReturnValue(200);
    vi.spyOn(chipsRow, "clientHeight", "get").mockReturnValue(36);
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    // 折叠态：chips 行不换行（flex-nowrap）+ 展开按钮出现
    expect(chipsRow.className).toContain("flex-nowrap");
    expect(screen.getByText("展开")).toBeInTheDocument();

    fireEvent.click(screen.getByText("展开"));
    // 展开态：恢复换行 + 收起按钮
    expect(chipsRow.className).not.toContain("flex-nowrap");
    expect(screen.getByText("收起")).toBeInTheDocument();

    fireEvent.click(screen.getByText("收起"));
    expect(chipsRow.className).toContain("flex-nowrap");
    expect(screen.getByText("展开")).toBeInTheDocument();
  });
});
