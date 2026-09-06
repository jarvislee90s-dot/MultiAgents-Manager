// 主题 store 回归测试 — issue #3：主题 SSOT 上移为 DB settings KV + 全局事件，
// DB 拉取为模块级初始化（覆盖所有窗口，包括不消费 useTheme 的窗口）。
// 原实现以 localStorage 为事实源、跨窗口依赖 storage 事件；Tauri 各窗口是独立
// WebView，storage 事件不互通 → 设置窗口恒为浅色。新实现：setTheme 写 DB
// （set_theme 命令）并广播 mam-theme-changed；每个窗口模块加载时应用首帧缓存 +
// initTheme() 从 DB 拉取校正 + 订阅全局事件实时跟随。
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { invokeMock, themeHandlers } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  // 模块级 listen 注册发生在模块首次导入时（早于任何测试），且 resetModules 重导入
  // 会再次注册——用持久数组收集全部 handler，测试按需调用，不受 beforeEach 清理影响
  themeHandlers: [] as ((e: { payload: { theme: string } }) => void)[],
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// 注意：theme-provider 在模块加载时即调用 listen 注册全局事件监听（早于任何组件挂载），
// 因此实现必须直接写在 mock 工厂里（工厂随模块首次导入执行），不能在 beforeEach 里
// 用 mockImplementation 补——那会晚于模块求值，监听注册不到
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, handler: (e: { payload: { theme: string } }) => void) => {
    themeHandlers.push(handler);
    return () => {};
  }),
}));

import { useTheme } from "@/components/common/theme-provider";

function Harness() {
  const { theme, setTheme } = useTheme();
  return (
    <div>
      <span data-testid="current">{theme}</span>
      <button data-testid="btn-dark" onClick={() => setTheme("dark")} />
      <button data-testid="btn-light" onClick={() => setTheme("light")} />
    </div>
  );
}

describe("theme store（DB SSOT + 模块级初始化 + 全局事件）", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark", "light");
    invokeMock.mockReset();
    // 默认：DB 无主题记录（模块级 initTheme 返回 null，保持缓存值）
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "get_setting") return args?.key === "ui_theme" ? null : null;
      return undefined;
    });
  });

  it("setTheme：html 类 + localStorage 缓存 + 写 DB 广播 + 同窗口多订阅组件同步", async () => {
    render(
      <>
        <Harness />
        <Harness />
      </>
    );
    const btns = screen.getAllByTestId("btn-dark");
    fireEvent.click(btns[0]);
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("tauri-ui-theme")).toBe("dark");
    // 写 DB 事实源（set_theme 命令）
    expect(invokeMock).toHaveBeenCalledWith("set_theme", { theme: "dark" });
    // 两个订阅组件都应收到同窗口事件并更新
    await waitFor(() => {
      expect(screen.getAllByTestId("current").map((c) => c.textContent)).toEqual(["dark", "dark"]);
    });
  });

  it("切回浅色同样生效", () => {
    render(<Harness />);
    fireEvent.click(screen.getByTestId("btn-dark"));
    fireEvent.click(screen.getByTestId("btn-light"));
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(localStorage.getItem("tauri-ui-theme")).toBe("light");
    expect(invokeMock).toHaveBeenCalledWith("set_theme", { theme: "light" });
  });

  it("模块加载即应用缓存主题（首帧防闪白，不依赖组件挂载）", async () => {
    localStorage.setItem("tauri-ui-theme", "light");
    document.documentElement.classList.remove("light", "dark");
    document.documentElement.classList.add("dark");
    vi.resetModules();
    await import("@/components/common/theme-provider");
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("模块级初始化：窗口加载时从 DB 事实源拉取主题校正（缓存过期场景）", async () => {
    // 缓存为浅色，但 DB 事实源已是 dark（其他窗口切换过）；新窗口加载（新模块实例）
    // 时 initTheme 应从 DB 拉取并校正，且不依赖任何 useTheme 消费者挂载
    localStorage.setItem("tauri-ui-theme", "light");
    document.documentElement.classList.remove("light", "dark");
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      return cmd === "get_setting" && args?.key === "ui_theme" ? "dark" : null;
    });
    vi.resetModules();
    await import("@/components/common/theme-provider");
    await waitFor(() => {
      expect(document.documentElement.classList.contains("dark")).toBe(true);
    });
    expect(document.documentElement.classList.contains("light")).toBe(false);
    expect(localStorage.getItem("tauri-ui-theme")).toBe("dark");
  });

  it("全局事件（其他窗口切换主题）实时跟随", async () => {
    render(<Harness />);
    expect(themeHandlers.length).toBeGreaterThan(0);
    // 模拟 Rust set_theme 广播：另一窗口把主题切为 dark（事件回调触发 React 状态更新，
    // 需包在 act 内，与 toolsChangedSync 测试同模式）
    await act(async () => {
      themeHandlers[themeHandlers.length - 1]?.({ payload: { theme: "dark" } });
    });
    expect(screen.getByTestId("current").textContent).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("tauri-ui-theme")).toBe("dark");
  });

  it("窄竞态守卫：用户已手动切换后，迟到的 DB 拉取值不覆盖", async () => {
    // 窗口加载时 DB 拉取在途（延时返回旧值 dark），期间用户手动切到 light——
    // 迟到的旧值不得覆盖用户刚做的选择（dirtySinceLoad 守卫）
    localStorage.setItem("tauri-ui-theme", "system");
    document.documentElement.classList.remove("light", "dark");
    let resolveDb!: (v: string | null) => void;
    invokeMock.mockImplementation(async (cmd: string, args?: { key?: string }) => {
      if (cmd === "get_setting" && args?.key === "ui_theme") {
        return new Promise<string | null>((r) => {
          resolveDb = r;
        });
      }
      return undefined;
    });
    vi.resetModules();
    const mod = await import("@/components/common/theme-provider");
    function FreshHarness() {
      const { theme, setTheme } = mod.useTheme();
      return (
        <div>
          <span data-testid="fresh-current">{theme}</span>
          <button data-testid="fresh-btn-light" onClick={() => setTheme("light")} />
        </div>
      );
    }
    render(<FreshHarness />);
    fireEvent.click(screen.getByTestId("fresh-btn-light"));
    expect(screen.getByTestId("fresh-current").textContent).toBe("light");
    // 迟到的 DB 旧值就位
    await act(async () => {
      resolveDb("dark");
    });
    expect(screen.getByTestId("fresh-current").textContent).toBe("light");
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
