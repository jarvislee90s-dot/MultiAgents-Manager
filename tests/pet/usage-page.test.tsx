// Task 6（计划②）· 大看板首屏 `src/pages/usage-dashboard.tsx`（8 用例；第 8 条是 2026-10-06
// 用户裁决补的「没量到 token 的窗口不印 0」，第三轮补了 `用户输入(估)` 恢复 KEEP 的两条分支）。
// 判据来源：spec P1（hero + 7 行固定顺序 + 页脚口径）、P8（hero 完整千分位 / 命中率 1 位小数 /
// 环比对比段 / 空态不显示 0 / 三态）、P6（五档 + 自定义 31 天截断常驻提示）、计划② §3 第 5/8/17/18
// 条与 Task 6 步骤 5/6/7/13。
//
// 纪律：
//  * 语言固定 zh：`src/i18n/index.ts` 的 `fallbackLng: "en"` + detector（jsdom navigator 是 en-US）
//    ⇒ 默认英文；全角「（上一周期」模板只有 zh 有（en 是半角 + 前导空格）。
//  * 日期期望值**按运行时今天做日历加法**推算（本地 `setDate`，不写死日期、不用毫秒常数）；
//    时刻只断 `/\d{2}:\d{2}:\d{2}/` 形态 ⇒ 断言与本机时区/日历无关。
//  * 独立 cache 且 `retry: false`：真实 App 的 queryClient 默认 `retry: 2`（1s + 2s 退避），本文件
//    只验页面三态分支，重试策略不是本任务的产物。
//  * `invoke` 走 tests/setup.ts 接进 `@tauri-apps/api/core` 的那个单例（① 的 mock case → ② 同源夹具），
//    不另起 `vi.mock("@tauri-apps/api/core")`（那会绕开夹具）。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";

// jsdom 未实现 matchMedia（theme-provider 的首帧主题判定会用到；与 petSettings 同模式）
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

// WindowFrame/TitleBar 依赖 Tauri 窗口 API，jsdom 下全 mock（与 petSettings/toolManagement 同模式）
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

import i18n from "@/i18n";
import UsageDashboardPage from "@/pages/usage-dashboard";
import { UsageSummary } from "@/components/usage/UsageSummary";
import { tauriInvokeMock } from "../msw/tauriMocks";
import { EM_DASH } from "@/lib/usage/format";
import { mockUsageDashboard } from "@/lib/usage/mockFixtures";

type InvokeCall = [string, Record<string, unknown> | undefined];

/** setup.ts 把 invoke 接到了这个 vi.fn 上（每用例后清空调用记录） */
function callsOf(cmd: string): InvokeCall[] {
  return tauriInvokeMock.mock.calls.filter(([c]) => c === cmd) as InvokeCall[];
}

