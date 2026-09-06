// tests/notification/bellClear.test.tsx — #5b：铃铛面板双清理按钮交互。
// 清空全部（两步确认）与清理已读分别生效、持久化（localStorage 落盘）、未读角标同步
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, toastMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  toastMock: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("sonner", () => ({ toast: toastMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

import i18n from "@/i18n";
import { NotificationBell } from "@/components/notifications/NotificationBell";
import { addHistory, getHistory, getUnreadCount } from "@/lib/notificationHistory";
import type { HistoryEntry } from "@/lib/notificationHistory";

void i18n;

const KEY = "mam-notification-history";

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

const seed = (entries: HistoryEntry[]) => {
  localStorage.setItem(KEY, JSON.stringify(entries));
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue({ type: "focused" });
  toastMock.success.mockClear();
  localStorage.clear();
});

// 打开面板（打开即自动全读，spec 既有行为）
const openPanel = () => {
  render(<NotificationBell />);
  fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
};

describe("清空全部（#5b 两步确认）", () => {
  it("首次点击进入确认态不执行，二次点击清空并持久化", () => {
    seed([entry({ sessionId: "s1" }), entry({ sessionId: "s2", read: false })]);
    openPanel();

    // 首次点击：进入"确认清空？"待确认态，条目仍在
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAll")));
    expect(screen.getByText(i18n.t("notifications.clearAllConfirm"))).toBeInTheDocument();
    expect(screen.getAllByText(/项目A ·/)).toHaveLength(2);
    expect(getHistory()).toHaveLength(2);

    // 二次点击：真正清空，面板即时刷新为空态，localStorage 落盘为空数组
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAllConfirm")));
    expect(screen.queryByText(/项目A ·/)).not.toBeInTheDocument();
    expect(screen.getByText(i18n.t("notifications.historyEmpty"))).toBeInTheDocument();
    expect(getHistory()).toEqual([]);
    expect(JSON.parse(localStorage.getItem(KEY) ?? "[]")).toEqual([]);
    expect(toastMock.success).toHaveBeenCalledWith(i18n.t("notifications.clearedToast"));
  });

  it("确认态在面板关闭后还原（下次打开不残留）", () => {
    seed([entry()]);
    openPanel();
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAll")));
    expect(screen.getByText(i18n.t("notifications.clearAllConfirm"))).toBeInTheDocument();

    // 关闭再打开：按钮还原为"清空全部"，且未误清空
    fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
    fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
    expect(screen.getByText(i18n.t("notifications.clearAll"))).toBeInTheDocument();
    expect(getHistory()).toHaveLength(1);
  });

  it("确认态 3 秒超时自动还原（不执行清空）", () => {
    vi.useFakeTimers();
    try {
      seed([entry()]);
      openPanel();
      fireEvent.click(screen.getByText(i18n.t("notifications.clearAll")));
      expect(screen.getByText(i18n.t("notifications.clearAllConfirm"))).toBeInTheDocument();

      act(() => {
        vi.advanceTimersByTime(3001);
      });
      expect(screen.getByText(i18n.t("notifications.clearAll"))).toBeInTheDocument();
      expect(getHistory()).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("清理已读（#5b）", () => {
  it("面板打开（自动全读）后点击清理已读 → 全部清空并持久化", () => {
    seed([entry({ sessionId: "s1" }), entry({ sessionId: "s2", read: false })]);
    openPanel();

    fireEvent.click(screen.getByText(i18n.t("notifications.clearRead")));
    expect(screen.queryByText(/项目A ·/)).not.toBeInTheDocument();
    expect(screen.getByText(i18n.t("notifications.historyEmpty"))).toBeInTheDocument();
    expect(getHistory()).toEqual([]);
    expect(JSON.parse(localStorage.getItem(KEY) ?? "[]")).toEqual([]);
    expect(toastMock.success).toHaveBeenCalledWith(i18n.t("notifications.readClearedToast"));
  });

  it("确认态下点击清理已读 → 确认态消失、不误执行清空全部", () => {
    seed([entry({ sessionId: "s1" }), entry({ sessionId: "s2" })]);
    openPanel();

    // 武装"清空全部"确认态
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAll")));
    expect(screen.getByText(i18n.t("notifications.clearAllConfirm"))).toBeInTheDocument();

    // 确认态下点"清理已读"：确认态消失（按钮还原为"清空全部"），
    // 且走的是 clearRead 而非 clearHistory——面板内已全读故全部清空，
    // 但 toast 必须是 readClearedToast 而非 clearedToast（证明未误触发清空全部）
    fireEvent.click(screen.getByText(i18n.t("notifications.clearRead")));
    expect(screen.queryByText(i18n.t("notifications.clearAllConfirm"))).not.toBeInTheDocument();
    expect(screen.getByText(i18n.t("notifications.clearAll"))).toBeInTheDocument();
    expect(getHistory()).toEqual([]);
    expect(toastMock.success).toHaveBeenCalledWith(i18n.t("notifications.readClearedToast"));
    expect(toastMock.success).not.toHaveBeenCalledWith(i18n.t("notifications.clearedToast"));
  });

  it("面板打开自动全读（角标归零）后清理已读 → 全部清空", () => {
    // 面板关闭时补一条未读（模拟新通知到达），角标显示 1
    seed([entry({ sessionId: "s1" })]);
    const { unmount } = render(<NotificationBell />);
    act(() => {
      addHistory({
        agentType: "workbuddy",
        projectName: "项目B",
        status: "idle",
        lastMessage: "new",
        pid: 0,
        sessionId: "s2",
        at: Date.now(),
      });
    });
    expect(getUnreadCount()).toBe(1);
    expect(screen.getByText("1")).toBeInTheDocument();
    unmount();

    // 重新挂载并打开面板：打开即自动全读（既有行为），角标消失
    openPanel();
    expect(screen.queryByText("1")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText(i18n.t("notifications.clearRead")));
    expect(getHistory()).toEqual([]);
    expect(getUnreadCount()).toBe(0);
  });
});

describe("未读角标同步（#5b 验收）", () => {
  it("清空全部后角标消失；新通知仍可继续累积", () => {
    seed([entry({ sessionId: "s1", read: false }), entry({ sessionId: "s2", read: false })]);
    const { unmount } = render(<NotificationBell />);
    // 打开前：角标显示 2
    expect(screen.getByText("2")).toBeInTheDocument();

    // 打开面板（自动全读 → 角标消失）→ 清空全部
    fireEvent.click(screen.getByTitle(i18n.t("notifications.historyTitle")));
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAll")));
    fireEvent.click(screen.getByText(i18n.t("notifications.clearAllConfirm")));
    expect(screen.queryByText("2")).not.toBeInTheDocument();
    unmount();

    // 清空后新通知照常入历史并恢复角标
    render(<NotificationBell />);
    act(() => {
      addHistory({
        agentType: "workbuddy",
        projectName: "项目C",
        status: "idle",
        lastMessage: "after",
        pid: 0,
        sessionId: "s3",
        at: Date.now(),
      });
    });
    expect(getHistory()).toHaveLength(1);
    expect(screen.getByText("1")).toBeInTheDocument();
  });
});
