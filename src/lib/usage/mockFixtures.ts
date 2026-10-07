// 用量域 mock 夹具的**单一事实源**（计划② Task 1）：`src/tauri-mock.ts`（浏览器 / Playwright）
// 与 `tests/msw/tauriMocks.ts`（vitest）共用本模块 —— 两端 mock 的形状从源头一致，
// 「双端漂移」在构造上不可能发生（门禁：tests/usage/usageMockParity.test.ts 用同一份断言跑两端）。
// 与 `src/tauri-mock.ts` 同属 dev-only 资产（本就随主包分发，仅非 Tauri 环境生效）。
// **不得引入 canvas / document 等浏览器 API**：本模块要能在纯 node 上下文里 import（Task 1 判据）；
// localStorage 只在 mockUsageMode() 里读，且做了存在性守卫——无存档（node/SSR）= 正常态。
//
// 目验四态开关（localStorage["mam-mock-usage"]，供目验空态 / 错误态 / 没量到 token 态）：
//   "empty" → 空态（**行**空数组 + 满窗口零值趋势 + collectedAt 0，绝不填零值行；见 zeroTrendOf）
//   "error" → 错误态（用量命令 reject，由两端 mock 的 case 体落地，不在本模块）
//   "unmeasured" → **没量到 token 态**（有行、计数类真值照常，四桶全零；见 zeroTokensOf）
//   其他 / 无存档 → 正常态
import type {
  UsageBuckets,
  UsageCard,
  UsageCollectResult,
  UsageDashboard,
  UsageFilters,
  UsageGroupBy,
  UsageMetrics,
  UsageRange,
  UsageRecords,
  UsageRecordsGroupBy,
  UsageRow,
  UsageSettings,
  UsageSettingsPatch,
  UsageSourceId,
  UsageSourceStatus,
  TrendPoint,
} from "@/types/usage";

/** 固定的「采集时刻」：两端深比较时唯一被抹掉的键（parity 的 stripTimestamps），故两端可同值 */
export const MOCK_USAGE_COLLECTED_AT = Date.parse("2026-10-03T14:37:00+08:00");

/**
 * 夹具的工具池（**4** 个）。契约 §2 的 `UsageGroupBy` 四值里只有这 4 个工具带数字：
 * Task 6 的 hero 期望 `2,036,981 = 1,954,268 + 82,713` 正是这 4 个工具求和
 * （ΣrequestTotal = 1,954,268、Σoutput = 82,713）。
 * **不要**当成 D1 的 7 个采集源：7 源只出现在 `mockUsageCollect` 的 sources 里
 * （记录页的工具 chips 是那 7 源，属 Task 9，与卡片池是不同维度）。
 */
export const MOCK_USAGE_TOOL_IDS = ["claude", "codex", "zcode", "workbuddy"] as const;

/**
 * 供应商不可得时的行名 —— 必须与 Rust 的 `UNKNOWN_PROVIDER_LABEL_KEY` 逐字相同
 * （`src-tauri/src/services/usage/mod.rs`；W5：后端不硬编码中文，前端 `t(label)` 渲染）。
 * 夹具用它而不是 `"unknown"` 字面量，否则「provider 维度必须 `t(label)`」这条纪律测不到。
 */
const UNKNOWN_PROVIDER_LABEL_KEY = "usage.label.unknownProvider";

/**
 * 记录页**卡内行**的模型池：行维度**固定** =「供应商 / 模型」（D6，与卡片分组解耦）。
 * claude 卡刻意给 15 行，用于覆盖「超 12 行折叠为展开其余 N 行」分支（Task 9）。
 */
const MODEL_POOL = [
  "claude-sonnet-4-5",
  "claude-opus-4-1",
  "claude-haiku-4-5",
  "gpt-5-codex",
  "gpt-5",
  "o4-mini",
  "deepseek-v4.1-flash",
  "deepseek-v3.2",
  "glm-5.3-flash",
  "glm-4.6",
  "kimi-k2",
  "qwen3-max",
  "llama-4-405b",
  "mistral-large-3",
  "gemini-3-pro",
];

/** 每**工具**卡的行数（确定性：claude 15 行覆盖折叠分支）；按项目分组时键不命中 → 兜底 2 行 */
const ROW_COUNT: Record<string, number> = { claude: 15, codex: 4, zcode: 3, workbuddy: 1 };

