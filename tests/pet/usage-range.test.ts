// Task 3（计划②）· 时间范围与环比纯层 `src/lib/usage/range.ts` 的行为锁（24 条用例）。
// 期望值逐字取自：spec P6（五档定义与「近 5 小时」整点桶口径）/ P8（环比呈现规则：上一周期
// 无数据只显示当前值，**不得**出现 0% 或 NaN）/ D14（「上一周期」= 紧邻当前区间之前的等长区间）
// 与冻结契约 §3 要点 1（`compare` 只给上一周期完整聚合 → 各项差值由前端算，五档共用一次查询）。
//
// 两条日期纪律（N2 回归，用例 8–11）：日期加减**必须是日历加法**（`setDate(getDate() + n)`）；
// 毫秒推算（`+ n × 一天的毫秒数`）只在**窗口内含春季前跳（23 小时日）**的时区才分叉——
// 秋季回拨（25 小时日）只会落到同日 01:00，仍是 31 天。因此夹具日窗口（用例 9）之外，
// 另补美区（2026-03-08）/欧区（2026-03-29）两条窗口，让**普通美/欧开发机**上也能红；
// CI（UTC runner）抓不到时区相关行为，由用例 14 的**源码锁**兜底。
// 纯函数：**不使用任何 jsdom 专属 API**（计划 §3 第 33/34 条），node 上下文也能直测。
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CUSTOM_MAX_SPAN_DAYS,
  clampCustomRange,
  compareSuffix,
  dayKeyOf,
  defaultCustomRange,
  hitCompareSuffix,
  isDashboardEmpty,
  prevOf,
  rangeLabelOf,
  rangeOf,
  spanLabelOf,
  tokenUnmeasured,
  USAGE_PRESETS,
} from "@/lib/usage/range";
import { mockUsageDashboard } from "@/lib/usage/mockFixtures";
import type { CompareBlock } from "@/types/usage";

/** 假翻译器：把插值原样摊进结果，让「传了哪些插值」也能被断言（真机文案在 i18n 里） */
const t = (key: string, opts?: Record<string, unknown>) =>
  opts ? `${key}|${JSON.stringify(opts)}` : key;

/**
 * 上一周期聚合基准（契约只管上一周期完整聚合，差值前端算）——`prevOf` 与「环比文案」两组共用，
 * 因此必须声明在**文件顶层**：写进某个 describe 回调里会让另一组的 `prevOf(COMPARE)` 抛
 * `ReferenceError`（跨作用域取不到，整个测试文件在收集期即崩）。
 */
const COMPARE: CompareBlock = {
  prevBuckets: { inputFresh: 400_000, cacheRead: 1_196_266, cacheWrite: 12_601, output: 71_133 },
  prevMetrics: { requestTotal: 1_680_000, cacheHitRate: 0.712, userEst: 15_400, requests: 396 },
};

/**
 * 独立的**日历步数 oracle**：从 `from` 用 `setDate` 一步一步走到 `to`，含首尾数步数。
 * - **不得**改成读实现：`addDays` 是模块私有，且正是被测对象（用实现验实现等于没验）；
 * - 这里连本地日格式化都自己写一份（不用被测的 `dayKeyOf`），保持零共享代码；
 * - 结果与机器时区无关：`new Date(y, m-1, d)` + `setDate` 的日历结果处处相同。
 */
