import { act, cleanup, render } from "@testing-library/react";
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
    expect(fetchMock).toHaveBeenCalledTimes(3); // 轮询在继续
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
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await advance(POLL_MS * 2 + 100); // 挂起期间到点的两拍全部跳过
    expect(fetchMock).toHaveBeenCalledTimes(1);

    resolveFirst(okSessions(0));
    await advance(0); // 首拍落地 → 成功通知（且仅一次）
    expect(onPaired).toHaveBeenCalledTimes(1);

    await advance(POLL_MS + 100); // 恢复后正常走下一拍
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });
});
