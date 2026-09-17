// tests/settings/settingsWindowSize.test.tsx — M5 A6：线稿 v5 定稿「窗口默认 880×640」。
// 设置窗口创建参数在 MainTitleBar.handleOpenSettings（此前 600×500），本测试锁定
// createWindow 以 (settings, width 880, height 640) 调用，防回退。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { createWindowMock } = vi.hoisted(() => ({ createWindowMock: vi.fn() }));
// 只 mock 窗口创建入口（@/lib/window 的 createWindow）；标题栏其余行为不波及
vi.mock("@/lib/window", () => ({ createWindow: createWindowMock }));
// theme-provider 模块级 listen(@tauri-apps/api/event) 在 jsdom 无 Tauri 内核，须 mock
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => {}),
}));
// TitleBar 的最大化监听在 jsdom 无 __TAURI_INTERNALS__（getCurrentWebviewWindow 即抛），桩掉
vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: () => ({
    isMaximized: async () => false,
    onResized: async () => () => {},
  }),
}));

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言（jsdom navigator.language=en）
import i18n from "@/i18n";
import { MainTitleBar } from "@/components/common/main-title-bar";

void i18n;

describe("设置窗口创建参数（M5 A6：880×640）", () => {
  beforeEach(() => {
    createWindowMock.mockReset();
  });

  it("点标题栏设置按钮 → createWindow(settings, { width: 880, height: 640 })", async () => {
    render(<MainTitleBar />);
    fireEvent.click(screen.getByRole("button", { name: /settings/i }));
    await waitFor(() => expect(createWindowMock).toHaveBeenCalledTimes(1));
    const [label, options] = createWindowMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(label).toBe("settings");
    expect(options.width).toBe(880);
    expect(options.height).toBe(640);
  });

  it("其余子窗口不受波及：关于窗口仍为 500×400", async () => {
    render(<MainTitleBar />);
    fireEvent.click(screen.getByRole("button", { name: /about/i }));
    await waitFor(() => expect(createWindowMock).toHaveBeenCalledTimes(1));
    const [label, options] = createWindowMock.mock.calls[0] as [string, Record<string, unknown>];
    expect(label).toBe("about");
    expect(options.width).toBe(500);
    expect(options.height).toBe(400);
  });
});
