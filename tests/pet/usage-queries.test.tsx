// Task 5（计划②）· 用量数据层 `src/lib/query/queries/usage.ts` 的行为锁。
// 纪律（契约 §3 / 计划② §3.15–3.17）：
//  * 三条读查询（dashboard / records / settings）**只读账本、绝不触发扫描**——每个查询用例都断言
//    `invokedCommands()` 里没有 `usage_collect`（§3.15：扫描只此一条命令，且它不进 3 秒轮询路径）；
//  * `useUsageCollect` 挂载即按需采集一次（`force` 恒 `false`，前端不做二次节流），成功后一次失效
//    `["usage"]` 前缀（看板 / 记录页 / 设置全被覆盖），失败只 `console.warn` 记录、不抛给 UI；
//  * 兜底 ticker **必须保留**（后端只有启动后一次性采集 + 最小间隔复用，无周期调度）：间隔取
//    `settings.collectIntervalMin`（默认 10），`enabled=false` / 间隔 ≤ 0 不设，且只在
//    `document.visibilityState === "visible"` 时触发。
// 本文件**不自行 mock `@tauri-apps/api/core`**：`tests/setup.ts` 已把 invoke 接到
// `tests/msw/tauriMocks.ts`（① 的 mock case → ② 同源夹具）；`invokedCommands()` 与失败路径的
// `mam-mock-usage="error"` 开关都从这条链上读，另起一套 mock 会绕开本任务要验的夹具。
import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { tauriInvokeMock } from "../msw/tauriMocks";
import {
  useUsageCollect,
  useUsageDashboardQuery,
  useUsageRecordsQuery,
  useUsageSettingsQuery,
} from "@/lib/query/queries/usage";
import {
  MOCK_USAGE_COLLECTED_AT,
  MOCK_USAGE_TOOL_IDS,
  mockUsageCollect,
  mockUsageDashboard,
  mockUsageSettings,
} from "@/lib/usage/mockFixtures";
import type { UsageFilters, UsageRange, UsageSettings } from "@/types/usage";

const TODAY: UsageRange = { preset: "today" };
const LAST7D: UsageRange = { preset: "last7d" };

// 兜底 ticker 的期望间隔（毫秒）= 夹具 `collectIntervalMin`（10 分钟）；写死常数以免
// 「实现与断言一起改」的循环论证。
const TICK_MS = 10 * 60_000;

/** setup.ts 把 `@tauri-apps/api/core` 的 invoke 接到了这个 vi.fn 上（每用例后清空调用记录） */
function invokedCommands(): string[] {
  return tauriInvokeMock.mock.calls.map((c) => c[0]);
}

/** `usage_collect` 的每次调用 = (命令名, 入参)——用于锁「挂载只采集一次且 force 恒 false」 */
function collectCalls(): Array<[string, unknown]> {
  return tauriInvokeMock.mock.calls.filter(([cmd]) => cmd === "usage_collect") as Array<
    [string, unknown]
  >;
}

/** 每用例独立 cache（照 tests/hooks/useSessions.test.tsx 的范式）；retry=false 保持用例快而确定 */
function createWrapper(
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
) {
  return {
    client,
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  };
}

/** 以预置的设置缓存挂载 `useUsageCollect`（设置查询走同一查询键，预置值覆盖挂载帧） */
function renderCollectWithSettings(settings: UsageSettings) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  client.setQueryData(["usage", "settings"], settings);
  return renderHook(() => useUsageCollect(), { wrapper: createWrapper(client).wrapper });
}

// 采集失败用例要静音 console.warn（否则套件输出被污染），ticker 用例要 spy setInterval；
// 都在用例内登记、用例后还原（不在 setup.ts 之外改全局 mock 语义）。
const restorers: Array<() => void> = [];
afterEach(() => {
  restorers.splice(0).forEach((restore) => restore());
  localStorage.clear();
});

