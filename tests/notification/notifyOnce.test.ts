// tests/notification/notifyOnce.test.ts — review F7③：首见未读绿卡只补发一次通知，
// 且受 F5 新鲜度门控约束（转绿 > 2 分钟的老卡静默）
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, sendNotificationMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  sendNotificationMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: sendNotificationMock,
  onAction: vi.fn(async () => () => {}),
  registerActionTypes: vi.fn(async () => {}),
}));
vi.mock("@/lib/audio", () => ({ playCompletionSound: vi.fn() }));
vi.mock("@/lib/notificationHistory", () => ({ addHistory: vi.fn() }));
// 宠物隐藏：浮窗路径走 invoke（mock 中强制失败）→ 降级系统通知 sendNotification
vi.mock("@/components/pet/petConfig", () => ({
  petSuppressPopup: () => false,
  petSoundTakeover: () => false,
}));

import { useSessionStore } from "@/stores/sessionStore";
import { useNotification } from "@/hooks/useNotification";

const freshUnread = {
  id: "s-fresh",
  agentType: "workbuddy",
  form: "app" as const,
  projectName: "Demo",
  title: "T",
  gitBranch: null,
  github_url: null,
  status: "idle",
  lastMessage: "ok",
  lastMessageRole: null,
  lastActivityAt: new Date(Date.now() - 30_000).toISOString(),
  pid: 0,
  cpuUsage: 0,
  activeSubagentCount: 0,
  jumpSupported: true,
  unread: true,
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    // 浮窗失败 → 强制走系统通知降级路径（可观测断言点）
    if (cmd === "show_notification_window") throw new Error("no popup in jsdom");
    return null;
  });
  sendNotificationMock.mockReset();
  useSessionStore.setState({ sessions: [] });
});

describe("首见未读通知（review F7③ + F5 门控）", () => {
  it("首见未读绿卡只补发一次；同卡重入不重发；超时老卡静默", async () => {
    const { rerender } = renderHook(() => useNotification());
    // 等初始化 effect（权限/设置）的微任务排空
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });

    // 首次出现新鲜未读绿卡 → 恰好 1 条系统通知
    act(() => {
      useSessionStore.setState({ sessions: [{ ...freshUnread }] });
    });
    await waitFor(() => expect(sendNotificationMock).toHaveBeenCalledTimes(1));

    // 同一会话再次轮询（新数组、同内容）→ 不重发
    act(() => {
      useSessionStore.setState({ sessions: [{ ...freshUnread }] });
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });
    expect(sendNotificationMock).toHaveBeenCalledTimes(1);

    // 另一张转绿超 2 分钟的老未读卡（补偿/重启残留）→ F5 门控静默
    const stale = {
      ...freshUnread,
      id: "s-stale",
      lastActivityAt: new Date(Date.now() - 10 * 60_000).toISOString(),
    };
    act(() => {
      useSessionStore.setState({ sessions: [stale] });
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });
    expect(sendNotificationMock).toHaveBeenCalledTimes(1);

    rerender();
  });
});
