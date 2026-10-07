// 趋势卡（计划② Task 7 步骤 3）——把 Task 4 的纯 SVG 趋势图包成看板里的一张卡。
// 消费方：大看板页（board 分支，首屏之后、工作小结之前）。
//
// 三条纪律：
//  ① **内层直接用 `TrendChartWithTooltip`**（不复制几何、不另写 tooltip）：hover 出精确值这件事
//     由 Task 4 组件独占承载（§3 第 29 条：纯 SVG 自绘、零新增图表依赖）。
//  ② **`busy` 时不出峰值**：峰值是「上一档 / 上一次取数」的数字，刷新期间印出来就是撒谎；
//     有图时右槽改出 `loadingLabel`，没图时整卡走加载态（加载态优先于空态——「正在取数」不是
//     「没有数据」，spec P8 三态）。点数 < 2 画不出线 ⇒ 走 `UsageEmpty`，**绝不伪造 0 点或假图**。
//     **全零趋势同样是「没有数据」**（2026-10-06 第二轮）：本窗口四个 token 桶全零（`tokenUnmeasured`）
//     时 `TrendChart` 会把所有点画在**底线上**（`max === 0`）——那是一条**贴底的零线**，与「真的有
//     一条零线」外观无法区分 ⇒ 整卡走 `UsageEmpty`，**不得**画平线（与上一条同源：假图 = 撒谎）。
//  ③ 取色全在 `TrendChart` 内（两个主题变量），本卡只用语义类（`text-muted-foreground` 等），
//     零硬编码色值（§3 第 19/20 条）。
import { Card, CardAction, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { TrendChartWithTooltip, type ChartPoint } from "@/components/usage/TrendChart";
import { UsageEmpty, UsageLoading } from "@/components/usage/UsageStates";

/**
 * 图 `viewBox` 宽高（数值坐标，不是渲染尺寸）：看板窗口 1040×720（`openWindow.ts`，`minWidth` 840），
 * 卡内可用宽度约 950px。SVG 里的字号也随 `viewBox` 等比缩放 ⇒ 若沿用 320 的缺省宽度，x 轴标签
 * （`fontSize={9}`）会被放大到约 27px；故按真实可用宽度取 880×150（宽高比 ≈ 5.9 : 1，标签 ≈ 10px）。
 */
const CHART_W = 880;
const CHART_H = 150;

export interface UsageTrendCardProps {
  /** 趋势点（Task 4 的 `trendPoints` 产物）；**< 2 点画不出线** ⇒ 走空态 */
  points: ChartPoint[];
  /**
   * 本窗口**没量到 token**（`range.ts::tokenUnmeasured` 的产物，**窗口级**判据）：为真时**整卡走空态**。
   *
   * 为什么不只是「不显示峰值」：全零趋势在 `TrendChart` 里会画成一条**贴底的零线**（`max === 0`
   * ⇒ 所有点落底线）——那是**伪造的图**（见文件头纪律 ②）。页面**不再**用 `peakLabel = —` 去补它：
   * 整卡不渲染，就不会出现「峰值 —」+「一条 0 线」自相矛盾的一屏。
   */
  unmeasured: boolean;
  /** 右峰值文案（页面按 `usage.card.peak` + `fmtTokens` 生成，万/亿缩写）；`busy` 时不显示 */
  peakLabel: string;
  /**
   * 环比文案（page 用 `compareSuffix` + `fmtTokens` 生成；空串 = 整段不渲染）。
   *
   * **为什么趋势区也要环比**（spec P8 逐字）：「环比…：指标网格各项**与趋势区**都显示对比值」。
   * 只给**文字**环比（不画上一周期的折线）：契约只带上一周期的**聚合值**
   * （`compare.prevMetrics` / `prevBuckets`），没有上一周期的**序列**，画线要改冻结契约。
   * 口径 = 本区间的 hero（= Σ 趋势点值 = `requestTotal + output`）环比上一等长区间。
   * `compare === null` / 上一周期无数据 → 空串 ⇒ **只显示当前值，不显示对比段**（不得出 0% / NaN）。
   */
  compareLabel: string;
  /** 卡头标题（i18n 由调用方译好：`usage.card.trend`） */
  title: string;
  /** 取数中的文案（`usage` 域无专属键，调用方传 `common.loading`） */
  loadingLabel: string;
  /** 空态文案（调用方传 `usage.empty`） */
  emptyLabel: string;
  /** 取数中（页面传 `dash.isFetching`）：不出峰值；没有图时优先出加载态 */
  busy: boolean;
}

export function UsageTrendCard({
  points,
  unmeasured,
  peakLabel,
  compareLabel,
  title,
  loadingLabel,
  emptyLabel,
  busy,
}: UsageTrendCardProps) {
  // 两种「没有数据」共用一个出口：① 点数 < 2（画不出线）；② 本窗口**没量到 token**（全零趋势会被
  // 画成贴底零线 = 假图）。优先级：`busy` 先于空态——「正在取数」不是「没有数据」（spec P8 三态），
  // 故这两种形态在取数中都先出加载态，**绝不**先画一条零线。
  // **不得**在这里画一条平线或 0 值的图：空数据画成图与「真的没有用量」外观无法区分。
  if (unmeasured || points.length < 2) {
    return busy ? <UsageLoading label={loadingLabel} /> : <UsageEmpty label={emptyLabel} />;
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardAction>
          {/* 峰值与环比同处右槽（竖排）：**不新增第三个 CardHeader 子节点**——`CardHeader` 是
              `grid-rows-[auto_auto]` + `card-action:grid-cols-[1fr_auto]`，插第三个孩子会挤行、
              并与显式定位的 `CardAction` 抢格子。 */}
          <div className="flex flex-col items-end gap-0.5">
            {busy ? (
              <span className="text-muted-foreground text-xs">{loadingLabel}</span>
            ) : (
              <span className="text-muted-foreground text-xs" data-testid="usage-trend-peak">
                {peakLabel}
              </span>
            )}
            {/* 环比：空串 = 上一周期无可比数据 ⇒ 整段不渲染（P8：只显示当前值） */}
            {compareLabel ? (
              <span className="text-muted-foreground text-xs" data-testid="usage-trend-compare">
                {compareLabel}
              </span>
            ) : null}
          </div>
        </CardAction>
      </CardHeader>
      <CardContent>
        <TrendChartWithTooltip
          points={points}
          width={CHART_W}
          height={CHART_H}
          idPrefix="usage-trend-card"
        />
      </CardContent>
    </Card>
  );
}
