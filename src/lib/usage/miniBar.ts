// 浮窗迷你条的**几何与文案口纯层**（计划② Task 13 步骤 2）——「浮窗多高、什么时候装得下、
// 第 3 行怎么折」的唯一出处。消费方：`src/components/pet/UsageMiniBar.tsx`（视图与数据容器）与
// `src/components/pet/FoxbellPet.tsx`（窗口高度求和、时序三常量）。
//
// 纪律（逐条都有计划出处）：
//  ① **固定 5 行**（spec D20）：`MINI_BAR_ROWS` 是行数，不是「最多几行」——视图恒渲 5 行，
//     高度才确定、窗口才不抖。
//  ② `getBoundingClientRect()` 在 jsdom 里恒 0（§3 第 33 条）⇒ 容量只由本层的公式推算
//     （`miniBarHeight` / `windowHeightSum` / `miniBarFits`），实机目验归 Task 15 的 U-5…U-10。
//     本文件零 DOM API、零新增依赖。
//  ③ 浮窗与状态卡片是**竖直堆叠**关系 ⇒ `windowHeightSum` 在浮窗显示时**求和**（D20 核心修订）；
//     菜单 / 候选浮层与卡片同锚点、三者互斥 ⇒ 那一支仍取 `max`（既有口径不变）。
//  ④ 第 3 行只出**一条**汇总行（D20 修订 D4）：过滤零请求行 → 按请求输入降序 → 取前 `limit` 条 →
//     余量折「等 N」由调用方渲染。全零 / 空输入 → **空串**（调用方渲染空态，**绝不显示 0**，§3 第 5 条）。
import { FRAME_H } from "@/components/pet/petAnimations";
import { usageAgentLabel } from "@/lib/usage/format";
import type { UsageRow } from "@/types/usage";

/** 浮窗固定行数（spec D20：请求+命中率 / 缓存命中+产出 / 分工具汇总 / 本会话 / 详情 »）。 */
export const MINI_BAR_ROWS = 5;

/** 第 3 行最多列几个工具（**条数**，不是行数）；与设置项 `miniBarToolRows` 的默认值同值（spec P7）。 */
export const MINI_BAR_TOOL_LIMIT = 3;

/** 单行高（逻辑 px，spec D20 的公式常量；**不得**用实测值替换——jsdom 量不到）。 */
export const MINI_LINE_H = 17.4;

/** 容器上下内边距合计（同理，公式常量）。 */
export const MINI_BAR_PAD = 16;

/** 容器高度上限（逻辑 px）；`maxHeight` + `overflowY: auto` 是「放不下就截断」的兜底（spec P2）。 */
export const MINI_BAR_MAX_H = 120;

/** 悬停多久出浮窗（spec P2「悬停约 0.5 秒」）。 */
export const MINI_HOVER_MS = 500;

/** 离开精灵后的宽限期（D13）：留出从精灵移到浮窗、点「详情 »」的时间。 */
// 2026-10-07 用户实测裁决：200ms 的宽限**够不到浮窗** ——
// 「我把鼠标放在宠物身上，上面会出现一个悬浮窗；移开鼠标时，悬浮窗会立刻消失」，
// 从精灵拖到浮窗的这段时间里指针既不在精灵上、也不在浮窗上（200ms 根本来不及）⇒ 浮窗先消失了。
// 改成 2 秒：给「从宠物移向浮窗」留出容错窗口（进入浮窗会由 clearMiniGrace 立刻取消隐藏）。
// 注意：拖拽路径在 pointerdown 里显式 clearMiniGrace()，不受本值影响（不会让拖拽后残留浮窗）。
export const MINI_GRACE_MS = 2000;

/** 松手后多久恢复浮窗（D13：拖拽宠物期间浮窗隐藏、松手 0.5s 后恢复）。 */
export const MINI_RESTORE_MS = 500;

