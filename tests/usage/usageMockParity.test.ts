// 双端 mock parity（Task 20 / 契约 §5「前端类型与双端 mock 的所有权」）：
// * `tests/msw/tauriMocks.ts`（vitest/jsdom 走的那个）与 `src/tauri-mock.ts`（浏览器/Playwright
//   渲染时自安装的那个）必须**逐字同形**——mock 与真实现的漂移、以及两端 mock 之间的漂移，
//   是本任务最阴的一类缺陷（浏览器里看着对、单测里也看着对，真机上形状不一致）。
// * 断言分两层：① **同一份结构断言跑两端**（下面 describe.each）；② 两端样例**深比较**。
// * Rust 真实现的 wire 形状由 `src-tauri/tests/usage_ipc_test.rs` 用同一组键名锁住，
//   本文件只负责 mock 侧（**mock 跑通 ≠ 真跑通**）。
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import path from "node:path";
import { tauriInvokeMock } from "../msw/tauriMocks";
import { KNOWN_USAGE_CODES } from "@/components/usage/usageErrors";
import type {
  UsageCard,
  UsageCollectResult,
  UsageDashboard,
  UsageMetrics,
  UsageRecords,
  UsageRow,
  UsageMetric,
  UsageSettings,
  UsageSourceStatus,
  UsageBuckets,
  WorkSummary,
} from "@/types/usage";
// 副作用导入：非 Tauri 环境（jsdom）下它会把 mock 装到 window.__TAURI_INTERNALS__ 上
import "@/tauri-mock";

type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

const mswInvoke: Invoke = tauriInvokeMock;

const browserInternals = (window as unknown as { __TAURI_INTERNALS__?: { invoke: Invoke } })
  .__TAURI_INTERNALS__;
const browserInvoke: Invoke = (cmd, args) => {
  if (!browserInternals) throw new Error("src/tauri-mock.ts 未在 jsdom 里安装 __TAURI_INTERNALS__");
  return browserInternals.invoke(cmd, args);
};

const SOURCE_IDS = ["claude", "codex", "kimi", "opencode", "workbuddy", "zcode", "dsh"];

/** 契约 §2 的 12 个可得性指标值（类型桥：漏一个值 → 编译错） */
const USAGE_METRICS: readonly UsageMetric[] = [
  "sessions",
  "turn",
  "errorModel",
  "errorTurn",
  "errorTool",
  "interrupted",
  "longestTurn",
  "toolCalls",
  "toolAvgMs",
  "topTool",
  "topToolMs",
  "userEst",
];
const USAGE_COMMANDS = [
  "usage_collect",
  "usage_dashboard",
  "usage_records",
  "usage_export_csv",
  "usage_get_settings",
  "usage_set_settings",
];

/** 键名列表的**类型桥**：`keyof T` 逼 `src/types/usage.ts` 真有这些字段（镜像改名 → 编译错） */
function keysOf<T>(keys: readonly (keyof T)[]): string[] {
  return [...(keys as readonly string[])].sort();
}

function actualKeys(o: unknown): string[] {
  return Object.keys(o as Record<string, unknown>).sort();
}

/** 深拷贝并抹掉时间戳（两端 `Date.now()` 必然不同，其余必须逐字相同） */
function stripTimestamps<T>(v: T): T {
  if (Array.isArray(v)) return v.map(stripTimestamps) as unknown as T;
  if (v && typeof v === "object") {
    return Object.fromEntries(
      Object.entries(v as Record<string, unknown>).map(([k, val]) => [
        k,
        k === "collectedAt" ? 0 : stripTimestamps(val),
      ])
    ) as T;
  }
  return v;
}

const today = () => ({ range: { preset: "today" }, groupBy: "tool", filters: {} });

async function assertUsageCollect(invoke: Invoke) {
  const c = (await invoke("usage_collect", { force: true })) as UsageCollectResult;
  expect(c, "usage_collect 未 mock（undefined = 前端会静默拿到空数据）").toBeTruthy();
  expect(actualKeys(c)).toEqual(
    keysOf<UsageCollectResult>(["collectedAt", "durationMs", "sources", "totalNewRecords"])
  );
  expect(c.sources).toHaveLength(7);
  expect(c.durationMs).toBeGreaterThan(0);
  expect(c.collectedAt).toBeGreaterThan(0);
  const ids: string[] = [];
  for (const s of c.sources) {
    const keys = keysOf<UsageSourceStatus>(["sourceId", "ok", "parsedFiles", "newRecords"]);
    // `errorCode?: string` → 只在失败源出现（skip_serializing_if 的 mock 侧等价物）
    expect(actualKeys(s)).toEqual(s.ok ? keys : [...keys, "errorCode"].sort());
    expect(typeof s.ok).toBe("boolean");
    expect(typeof s.parsedFiles).toBe("number");
    expect(typeof s.newRecords).toBe("number");
    if (!s.ok) {
      // W-23：失败源必须给码，且码要在前端白名单里（否则静默收敛成通用错误）
      expect(KNOWN_USAGE_CODES as readonly string[]).toContain(s.errorCode);
    }
    ids.push(s.sourceId);
  }
  expect([...ids].sort()).toEqual([...SOURCE_IDS].sort());
  expect(c.sources.filter((s) => !s.ok)).toHaveLength(1);
  expect(c.totalNewRecords).toBe(c.sources.reduce((n, s) => n + s.newRecords, 0));
}

