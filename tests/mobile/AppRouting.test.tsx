import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "@/mobile/App";
import { MockEventSource } from "./eventSourceMock";
import type { Session, SessionsResponse } from "@/types/session";

// M3 Task 8：App 三态路由（board ↔ detail；预览是 SessionDetail 内部状态不进路由）。
// Board 常驻挂载 hidden 切换：进入详情后 Board 的 DOM 仍在（数据保持），
// 返回后卡片原样可见。链路与既有 App.test.tsx 同构（SSE 快照驱动探测）。

function sessionFixture(overrides: Partial<Session> = {}): Session {
  return {
    id: "sess-route-1",
    agentType: "claude",
    projectName: "demo-proj",
    projectPath: "/tmp/demo",
    title: "任务标题",
    gitBranch: null,
    githubUrl: null,
    status: "processing",
    lastMessage: null,
    lastMessageRole: null,
    lastActivityAt: "2026-09-15T00:00:00Z",
    pid: 1,
    cpuUsage: 0,
    activeSubagentCount: 0,
    form: "cli",
    jumpSupported: false,
    unread: false,
    ...overrides,
  };
}

function okSessions(sessions: Session[]): SessionsResponse {
  return { sessions, totalCount: sessions.length, waitingCount: 0 };
}

/** 脚本化 EventSource：连接建立后自动送达快照 */
function installSse(snapshot: SessionsResponse) {
  class ScriptedEventSource extends MockEventSource {
    constructor(url: string) {
      super(url);
      setTimeout(() => this.emit("snapshot", snapshot), 0);
    }
  }
  vi.stubGlobal("EventSource", ScriptedEventSource);
}

/** fetch 分路：host（Board 页头）+ session-messages + session-files + file */
function installFetch() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/m/api/v1/host")) {
        return new Response(
          JSON.stringify({
            host: { name: "JARVIS-Mac", platform: "macos", version: "0.4.1" },
            enabledTools: ["claude"],
          }),
          { status: 200 }
        );
      }
      if (url.includes("/session-messages")) {
        return new Response(
          JSON.stringify({
            messages: [
              {
                seq: 0,
                role: "user",
                kind: "user",
                content: "详情页首条",
                ts: 1,
                collapsed: false,
              },
            ],
          }),
          { status: 200 }
        );
      }
      if (url.includes("/session-files")) {
        return new Response(JSON.stringify({ files: [], truncated: false }), { status: 200 });
      }
      if (url.includes("/file?")) {
        return new Response(JSON.stringify({ content: "x", mime: "text/plain" }), { status: 200 });
      }
      if (url.includes("/m/api/v1/sessions")) {
        return new Response(JSON.stringify(okSessions([])), { status: 200 });
      }
      throw new Error(`unexpected fetch: ${url}`);
    })
  );
}

beforeEach(() => {
  MockEventSource.reset();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("App 路由：看板卡片 ↔ 会话详情", () => {
  it("卡片点击进入详情（消息可见），返回回到看板且卡片数据保持", async () => {
    installSse(okSessions([sessionFixture()]));
    installFetch();
    render(<App />);

    // 探测成功：配对页卸载，看板卡片出现
    expect(await screen.findByText("demo-proj")).toBeTruthy();
    expect(screen.queryByText("MAM 远程接入")).toBeNull();

    // 卡片点击 → 详情视图（Board 转 hidden 但仍挂载）
    fireEvent.click(screen.getByText("demo-proj").closest("li") as HTMLLIElement);
    expect(await screen.findByTestId("detail-back")).toBeTruthy();
    expect(await screen.findByText("详情页首条")).toBeTruthy();
    // Board DOM 仍在（hidden 切换，数据与滚动位置由常驻挂载保持）
    expect(screen.getByText("会话看板")).toBeTruthy();

    // 返回 → 看板恢复，详情卸载
    fireEvent.click(screen.getByTestId("detail-back"));
    expect(screen.queryByTestId("detail-back")).toBeNull();
    expect(screen.getByText("demo-proj")).toBeTruthy();
  });

  it("未配对态不渲染详情入口：卡片点击前（无 SSE 快照）只有配对页", async () => {
    vi.stubGlobal("EventSource", undefined); // 环境不支持 SSE → 降级轮询
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("network down");
      })
    );
    render(<App />);
    // 探测中（paired=null）：配对页在场，无详情
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(screen.getByText("MAM 远程接入")).toBeTruthy();
    expect(screen.queryByTestId("detail-back")).toBeNull();
  });
});
