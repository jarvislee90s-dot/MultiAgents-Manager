import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "@/mobile/App";
import type { SessionsResponse } from "@/types/session";

// stub 全局 fetch（盖过 setup.ts 的 msw），探测链路完全确定；结束还原防泄漏到其他用例
afterEach(() => {
  vi.unstubAllGlobals();
});

function okSessions(totalCount: number): Response {
  const body: SessionsResponse = { sessions: [], totalCount, waitingCount: 0 };
  return new Response(JSON.stringify(body), { status: 200 });
}

describe("App 配对状态机：探测成功 → 配对页卸载（已配对设备刷新免重配）", () => {
  it("已配对设备：Board 首拍拉到数据后配对页卸载，直接进看板", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => okSessions(2))
    );
    render(<App />);

    // 首帧：探测中（paired=null），配对页先出（沿用不闪白口径）
    expect(screen.getByText("MAM 远程接入")).toBeTruthy();

    // 首拍数据到达（"2 个会话" 仅在成功拍后渲染；首拍 = 探测）
    expect(await screen.findByText("2 个会话")).toBeTruthy();
    // 配对页已卸载 —— Critical 回归锁：删除首拍成功回调后本断言变红
    expect(screen.queryByText("MAM 远程接入")).toBeNull();
  });

  it("网络异常：不误判已配对，配对页保持且看板提示重试", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("network down");
      })
    );
    render(<App />);
    // 等 catch 路径执行完（loadError 横幅渲染在隐藏的 Board 里，DOM 存在即可查）
    expect(await screen.findByText(/网络连接失败/)).toBeTruthy();
    // 配对页仍在：网络异常不得触发 null→true 翻转
    expect(screen.getByText("MAM 远程接入")).toBeTruthy();
  });
});
