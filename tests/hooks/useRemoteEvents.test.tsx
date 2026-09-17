// tests/hooks/useRemoteEvents.test.tsx — M5 A6 更新：/pair/pin 认证制落地后
// remote-pair-request 配对请求事件已随审批制下线（A6 删除监听与 i18n 文案），本测试
// 改为锁定两件事：① 监听注册表里不再出现 remote-pair-request（防回潮）；
// ② 保留事件（隧道地址就绪）仍解析到真实文案，不得出现原始键路径（键错位回归哨兵，
// check:i18n 只验中英成对、不验代码↔键一致性）。
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { handlers, listenMock, sendNotificationMock, toastSuccessMock } = vi.hoisted(() => {
  const handlers = new Map<string, (e: { payload: unknown }) => void | Promise<void>>();
  return {
    handlers,
    listenMock: vi.fn(
      async (event: string, handler: (e: { payload: unknown }) => void | Promise<void>) => {
        handlers.set(event, handler);
        return () => {
          handlers.delete(event);
        };
      }
    ),
    sendNotificationMock: vi.fn(),
    toastSuccessMock: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock, emit: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: sendNotificationMock,
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { info: vi.fn(), success: toastSuccessMock, error: vi.fn() }),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { useRemoteEvents } from "@/hooks/useRemoteEvents";

void i18n;

describe("useRemoteEvents 全局远程事件（M5 A6）", () => {
  it("remote-pair-request 监听已随审批制下线，不得再注册", async () => {
    renderHook(() => useRemoteEvents());
    // 等 effect 内 async IIFE 把保留事件全部注册完
    await waitFor(() => expect(handlers.has("remote-tunnel-address")).toBe(true));
    expect(handlers.has("remote-pair-request")).toBe(false);
  });

  it("remote-tunnel-address：toast 解析 tunnelReadyToast 真实文案（键错位回归哨兵）", async () => {
    renderHook(() => useRemoteEvents());
    await waitFor(() => expect(handlers.has("remote-tunnel-address")).toBe(true));
    const handler = handlers.get("remote-tunnel-address");
    if (!handler) throw new Error("remote-tunnel-address handler not registered");
    await handler({ payload: { url: "https://mam.example.com/m" } });
    await waitFor(() => expect(toastSuccessMock).toHaveBeenCalled());
    const msg = String(toastSuccessMock.mock.calls[0][0]);
    expect(msg).not.toContain("settings.remote"); // 缺键即泄漏键路径
    expect(msg).toContain("https://mam.example.com/m");
  });
});