function calendarSpan(range: { from: string; to: string }): number {
  const localKey = (d: Date) =>
    `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const [fy, fm, fd] = range.from.split("-").map(Number);
  const cursor = new Date(fy, fm - 1, fd);
  let span = 1; // 含首尾
  while (localKey(cursor) !== range.to) {
    cursor.setDate(cursor.getDate() + 1);
    span += 1;
    if (span > 400) throw new Error(`日历计数失控：from=${range.from} 走不到 to=${range.to}`);
  }
  return span;
}

describe("五档定义（P6 / D5）", () => {
  it("1. 顺序固定为 last5h / today / last7d / last30d / custom", () => {
    expect(USAGE_PRESETS).toEqual(["last5h", "today", "last7d", "last30d", "custom"]);
  });

  it("2. 五档逐个走：预设四档**只带 preset**（窗口边界由后端裁定），自定义档带 from/to", () => {
    const custom = { from: "2026-09-01", to: "2026-10-03" };
    for (const preset of USAGE_PRESETS) {
      const range = rangeOf(preset, custom);
      if (preset === "custom") {
        expect(range).toEqual({ preset: "custom", from: "2026-09-01", to: "2026-10-03" });
      } else {
        expect(range).toEqual({ preset });
        // 线上形状：预设档**不得**多带任何键（多带 from/to 会与后端窗口口径打架）
        expect(Object.keys(range)).toEqual(["preset"]);
      }
    }
  });
});

describe("dayKeyOf / defaultCustomRange", () => {
  it("3. 本地日 YYYY-MM-DD（不走 UTC，避免跨时区偏移一天）", () => {
    expect(dayKeyOf(new Date(2026, 9, 3, 0, 5))).toBe("2026-10-03");
    expect(dayKeyOf(new Date(2026, 0, 1, 23, 59))).toBe("2026-01-01");
  });

  it("4. 默认自定义区间 = 今日往前 6 天（7 天窗，即「近 7 天」的可编辑版）", () => {
    expect(defaultCustomRange(new Date(2026, 9, 3))).toEqual({
      from: "2026-09-27",
      to: "2026-10-03",
    });
  });
});

describe("clampCustomRange：跨度 ≤31 天（含首尾），超限截断", () => {
  it("5. 31 天整不截断", () => {
    expect(clampCustomRange("2026-09-03", "2026-10-03")).toEqual({
      from: "2026-09-03",
      to: "2026-10-03",
      clamped: false,
    });
  });

  it("6. 32 天截断为最近 31 天（保留 to，前移 from）", () => {
    expect(clampCustomRange("2026-09-01", "2026-10-03")).toEqual({
      from: "2026-09-03",
      to: "2026-10-03",
      clamped: true,
    });
  });

  it("7. from 晚于 to 时以 to 为准退化为单日，且**不算截断**（不得误报「已截断为最近 31 天」）", () => {
    expect(clampCustomRange("2026-10-05", "2026-10-03")).toEqual({
      from: "2026-10-03",
      to: "2026-10-03",
      clamped: false,
    });
    // 单日区间同属「没被上限截断」的一支
    expect(clampCustomRange("2026-10-03", "2026-10-03")).toEqual({
      from: "2026-10-03",
      to: "2026-10-03",
      clamped: false,
    });
  });

  // ---- N2 回归：日期加减必须是**日历加法**，不是 `n × 一天的毫秒数` 的毫秒推算 ----
  // 毫秒推算只在窗口内含**春季前跳**（23 小时日）的时区才把跨度撑成 32 天；Task 6 的截断断言按
  // 运行时今天 + 日历加法推期望值 → 两边口径不一致就在特定时区的特定月份红，极难复现。
  // 下面用**跨月 / 闰日 / 跨年**三种边界把日历语义钉死（结果与机器时区无关）。
  it("8. 跨月 / 闰日 / 跨年的 31 天截断都是日历加法（N2 回归）", () => {
    // 3/1 往前 30 天 = 1/30（跨过 2 月）
    expect(clampCustomRange("2026-01-01", "2026-03-01")).toEqual({
      from: "2026-01-30",
      to: "2026-03-01",
      clamped: true,
    });
    // 闰年：2028-03-01 往前 30 天 = 2028-01-31（2028 年 2 月有 29 天）
    expect(clampCustomRange("2028-01-01", "2028-03-01")).toEqual({
      from: "2028-01-31",
      to: "2028-03-01",
      clamped: true,
    });
    // 跨年：2027-01-05 往前 30 天 = 2026-12-06
    expect(clampCustomRange("2026-01-01", "2027-01-05")).toEqual({
      from: "2026-12-06",
      to: "2027-01-05",
      clamped: true,
    });
  });

  it("9. 夹具日窗口按日历计数恰好 31 天（含首尾）——毫秒推算只在南半球前跳时区露馅", () => {
    const clamped = clampCustomRange("2000-01-01", "2026-10-03");
    expect(clamped.from).toBe("2026-09-03");
    expect(calendarSpan(clamped)).toBe(CUSTOM_MAX_SPAN_DAYS);
  });

  // 以下两条针对**北半球开发机**：UTC（CI runner）与 CST 下毫秒推算与日历加法同值，
  // 只有窗口内含北美/欧洲春季前跳的时区才会分叉 —— 故必须各补一条，否则 CI 与本机门禁
  // 都拦不住被禁的毫秒写法（评审 Important 1）。
  it("10. 窗口含**北美**春季前跳（2026-03-08）→ 仍是日历 31 天（N2 · 美区开发机可抓）", () => {
    const clamped = clampCustomRange("2000-01-01", "2026-03-15");
    expect(clamped.from).toBe("2026-02-13");
    expect(calendarSpan(clamped)).toBe(CUSTOM_MAX_SPAN_DAYS);
  });

  it("11. 窗口含**欧洲**春季前跳（2026-03-29）→ 仍是日历 31 天（N2 · 欧区开发机可抓）", () => {
    const clamped = clampCustomRange("2000-01-01", "2026-04-05");
    expect(clamped.from).toBe("2026-03-06");
    expect(calendarSpan(clamped)).toBe(CUSTOM_MAX_SPAN_DAYS);
  });
});

describe("prevOf：上一周期聚合（契约只给聚合，差值前端算）", () => {
  it("12. compare 为 null → null（上一周期完全无数据）", () => {
    expect(prevOf(null)).toBeNull();
  });

  it("13. compare 在位 → **原样**给出上一周期四桶与派生口径（逐字段相等，不是抽查）", () => {
    const prev = prevOf(COMPARE);
    expect(prev?.buckets).toEqual(COMPARE.prevBuckets);
    expect(prev?.metrics).toEqual(COMPARE.prevMetrics);
    // 形状只此两键（toStrictEqual 区分 undefined 与缺键，见 ① parity 测试同款纪律）
    expect(prev).toStrictEqual({
      buckets: COMPARE.prevBuckets,
      metrics: COMPARE.prevMetrics,
    });
    // `prevMetrics.userEst` 可空：原样透传，**不得**把 null 化成 0（环比文案要容忍 null）
    expect(prev?.metrics.userEst).toBe(15_400);
  });
});

describe("源码锁：日期口径（与时区无关，CI 上也生效）", () => {
  it("14. range.ts 的**代码**不得出现毫秒推算 / UTC 取日，日历加减只经 setDate + 本地 getter", () => {
    const src = readFileSync(resolve(__dirname, "../../src/lib/usage/range.ts"), "utf8");
    // 锁只作用于**剥离注释后的代码行**：本文件头四条口径注释与各函数 JSDoc 本身就要写
    // 「禁用 toISOString / 毫秒推算」这类字样，不能自锁。
    const code = src
      .replace(/\/\*[\s\S]*?\*\//g, "") // 块注释（含 JSDoc）
      .split("\n")
      .map((line) => line.replace(/\s\/\/.*$/, "")) // 行内注释
      .filter((line) => !line.trim().startsWith("//")) // 整行注释
      .join("\n");

    for (const banned of [
      "86_400_000",
      "86400000",
      "toISOString",
      "getUTC",
      "Date.parse",
      "Date.UTC",
    ]) {
      expect(code, `range.ts 代码里不得出现 ${banned}（口径②③）`).not.toContain(banned);
    }
    // 正向：日历加减只经 setDate；本地日取数只经本地 getter
    expect(code, "日历加减必须经 setDate").toContain(".setDate(");
    expect(code, "本地日必须经本地 getter 取数").toMatch(
      /getFullYear\(\)|getMonth\(\)|getDate\(\)/
    );

    // ⚠️ **已知盲区（如实登记，别当万无一失）**：
    //  ① 字面量锁可被等价写法绕过（`1000 * 60 * 60 * 24` / `864e5` / `DAY_MS * n`）。缓解：上面那条
    //     正向断言（代码里**必须有** `.setDate(`）能抓住「把日历加法整段换成毫秒推算」的自然写法——
    //     实测把 `addDays` 改成 `1000 * 60 * 60 * 24` 会因 `.setDate(` 消失而红（UTC 下也红）；
    //     但若将来在**别处**留一个 `setDate`、同时用毫秒推算算窗口，这条锁就看不见了。
    //  ② 「不读 UTC」只锁了四个名字：`Intl.DateTimeFormat({ timeZone: "UTC" })`、手写 `+08:00`
    //     偏移换算、`new Date("2026-10-03")`（ISO 串按 **UTC** 解析）都不在判据里；
    //  ③ 字符串拼接/替换构造日键（不经 toISOString）同样绕得过；
    //  ④ 只扫本文件：别的文件另起炉灶做日期运算不受这条锁约束；
    //  ⑤ 注释剥离是行级的，对含 `//` 的字符串字面量与非常规写法无能为力。
    //  真正兜底的是**行为用例**（8–11）：美/欧/南半球春季前跳窗口里把日历语义钉死。
  });
});

