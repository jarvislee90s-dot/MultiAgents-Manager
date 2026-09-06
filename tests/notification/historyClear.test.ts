// tests/notification/historyClear.test.ts — #5b：通知历史双清理按钮的存储层行为。
// clearHistory 清空全部（含未读）；clearRead 仅删已读（未读保留、角标不变）；
// 两者均照 markAllRead 模式落盘 + 派发 mam-history-updated
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  addHistory,
  clearHistory,
  clearRead,
  getHistory,
  getUnreadCount,
  markAllRead,
} from "@/lib/notificationHistory";

const KEY = "mam-notification-history";

const seed = (entries: Array<{ read: boolean; id: string }>) => {
  localStorage.setItem(
    KEY,
    JSON.stringify(
      entries.map((e) => ({
        agentType: "workbuddy",
        projectName: "项目A",
        status: "idle",
        lastMessage: "done",
        pid: 0,
        sessionId: e.id,
        at: Date.now(),
        ...e,
      }))
    )
  );
};

beforeEach(() => {
  localStorage.clear();
});

describe("clearHistory（#5b 清空全部）", () => {
  it("清空全部条目（含未读），落盘为空数组", () => {
    seed([{ read: true, id: "s1" }, { read: false, id: "s2" }]);
    clearHistory();
    expect(getHistory()).toEqual([]);
    expect(JSON.parse(localStorage.getItem(KEY) ?? "[]")).toEqual([]);
  });

  it("派发 mam-history-updated 事件（照 markAllRead 模式）", () => {
    const listener = vi.fn();
    window.addEventListener("mam-history-updated", listener);
    seed([{ read: true, id: "s1" }]);
    clearHistory();
    expect(listener).toHaveBeenCalledTimes(1);
    window.removeEventListener("mam-history-updated", listener);
  });

  it("清空后未读角标计数归零", () => {
    seed([{ read: false, id: "s1" }, { read: false, id: "s2" }]);
    expect(getUnreadCount()).toBe(2);
    clearHistory();
    expect(getUnreadCount()).toBe(0);
  });
});

describe("clearRead（#5b 仅清理已读）", () => {
  it("只删已读条目，未读保留", () => {
    seed([{ read: true, id: "s1" }, { read: false, id: "s2" }, { read: true, id: "s3" }]);
    clearRead();
    const rest = getHistory();
    expect(rest).toHaveLength(1);
    expect(rest[0].sessionId).toBe("s2");
    expect(rest[0].read).toBe(false);
  });

  it("派发 mam-history-updated 事件（照 markAllRead 模式）", () => {
    const listener = vi.fn();
    window.addEventListener("mam-history-updated", listener);
    seed([{ read: true, id: "s1" }]);
    clearRead();
    expect(listener).toHaveBeenCalledTimes(1);
    window.removeEventListener("mam-history-updated", listener);
  });

  it("未读角标计数不变（未读条目未被触碰）", () => {
    seed([{ read: true, id: "s1" }, { read: false, id: "s2" }]);
    expect(getUnreadCount()).toBe(1);
    clearRead();
    expect(getUnreadCount()).toBe(1);
  });

  it("全部已读时清空后为空数组", () => {
    seed([{ read: true, id: "s1" }, { read: true, id: "s2" }]);
    clearRead();
    expect(getHistory()).toEqual([]);
  });
});

describe("与既有 API 协同（#5b 回归）", () => {
  it("markAllRead 后 clearRead 可清空全部（面板打开即全读的路径）", () => {
    seed([{ read: false, id: "s1" }, { read: false, id: "s2" }]);
    markAllRead();
    clearRead();
    expect(getHistory()).toEqual([]);
  });

  it("addHistory 新条目不受清理影响（清空后仍可继续累积）", () => {
    seed([{ read: true, id: "s1" }]);
    clearHistory();
    addHistory({
      agentType: "workbuddy",
      projectName: "项目B",
      status: "idle",
      lastMessage: "new",
      pid: 0,
      sessionId: "s2",
      at: Date.now(),
    });
    expect(getHistory()).toHaveLength(1);
    expect(getHistory()[0].sessionId).toBe("s2");
  });
});
