import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModeBar from "@/mobile/ModeBar";
import type { SessionModeView } from "@/mobile/api";

// 批次丙 T6 + 丁T4：模式栏。覆盖——
// ① 显示当前档（屏读回显成功时）；② 档未知时**必须**显示人工核对提示（红线 4）；
// ③ unsupported 工具不渲染；④ 切档回执 verified=false → 人工核对文案；
// 丁T4 新增：⑤ 二维家（codex/kimi）两组按钮与各自的当前档；⑥ 单轴家切换钮 + 回显；
// ⑦ 裁6 opencode「默认」（Build 已术语对齐）；⑧ 裁7 退役档不作可点按钮；
// ⑨ 旧后端（无 groups 字段）回落单轴渲染且 POST 不带 group；
// 2026-10-10 用户指令：⑩ codex 模式组三段 [计划][◀▶][操作]（无零投递闸）、
// ⑪ 权限组四档 chips 常驻直选（生效档高亮框）+ picker 兜底入口。

interface Routes {
  mode?: SessionModeView;
  modeStatus?: number;
  modeNetworkFail?: boolean;
  switchBody?: Record<string, unknown>;
  switchStatus?: number;
  /** 2026-09-23 picker：/session-mode/menu 的响应体（open/pick/GET 重读同形） */
  menuBody?: Record<string, unknown>;
  menuStatus?: number;
  /** T5：菜单端点的**按次响应队列**——每次菜单 fetch 先消费一项，取尽后回落
   *  `menuBody` 静态值。confirm-cancelled 的「pick 回执 → 自动重开菜单」在同一条
   *  异步链里连发两次菜单请求，静态单值无法区分两次响应，故加队列。 */
  menuBodyQueue?: Record<string, unknown>[];
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
      const queued = routes.menuBodyQueue?.shift();
      const body = queued ?? routes.menuBody;
      if (routes.menuStatus) {
        return new Response(JSON.stringify(body ?? { error: "internal" }), {
          status: routes.menuStatus,
        });
      }
      return new Response(JSON.stringify(body ?? { status: "none", options: [] }), {
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

/** 丁T4 新后端形态：二维（codex）。2026-09-23 codex 模式切换改造 + 2026-10-10
 *  用户指令改版后：模式组 = 三段 [计划][◀▶][操作]（shift+tab 双向，无零投递闸）；
 *  权限组 = 四档 chips 常驻直选 + 单选面板兜底（picker：后端读回终端菜单选项，
 *  用户点选哪项就敲哪个数字键）。
 *  `modeCurrent` 驱动 toggle 的高亮与翻转方向；`permissionCurrent` 用来测 chips
 *  高亮与「上次切换」记忆标注（null = 无记忆 → 模式未知）。
 *  终审 P1-2 mock 改写：权限组回读源 2026-10-09 起 readback=true（最新回执行），
 *  旧 mock 的 `readback: false` 过期；「上次切换」标注改由 `currentSource` 驱动
 *  （默认：有记忆 = "memory"、无 = "null"；可显式传 "screen" 测屏读来源不标注）。 */
function codexTwoAxis(opts?: {
  modeCurrent?: MamMode;
  permissionCurrent?: MamMode | null;
  permissionCurrentSource?: "screen" | "memory" | "null";
}): SessionModeView {
  const modeCurrent = opts?.modeCurrent ?? "plan";
  const permissionCurrent = opts?.permissionCurrent ?? null;
  const permissionCurrentSource =
    opts?.permissionCurrentSource ?? (permissionCurrent === null ? "null" : "memory");
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
        currentSource: "screen",
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
        readback: true,
        layout: "picker",
        current: permissionCurrent,
        currentLabel: permissionCurrent === null ? null : "只读",
        currentSource: permissionCurrentSource,
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
  it("二维家（codex）：渲染模式组 + 权限组两组，各自显示当前档；权限组 current=null（屏上无回执行）→ 人工核对", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t1" }} />);
    expect(await screen.findByTestId("mode-bar")).toBeTruthy();
    // 结构标记（前端渲染分支的判据）
    expect(screen.getByTestId("mode-bar").getAttribute("data-structure")).toBe("twoAxis");
    // 两组都在
    expect(screen.getByTestId("mode-group-mode")).toBeTruthy();
    expect(screen.getByTestId("mode-group-permission")).toBeTruthy();
    // 模式组：回读命中「计划」（tabs 高亮承载，2026-10-10 用户指令移除文本回显）；
    // 权限组：屏上无回执行（current=null）→ 模式未知 + 人工核对
    expect(
      screen.getByTestId("mode-toggle-label-mode-plan").getAttribute("data-current")
    ).toBe("true");
    expect(screen.getByTestId("mode-unknown-hint-permission").textContent).toContain("人工核对");
    // 组标题可见（两组才显示，帮助用户区分两个轴）
    expect(screen.getByTestId("mode-group-permission").textContent).toContain("权限");
  });

  it("二维家：权限组 = 四档 chips 常驻直选 +「切换权限」picker 兜底入口（2026-10-10）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t2" }} />);
    await screen.findByTestId("mode-bar");
    // 四档 chips 常驻（来自 tiers 里 selectable 档）
    expect(screen.getByTestId("mode-switch-permission-readOnly")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-permission-default")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-permission-acceptEdits")).toBeTruthy();
    expect(screen.getByTestId("mode-switch-permission-bypass")).toBeTruthy();
    // 「切换权限」picker 入口保留为兜底；面板未点开时不出
    expect(screen.getByTestId("mode-picker-permission-open")).toBeTruthy();
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

  it("2026-10-10：codex 模式组 = [计划] [◀▶] [操作] 三段（标签指示器 + 切换钮，无逐档按钮）", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    render(<ModeBar session={{ id: "t4" }} />);
    await screen.findByTestId("mode-bar");
    // 双标签指示器在（计划在左、操作在右）
    expect(screen.getByTestId("mode-toggle-label-mode-plan").textContent).toBe("计划");
    expect(screen.getByTestId("mode-toggle-label-mode-default").textContent).toBe("操作");
    // ◀▶ 切换钮在（旧「计划 ⇄ 操作」单钮文案退役）
    expect(screen.getByTestId("mode-switch-mode-toggle")).toBeTruthy();
    // 不再有逐档直达按钮，也不再有「不可用」说明档
    expect(screen.queryByTestId("mode-switch-mode-default")).toBeNull();
    expect(screen.queryByTestId("mode-switch-mode-plan")).toBeNull();
    expect(screen.queryByTestId("mode-tier-disabled-mode-default")).toBeNull();
  });

  it("2026-10-10：模式组当前档高亮——current=plan 时计划标签 data-current=true、操作标签 false", async () => {
    installFetch();
    routes.mode = codexTwoAxis(); // mode 组 current = plan
    render(<ModeBar session={{ id: "t4-hl" }} />);
    await screen.findByTestId("mode-bar");
    expect(screen.getByTestId("mode-toggle-label-mode-plan").getAttribute("data-current")).toBe(
      "true"
    );
    expect(screen.getByTestId("mode-toggle-label-mode-default").getAttribute("data-current")).toBe(
      "false"
    );
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
  it("当前档高亮：toggle 标签 data-current=当前档；权限组生效档 chips 高亮框选中（memory 来源仍标「上次切换」）", async () => {
    installFetch();
    // 终审 P1-2：标注判据 = currentSource === "memory"（显式传入驱动本用例）
    routes.mode = codexTwoAxis({
      permissionCurrent: "readOnly",
      permissionCurrentSource: "memory",
    });
    render(<ModeBar session={{ id: "s-e3-hl" }} />);
    await screen.findByTestId("mode-picker-permission-open");
    // 模式组双标签：当前档（plan）标签高亮；◀▶ 钮 data-current 承载当前档
    expect(screen.getByTestId("mode-toggle-label-mode-plan").getAttribute("data-current")).toBe(
      "true"
    );
    expect(screen.getByTestId("mode-switch-mode-toggle").getAttribute("data-current")).toBe("plan");
    // 权限组 chips：生效档（只读）高亮框选中（data-current）；其余不选中
    expect(screen.getByTestId("mode-switch-permission-readOnly").getAttribute("data-current")).toBe(
      "true"
    );
    expect(screen.getByTestId("mode-switch-permission-bypass").getAttribute("data-current")).toBe(
      "false"
    );
    // memory 来源仍保留「上次切换」标注（chips 高亮不取代口径声明；档名文本
    // 回显已按 2026-10-10 用户指令移除——高亮即状态）
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

  it("当前档未知（2026-10-10 无零投递闸）：◀▶ 不再禁用，点击照发（target=plan 仅作核验预期兜底）", async () => {
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
    routes.switchBody = { status: "key_sent", verified: true, hint: "已切换（屏读核验命中）" };
    render(<ModeBar session={{ id: "mc-t2b" }} />);
    const toggle = await screen.findByTestId("mode-switch-mode-toggle");
    // 旧语义「未知即禁用（盲按会 50% 误切）」随零投递闸一并退役：动作无条件，
    // 落点由后端屏读核验（前读不可判 → 前端 target 兜底），卡面跟随屏读结果。
    expect((toggle as HTMLButtonElement).disabled).toBe(false);
    expect(toggle.getAttribute("data-current")).toBe("unknown");
    // 双标签在、都不高亮（未知档无从高亮——如实）
    expect(screen.getByTestId("mode-toggle-label-mode-plan").getAttribute("data-current")).toBe(
      "false"
    );
    expect(screen.getByTestId("mode-toggle-label-mode-default").getAttribute("data-current")).toBe(
      "false"
    );
    fireEvent.click(toggle);
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("mode");
    expect(body.target).toBe("plan", "当前档未知 → 兜底 target=plan（仅核验预期用）");
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
    routes.menuBody = {
      status: "done",
      verified: true,
      hint: "终端回执：Permissions updated to Approve for me",
    };
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
    routes.menuBody = {
      status: "done",
      verified: true,
      hint: "终端回执：Permissions updated to Read Only",
    };
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

  it("picker 二阶段：pick 返回 confirm → 面板切为确认框选项（由用户再点，兔维斯 不代按）", async () => {
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

  it("权限组记忆标注：currentSource=memory → 显示档名 +「（上次切换）」+ 生效档 chip 高亮；screen 来源不标注", async () => {
    installFetch();
    routes.mode = codexTwoAxis({
      permissionCurrent: "readOnly",
      permissionCurrentSource: "memory",
    });
    const { unmount } = render(<ModeBar session={{ id: "mc-t6" }} />);
    await screen.findByTestId("mode-picker-permission-open");
    expect(screen.getByTestId("mode-current-source-permission").textContent).toBe("（上次切换）");
    expect(screen.queryByTestId("mode-unknown-hint-permission")).toBeNull();
    // 生效档 chip 高亮（chips 常驻直选，2026-10-10）
    expect(screen.getByTestId("mode-switch-permission-readOnly").getAttribute("data-current")).toBe(
      "true"
    );
    unmount();

    // 终审 P1-2 反向面：屏读来源（screen）= 实时权威 → **不标注**「上次切换」
    // （旧判据 readback:false 在 readback 全开后不可达——此用例同时锁「不误标」）
    installFetch();
    routes.mode = codexTwoAxis({
      permissionCurrent: "readOnly",
      permissionCurrentSource: "screen",
    });
    render(<ModeBar session={{ id: "mc-t6-screen" }} />);
    await screen.findByTestId("mode-picker-permission-open");
    expect(screen.queryByTestId("mode-current-source-permission")).toBeNull();
  });

  it("权限 chips 直选：点「只读」→ POST {group:'permission', target:'readOnly'}；点「自动审批」→ target:'acceptEdits'", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = { status: "key_sent", verified: true, hint: "已切换（屏读核验命中）" };
    render(<ModeBar session={{ id: "mc-chips" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-permission-readOnly"));
    await flushAsync();
    let call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("permission");
    expect(body.target).toBe("readOnly");
    // 第二颗 chip：acceptEdits（走同一 switch 端点 Menu 编排，前端不猜编号）。
    // 第一次切换 verified 后组件重拉过 GET（DOM 重渲染）——用 findBy 重取当前节点。
    fireEvent.click(await screen.findByTestId("mode-switch-permission-acceptEdits"));
    await flushAsync();
    const calls = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    expect(calls.length).toBe(2, "两颗 chip 各发一次 switch");
    const body2 = JSON.parse(String((calls[calls.length - 1]![1] as RequestInit).body));
    expect(body2.target).toBe("acceptEdits");
  });

  it("权限 chips 的「完全信任」仍守二次确认防线：先出确认条，确认后才发 bypass", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = { status: "key_sent", verified: true, hint: null };
    render(<ModeBar session={{ id: "mc-chips-bypass" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-permission-bypass"));
    await flushAsync();
    // 未确认前零请求，确认条先出
    expect(
      fetchMock.mock.calls.filter((c: unknown[]) =>
        String(c[0]).includes("/session-mode/switch")
      )
    ).toHaveLength(0);
    expect(screen.getByTestId("mode-bypass-confirm")).toBeTruthy();
    fireEvent.click(screen.getByTestId("mode-bypass-confirm-yes"));
    await flushAsync();
    const call = fetchMock.mock.calls.find((c: unknown[]) =>
      String(c[0]).includes("/session-mode/switch")
    );
    const body = JSON.parse(String((call![1] as RequestInit).body));
    expect(body.group).toBe("permission");
    expect(body.target).toBe("bypass");
  });
});

// ==== T5（spec §3.1/§3.2/§6-T5）：codex 模式/权限两链的回执消费 ====
// （2026-10-10 用户指令：「已在目标档（零投递）」态删除——点切换必然发键，
// 后端 hint 只剩命中/不符/未生效三态；原零投递回执用例随之删除。）
describe("ModeBar：T5 codex 回执消费（observed / confirm-cancelled）", () => {

  it("codex 模式切换 verified=false 带 observed → 回执原样透出「屏已切换至 X（预期 Y）」，不重复拼接 observed 档名", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    routes.switchBody = {
      status: "key_sent",
      verified: false,
      hint: "屏已切换至 操作（预期 计划）——请人工核对终端",
      observed: "default",
      current: "default",
      currentLabel: "操作",
    };
    render(<ModeBar session={{ id: "t5-obs" }} />);
    fireEvent.click(await screen.findByTestId("mode-switch-mode-toggle"));
    const receipt = await screen.findByTestId("mode-receipt");
    // 整串等值 = 前端只透传未拼接（observed 档名已并入后端 hint，前端不再叠一次）
    expect(receipt.textContent).toBe("屏已切换至 操作（预期 计划）——请人工核对终端");
  });

  it("codex picker confirm-cancelled（Cancel 回菜单）→ 显示 hint + 自动重拉菜单选项表（既有 open 流程），面板回菜单态", async () => {
    installFetch();
    routes.mode = codexTwoAxis();
    // 菜单端点按次响应：open→menu、pick(4)→confirm、pick(2)→confirm-cancelled、
    // 自动重开（open）→menu
    const menuOptions = [
      { number: 1, label: "Read Only", highlighted: false },
      { number: 4, label: "Full Access", highlighted: false },
    ];
    routes.menuBodyQueue = [
      { status: "menu", options: menuOptions },
      {
        status: "confirm",
        options: [
          { number: 1, label: "Yes, continue anyway", highlighted: true },
          { number: 2, label: "Cancel", highlighted: false },
        ],
      },
      { status: "confirm-cancelled", hint: "已取消 Full Access 确认，菜单已回到屏上" },
      { status: "menu", options: menuOptions },
    ];
    render(<ModeBar session={{ id: "t5-cc" }} />);
    fireEvent.click(await screen.findByTestId("mode-picker-permission-open"));
    await flushPanel();
    expect(screen.getByTestId("mode-menu-panel").getAttribute("data-panel")).toBe("menu");
    // 选 Full Access → 二阶段确认框
    fireEvent.click(screen.getByTestId("mode-menu-option-4"));
    await flushPanel();
    expect(screen.getByTestId("mode-menu-panel").getAttribute("data-panel")).toBe("confirm");
    // 点 Cancel（2）→ confirm-cancelled → 前端自动重开菜单
    fireEvent.click(screen.getByTestId("mode-menu-option-2"));
    await flushPanel();
    await flushPanel();
    // 面板回到菜单态（用户不用手点「重新读取」）
    expect(screen.getByTestId("mode-menu-panel").getAttribute("data-panel")).toBe("menu");
    // 取消回执保留为面板 note
    expect(screen.getByTestId("mode-menu-note").textContent).toContain("已取消 Full Access 确认");
    // 重开的选项表渲染出来（第 4 项回来可选）
    expect(screen.getByTestId("mode-menu-option-4")).toBeTruthy();
    // 第 4 次菜单请求 = 自动重开（POST action=open）
    const calls = fetchMock.mock.calls.filter((c: unknown[]) =>
      String(c[0]).includes("/session-mode/menu")
    );
    expect(calls.length).toBe(4);
    const lastBody = JSON.parse(String((calls[calls.length - 1]![1] as RequestInit).body));
    expect(lastBody.action).toBe("open");
  });
});