describe("rangeLabelOf / spanLabelOf", () => {
  it("15. 预设档走 i18n 键，自定义档出日期区间", () => {
    expect(rangeLabelOf({ preset: "last5h" }, t)).toBe("usage.range.last5h");
    expect(rangeLabelOf({ preset: "custom", from: "2026-09-01", to: "2026-10-03" }, t)).toBe(
      "2026-09-01 ~ 2026-10-03"
    );
  });

  it("16. spanLabelOf：日桶出 M/D – M/D；小时桶出 M/D HH:00–HH:00；空/单点出空串", () => {
    expect(spanLabelOf([{ key: "2026-09-28" }, { key: "2026-10-03" }])).toBe("09/28 – 10/03");
    expect(spanLabelOf([{ key: "2026-10-03T09" }, { key: "2026-10-03T13" }])).toBe(
      "10/03 09:00–13:00"
    );
    expect(spanLabelOf([])).toBe("");
    expect(spanLabelOf([{ key: "2026-10-03" }])).toBe("");
  });
});

describe("环比文案（P8：上一周期无数据只显示当前值，不得出现 0% / NaN）", () => {
  // ⚠️ `compareSuffix` 自 2026-10-06 起**不再收 `fmt`**：上一周期的值按用户裁决（「同比冗余字太多」）
  // 不进屏上文案，改由调用方用 `usage.compareHint` 挂进 `title`。屏上只剩差值 ⇒ 本文件的期望串
  // 一律只剩 `delta`（`prev` 不再出现在插值里）。

  it("17. 数值型：prev 为 null 或非有限 → 空串", () => {
    expect(compareSuffix(100, null, t)).toBe("");
    expect(compareSuffix(100, Number.NaN, t)).toBe("");
  });

  it("18. 数值型：正常升降出百分比差（1 位小数带符号）", () => {
    expect(compareSuffix(120, 100, t)).toBe('usage.compare|{"delta":"+20.0%"}');
    expect(compareSuffix(80, 100, t)).toBe('usage.compare|{"delta":"-20.0%"}');
  });

  it("19. 数值型：上一周期为 0 → 显示「新增」而不是除零 Infinity", () => {
    expect(compareSuffix(50, 0, t)).toBe('usage.compare|{"delta":"usage.compare.new"}');
  });

  it("20. 命中率型：百分点差由前端从 prevMetrics.cacheHitRate 算出（当前 71.9% vs 上一周期 71.2%）", () => {
    expect(hitCompareSuffix(0.719, COMPARE, t)).toBe('usage.compare|{"delta":"+0.7 pt"}');
    expect(hitCompareSuffix(0.678, COMPARE, t)).toBe('usage.compare|{"delta":"-3.4 pt"}');
    expect(hitCompareSuffix(0.712, COMPARE, t)).toBe('usage.compare|{"delta":"±0.0 pt"}');
  });

  it("21. 命中率型：compare 为 null → 空串（整段对比不渲染）", () => {
    expect(hitCompareSuffix(0.719, null, t)).toBe("");
  });

  it("22. 零值字形：**先四舍五入再取符号**——近零差不得印成 -0.0% / -0.0 pt", () => {
    // 精确零：两侧零值字形统一为 `±`（不是 `+0.0%`）
    expect(compareSuffix(1_000_000, 1_000_000, t)).toBe('usage.compare|{"delta":"±0.0%"}');
    // 近零负差（-400 / 1e6 = -0.04%，舍入后为 0）——符号取未舍入值时会印成 -0.0%
    expect(compareSuffix(999_600, 1_000_000, t)).toBe('usage.compare|{"delta":"±0.0%"}');
    // 近零正差（+400 / 1e6 = +0.04%，舍入后同样为 0）→ 也是 ±，不是 +0.0%
    expect(compareSuffix(1_000_400, 1_000_000, t)).toBe('usage.compare|{"delta":"±0.0%"}');
    // 舍入后**仍非零**的差值不得被「零值化」吃掉符号
    expect(compareSuffix(999_000, 1_000_000, t)).toBe('usage.compare|{"delta":"-0.1%"}');
    // 命中率同理：-0.02 pt / +0.02 pt 舍入后为 0 → ±0.0 pt
    expect(hitCompareSuffix(0.7118, COMPARE, t)).toBe('usage.compare|{"delta":"±0.0 pt"}');
    expect(hitCompareSuffix(0.7122, COMPARE, t)).toBe('usage.compare|{"delta":"±0.0 pt"}');
  });
});