function assertBuckets(b: UsageBuckets) {
  expect(actualKeys(b)).toEqual(
    keysOf<UsageBuckets>(["inputFresh", "cacheRead", "cacheWrite", "output"])
  );
  for (const v of Object.values(b)) expect(typeof v).toBe("number");
}

function assertMetrics(m: UsageMetrics) {
  expect(actualKeys(m)).toEqual(
    keysOf<UsageMetrics>(["requestTotal", "cacheHitRate", "userEst", "requests"])
  );
  // userEst 是 `| null`（不可得 ≠ 0）→ 必须**在场**
  expect("userEst" in m).toBe(true);
  expect(m.userEst === null || typeof m.userEst === "number").toBe(true);
}

function assertRow(r: UsageRow) {
  expect(actualKeys(r)).toEqual(
    keysOf<UsageRow>(["key", "label", "buckets", "metrics", "sourceKind", "isSubagent"])
  );
  expect(["measured", "inferred", "unknown"]).toContain(r.sourceKind);
  // isSubagent 是 `boolean | null`（契约 §2 2026-10-03 裁决：日档 / 记录页卡内行**算不出** → null）。
  // `null` 同样必须**在场**（`undefined`/缺键会让下面两式都判假）——与紧邻 `assertMetrics` 的
  // `userEst` 同款写法；**不得**把 `null` 当 `false`（「不知道」≠「非子代理」）。
  expect("isSubagent" in r).toBe(true);
  expect(r.isSubagent === null || typeof r.isSubagent === "boolean").toBe(true);
  assertBuckets(r.buckets);
  assertMetrics(r.metrics);
}

function assertWorkSummary(ws: WorkSummary) {
  expect(actualKeys(ws)).toEqual(
    keysOf<WorkSummary>([
      "sessions",
      "turnsPerTool",
      "errorModel",
      "errorTurn",
      "errorTool",
      "interrupted",
      "toolCalls",
      "toolAvgMs",
      "topTool",
      "topToolMs",
      "longestTurnPerTool",
    ])
  );
  // 契约 `| null` 的**值**位：mock 里这两个工具不可得 → 必须显式 null（不是 undefined/缺键）
  expect(ws.turnsPerTool.workbuddy).toBeNull();
  expect(ws.longestTurnPerTool.workbuddy).toBeNull();
}

async function assertUsageDashboard(invoke: Invoke) {
  const d = (await invoke("usage_dashboard", {
    range: { preset: "today" },
    groupBy: "tool",
  })) as UsageDashboard;
  expect(d, "usage_dashboard 未 mock").toBeTruthy();
  expect(actualKeys(d)).toEqual(
    keysOf<UsageDashboard>([
      "range",
      "groupBy",
      "rows",
      "totals",
      "totalsBuckets",
      "hero",
      "trend",
      "compare",
      "recentSession",
      "workSummary",
      "availability",
      "collectedAt",
    ])
  );
  expect(d.groupBy).toBe("tool");
  // 契约 `| null`：必须**显式**给值。**Minor 2**：`in` / `Object.keys` 对值为 `undefined`
  // 的键同样为真 → 这里直接钉值（mock 的 compare 固定 null；recentSession 为对象或 null）。
  expect(d.compare).toBeNull();
  expect(d.recentSession === null || typeof d.recentSession === "object").toBe(true);
  // hero = totals.requestTotal + totalsBuckets.output（说明书 P1 第 2 条）
  expect(d.hero).toBe(d.totals.requestTotal + d.totalsBuckets.output);
  assertMetrics(d.totals);
  assertBuckets(d.totalsBuckets);
  expect(Array.isArray(d.rows) && d.rows.length > 0).toBe(true);
  d.rows.forEach(assertRow);
  expect(d.trend.length).toBeGreaterThan(0);
  for (const p of d.trend) {
    expect(actualKeys(p)).toEqual(keysOf<typeof p>(["key", "label", "buckets", "metrics"]));
    assertBuckets(p.buckets);
    assertMetrics(p.metrics);
  }
  assertWorkSummary(d.workSummary);
  if (d.recentSession) {
    expect(actualKeys(d.recentSession)).toEqual(
      keysOf<NonNullable<UsageDashboard["recentSession"]>>([
        "sourceId",
        "sessionId",
        "title",
        "buckets",
        "metrics",
      ])
    );
    // title 是 `string | null`（会话维度表可空）
    expect("title" in d.recentSession).toBe(true);
  }
  expect(d.availability.length).toBeGreaterThan(0);
  for (const a of d.availability) {
    expect(USAGE_METRICS).toContain(a.metric);
    expect(typeof a.available).toBe("boolean");
  }
}