/** 记录页「按项目分组」（D7）的卡片池：key = 小写规范化项目键（D21 契约口径）、label = 原文 */
const PROJECT_POOL: ReadonlyArray<{ key: string; label: string }> = [
  { key: "multiagents-manager", label: "MultiAgents-Manager" },
  { key: "dsh-foxbell-pet", label: "dsh-foxbell-pet" },
  { key: "deepseek-plugins", label: "DeepSeek-plugins" },
  { key: "prompt-lab", label: "prompt-lab" },
];

/**
 * 非 tool 维度的行名池（`mockUsageDashboard` 换键用）：按 `groupBy` 取一组确定性标签。
 * tool 维度**不在此表** —— 它直接用 `MOCK_USAGE_TOOL_IDS`（`label` = 工具 id，见 `rowOf` 注释）。
 */
const GROUP_LABELS: Record<Exclude<UsageGroupBy, "tool">, readonly string[]> = {
  project: ["MultiAgents-Manager", "dsh-foxbell-pet", "DeepSeek-plugins", "prompt-lab"],
  // ⚠️ 真机：供应商**不可得**时 `UsageRow.label` 是 **i18n 键**（Rust 侧 `UNKNOWN_PROVIDER_LABEL_KEY`，
  // 见 `src/types/usage.ts` 的 label 注释）——**不是** `"unknown"` 字面量。夹具必须带这个键，否则
  // 「provider 维度必须 `t(label)`」这条纪律永远测不到；**下标 3 必须仍是这个键**（消费方按 `i` 取）。
  provider: ["anthropic", "openai", "ollama-cloud", UNKNOWN_PROVIDER_LABEL_KEY],
  model: ["claude-sonnet-4-5", "gpt-5-codex", "glm-5.3-flash", "deepseek-v4.1-flash"],
};

/** D1 的 7 个采集源（`mockUsageCollect` 用；与 ① 的 parity 断言 `:97/:114` 同集合） */
const SOURCES: UsageSourceId[] = [
  "claude",
  "codex",
  "kimi",
  "opencode",
  "workbuddy",
  "zcode",
  "dsh",
];

export type MockUsageMode = "ok" | "empty" | "error" | "unmeasured";

/**
 * 夹具四态开关。只有 `"empty"` / `"error"` / `"unmeasured"` 是特殊态；**其他值、无存档、读不到**
 * （node/SSR、隐私模式）一律按正常态 `"ok"` 处理 —— 夹具默认必须是「有数据」。
 */
export function mockUsageMode(): MockUsageMode {
  try {
    if (typeof localStorage === "undefined") return "ok";
    const v = localStorage.getItem("mam-mock-usage");
    if (v === "empty" || v === "error" || v === "unmeasured") return v;
  } catch {
    // 隐私模式下访问 localStorage 会抛：按正常态处理
  }
  return "ok";
}

/**
 * 每工具一行的确定值（测试逐值断言；dashboard 的 totals / hero 由它求和）。
 * **刻意不放展示名**（`"Claude"` 之类）：用量域的展示名唯一来源是 `usageAgentLabel`（§3.13/14），
 * 夹具里再存一份就会诱出「tool 维度 label 写成展示名」的漂移（评审修复轮 1 的缺陷本体）。
 */
const TOOL_FIXTURE: Record<
  string,
  {
    inputFresh: number;
    cacheRead: number;
    cacheWrite: number;
    output: number;
    requests: number;
    userEst: number | null;
    sourceKind: UsageRow["sourceKind"];
  }
> = {
  claude: {
    inputFresh: 344_444,
    cacheRead: 890_123,
    cacheWrite: 12_000,
    output: 45_678,
    requests: 321,
    userEst: 12_345,
    sourceKind: "inferred",
  },
  codex: {
    inputFresh: 156_789,
    cacheRead: 300_000,
    cacheWrite: 4_000,
    output: 23_456,
    requests: 88,
    userEst: 5_678,
    sourceKind: "inferred",
  },
  zcode: {
    inputFresh: 114_567,
    cacheRead: 120_000,
    cacheWrite: 0,
    output: 12_345,
    requests: 45,
    // zcode 拿不到用户文本 → userEst 为 null（不可得 ≠ 0，契约 §3 要点 2）
    userEst: null,
    sourceKind: "measured",
  },
  workbuddy: {
    inputFresh: 4_345,
    cacheRead: 8_000,
    cacheWrite: 0,
    output: 1_234,
    requests: 6,
    userEst: null,
    sourceKind: "unknown",
  },
};

/** 空态哨兵值（绝不拿它冒充「有数据」） */
const zeroBuckets = (): UsageBuckets => ({ inputFresh: 0, cacheRead: 0, cacheWrite: 0, output: 0 });
const zeroMetrics = (): UsageMetrics => ({
  requestTotal: 0,
  cacheHitRate: 0,
  userEst: null,
  requests: 0,
});

