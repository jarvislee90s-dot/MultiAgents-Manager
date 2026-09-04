// tests/settings/toolManagement.test.tsx — review F7①②：工具管理批量保存 happy path
// 与未保存离开「放弃更改」路径（mock invoke，断言 IPC 入参与确认弹窗分类文案）
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
// WindowFrame/TitleBar 依赖 Tauri 窗口 API，jsdom 下全 mock（与 petSettings 同模式）
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

// tests/setup.ts 未初始化 i18n，显式引入并按默认英文断言
import i18n from "@/i18n";
import SettingsPage from "@/pages/settings";

void i18n;

const rows = [
  { toolId: "opencode", name: "OpenCode", enabled: true, installed: true, managed: true },
  { toolId: "codex", name: "Codex CLI", enabled: true, installed: true, managed: false },
];

beforeEach(() => {
  window.localStorage.clear();
  invokeMock.mockReset();
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === "get_tool_settings") return structuredClone(rows);
    if (cmd === "update_tool_settings") {
      return { restored: [], restoredMcps: [], rebuildFailed: [], skipped: [] };
    }
    if (cmd === "list_enabled_tools") return rows.map((r) => ({ id: r.toolId, label: r.name }));
    return [];
  });
});

const renderPage = () =>
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SettingsPage />
    </QueryClientProvider>
  );

// jsdom 未实现 matchMedia（ThemeProviders 需要），与 petSettings 同款 shim
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

async function gotoToolsAndToggleOpenCode() {
  renderPage();
  fireEvent.click(await screen.findByRole("button", { name: "Tool Management" }));
  const opencode = await screen.findByText("OpenCode");
  const row = opencode.closest("div[class*='justify-between']") as HTMLElement;
  fireEvent.click(row.querySelector('[role="switch"]') as HTMLElement);
  await screen.findByText("Save Settings");
}

describe("工具管理（review F7①②）", () => {
  it("F7① toggle→确认→apply：弹窗分类提示（还原/回溯）与 update_tool_settings 入参", async () => {
    await gotoToolsAndToggleOpenCode();
    // 打开保存确认弹窗
    fireEvent.click(await screen.findByText("Save Settings"));
    expect(await screen.findByText("Apply Changes")).toBeInTheDocument();
    // OpenCode（managed=true、关闭）→ 还原/回溯分类文案
    expect(
      screen.getByText(/Restore\/rollback: skill\/plugin links restored to real files/)
    ).toBeInTheDocument();
    // 确认应用 → IPC 入参断言
    fireEvent.click(screen.getByText("Apply"));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("update_tool_settings", {
        changes: [{ toolId: "opencode", enabled: false }],
      });
    });
  });

  it("F7② 未保存切分区 → 放弃更改：不持久化且开关回到已保存状态", async () => {
    await gotoToolsAndToggleOpenCode();
    // 切分区触发三选拦截
    fireEvent.click(screen.getByRole("button", { name: "Appearance" }));
    expect(await screen.findByText("Unsaved Changes")).toBeInTheDocument();
    // 放弃更改 → 不落盘
    fireEvent.click(screen.getByText("Discard Changes"));
    await waitFor(() =>
      expect(screen.queryByText("Unsaved Changes")).not.toBeInTheDocument()
    );
    expect(invokeMock).not.toHaveBeenCalledWith(
      "update_tool_settings",
      expect.anything()
    );
    // 回到工具分区：开关为已保存状态（checked）、dirty 已重置（无保存按钮）
    fireEvent.click(await screen.findByRole("button", { name: "Tool Management" }));
    const opencode = await screen.findByText("OpenCode");
    const row = opencode.closest("div[class*='justify-between']") as HTMLElement;
    expect(row.querySelector('[role="switch"]')).toHaveAttribute("data-state", "checked");
    expect(screen.queryByText("Save Settings")).not.toBeInTheDocument();
  });
});