/**
 * 浮窗与卡片之间的竖直间距（逻辑 px）。
 *
 * **为什么它必须进高度公式**：定位式（Task 13 步骤 6 逐字）是
 * `bottom = px(50 + FRAME_H + 10) + cardsHeight + px(6)` ⇒ 浮窗顶边 =
 * `base + cardsH + px(6) + miniH`；而逐字的高度式只有 `base + cardsH + miniH`
 * ⇒ 浮窗会比窗口上沿**高出 6 × scale**（1.25 档 8px），实机表现为浮窗顶部边框/圆角与上内边距被裁。
 * 计划 Task 15 的 U-5 要求「浮窗**完整可见**、窗口高度 = 卡片 + 浮窗之和」，与定位式差的正是这 6px
 * ——这是计划自身的自相矛盾，**控制器裁决（方案 A）：把间距计入高度，定位式不动**。
 * 两个式子共用本常量，避免再次漂移（`FoxbellPet` 的定位与本函数都用它）。
 */
export const MINI_BAR_GAP = 6;

/** 浮窗与卡片间距的同 scale 换算（`px(6)`），高度式与定位式共用同一个值。 */
export function miniBarGap(scale: number): number {
  return Math.round(MINI_BAR_GAP * scale);
}

/**
 * 浮窗高度估算（spec D20 的公式，逐字）：`round((rows × 17.4 + 16) × scale)`。
 * `scale` 取 `PET_SCALES` 三档（0.75 / 1 / 1.25）；`rows` 默认固定 5 行，只有交叉校验才传别的值。
 */
export function miniBarHeight(scale: number, rows: number = MINI_BAR_ROWS): number {
  return Math.round((rows * MINI_LINE_H + MINI_BAR_PAD) * scale);
}

/** 容器 `maxHeight`（逻辑 px）：`round(120 × scale)`。5 行时恒大于实际高度，只在越界时咬。 */
export function miniBarMaxHeight(scale: number): number {
  return Math.round(MINI_BAR_MAX_H * scale);
}

/** `windowHeightSum` 的入参：三块「锚在精灵上方」的内容高度 + 浮窗高度（逻辑 px）。 */
export interface WindowHeightInput {
  /** 宠物三档缩放（`PET_SCALES`） */
  scale: number;
  /** 状态卡片区实测高度（jsdom 恒 0） */
  cardsH: number;
  /** 右键菜单实测高度（菜单打开时才 > 0） */
  menuH: number;
  /** 跳转歧义候选浮层实测高度（浮层打开时才 > 0） */
  candidatesH: number;
  /** 浮窗高度（`miniBarHeight(scale)`）；**0 = 浮窗未显示**（含菜单打开时被渲染守卫隐藏） */
  miniH: number;
}

/**
 * 宠物窗口的总高度（`syncSize` 的 h 入参）。
 *
 * `base = round((50 + FRAME_H + 10) × scale)` —— 沿用 `FoxbellPet` 既有公式（底部气泡区 + 精灵 + 间隙）。
 * `stack`：**浮窗显示时 `cardsH + miniBarGap(scale) + miniH`**（竖直堆叠 **+ 6px 间距**，D20 核心修订；
 * 间距为什么必须算进来见 `MINI_BAR_GAP` 的长注释）；浮窗不显示时退回旧口径
 * `max(cardsH, menuH, candidatesH)`（菜单 / 候选浮层与卡片同锚点、三者互斥，相加无意义）。
 * 返回 `ceil(base + stack)`：窗口尺寸只能是整数。
 */
export function windowHeightSum({
  scale,
  cardsH,
  menuH,
  candidatesH,
  miniH,
}: WindowHeightInput): number {
  const base = Math.round((50 + FRAME_H + 10) * scale);
  const stack =
    miniH > 0 ? cardsH + miniBarGap(scale) + miniH : Math.max(cardsH, menuH, candidatesH);
  return Math.ceil(base + stack);
}

/**
 * 当前可用区高度下，浮窗能否完整显示（由 `windowHeightSum` 推得，**不另立第二套公式**）。
 * 实机可用区取 `usePetWindow` 的 `clampToWorkArea` 口径（逻辑 px）；本层只做纯算术，
 * 不做 clamp、不改窗口。`availableH` 含 `base`（整窗高度）与那 6px 间距（见 `MINI_BAR_GAP`），
 * 不是「浮窗能用的那一块」。
 *
 * ⚠️ 如实交代：**本任务没有生产调用点**——宠物窗口的越界收口仍在 `usePetWindow.clampToWorkArea`
 * （既有机制，本任务不动）。它的用处有两个：Task 15 的实机容量核对（拿真实可用区验 D20 的余量），
 * 以及将来「小屏自动收起浮窗」时的现成判据。判据只此一处，**不要**在别处另写一套高度比较。
 */