/**
 * 「没量到 token」的 metrics 清零：**token 派生位**（`requestTotal` / `cacheHitRate`）归零，
 * `requests` 等**计数类原样保留**；`userEst` **默认也原样保留** —— 它是从用户打的字估出来的
 * KEEP 类（理由见 `zeroTokensOf` 的注释），只有调用方**显式**传值时才会被改写
 * （看板档统一给非零真值 `7`；记录页卡档不动它）。
 */
function blankMetrics(m: UsageMetrics, userEst: number | null = m.userEst): UsageMetrics {
  return { ...m, requestTotal: 0, cacheHitRate: 0, userEst };
}

const sumBuckets = (list: UsageBuckets[]): UsageBuckets =>
  list.reduce(
    (a, b) => ({
      inputFresh: a.inputFresh + b.inputFresh,
      cacheRead: a.cacheRead + b.cacheRead,
      cacheWrite: a.cacheWrite + b.cacheWrite,
      output: a.output + b.output,
    }),
    zeroBuckets()
  );

/** 小时档 = last5h / today（契约 §2 2026-10-03 裁决：只有小时档算得出 `isSubagent`） */
const isHourTier = (range: UsageRange): boolean =>
  range.preset === "last5h" || range.preset === "today";

/**
 * 每工具一行。`isSubagent` 由调用方给：
 * dashboard **小时档**给真值（workbuddy 行模拟「只由子代理会话贡献」，供目验分层标记）、
 * **日档**与**记录页卡内行**算不出（行维度不带 `session_id`）→ 一律 `null`，**绝不写 `false`**。
 *
 * ⚠️ `label` 与 `key` **都是工具 id**：真机 `group_of`（`src-tauri/src/services/usage/query.rs`）的
 * `UsageGroupBy::Tool => (row.source_id.clone(), row.source_id.clone())` —— 大看板与记录页共用同一
 * 函数，两侧口径相同。**展示名不在这里**：由前端 `usageAgentLabel(label)`（= `AGENT_BADGE[id].label`）
 * 解析（§3.13/14：用量域的展示名只有这一个来源）。夹具若写成 `"Claude"` 这类展示名，
 * `usageAgentLabel("Claude")` 查不到键 → 回 `UNKNOWN_TOKEN`（`"?"`），而门禁不会红、真机不可达。
 */
function rowOf(toolId: string, isSubagent: boolean | null): UsageRow {
  const f = TOOL_FIXTURE[toolId];
  const buckets: UsageBuckets = {
    inputFresh: f.inputFresh,
    cacheRead: f.cacheRead,
    cacheWrite: f.cacheWrite,
    output: f.output,
  };
  const requestTotal = f.inputFresh + f.cacheRead + f.cacheWrite;
  return {
    key: toolId,
    label: toolId,
    buckets,
    metrics: {
      requestTotal,
      cacheHitRate: requestTotal > 0 ? f.cacheRead / requestTotal : 0,
      userEst: f.userEst,
      requests: f.requests,
    },
    sourceKind: f.sourceKind,
    isSubagent,
  };
}

/** 确定性趋势点：5h 档 5 个小时桶；当日档 9 个小时桶（06:00–14:00）；日档 n 个日桶 */
function trendOf(range: UsageRange): TrendPoint[] {
  const mk = (key: string, label: string, i: number): TrendPoint => {
    const inputFresh = 12_000 + ((i * 7_919) % 40_000);
    const cacheRead = 30_000 + ((i * 4_421) % 90_000);
    const cacheWrite = (i % 3) * 1_000;
    const output = 2_000 + ((i * 1_013) % 9_000);
    const buckets: UsageBuckets = { inputFresh, cacheRead, cacheWrite, output };
    const requestTotal = inputFresh + cacheRead + cacheWrite;
    return {
      key,
      label,
      buckets,
      metrics: {
        requestTotal,
        cacheHitRate: requestTotal > 0 ? cacheRead / requestTotal : 0,
        userEst: 300 + ((i * 137) % 900),
        requests: 4 + (i % 11),
      },
    };
  };
  if (range.preset === "last5h") {
    const hours = [9, 10, 11, 12, 13];
    return hours.map((h, i) =>
      mk(`2026-10-03T${String(h).padStart(2, "0")}`, `${String(h).padStart(2, "0")}:00`, i)
    );
  }
  if (range.preset === "today") {
    return Array.from({ length: 9 }, (_, i) => {
      const h = 6 + i;
      return mk(`2026-10-03T${String(h).padStart(2, "0")}`, `${String(h).padStart(2, "0")}:00`, i);
    });
  }
  const days = range.preset === "last7d" ? 7 : range.preset === "last30d" ? 30 : 31;
  return Array.from({ length: days }, (_, i) => {
    // ⚠️ 必须用**本地日历**推算（`new Date(2026, 9, 3)` + `setDate`），不要写成
    // `Date.parse("2026-10-03T00:00:00+08:00") - n * 86_400_000`：后者是绝对时刻，在非 +08:00
    // 的机器上经 getFullYear/getMonth/getDate 取出的会是**另一天**，让依赖 trend key 的断言
    // （跨天标签、复制文本首行、分享图头部）随机器时区变化。本地日历推算在任意时区都产出
    // 2026-09-27 … 2026-10-03，夹具因此与时区无关。
    const d = new Date(2026, 9, 3);
    d.setDate(d.getDate() - (days - 1 - i));
    const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    return mk(key, `${d.getMonth() + 1}/${d.getDate()}`, i);
  });
}

