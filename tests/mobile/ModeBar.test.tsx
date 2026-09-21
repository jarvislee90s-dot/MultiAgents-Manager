import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModeBar from "@/mobile/ModeBar";
import type { SessionModeView } from "@/mobile/api";

// 批次丙 T6：模式栏。覆盖三件事——
// ① 显示当前档（屏读回显成功时）；
// ② 档未知时（屏读失败/不支持回显）**必须**显示人工核对提示，不得假装知道（红线 4）；
// ③ unsupported 工具不渲染；切档回执 verified=false 时给人工核对文案。

interface Routes {
  mode?: SessionModeView;
  modeStatus?: number;
  modeNetworkFail?: boolean;
  switchBody?: Record<string, unknown>;
  switchStatus?: number;
}

let routes: Routes;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  routes = {};
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function installFetch() {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/session-mode/switch")) {
      if (routes.switchStatus) {
        return new Response(JSON.stringify(routes.switchBody ?? { error: "internal" }), {
          status: routes.switchStatus,
        });
      }
      return new Response(
        JSON.stringify(routes.switchBody ?? { status: "key_sent", verified: false, hint: null }),
        { status: 200 }
      );
    }
    if (url.includes("/session-mode")) {
      if (routes.modeNetworkFail) throw new TypeError("network down");
      if (routes.modeStatus) return new Response("no", { status: routes.modeStatus });
      return new Response(JSON.stringify(routes.mode ?? opencodePlan()), { status: 200 });
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

function opencodePlan(): SessionModeView {
  return {
    tool: "opencode",
    current: "plan",
    currentLabel: "计划",
    readback: true,
    switchKind: "shiftTab",
  };
}

async function flushAsync() {
  for (let i = 0; i < 6; i += 1) {
    await Promise.resolve();
  }
}

describe("ModeBar：模式显示与切档（批次丙 T6）", () => {
  it("显示当前档（opencode 屏读回显成功）：档名可见 + 切换模式按钮", async () => {
    installFetch();
    render(<ModeBar session={{ id: "s1" }} />);
    expect(await screen.findByTestId("mode-bar")).toBeTruthy();
    expect(screen.getByTestId("mode-bar").getAttribute("data-mode")).toBe("plan");
    expect(screen.getByTestId("mode-current").textContent).toBe("计划");
    // 回显成功 → 无「人工核对」提示
    expect(screen.queryByTestId("mode-unknown-hint")).toBeNull();
    // shiftTab 档 → 单钮「切换模式」（循环切一档，不提供直达）
    expect(screen.getByTestId("mode-switch-next")).toBeTruthy();
  });

  it("档未知（屏读失败/不支持回显）：显示「模式未知」+ 必须给人工核对提示（红线 4）", async () => {
    installFetch();
    routes.mode = {
      tool: "claude",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "shiftTab",
    };
    render(<ModeBar session={{ id: "s2" }} />);
    expect(await screen.findByTestId("mode-bar")).toBeTruthy();
    expect(screen.getByTestId("mode-bar").getAttribute("data-mode")).toBe("unknown");
    expect(screen.getByTestId("mode-current").textContent).toBe("模式未知");
    // 红线 4：不假装成功——必须有核对提示
    expect(screen.getByTestId("mode-unknown-hint").textContent).toContain("人工核对");
    // 切换入口仍在（盲切是允许的，只是如实标注不可验证）
    expect(screen.getByTestId("mode-switch-next")).toBeTruthy();
  });

  it("切档 verified=false：回执为人工核对提示，不声称已切到目标档", async () => {
    installFetch();
    routes.mode = {
      tool: "claude",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "shiftTab",
    };
    routes.switchBody = {
      status: "key_sent",
      verified: false,
      hint: "该工具的模式回显未实测，请人工核对终端当前模式",
    };
    render(<ModeBar session={{ id: "s3" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-next"));
    const receipt = await screen.findByTestId("mode-receipt");
    expect(receipt.textContent).toContain("人工核对");
    expect(receipt.textContent).not.toContain("已切换到");
    // POST 体正确
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.sessionId).toBe("s3");
    expect(body.target).toBe("default");
  });

  it("codex（slashCommand）：渲染逐档按钮（plan / bypass），点按提交对应 target", async () => {
    installFetch();
    routes.mode = {
      tool: "codex",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "slashCommand",
    };
    routes.switchBody = { status: "key_sent", verified: false, hint: "请人工核对" };
    render(<ModeBar session={{ id: "s4" }} />);
    expect(await screen.findByTestId("mode-switch-plan")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-bypass")).toBeTruthy();
    // codex 无命令证据的档不渲染（acceptEdits/readOnly）
    expect(screen.queryByTestId("mode-switch-acceptEdits")).toBeNull();
    fireEvent.click(screen.getByTestId("mode-switch-plan"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.target).toBe("plan");
  });

  it("unsupported 工具：不渲染（无实测切换机制）", async () => {
    installFetch();
    routes.mode = {
      tool: "zcode",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "unsupported",
    };
    const { container } = render(<ModeBar session={{ id: "s5" }} />);
    await flushAsync();
    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId("mode-bar")).toBeNull();
  });

  it("拉取失败：静默自隐（不渲染、不报错）", async () => {
    installFetch();
    routes.modeNetworkFail = true;
    const { container } = render(<ModeBar session={{ id: "s6" }} />);
    await flushAsync();
    expect(container.firstChild).toBeNull();
  });

  it("409 no_mechanism：显示中文文案「该工具的模式切换未实测」", async () => {
    installFetch();
    routes.mode = {
      tool: "codex",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "slashCommand",
    };
    routes.switchStatus = 409;
    routes.switchBody = { error: "no_mechanism" };
    render(<ModeBar session={{ id: "s7" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-plan"));
    expect((await screen.findByTestId("mode-error")).textContent).toContain("未实测");
  });
});
