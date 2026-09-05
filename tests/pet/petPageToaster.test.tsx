// tests/pet/petPageToaster.test.tsx — P2-7：宠物窗口页必须挂载 Toaster。
// 此前 useSessionJump 的 M3 兜底提示（appFallbackHint）与跳转失败提示在宠物窗口
// 因无通知容器而静默丢弃——提示"发出去了"但用户永远看不到
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    setAlwaysOnTop: vi.fn(async () => {}),
    show: vi.fn(async () => {}),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@/components/pet/petConfig", () => ({
  loadConfig: () => ({ alwaysOnTop: true }),
  loadVisible: () => false,
  saveVisible: vi.fn(),
  subscribeConfig: () => () => {},
}));
vi.mock("@/components/pet/FoxbellPet", () => ({ FoxbellPet: () => <div /> }));
vi.mock("@/components/ui/sonner", () => ({
  Toaster: (props: { position?: string }) => (
    <div data-testid="pet-toaster" data-position={props.position} />
  ),
}));

import PetPage from "@/pages/pet";

describe("宠物窗口通知容器（P2-7）", () => {
  it("pet 页挂载 Toaster 且 position=top-center（避免被桌宠本体遮挡）", () => {
    render(<PetPage />);
    expect(screen.getByTestId("pet-toaster").dataset.position).toBe("top-center");
  });
});