/**
 * 空窗口趋势：后端**恒按窗口枚举桶**（`src-tauri/src/services/usage/query.rs:507-524` 在
 * `resolved.cur.keys` 上逐桶 `aggregate`），空账本也出满窗口的点、只是每点全零
 * （同文件 `:1879-1915` 的断言 `assert_eq!(d.trend.len(), 13, "空数据也要出满窗口的桶")`，
 * 并注明「UI 自己判『暂无数据』」）。
 *
 * 夹具**必须**照此保真：早前的 `trend: []` 是与真机不符的旧形状——它让空态判定在夹具上
 * 「碰巧成立」（旧判据含 `trend.length === 0`）而掩盖了真机永不可达（计划② Task 15 最终修复波）。
 * 键 / 标签 / 点数全部复用 `trendOf`（只清值），故形状随档位一致，不会与真机二次漂移。
 */
function zeroTrendOf(range: UsageRange): TrendPoint[] {
  return trendOf(range).map((p) => ({
    key: p.key,
    label: p.label,
    buckets: zeroBuckets(),
    metrics: zeroMetrics(),
  }));
}

/**
 * 「**没量到 token**」形态（2026-10-06 用户裁决的目验开关 `mam-mock-usage = "unmeasured"`）。
 *
 * 真机形状 = **真空回合**：窗口里**有行**、计数类真值（requests / 会话 / turn / 报错 / 工具调用）
 * 照常有值，但**四个 token 桶全零**（用户给出的账本实测：15 行 / 50 requests，分布在
 * claude / opencode / zcode）。用户裁决：「只有是真实的数值，你才能写它的数值。如果没有产生数值，
 * 就不显示了」⇒ 这种窗口里 token 位一律 `EM_DASH`，**绝不印 0**。
 *
 * ⚠️ 与 `"empty"` 的区别必须看清（两者都由同一套 UI 判据拦住，但**判据不是同一条**）：
 *   * `"empty"`：一行都没有 ⇒ `isDashboardEmpty` 为**真** ⇒ 页面/浮窗走「暂无数据」整块空态；
 *   * 本档：**有行** ⇒ `isDashboardEmpty` 为**假**（页面照常出看板），拦 0 的是
 *     `tokenUnmeasured`（`range.ts`）——它只看四个 token 桶的和。
 *
 * `compare` 刻意**保留**（上一周期当然可以有数据）：这样「当前值没量到 ⇒ 不显示环比段」这条
 * 也能被用例锁住（否则会印出 `— ↑100.0%` 这种自相矛盾的读数）。
 */