export function miniBarFits(
  scale: number,
  cardsH: number,
  availableH: number,
  rows: number = MINI_BAR_ROWS
): boolean {
  return (
    windowHeightSum({
      scale,
      cardsH,
      menuH: 0,
      candidatesH: 0,
      miniH: miniBarHeight(scale, rows),
    }) <= availableH
  );
}

/**
 * 第 3 行的「分工具汇总」文本（D20）：`Codex 123.46万 · Claude 45.60万 · Kimi 12.35万`
 * （样例按 2026-10-06 用户裁决的 **2 位小数**口径给；旧写的 1 位样例已作废）。
 *
 * * 只收 `metrics.requestTotal > 0` 的行（零请求行既不成行、也不进「等 N」，与 Task 2 的
 *   `distributionRows` 同口径）；按请求输入**降序**（真机看板查询 `usageDashboard` 也是这个序，
 *   `query.rs::dashboard` 的 `sort_by_key(Reverse(request_total))`）。
 * * 取前 `limit` 条；`moreCount` = **被折掉的条数**（调用方渲染 `usage.moreN`）。
 * * 全零 / 空输入 / `limit <= 0` → `text` 为空串：**由调用方渲染空态**，
 *   本层绝不回退成「0」（§3 第 5 条）。
 * * 显示名一律 `usageAgentLabel(row.label)`（§3 第 13 条）：真机 tool 维度的 `key` 与 `label`
 *   同值 = 采集源 id（`query.rs::group_of`），展示名只由 `AGENT_BADGE` 解析；`fmt` 由调用方给
 *   （本层不猜缩写口径，组件传 `fmtTokens`）。
 */
export function miniBarToolLine(
  rows: readonly Pick<UsageRow, "key" | "label" | "metrics">[] | null | undefined,
  limit: number,
  fmt: (n: number) => string
): MiniBarToolLine {
  const list = (Array.isArray(rows) ? rows : [])
    .filter((r) => r.metrics.requestTotal > 0)
    .sort((a, b) => b.metrics.requestTotal - a.metrics.requestTotal);
  const head = list.slice(0, Math.max(0, Math.floor(limit)));
  return {
    text: head.map((r) => `${usageAgentLabel(r.label)} ${fmt(r.metrics.requestTotal)}`).join(" · "),
    moreCount: list.length - head.length,
  };
}

/** `miniBarToolLine` 的产物（列表文本 + 被折掉的条数） */
export interface MiniBarToolLine {
  text: string;
  moreCount: number;
}

/**
 * 第 3 行的**完整行文本** = 行首口径标签 + 列表 + 行尾「等 N」——本行文案的唯一出处。
 *
 * 为什么要有标签（用户 2026-10-06 裁决）：本行显示的是**请求输入**（第 ① 行的按工具拆解，
 * 各工具相加逐字等于第 ① 行总数），而看板的分布卡 / 文本摘要 / 分享图 / CSV 显示的是
 * **请求输入 + 产出**（分布卡旁边是 hero 大数字，口径随 hero）⇒ 同一个工具在两个面上是两个数。
 * 数值口径**不变**（仍是请求输入），但**必须让用户看得出这一行的口径**。
 *
 * 参数都由调用方给：`label` = i18n 键 `usage.mini.toolsBasis`（**值自带分隔符**——zh 是全角冒号、
 * en 是半角冒号 + 一个空格：标点属语言，故不写在本层，本层也不额外补空格）、
 * `more` = `usage.moreN` 的译函数。
 *
 * **空态**：`line.text === ""`（全零 / 空输入 / `limit <= 0`）⇒ 返回**空串**，
 * 由调用方渲染空态占位。**绝不**回退成「只有标签」——一行光秃秃的「按工具 请求输入：」
 * 既不是空态、也读不出「没有数据」（spec P8 空态条）。
 */
export function miniBarToolRowText(
  line: MiniBarToolLine,
  label: string,
  more: (n: number) => string
): string {
  if (line.text === "") return "";
  return label + line.text + (line.moreCount > 0 ? ` · ${more(line.moreCount)}` : "");
}
