// tests/hooks/useRemoteEvents.test.tsx — 评审修复 Fix Round 1（M4 T2）：
// useRemoteEvents 的 i18n 键曾错位（pairNotifyUnknown ≠ locales 的 pendingUnknown），
// check:i18n 只验中英成对、不验代码↔键一致性，既有测试又只渲染 RemoteSection
// 触达不到本 hook。本测试最轻覆盖：无 name 设备触发 remote-pair-request 时，
// 系统通知正文必须解析到真实文案（不得出现原始键路径）。
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const { handlers, listenMock, sendNotificationMock } = vi.hoisted(() => {
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
  };
});

vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock, emit: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: sendNotificationMock,
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { info: vi.fn(), success: vi.fn(), error: vi.fn() }),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { useRemoteEvents } from "@/hooks/useRemoteEvents";

void i18n;

describe("useRemoteEvents 配对请求系统通知（M4 T2 评审修复）", () => {
  it("无 name 设备：通知正文回落 pendingUnknown 真实文案，不得渲染原始键路径", async () => {
    renderHook(() => useRemoteEvents());
    // effect 内 async IIFE 逐个 await listen——等到目标监听注册完成
    await waitFor(() => expect(handlers.has("remote-pair-request")).toBe(true));

    const handler = handlers.get("remote-pair-request");
    if (!handler) throw new Error("remote-pair-request handler not registered");
    await handler({ payload: { name: "", ip: "192.168.1.9" } });
    await waitFor(() => expect(sendNotificationMock).toHaveBeenCalled());

    const arg = sendNotificationMock.mock.calls[0][0] as {
      title: string;
      body: string;
      actionTypeId: string;
    };
    // 键错位回归哨兵：正文/标题不得出现原始键路径（i18next 缺键即泄漏键名）
    expect(arg.body).not.toContain("settings.remote");
    expect(arg.title).not.toContain("settings.remote");
    expect(arg.body).toContain("New device"); // pendingUnknown（en）真实文案
    expect(arg.body).toContain("192.168.1.9");
    expect(arg.actionTypeId).toBe("open-pairing"); // 点击直达设置页（useNotification 单点分发）
  });
});
