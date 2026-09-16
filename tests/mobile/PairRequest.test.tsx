import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import PairPage from "@/mobile/PairPage";
import { ApiError, confirmPairing, pollPairing, requestPairing } from "@/mobile/api";

// 只 mock 请求接入三函数（状态机用例不依赖真实 fetch）；ApiError / pair 保持真实现
vi.mock("@/mobile/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/mobile/api")>();
  return { ...actual, requestPairing: vi.fn(), pollPairing: vi.fn(), confirmPairing: vi.fn() };
});

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

/** 把组件推进到 waiting 态：请求成功 + 首帧 tick（pending）冲刷完毕 */
async function gotoWaiting(requestId: string) {
  vi.mocked(requestPairing).mockResolvedValue({ requestId, expiresAt: 1 });
  vi.mocked(pollPairing).mockResolvedValue("pending");
  render(<PairPage onPaired={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
  await act(async () => {}); // 冲刷 requestPairing → waiting + 首帧 tick
}

describe("PairPage 请求接入流（M4 T2）", () => {
  it("请求→pending→approved 状态机：approved 即回调 onPaired", async () => {
    const onPaired = vi.fn();
    vi.mocked(requestPairing).mockResolvedValue({ requestId: "r0", expiresAt: 1 });
    vi.mocked(pollPairing).mockResolvedValueOnce("pending").mockResolvedValueOnce("approved");
    render(<PairPage onPaired={onPaired} />);

    fireEvent.change(screen.getByPlaceholderText("设备名称（选填）"), {
      target: { value: "我的手机" },
    });
    fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
    await act(async () => {});

    // waiting 态：等待批准文案 + 首帧 tick 已发（pending），尚未批准
    expect(screen.getByText(/等待桌面批准/)).toBeTruthy();
    expect(requestPairing).toHaveBeenCalledWith("我的手机");
    expect(pollPairing).toHaveBeenCalledTimes(1);
    expect(onPaired).not.toHaveBeenCalled();

    // 3s 后第二次 tick → approved → onPaired
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });
    expect(pollPairing).toHaveBeenCalledTimes(2);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("轮询 expired → 回 idle 并提示重新发起", async () => {
    vi.mocked(requestPairing).mockResolvedValue({ requestId: "r1", expiresAt: 1 });
    vi.mocked(pollPairing).mockResolvedValue("expired");
    render(<PairPage onPaired={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
    await act(async () => {});

    expect(screen.getByText(/请求已过期（5 分钟），请重新发起/)).toBeTruthy();
    // 回 idle：请求接入按钮重新出现
    expect(screen.getByRole("button", { name: "请求接入" })).toBeTruthy();
  });

  it("429 queue_full / ip_busy → 同一友好文案「请求过多，请稍后再试」", async () => {
    vi.mocked(requestPairing).mockRejectedValue(new ApiError(429, "queue_full"));
    render(<PairPage onPaired={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
    await act(async () => {});
    expect(screen.getByText("请求过多，请稍后再试")).toBeTruthy();

    // ip_busy 同文案（不给队列状态预言机）
    vi.mocked(requestPairing).mockRejectedValue(new ApiError(429, "ip_busy"));
    fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
    await act(async () => {});
    expect(screen.getByText("请求过多，请稍后再试")).toBeTruthy();
  });

  it("确认码 trim 后不足 4 位不提交；wrong → 剩余次数文案", async () => {
    await gotoWaiting("r2");
    const codeInput = screen.getByPlaceholderText("4 位确认码");

    fireEvent.change(codeInput, { target: { value: " 12 " } });
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    expect(confirmPairing).not.toHaveBeenCalled();

    vi.mocked(confirmPairing).mockResolvedValue({ ok: false, error: "wrong", triesLeft: 2 });
    fireEvent.change(codeInput, { target: { value: "9027" } });
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    await act(async () => {});

    expect(confirmPairing).toHaveBeenCalledWith("r2", "9027");
    expect(screen.getByText(/确认码错误，剩余 2 次机会/)).toBeTruthy();
  });

  it("cap_full → 设备已满文案，保持在 waiting 可重试", async () => {
    await gotoWaiting("r3");
    vi.mocked(confirmPairing).mockResolvedValue({ ok: false, error: "cap_full" });
    fireEvent.change(screen.getByPlaceholderText("4 位确认码"), { target: { value: "1234" } });
    fireEvent.click(screen.getByRole("button", { name: "确认" }));
    await act(async () => {});

    expect(screen.getByText("设备已满，请在桌面端花名册腾位后重试")).toBeTruthy();
    // 未回 idle：等待批准视图仍在
    expect(screen.getByText(/等待桌面批准/)).toBeTruthy();
  });

  it("卸载即停轮询（stop 标志 + clearInterval）", async () => {
    vi.mocked(requestPairing).mockResolvedValue({ requestId: "r4", expiresAt: 1 });
    vi.mocked(pollPairing).mockResolvedValue("pending");
    const { unmount } = render(<PairPage onPaired={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "请求接入" }));
    await act(async () => {});
    expect(pollPairing).toHaveBeenCalledTimes(1);

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9000);
    });
    expect(pollPairing).toHaveBeenCalledTimes(1); // 卸载后无新 tick
  });
});
