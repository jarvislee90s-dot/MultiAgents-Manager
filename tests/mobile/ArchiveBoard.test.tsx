import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ArchiveBoard from "@/mobile/ArchiveBoard";
import type { ArchivedPayload } from "@/mobile/api";

const payload: ArchivedPayload = {
  archived: [
    {
      sessionId: "a1", agentType: "codex", projectPath: "/tmp/p1", projectName: "proj-1",
      title: "修复登录", lastStatus: "idle", lastSeenAt: new Date(Date.now() - 3600_000).toISOString(),
    },
  ],
  projects: ["proj-1"],
};

function installFetch(ok = true) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/sessions-archived") && !url.includes("DELETE")) {
        if (!ok) return new Response("boom", { status: 500 });
        return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
      }
      return new Response("{}", { status: 404 });
    }),
  );
}

describe("ArchiveBoard：历史页", () => {
  beforeEach(() => installFetch());
  afterEach(() => { vi.unstubAllGlobals(); cleanup(); });

  it("默认 1 天拉取并渲染卡片；卡片上无任何打开按钮", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (i: RequestInfo | URL) => {
      calls.push(String(i));
      return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
    }));
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    // selector 收窄到卡片 span：项目下拉 <option> 同文本，findByText 多匹配会抛错
    expect(await screen.findByText("proj-1", { selector: "span" })).toBeTruthy();
    expect(calls[0]).toContain("days=1");
    expect(screen.queryByTestId("session-open")).toBeNull(); // 裁决 6：按钮不浮在卡片上
  });

  it("切 7 天 → 重新拉取（URL 带 days=7）", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", vi.fn(async (i: RequestInfo | URL) => {
      calls.push(String(i));
      return new Response(JSON.stringify(payload), { headers: { "content-type": "application/json" } });
    }));
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    await screen.findByText("proj-1", { selector: "span" });
    fireEvent.click(screen.getByTestId("archive-days-7"));
    await waitFor(() => expect(calls.some((c) => c.includes("days=7"))).toBe(true));
  });

  it("失败态显示重试按钮；点击重发", async () => {
    installFetch(false);
    render(<ArchiveBoard onBack={() => {}} onOpenCard={() => {}} />);
    expect(await screen.findByTestId("archive-retry")).toBeTruthy();
  });
});
