// 时间范围与环比纯层（计划② Task 3）——「哪一档、截到几天、上一周期怎么比」的唯一出处。
// 消费方：Task 6（`UsageRangeBar` 五档 / 页面 `rangeOf` + `clampCustomRange` + `isDashboardEmpty` /
// `UsageSummary` 的 `prevOf` + 环比文案）、Task 7 / 9（档位与空态）、Task 11（`rangeLabelOf` /
// `spanLabelOf` 文本摘要）、Task 12（分享图头部）。
//
// **四条口径（逐条不得发挥）**：
//  ① **一律宿主本地日历**推算（`getFullYear/getMonth/getDate` / `new Date(y, m, d)`）：落库键
//     （`hourKey` / `dayKey`）与查询窗口键同源，故本层不是近似（spec P6，2026-10-05 用户裁决）。
//  ② **不引入第二套时钟**：不读任何源自带时区、不按 UTC / 固定偏移算日界、**不用
//     `toISOString()` 取日期**（UTC 会偏移一天）；`Date.parse("…T00:00:00+08:00")` 同理禁用。
//  ③ 日期加减**一律走 `addDays`**（`setDate(getDate() + n)` 的日历加法）；毫秒推算
//     （`n × 86_400_000`）禁用——夏令时回拨日不是 `86_400_000` ms，会把 31 天窗算成 32 天
//     （N2 回归）。毫秒常量 `DAY_MS` 按定义**只允许用于「时刻差比较」**；本层没有任何毫秒级
//     时刻差比较，故**未定义**它（定义了即未使用变量，`pnpm lint` 与 `tsc` 双红），
//     「不做毫秒加减」这条纪律由 `addDays` 独占承载。
//  ④ 环比差值**全部由前端算**：`compare` 只提供上一周期完整聚合（`prevBuckets` / `prevMetrics`），
//     五档都靠它推出各指标差值（含命中率百分点差），**绝不**为上一周期发第二次查询。
//
// 其余：零 DOM API、零新增依赖（全仓无 date-fns / dayjs，见 spec 附 D.3）。
// 预设四档**只发 `preset`**：窗口边界（5h 的「最近 5 个完整整点桶、不含当前小时」、上一等长周期的
// 起止）由后端裁定，前端**不得自行推算**——这正是「两套口径」的堵口（契约 §3 要点 1）。
import { fmtDateTime, fmtPct } from "@/lib/usage/format";
import type {
  CompareBlock,
  UsageBuckets,
  UsageDashboard,
  UsageMetrics,
  UsageRange,
  UsageRangePreset,
} from "@/types/usage";

/** 五档；数组顺序**即 UI 顺序**（Task 6 的范围条按它渲染）。 */
export const USAGE_PRESETS: UsageRangePreset[] = ["last5h", "today", "last7d", "last30d", "custom"];

/** 首屏默认档：近 7 天（首屏即有可读趋势；「当日」在日桶口径下只有一个点，趋势卡会走空态分支）。 */
export const DEFAULT_PRESET: UsageRangePreset = "last7d";

/** 自定义档跨度上限（**含首尾**：31 天 = `from` 与 `to` 之间最多 30 个日历步）。 */
export const CUSTOM_MAX_SPAN_DAYS = 31;

/**
 * i18n 翻译函数的最小形状——**全仓唯一声明处**（计划② 各任务一律从这里 import，不得另造）。
 * 与 `usageErrMsg` / `petErrMsg` 的入参形状一致（仓内既有范式）：i18next 的 `t` 可直接传入。
 */
export type TFn = (key: string, opts?: Record<string, unknown>) => string;