function zeroTokensOf(d: UsageDashboard): UsageDashboard {
  // metrics 一律走 `blankMetrics` 清零；`userEst` **显式**给**非零真值 7**（不是 0、更不是 null）：
  // 它是从**用户打的字**估出来的（`collectors/{claude,codex,kimi}.rs` 三源独立估），**不是**四个
  // token 桶的派生量 ——「四桶全零」并不蕴含「用户没打字」。真机账本里就有这种行：`claude |
  // requests=1 | 四桶全零 | user_est = 7`（15 条真空回合行里有 4 条带真值）。
  //
  // 2026-10-06 **第三轮**用户裁决：`userEst` 属 KEEP 类，**不得**被「没量到 token」置空；
  // 这里刻意给**非零真值 7**（早先写 0 是为了证明「拦住它的是判据而不是 `null` 分支」，
  // 那个理由随本轮裁决作废）⇒ 用例现在能证明的是「**真值照常显示**」。
  // 「不可得 ⇒ `—`」那一支由用例**本地**构造（`userEst: null`）覆盖，不靠本档。
  const blank = (m: UsageMetrics) => blankMetrics(m, 7);
  return {
    ...d,
    rows: d.rows.map((r) => ({
      ...r,
      buckets: zeroBuckets(),
      metrics: blank(r.metrics), // requests 等计数保留真值
    })),
    totalsBuckets: zeroBuckets(),
    totals: blank(d.totals),
    hero: 0,
    trend: d.trend.map((p) => ({
      ...p,
      buckets: zeroBuckets(),
      metrics: blank(p.metrics),
    })),
    recentSession: null,
    // ⚠️ **必须是 `null`，不许"保留一个非 null 的会话再把它的值清零"**（2026-10-06 第二轮保真修复）：
    // 两种语义下都有恒等式 **四桶之和 ≥ `request_total`**（`semantics.rs`：Exclusive 下
    // `request_total = input + cache_read + cache_write`、和 = 它 + `output`；Subset 下
    // `request_total = input_raw = input_fresh + cache_read`、和 = 它 + `cache_write` + `output`）
    // ⇒ **四桶全零 ⇒ 每一行 `request_total` 都是 0** ⇒ 后端 `query.rs::recent_session_with` 的候选集
    // （要求 `request_total > 0`）**为空** ⇒ 真机在这一档**必给 `null`**。
    // 造一个非 null 的会话既违反 `recentSession.requestTotal ≤ totals.requestTotal`，也会让
    // 「浮窗行④的 token 位不许印 0」那条门控在真机上**永不可达**（= 空锁）。故这里与真机对齐。
  };
}

export function mockUsageCollect(force: boolean): UsageCollectResult {
  // ⚠️ 三条与 ① 的 parity 测试（`tests/usage/usageMockParity.test.ts:91-117`）逐字对齐的纪律：
  //  ① 成功源**不得出现 `errorCode` 键**（写 `errorCode: undefined` 同样会建键 → 断言必红）；
  //  ② 失败源的码必须在前端白名单 `KNOWN_USAGE_CODES` 内（用 ① 真实 mock 同款的 code）;
  //  ③ `totalNewRecords` **必须等于各源 `newRecords` 之和**（写死常数必红）。
  //  另：`durationMs` 分叉 force，供「前端确实把 force 传下来了」这类行为断言。
  const sources: UsageSourceStatus[] = SOURCES.map((sourceId, i) => {
    const base = { sourceId, parsedFiles: i === 1 ? 425 : 17, newRecords: i === 1 ? 35_935 : 120 };
    // 第 6 个源（zcode）固定失败：`ok=false` + 白名单内的码（W-23 失败源夹具）
    return i === 5
      ? { ...base, ok: false, errorCode: "usage-source-db-open" }
      : { ...base, ok: true };
  });
  return {
    collectedAt: MOCK_USAGE_COLLECTED_AT,
    durationMs: force ? 4_412 : 12,
    sources,
    totalNewRecords: sources.reduce((n, s) => n + s.newRecords, 0),
  };
}

