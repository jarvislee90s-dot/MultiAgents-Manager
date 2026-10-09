import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SubagentChips, { chipTokenText, formatElapsed } from "@/mobile/SubagentChips";
import type { Session } from "@/types/session";

// spec §6/§7（前端）：渲染/多枚并排/空列表不渲染/拉取失败不渲染/时长自走（fake timers，
// tick 不出组件）/refreshTick 重拉/finished 不拉。

let fetchMock: ReturnType<typeof vi.fn>;
const baseSession: Session = {
  id: "s1",
  agentType: "claude",
  projectName: "p",
  projectPath: "p",
  title: null,
  gitBranch: null,
  githubUrl: null,
  status: "processing",
  lastMessage: null,
  lastMessageRole: null,
  lastActivityAt: "2026-10-08T07:00:00Z",
  pid: 1,
  cpuUsage: 0,
  activeSubagentCount: 0,
  form: "cli",
  jumpSupported: false,
  unread: false,
};
function subagent(
  id: string,
  name: string,
  spawnTs: string | null,
  t: { input: number; cacheRead: number; cacheCreation: number; output: number }
) {
  return { id, name, description: "设计新建会话两改动实现方案", spawnTs, tokens: t };
}

/** 桩路由表（用例就地改写；fail=true 模拟网络异常） */
let routes: { body?: ReturnType<typeof subagent>[]; status?: number; fail?: boolean };

beforeEach(() => {
  routes = {};
  vi.stubGlobal(
    "fetch",
    (fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (!url.includes("/session-subagents")) throw new Error(`unexpected fetch: ${url}`);
      if (routes.fail) throw new TypeError("network down");
      return new Response(JSON.stringify({ subagents: routes.body ?? [] }), {
        status: routes.status ?? 200,
      });
    }))
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

/** 纯函数：时长格式（M:SS；≥1h H:MM:SS）与 token 合计（fmtTokens 口径） */
describe("formatElapsed / chipTokenText", () => {
  it("时长与 token 缩写", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(59_000)).toBe("0:59");
    expect(formatElapsed(338_000)).toBe("5:38");
    expect(formatElapsed(3_753_000)).toBe("1:02:33");
    expect(
      chipTokenText({ input: 5124, cacheRead: 56448, cacheCreation: 0, output: 9312 })
    ).toBe("7.09万");
    expect(chipTokenText({ input: 0, cacheRead: 0, cacheCreation: 0, output: 0 })).toBe("0");
  });
});

/** 渲染矩阵（fake timers：spawnTs 固定为 10s 前） */
describe("SubagentChips", () => {
  it("渲染名称+时长+token；description 进 title", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-08T07:36:45Z")); // 与 spawnTs 差 338s = 5:38
    routes.body = [
      subagent(
        "a86f",
        "Plan",
        "2026-10-08T07:31:07Z",
        { input: 5124, cacheRead: 56448, cacheCreation: 0, output: 9312 }
      ),
    ];
    render(<SubagentChips session={baseSession} refreshTick={0} />);
    // 首拉落地（RTL findBy 的轮询不被 vitest 假计时器驱动——仓内先例 act+await）
    await act(async () => {});
    const chip = screen.getByTestId("subagent-chip-a86f");
    expect(chip.textContent).toContain("Plan");
    expect(chip.textContent).toContain("5:38");
    expect(chip.textContent).toContain("7.09万");
    expect(chip.getAttribute("title")).toBe("设计新建会话两改动实现方案");
  });

  it("spawnTs=null → 只显名称与 token，无时长", async () => {
    routes.body = [
      subagent("x1", "Explore", null, { input: 100, cacheRead: 0, cacheCreation: 0, output: 0 }),
    ];
    render(<SubagentChips session={baseSession} refreshTick={0} />);
    const chip = await screen.findByTestId("subagent-chip-x1");
    expect(chip.textContent).not.toMatch(/\d:\d\d/);
    expect(chip.textContent).toContain("100");
  });

  it("多枚并排", async () => {
    routes.body = [
      subagent("a", "Plan", "2026-10-08T07:30:00Z", {
        input: 1,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
      }),
      subagent("b", "Explore", "2026-10-08T07:31:00Z", {
        input: 2,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
      }),
    ];
    render(<SubagentChips session={baseSession} refreshTick={0} />);
    expect(await screen.findByTestId("subagent-chip-a")).toBeTruthy();
    expect(screen.getByTestId("subagent-chip-b")).toBeTruthy();
  });

  it("空列表不渲染；首拉失败不渲染（静默）", async () => {
    routes.body = [];
    const { container, rerender } = render(<SubagentChips session={baseSession} refreshTick={0} />);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(container.querySelector('[data-testid="subagent-chips"]')).toBeNull();
    // 首拉即网络异常 → 维持 null，不渲染、不报错
    fetchMock.mockClear();
    routes.fail = true;
    rerender(<SubagentChips session={baseSession} refreshTick={1} />);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));
    expect(container.querySelector('[data-testid="subagent-chips"]')).toBeNull();
  });

  it("时长自走：tick 不出组件（父零重渲染）", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-08T07:36:35Z"));
    routes.body = [
      subagent("a86f", "Plan", "2026-10-08T07:36:35Z", {
        input: 1,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
      }),
    ];
    let parentRenders = 0;
    const Probe = () => {
      parentRenders += 1;
      return <SubagentChips session={baseSession} refreshTick={0} />;
    };
    render(<Probe />);
    await act(async () => {}); // 首拉落地（同上：findBy 不吃 vitest 假计时器）
    expect(screen.getByTestId("subagent-chip-a86f").textContent).toContain("0:00");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(61_000);
    });
    expect(screen.getByTestId("subagent-chip-a86f").textContent).toContain("1:01");
    expect(parentRenders).toBe(1, "1s tick 只重渲染 SubagentChips，不拖整页（spec §6）");
  });

  it("refreshTick bump → 重拉；成功空列表 → 清空", async () => {
    routes.body = [
      subagent("a", "Plan", "2026-10-08T07:36:35Z", {
        input: 1,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
      }),
    ];
    const { rerender } = render(<SubagentChips session={baseSession} refreshTick={0} />);
    await screen.findByTestId("subagent-chip-a");
    routes.body = [];
    rerender(<SubagentChips session={baseSession} refreshTick={1} />);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() =>
      expect(document.querySelector('[data-testid="subagent-chips"]')).toBeNull()
    );
  });

  it("失败保留上一份（不闪断）；finished 不拉取", async () => {
    routes.body = [
      subagent("a", "Plan", "2026-10-08T07:36:35Z", {
        input: 1,
        cacheRead: 0,
        cacheCreation: 0,
        output: 0,
      }),
    ];
    const { rerender } = render(<SubagentChips session={baseSession} refreshTick={0} />);
    await screen.findByTestId("subagent-chip-a");
    routes.fail = true; // 网络失败 → 静默保留上一份
    rerender(<SubagentChips session={baseSession} refreshTick={1} />);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    expect(screen.getByTestId("subagent-chip-a")).toBeTruthy();
    // finished：不挂载不拉取
    fetchMock.mockClear();
    rerender(<SubagentChips session={{ ...baseSession, status: "finished" }} refreshTick={2} />);
    expect(fetchMock).toHaveBeenCalledTimes(0);
  });
});
