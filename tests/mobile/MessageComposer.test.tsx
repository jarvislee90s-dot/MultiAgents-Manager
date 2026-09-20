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
  /** 非 2xx 时响应体 JSON（403 not_injectable{reason,reasonCode} 等，P2-10） */
  sendBody?: Record<string, unknown>;
  /** 发送请求挂起不响应（灰3：投递中 chip 的慢消费者场景） */
  sendHang?: boolean;
  queue?: QueueItemView[];
  jump?: Record<string, unknown>;
  retractOk?: boolean;
  /** 撤回网络层异常（fetch 直接 reject，P2-7 对账测试用） */
  retractReject?: boolean;
  /** 撤回忙时拒收：200 {status:"failed",error}（P2-6 忙时回执，条目仍在队） */
  retractBusy?: boolean;
  /** 队列列表 /session-queue 拉取网络层异常（复核失败场景） */
  queueReject?: boolean;
  /** 附件上传（2026-09-20）：成功载荷 / 413·404 等非 2xx 状态 */
  attach?: { path: string; size: number };
  attachStatus?: number;
  attachError?: string;
  /** 附件上传挂起不响应（上传中禁发测试） */
  attachHang?: boolean;
}

let routes: Routes;
let fetchMock: ReturnType<typeof vi.fn>;
/** sendHang 挂起请求的放行器（测试中手动 resolve 模拟响应到达） */
let releaseSend: ((r: Response) => void) | null = null;
/** attachHang 挂起请求的放行器（上传中禁发测试用） */
let releaseAttach: ((r: Response) => void) | null = null;

