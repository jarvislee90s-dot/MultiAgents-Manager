// 主题 store 回归测试 — 修复：设置页深浅色切换失效（原 Context 方案 no-op）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

describe("theme store（无 Context，事件驱动）", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.classList.remove("dark", "light");
  });

  it("setTheme：html 类 + localStorage + 同窗口多订阅组件同步", async () => {
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
  });

  it("模块加载即应用持久化主题（不依赖组件挂载）", async () => {
    localStorage.setItem("tauri-ui-theme", "light");
    document.documentElement.classList.remove("light", "dark");
    document.documentElement.classList.add("dark");
    vi.resetModules();
    await import("@/components/common/theme-provider");
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});