export function mockUsageDashboard(range: UsageRange, groupBy: UsageGroupBy): UsageDashboard {
  const mode = mockUsageMode();
  if (mode === "empty") {
    // 空态：**行**一律空数组 + `collectedAt: 0` 哨兵（UI 显示「尚未采集」），**绝不填 0 值行**。
    // ⚠️ 趋势**不是**空数组：后端恒出满窗口的零值桶（见 `zeroTrendOf` 注释），空态判据靠
    // 「零行」而不是「零趋势点」（`isDashboardEmpty`）。
    return {
      range,
      groupBy,
      rows: [],
      totals: zeroMetrics(),
      totalsBuckets: zeroBuckets(),
      hero: 0,
      trend: zeroTrendOf(range),
      compare: null,
      recentSession: null,
      workSummary: {
        sessions: null,
        turnsPerTool: {},
        errorModel: null,
        errorTurn: null,
        errorTool: null,
        interrupted: null,
        toolCalls: null,
        toolAvgMs: null,
        topTool: null,
        topToolMs: null,
        longestTurnPerTool: {},
      },
      availability: [
        { metric: "turn", available: false, reason: "mock-empty" },
        { metric: "toolCalls", available: false, reason: "mock-empty" },
      ],
      collectedAt: 0,
    };
  }
  const hourTier = isHourTier(range);
  // 行档位与真机一致：小时档给真值、日档该位算不出 → null（见 rowOf 注释）
  //
  // ⚠️ `recentSession` 的**档位分叉**（Task 15 收口，此前是本夹具登记在案的保真缺口）：
  // 真机 `query.rs::recent_session_with` 的 `RangeGranularity::Day` 分支**直接返回 `None`**
  // ——日聚合行不带 `session_id`，照旧聚合会得到一个 `buckets` 全 0 的 `recentSession`，
  // 显示成「本会话 0」这种**看起来像真实测量值的假数字**。故日档必须给 `null`
  // （契约 `RecentSessionUsage | null` 内的正确表达），浮窗第 ④ 行随之必然空态。
  const toolRows = MOCK_USAGE_TOOL_IDS.map((toolId) =>
    rowOf(toolId, hourTier ? toolId === "workbuddy" : null)
  );
  const buckets = sumBuckets(toolRows.map((r) => r.buckets));
  const requestTotal = toolRows.reduce((s, r) => s + r.metrics.requestTotal, 0);
  const totals: UsageMetrics = {
    requestTotal,
    cacheHitRate: requestTotal > 0 ? buckets.cacheRead / requestTotal : 0,
    userEst: 18_023,
    requests: toolRows.reduce((s, r) => s + r.metrics.requests, 0),
  };
  // groupBy 非 tool 时用同一批数值换键，保证四维切换都有行可渲染（形状正确优先于语义精确）
  const rows =
    groupBy === "tool"
      ? toolRows
      : toolRows.map((r, i) => ({
          ...r,
          key: `${groupBy}-${i}`,
          label: GROUP_LABELS[groupBy][i],
        }));
  const dash: UsageDashboard = {
    range,
    groupBy,
    rows,
    totals,
    totalsBuckets: buckets,
    hero: totals.requestTotal + buckets.output,
    trend: trendOf(range),
    // 环比：上一周期完整聚合（差值全由前端算，契约 §3 要点 1 只发一次查询）
    compare: {
      prevBuckets: {
        inputFresh: Math.round(buckets.inputFresh * 0.86),
        cacheRead: Math.round(buckets.cacheRead * 0.86 * 0.712),
        cacheWrite: Math.round(buckets.cacheWrite * 0.86),
        output: Math.round(buckets.output * 0.86),
      },
      prevMetrics: {
        requestTotal: Math.round(requestTotal * 0.86),
        cacheHitRate: 0.712,
        userEst: 15_400,
        requests: Math.round(totals.requests * 0.86),
      },
    },
    recentSession: hourTier
      ? {
          sourceId: "claude",
          sessionId: "sess-recent-1",
          title: "演示会话",
          buckets: { inputFresh: 12_000, cacheRead: 45_000, cacheWrite: 0, output: 3_200 },
          metrics: {
            requestTotal: 57_000,
            cacheHitRate: 45_000 / 57_000,
            userEst: 640,
            requests: 12,
          },
        }
      : // 日档（last7d / last30d / custom）：真机恒 `null`（见函数头注释）
        null,
    workSummary: {
      sessions: 42,
      // D16：workbuddy 无回合概念 → null（UI 必须显示空态而不是 0）
      turnsPerTool: { claude: 64, codex: 3_189, zcode: 1_207, workbuddy: null },
      errorModel: 21,
      errorTurn: 8,
      // opencode 工具级不可得 → 契约层用统一数字示例，逐源缺失由 availability.perSource 表达
      errorTool: 343,
      interrupted: 344,
      toolCalls: 41_412,
      toolAvgMs: 79,
      topTool: { name: "Bash", count: 12_004 },
      topToolMs: { name: "Bash", ms: 1_864_000 },
      longestTurnPerTool: {
        claude: { p50: 24_400, max: 4_247_097 },
        codex: { p50: 79_000, max: 33_637_458 },
        zcode: { p50: 310_607, max: 50_311_958 },
        workbuddy: null,
      },
    },
    availability: [
      // 指标级 available=false 表示「所有源都不可得」（本机实测无此情形，保留为防御分支，
      // 由 Task 8 的定向单测覆盖）；逐源不可得一律用 perSource 表达（D16/D19）。
      // ⚠️ **逐源缺口照 `src-tauri/src/services/usage/caps.rs` 的真值表**（修复轮 2，B-5）：
      //   真机 `availability_table()` 的 `per` 闭包对 `UsageSourceId::ALL` **全量 7 源**建映射、
      //   且挂在**每一个**指标上（`query.rs:362-375`）⇒ **四层报错行都有缺口**：
      //     errorModel ← workbuddy.error_model=false；errorTurn ← claude + workbuddy.error_turn=false；
      //     errorTool ← opencode.error_tool=false；interrupted ← kimi.interrupted=false。
      //   本夹具照仓内既有约定**只列值为 `false` 的源**（不铺 7 源全量映射）。
      { metric: "turn", available: true, perSource: { workbuddy: false } },
      { metric: "longestTurn", available: true, perSource: { workbuddy: false } },
      { metric: "userEst", available: true, perSource: { zcode: false, workbuddy: false } },
      { metric: "toolCalls", available: true },
      { metric: "errorModel", available: true, perSource: { workbuddy: false } },
      { metric: "errorTurn", available: true, perSource: { claude: false, workbuddy: false } },
      { metric: "errorTool", available: true, perSource: { opencode: false } },
      { metric: "interrupted", available: true, perSource: { kimi: false } },
    ],
    collectedAt: MOCK_USAGE_COLLECTED_AT,
  };
  // 目验「没量到 token」态（见 `zeroTokensOf` 的长注释）：**有行**、计数类真值照常，四桶全零。
  return mode === "unmeasured" ? zeroTokensOf(dash) : dash;
}

