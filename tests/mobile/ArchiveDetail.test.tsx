import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ArchiveDetail from "@/mobile/ArchiveDetail";
import type { ArchivedSession } from "@/mobile/api";

const card: ArchivedSession = {
  sessionId: "dead-1", agentType: "codex", projectPath: "/tmp/p1", projectName: "proj-1",
  title: "标题", lastStatus: "idle", lastSeenAt: new Date().toISOString(),
};

function installFetch(routes: Record<string, unknown>) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/session-open")) {
        const body = (routes["sessionOpen"]) as Record<string, string> | undefined;
        const status = routes["sessionOpenStatus"] as number | undefined;
        return new Response(JSON.stringify(body ?? { status: "opening" }), {
          status: status ?? 200,
          headers: { "content-type": "application/json" },
        });
      }
      if (url.includes("/session-messages")) {
        return new Response(
          JSON.stringify({ messages: [{ seq: 1, role: "user", content: "旧消息", kind: "text", ts: 1 }] }),
          { headers: { "content-type": "application/json" } },
        );
      }
      if (url.includes("/sessions-archived")) {
        return new Response(JSON.stringify({ deleted: 1 }), { headers: { "content-type": "application/json" } });
      }
      return new Response("{}", { status: 404 });
    }),
  );
}

describe("ArchiveDetail：归档详情与激活", () => {
  beforeEach(() => installFetch({}));
  afterEach(() => { vi.unstubAllGlobals(); cleanup(); });

  it("渲染历史消息内容 + 在桌面端打开按钮存在", async () => {
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    expect(await screen.findByText("旧消息")).toBeTruthy();
    expect(screen.getByTestId("session-open").textContent).toBe("在桌面端打开");
  });

  it("opening 回执 → onActivated 回调（乐观回看板，裁决 6 闭环）", async () => {
    const activated = vi.fn();
    installFetch({ sessionOpen: { status: "opening" } });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={activated} />);
    fireEvent.click(screen.getByTestId("session-open"));
    await screen.findByText("正在电脑上打开终端…");
    expect(activated).toHaveBeenCalled();
  });

  it("failed 回执 → 错误文案上屏、不回调（可重试）", async () => {
    const activated = vi.fn();
    installFetch({ sessionOpen: { status: "failed", error: "终端启动失败（模拟）" } });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    fireEvent.click(screen.getByTestId("session-open"));
    expect(await screen.findByTestId("session-open-error")).toBeTruthy();
    expect(activated).not.toHaveBeenCalled();
  });

  it("打开失败哨兵分诊：no_session（404 载荷）→ 归档不存在文案", async () => {
    installFetch({ sessionOpen: { error: "no_session" }, sessionOpenStatus: 404 });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    fireEvent.click(screen.getByTestId("session-open"));
    expect(await screen.findByText("归档记录已不存在，请刷新列表")).toBeTruthy();
  });

  it("打开失败哨兵分诊：no_resume_command（404 载荷）→ 复用工具不支持文案", async () => {
    installFetch({ sessionOpen: { error: "no_resume_command" }, sessionOpenStatus: 404 });
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    fireEvent.click(screen.getByTestId("session-open"));
    expect(
      await screen.findByText("打开失败：该工具 resume 命令待查证，暂不支持一键打开"),
    ).toBeTruthy();
  });

  it("信息行 agentType 用 TOOL_LABELS 中文（不再裸显工具 id）", async () => {
    render(<ArchiveDetail session={card} onBack={() => {}} onActivated={() => {}} />);
    await screen.findByText("旧消息");
    expect(screen.getByText(/Codex · \/tmp\/p1 · /)).toBeTruthy();
  });

  it("不支持 resume 的工具 → 按钮禁用 + 原因（门迁移回归锁）", () => {
    render(
      <ArchiveDetail
        session={{ ...card, agentType: "workbuddy" }}
        onBack={() => {}}
        onActivated={() => {}}
      />,
    );
    const btn = screen.getByTestId("session-open") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(screen.getByText("该工具 resume 命令待查证，暂不支持一键打开")).toBeTruthy();
  });

  it("从归档移除 → DELETE 后返回", async () => {
    const back = vi.fn();
    render(<ArchiveDetail session={card} onBack={back} onActivated={() => {}} />);
    fireEvent.click(screen.getByTestId("archive-remove"));
    fireEvent.click(screen.getByTestId("archive-remove-confirm"));
    await new Promise((r) => setTimeout(r, 0));
    expect(back).toHaveBeenCalled();
  });
});