beforeEach(() => {
  routes = {};
  releaseSend = null;
  releaseAttach = null;
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
      if (routes.retractReject) throw new TypeError("retract 网络断开（模拟网络层异常）");
      if (routes.retractBusy) {
        return new Response(JSON.stringify({ status: "failed", error: "投递进行中，请稍后重试" }), {
          status: 200,
        });
      }
      return new Response(JSON.stringify({ ok: routes.retractOk ?? true }), { status: 200 });
    }
    if (url.includes("/session-send")) {
      if (routes.sendHang) {
        return new Promise<Response>((resolve) => {
          releaseSend = resolve;
        });
      }
      if (routes.sendStatus) {
        return new Response(JSON.stringify(routes.sendBody ?? { error: "internal" }), {
          status: routes.sendStatus,
        });
      }
      return new Response(JSON.stringify(routes.send ?? { status: "delivered" }), { status: 200 });
    }
    if (url.includes("/session-attachment")) {
      if (routes.attachHang) {
        return new Promise<Response>((resolve) => {
          releaseAttach = resolve;
        });
      }
      if (routes.attachStatus) {
        return new Response(JSON.stringify({ error: routes.attachError ?? "too_large" }), {
          status: routes.attachStatus,
        });
      }
      return new Response(
        JSON.stringify(routes.attach ?? { path: "E:/proj/.mam-attachments/s-1/1-a.png", size: 5 }),
        { status: 200 }
      );
    }
    if (url.includes("/session-queue")) {
      if (routes.queueReject) throw new TypeError("queue 网络断开（模拟复核失败）");
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
    expect(
      await screen.findByTestId("send-disabled-reason").then((el) => el.textContent)
    ).toContain("WorkBuddy 黑盒会话无法注入");
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByTestId("composer-send") as HTMLButtonElement).disabled).toBe(true);
  });

  it("排队态：回执「排队中 第1位」+ 立即发送/撤回；撤回成功（{ok:true}）→ 免复核直接收敛", async () => {
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
    // 撤回：服务端确认已撤（200 {ok:true}）→ 免复核直接收敛（评审必须1快路径）
    fireEvent.click(screen.getByTestId("queue-retract"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect(
      fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("/session-queue/retract"))
    ).toBe(true);
    expect(queueListCalls()).toBe(0); // {ok:true} 快路径不再拉队列复核
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

// ==== M9R 注入加固前端对齐（P2-7 对账恢复 / 灰3 投递中 chip / P2-10 补锁）====
describe("MessageComposer：M9R 注入加固（P2-7 / 灰3 / P2-10）", () => {
  it("jump_busy_restores_queued_view：插队遇忙（200 failed）→ fetchQueue 复核条目仍在 → 恢复排队视图与按钮；复核确认不在队 → 中性 gone 收敛（不落 failed）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.jump = { status: "failed", error: "投递进行中，请稍后重试" };
    routes.queue = [queueItem({ id: 7, position: 2 })];
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "插队一下" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    // 插队遇忙 → 复核 /session-queue → 条目仍在 pending → 恢复排队视图，
    // 队位刷新 第1位→第2位（队位变化即复核发生的真证据，不用调用计数——轮询同端点）
    fireEvent.click(screen.getByTestId("queue-jump"));
    const chip = await screen.findByTestId("send-receipt-queued");
    expect(chip.textContent).toContain("排队中 第2位");
    await waitFor(() =>
      expect((screen.getByTestId("queue-jump") as HTMLButtonElement).disabled).toBe(false)
    );
    expect(screen.getByTestId("queue-retract")).toBeTruthy();
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    // 复核真不在队（并发消费已把条目投出）→ 中性 gone 收敛：
    // 大概率已送达（守卫方 flush 循环刚投出），不得落 failed「可重试」诱发重复注入
    routes.queue = [];
    fireEvent.click(screen.getByTestId("queue-jump"));
    const gone = await screen.findByTestId("send-receipt-gone");
    expect(gone.textContent).toBe("条目已离开队列（可能已送达，可在会话内容中确认）");
    expect(gone.textContent).not.toContain("可重试");
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    expect(screen.queryByTestId("send-receipt-queued")).toBeNull();
  });

  it("retract_failure_same_reconcile：撤回网络错 → 同款队列复核 → 条目仍在 → 恢复排队视图（不落 failed 终态，可重试）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.retractReject = true;
    routes.queue = [queueItem({ id: 7, position: 1 })];
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "待撤回" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    fireEvent.click(screen.getByTestId("queue-retract"));
    // 网络错 → 复核 /session-queue → 条目仍在队 → 排队视图恢复（按钮回到可点，可重试撤回）
    const chip = await screen.findByTestId("send-receipt-queued");
    expect(chip.textContent).toContain("排队中 第1位");
    await waitFor(() =>
      expect((screen.getByTestId("queue-retract") as HTMLButtonElement).disabled).toBe(false)
    );
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
  });

  it("maxlength_guard：textarea maxLength=10000；超限输入被截断（与后端 MAX_SEND_CHARS 对齐）", async () => {
    installFetch();
    routes.info = sendInfo();
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = (await screen.findByTestId("composer-input")) as HTMLTextAreaElement;
    expect(input.maxLength).toBe(10000);
    fireEvent.change(input, { target: { value: "a".repeat(10003) } });
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toHaveLength(10000);
  });

  it("delivering_chip_during_await：发送 await 全程显示「投递中…」chip，完成后被结果 chip 覆盖（灰3 慢消费者不空白）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.sendHang = true;
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "一万字长文（慢消费者）" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    // await 未返回期间：投递中 chip 在场（长文投递可达分钟级，界面不空白）
    const delivering = await screen.findByTestId("send-receipt-delivering");
    expect(delivering.textContent).toContain("投递中…");
    expect(delivering.textContent).toContain("长文投递可能需要几分钟"); // 副文案（评审 Minor4）
    expect(screen.queryByTestId("send-receipt-delivered")).toBeNull();
    // 放行响应：结果 chip 覆盖投递中
    await act(async () => {
      releaseSend!(new Response(JSON.stringify({ status: "delivered" }), { status: 200 }));
    });
    expect(await screen.findByTestId("send-receipt-delivered")).toBeTruthy();
    expect(screen.queryByTestId("send-receipt-delivering")).toBeNull();
  });

  it("send_403_reason_chip：session-send 403 not_injectable → 失败 chip 含后端 reason 原文（P2-10）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.sendStatus = 403;
    routes.sendBody = {
      error: "not_injectable",
      reason: "会话形态已漂移为黑盒，无法注入",
      reasonCode: "blackbox",
    };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "你好" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-failed");
    expect(chip.textContent).toContain("会话形态已漂移为黑盒，无法注入");
  });

  it("queued position=0 容忍：并发消费窗口返回位次 0 → 显示「排队中」不带位次数字，按钮保留", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 0 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "刚入队即撞上消费窗口" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-queued");
    expect(chip.textContent).toBe("排队中");
    expect(screen.getByTestId("queue-jump")).toBeTruthy();
    expect(screen.getByTestId("queue-retract")).toBeTruthy();
  });

  it("retract busy 分流（评审必须1）：撤回忙时（200 failed 条目仍在队）→ 复核在队恢复（队位 1→2 为复核真证据）；复核也失败 → 保守恢复排队视图不失控", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.retractBusy = true; // 后端忙时拒收：200 {status:"failed"}，条目未被撤
    routes.queue = [queueItem({ id: 7, position: 2 })];
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "撤回我" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    // 阶段一：忙时撤回 → 复核 /session-queue → 条目仍在 → 恢复排队视图；
    // 队位 第1位→第2位（chip 变化即复核发生的真证据，不用调用计数——轮询同端点会污染）
    fireEvent.click(screen.getByTestId("queue-retract"));
    const chip = await screen.findByTestId("send-receipt-queued");
    expect(chip.textContent).toContain("排队中 第2位");
    await waitFor(() =>
      expect((screen.getByTestId("queue-retract") as HTMLButtonElement).disabled).toBe(false)
    );
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    expect(screen.queryByTestId("send-receipt-gone")).toBeNull();
    // 阶段二：复核也网络失败 → 保守恢复排队视图（沿用最近已知队位），条目实际仍在队、
    // 撤回/插队按钮不失控，3s 轮询随后自愈——不得落回执消失/终态
    routes.queueReject = true;
    fireEvent.click(screen.getByTestId("queue-retract"));
    await screen.findByTestId("send-receipt-queued");
    expect(screen.getByTestId("send-receipt-queued").textContent).toContain("排队中 第2位");
    await waitFor(() =>
      expect((screen.getByTestId("queue-retract") as HTMLButtonElement).disabled).toBe(false)
    );
    expect(screen.getByTestId("queue-jump")).toBeTruthy();
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    expect(screen.queryByTestId("send-receipt-gone")).toBeNull();
  });

  it("retract gone 中性收敛（评审必须2）：撤回失败后复核确认不在队 → 中性「条目已不在队列」，不标失败不带可重试", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.retractReject = true; // 撤回网络错
    routes.queue = []; // 复核确认条目已不在队（已被消费/他端撤回）
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "试试撤回" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    fireEvent.click(screen.getByTestId("queue-retract"));
    const gone = await screen.findByTestId("send-receipt-gone");
    expect(gone.textContent).toBe("条目已不在队列");
    expect(gone.textContent).not.toContain("可重试");
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    expect(screen.queryByTestId("send-receipt-queued")).toBeNull();
  });

  it("sending 期间排队按钮加闸（评审必须3）：发送 await 未返回时 queue-jump/queue-retract 可见但禁用", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "第一条（入队）" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    // 第二条发送挂起（慢消费者）：sending=true 期间排队操作面可见但不可点
    routes.sendHang = true;
    fireEvent.change(screen.getByTestId("composer-input"), {
      target: { value: "第二条（慢投递）" },
    });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivering");
    expect(screen.getByTestId("queue-jump")).toBeTruthy(); // 可见（不失联）
    expect(screen.getByTestId("queue-retract")).toBeTruthy();
    expect((screen.getByTestId("queue-jump") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("queue-retract") as HTMLButtonElement).disabled).toBe(true);
    // 放行第二条 → delivered：queued chip 收敛，闸随 sending 翻转解除
    await act(async () => {
      releaseSend!(new Response(JSON.stringify({ status: "delivered" }), { status: 200 }));
    });
    expect(await screen.findByTestId("send-receipt-delivered")).toBeTruthy();
    expect(screen.queryByTestId("send-receipt-queued")).toBeNull();
  });
});

