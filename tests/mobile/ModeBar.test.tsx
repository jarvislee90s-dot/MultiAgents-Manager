import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModeBar from "@/mobile/ModeBar";
import type { SessionModeView } from "@/mobile/api";

// 批次丙 T6 + 丁T4：模式栏。覆盖——
// ① 显示当前档（屏读回显成功时）；② 档未知时**必须**显示人工核对提示（红线 4）；
// ③ unsupported 工具不渲染；④ 切档回执 verified=false → 人工核对文案；
// 丁T4 新增：⑤ 二维家（codex/kimi）两组按钮与各自的当前档；⑥ 单轴家切换钮 + 回显；
// ⑦ 裁6 opencode「默认」（Build 已术语对齐）；⑧ 裁7 退役档不作可点按钮；
// ⑨ 旧后端（无 groups 字段）回落单轴渲染且 POST 不带 group。

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

/** 旧后端形态（无 groups/structure）——opencode 单轴 Plan 档 */
function opencodePlan(): SessionModeView {
  return {
    tool: "opencode",
    current: "plan",
    currentLabel: "计划",
    readback: true,
    switchKind: "shiftTab",
  };
}

/** 丁T4 新后端形态：单轴（opencode，默认档） */
function opencodeSingleAxis(): SessionModeView {
  return {
    tool: "opencode",
    current: "default",
    currentLabel: "默认",
    readback: true,
    switchKind: "shiftTab",
    structure: "singleAxis",
    groups: [
      {
        id: "mode",
        label: "模式",
        step: true,
        readback: true,
        current: "default",
        currentLabel: "默认",
        tiers: [
          { mode: "default", label: "默认", selectable: true },
          { mode: "plan", label: "计划", selectable: true },
        ],
        legacy: [],
      },
    ],
  };
}

/** 丁T4 新后端形态：二维（codex）。2026-09-23 codex 模式切换改造后：模式组 =
 *  单钮 toggle（计划 ⇄ 操作，shift+tab 双向）；权限组四档（含自动审批）。
 *  `modeCurrent` 驱动 toggle 的翻转方向；`permissionCurrent` 用来测「上次切换」
 *  记忆标注（null = 无记忆 → 模式未知）。 */
