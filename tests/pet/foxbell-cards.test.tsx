// tests/pet/foxbell-cards.test.tsx — 卡片渲染 + 点击跳转 ack（msw/invoke mock 走 tests/msw）
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
    // 题头与看板一致：工具名 + 项目 + 会话名（问题 3）
    expect(card.textContent).toContain("Claude");
    expect(card.textContent).toContain("项目A");
    expect(card.textContent).toContain("标题A");
    expect(card.textContent).toContain("等待操作");
    fireEvent.click(card);
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("focus_session", expect.objectContaining({ pid: 42, sessionId: "s1" }))
    );
  });

  it("歧义候选浮层：点外/Esc 关闭且不 ack，点内不关闭（spec §11）", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "focus_session") {
        return Promise.resolve({
          type: "ambiguous",
          windows: [{ hwnd: 1, title: "win-a", process: "iTerm2" }, { hwnd: 2, title: "win-b", process: "iTerm2" }],
        });
      }
      return Promise.resolve(undefined);
    });
    render(<FoxbellPet />);
    fireEvent.click(await screen.findByTestId("pet-card-s1"));
    const overlay = await screen.findByTestId("pet-jump-candidates", undefined, { timeout: 3000 });
    await act(async () => {}); // 排空被动效应：确保浮层关闭监听已注册再触发事件（FIX-11）
    // 点浮层内部：不关闭
    fireEvent.pointerDown(overlay.firstChild as HTMLElement);
    expect(screen.getByTestId("pet-jump-candidates")).toBeTruthy();
    // 点外：关闭（不 ack，卡片保留）
    fireEvent.pointerDown(document.body);
    await waitFor(() => expect(screen.queryByTestId("pet-jump-candidates")).toBeNull(), { timeout: 3000 });
    expect(screen.getByTestId("pet-card-s1")).toBeTruthy();
    // 再次触发歧义 → Esc 关闭
    fireEvent.click(screen.getByTestId("pet-card-s1"));
    await screen.findByTestId("pet-jump-candidates", undefined, { timeout: 3000 });
    await act(async () => {}); // 排空被动效应（Esc 关闭段，FIX-11）
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByTestId("pet-jump-candidates")).toBeNull(), { timeout: 3000 });
    expect(screen.getByTestId("pet-card-s1")).toBeTruthy();
    vi.mocked(invoke).mockClear();
  });
});
