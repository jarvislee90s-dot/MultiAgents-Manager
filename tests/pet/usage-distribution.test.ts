// Task 2（计划②）· 分布行折叠纯层 `src/lib/usage/distribution.ts` 的行为锁。
// 计划 §3 与 brief 关键坑：最多 10 行、超出折「等 N」；空 / `null` / `undefined` 入参不抛；
// **值口径必须与 hero 一致（请求输入 + 产出），不得改成四桶之和**；占比是 0–1 分数。
// 纯函数：不使用任何 jsdom 专属 API（计划 §3 第 33/34 条），node 上下文也能直测。
import { describe, expect, it } from "vitest";
import {
  DIST_MAX_ROWS,
  DIST_MIN_BAR_PCT,
  distValue,
  distributionRows,
} from "@/lib/usage/distribution";
import { mockUsageDashboard } from "@/lib/usage/mockFixtures";
import type { UsageRow } from "@/types/usage";

/** 合成行：形状按真机 `UsageRow` 补全，本层只用得到 key / label / buckets.output / metrics.requestTotal */
function row(key: string, requestTotal: number, output: number, label = key): UsageRow {
  return {
    key,
    label,
    buckets: { inputFresh: 0, cacheRead: 0, cacheWrite: 0, output },
    metrics: { requestTotal, cacheHitRate: 0, userEst: null, requests: 1 },
    sourceKind: "measured",
    isSubagent: null,
  };
}

describe("distribution.ts（分布行折叠）", () => {
  it("1. 最多 DIST_MAX_ROWS 行；超出部分折「等 N」（moreCount = 被折叠的候选行数）", () => {
    const many = Array.from({ length: 12 }, (_, i) => row(`t${i}`, 1_000 - i * 10, 0));

    const { rows, moreCount } = distributionRows(many);
    expect(DIST_MAX_ROWS).toBe(10);
    expect(rows).toHaveLength(10);
    expect(moreCount).toBe(2);
    // 折掉的是尾部小值（排序见第 3 条）；用 join 断言免得数组被格式化成一列
    expect(rows.map((r) => r.key).join(",")).toBe("t0,t1,t2,t3,t4,t5,t6,t7,t8,t9");

    // 边界：恰好 10 行不折；11 行折 1
    expect(distributionRows(many.slice(0, 10)).moreCount).toBe(0);
    expect(distributionRows(many.slice(0, 11)).rows).toHaveLength(10);
    expect(distributionRows(many.slice(0, 11)).moreCount).toBe(1);
  });

  it("2. 空数组 / null / undefined 入参不抛，一律空结果（零值行同样出空态，不出 0）", () => {
    for (const input of [[], null, undefined] as const) {
      expect(distributionRows(input)).toEqual({ rows: [], moreCount: 0 });
    }
    // Task 7「零值出空态不出 0」：没有量的行不成行，也不计入 moreCount
    expect(distributionRows([row("a", 0, 0), row("b", 0, 0)])).toEqual({
      rows: [],
      moreCount: 0,
    });
  });

  it("3. 排序：按 value 降序；同值保持入参顺序；零值与非有限值直接丢弃", () => {
    const mixed = [
      row("a", 100, 0),
      row("zero", 0, 0),
      row("b", 900, 0),
      row("nan", Number.NaN, 100),
      row("c", 900, 0),
      row("neg", -5, 0),
    ];
    const { rows, moreCount } = distributionRows(mixed);
    expect(rows.map((r) => r.key)).toEqual(["b", "c", "a"]); // 降序；b/c 同值保序；零/NaN/负被丢
    expect(moreCount).toBe(0);
    // label 原样透传（维度分派由组件做：provider→t(label)、model→shortModel、tool→usageAgentLabel）
    expect(distributionRows([row("k", 10, 0, "usage.label.unknownProvider")]).rows[0].label).toBe(
      "usage.label.unknownProvider"
    );
  });

  it("4. 占比口径：share = value / max，0–1 分数、DIST_MIN_BAR_PCT 下限、最大行恒 1", () => {
    const { rows } = distributionRows([
      row("a", 1_000, 0),
      row("b", 500, 0),
      row("c", 100, 0),
      row("d", 10, 0),
    ]);
    expect(DIST_MIN_BAR_PCT).toBe(4);
    expect(rows.map((r) => r.value)).toEqual([1_000, 500, 100, 10]);
    // 是 0–1 分数（不是 0–100），下限 4% = 0.04
    expect(rows.map((r) => r.share)).toEqual([1, 0.5, 0.1, DIST_MIN_BAR_PCT / 100]);
    for (const r of rows) {
      expect(r.share).toBeGreaterThanOrEqual(DIST_MIN_BAR_PCT / 100);
      expect(r.share).toBeLessThanOrEqual(1);
    }
    expect(new Set(rows.map((r) => r.key)).size).toBe(rows.length); // key 唯一，供 usage-dist-<key>
  });

  it("5. value 口径 = requestTotal + output（与 hero 一致，不是四桶之和）", () => {
    const fourBuckets: UsageRow = {
      key: "x",
      label: "X",
      buckets: { inputFresh: 100, cacheRead: 900, cacheWrite: 50, output: 70 },
      metrics: { requestTotal: 1_234, cacheHitRate: 0, userEst: null, requests: 3 },
      sourceKind: "measured",
      isSubagent: null,
    };
    expect(distValue(fourBuckets)).toBe(1_304); // requestTotal + output
    expect(distValue(fourBuckets)).not.toBe(1_120); // ≠ 四桶之和 100+900+50+70
    expect(distValue(row("bad", Number.NaN, Number.NaN))).toBe(0); // 非有限值归 0，绝不外泄 NaN

    // 夹具交叉核对：4 工具行的值之和 = dashboard.hero = 2,036,981（1,954,268 请求 + 82,713 产出）
    const dash = mockUsageDashboard({ preset: "today" }, "tool");
    const { rows, moreCount } = distributionRows(dash.rows);
    expect(dash.hero).toBe(2_036_981);
    expect(rows).toHaveLength(4);
    expect(moreCount).toBe(0);
    expect(rows.reduce((s, r) => s + r.value, 0)).toBe(dash.hero);
    expect(rows.map((r) => r.key)).toEqual(["claude", "codex", "zcode", "workbuddy"]);
    expect(rows[0].share).toBe(1);
    // workbuddy 只占 1.05% → 落 4% 下限（真机长尾行的显示底线）
    expect(rows[3].share).toBe(DIST_MIN_BAR_PCT / 100);
  });
});
