// tests/toolsChangedSync.test.tsx — N2：设置窗口保存工具勾选后，主窗口的 react-query
// 缓存必须经 "tools-changed" 事件失效并 refetch。设置窗口是独立 WebView，与主窗口
// 各持 QueryClient，invalidateQueries 不跨窗口——这是用户实测「资源分布需切页才刷新」
// 的根因；仅有 remount 时的 stale refetch 侥幸生效
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

const { invokeMock, listenMock, eventHandlerRef } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  eventHandlerRef: { current: null as null | (() => void) },
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

listenMock.mockImplementation(async (_event: string, handler: () => void) => {
  eventHandlerRef.current = handler;
  return () => {};
});

import { setupToolsChangedListener } from "@/lib/query/toolsChangedSync";
import { useEnabledToolsQuery } from "@/lib/query/queries/tools";

function Probe() {
  const { data = [] } = useEnabledToolsQuery();
  return <div data-testid="tools">{data.map((t) => t.label).join(",") || "(empty)"}</div>;
}

describe("tools-changed 跨窗口缓存失效（N2）", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    eventHandlerRef.current = null;
  });

  it("事件到达后失效缓存并 refetch 启用工具列表", async () => {
    invokeMock.mockResolvedValueOnce([{ id: "claude", label: "Claude" }]);

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <Probe />
      </QueryClientProvider>
    );
    expect(await screen.findByText("Claude")).toBeInTheDocument();

    // 主窗口启动时挂监听（main.tsx 接线）
    await setupToolsChangedListener(queryClient);
    expect(eventHandlerRef.current).toBeTruthy();

    // 设置窗口保存后：下一次拉取返回新集合（codex 启用、claude 停用）
    invokeMock.mockResolvedValueOnce([{ id: "codex", label: "Codex" }]);
    await act(async () => {
      eventHandlerRef.current?.();
    });

    // 无需 remount / 切页，事件失效即触发 refetch
    await waitFor(() => expect(screen.getByTestId("tools").textContent).toBe("Codex"));
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });
});