describe("用量数据层（src/lib/query/queries/usage.ts）", () => {
  it('1. useUsageDashboardQuery：读账本出契约数据，键为 ["usage","dashboard",range,groupBy]，绝不触发采集', async () => {
    const { client, wrapper } = createWrapper();
    const { result } = renderHook(() => useUsageDashboardQuery(TODAY, "tool"), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const dash = result.current.data!;
    // hero 口径（契约 §2 / ① parity 同式）= 请求输入 + 产出；环比与采集时刻随同一次查询回来，
    // **不需要第二个查询**（§3.17）。
    expect(dash.hero).toBe(dash.totals.requestTotal + dash.totalsBuckets.output);
    expect(dash.collectedAt).toBe(MOCK_USAGE_COLLECTED_AT);
    expect(dash.compare).not.toBeNull();
    expect(dash.rows.map((r) => r.key)).toEqual([...MOCK_USAGE_TOOL_IDS]);

    // 键形状 `["usage", <域>, ...参数]`：`invalidateQueries(["usage"])` 才能到达本查询
    expect(
      client
        .getQueryCache()
        .getAll()
        .map((q) => q.queryKey)
    ).toEqual([["usage", "dashboard", TODAY, "tool"]]);

    expect(invokedCommands()).toEqual(["usage_dashboard"]);
    expect(invokedCommands()).not.toContain("usage_collect");
  });

  it("2. useUsageRecordsQuery：读账本出卡片（卡内行恒「供应商/模型」），键含 range/groupBy/filters，绝不触发采集", async () => {
    const filters: UsageFilters = { toolIds: ["claude"] };
    const { client, wrapper } = createWrapper();
    const { result } = renderHook(() => useUsageRecordsQuery(LAST7D, "tool", filters), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const records = result.current.data!;
    expect(records.cards.map((c) => c.toolId)).toEqual(["claude"]); // toolIds 筛选生效
    expect(records.cards[0].rows).toHaveLength(15); // 夹具确定值：claude 卡 15 行（覆盖折叠分支）
    // 卡内行维度不含 session_id → 分层算不出 = null，**不得**回退成 false（契约 §2 裁决）
    expect(records.cards[0].rows.every((r) => r.isSubagent === null)).toBe(true);
    expect(records.collectedAt).toBe(MOCK_USAGE_COLLECTED_AT);

    expect(
      client
        .getQueryCache()
        .getAll()
        .map((q) => q.queryKey)
    ).toEqual([["usage", "records", LAST7D, "tool", filters]]);

    expect(invokedCommands()).toEqual(["usage_records"]);
    expect(invokedCommands()).not.toContain("usage_collect");
  });

  it("3. useUsageSettingsQuery：唯一设置入口——两个消费者共享同键同一次请求，返回完整 8 字段，绝不触发采集", async () => {
    const { client, wrapper } = createWrapper();
    const { result } = renderHook(
      () => ({ a: useUsageSettingsQuery(), b: useUsageSettingsQuery() }),
      { wrapper }
    );

    await waitFor(() => expect(result.current.a.isSuccess).toBe(true));
    const settings: UsageSettings = result.current.a.data!;
    expect(Object.keys(settings).sort()).toEqual([
      "collectIntervalMin",
      "detailRetentionDays",
      "enabled",
      "exportPose",
      "exportQuote",
      "miniBarRange",
      "miniBarToolRows",
      "providerMapRules",
    ]);
    expect(settings.collectIntervalMin).toBe(10);
    // 同键 → 同一份缓存对象：别处（Task 6/9/13/14）复用本 hook 即可被 ["usage"] 失效波及
    expect(result.current.b.data).toBe(settings);

    expect(
      client
        .getQueryCache()
        .getAll()
        .map((q) => q.queryKey)
    ).toEqual([["usage", "settings"]]);
    expect(invokedCommands()).toEqual(["usage_get_settings"]); // 两个消费者只发一次请求
    expect(invokedCommands()).not.toContain("usage_collect");
  });

  it('4. useUsageCollect：挂载即采集一次（force=false）、成功后失效 ["usage"]、兜底 ticker 按 collectIntervalMin 且仅可见时触发', async () => {
    const { client, wrapper } = createWrapper();
    // 预置看板缓存（staleTime 内 fresh）→ 挂载本身不发查询；只有「采集成功的失效」会让它重查。
    // 这同时锁住键形状：键写错则挂载即查、失效再查 = 2 次，断言 `1` 必红。
    client.setQueryData(["usage", "dashboard", TODAY, "tool"], mockUsageDashboard(TODAY, "tool"));

    const interval = vi.spyOn(globalThis, "setInterval");
    restorers.push(() => interval.mockRestore());

    const { result, unmount } = renderHook(
      () => ({ dash: useUsageDashboardQuery(TODAY, "tool"), collect: useUsageCollect() }),
      { wrapper }
    );

    await waitFor(() => expect(result.current.collect.lastResult).not.toBeNull());
    // 挂载即按需采集一次，force 恒 false（夹具 durationMs 分叉 force，反向验证参数确实传下去了）
    expect(collectCalls()).toEqual([["usage_collect", { force: false }]]);
    expect(result.current.collect.lastResult).toEqual(mockUsageCollect(false));

    // 成功后失效 ["usage"]：看板查询恰好重查一次（挂载时被 fresh 缓存挡住）
    await waitFor(() =>
      expect(invokedCommands().filter((cmd) => cmd === "usage_dashboard")).toHaveLength(1)
    );

    // 兜底 ticker 必须保留（后端无周期调度）：间隔 = settings.collectIntervalMin，且只注册一条
    const ticks = interval.mock.calls.filter(([, delay]) => delay === TICK_MS);
    expect(ticks).toHaveLength(1);
    const tick = ticks[0][0] as () => void;

    // 不可见（后台/被遮挡）→ 不触发采集
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    restorers.push(() =>
      Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true })
    );
    tick();
    expect(collectCalls()).toHaveLength(1);
    // 可见 → 触发
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    tick();
    await waitFor(() => expect(collectCalls()).toHaveLength(2));

    // 设置驱动 ticker（设置走同一查询键，预置缓存即覆盖挂载帧；断言紧贴挂载帧——采集成功后的
    // 那次失效会让设置查询重查、打回夹具默认值）：
    // ① enabled=false → 不设 ticker；② 间隔非默认值 → 按 settings.collectIntervalMin（7 分钟）注册。
    unmount();
    const ticksBefore = interval.mock.calls.filter(([, delay]) => delay === TICK_MS).length;
    const disabled = renderCollectWithSettings({ ...mockUsageSettings(), enabled: false });
    expect(interval.mock.calls.filter(([, delay]) => delay === TICK_MS)).toHaveLength(ticksBefore);
    await waitFor(() => expect(disabled.result.current.lastResult).not.toBeNull()); // 收尾：等挂载采集跑完
    const custom = renderCollectWithSettings({ ...mockUsageSettings(), collectIntervalMin: 7 });
    expect(interval.mock.calls.filter(([, delay]) => delay === 7 * 60_000)).toHaveLength(1);
    await waitFor(() => expect(custom.result.current.lastResult).not.toBeNull());
  });

  it("5. useUsageCollect 失败路径：只 console.warn 记录、不抛给 UI（错误态由查询命令负责）", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    restorers.push(() => warn.mockRestore());
    // 夹具三态开关（与目验同一个）：用量命令 reject → 采集失败但 UI 不受影响
    localStorage.setItem("mam-mock-usage", "error");

    const { result } = renderHook(() => useUsageCollect(), { wrapper: createWrapper().wrapper });

    await waitFor(() => expect(warn).toHaveBeenCalled());
    // 失败只记录：结构化 { code, detail } 原样进日志；lastResult 保持 null，没有异常逃逸到渲染
    expect(warn.mock.calls[0][0]).toContain("[usage]");
    expect(warn.mock.calls[0][1]).toEqual({
      code: "usage-db-failed",
      detail: "mam-mock-usage=error",
    });
    expect(result.current.lastResult).toBeNull();
    expect(collectCalls()).toEqual([["usage_collect", { force: false }]]);
  });
});
