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
  /** 2026-09-23 picker：/session-mode/menu 的响应体（open/pick/GET 重读同形） */
  menuBody?: Record<string, unknown>;
  menuStatus?: number;
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
    // 长路径必须先判（否则被 `/session-mode` 前缀吞）：picker 的菜单端点在 switch 之前
    if (url.includes("/session-mode/menu")) {
      if (routes.menuStatus) {
        return new Response(JSON.stringify(routes.menuBody ?? { error: "internal" }), {
          status: routes.menuStatus,
        });
      }
      return new Response(JSON.stringify(routes.menuBody ?? { status: "none", options: [] }), {
        status: 200,
      });
    }
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
 *  单钮 toggle（计划 ⇄ 操作，shift+tab 双向）；权限组 = **单选面板**（picker，
 *  用户方案：后端读回终端菜单选项，用户点选哪项就敲哪个数字键）。
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
        layout: "picker",
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

/** picker 的 fetch 链比切档长（async 回调里再 await + setState），6 轮微任务不够
 *  ——用一轮宏任务（0ms 定时器）等 React 把面板状态刷完。 */
async function flushPanel() {
  await flushAsync();
  await new Promise((res) => setTimeout(res, 0));
  await flushAsync();
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

  it("二维家：权限组是**单选面板**（picker）——不渲染逐档按钮，只出一颗「切换权限」钮", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t2" }} />);
    expect(await screen.findByTestId("mode-picker-permission-open")).toBeTruthy();
    // 旧逐档按钮**不在了**（前端不再硬编码「哪档对应哪个数字」——档位编号随
    // Guardian 配置前移，硬编码会错位；编号一律来自终端屏读）
    expect(screen.queryByTestId("mode-switch-permission-readOnly")).toBeNull();
    expect(screen.queryByTestId("mode-switch-permission-bypass")).toBeNull();
    // 面板未点开时不出
    expect(screen.queryByTestId("mode-menu-panel")).toBeNull();
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
  it("当前档高亮：toggle 单钮 data-current=当前档；权限组高亮走「上次切换」记忆（无逐档按钮）", async () => {
    installFetch();
    routes.mode = codexTwoAxis({ permissionCurrent: "readOnly" });
    render(<ModeBar session={{ id: "s-e3-hl" }} />);
    await screen.findByTestId("mode-picker-permission-open");
    // 模式组是 toggle 单钮：data-current 直接承载当前档（plan）
    expect(screen.getByTestId("mode-switch-mode-toggle").getAttribute("data-current")).toBe("plan");
    // 权限组是 picker：当前档由「上次切换」记忆显示（高亮不再落在按钮上——
    // 逐档按钮已退役，编号一律来自终端屏读）
    expect(screen.getByTestId("mode-current-permission").textContent).toBe("只读");
    expect(screen.getByTestId("mode-current-source-permission").textContent).toBe("（上次切换）");
  });

  it("E3④ 问答待决：questionPending=true → 按钮全部禁用 + 原因文案 + data-question-pending", async () => {
    installFetch();
    routes.mode = { ...codexTwoAxis(), questionPending: true };
    render(<ModeBar session={{ id: "s-e3-pending" }} />);
    await screen.findByTestId("mode-question-pending-hint");
    expect(screen.getByTestId("mode-bar").getAttribute("data-question-pending")).toBe("true");
    expect((screen.getByTestId("mode-switch-mode-toggle") as HTMLButtonElement).disabled).toBe(
      true
    );
    expect((screen.getByTestId("mode-picker-permission-open") as HTMLButtonElement).disabled).toBe(
      true
    );
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

  it("picker：点「切换权限」→ POST {action:'open'} → 渲染终端菜单选项（编号+屏上原文）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [
        { number: 1, label: "Read Only", highlighted: false },
        { number: 2, label: "Ask for approval (non-admin sandbox) (current)", highlighted: true },
        { number: 3, label: "Approve for me", highlighted: false },
        { number: 4, label: "Full Access", highlighted: false },
      ],
    };
    render(<ModeBar session={{ id: "mc-p1" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/menu")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.action).toBe("open");
    // 选项渲染：编号徽标 = 屏上编号；文本 = 屏上原文（含内联后缀原样展示）
    expect(screen.getByTestId("mode-menu-option-1").textContent).toContain("1");
    expect(screen.getByTestId("mode-menu-option-2").textContent).toContain(
      "Ask for approval (non-admin sandbox) (current)"
    );
    expect(screen.getByTestId("mode-menu-option-2").getAttribute("data-highlighted")).toBe("true");
    expect(screen.getByTestId("mode-menu-option-4")).toBeTruthy();
    expect(screen.getByTestId("mode-menu-panel").getAttribute("data-panel")).toBe("menu");
  });

  it("picker：点选项 → POST {action:'pick', number}（发的是**屏上编号**）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [
        { number: 1, label: "Read Only", highlighted: false },
        { number: 3, label: "Approve for me", highlighted: false },
      ],
    };
    render(<ModeBar session={{ id: "mc-p2" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    // 点第 3 项 → 发 number=3（**不是前端算的「第 2 项」**——屏上印的是 3，就敲 3）
    routes.menuBody = { status: "done", verified: true, hint: "终端回执：Permissions updated to Approve for me" };
    fireEvent.click(screen.getByTestId("mode-menu-option-3"));
    await flushPanel();
    const calls = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-mode/menu")
    );
    const pickBody = JSON.parse(String((calls[calls.length - 1]![1] as RequestInit).body));
    expect(pickBody.action).toBe("pick");
    expect(pickBody.number).toBe(3);
  });

  it("picker：done + verified → 收起面板 + 显示后端回执原文（含终端回执行）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 1, label: "Read Only", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p3" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    routes.menuBody = { status: "done", verified: true, hint: "终端回执：Permissions updated to Read Only" };
    fireEvent.click(screen.getByTestId("mode-menu-option-1"));
    const receipt = await screen.findByTestId("mode-receipt");
    expect(receipt.textContent).toContain("Permissions updated to Read Only");
    expect(screen.queryByTestId("mode-menu-panel")).toBeNull();
  });

  it("picker：done + **verified=false** → 不假装成功（原样透出后端文案 + 显红色错误位）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 1, label: "Read Only", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p4" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    routes.menuBody = {
      status: "done",
      verified: false,
      hint: "已按你点选的编号投递，但未在屏上读到成功回执——请人工核对终端",
    };
    fireEvent.click(screen.getByTestId("mode-menu-option-1"));
    const err = await screen.findByTestId("mode-error");
    expect(err.textContent).toContain("请人工核对终端");
    expect(screen.queryByTestId("mode-receipt")).toBeNull();
  });

  it("picker 二阶段：pick 返回 confirm → 面板切为确认框选项（由用户再点，MAM 不代按）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 4, label: "Full Access", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p5" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    routes.menuBody = {
      status: "confirm",
      options: [
        { number: 1, label: "Yes, continue anyway", highlighted: true },
        { number: 2, label: "Cancel", highlighted: false },
      ],
    };
    fireEvent.click(screen.getByTestId("mode-menu-option-4"));
    await flushPanel();
    expect(screen.getByTestId("mode-menu-panel").getAttribute("data-panel")).toBe("confirm");
    expect(screen.getByTestId("mode-menu-option-1").textContent).toContain("Yes, continue anyway");
    // 确认框选项也要用户点——**第二次请求前不发任何东西**
    const before = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-mode/menu")
    ).length;
    expect(before).toBe(2, "open + pick 两次；确认键尚未发");
  });

  it("picker：关闭按钮只收起面板，零请求", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 1, label: "Read Only", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p6" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    const before = fetchMock.mock.calls.length;
    fireEvent.click(screen.getByTestId("mode-menu-close"));
    expect(screen.queryByTestId("mode-menu-panel")).toBeNull();
    expect(fetchMock.mock.calls.length).toBe(before);
  });

  it("picker：「重新读取」用 GET（零注入）重同步", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 1, label: "Read Only", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p7" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 2, label: "Ask for approval (current)", highlighted: true }],
    };
    fireEvent.click(screen.getByTestId("mode-menu-reload"));
    await flushPanel();
    const calls = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-mode/menu")
    );
    const last = calls[calls.length - 1]!;
    const init = last[1] as RequestInit | undefined;
    expect(init?.method ?? "GET").toBe("GET");
    expect(screen.getByTestId("mode-menu-option-2")).toBeTruthy();
  });

  it("picker：failed → 保留面板 + 显示后端失败原文（用户可重读或重选）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.menuBody = {
      status: "menu",
      options: [{ number: 1, label: "Read Only", highlighted: false }],
    };
    render(<ModeBar session={{ id: "mc-p8" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    routes.menuBody = {
      status: "failed",
      error: "屏上没有权限菜单或确认框——零投递（面板数据可能已过期，请点「重新读取」）",
    };
    fireEvent.click(screen.getByTestId("mode-menu-option-1"));
    const note = await screen.findByTestId("mode-menu-note");
    expect(note.textContent).toContain("零投递");
    // 面板**不收起**（不逼用户重新开菜单）
    expect(screen.getByTestId("mode-menu-panel")).toBeTruthy();
  });

  it("权限组记忆标注：current 有值（上次切换）→ 显示档名 +「（上次切换）」，无未知提示", async () => {
    installFetch();
    routes.mode = codexTwoAxis({ permissionCurrent: "readOnly" });
    render(<ModeBar session={{ id: "mc-t6" }} />);
    await screen.findByTestId("mode-picker-permission-open");
    expect(screen.getByTestId("mode-current-permission").textContent).toBe("只读");
    expect(screen.getByTestId("mode-current-source-permission").textContent).toBe("（上次切换）");
    expect(screen.queryByTestId("mode-unknown-hint-permission")).toBeNull();
    // 权限组不渲染逐档按钮（编号一律来自终端屏读）
    expect(screen.queryByTestId("mode-switch-permission-readOnly")).toBeNull();
  });
});