/**
 * 「没量到 token」里**一张 `UsageCard`**（记录页目验态；与 `zeroTokensOf` 同源）：
 * 卡片与**卡内行**的 token 数字清零，**计数类保留**（`requests` / `userEst` 不动——它们是真值）。
 *
 * 为什么记录页要**逐卡**而不是用窗口级判据：记录页拿到的是「每工具/每项目一卡」，
 * 一行都没有的卡与「有行但没量到 token」的卡是两回事（后者真机可达：某工具只落了**真空回合**行）。
 * 两个判据见 `range.ts::{tokenUnmeasured, cardTokenUnmeasured}`。
 */
function zeroCardTokens(card: UsageCard): UsageCard {
  const blank = <T extends { buckets: UsageBuckets; metrics: UsageMetrics }>(x: T): T => ({
    ...x,
    buckets: zeroBuckets(),
    // `userEst` 不传 ⇒ 原样保留（卡档不统一改写成 7，见 `blankMetrics` 的注释）
    metrics: blankMetrics(x.metrics),
  });
  return { ...blank(card), rows: card.rows.map(blank) };
}

export function mockUsageRecords(
  range: UsageRange,
  groupBy: UsageRecordsGroupBy,
  filters: UsageFilters
): UsageRecords {
  // 卡片分组 = groupBy（D7）："tool" → 每工具一卡（D6 默认）；"project" → 每项目一卡。
  // 契约 `UsageCard` 的字段名沿用 toolId / toolLabel —— 语义是**当前卡片分组键**的 key / 显示名。
  // 卡内每行**固定**为「供应商 / 模型」组合（D6）：与卡片分组解耦，故行标签恒取 MODEL_POOL。
  // 空态必须**整条记录也为空**：只让 dashboard 空、卡片池照旧非零，会让记录页的
  // `cards.length === 0` 分支永远不成立 → 空态永不存在（Task 9 的空态用例必红）。
  // 契约口径：无数据一律空数组 + availability 说明，绝不填 0（与 mockUsageDashboard 空态同源）。
  const mode = mockUsageMode();
  if (mode === "empty") {
    return { range, groupBy, cards: [], availability: [], collectedAt: 0 };
  }
  const dash = mockUsageDashboard(range, groupBy);
  // toolIds 筛选只在「按工具」分组时决定卡片集合；「按项目」分组时卡片集合由项目池决定
  // （真实后端会按筛选后的记录重算项目卡片，mock 不做二次模拟——口径由联调清单目验）
  const keep = (toolId: string) =>
    !filters.toolIds || filters.toolIds.length === 0 || filters.toolIds.includes(toolId);
  const groups =
    groupBy === "project"
      ? PROJECT_POOL.map((p, i) => ({
          key: p.key,
          label: p.label,
          base: rowOf(MOCK_USAGE_TOOL_IDS[i % MOCK_USAGE_TOOL_IDS.length], null),
        }))
      : MOCK_USAGE_TOOL_IDS.filter(keep).map((toolId) => ({
          key: toolId,
          // ⚠️ 真机 `groupBy="tool"` 时 **`toolLabel` = 工具 id**（`query.rs` 的
          // `(r.source_id.clone(), r.source_id.clone())`，展示名由前端 `usageAgentLabel` 解析）
          // ——夹具必须同样是 id，否则「卡片标题走工具展示名」这条纪律测不到（Task 9）。
          label: toolId,
          base: rowOf(toolId, null),
        }));
  const cards: UsageCard[] = groups.map(({ key, label, base }) => {
    const n = ROW_COUNT[key] ?? (groupBy === "project" ? 2 : 1);
    // 份额按等差数列归一（sum(share) = 1），逐行值确定性可断言
    const rows: UsageRow[] = Array.from({ length: n }, (_, i) => {
      const share = (n - i) / ((n * (n + 1)) / 2);
      return {
        key: `${key}:${i}`,
        label: MODEL_POOL[i % MODEL_POOL.length],
        buckets: {
          inputFresh: Math.round(base.buckets.inputFresh * share),
          cacheRead: Math.round(base.buckets.cacheRead * share),
          cacheWrite: 0,
          output: Math.round(base.buckets.output * share),
        },
        metrics: {
          requestTotal: Math.round(base.metrics.requestTotal * share),
          cacheHitRate: base.metrics.cacheHitRate,
          userEst: base.metrics.userEst,
          requests: Math.max(1, Math.round(base.metrics.requests * share)),
        },
        sourceKind: base.sourceKind,
        // ⚠️ 记录页**卡内行**不带 `session_id` → `isSubagent` **两档都是 `null`**
        //（契约 §2 2026-10-03 裁决：算不出 = null；**不得**回退成 `false` 的「非子代理」语义）。
        // D17 的分层只在 dashboard 行与 `workSummary` 计数类上生效。
        isSubagent: null,
      };
    });
    return {
      toolId: key,
      toolLabel: label,
      buckets: base.buckets,
      metrics: base.metrics,
      rows,
    };
  });
  return {
    range,
    groupBy,
    // 目验「没量到 token」态（与 `zeroTokensOf` 同源，作用在卡片上）：卡片与**卡内行**的 token 数字
    // 清零、**计数类保留**（`requests` / `userEst` 不动——它们是量出来的真值）⇒ 记录页按**逐卡**判据
    // 把 token 位换成 `EM_DASH`（见 `range.ts::cardTokenUnmeasured`：窗口级判据在记录页不适用）。
    cards: mode === "unmeasured" ? cards.map(zeroCardTokens) : cards,
    availability: dash.availability,
    collectedAt: dash.collectedAt,
  };
}