/** 本地日键 `YYYY-MM-DD`（与 ① 落库的 `dayKey` 同源）。**不走 `toISOString()`**：那是 UTC 日。 */
export function dayKeyOf(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** 日键 → 本地零点的时刻。**只用于比较**（`clampCustomRange` 判 from / to 先后），不做加减。 */
function parseDay(key: string): number {
  const [y, m, d] = key.split("-").map(Number);
  return new Date(y, (m ?? 1) - 1, d ?? 1).getTime();
}

/**
 * 日期加减：**日历加法**（模块私有，只经 `defaultCustomRange` / `clampCustomRange` 使用）。
 * 一律走这里，**不得**退回 `key + n × 86_400_000` 的毫秒推算：夏令时回拨日不是 `86_400_000` ms，
 * 毫秒推算在那种日期上会偏差一天（把 `to - 30 天` 算成 `to - 31 天`）。Task 6 的截断断言按
 * 运行时今天 + 日历加法推期望值，两边口径不一致就会在特定时区的特定月份红，且极难复现
 * （计划② N2 回归）。
 */
function addDays(key: string, days: number): string {
  const [y, m, d] = key.split("-").map(Number);
  const dt = new Date(y, (m ?? 1) - 1, d ?? 1);
  dt.setDate(dt.getDate() + days); // 跨月 / 闰日 / 跨年交给 Date 自己进位
  return dayKeyOf(dt);
}

/** 自定义档默认区间：今日往前 6 天（7 天窗，即「近 7 天」的可编辑等价物）。 */
export function defaultCustomRange(now: Date): { from: string; to: string } {
  const to = dayKeyOf(now);
  return { from: addDays(to, -6), to };
}

/**
 * 31 天截断（含首尾）：保留 `to`，把 `from` 前移到 `to - 30` 天；
 * `from` 晚于 `to`（用户先改「止」再改「起」的中间态）时**防御性退化为单日**。
 * `clamped` = **是否被 31 天上限截断过**（Task 6 据此**常驻**显示 `usage.rangeClamped`，不是一帧即逝）：
 * 反向区间那一支只是降级、**没有**触发上限，故也返回 `false`——否则界面会误报「已截断为最近 31 天」。
 */
export function clampCustomRange(
  from: string,
  to: string
): { from: string; to: string; clamped: boolean } {
  const maxFrom = addDays(to, -(CUSTOM_MAX_SPAN_DAYS - 1));
  if (parseDay(from) > parseDay(to)) return { from: to, to, clamped: false };
  if (parseDay(from) < parseDay(maxFrom)) return { from: maxFrom, to, clamped: true };
  return { from, to, clamped: false };
}

/** 档位 → 查询入参：预设四档**只发 `preset`**（窗口边界由后端裁定），自定义档发 `from` / `to`。 */
export function rangeOf(
  preset: UsageRangePreset,
  custom: { from: string; to: string }
): UsageRange {
  if (preset === "custom") return { preset, from: custom.from, to: custom.to };
  return { preset };
}

/**
 * 上一周期聚合（契约只给聚合，差值由前端算）；`compare === null` = 上一周期**完全无数据**
 * （后端判据：`prevMetrics.requests === 0` 且四桶全零）→ 各项对比段整段不渲染。
 */
export function prevOf(
  compare: CompareBlock | null
): { buckets: UsageBuckets; metrics: UsageMetrics } | null {
  if (!compare) return null;
  return { buckets: compare.prevBuckets, metrics: compare.prevMetrics };
}

/** 展示用范围标签：预设档走 i18n 键 `usage.range.<preset>`，自定义档出 `from ~ to` 原文。 */
export function rangeLabelOf(range: UsageRange, t: TFn): string {
  if (range.preset === "custom") return `${range.from ?? ""} ~ ${range.to ?? ""}`;
  return t(`usage.range.${range.preset}`);
}

/**
 * 「采集时刻」的值（Task 6 网格 asOf 行与页脚 freshness / Task 11 文本摘要 / Task 12 分享图头部）：
 * `collectedAt === 0` 是「尚未采集」哨兵（计划 §3 第 8 条）→ `usage.notCollected`；否则 `fmtDateTime`
 * 出**本地时区**时刻。**本函数是这条口径的唯一出处**——四个消费方一律调它，不得各自展开这个三元
 * 表达式（展开四份即四处漂移点）。放在本文件是因为 `TFn` 在这里（放 `format.ts` 要反向 import 本文件）。
 */
export function asOfText(collectedAt: number, t: TFn): string {
  return collectedAt === 0 ? t("usage.notCollected") : fmtDateTime(collectedAt);
}

/**
 * 趋势首末键 → 具体跨度文案（Task 11 文本摘要 / Task 12 分享图头部用）：
 * 日桶 `M/D – M/D`（半角减号两侧带空格）；小时桶 `M/D HH:00–HH:00`；空数组与单点出空串（不造假跨度）。
 * 取月 / 日必须用**字符串切片**：`Number("09")` 会掉成 `9`，产出 `9/28 – 10/3`（B2 回归）。
 */
export function spanLabelOf(points: { key: string }[]): string {
  const list = Array.isArray(points) ? points : [];
  if (list.length < 2) return "";
  const first = list[0].key;
  const last = list[list.length - 1].key;
  const md = (key: string) => `${key.slice(5, 7)}/${key.slice(8, 10)}`;
  if (first.includes("T")) {
    return `${md(first)} ${first.slice(11, 13)}:00–${last.slice(11, 13)}:00`;
  }
  return `${md(first)} – ${md(last)}`;
}

/**
 * 数值型环比后缀：`prev` 为 `null` / 非有限 → 空串（只显示当前值，**绝不**显示 0% 或 NaN）。
 * `prev` 为 0 → 差值为「新增」（`usage.compare.new`），不得出现除零 `Infinity`。
 * **符号取自四舍五入后的差值**：取未舍入值会让近零负差印成 `-0.0%`（界面撒谎）；
 * 舍入后为零一律用 `±0.0%`（与 `hitCompareSuffix` 的零值字形统一）。
 * `fmt` 由调用方给（hero 口径 `fmtInt` / 计数口径自定义），本层不猜缩写。
 */
export function compareSuffix(
  cur: number,
  prev: number | null,
  fmt: (n: number) => string,
  t: TFn
): string {
  if (prev === null || !Number.isFinite(prev)) return "";
  if (prev === 0) return t("usage.compare", { prev: fmt(prev), delta: t("usage.compare.new") });
  const delta = cur - prev;
  const pct = Number(((delta / prev) * 100).toFixed(1)); // 先四舍五入，再取符号
  const sign = pct > 0 ? "+" : pct < 0 ? "-" : "±";
  return t("usage.compare", { prev: fmt(prev), delta: sign + Math.abs(pct).toFixed(1) + "%" });
}

/**
 * 命中率环比后缀：百分点差由**前端**从 `compare.prevMetrics.cacheHitRate` 算出（契约只给聚合），
 * 口径 `(cur - prev) × 100`；**先四舍五入再取符号**（近零差不得印成 `-0.0 pt`），零值字形 `±`。
 * `compare === null` 或 prev 非有限 → 空串（P8：上一周期无数据只显示当前值，不得出现 0% / NaN）。
 */
export function hitCompareSuffix(cur: number, compare: CompareBlock | null, t: TFn): string {
  if (!compare) return "";
  const prev = compare.prevMetrics.cacheHitRate;
  if (!Number.isFinite(prev)) return "";
  const pt = Number(((cur - prev) * 100).toFixed(1)); // 先四舍五入，再取符号
  const sign = pt > 0 ? "+" : pt < 0 ? "-" : "±";
  return t("usage.compare", { prev: fmtPct(prev), delta: sign + Math.abs(pt).toFixed(1) + " pt" });
}

/**
 * 空态判定（P8 三态之一：显示「暂无数据」而不是 0 或空白）：零请求 + hero 为 0 + **零行**。
 * 三个条件都要：只判 hero 会把「有请求但四桶全零」误判为空，只判 requests 会把
 * 「尚未采集但账本有历史行（`collectedAt === 0`）」误判为有数据。
 *
 * **第三条件是「无行」而不是「无趋势点」**（2026-10-06 评审定案）：后端**恒按窗口枚举桶**——
 * `src-tauri/src/services/usage/query.rs:512-524` 的趋势是 `resolved.cur.keys` 上逐桶 `aggregate`，
 * 空账本也有满窗口的点（后端自带断言 `assert_eq!(d.trend.len(), 13, "空数据也要出满窗口的桶")`，
 * `:1915`，其注释明确「UI 自己判『暂无数据』」）。故 `trend.length === 0` 在真机**永不成立**，
 * 拿它当空态必落到看板上：hero `0` + 满屏 0 值指标网格 + 一条全零平线（「峰值 0」）。
 * 行数组才是零值枚举的产物：空账本时后端 `groups` 为空 → `rows: []`（同测试 `assert!(d.rows.is_empty())`）。
 * **这不是「严格更强」，而是把判据挪到后端真的会零填的量上**：判据不再看 `trend`，改看
 * `rows` / `hero` / `requests` 才让「暂无数据」**可达**。与旧判据相比：**新增接受**
 * 「零请求 + hero 0 + trend 非空 + rows 空」——正是真机空窗口的形状（后端零填见上，本次修复的目标）；
 * **新增拒绝**「trend 空 + rows 非空」——后端不可能产出的形状。**有行（`rows.length > 0`）的看板永不被判空**。
 */
export function isDashboardEmpty(d: UsageDashboard | null | undefined): boolean {
  if (!d) return true;
  return d.totals.requests === 0 && d.hero === 0 && d.rows.length === 0;
}

/**
 * 「本窗口**没量到 token**」判定（2026-10-06 用户裁决）——「只有是真实的数值，你才能写它的数值。
 * 如果没有产生数值，就不显示了，它完全是空值的话。」
 *
 * 语义 = **窗口内四个 token 桶（inputFresh / cacheRead / cacheWrite / output）的和是否为 0**；
 * 为真 ⇒ 所有**由这四个桶派生的位**渲染 `EM_DASH`（浮窗第①②③行的数值位、看板 hero 与
 * 产出 / 请求输入 / 缓存命中 / 命中率四个行、趋势峰值），**绝不印 0**（spec P8「空态不显示 0」）。
 * **不属于四桶派生量的真值一律不受影响**（KEEP 类，逐条点名）：
 *   * **计数类**：请求次数 / 会话数 / turn 数 / 报错次数 / 工具调用次数；
 *   * **`用户输入(估)`（`userEst`）**：它是从**用户打的字**估出来的（`collectors/{claude,codex,kimi}.rs`
 *     三源独立估），**不是**四桶的派生量 ⇒ 「四桶全零」**不蕴含**「用户没打字」（真机账本实测：
 *     `claude | requests=1 | 四桶全零 | user_est = 7`，15 条真空回合行里 4 条带真值）。它只有
 *     「不可得（`null`）」这一条空态。把它一并抹成 `—` 等于把一个真值藏起来（2026-10-06 第三轮裁决）。
 *   * 采集时刻（`collectedAt`）与「尚未采集」哨兵同理。
 *
 * ⚠️ **与 `isDashboardEmpty` 不是一回事、不可互替**（两条判据各有各的消费方）：
 *   * `isDashboardEmpty` = 「窗口内**一行都没有**」⇒ 页面/浮窗走**整块**「暂无数据」空态；
 *   * 本判据 = 「有行、但四个桶全零」⇒ 看板/浮窗**照常渲染**，只把**四桶派生的位**换成 `—`
 *     （KEEP 类照常：计数类、`userEst`、采集时刻）。
 *   真机两条都可达：**真空回合**行（四桶全零、`requests ≥ 1`——用户给出的账本实测 15 行 / 50
 *   requests，分布在 claude / opencode / zcode）走本判据；空账本走 `isDashboardEmpty`。
 *
 * `null` / 非有限（`NaN`）按「没量到」处理：界面上宁可出 `—`，也不出一个假的 0。
 */
/**
 * 四桶之和是否为 0 ——「有没有量到 token」的**唯一求和实现**（两个对外判据都走它：
 * `tokenUnmeasured`（窗口级）与 `cardTokenUnmeasured`（卡片 / 卡内行级））。
 * 非有限（`NaN`）与负数按「没量到」处理：界面上宁可出 `—`，也不出一个假的 0。
 */
function bucketsUnmeasured(b: UsageBuckets): boolean {
  return !(b.inputFresh + b.cacheRead + b.cacheWrite + b.output > 0);
}

export function tokenUnmeasured(d: UsageDashboard | null | undefined): boolean {
  if (!d) return true;
  return bucketsUnmeasured(d.totalsBuckets);
}

/**
 * **卡片级**（同一个函数也服务**卡内行**）的「没量到 token」：某个 `UsageCard` / `UsageRow` **自己**
 * 四个 token 桶全零 ⇒ 它展示的 token 位出 `EM_DASH`（该卡的合计、该行的值）。
 * **计数类不动**：`requests` / `userEst` 是真值，照常显示。
 *
 * ⚠️ **与 `tokenUnmeasured` 不可互替**（记录页专用，别拿窗口级的去替换它）：
 *   * `tokenUnmeasured` 看的是**整窗** `totalsBuckets` —— 看板 / 浮窗 / 文本摘要 / 分享图的口径；
 *   * 本判据看的是**这一张卡自己的桶**。真机可达的反例：一个**有 token** 的窗口里，某个工具
 *     **只落了真空回合行**（四桶全零、但 `requests ≥ 1`）⇒ 窗口级判据为**假**，而这张卡该出 `—`。
 * 结构上只吃 `buckets` 一个字段：`UsageCard` 与 `UsageRow` 都有它，故一个函数服务两者；
 * 「卡合计 = Σ 卡内行」是后端恒等式，故卡片级与逐行判读结果必然一致（这里仍按各自的值判，
 * 谁被显示就判谁）。`null` / 非有限按「没量到」处理（与 `tokenUnmeasured` 同口径）。
 */
export function cardTokenUnmeasured(x: { buckets: UsageBuckets } | null | undefined): boolean {
  return x ? bucketsUnmeasured(x.buckets) : true;
}