function codexTwoAxis(opts?: {
  modeCurrent?: MamMode;
  permissionCurrent?: MamMode | null;
}): SessionModeView {
  const modeCurrent = opts?.modeCurrent ?? "plan";
  const permissionCurrent = opts?.permissionCurrent ?? null;
  return {
    tool: "codex",
    current: modeCurrent,
    currentLabel: modeCurrent === "plan" ? "计划" : "操作",
    readback: true,
    switchKind: "slashCommand",
    structure: "twoAxis",
    groups: [
      {
        id: "mode",
        label: "模式",
        step: false,
        readback: true,
        layout: "toggle",
        current: modeCurrent,
        currentLabel: modeCurrent === "plan" ? "计划" : "操作",
        tiers: [
          { mode: "default", label: "操作", selectable: true },
          { mode: "plan", label: "计划", selectable: true },
        ],
        legacy: [],
      },
      {
        id: "permission",
        label: "权限",
        step: false,
        readback: false,
        current: permissionCurrent,
        currentLabel: permissionCurrent === null ? null : "只读",
        tiers: [
          { mode: "readOnly", label: "只读", selectable: true },
          { mode: "default", label: "默认", selectable: true },
          { mode: "acceptEdits", label: "自动审批", selectable: true },
          { mode: "bypass", label: "完全信任", selectable: true },
        ],
        legacy: [
          { label: "untrusted", note: "官方 0.149.0 起退役（配置即拒启）——不作为可选档" },
          { label: "on-failure", note: "官方已弃用（deprecated）——不作为可选档" },
        ],
      },
    ],
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
    expect(screen.getByTestId("mode-current-mode").textContent).toBe("计划");
    // 回显成功 → 无「人工核对」提示
    expect(screen.queryByTestId("mode-unknown-hint-mode")).toBeNull();
    // shiftTab 档 → 单钮「切换模式」（循环切一档，不提供直达）
    expect(screen.getByTestId("mode-switch-next-mode")).toBeTruthy();
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
    expect(screen.getByTestId("mode-current-mode").textContent).toBe("模式未知");
    // 红线 4：不假装成功——必须有核对提示
    expect(screen.getByTestId("mode-unknown-hint-mode").textContent).toContain("人工核对");
    // 切换入口仍在（盲切是允许的，只是如实标注不可验证）
    expect(screen.getByTestId("mode-switch-next-mode")).toBeTruthy();
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
    fireEvent.click(await screen.findByTestId("mode-switch-next-mode"));
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
    routes.mode = codexTwoAxis();
    routes.switchStatus = 409;
    routes.switchBody = { error: "no_mechanism" };
    render(<ModeBar session={{ id: "s7" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-mode-toggle"));
    expect((await screen.findByTestId("mode-error")).textContent).toContain("未实测");
  });
});

// ==== 丁T3 §2.7：对话框在场红线（裁8/9，问题 5）====
describe("乙T3 对话框在场拒绝对接（blocked_by_dialog）", () => {
  it("409 blocked_by_dialog：显示后端 reason 原文「终端有待决对话框，请先处理」", async () => {
    installFetch();
    routes.mode = {
      tool: "claude",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "shiftTab",
    };
    routes.switchStatus = 409;
    routes.switchBody = {
      error: "blocked_by_dialog",
      reason: "终端有待决对话框，请先处理",
    };
    render(<ModeBar session={{ id: "s8" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-next-mode"));
    const err = await screen.findByTestId("mode-error");
    expect(err.textContent).toBe("终端有待决对话框，请先处理");
    // 不得误报为「已发送切换」/「未实测」——拒绝语义必须与另两态可分
    expect(screen.queryByTestId("mode-receipt")).toBeNull();
  });

  it("blocked_by_dialog 无 reason 字段（旧后端/异体）→ 前端兜底同义文案，不显示空串", async () => {
    installFetch();
    routes.mode = {
      tool: "claude",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "shiftTab",
    };
    routes.switchStatus = 409;
    routes.switchBody = { error: "blocked_by_dialog" };
    render(<ModeBar session={{ id: "s9" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-next-mode"));
    expect((await screen.findByTestId("mode-error")).textContent).toBe(
      "终端有待决对话框，请先处理"
    );
  });
});

// ==== 丁T4 §2.6：二维结构 / 单轴 / 裁6 / 裁7 ====
describe("丁T4 模式二维与回读（§2.6 规格表）", () => {
  it("二维家（codex）：渲染模式组 + 权限组两组，各自显示当前档；权限组无回读源 → 人工核对", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t1" }} />);
    expect(await screen.findByTestId("mode-bar")).toBeTruthy();
    // 结构标记（前端渲染分支的判据）
    expect(screen.getByTestId("mode-bar").getAttribute("data-structure")).toBe("twoAxis");
    // 两组都在
    expect(screen.getByTestId("mode-group-mode")).toBeTruthy();
    expect(screen.getByTestId("mode-group-permission")).toBeTruthy();
    // 模式组：回读命中「计划」；权限组：无回读源 → 模式未知 + 人工核对
    expect(screen.getByTestId("mode-current-mode").textContent).toBe("计划");
    expect(screen.getByTestId("mode-current-permission").textContent).toBe("模式未知");
    expect(screen.getByTestId("mode-unknown-hint-permission").textContent).toContain("人工核对");
    // 组标题可见（两组才显示，帮助用户区分两个轴）
    expect(screen.getByTestId("mode-group-permission").textContent).toContain("权限");
  });

  it("二维家切档带 group：点权限组「只读」→ POST {group:'permission', target:'readOnly'}", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = { status: "key_sent", verified: false, hint: "请人工核对终端" };
    render(<ModeBar session={{ id: "t2" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-permission-readOnly"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("permission");
    expect(body.target).toBe("readOnly");
  });

  it("裁7：codex 退役旧档（untrusted/on-failure）**不渲染为可点按钮**，只作说明", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t3" }} />);
    await screen.findByTestId("mode-bar");
    // 没有任何按钮的 testid 含 untrusted / on-failure
    const ids = Array.from(document.querySelectorAll("[data-testid]")).map((el) =>
      el.getAttribute("data-testid")
    );
    expect(ids.some((id) => id !== null && /untrusted|on-failure/i.test(id))).toBe(false);
    // 但必须如实说明「已退役」（用户看得见为什么没有这两个档）
    expect(screen.getByTestId("mode-legacy-permission").textContent).toContain("untrusted");
    expect(screen.getByTestId("mode-legacy-permission").textContent).toContain("on-failure");
  });

  it("2026-09-23：codex 模式组 = 单钮 toggle「计划 ⇄ 操作」（不再有逐档按钮/不可用档）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t4" }} />);
    await screen.findByTestId("mode-bar");
    // 单钮 toggle 在（shift+tab 双向，目标档由前端按当前档翻转）
    expect(screen.getByTestId("mode-switch-mode-toggle")).toBeTruthy();
    // 不再有逐档直达按钮，也不再有「不可用」说明档
    expect(screen.queryByTestId("mode-switch-mode-default")).toBeNull();
    expect(screen.queryByTestId("mode-switch-mode-plan")).toBeNull();
    expect(screen.queryByTestId("mode-tier-disabled-mode-default")).toBeNull();
  });

  it("裁6 + 单轴：opencode 两档渲染「默认」/「计划」+ 切换钮 + 当前档回显", async () => {
    installFetch();
    routes.mode = opencodeSingleAxis();
    render(<ModeBar session={{ id: "t5" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-bar").getAttribute("data-structure")).toBe("singleAxis");
    // 当前档回显：默认（**不是** Build——裁6 术语对齐）
    expect(screen.getByTestId("mode-current-mode").textContent).toBe("默认");
    expect(screen.queryByText(/Build/)).toBeNull();
    // 单轴家仍渲染唯一的「切换模式」钮（不渲染逐档直达钮——shift+tab 一步一档）
    expect(screen.getByTestId("mode-switch-next-mode")).toBeTruthy();
    expect(screen.queryByTestId("mode-switch-mode-plan")).toBeNull();
  });

  it("单轴家只有一组 → 不渲染组标题（避免噪音）；二维家渲染", async () => {
    installFetch();
    routes.mode = opencodeSingleAxis();
    const { unmount } = render(<ModeBar session={{ id: "t6" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-group-mode").textContent).not.toContain("模式模式");
    unmount();
  });

  it("旧后端（无 groups/structure）回落单轴渲染，且 POST **不带 group**（前向兼容）", async () => {
    installFetch();
    routes.mode = opencodePlan(); // 旧形态
    routes.switchBody = { status: "key_sent", verified: false, hint: "请人工核对" };
    render(<ModeBar session={{ id: "t7" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-bar").getAttribute("data-structure")).toBe("legacy");
    expect(screen.getByTestId("mode-current-mode").textContent).toBe("计划");
    fireEvent.click(screen.getByTestId("mode-switch-next-mode"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBeUndefined();
    expect(body.target).toBe("default");
  });

  it("回读命中（verified=true）：回执「已切换」+ 重拉 GET 刷新显示", async () => {
    installFetch();
    routes.mode = opencodeSingleAxis();
    routes.switchBody = {
      status: "key_sent",
      verified: true,
      hint: null,
      current: "plan",
      currentLabel: "计划",
    };
    render(<ModeBar session={{ id: "t8" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-next-mode"));
    expect((await screen.findByTestId("mode-receipt")).textContent).toBe("已切换");
    // 重拉了一次 GET（切换后确认新档）
    const gets = fetchMock.mock.calls.filter(
      (c: unknown[]) => String(c[0]).includes("/session-mode?") && !String(c[0]).includes("switch")
    );
    expect(gets.length).toBeGreaterThanOrEqual(2);
  });

  it("回读与预期不符（verified=false 带预期/实际）：回执原样透出后端 hint", async () => {
    installFetch();
    routes.mode = opencodeSingleAxis();
    routes.switchBody = {
      status: "key_sent",
      verified: false,
      hint: "回读到的档与预期不符（预期「计划」、实际「默认」）——请人工核对终端",
    };
    render(<ModeBar session={{ id: "t9" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-next-mode"));
    const receipt = await screen.findByTestId("mode-receipt");
    expect(receipt.textContent).toContain("预期「计划」");
    expect(receipt.textContent).toContain("实际「默认」");
    expect(receipt.textContent).not.toBe("已切换");
  });

  it("mode_switch_busy（codex 运行中）：failed 回执进 error 区，不进 receipt", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = {
      status: "failed",
      error: "codex 运行中不接受模式切换（shift+tab），请等回合结束后重试",
    };
    render(<ModeBar session={{ id: "t10" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-mode-toggle"));
    const err = await screen.findByTestId("mode-error");
    expect(err.textContent).toContain("运行中不接受模式切换");
    expect(screen.queryByTestId("mode-receipt")).toBeNull();
  });
});

// ==== 批次戊 E3④：当前档高亮 + 问答待决置灰 ====
describe("ModeBar：E3④ 当前档高亮与待决置灰", () => {
  it("当前档按钮高亮：data-current=true + 反色样式；其余档 data-current=false", async () => {
    installFetch();
    // 权限组带「上次切换」记忆（readOnly）→ 逐档高亮断言落在权限组
    routes.mode = codexTwoAxis({ permissionCurrent: "readOnly" });
    render(<ModeBar session={{ id: "s-e3-hl" }} />);
    const readOnlyBtn = await screen.findByTestId("mode-switch-permission-readOnly");
    expect(readOnlyBtn.getAttribute("data-current")).toBe("true");
    expect(readOnlyBtn.className).toContain("font-semibold");
    const bypassBtn = screen.getByTestId("mode-switch-permission-bypass");
    expect(bypassBtn.getAttribute("data-current")).toBe("false");
    expect(bypassBtn.className).not.toContain("font-semibold");
    // 模式组是 toggle 单钮：data-current 直接承载当前档（plan）
    expect(screen.getByTestId("mode-switch-mode-toggle").getAttribute("data-current")).toBe("plan");
  });

  it("问答待决：questionPending=true → 按钮全部禁用 + 原因文案 + data-question-pending", async () => {
    installFetch();
    routes.mode = { ...codexTwoAxis(), questionPending: true };
    render(<ModeBar session={{ id: "s-e3-pending" }} />);
    await screen.findByTestId("mode-question-pending-hint");
    expect(screen.getByTestId("mode-bar").getAttribute("data-question-pending")).toBe("true");
    expect((screen.getByTestId("mode-switch-mode-toggle") as HTMLButtonElement).disabled).toBe(
      true
    );
    expect(
      (screen.getByTestId("mode-switch-permission-readOnly") as HTMLButtonElement).disabled
    ).toBe(true);
  });

  it("非待决（questionPending 缺省/ false）不置灰不显文案", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "s-e3-normal" }} />);
    await screen.findByTestId("mode-switch-mode-toggle");
    expect(screen.queryByTestId("mode-question-pending-hint")).toBeNull();
    expect((screen.getByTestId("mode-switch-mode-toggle") as HTMLButtonElement).disabled).toBe(
      false
    );
  });
});

// ==== 2026-09-23 codex 模式切换改造：toggle 翻转 / bypass 二次确认 / 档位记忆 ====
describe("ModeBar：codex toggle 与完全信任二次确认（2026-09-23）", () => {
  it("toggle 点击 = shift+tab 语义：current=plan 时发 {group:'mode', target:'default'}", async () => {
    installFetch();
    routes.mode = codexTwoAxis(); // mode 组 current = plan
    routes.switchBody = { status: "key_sent", verified: true, hint: null };
    render(<ModeBar session={{ id: "mc-t1" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-mode-toggle"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("mode");
    expect(body.target).toBe("default", "当前是计划 → 点击切到操作");
  });

  it("权限组 current 有值时 toggle 翻转方向相反：current=default → 发 target=plan", async () => {
    installFetch();
    // mode 组 current = 操作（default）→ 点击应发 target=plan（切到计划）
    routes.mode = codexTwoAxis({ modeCurrent: "default" });
    routes.switchBody = { status: "key_sent", verified: true, hint: null };
    render(<ModeBar session={{ id: "mc-t2" }} />);
    const toggle = await screen.findByTestId("mode-switch-mode-toggle");
    expect(toggle.getAttribute("data-current")).toBe("default");
    fireEvent.click(toggle);
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.target).toBe("plan");
  });

  it("权限组 current=未知时 toggle 禁用（盲按会 50% 误切）", async () => {
    installFetch();
    // 权限组记忆不影响模式组——用旧后端回落形态让 mode 组 current=null
    routes.mode = {
      tool: "codex",
      current: null,
      currentLabel: null,
      readback: false,
      switchKind: "shiftTab",
      structure: "twoAxis",
      groups: [
        {
          id: "mode",
          label: "模式",
          step: false,
          readback: false,
          layout: "toggle",
          current: null,
          currentLabel: null,
          tiers: [
            { mode: "default", label: "操作", selectable: true },
            { mode: "plan", label: "计划", selectable: true },
          ],
          legacy: [],
        },
      ],
    };
    render(<ModeBar session={{ id: "mc-t2b" }} />);
    const toggle = await screen.findByTestId("mode-switch-mode-toggle");
    expect((toggle as HTMLButtonElement).disabled).toBe(true);
    expect(toggle.getAttribute("data-current")).toBe("unknown");
  });

  it("权限组四档渲染：自动审批（acceptEdits）档出现在只读/默认与完全信任之间", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "mc-t3" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-switch-permission-acceptEdits").textContent).toBe("自动审批");
    expect(screen.getByTestId("mode-switch-permission-readOnly")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-permission-default")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-permission-bypass")).toBeTruthy();
  });

  it("完全信任二次确认：点完全信任先出确认条且**不发请求**；确认后才发 bypass", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = { status: "key_sent", verified: true, hint: null };
    render(<ModeBar session={{ id: "mc-t4" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-permission-bypass"));
    await flushAsync();
    // 确认条出现，但**零 POST**（未确认不发任何键）
    expect(screen.getByTestId("mode-bypass-confirm").textContent).toContain("两次按键");
    expect(
      fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("/session-mode/switch"))
    ).toBe(false);
    // 确认启用 → POST {group:'permission', target:'bypass'}
    fireEvent.click(screen.getByTestId("mode-bypass-confirm-yes"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("permission");
    expect(body.target).toBe("bypass");
  });

  it("完全信任二次确认：取消收起确认条，全程零请求", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "mc-t5" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-permission-bypass"));
    expect(screen.getByTestId("mode-bypass-confirm")).toBeTruthy();
    fireEvent.click(screen.getByTestId("mode-bypass-confirm-no"));
    expect(screen.queryByTestId("mode-bypass-confirm")).toBeNull();
    expect(
      fetchMock.mock.calls.some((c: unknown[]) => String(c[0]).includes("/session-mode/switch"))
    ).toBe(false);
  });

  it("权限组记忆标注：current 有值（上次切换）→ 显示档名 +「（上次切换）」，无未知提示", async () => {
    installFetch();
    routes.mode = codexTwoAxis({ permissionCurrent: "readOnly" });
    render(<ModeBar session={{ id: "mc-t6" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-current-permission").textContent).toBe("只读");
    expect(screen.getByTestId("mode-current-source-permission").textContent).toBe("（上次切换）");
    expect(screen.queryByTestId("mode-unknown-hint-permission")).toBeNull();
    // 只读按钮按记忆高亮
    expect(screen.getByTestId("mode-switch-permission-readOnly").getAttribute("data-current")).toBe(
      "true"
    );
  });
});
