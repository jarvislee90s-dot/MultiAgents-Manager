// tests/pet/foxbell-render.test.tsx — 渲染骨架 + 帧步进 + 显隐应用（窗口 API 全 mock）
import { render, screen, act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const showMock = vi.fn();
const setAlwaysOnTopMock = vi.fn();
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    show: showMock,
    hide: vi.fn(),
    setAlwaysOnTop: setAlwaysOnTopMock,
    setIgnoreCursorEvents: vi.fn(),
    setPosition: vi.fn(),
    setSize: vi.fn(),
    outerPosition: vi.fn(),
    outerSize: vi.fn(),
    scaleFactor: vi.fn(),
    currentMonitor: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/lib/query/queries/sessions", () => ({ useSessionsQuery: () => ({ data: undefined }) }));

import { FoxbellPet } from "@/components/pet/FoxbellPet";

describe("FoxbellPet 骨架", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    localStorage.clear();
  });
  afterEach(() => vi.useRealTimers());

  it("渲染精灵：图集尺寸随 scale 等比例（spec D15）", () => {
    localStorage.setItem("mam-pet-config", JSON.stringify({ scale: 1.25 }));
    render(<FoxbellPet />);
    const sprite = screen.getByTestId("pet-sprite");
    expect(sprite.style.width).toBe("240px"); // 192×1.25
    expect(sprite.style.height).toBe("260px"); // 208×1.25
    expect(sprite.style.backgroundSize).toBe("1920px 2860px");
  });

  it("idle 帧步进：前进到第 2 帧（spec F1）", async () => {
    render(<FoxbellPet />);
    const sprite = screen.getByTestId("pet-sprite");
    const before = sprite.style.backgroundPosition;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(280); // 第 1 帧时长
    });
    expect(sprite.style.backgroundPosition).not.toBe(before);
  });
});
