// tests/pet/foxbell-interactions.test.tsx — 指针交互与语音触发（窗口 API mock）
import { fireEvent, render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: vi.fn(),
    hide: vi.fn(),
    setAlwaysOnTop: vi.fn(),
    setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(),
    setSize: vi.fn(),
    outerPosition: vi.fn(async () => ({ x: 0, y: 0 })),
    outerSize: vi.fn(async () => ({ width: 680, height: 520 })),
    scaleFactor: vi.fn(async () => 1),
    currentMonitor: vi.fn(async () => ({
      workArea: { x: 0, y: 0, width: 1440, height: 900 },
      scaleFactor: 1,
    })),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/lib/query/queries/sessions", () => ({ useSessionsQuery: () => ({ data: undefined }) }));
// manifest 返回 1 条 general 语音：空 manifest 会使 VoicePlayer.pick 返回 null（spec E5），
// 字幕回调无从触发，因此需至少含一条 general 条目（jsdom 有 Audio，走 els 播放路径，时长兜底 2.5s）
const fetchMock = vi.fn(async () => ({
  json: async () => [{ group: "general", name: "你好呀", file: "general/hello.m4a" }],
}));
vi.stubGlobal("fetch", fetchMock);

import { FoxbellPet } from "@/components/pet/FoxbellPet";

describe("FoxbellPet 指针交互", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
    // tests/setup.ts 的 beforeAll(server.listen) 会用 MSW 拦截器重写 globalThis.fetch，
    // 且其运行时机晚于本文件的模块级 stubGlobal —— 这里在每个用例前重打桩保证 fetchMock 生效
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.useRealTimers());

  it("单击：挥手不出声（spec A1）", async () => {
    render(<FoxbellPet />);
    const sprite = screen.getByTestId("pet-sprite");
    fireEvent.pointerDown(sprite, { pointerId: 1, button: 0, clientX: 100, clientY: 100 });
    fireEvent.pointerUp(sprite, { pointerId: 1, clientX: 100, clientY: 100 });
    // waving 行 3：backgroundPosition y = -3×208×scale
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(sprite.style.backgroundPosition).toContain("-624px");
  });

  it("双击：说话（general 语音 + 字幕气泡，spec A2）", async () => {
    localStorage.setItem("mam-pet-config", JSON.stringify({ muted: false, talkative: true }));
    render(<FoxbellPet />);
    const sprite = screen.getByTestId("pet-sprite");
    // manifest 拉取异步完成：先推进微任务让 voiceRef 就绪，再双击
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    fireEvent.dblClick(sprite);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20);
    });
    const bubble = screen.queryByTestId("pet-bubble");
    expect(bubble).not.toBeNull(); // 测试环境 Audio 无声仍显示字幕（时长兜底 2.5s）
  });

  it("拖拽方向动画：上拖跳跃（spec A3）", async () => {
    render(<FoxbellPet />);
    const sprite = screen.getByTestId("pet-sprite");
    fireEvent.pointerDown(sprite, { pointerId: 1, button: 0, clientX: 100, clientY: 300 });
    // beginDrag 异步读取窗口几何：先推进微任务让 dragRef 就绪
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    fireEvent.pointerMove(sprite, { pointerId: 1, clientX: 100, clientY: 300 }); // 建立采样基线（movedY=0）
    fireEvent.pointerMove(sprite, { pointerId: 1, clientX: 100, clientY: 250 }); // dy=-50, movedY=-50
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    // jumping 行 4：y = -4×208
    expect(sprite.style.backgroundPosition).toContain("-832px");
  });
});
