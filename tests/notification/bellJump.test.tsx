// tests/notification/bellJump.test.tsx — review 补充：铃铛历史面板跳转必须透传 form。
// HistoryEntry 已带 form（useNotification.addHistory 写入）但 jumpTo 未传 → Windows 上
// App 形态会话进不了 focus_session 深链第一顺位（与 issue #34 第 5 条同族、换入口）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("sonner", () => ({ toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import i18n from "@/i18n";
import { NotificationBell } from "@/components/notifications/NotificationBell";
import type { HistoryEntry } from "@/lib/notificationHistory";

void i18n;

const entry = (over: Partial<HistoryEntry> = {}): HistoryEntry => ({
  agentType: "workbuddy",
  form: "app",
  projectName: "项目A",
  status: "idle",
  lastMessage: "done",
  pid: 0,
  sessionId: "s1",
  at: Date.now(),
  read: true,
  ...over,
});

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({ type: "focused" });
  localStorage.clear();
});

describe("铃铛历史跳转透传 form（review）", () => {
  it("App 形态历史条目点击 → focus_session 收到 form=app（深链第一顺位可达）", async () => {
    localStorage.setItem("mam-notification-history", JSON.stringify([entry()]));
    render(<NotificationBell />);
    fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
    fireEvent.click(await screen.findByText(/项目A ·/));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "focus_session",
        expect.objectContaining({
          pid: 0,
          form: "app",
          sessionId: "s1",
          agentType: "workbuddy",
        })
      )
    );
  });

  it("旧条目无 form（历史遗留数据）→ 跳转不回归（form 仅缺省）", async () => {
    localStorage.setItem(
      "mam-notification-history",
      JSON.stringify([entry({ form: undefined, pid: 42 })])
    );
    render(<NotificationBell />);
    fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
    fireEvent.click(await screen.findByText(/项目A ·/));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith(
        "focus_session",
        expect.objectContaining({ pid: 42 })
      )
    );
  });
});
