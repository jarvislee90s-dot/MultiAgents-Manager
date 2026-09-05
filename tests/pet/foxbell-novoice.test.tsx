// tests/pet/foxbell-novoice.test.tsx — 无语音宠物的最低档行为（spec §5.1）：
// 显示、全部常规动画与交互照常，仅不出声不出字幕（spec §5.2）。
// P1-5：playVoice 的 hasVoice 早退曾把 playTransient 一并拦下，无语音宠物连动作动画都没有。
// 本文件全部使用假定时器（同 foxbell-events：RTL waitFor 在假定时器下会死锁）
import { render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: vi.fn(),
    hide: vi.fn(),
    setAlwaysOnTop: vi.fn(),
    setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(),
    setSize: vi.fn(),
    outerPosition: vi.fn(),
    outerSize: vi.fn(),
    scaleFactor: vi.fn(),
    currentMonitor: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

const noVoicePet = {
  id: "silent-pet",
  displayName: "Silent",
  spritesheetUrl: "/x/silent.webp",
  rows: 11 as const,
  hasVoice: false,
  hasSubtitle: false,
  voices: [],
  resolveVoiceUrl: () => "",
  dispose: vi.fn(),
};
vi.mock("@/components/pet/petRuntime", async (importOriginal) => {
  const orig = await importOriginal<typeof import("@/components/pet/petRuntime")>();
  return { ...orig, resolveActivePet: vi.fn(async () => noVoicePet) };
});

let data: unknown = undefined;
vi.mock("@/lib/query/queries/sessions", () => ({ useSessionsQuery: () => ({ data }) }));

import { FoxbellPet } from "@/components/pet/FoxbellPet";

const mk = (id: string, status: string) => ({
  id,
  agentType: "claude",
  projectName: "P",
  projectPath: "/",
  title: null,
  gitBranch: null,
  githubUrl: null,
  status,
  lastMessage: "m",
  lastMessageRole: null,
  lastActivityAt: "",
  pid: 1,
  cpuUsage: 0,
  activeSubagentCount: 0,
  form: "cli",
  jumpSupported: true,
});

describe("FoxbellPet 无语音宠物最低档（P1-5）", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    localStorage.setItem("mam-pet-visible", "1");
  });
  afterEach(() => vi.useRealTimers());

  it("任务完成：瞬时动作照播、不出声不出字幕（spec §5.1/§5.2）", async () => {
    data = { sessions: [mk("a", "thinking")], totalCount: 1, waitingCount: 0 };
    const { rerender } = render(<FoxbellPet />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    data = { sessions: [mk("a", "idle")], totalCount: 1, waitingCount: 0 };
    rerender(<FoxbellPet />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    // done 默认动作 jumping（行 4）：y = -4×208 —— 修复前 hasVoice 闸门把动作一起拦下
    expect(screen.getByTestId("pet-sprite").style.backgroundPosition).toContain("-832px");
    expect(screen.queryByTestId("pet-bubble")).toBeNull();
  });

  it("双击：动作照播（不出声不出字幕，spec §5.1）", async () => {
    render(<FoxbellPet />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(50);
    });
    const sprite = screen.getByTestId("pet-sprite");
    const { fireEvent } = await import("@testing-library/react");
    fireEvent.dblClick(sprite);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    // 双击默认动作 waving（行 3）：y = -3×208
    expect(sprite.style.backgroundPosition).toContain("-624px");
    expect(screen.queryByTestId("pet-bubble")).toBeNull();
  });
});
