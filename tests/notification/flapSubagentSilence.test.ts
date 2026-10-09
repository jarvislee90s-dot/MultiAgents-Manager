// tests/notification/flapSubagentSilence.test.ts — F2b 子 agent 活动打标静默（终审发现 C）：
// 后端打标（Session.flapFromSubagentActivity，决策 4 谓词）+ 桌面开关
// silence_subagent_activity_flap（默认 "true"=静默）→ 打标跃迁的整条通知流静默
// （含历史，早退语义同 §四 开关）；开关关 / 未打标 → 照常
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, sendNotificationMock, addHistoryMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  sendNotificationMock: vi.fn(),
  addHistoryMock: vi.fn(),
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
vi.mock("@/lib/notificationHistory", () => ({ addHistory: addHistoryMock }));
vi.mock("@/components/pet/petConfig", () => ({
  petSuppressPopup: () => false,
  petSoundTakeover: () => false,
}));

import { useSessionStore } from "@/stores/sessionStore";
import { useNotification, GREEN_STABLE_MS } from "@/hooks/useNotification";

const mkSession = (id: string, status: string, flap: boolean) => ({
  id,
  agentType: "claude",
  form: "cli" as const,
  projectName: "Demo",
  title: "T",
  gitBranch: null,
  githubUrl: null,
  status,
  lastMessage: "ok",
  lastMessageRole: null,
  lastMessageSubagentReport: false,
  flapFromSubagentActivity: flap,
  lastActivityAt: new Date().toISOString(),
  pid: 1,
  cpuUsage: 0,
  activeSubagentCount: 2,
  jumpSupported: true,
  unread: false,
});

// 排空微任务（同 greenStableWindow/sameDirectionThrottle 的 drain）：setState 触发的
// 异步播报链（invoke×3 → 浮窗失败 → 降级 sendNotification）需要多轮微任务
async function flushInit() {
  for (let i = 0; i < 10; i++) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "show_notification_window") throw new Error("no popup in jsdom");
    return null; // silence_subagent_activity_flap 缺省 → 默认静默
  });
  sendNotificationMock.mockReset();
  addHistoryMock.mockReset();
  useSessionStore.setState({ sessions: [] });
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("子 agent 活动打标静默（silence_subagent_activity_flap）", () => {
  it("打标 + 默认静默：黄→绿不弹通知不进历史", async () => {
    renderHook(() => useNotification());
    await flushInit();

    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "processing", true)] });
    });
    await flushInit();
    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "idle", true)] });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(GREEN_STABLE_MS + 100);
    });
    await flushInit();

    expect(sendNotificationMock).not.toHaveBeenCalled();
    expect(addHistoryMock).not.toHaveBeenCalled();
  });

  it("打标 + 开关切到 false（不静默）：照常弹", async () => {
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "show_notification_window") throw new Error("no popup in jsdom");
      if (cmd === "get_setting" && args?.key === "silence_subagent_activity_flap") return "false";
      return null;
    });
    renderHook(() => useNotification());
    await flushInit();

    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "processing", true)] });
    });
    await flushInit();
    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "idle", true)] });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(GREEN_STABLE_MS + 100);
    });
    await flushInit();

    expect(sendNotificationMock).toHaveBeenCalledTimes(1);
  });

  it("未打标 + 默认静默：照常弹（静默门只吃打标跃迁）", async () => {
    renderHook(() => useNotification());
    await flushInit();

    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "processing", false)] });
    });
    await flushInit();
    act(() => {
      useSessionStore.setState({ sessions: [mkSession("s1", "idle", false)] });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(GREEN_STABLE_MS + 100);
    });
    await flushInit();

    expect(sendNotificationMock).toHaveBeenCalledTimes(1);
  });
});
