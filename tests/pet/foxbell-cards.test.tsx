// tests/pet/foxbell-cards.test.tsx — 卡片渲染 + 点击跳转 ack（msw/invoke mock 走 tests/msw）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: vi.fn(), hide: vi.fn(), setAlwaysOnTop: vi.fn(), setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(), setSize: vi.fn(), outerPosition: vi.fn(), outerSize: vi.fn(),
    scaleFactor: vi.fn(), currentMonitor: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));

const sessionsData = {
  sessions: [{
    id: "s1", agentType: "claude", projectName: "项目A", projectPath: "/a", title: "标题A",
    gitBranch: null, githubUrl: null, status: "waiting", lastMessage: "等你确认",
    lastMessageRole: null, lastActivityAt: "", pid: 42, cpuUsage: 0,
    activeSubagentCount: 0, form: "cli", jumpSupported: true,
  }],
  totalCount: 1, waitingCount: 1,
};
vi.mock("@/lib/query/queries/sessions", () => ({
  useSessionsQuery: () => ({ data: sessionsData }),
}));

import { invoke } from "@tauri-apps/api/core";
import { FoxbellPet } from "@/components/pet/FoxbellPet";

describe("FoxbellPet 卡片", () => {
  it("waiting 会话渲染红卡；点击调用 focus_session（spec C1）", async () => {
    render(<FoxbellPet />);
    const card = await screen.findByTestId("pet-card-s1");
    expect(card.textContent).toContain("标题A");
    expect(card.textContent).toContain("等待操作");
    fireEvent.click(card);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("focus_session", expect.objectContaining({ pid: 42, sessionId: "s1" }))
    );
  });
});