/** 本地日键 `YYYY-MM-DD`（测试自算，不 import 被验实现；不走 toISOString 的 UTC 日） */
function dayKey(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** 日历加法（与实现同口径：`setDate`，不用毫秒推算——夏令时回拨日不是 86_400_000 ms） */
function addDays(d: Date, n: number): Date {
  const next = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  next.setDate(next.getDate() + n);
  return next;
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return render(<UsageDashboardPage />, { wrapper });
}

beforeAll(async () => {
  await i18n.changeLanguage("zh");
});

describe("usage-page（计划② Task 6：大看板首屏）", () => {
  beforeEach(() => localStorage.clear());

  it("1. hero 千分位 + 7 行固定顺序 + 命中率环比段 + asOf 时刻形态", async () => {
    renderPage();
    // 夹具四工具求和：ΣrequestTotal 1,954,268 + Σoutput 82,713 = 2,036,981（不得改断言口径）
    expect(await screen.findByTestId("usage-hero")).toHaveTextContent("2,036,981");

    expect(
      screen.getAllByTestId(/^usage-grid-/).map((el) => el.getAttribute("data-testid"))
    ).toEqual([
      "usage-grid-userEst",
      "usage-grid-output",
      "usage-grid-requestTotal",
      "usage-grid-cacheRead",
      "usage-grid-hitRate",
      "usage-grid-requests",
      "usage-grid-asOf",
    ]);

    // 常规 token 数值走「万」缩写 + hover 给精确值（spec P8 与 hero 的千分位刻意区分）
    const reqRow = screen.getByTestId("usage-grid-requestTotal");
    expect(reqRow).toHaveTextContent("195.43万");
    expect(reqRow.querySelector("[title='1,954,268']")).toBeTruthy();

    // 命中率行：当前值 1 位小数 + 全角括号的环比段（zh 模板）
    const hitRow = screen.getByTestId("usage-grid-hitRate");
    expect(hitRow).toHaveTextContent("67.4%");
    expect(hitRow).toHaveTextContent("（上一周期");

    expect(screen.getByTestId("usage-grid-asOf")).toHaveTextContent(/\d{2}:\d{2}:\d{2}/);

    // **趋势区也必须有环比**（spec P8：「指标网格各项**与趋势区**都显示对比值」）。
    // 页面接线锁：口径 = 区间 hero（2,036,981）环比上一等长区间
    // （夹具 prevBuckets.output 71,133 + prevMetrics.requestTotal 1,680,670 = 1,751,803）
    // ⇒ 区间合计 （上一周期 175.18万 · +16.3%）。主语（「区间合计」）不能省：这一行紧贴在
    // 「峰值 {{v}}」正下方，没主语会被读成「峰值的上一周期是 175.18万」（数字对、归属错）。
    expect(screen.getByTestId("usage-trend-compare")).toHaveTextContent(
      "区间合计 （上一周期 175.18万 · +16.3%）"
    );
  });

  it("2. 默认档 last7d；切「近 30 天」只把 preset 交给后端重查（前端不自行推算窗口）", async () => {
    renderPage();
    await screen.findByTestId("usage-hero");
    expect(callsOf("usage_dashboard")[0][1]).toEqual({
      range: { preset: "last7d" },
      groupBy: "tool",
    });

    fireEvent.click(screen.getByText("近 30 天"));
    await waitFor(() =>
      expect(
        callsOf("usage_dashboard").some(
          (c) => JSON.stringify(c[1]?.range) === JSON.stringify({ preset: "last30d" })
        )
      ).toBe(true)
    );
    expect(screen.getByText("近 30 天")).toHaveAttribute("aria-pressed", "true");

    // 预设四档**只发 preset**：窗口边界（含上一等长周期）由后端裁定（§3 第 17 条 / 契约要点 1）
    for (const call of callsOf("usage_dashboard")) {
      expect(call[1]?.range).not.toHaveProperty("from");
      expect(call[1]?.range).not.toHaveProperty("to");
    }
  });

  it("3. 自定义跨度 >31 天：夹到 31 天 + 常驻提示 + 以整日区间重查", async () => {
    renderPage();
    await screen.findByTestId("usage-hero");
    fireEvent.click(screen.getByText("自定义"));

    const today = new Date();
    const todayKey = dayKey(today);
    const clampedFrom = dayKey(addDays(today, -30)); // 31 天含首尾 ⇒ from = to - 30
    fireEvent.change(screen.getByTestId("usage-range-from"), {
      target: { value: dayKey(addDays(today, -60)) },
    });

    await waitFor(() =>
      expect(
        callsOf("usage_dashboard").some(
          (c) =>
            JSON.stringify(c[1]?.range) ===
            JSON.stringify({ preset: "custom", from: clampedFrom, to: todayKey })
        )
      ).toBe(true)
    );

    // 输入框回写夹取后的值；提示常驻（不是一帧即逝）
    expect(screen.getByTestId("usage-range-from")).toHaveValue(clampedFrom);
    expect(screen.getByTestId("usage-range-clamped")).toHaveTextContent("已截断为最近 31 天");
    // 起 ≤ 止、止 ≤ 今天（步骤 5 的输入边界）
    expect(screen.getByTestId("usage-range-from")).toHaveAttribute("max", todayKey);
    expect(screen.getByTestId("usage-range-to")).toHaveAttribute("min", clampedFrom);
    expect(screen.getByTestId("usage-range-to")).toHaveAttribute("max", todayKey);

    // 新查询已落定、看板已重渲染（换档 = 新查询键 → 先走加载态）⇒ 提示仍在；再点一次「自定义」
    // 也不清（清只发生在切到别的档）
    await waitFor(() => expect(screen.getByTestId("usage-hero")).toBeTruthy());
    fireEvent.click(screen.getByText("自定义"));
    expect(screen.getByTestId("usage-range-clamped")).toBeTruthy();
  });

  it("4. 错误态：可见错误正文（结构化错误经 usageErrMsg）+ 可重试 + 不出 hero", async () => {
    // 夹具 "error" 档让用量命令 reject 结构化 `{code, detail}`——直印 String(e) 会得到 `[object Object]`。
    // 采集 hook 的失败只 console.warn（不抛给 UI），本用例静音以免污染输出。
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    localStorage.setItem("mam-mock-usage", "error");
    renderPage();

    const alert = await screen.findByTestId("usage-error");
    expect(alert).toHaveAttribute("role", "alert");
    expect(alert).toHaveTextContent("用量数据加载失败");
    expect(alert).toHaveTextContent("mam-mock-usage=error");
    expect(alert.textContent).not.toContain("[object Object]");
    expect(screen.queryByTestId("usage-hero")).toBeNull();

    const before = callsOf("usage_dashboard").length;
    fireEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(callsOf("usage_dashboard").length).toBeGreaterThan(before));
    warn.mockRestore();
  });

  it("5. 空态：显示「暂无数据」而不是 0，且不出 hero / 指标网格", async () => {
    localStorage.setItem("mam-mock-usage", "empty");
    renderPage();
    expect(await screen.findByTestId("usage-empty")).toHaveTextContent("暂无数据");
    expect(screen.queryByTestId("usage-hero")).toBeNull();
    expect(screen.queryByTestId("usage-grid-requests")).toBeNull();
  });

  it("8. **没量到 token 的窗口**（有行、requests ≥ 1、四桶全零）→ 四桶派生的行一律 `—`，真值照常（P8）", async () => {
    // 真机形状（用户 2026-10-06 给出的账本实测）：**真空回合**行——四桶全零、但 requests ≥ 1
    // （15 行 / 50 requests，分布在 claude / opencode / zcode）。用户裁决：「只有是真实的数值，
    // 你才能写它的数值。如果没有产生数值，就不显示了」⇒ **四桶派生的位**出 `—`，**绝不印 0**。
    // 与用例 5 的区别：这里 `isDashboardEmpty` 为**假**（有行）⇒ 页面照常出看板，拦 0 的是
    // `tokenUnmeasured`（只看四个 token 桶的和）。两者**不可互替**。
    localStorage.setItem("mam-mock-usage", "unmeasured");
    const page = renderPage();

    // 看板照常在（不是整块「暂无数据」）；hero 是 token 合计 ⇒ 出 `—` 而不是 0
    expect(await screen.findByTestId("usage-hero")).toHaveTextContent(EM_DASH);
    // 页面没走空态分支（空态分支只渲染 `UsageEmpty`，不会有 hero）：这里的两个 `usage-empty` 都是
    // **卡片自己的**内容区标签——四桶全零 ⇒ `distributionRows` 把零值行丢光（分布卡）+
    // 趋势点全零 ⇒ 整卡走空态（趋势卡，2026-10-06 第二轮）。两卡的空态机制**不同**：前者是被
    // `distributionRows` 丢空、后者是判据 `unmeasured` 直接拦（页面级三态没变，hero 仍在）。
    //
    // ⚠️ **不能同步计数**（2026-10-06 第三轮防 flake）：趋势卡的空态出口是
    // `busy ? UsageLoading : UsageEmpty`，而页面传的是 `busy={dashboard.isFetching}` ——
    // 挂载期的采集 invalidate 会让趋势卡**先出一帧加载态**，那一刻 `usage-empty` 只有 1 个
    // （分布卡那个）。故先 waitFor 等两个都稳定在场，再取快照计数（waitFor 会重试到稳定态）。
    await waitFor(() => expect(screen.getAllByTestId("usage-empty")).toHaveLength(2));
    const emptyNodes = screen.getAllByTestId("usage-empty");
    expect(emptyNodes).toHaveLength(2);
    for (const node of emptyNodes) {
      expect(node).toHaveTextContent("暂无数据");
      expect(node.textContent).not.toMatch(/\d/);
    }

    // **四个由 token 桶派生的行**一律 `—`（产出 / 请求输入(全文累计) / 缓存命中 / 命中率）；
    // `用户输入(估)` **不在其中**（见下）
    for (const key of ["output", "requestTotal", "cacheRead", "hitRate"]) {
      const row = screen.getByTestId(`usage-grid-${key}`);
      expect(row).toHaveTextContent(EM_DASH);
      expect(row.textContent).not.toMatch(/\d/); // 一个数字都不许出现（0 / 0.0% 都不行）
      // 没量到 ⇒ 也**不显示环比段**：`— ↑100.0%` 是自相矛盾的读数（夹具刻意保留非 null 的 compare）
      expect(row.textContent).not.toContain("上一周期");
    }

    // **`用户输入(估)` 恢复 KEEP**（2026-10-06 **第三轮**用户裁决）：它取自**用户打的字**
    // （`collectors/{claude,codex,kimi}.rs` 三源独立估），**不是**四桶的派生量 ⇒「四桶全零」
    // 不蕴含「用户没打字」（真机：`requests=1 | 四桶全零 | user_est = 7`）。夹具这一档刻意给
    // **非零真值 7**（`mockFixtures.ts::zeroTokensOf`）⇒ 本断言**非空洞**：实现若把它一并抹成
    // `—`，这里当场红。它的环比照常算（当前值与上一周期值都是真值）。
    const estRow = screen.getByTestId("usage-grid-userEst");
    expect(estRow).toHaveTextContent("~7");
    expect(estRow.textContent).toMatch(/\d/);
    expect(estRow.textContent).not.toContain(EM_DASH);

    // **计数类真值照常显示**（用户裁决逐字点名：请求次数是量出来的真值，不得被置空）
    const requests = mockUsageDashboard({ preset: "last7d" }, "tool").totals.requests;
    expect(requests).toBeGreaterThan(0);
    expect(screen.getByTestId("usage-grid-requests")).toHaveTextContent(String(requests));
    expect(screen.getByTestId("usage-grid-requests")).not.toHaveTextContent(EM_DASH);
    // **夹具保真锁**（2026-10-06 第二轮）：这一档 `recentSession` 必为 `null`——四桶全零 ⇒ 每行
    // `request_total` 也是 0（`semantics.rs`：四桶之和 ≥ request_total）⇒ 后端候选集为空。
    // 摆一个非 null 的会话会造出后端产生不了的形状，并让「行④不许印 0」变成不可达的空锁。
    expect(mockUsageDashboard({ preset: "last7d" }, "tool").recentSession).toBeNull();

    // 趋势卡：**整卡走空态**——全零趋势也是「没有数据」，`TrendChart` 在 `max === 0` 时会把所有点
    // 画在底线上（一条贴着底边的零线 = 伪造的图）。故 `usage-trend-svg` 必须**不存在**，
    // `usage-trend-peak` / `usage-trend-compare` 也随之不存在（「峰值 —」这种怪值不该存在）。
    expect(screen.queryByTestId("usage-trend-svg")).toBeNull();
    expect(screen.queryByTestId("usage-trend-peak")).toBeNull();
    expect(screen.queryByTestId("usage-trend-compare")).toBeNull();
    // 反向锚：空态文案确实**在**（不能靠"整块没渲染"蒙混过关）
    expect(screen.getAllByTestId("usage-empty").map((n) => n.textContent)).toContain("暂无数据");

    // 页脚/截止时间不受影响：这一档是**采集过但没有 token**，不是「尚未采集」
    expect(screen.getByTestId("usage-grid-asOf")).toHaveTextContent(/\d{2}:\d{2}:\d{2}/);

    // **另一支：`userEst === null` → `—`**（不可得才空；与「没量到 token」是**两条独立**的分支）。
    // 这一支不出现在 mock 夹具的 unmeasured 档里（那一档给的是非零真值，用来证明「真值照常」），
    // 故就地改一个字段直接渲染组件——**不**给夹具加第二套语义（免得同一个模式名有两种含义）。
    // 先卸掉页面，否则同 testid 会命中两个节点（getByTestId 多匹配即抛）。
    page.unmount();
    const t = i18n.t.bind(i18n);
    const base = mockUsageDashboard({ preset: "last7d" }, "tool");
    const nullEst = render(
      <UsageSummary
        dash={{ ...base, totals: { ...base.totals, userEst: null } }}
        retentionDays={null}
        t={t}
      />
    );
    const nullRow = screen.getByTestId("usage-grid-userEst");
    expect(nullRow).toHaveTextContent(EM_DASH);
    expect(nullRow.textContent).not.toMatch(/\d/);
    expect(nullRow.textContent).not.toContain("上一周期"); // 当前值不可得 ⇒ 不做环比
    nullEst.unmount();

    // **反断言（正常窗口逐字不变）**：同一格在「真有 token」的窗口下重渲一次——`用户输入(估)` 的
    // 缩写值与 hover 精确值都**不得**因本轮改动而漂（防「顺手一律置空」的反向；用例 1 只钉了网格
    // 顺序与其它几行，没有单独钉过这一格）。**先摘掉 `unmeasured` 档**：`beforeEach` 只在用例之间
    // 清 localStorage，本用例开头设的模式还在。
    localStorage.removeItem("mam-mock-usage");
    const okView = render(
      <UsageSummary
        dash={mockUsageDashboard({ preset: "last7d" }, "tool")}
        retentionDays={null}
        t={t}
      />
    );
    const okRow = screen.getByTestId("usage-grid-userEst");
    expect(okRow).toHaveTextContent("~1.80万"); // 18,023 → 万缩写必须 2 位小数（本轮未改口径）
    expect(okRow.querySelector("[title='~18,023']")).toBeTruthy();
    expect(okRow.textContent).not.toContain(EM_DASH);
    okView.unmount();
  });

  it("6. 页脚口径常驻 + 保留期由设置项驱动 + collectedAt===0 哨兵在两处可见", async () => {
    const page = renderPage();
    await screen.findByTestId("usage-hero");
    const footer = screen.getByTestId("usage-footer");
    expect(footer).toHaveTextContent("纯 token · 含子代理 · 本地聚合");
    // 设置夹具默认 `detailRetentionDays = 90` ⇒ 页面把设置值传进页脚（设置查询与看板并行，
    // 谁先落地不定 ⇒ 这句要等）
    await waitFor(() =>
      expect(screen.getByTestId("usage-footer")).toHaveTextContent("90 天前无明细")
    );
    expect(screen.getByTestId("usage-footer")).toHaveTextContent(/\d{2}:\d{2}:\d{2}/);
    page.unmount();

    // `collectedAt === 0` 是「尚未采集」哨兵。mock 的 0 哨兵必与空态同现（页面走 UsageEmpty），
    // 故这一支只能直接渲染组件覆盖：asOf 与页脚 freshness 两处都要出哨兵，**不得**出 `—`。
    const dash = {
      ...mockUsageDashboard({ preset: "last7d" }, "tool"),
      compare: null,
      collectedAt: 0,
    };
    const t = i18n.t.bind(i18n);
    const one = render(<UsageSummary dash={dash} retentionDays={30} t={t} />);
    expect(screen.getByTestId("usage-grid-asOf")).toHaveTextContent("尚未采集");
    expect(screen.getByTestId("usage-footer")).toHaveTextContent("尚未采集");
    expect(screen.getByTestId("usage-footer")).toHaveTextContent("30 天前无明细");

    // compare === null ⇒ 只显示当前值：不得出现 0% / NaN / Infinity，也不得出现对比段
    const hitRow = screen.getByTestId("usage-grid-hitRate");
    expect(hitRow).toHaveTextContent("67.4%");
    expect(hitRow).not.toHaveTextContent("上一周期");
    expect(screen.getByTestId("usage-footer").textContent).not.toMatch(/NaN|Infinity|0%/);
    one.unmount();

    // `retentionDays` 为空 ⇒ 不出保留期（步骤 6 的页脚三件套）
    render(<UsageSummary dash={dash} retentionDays={null} t={t} />);
    expect(screen.getByTestId("usage-footer")).not.toHaveTextContent("天前无明细");
  });

  it("7. 打开即按需采集恰一次（force=false）；切档重查不触发扫描", async () => {
    renderPage();
    await screen.findByTestId("usage-hero");

    expect(callsOf("usage_collect")).toHaveLength(1);
    expect(callsOf("usage_collect")[0][1]).toEqual({ force: false });
    // 三条读命令只读账本（§3 第 15 条）
    expect(callsOf("usage_dashboard").length).toBeGreaterThan(0);
    expect(callsOf("usage_get_settings").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByText("近 5 小时"));
    await waitFor(() =>
      expect(
        callsOf("usage_dashboard").some(
          (c) => JSON.stringify(c[1]?.range) === JSON.stringify({ preset: "last5h" })
        )
      ).toBe(true)
    );
    expect(callsOf("usage_collect")).toHaveLength(1);
  });
});
