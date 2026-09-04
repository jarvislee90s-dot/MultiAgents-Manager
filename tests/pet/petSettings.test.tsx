// tests/pet/petSettings.test.tsx — 设置页桌宠分区控件与开关行为（spec §10.2/D8）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { loadVisible, loadConfig } from "@/components/pet/petConfig";
// tests/setup.ts 未初始化 i18n，测试断言需显式引入并固定英文
import i18n from "@/i18n";

// jsdom 未实现 matchMedia，ThemeProviders（WindowFrame 内置）需要它
window.matchMedia = ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: vi.fn(),
  removeListener: vi.fn(),
  addEventListener: vi.fn(),
  removeEventListener: vi.fn(),
  dispatchEvent: vi.fn(),
})) as unknown as typeof window.matchMedia;

// WindowFrame/TitleBar 依赖 Tauri 窗口 API，jsdom 下全 mock（与 foxbell-*.test.tsx 同模式）
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));

import SettingsPage from "@/pages/settings";

beforeAll(async () => {
  await i18n.changeLanguage("en");
});

describe("settings 桌宠分区", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockClear();
  });

  it("切到桌宠分区：三个控件齐备（开启/置顶/大小）", () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByText("Pet")); // i18n 默认 en
    expect(screen.getByText("Enable pet")).toBeTruthy();
    expect(screen.getByText("Always on top")).toBeTruthy();
    expect(screen.getByText("Size")).toBeTruthy();
  });

  it("开启开关：写 localStorage 并调用 set_pet_visible", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByText("Pet"));
    const toggles = screen.getAllByRole("switch");
    fireEvent.click(toggles[0]); // 分区内第一个 Switch = 开启桌宠
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_pet_visible", { visible: true }));
    expect(loadVisible()).toBe(true);
  });

  it("大小三档：点 Large 写 scale=1.25", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByText("Pet"));
    fireEvent.click(screen.getByText("Large"));
    await waitFor(() => expect(loadConfig().scale).toBe(1.25));
  });
});

describe("外部宠物三入口（spec §11）", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockClear();
  });

  it("渲染当前宠物行与切换按钮", async () => {
    render(<SettingsPage />);
    fireEvent.click(screen.getByText("Pet")); // i18n 固定 en：英文分支断言
    expect(await screen.findByText(/当前宠物|Current pet/)).toBeInTheDocument();
    const switchBtn = await screen.findByRole("button", { name: /切换宠物|Switch pet/ });
    fireEvent.click(switchBtn);
    expect(await screen.findByTestId("pet-switch-list")).toBeInTheDocument();
  });
});
