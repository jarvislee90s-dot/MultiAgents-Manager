import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MessageComposer from "@/mobile/MessageComposer";
import type { QueueItemView, SendInfo } from "@/mobile/api";

// M7 Task 7：移动端发送 UI（W4）。fetch 全量 stub（盖过 setup.ts 的 msw），
// 按 URL 分路到 send-info / session-send / queue 三族端点（注意 URL 前缀包含
// 关系：/session-send-info ⊃ /session-send、/session-queue/jump|retract ⊃
// /session-queue，长路径必须先判）。组件挂载即拉 send-info，用例先 findBy
// 输入框就绪再交互。

/** 可注入态夹具（后端 SendInfo camelCase 契约） */
function sendInfo(overrides: Partial<SendInfo> = {}): SendInfo {
  return { injectable: true, channels: ["tmux"], visibility: "realtime", ...overrides };
}

/** 排队条目夹具（GET /session-queue 的 items 元素） */
function queueItem(overrides: Partial<QueueItemView> = {}): QueueItemView {
  return { id: 7, content: "[mobile 测试机] 你好", enqueuedAt: 1000, position: 1, ...overrides };
}

interface Routes {
  info?: SendInfo;
  infoStatus?: number;
  send?: Record<string, unknown>;
  sendStatus?: number;
  queue?: QueueItemView[];
  jump?: Record<string, unknown>;
  retractOk?: boolean;
}

