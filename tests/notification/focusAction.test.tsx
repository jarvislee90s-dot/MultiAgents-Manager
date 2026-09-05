// tests/notification/focusAction.test.tsx — P2-5：系统通知"查看会话"点击必须
// 透传 form 且不再吞掉 pid=0 的 App 形态未读卡。修复前双重死路：
// ① `if (pid > 0)` 直接丢弃 pid=0 的 App 未读卡点击；② invoke 不传 form，
// Windows 深链分支（form == "app"）永不可达 → 点击后无任何反应
import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

type ActionHandler = (notification: {
  actionTypeId: string;
  extra?: Record<string, unknown>;
}) => Promise<void>;

const { invokeMock, onActionRef } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  onActionRef: { current: null as null | ActionHandler },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(async () => true),
  requestPermission: vi.fn(async () => "granted"),
  sendNotification: vi.fn(),
  registerActionTypes: vi.fn(async () => {}),
  onAction: vi.fn(async (handler: ActionHandler) => {
    onActionRef.current = handler;
  }),
}));

import { useNotification } from "@/hooks/useNotification";

function Probe() {
  useNotification();
  return null;
}

describe("系统通知「查看会话」点击（P2-5）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    // 通知开关读取（get_setting）返回 null → 视为开启
    invokeMock.mockResolvedValue(null);
    onActionRef.current = null;
  });

  it("pid=0 的 App 形态未读卡点击触发 focus_session 且透传 form", async () => {
    render(<Probe />);
    await waitFor(() => expect(onActionRef.current).toBeTruthy());
    await onActionRef.current?.({
      actionTypeId: "focus-session",
      extra: {
        pid: 0,
        form: "app",
        sessionId: "s1",
        agentType: "workbuddy",
        projectName: "P",
        lastMessage: "m",
      },
    });
    expect(invokeMock).toHaveBeenCalledWith(
      "focus_session",
      expect.objectContaining({
        pid: 0,
        form: "app",
        sessionId: "s1",
        agentType: "workbuddy",
      })
    );
  });

  it("pid>0 的 CLI 会话点击照常透传（含 form）", async () => {
    render(<Probe />);
    await waitFor(() => expect(onActionRef.current).toBeTruthy());
    await onActionRef.current?.({
      actionTypeId: "focus-session",
      extra: { pid: 42, form: "cli", sessionId: "s2", agentType: "claude" },
    });
    expect(invokeMock).toHaveBeenCalledWith(
      "focus_session",
      expect.objectContaining({ pid: 42, form: "cli", sessionId: "s2" })
    );
  });

  it("非 focus-session action 的通知不触发跳转", async () => {
    render(<Probe />);
    await waitFor(() => expect(onActionRef.current).toBeTruthy());
    await onActionRef.current?.({ actionTypeId: "other", extra: { pid: 1 } });
    expect(invokeMock).not.toHaveBeenCalledWith("focus_session", expect.anything());
  });
});
