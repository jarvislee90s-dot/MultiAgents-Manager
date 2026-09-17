// connectEvents（M3 Task 6 SSE 客户端）行为锁：首帧快照 / 增量跃迁 / 退避重连 /
// 2 次失败降级 / 停止清理。EventSource 用测试替身手动驱动（jsdom 无该 API）。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  DEGRADE_AFTER_FAILURES,
  EVENTS_PATH,
  RECONNECT_BASE_MS,
  connectEvents,
} from "@/mobile/api";
import { MockEventSource } from "./eventSourceMock";
import type { SessionsResponse, TransitionEvent } from "@/types/session";

function snapshot(totalCount = 3): SessionsResponse {
  return { sessions: [], totalCount, waitingCount: 0 };
}

function transition(overrides: Partial<TransitionEvent> = {}): TransitionEvent {
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

/** 建立连接并返回三元回调 + 停止函数（每个用例共同的起手式） */
function setup() {
  const onSnapshot = vi.fn();
  const onTransition = vi.fn();
  const onDegraded = vi.fn();
  const stop = connectEvents(onSnapshot, onTransition, onDegraded);
  return { onSnapshot, onTransition, onDegraded, stop };
}

beforeEach(() => {
  MockEventSource.reset();
  vi.useFakeTimers();
  vi.stubGlobal("EventSource", MockEventSource);
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("connectEvents：连接与帧分发", () => {
  it("挂载即连 /m/api/v1/events；snapshot 帧转 onSnapshot、transition 帧转 onTransition（各含 payload）", () => {
    const { onSnapshot, onTransition } = setup();
    const es = MockEventSource.latest();
    expect(es.url).toBe(EVENTS_PATH);

    es.emit("snapshot", snapshot(7));
    expect(onSnapshot).toHaveBeenCalledTimes(1);
    expect(onSnapshot.mock.calls[0][0]).toEqual(snapshot(7));

    const ev = transition();
    es.emit("transition", ev);
    expect(onTransition).toHaveBeenCalledTimes(1);
    expect(onTransition.mock.calls[0][0]).toEqual(ev);
  });

  it("坏帧（非法 JSON）被跳过：不抛错、不断流、不影响后续帧", () => {
    const { onSnapshot, onTransition } = setup();
    const es = MockEventSource.latest();
    expect(() => es.emit("snapshot", "{截断")).not.toThrow();
    expect(onSnapshot).not.toHaveBeenCalled();
    expect(es.closed).toBe(false); // 单帧损坏不得断流

    es.emit("transition", transition({ sessionId: "s2" }));
    expect(onTransition).toHaveBeenCalledTimes(1);
  });

  it("停止函数：关闭连接后再投帧不再回调（卸载后无幽灵更新）", () => {
    const { onSnapshot, stop } = setup();
    const es = MockEventSource.latest();
    stop();
    expect(es.closed).toBe(true);
    es.emit("snapshot", snapshot());
    expect(onSnapshot).not.toHaveBeenCalled();
  });

  it("停止函数关掉的是**当前**连接：退避期间（旧连接已 close）调用 stop 不得留下待重连定时器", async () => {
    const { onSnapshot, stop } = setup();
    MockEventSource.latest().fail(); // 连接已 close、退避定时器已装
    stop(); // 退避中途卸载（组件卸载 / 403 回配对页）
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS * 5);
    expect(MockEventSource.instances).toHaveLength(1); // 不得再建连
    expect(onSnapshot).not.toHaveBeenCalled();
  });
});

describe("connectEvents：断线退避与降级", () => {
  it("首次失败：关闭当前连接（禁用浏览器内建重连）+ 1s 后新建连接，未达阈值不降级", async () => {
    const { onDegraded, onSnapshot } = setup();
    const first = MockEventSource.latest();
    first.fail();
    expect(first.closed).toBe(true);
    expect(MockEventSource.instances).toHaveLength(1); // 退避期内不立即重连
    expect(onDegraded).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS);
    expect(MockEventSource.instances).toHaveLength(2); // 退避到期后手动重连
    // 重连后的快照照常送达（新连接可用）
    MockEventSource.latest().emit("snapshot", snapshot(5));
    expect(onSnapshot).toHaveBeenCalledTimes(1);
  });

  it(`${DEGRADE_AFTER_FAILURES} 次失败 → onDegraded 回调一次且不再新建连接（转轮询）`, async () => {
    const { onDegraded } = setup();
    MockEventSource.latest().fail(); // 第 1 次
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS);
    expect(MockEventSource.instances).toHaveLength(2);
    MockEventSource.latest().fail(); // 第 2 次 → 达阈值
    expect(onDegraded).toHaveBeenCalledTimes(1);

    // 降级是终态：再推进大段时间也不得新建连接（轮询由调用方接管）
    const created = MockEventSource.instances.length;
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS * 10);
    expect(MockEventSource.instances).toHaveLength(created);
    expect(onDegraded).toHaveBeenCalledTimes(1);
  });

  it("成功帧清零失败计数：断续不累积到降级（网络抖动自愈）", async () => {
    const { onDegraded, onSnapshot } = setup();
    MockEventSource.latest().fail(); // 第 1 次失败
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS);
    const second = MockEventSource.latest();
    second.emit("snapshot", snapshot(1)); // 重连成功、收到帧 → 计数清零
    expect(onSnapshot).toHaveBeenCalledTimes(1);

    second.fail(); // 计数清零后这仍是「第 1 次失败」
    expect(onDegraded).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS);
    expect(MockEventSource.instances).toHaveLength(3); // 仍在退避重连，未降级
  });

  it("降级后停止函数仍安全（幂等清理，不再有定时器残留）", async () => {
    const { onDegraded, stop } = setup();
    MockEventSource.latest().fail();
    await vi.advanceTimersByTimeAsync(RECONNECT_BASE_MS);
    MockEventSource.latest().fail();
    expect(onDegraded).toHaveBeenCalledTimes(1);
    expect(() => stop()).not.toThrow();
  });

  it("环境无 EventSource：直接降级轮询（不做注定失败的重试）", () => {
    vi.unstubAllGlobals();
    vi.stubGlobal("EventSource", undefined);
    const { onDegraded } = setup();
    expect(onDegraded).toHaveBeenCalledTimes(1);
    expect(MockEventSource.instances).toHaveLength(0);
  });
});