let routes: Routes;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  routes = {};
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** 按 URL 分路的 fetch stub（判序：长路径在前，避免前缀误吞） */
function installFetch() {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/session-send-info")) {
      if (routes.infoStatus) return new Response("no", { status: routes.infoStatus });
      return new Response(JSON.stringify(routes.info ?? sendInfo()), { status: 200 });
    }
    if (url.includes("/session-queue/jump")) {
      return new Response(JSON.stringify(routes.jump ?? { status: "delivered" }), { status: 200 });
    }
    if (url.includes("/session-queue/retract")) {
      return new Response(JSON.stringify({ ok: routes.retractOk ?? true }), { status: 200 });
    }
    if (url.includes("/session-send")) {
      if (routes.sendStatus) return new Response("no", { status: routes.sendStatus });
      return new Response(JSON.stringify(routes.send ?? { status: "delivered" }), { status: 200 });
    }
    if (url.includes("/session-queue")) {
      return new Response(JSON.stringify({ items: routes.queue ?? [] }), { status: 200 });
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

/** POST /session-send 的调用（URL 精确到 /session-send 结尾，排除 -info 前缀） */
function sendCalls(): Array<Array<unknown>> {
  return fetchMock.mock.calls.filter((c: unknown[]) => /\/session-send$/.test(String(c[0])));
}

/** GET /session-queue（列表）调用数（列表带 ?session_id= 查询串，排除 jump/retract） */
function queueListCalls(): number {
  return fetchMock.mock.calls.filter((c: unknown[]) => {
    const u = String(c[0]);
    return u.includes("/session-queue?") || u.endsWith("/session-queue");
  }).length;
}

/** 放行 mock fetch 的 promise 链（若干轮微任务冲刷，足以走完 fetch→json→setState） */
async function flushAsync() {
  for (let i = 0; i < 6; i += 1) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

describe("MessageComposer：发送与回执（W4）", () => {
  it("injectable=true：多行文本原样上行（body={sessionId,text}），回执「已送达终端」且输入清空", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = (await screen.findByTestId("composer-input")) as HTMLTextAreaElement;
    fireEvent.change(input, { target: { value: "第一行\n第二行" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    expect(await screen.findByTestId("send-receipt-delivered").then((el) => el.textContent)).toBe(
      "已送达终端"
    );
    // fetch body 契约：多行原样上行（归一在服务端，前端不动文本）
    expect(sendCalls()).toHaveLength(1);
    expect(JSON.parse(String((sendCalls()[0][1] as RequestInit).body))).toEqual({
      sessionId: "sess-1",
      text: "第一行\n第二行",
    });
    // 送达成功 → 输入清空
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("");
  });

  it("injectable=false：输入区禁用 + 展示 reason（send-disabled-reason）", async () => {
    installFetch();
    routes.info = sendInfo({
      injectable: false,
      reasonCode: "blackbox",
      reason: "WorkBuddy 黑盒会话无法注入",
    });
    render(<MessageComposer session={{ id: "sess-1" }} />);
    expect(await screen.findByTestId("send-disabled-reason").then((el) => el.textContent)).toContain(
      "WorkBuddy 黑盒会话无法注入"
    );
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByTestId("composer-send") as HTMLButtonElement).disabled).toBe(true);
  });

  it("排队态：回执「排队中 第1位」+ 立即发送/撤回；撤回后 fetchQueue 刷新为空、回执消失", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.queue = [];
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "跑个长任务" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-queued");
    expect(chip.textContent).toContain("排队中 第1位");
    expect(screen.getByTestId("queue-jump").textContent).toContain("立即发送");
    expect(screen.getByTestId("queue-retract").textContent).toContain("撤回");
    // 撤回：retract 成功 → 刷新 fetchQueue（空）→ 回执收敛消失
    fireEvent.click(screen.getByTestId("queue-retract"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect(
      fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("/session-queue/retract"))
    ).toBe(true);
    expect(queueListCalls()).toBeGreaterThanOrEqual(1);
  });

  it("立即发送（插队）：按 itemId 点名直发，delivered 后回执转「已送达终端」", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.jump = { status: "delivered" };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "插队试试" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    fireEvent.click(screen.getByTestId("queue-jump"));
    expect(await screen.findByTestId("send-receipt-delivered")).toBeTruthy();
    const jumpCall = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-queue/jump")
    );
    expect(jumpCall).toBeTruthy();
    expect(JSON.parse(String((jumpCall![1] as RequestInit).body))).toEqual({
      sessionId: "sess-1",
      itemId: 7,
    });
    expect(screen.queryByTestId("send-receipt-queued")).toBeNull();
  });

  it("回车不触发发送（textarea 天然换行）；text.trim() 为空时发送按钮禁用", async () => {
    installFetch();
    routes.info = sendInfo();
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = (await screen.findByTestId("composer-input")) as HTMLTextAreaElement;
    const sendBtn = screen.getByTestId("composer-send") as HTMLButtonElement;
    expect(sendBtn.disabled).toBe(true); // 初始空文案禁用
    fireEvent.change(input, { target: { value: "   " } });
    expect(sendBtn.disabled).toBe(true); // 纯空白仍禁用
    fireEvent.change(input, { target: { value: "第一行" } });
    expect(sendBtn.disabled).toBe(false);
    fireEvent.keyDown(input, { key: "Enter" });
    await flushAsync();
    expect(sendCalls()).toHaveLength(0); // 回车不发送
    expect(input.value).toBe("第一行"); // 输入保持
  });

  it("发送失败（failed 回执）：红 chip 带错误文案、输入保留，可重试转绿", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "failed", error: "定位终端失败：tmux 会话不存在" };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "再来一次" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-failed");
    expect(chip.textContent).toContain("发送失败");
    expect(chip.textContent).toContain("定位终端失败：tmux 会话不存在");
    // 失败不清空输入（保留原文供重试）
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("再来一次");
    // 重试 = 再按发送：修正路由后转 delivered
    routes.send = { status: "delivered" };
    fireEvent.click(screen.getByTestId("composer-send"));
    expect(await screen.findByTestId("send-receipt-delivered")).toBeTruthy();
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    expect(sendCalls()).toHaveLength(2);
  });

  it("网络层 ApiError（如 500）：错误文案可见且可重试", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.sendStatus = 500;
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "你好" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-failed");
    expect(chip.textContent).toContain("500");
    routes.sendStatus = undefined;
    routes.send = { status: "delivered" };
    fireEvent.click(screen.getByTestId("composer-send"));
    expect(await screen.findByTestId("send-receipt-delivered")).toBeTruthy();
  });
});

describe("MessageComposer：排队轮询（3s 定时器，unmount 清理）", () => {
  it("排队态 3s 轮询刷新队位；条目从队列消失则回执收敛；卸载后停止轮询", async () => {
    vi.useFakeTimers();
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.queue = [queueItem({ id: 7, position: 2 })];
    const { unmount } = render(<MessageComposer session={{ id: "sess-1" }} />);
    await flushAsync(); // send-info 落地
    fireEvent.change(screen.getByTestId("composer-input"), { target: { value: "长任务" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await flushAsync(); // send → queued
    expect(screen.getByTestId("send-receipt-queued").textContent).toContain("第1位");
    expect(queueListCalls()).toBe(0); // 尚未到轮询点
    // 3s 轮询一次：队位已前移（队首被 flush）→ chip 更新为 第2位
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(queueListCalls()).toBe(1);
    expect(screen.getByTestId("send-receipt-queued").textContent).toContain("第2位");
    // 条目消失（已被 flush 送达/他端撤回）→ 回执收敛
    routes.queue = [];
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(screen.queryByTestId("send-receipt-queued")).toBeNull();
    // 卸载后定时器清理：不再轮询
    unmount();
    const before = queueListCalls();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9000);
    });
    expect(queueListCalls()).toBe(before);
  });
});
