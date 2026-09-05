// tests/pet/sessionReadSync.test.tsx — T1：跨窗口已读同步。看板点掉未读卡（X/跳转）
// 后后端广播 "session-read"，宠物窗口凭此事件对状态机做已读置位 → 头顶卡立即消隐
// （此前宠物感知不到别处的已读，头顶卡晚 ~60s 才消）。
// 注意：useSessionsQuery 的 mock 必须返回稳定引用（真实 react-query 仅在 refetch
// 返回新数据时才变引用）。若每次渲染返回新对象，FoxbellPet 的 [data] effect 会
// setCards → 无限重渲染 → 测试进程挂死（已实测）。
import { act, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Session } from "@/types/session";

interface Frame {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
}

const { listenMock, frame } = vi.hoisted(() => ({
  listenMock: vi.fn(),
  // 稳定容器：mock 每次返回 frame.current 引用；测试在两帧预建数据间切换
  frame: { current: null as Frame | null },
}));

listenMock.mockImplementation(async (_event: string, handler: (e: unknown) => void) => {
  return () => {};
});

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
  emit: vi.fn(async () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: vi.fn(), hide: vi.fn(), setAlwaysOnTop: vi.fn(), setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(), setSize: vi.fn(), outerPosition: vi.fn(), outerSize: vi.fn(),
    scaleFactor: vi.fn(), currentMonitor: vi.fn(),
  }),
}));
vi.mock("@/lib/query/queries/sessions", () => ({
  useSessionsQuery: () => ({ data: frame.current }),
}));

import { FoxbellPet } from "@/components/pet/FoxbellPet";

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
  ...over,
});

describe("session-read 跨窗口已读同步（T1）", () => {
  it("session-read 事件 → 宠物已读置位 → 头顶卡消隐", async () => {
    // 第一帧 thinking（黄卡 running）；第二帧 idle → 差分产出 done+unread 绿卡
    const f1: Frame = { sessions: [mk("s1", "thinking")], totalCount: 1, waitingCount: 0 };
    const f2: Frame = { sessions: [mk("s1", "idle")], totalCount: 1, waitingCount: 0 };
    frame.current = f1;
    const view = render(<FoxbellPet />);
    await screen.findByTestId("pet-card-s1");

    frame.current = f2; // 新引用（预建常量）→ 模拟 refetch 返回新数据
    view.rerender(<FoxbellPet />);
    const card = await screen.findByTestId("pet-card-s1");
    expect(card.textContent).toContain("已完成");

    // 看板点掉未读卡 → 后端广播 session-read → 宠物已读置位 → 卡片立即消失。
    // listen 的 handler 从 mock 调用记录中取（setupSessionReadListener 注册的回调）
    await waitFor(() => expect(listenMock).toHaveBeenCalled());
    const handler = listenMock.mock.calls[0][1] as (e: unknown) => void;
    await act(async () => {
      handler({ payload: { agentType: "claude", sessionId: "s1" } });
    });
    await waitFor(() => expect(screen.queryByTestId("pet-card-s1")).toBeNull());
  });
});