describe("isDashboardEmpty：空态判定（显示「暂无数据」而不是 0 或空白）", () => {
  it("23. 零请求 + 无趋势点 + hero 为 0 → 空态", () => {
    const d = mockUsageDashboard({ preset: "last7d" }, "tool");
    expect(
      isDashboardEmpty({
        ...d,
        rows: [],
        trend: [],
        hero: 0,
        totals: { ...d.totals, requests: 0 },
      })
    ).toBe(true);
  });

  it("24. 有请求 → 非空态", () => {
    expect(isDashboardEmpty(mockUsageDashboard({ preset: "last7d" }, "tool"))).toBe(false);
  });

  it("25. 空窗口回归锁：trend **非空**（后端恒填满窗口桶）也必须判空", () => {
    // 后端 `services/usage/query.rs:507-524` 恒按窗口枚举桶、`:1879-1915` 断言空账本仍出满桶
    // ⇒ 真机空窗口的 `trend.length` 是 7（或 5/13/30/31），**永不**为 0。
    // 旧判据（`trend.length === 0`）因此永不成立，A-8 的「暂无数据」不可达；本用例锁住新判据。
    const d = mockUsageDashboard({ preset: "last7d" }, "tool");
    const zeroPoint: (typeof d.trend)[number] = {
      ...d.trend[0],
      buckets: { inputFresh: 0, cacheRead: 0, cacheWrite: 0, output: 0 },
      metrics: { requestTotal: 0, cacheHitRate: 0, userEst: null, requests: 0 },
    };
    // `placeholderTrendBuckets`：桶键是**占位日期**，`27 + i` 在 i ≥ 4 时越过 9 月末
    // （9/31–9/33，非真实日历日），只为凑满 7 个点；判据不看 trend，故不影响断言。
    const placeholderTrendBuckets = Array.from({ length: 7 }, (_, i) => ({
      ...zeroPoint,
      key: `2026-09-${27 + i}`,
    }));
    expect(
      isDashboardEmpty({
        ...d,
        rows: [],
        trend: placeholderTrendBuckets,
        hero: 0,
        totals: { ...d.totals, requests: 0 },
      })
    ).toBe(true);
    // 反向：窗口内**有历史行**（`collectedAt === 0` 的「尚未采集」口径）不得判空——判空会把已有数据藏起来
    expect(
      isDashboardEmpty({
        ...d,
        collectedAt: 0,
        hero: 0,
        totals: { ...d.totals, requests: 0 },
      })
    ).toBe(false);
  });
});