async function assertUsageRecords(invoke: Invoke) {
  const r = (await invoke("usage_records", {
    range: { preset: "today" },
    groupBy: "project",
    filters: {},
  })) as UsageRecords;
  expect(r, "usage_records 未 mock").toBeTruthy();
  expect(actualKeys(r)).toEqual(
    keysOf<UsageRecords>(["range", "groupBy", "cards", "availability", "collectedAt"])
  );
  expect(r.groupBy).toBe("project");
  expect(Array.isArray(r.cards) && r.cards.length > 0).toBe(true);
  for (const card of r.cards) {
    expect(actualKeys(card)).toEqual(
      keysOf<UsageCard>(["toolId", "toolLabel", "buckets", "metrics", "rows"])
    );
    assertBuckets(card.buckets);
    assertMetrics(card.metrics);
    expect(card.rows.length).toBeGreaterThan(0);
    card.rows.forEach(assertRow);
  }
}

async function assertUsageExportCsv(invoke: Invoke) {
  const csv = (await invoke("usage_export_csv", today())) as string;
  expect(typeof csv, "usage_export_csv 未 mock").toBe("string");
  const header = csv.split("\n")[0];
  expect(header.split(",")).toEqual([
    "groupKey",
    "label",
    "inputFresh",
    "cacheRead",
    "cacheWrite",
    "output",
    "requestTotal",
    "cacheHitRate",
    "requests",
    "userEst",
    "sourceKind",
  ]);
}

async function assertUsageSettings(invoke: Invoke) {
  const s = (await invoke("usage_get_settings")) as UsageSettings;
  expect(s, "usage_get_settings 未 mock").toBeTruthy();
  expect(actualKeys(s)).toEqual(
    keysOf<UsageSettings>([
      "enabled",
      "miniBarRange",
      "miniBarToolRows",
      "detailRetentionDays",
      "collectIntervalMin",
      "providerMapRules",
      "exportQuote",
      "exportPose",
    ])
  );
  const merged = (await invoke("usage_set_settings", {
    patch: { detailRetentionDays: 30 },
  })) as UsageSettings;
  expect(merged.detailRetentionDays).toBe(30);
  expect(merged.miniBarToolRows).toBe(s.miniBarToolRows); // patch 未提的字段不得被清空
}

describe("usage 命令双端 mock 形状（同一份断言跑两端）", () => {
  it("src/tauri-mock.ts 在 jsdom 里真的自安装了", () => {
    expect(browserInternals).toBeTruthy();
    expect(typeof browserInternals?.invoke).toBe("function");
  });

  it("未 mock 的命令在两端都返回 undefined/空（对照：证明断言不是恒真）", async () => {
    expect(await mswInvoke("usage_nonexistent_cmd")).toBeUndefined();
    expect(await browserInvoke("usage_nonexistent_cmd")).toBeNull();
  });

  for (const [name, invoke] of [
    ["tests/msw", mswInvoke],
    ["src/tauri-mock", browserInvoke],
  ] as const) {
    describe(`端点 ${name}`, () => {
      it("usage_collect：7 源含 1 失败源，ok 与 errorCode 配对", () => assertUsageCollect(invoke));
      it("usage_dashboard：契约 §2 的 12 字段 + 不可得为 null", () => assertUsageDashboard(invoke));
      it("usage_records：卡片维度 + 卡内行形状", () => assertUsageRecords(invoke));
      it("usage_export_csv：11 列表头", () => assertUsageExportCsv(invoke));
      it("usage_get_settings / usage_set_settings：8 字段 + patch 合并", () =>
        assertUsageSettings(invoke));
    });
  }

  it("两端样例逐字同形（仅时间戳可不同）", async () => {
    for (const [cmd, args] of [
      ["usage_collect", { force: true }],
      ["usage_dashboard", { range: { preset: "today" }, groupBy: "tool" }],
      ["usage_records", { range: { preset: "today" }, groupBy: "project", filters: {} }],
      ["usage_export_csv", today()],
      ["usage_get_settings", undefined],
      ["usage_set_settings", { patch: { detailRetentionDays: 30 } }],
    ] as const) {
      const a = await mswInvoke(cmd as string, args as Record<string, unknown> | undefined);
      const b = await browserInvoke(cmd as string, args as Record<string, unknown> | undefined);
      // **Minor 2**：`toEqual` 忽略值为 `undefined` 的属性 → 两端同步改成 undefined/漏写
      // `title: null` 时也会绿，而真机是**显式 null**。`toStrictEqual` 区分 undefined 与缺键。
      expect(stripTimestamps(b), `${cmd} 两端 mock 形状/样例不一致`).toStrictEqual(
        stripTimestamps(a)
      );
    }
  });

  it("两个 mock 文件里每条用量命令**各只有一个 case**（switch 重复 case 会静默遮蔽前者）", () => {
    for (const f of ["tests/msw/tauriMocks.ts", "src/tauri-mock.ts"]) {
      const src = readFileSync(path.join(process.cwd(), f), "utf8");
      for (const cmd of USAGE_COMMANDS) {
        const hits = src.split(`case "${cmd}"`).length - 1;
        expect(hits, `${f} 里 ${cmd} 的 case 数必须是 1（重复 case 静默遮蔽）`).toBe(1);
      }
    }
  });
});
