import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ApproveCard from "@/mobile/ApproveCard";
import type { ApproveOptionsView } from "@/mobile/api";

// M8 Task 12：移动端红卡审批选项卡。fetch 全量 stub（盖过 setup.ts 的 msw），
// 按 URL 分路到 approve-options / approve 两族端点（注意前缀包含关系：
// /session-approve-options ⊃ /session-approve，长路径必须先判——MessageComposer
// 测试同款教训）。组件挂载即拉选项，用例先 findBy 选项按钮就绪再交互。

/** 审批选项夹具（GET /session-approve-options 载荷；选项只含 id+label——
 *  键位是投递层机密不外泄 UI，组件契约只消费 id/label） */
function approveOptions(overrides: Partial<ApproveOptionsView> = {}): ApproveOptionsView {
  return {
    available: true,
    options: [
      { id: "approve", label: "允许" },
      { id: "reject", label: "拒绝" },
    ],
    verifiedWith: "claude 1.2.3 (2026-09-18 实测)",
    currentVersion: "claude 1.2.3",
    drift: false,
    ...overrides,
  };
}

interface Routes {
  options?: ApproveOptionsView;
  optionsStatus?: number;
  approve?: Record<string, unknown>;
  approveStatus?: number;
  approveBody?: Record<string, unknown>;
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

/** 按 URL 分路的 fetch stub（判序：/session-approve-options 在前，避免被
 *  /session-approve 前缀误吞） */
function installFetch() {
  fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/session-approve-options")) {
      if (routes.optionsStatus) return new Response("no", { status: routes.optionsStatus });
      return new Response(JSON.stringify(routes.options ?? approveOptions()), { status: 200 });
    }
    if (url.includes("/session-approve")) {
      if (routes.approveStatus) {
        return new Response(JSON.stringify(routes.approveBody ?? { error: "internal" }), {
          status: routes.approveStatus,
        });
      }
      return new Response(JSON.stringify(routes.approve ?? { status: "key_sent" }), { status: 200 });
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
}

/** POST /session-approve 的调用（URL 精确到 /session-approve 结尾，排除 -options 前缀） */
function approveCalls(): Array<Array<unknown>> {
  return fetchMock.mock.calls.filter((c: unknown[]) => /\/session-approve$/.test(String(c[0])));
}

/** 放行 mock fetch 的 promise 链（若干轮微任务冲刷，足以走完 fetch→json→setState） */
async function flushAsync() {
  for (let i = 0; i < 6; i += 1) {
    await act(async () => {
      await Promise.resolve();
    });
  }
}

describe("ApproveCard：红卡审批选项卡（M8 Task 12）", () => {
  it("available=true：「允许」「拒绝」按钮按 label 渲染（载荷无 key 字段，组件只消费 id/label）", async () => {
    installFetch();
    routes.options = approveOptions();
    const { container } = render(<ApproveCard session={{ id: "sess-1" }} />);
    expect(await screen.findByTestId("approve-option-approve")).toBeTruthy();
    expect(screen.getByTestId("approve-option-approve").textContent).toBe("允许");
    expect(screen.getByTestId("approve-option-reject").textContent).toBe("拒绝");
    expect(screen.getByTestId("approve-card").textContent).toContain("等待批准");
    // 契约锚点：响应载荷不含 key（键位不外泄 UI），组件仅凭 id/label 完整渲染
    expect(routes.options.options.every((o) => !("key" in o))).toBe(true);
    // 无漂移时提示条不出现
    expect(container.querySelector("[data-testid='approve-drift']")).toBeNull();
  });

  it("点「允许」：POST body {sessionId, optionId:\"approve\"} → 进入「已发送按键」态（按钮禁用）", async () => {
    installFetch();
    routes.options = approveOptions();
    routes.approve = { status: "key_sent" };
    render(<ApproveCard session={{ id: "sess-1" }} />);
    fireEvent.click(await screen.findByTestId("approve-option-approve"));
    expect(await screen.findByTestId("approve-sent").then((el) => el.textContent)).toBe(
      "已发送按键"
    );
    expect(approveCalls()).toHaveLength(1);
    expect(JSON.parse(String((approveCalls()[0][1] as RequestInit).body))).toEqual({
      sessionId: "sess-1",
      optionId: "approve",
    });
    // 终态：选项按钮禁用（防重复应答）
    expect((screen.getByTestId("approve-option-approve") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("approve-option-reject") as HTMLButtonElement).disabled).toBe(true);
  });

  it("available=false：组件不渲染（container empty，卡自身自隐）", async () => {
    installFetch();
    routes.options = approveOptions({ available: false, options: [] });
    const { container } = render(<ApproveCard session={{ id: "sess-1" }} />);
    await flushAsync(); // 选项落地（available=false）后依旧不渲染
    expect(container.firstElementChild).toBeNull();
  });

  it("drift=true：提示条「映射待实测确认，若提示不符请用普通发送」出现", async () => {
    installFetch();
    routes.options = approveOptions({ drift: true, currentVersion: "claude 2.0.0" });
    render(<ApproveCard session={{ id: "sess-1" }} />);
    expect(await screen.findByTestId("approve-drift").then((el) => el.textContent)).toContain(
      "映射待实测确认，若提示不符请用普通发送"
    );
  });

  it("failed{error}：错误文案展示且可重试（重试转「已发送按键」）", async () => {
    installFetch();
    routes.options = approveOptions();
    routes.approve = { status: "failed", error: "该会话投递进行中，请稍后重试" };
    render(<ApproveCard session={{ id: "sess-1" }} />);
    fireEvent.click(await screen.findByTestId("approve-option-approve"));
    expect(await screen.findByTestId("approve-error").then((el) => el.textContent)).toContain(
      "该会话投递进行中，请稍后重试"
    );
    // 可重试：按钮保持可点，修正路由后重按即重试成功
    expect((screen.getByTestId("approve-option-approve") as HTMLButtonElement).disabled).toBe(false);
    routes.approve = { status: "key_sent" };
    fireEvent.click(screen.getByTestId("approve-option-approve"));
    expect(await screen.findByTestId("approve-sent")).toBeTruthy();
    expect(approveCalls()).toHaveLength(2);
  });

  it("ApiError 409 not_waiting：中文文案「会话不在等待状态」", async () => {
    installFetch();
    routes.options = approveOptions();
    routes.approveStatus = 409;
    routes.approveBody = { error: "not_waiting" };
    render(<ApproveCard session={{ id: "sess-1" }} />);
    fireEvent.click(await screen.findByTestId("approve-option-approve"));
    expect(await screen.findByTestId("approve-error").then((el) => el.textContent)).toContain(
      "会话不在等待状态"
    );
    expect(approveCalls()).toHaveLength(1);
  });

  it("ApiError 404 no_mapping：降级文案「该工具暂不支持审批应答，请用普通发送」", async () => {
    installFetch();
    routes.options = approveOptions();
    routes.approveStatus = 404;
    routes.approveBody = { error: "no_mapping" };
    render(<ApproveCard session={{ id: "sess-1" }} />);
    fireEvent.click(await screen.findByTestId("approve-option-approve"));
    expect(await screen.findByTestId("approve-error").then((el) => el.textContent)).toContain(
      "该工具暂不支持审批应答，请用普通发送"
    );
    expect(approveCalls()).toHaveLength(1);
  });
});
