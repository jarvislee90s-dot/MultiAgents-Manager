// tests/pet/petCandidatesFail.test.tsx — P2-7：宠物歧义候选点选聚焦失败时
// 不得产生 unhandled rejection、不得误 ack（卡保留可重试），并给出失败提示。
// 修复前：sessionJumpFocusHwnd(...).then(...) 无 catch → unhandled rejection 且无提示
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Session } from "@/types/session";

interface Frame {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
}

const { frame, focusMock, focusHwndMock, toastMock } = vi.hoisted(() => ({
  frame: { current: null as null | Frame },
  focusMock: vi.fn(),
  focusHwndMock: vi.fn(),
  toastMock: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: vi.fn(), hide: vi.fn(), setAlwaysOnTop: vi.fn(), setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(), setSize: vi.fn(), outerPosition: vi.fn(), outerSize: vi.fn(),
    scaleFactor: vi.fn(), currentMonitor: vi.fn(),
  }),
}));
vi.mock("sonner", () => ({ toast: toastMock }));
vi.mock("@/lib/query/queries/sessions", () => ({
  useSessionsQuery: () => ({ data: frame.current }),
}));
vi.mock("@/hooks/useSessionJump", () => ({
  useSessionJump: () => ({ focus: focusMock, focusHwnd: focusHwndMock }),
}));

// tests/setup.ts 未初始化 i18n，显式引入（jumpFailed 文案断言走 key 调用即可）
import i18n from "@/i18n";
import { FoxbellPet } from "@/components/pet/FoxbellPet";

void i18n;

const mk = (id: string, status: Session["status"], over: Partial<Session> = {}): Session => ({
  id,
  agentType: "claude",
  projectName: "P",
  projectPath: "/p",
  title: null,
  gitBranch: null,
  githubUrl: null,
  status,
  lastMessage: "msg",
  lastMessageRole: null,
  lastActivityAt: "",
  pid: 1,
  cpuUsage: 0,
  activeSubagentCount: 0,
  form: "cli",
  jumpSupported: true,
  unread: false,
  ...over,
});

describe("宠物歧义候选点选失败（P2-7）", () => {
  it("focus_hwnd 拒绝 → 不 ack（卡保留）、toast 提示、候选层照常关闭", async () => {
    focusMock.mockResolvedValue([{ hwnd: 111, title: "候选窗", process: "wezterm" }]);
    focusHwndMock.mockRejectedValue(new Error("窗口聚焦被系统拒绝"));
    const f1: Frame = { sessions: [mk("s1", "thinking")], totalCount: 1, waitingCount: 0 };
    const f2: Frame = { sessions: [mk("s1", "idle", { unread: true })], totalCount: 1, waitingCount: 0 };
    frame.current = f1;
    const view = render(<FoxbellPet />);
    await screen.findByTestId("pet-card-s1");

    frame.current = f2; // 黄→绿差分：done+unread 卡点亮
    view.rerender(<FoxbellPet />);
    await screen.findByTestId("pet-card-s1");

    // 点卡片 → 歧义候选浮层（歧义分支不 ack，等点选后回标）
    fireEvent.click(screen.getByTestId("pet-card-s1"));
    const layer = await screen.findByTestId("pet-jump-candidates");
    expect(focusMock).toHaveBeenCalled();

    // 点候选 → focus_hwnd 失败：不产生 unhandled rejection，toast 报错
    fireEvent.click(layer.children[0]);
    await waitFor(() => expect(toastMock.error).toHaveBeenCalled());
    // 卡保留（未 ack）可重试；候选层已无条件关闭（spec W1 无论成败清除）
    expect(screen.getByTestId("pet-card-s1")).toBeTruthy();
    expect(screen.queryByTestId("pet-jump-candidates")).toBeNull();
  });
});