// ==== 「修改」按钮 + 排队条目他端消失提示（2026-09-20 用户裁决）====
describe("排队回执：修改按钮（撤回保持丢弃语义）", () => {
  /** 发送一条进入排队态的公共前缀：回执 queued{itemId:7,position:1,content} */
  async function sendIntoQueued(text = "跑个长任务") {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: text } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    return input;
  }

  it("排队态三钮并存：立即发送 / 修改 / 撤回", async () => {
    await sendIntoQueued();
    expect(screen.getByTestId("queue-jump").textContent).toContain("立即发送");
    expect(screen.getByTestId("queue-edit").textContent).toContain("修改");
    expect(screen.getByTestId("queue-retract").textContent).toContain("撤回");
  });

  it("修改：确认出队（{ok:true}）→ 正文放回输入框（可继续编辑），回执收敛", async () => {
    const input = await sendIntoQueued("跑个长任务");
    fireEvent.click(screen.getByTestId("queue-edit"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("跑个长任务");
    expect(
      fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("/session-queue/retract"))
    ).toBe(true);
  });

  it("修改：正文超长时截断到 MAX_SEND_CHARS（与输入框 maxLength 对齐）", async () => {
    const long = "长".repeat(10001);
    const input = await sendIntoQueued(long);
    fireEvent.click(screen.getByTestId("queue-edit"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toHaveLength(10000);
  });

  it("修改：忙时 failed（条目仍在队）→ 排队视图保留、输入框保持为空（防双份）", async () => {
    routes.retractBusy = true;
    routes.queue = [queueItem({ id: 7, position: 1, content: "跑个长任务" })];
    const input = await sendIntoQueued("跑个长任务");
    fireEvent.click(screen.getByTestId("queue-edit"));
    // 复核发现条目仍在队 → 恢复排队视图；正文不得放回（否则队里 + 输入框双份）
    await waitFor(() => expect(queueListCalls()).toBeGreaterThan(0));
    expect(screen.getByTestId("send-receipt-queued")).toBeTruthy();
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("");
  });

  it("撤回回归锁：仍为完全取消——确认出队后回执收敛且输入框保持为空", async () => {
    const input = await sendIntoQueued("只想取消");
    fireEvent.click(screen.getByTestId("queue-retract"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("");
  });
});

// ==== 修改重发只入队（D6，验收问题 #4）：修改后的重发强制走队列，防变相插队 ====
describe("修改重发只入队（D6）", () => {
  /** 发送一条进入排队态的公共前缀：回执 queued{itemId:7,position:1,content} */
  async function sendIntoQueued(text: string) {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: text } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    return input;
  }

  it("edit_resend_queues_only：修改确认出队后再发送 → 请求体带 queueOnly:true（手动改字不清标志）", async () => {
    const input = await sendIntoQueued("跑个长任务");
    fireEvent.click(screen.getByTestId("queue-edit"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect((input as HTMLTextAreaElement).value).toBe("跑个长任务");
    // 用户手动改字不清除标志（保守语义：修改后的重发一律入队）。入队后的放行
    // 节奏：会话转闲跃迁后事件臂即时放行；已空闲且无跃迁时由 60s 周期兜底放行
    // （可达分钟级）
    fireEvent.change(input, { target: { value: "改好的重发" } });
    routes.send = { status: "queued", itemId: 9, position: 1 };
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    // 调用序列：[0]=初次入队发送（不带标志），[1]=修改后的重发（带标志）
    expect(sendCalls()).toHaveLength(2);
    expect("queueOnly" in JSON.parse(String((sendCalls()[0][1] as RequestInit).body))).toBe(false);
    expect(JSON.parse(String((sendCalls()[1][1] as RequestInit).body))).toEqual({
      sessionId: "sess-1",
      text: "改好的重发",
      queueOnly: true,
    });
  });

  it("normal_send_omits_flag：未经修改的普通发送 → 请求体不含 queueOnly 键", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "普通发送" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    expect(sendCalls()).toHaveLength(1);
    const body = JSON.parse(String((sendCalls()[0][1] as RequestInit).body));
    // api.ts 口径：未修改不带该键（缺省键，保持既有请求体形态零漂移）
    expect("queueOnly" in body).toBe(false);
    expect(body).toEqual({ sessionId: "sess-1", text: "普通发送" });
  });

  it("flag_consumed_on_send：修改→发送（带标志）→再发送 → 第二次不再带标志（消费即清）", async () => {
    await sendIntoQueued("第一版");
    fireEvent.click(screen.getByTestId("queue-edit"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    routes.send = { status: "delivered" };
    fireEvent.click(screen.getByTestId("composer-send")); // 修改后的重发：带标志
    await screen.findByTestId("send-receipt-delivered");
    // 调用序列：[0]=初次入队发送，[1]=修改后的重发（带标志）
    expect(sendCalls()).toHaveLength(2);
    expect(JSON.parse(String((sendCalls()[1][1] as RequestInit).body)).queueOnly).toBe(true);
    // 第二次发送：标志已消费即清，回归普通发送语义（失败重试/新消息均不带）
    fireEvent.change(screen.getByTestId("composer-input"), { target: { value: "下一条" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    expect(sendCalls()).toHaveLength(3);
    const third = JSON.parse(String((sendCalls()[2][1] as RequestInit).body));
    expect("queueOnly" in third).toBe(false);
  });

  it("session_switch_drops_flag：换会话（同组件 rerender）即弃修改标志 → 新会话首条普通发送不含 queueOnly", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    const view = render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "旧会话排队" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-queued");
    // 修改确认出队 → 标志置位（正文放回输入框）
    fireEvent.click(screen.getByTestId("queue-edit"));
    await waitFor(() => expect(screen.queryByTestId("send-receipt-queued")).toBeNull());
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("旧会话排队");
    // 同组件换会话：queueOnlyNext 随 session.id effect 复位——旧会话「修改」的
    // 遗愿不得泄漏为新会话首条发送的入队意图
    view.rerender(<MessageComposer session={{ id: "sess-2" }} />);
    const input2 = await screen.findByTestId("composer-input"); // send-info 重拉后重新就绪
    routes.send = { status: "delivered" };
    fireEvent.change(input2, { target: { value: "新会话首条" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    expect(sendCalls()).toHaveLength(2);
    const body = JSON.parse(String((sendCalls()[1][1] as RequestInit).body));
    expect("queueOnly" in body).toBe(false);
    expect(body).toEqual({ sessionId: "sess-2", text: "新会话首条" });
  });

  it("edit_busy_failure_does_not_arm_flag：修改忙时失败（复核条目仍在队）→ 不置标志，随后的新文本发送不含 queueOnly", async () => {
    routes.retractBusy = true; // 忙时拒收：条目未被撤、仍在队
    routes.queue = [queueItem({ id: 7, position: 1, content: "跑个长任务" })];
    const input = await sendIntoQueued("跑个长任务");
    fireEvent.click(screen.getByTestId("queue-edit"));
    // 复核条目仍在队 → 排队视图恢复、正文不放回（onConfirmed 未触发 → 标志未置位）
    await waitFor(() => expect(queueListCalls()).toBeGreaterThan(0));
    expect(screen.getByTestId("send-receipt-queued")).toBeTruthy();
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("");
    // 随后输入全新文本发送：这是未经修改确认的普通发送，不得携带入队标志
    routes.send = { status: "delivered" };
    fireEvent.change(input, { target: { value: "全新消息" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    expect(sendCalls()).toHaveLength(2);
    const body = JSON.parse(String((sendCalls()[1][1] as RequestInit).body));
    expect("queueOnly" in body).toBe(false);
  });
});

describe("排队条目他端消失（2026-09-20 调查修复）：轮询收敛留痕", () => {
  it("3s 轮询发现条目不在队 → 中性 gone 提示（含电脑端去向），不再静默消失", async () => {
    vi.useFakeTimers();
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "queued", itemId: 7, position: 1 };
    routes.queue = []; // 轮询时条目已不在队（被 flush 送达 / 桌面端处理）
    render(<MessageComposer session={{ id: "sess-1" }} />);
    await act(async () => {});
    fireEvent.change(screen.getByTestId("composer-input"), {
      target: { value: "长任务" },
    });
    fireEvent.click(screen.getByTestId("composer-send"));
    await act(async () => {});
    expect(screen.getByTestId("send-receipt-queued")).toBeTruthy();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3100);
    });
    const gone = screen.getByTestId("send-receipt-gone");
    expect(gone.textContent).toContain("电脑端");
    vi.useRealTimers();
  });
});

// ==== 直发确认分诊回执（D7/T3，验收问题 #5）：submitted 中性回执非失败 ====
describe("直发确认分诊回执（D7/T3）", () => {
  it("submitted 中性回执：中性文案、不带可重试、输入框已清空（对齐 delivered 口径）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "submitted" };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "已被 TUI 收进队列的消息" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-submitted");
    expect(chip.textContent).toBe("已投递至终端输入，agent 空闲后处理（未确认落盘）");
    // 中性态：不冒充失败——failed chip 不在场、无「可重试」语义（重试 = 双发，
    // TUI 那份无法撤回）
    expect(chip.textContent).not.toContain("可重试");
    expect(screen.queryByTestId("send-receipt-failed")).toBeNull();
    // 消息已离开前端 → 输入框清空（对齐 delivered 口径）
    expect((screen.getByTestId("composer-input") as HTMLTextAreaElement).value).toBe("");
  });

  it("failed 防重警示回执照旧（T3 回归）：红 chip 携带后端防重文案与（可重试）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = {
      status: "failed",
      error: "已注入未确认（未见会话记录），请检查终端后重试",
    };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "滞留真失败" } });
    fireEvent.click(screen.getByTestId("composer-send"));
    const chip = await screen.findByTestId("send-receipt-failed");
    expect(chip.textContent).toContain("发送失败");
    expect(chip.textContent).toContain("已注入未确认（未见会话记录），请检查终端后重试");
    expect(chip.textContent).toContain("（可重试）");
    // 分诊两态不混淆：failed 不得渲染成 submitted 中性 chip
    expect(screen.queryByTestId("send-receipt-submitted")).toBeNull();
  });
});

// ==== 附件上传（2026-09-20）：+ 钮 / 粘贴图片 / 发送拼内联标记行 ====
describe("移动端附件上传（2026-09-20）", () => {
  /** jsdom 的 Blob 可能缺 arrayBuffer（Node 内建 File 才有）——兜底补齐 */
  function ensureFileArrayBuffer(file: File) {
    const proto = Object.getPrototypeOf(file) as { arrayBuffer?: unknown };
    if (typeof proto.arrayBuffer !== "function") {
      (Object.getPrototypeOf(file) as { arrayBuffer: () => Promise<ArrayBuffer> }).arrayBuffer =
        async () => new TextEncoder().encode("x").buffer as ArrayBuffer;
    }
  }

  function pngFile(name = "shot.png"): File {
    const f = new File([new Uint8Array([0x89, 0x50])], name, { type: "image/png" });
    ensureFileArrayBuffer(f);
    return f;
  }

  async function sendIntoQueuedWithAttachment() {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    routes.attach = { path: "E:/proj/.mam-attachments/s-1/1-shot.png", size: 2 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "看这张图" } });
    fireEvent.change(screen.getByTestId("attach-file-input"), {
      target: { files: [pngFile()] },
    });
    // 上传完成 → ready chip 出现（名字显示）
    await screen.findByText("shot.png");
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
  }

  it("选文件上传：chip 状态机 uploading→ready；发送文本含 <image path> 标记行", async () => {
    await sendIntoQueuedWithAttachment();
    const sendCall = sendCalls()[0];
    const sent = JSON.parse(String((sendCall![1] as RequestInit).body)).text as string;
    expect(sent).toContain("看这张图");
    expect(sent).toContain('<image path="E:/proj/.mam-attachments/s-1/1-shot.png">');
    // 发送成功 → chips 清空
    expect(screen.queryByTestId(/^attach-chip-/)).toBeNull();
    // 上传端点被调用（query 含 session_id 与文件名）
    const attachCall = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-attachment")
    );
    expect(String(attachCall![0])).toContain("name=shot.png");
  });

  it("文档附件（非图片）发送拼 <file path> 标记行", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    routes.attach = { path: "E:/proj/.mam-attachments/s-1/1-报告.docx", size: 9 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "见附件" } });
    const docx = new File([new Uint8Array([1, 2])], "报告.docx", {
      type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    });
    ensureFileArrayBuffer(docx);
    fireEvent.change(screen.getByTestId("attach-file-input"), { target: { files: [docx] } });
    await screen.findByText("报告.docx");
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    const sendCall = sendCalls()[0];
    const sent = JSON.parse(String((sendCall![1] as RequestInit).body)).text as string;
    expect(sent).toContain('<file path="E:/proj/.mam-attachments/s-1/1-报告.docx">');
  });

  it("粘贴图片：textarea onPaste 捕获 clipboard 图片文件并走上传链路", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    routes.attach = { path: "E:/proj/.mam-attachments/s-1/1-paste.png", size: 2 };
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "贴图" } });
    const file = pngFile("pasted.png");
    fireEvent.paste(input, { clipboardData: { files: [file] } });
    await screen.findByText("pasted.png");
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
    const sendCall = sendCalls()[0];
    const sent = JSON.parse(String((sendCall![1] as RequestInit).body)).text as string;
    expect(sent).toContain('<image path="E:/proj/.mam-attachments/s-1/1-paste.png">');
  });

  it("上传中禁发（挂起请求不放行）；放行后 ready 可发送", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    routes.attachHang = true;
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(input, { target: { value: "边传边发？" } });
    fireEvent.change(screen.getByTestId("attach-file-input"), {
      target: { files: [pngFile("hang.png")] },
    });
    // chip 出现（uploading 态，文本前缀「上传中：」）→ 发送钮禁用
    await screen.findByTestId("attachment-chips");
    expect(screen.getByText("上传中：hang.png")).toBeTruthy();
    expect((screen.getByTestId("composer-send") as HTMLButtonElement).disabled).toBe(true);
    // 上传中 × 可点：移除 = 取消（中断在途 fetch，chip 消失，不再落盘）
    fireEvent.click(screen.getByRole("button", { name: "移除附件 hang.png" }));
    // 放行上传 → ready → 可发送
    releaseAttach!(new Response(JSON.stringify({ path: "E:/p", size: 1 }), { status: 200 }));
    await waitFor(() =>
      expect((screen.getByTestId("composer-send") as HTMLButtonElement).disabled).toBe(false)
    );
    // 取消后 chip 消失、发送钮恢复可用（无 ready 附件也不拦发送）
    await waitFor(() => screen.queryByTestId("attach-chips") === null);
    fireEvent.click(screen.getByTestId("composer-send"));
    await screen.findByTestId("send-receipt-delivered");
  });

  it("404 no_cwd：chip 标失败 + 「+」钮禁用（与 resume 禁用口径同源）", async () => {
    installFetch();
    routes.info = sendInfo();
    routes.send = { status: "delivered" };
    routes.attachStatus = 404;
    routes.attachError = "no_cwd";
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    fireEvent.change(screen.getByTestId("attach-file-input"), {
      target: { files: [pngFile("nc.png")] },
    });
    const chip = await screen.findByText("失败：nc.png", { exact: false });
    expect(chip.textContent).toContain("该会话没有项目目录信息");
    expect((screen.getByTestId("attach-add") as HTMLButtonElement).disabled).toBe(true);
  });

  it("「?」徽标：点开展开存储说明（含用户项目目录字样），再点收起", async () => {
    installFetch();
    routes.info = sendInfo();
    render(<MessageComposer session={{ id: "sess-1" }} />);
    const input = await screen.findByTestId("composer-input");
    expect(screen.queryByTestId("attach-hint")).toBeNull();
    fireEvent.click(screen.getByTestId("attach-help"));
    const hint = screen.getByTestId("attach-hint");
    expect(hint.textContent).toContain("用户项目目录");
    expect(hint.textContent).toContain(".mam-attachments");
    fireEvent.click(screen.getByTestId("attach-help"));
    expect(screen.queryByTestId("attach-hint")).toBeNull();
  });
});