describe("tokenUnmeasured：本窗口是否**真的量到过 token**（2026-10-06 用户裁决：没产生数值就不显示）", () => {
  it("26. 四桶合计为 0 → true（**真空回合**：requests ≥ 1 也算「没量到」；与 isDashboardEmpty 不同义）", () => {
    // 真机形状（用户给出的账本实测）：四桶全零、但 `requests ≥ 1`——15 行 / 50 requests，
    // 分布在 claude / opencode / zcode。这种窗口 `isDashboardEmpty` 为**假**（有行），
    // 于是旧实现会渲染 `请求输入 0 · 命中率 0.0%` / hero 0 —— 正是 spec P8「空态不显示 0」要拦的。
    //
    // ⚠️ **本用例的夹具只喂判据读的那一个字段**（`totalsBuckets`），**不动 `rows`**：
    // 真机上 `totalsBuckets = Σ rows.buckets`，故这是一个**纯函数单测的形状**、不是真机形状
    // （真机形状见 `usage-page.test.tsx` 用例 8 与 `usage-minibar.test.tsx` 用例 13 的端到端夹具）。
    // 保持这样是刻意的：判据只读一个字段，单测就不替它维护整窗一致性；**不要**把它当成真机样本。
    const d = mockUsageDashboard({ preset: "today" }, "tool");
    const zeroBuckets = { inputFresh: 0, cacheRead: 0, cacheWrite: 0, output: 0 };
    const unmeasured = {
      ...d,
      totalsBuckets: zeroBuckets,
      hero: 0,
      totals: { ...d.totals, requestTotal: 0, cacheHitRate: 0, requests: 50 },
    };
    // 前提：它**不是**空窗口（有行、有请求次数）⇒ 两条判据不可互替
    expect(isDashboardEmpty({ ...unmeasured, rows: unmeasured.rows })).toBe(false);
    expect(unmeasured.rows.length).toBeGreaterThan(0);
    expect(unmeasured.totals.requests).toBe(50);
    expect(tokenUnmeasured(unmeasured)).toBe(true);
  });

  it("27. 任一桶 > 0 → false；null / 非有限 → true（不可得按「没量到」处理，绝不印 0）", () => {
    const d = mockUsageDashboard({ preset: "last7d" }, "tool");
    expect(tokenUnmeasured(d)).toBe(false);
    // 逐个桶单独抬起都要判「量到过」——四个桶一个都不能漏（漏掉的那个桶为唯一非零时会被误判成空）
    for (const k of ["inputFresh", "cacheRead", "cacheWrite", "output"] as const) {
      expect(
        tokenUnmeasured({
          ...d,
          totalsBuckets: { inputFresh: 0, cacheRead: 0, cacheWrite: 0, output: 0, [k]: 1 },
        })
      ).toBe(false);
    }
    // 不可得（null / undefined）与脏值（NaN）一律按「没量到」：界面上宁可出 `—`，也不出一个假 0
    expect(tokenUnmeasured(null)).toBe(true);
    expect(tokenUnmeasured(undefined)).toBe(true);
    expect(
      tokenUnmeasured({
        ...d,
        totalsBuckets: { inputFresh: NaN, cacheRead: 0, cacheWrite: 0, output: 0 },
      })
    ).toBe(true);
    // 有行但四桶全零（另一条真机路径：只有会话/报错计数、没有 token）同样是「没量到」
    expect(
      tokenUnmeasured({
        ...d,
        totalsBuckets: { inputFresh: 0, cacheRead: 0, cacheWrite: 0, output: 0 },
      })
    ).toBe(true);
  });
});
