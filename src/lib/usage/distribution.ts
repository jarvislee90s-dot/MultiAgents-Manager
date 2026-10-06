// 用量分布行纯层（计划② Task 2）——「最多几行、超出怎么折、占比怎么算」的唯一出处。
// 消费方：Task 7 分布卡（行 testid `usage-dist-<key>`、占比条下限 `DIST_MIN_BAR_PCT`）、
// Task 11 文本摘要（`label + 两个空格 + fmtInt(value)` 精确值）、Task 12 分享图。
// 纪律：零 DOM API、零新增依赖；值口径与 hero 一致（`metrics.requestTotal + buckets.output`），
// **不是**四桶之和（brief 关键坑）。label 原样透传 —— 维度分派由组件做
// （provider → `t(label)`、model → `shortModel(label)`、project → 原文、tool → `usageAgentLabel`）。
import type { UsageRow } from "@/types/usage";

/** 占比条最小宽度（**百分比**，Task 7 直接写进 style；0–1 分数口径见 `DistRow.share`） */
export const DIST_MIN_BAR_PCT = 4;

/** 分布卡最多显示几行，超出折「等 N」（`moreCount`） */
export const DIST_MAX_ROWS = 10;

/**
 * 一行分布（形状由控制器钉死）：
 * - `key`：Task 7 的 `usage-dist-<key>` testid 用；
 * - `label`：**原文**，不在这里做维度分派；
 * - `value`：**精确值**（与 hero 同口径），供 hover 精确值与文本摘要（`fmtInt`）；
 * - `share`：**0–1 分数**（绝不是 0–100），= `value / max(values)`，下限 `DIST_MIN_BAR_PCT / 100`，上限 1。
 */
export interface DistRow {
  key: string;
  label: string;
  value: number;
  share: number;
}

/**
 * 单行的值口径 = hero（`metrics.requestTotal + buckets.output`）。
 * 非有限值归 `0`：这一层绝不把 `NaN` / `Infinity` 外泄给 `fmtInt` / 占比计算。
 */
export function distValue(row: Pick<UsageRow, "buckets" | "metrics">): number {
  const v = row.metrics.requestTotal + row.buckets.output;
  return Number.isFinite(v) ? v : 0;
}

/**
 * 折叠分布行：
 * ① 丢掉没有量的行（`value <= 0` / 非有限）——零值不成行、也不进「等 N」（Task 7 空态靠它）；
 * ② 按 `value` 降序（同值保持入参顺序，`Array#sort` 稳定）；
 * ③ 截到 `DIST_MAX_ROWS` 行，其余计数为 `moreCount`。
 * `null` / `undefined` / 空数组一律返回 `{ rows: [], moreCount: 0 }`（上层查询未就绪时会直接传进来）。
 */
export function distributionRows(
  rows: readonly Pick<UsageRow, "key" | "label" | "buckets" | "metrics">[] | null | undefined
): { rows: DistRow[]; moreCount: number } {
  const candidates = (rows ?? [])
    .map((r) => ({ key: r.key, label: r.label, value: distValue(r) }))
    .filter((r) => r.value > 0)
    .sort((a, b) => b.value - a.value);

  const top = candidates.slice(0, DIST_MAX_ROWS);
  // 已降序且都已过滤为正 → 首行即最大值，`max > 0` 保证下面不会出现 0/0
  const max = candidates.length > 0 ? candidates[0].value : 0;
  return {
    rows: top.map((r) => ({
      key: r.key,
      label: r.label,
      value: r.value,
      share: Math.min(1, Math.max(DIST_MIN_BAR_PCT / 100, r.value / max)),
    })),
    moreCount: candidates.length - top.length,
  };
}
