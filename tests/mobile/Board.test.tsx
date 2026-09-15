import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Board from "@/mobile/Board";
import type { SessionsResponse } from "@/types/session";

const POLL_MS = 3000;

function okSessions(totalCount = 0): Response {
  const body: SessionsResponse = { sessions: [], totalCount, waitingCount: 0 };
  return new Response(JSON.stringify(body), { status: 200 });
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
      url === "/m/api/v1/host"
        ? new Response("", { status: 403 })
        : okSessions(0)
    );
    vi.stubGlobal("fetch", fetchMock);
    const onUnpaired = vi.fn();
    render(<Board onPaired={vi.fn()} onUnpaired={onUnpaired} />);
    await advance(0);
    expect(onUnpaired).not.toHaveBeenCalled();
  });
});