/** CSV 文本夹具：形状与后端 `query.rs` 的 `CSV_HEADER` 约定一致，不含任何会话正文 */
export function mockUsageCsv(
  range: UsageRange,
  groupBy: UsageGroupBy,
  _filters: UsageFilters
): string {
  // ⚠️ 列名与列序**逐字对齐真实后端**（`src-tauri/src/services/usage/query.rs` 的 `CSV_HEADER`，
  // 11 列含 `userEst`，末列 `sourceKind`）：行尾换行、命中率 `{:.6}`、`userEst` 不可得 = **空单元格**、
  // 供应商不可得的 label = **空串**（后端把 i18n 键还原成空）。① 的 parity 测试把表头逐字钉死，
  // 错一处必红。`filters` 未参与构造：mock 不二次模拟筛选（与真机差异由联调目验覆盖）。
  const dash = mockUsageDashboard(range, groupBy);
  const head = [
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
  ].join(",");
  const lines = dash.rows.map((r) =>
    [
      csvEscape(r.key),
      csvEscape(r.label === UNKNOWN_PROVIDER_LABEL_KEY ? "" : r.label),
      r.buckets.inputFresh,
      r.buckets.cacheRead,
      r.buckets.cacheWrite,
      r.buckets.output,
      r.metrics.requestTotal,
      r.metrics.cacheHitRate.toFixed(6),
      r.metrics.requests,
      r.metrics.userEst === null ? "" : String(r.metrics.userEst),
      r.sourceKind,
    ].join(",")
  );
  return head + "\n" + lines.map((l) => l + "\n").join("");
}

/** RFC 4180 最小转义（与后端 `csv_escape` 同判据：含 `,` `"` `\n` `\r` 时加引号并把 `"` 翻倍） */
function csvEscape(s: string): string {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

/** 设置项夹具（Rust `UsageSettings::default()` 同值：3 / 90 / 10 / today / random） */
const SETTINGS_DEFAULTS: UsageSettings = {
  enabled: true,
  miniBarRange: "today",
  miniBarToolRows: 3,
  detailRetentionDays: 90,
  collectIntervalMin: 10,
  providerMapRules: "",
  exportQuote: "",
  exportPose: "random",
};

/** 合并语义与后端 `merge_patch` 一致：patch 未提的字段保持原值，返回**完整 8 字段** */
export function mockUsageSettings(patch: UsageSettingsPatch = {}): UsageSettings {
  return { ...SETTINGS_DEFAULTS, ...patch };
}
