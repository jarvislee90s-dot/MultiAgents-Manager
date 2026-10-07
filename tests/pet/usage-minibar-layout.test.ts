// Task 13（计划②）· 浮窗迷你条**纯层**：几何公式与第 3 行分工具汇总（7 用例）。
// 判据来源：spec D20（固定 5 行 + 容量公式 `round((行数 × 17.4 + 16) × scale)`）、P2（5 行行序与
// `maxHeight` 兜底）、D13（时序三常量）、计划 §3 第 41 条与 Task 13 步骤 2。
//
// 纪律：
//  * **不碰任何 jsdom 专属 API**（计划 §3 第 33/34 条）：`getBoundingClientRect()` 在 jsdom 里恒 0
//    ⇒ 容量只能用本文件的公式推算 + 实机目验（Task 15 的 U-5…U-10）；
//    **禁止**写「渲染高度 = Npx」这类断言（本文件也不读 DOM）。
//  * 常量与公式**逐字**取自 Task 13 brief；D20 的两条对照值（8 行 @1× = 155、@1.25× = 194）在这里
//    作**外部交叉校验**——公式被改错时它们先红。
//  * `fmt` 由调用方给（本层不猜缩写口径），测试传 `fmtTokens`（= 组件里的真实口径）。
import { describe, expect, it } from "vitest";
import { FRAME_H } from "@/components/pet/petAnimations";
import { UNKNOWN_TOKEN, fmtTokens } from "@/lib/usage/format";
import {
  MINI_BAR_GAP,
  MINI_BAR_MAX_H,
  MINI_BAR_ROWS,
  MINI_BAR_TOOL_LIMIT,
  MINI_GRACE_MS,
  MINI_HOVER_MS,
  MINI_LINE_H,
  MINI_RESTORE_MS,
  miniBarFits,
  miniBarGap,
  miniBarHeight,
  miniBarMaxHeight,
  miniBarToolLine,
  miniBarToolRowText,
  windowHeightSum,
} from "@/lib/usage/miniBar";
import type { UsageRow } from "@/types/usage";

/**
 * 造一行**工具维度**的分组行。真机 `group_of(Tool)` 给 `(key, label) = (source_id, source_id)`
 * （`src-tauri/src/services/usage/query.rs`）⇒ 两个字段同值；显示名只由 `usageAgentLabel` 解析
 * （§3.13/14，本文件不另存一份展示名）。
 */
function toolRow(toolId: string, requestTotal: number): UsageRow {
  return {
    key: toolId,
    label: toolId,
    buckets: { inputFresh: requestTotal, cacheRead: 0, cacheWrite: 0, output: 0 },
    metrics: { requestTotal, cacheHitRate: 0, userEst: null, requests: 1 },
    sourceKind: "measured",
    isSubagent: null,
  };
}

describe("usage-minibar-layout（计划② Task 13 步骤 2：浮窗几何与工具汇总纯层）", () => {
  it("1. 常量表逐字钉死，且 D20 的两条对照值（8 行 @1× = 155 / @1.25× = 194）由公式复现", () => {
    expect(MINI_BAR_ROWS).toBe(5);
    expect(MINI_BAR_TOOL_LIMIT).toBe(3);
    expect(MINI_LINE_H).toBe(17.4);
    expect(MINI_BAR_MAX_H).toBe(120);
    expect(MINI_HOVER_MS).toBe(500);
    // 2026-10-07 用户裁决 200 → 2000：200ms 时「从宠物拖到浮窗」够不到（浮窗先消失）。
    // 本断言是**刻意**的硬编码锁：这个值是用户可感知的交互参数，改动必须显式过审。
    expect(MINI_GRACE_MS).toBe(2000);
    expect(MINI_RESTORE_MS).toBe(500);
    // D20 原文口径：`round((行数 × 17.4 + 16) × scale)`
    expect(miniBarHeight(1, 8)).toBe(155);
    expect(miniBarHeight(1.25, 8)).toBe(194);
    // 固定 5 行（默认 rows = MINI_BAR_ROWS）：@1× = 103、@1.25× = 129（D20 的 673px 那一版）
    expect(miniBarHeight(1)).toBe(103);
    expect(miniBarHeight(1.25)).toBe(129);
    expect(miniBarHeight(0.75)).toBe(77);
  });

  it("2. miniBarMaxHeight 按同一 scale 缩，恒 ≤ 固定 5 行的实际高度（兜底只在越界时才咬）", () => {
    expect(miniBarMaxHeight(1)).toBe(120);
    expect(miniBarMaxHeight(0.75)).toBe(90);
    expect(miniBarMaxHeight(1.25)).toBe(150);
    // 5 行 @三档都在上限之内（maxHeight 是「放不下就截断」的兜底，不是常态高度）
    for (const scale of [0.75, 1, 1.25]) {
      expect(miniBarHeight(scale)).toBeLessThan(miniBarMaxHeight(scale));
    }
    // 行数一旦超出上限口径（如 8 行），maxHeight 才开始起作用——这是兜底存在的理由
    expect(miniBarHeight(1.25, 8)).toBeGreaterThan(miniBarMaxHeight(1.25));
  });

  it("3. windowHeightSum：浮窗与卡片**竖直堆叠 → 求和**，且含那 6px 间距（D20 修订 + 评审裁决）", () => {
    // base 沿用 FoxbellPet 既有公式：round((50 + FRAME_H + 10) × scale)
    const base1 = Math.round((50 + FRAME_H + 10) * 1);
    expect(base1).toBe(268);
    // 卡片 120 + 间距 6 + 浮窗 103 ⇒ 268 + 229 = 497；若误写回 max 口径会得 388
    expect(windowHeightSum({ scale: 1, cardsH: 120, menuH: 0, candidatesH: 0, miniH: 103 })).toBe(
      497
    );
    // 小数部分一律向上取整（窗口尺寸只能是整数）
    expect(windowHeightSum({ scale: 1, cardsH: 0.4, menuH: 0, candidatesH: 0, miniH: 0 })).toBe(
      269
    );
    // 大档 1.25：base = 335；间距 round(6 × 1.25) = 8（浮窗顶边不再高出窗口上沿）
    expect(
      windowHeightSum({ scale: 1.25, cardsH: 150, menuH: 0, candidatesH: 0, miniH: 129 })
    ).toBe(Math.ceil(Math.round(268 * 1.25) + 150 + 8 + 129));
    // 「多出来的那一段」逐档对账：恒 = 间距 + 浮窗高度（间距只在浮窗显示时计入）
    for (const scale of [0.75, 1, 1.25]) {
      const withMini = windowHeightSum({
        scale,
        cardsH: 0,
        menuH: 0,
        candidatesH: 0,
        miniH: miniBarHeight(scale),
      });
      const without = windowHeightSum({ scale, cardsH: 0, menuH: 0, candidatesH: 0, miniH: 0 });
      expect(miniBarGap(scale)).toBe(Math.round(MINI_BAR_GAP * scale)); // 定位式与高度式同一个换算
      expect(withMini - without).toBe(miniBarGap(scale) + miniBarHeight(scale));
    }
  });

  it("4. windowHeightSum：无浮窗时退回旧口径 max(卡片, 菜单, 候选)——菜单/候选与卡片同锚点互斥", () => {
    // miniH = 0（浮窗未显示，或菜单打开时被渲染守卫隐藏）⇒ 三块互斥取最大，不得相加（也不计间距）
    expect(windowHeightSum({ scale: 1, cardsH: 120, menuH: 300, candidatesH: 40, miniH: 0 })).toBe(
      268 + 300
    );
    expect(windowHeightSum({ scale: 1, cardsH: 120, menuH: 0, candidatesH: 40, miniH: 0 })).toBe(
      268 + 120
    );
    // 菜单与浮窗互斥的证据：同一卡片高度下，带浮窗（miniH > 0）时菜单高度**不参与**
    const withMini = windowHeightSum({
      scale: 1,
      cardsH: 120,
      menuH: 300,
      candidatesH: 0,
      miniH: 103,
    });
    expect(withMini).toBe(268 + 120 + 6 + 103);
    expect(withMini).toBeLessThan(268 + 300);
  });

  it("5. miniBarFits：由 windowHeightSum 推得——恰好装得下为真、差 1px 为假", () => {
    // 无卡片：需要 ceil(268 + 6 + 103) = 377
    expect(miniBarFits(1, 0, 377)).toBe(true);
    expect(miniBarFits(1, 0, 376)).toBe(false);
    // 有卡片（jsdom 量不到高度，值由纯函数给）：120 + 6 + 103 ⇒ 497
    expect(miniBarFits(1, 120, 497)).toBe(true);
    expect(miniBarFits(1, 120, 496)).toBe(false);
    // 行数参数同样生效：8 行 @1×（155）比 5 行（103）多要 52px
    expect(miniBarFits(1, 0, 377, 8)).toBe(false);
    expect(miniBarFits(1, 0, 377 + 52, 8)).toBe(true);
  });

  it("6. miniBarToolLine：过滤零请求行、按请求输入降序、只取前 limit 条，超出折「等 N」", () => {
    const rows = [
      toolRow("codex", 460_789),
      toolRow("claude", 1_246_567),
      toolRow("workbuddy", 12_345),
      toolRow("zcode", 234_567),
    ];
    const r = miniBarToolLine(rows, MINI_BAR_TOOL_LIMIT, fmtTokens);
    // 降序：claude > codex > zcode > workbuddy；前 3 条入行，余 1 条折「等 1」（计数由调用方渲染）
    expect(r.text).toBe("Claude 124.66万 · Codex 46.08万 · ZCode 23.46万");
    expect(r.moreCount).toBe(1);
    // 不超限时不出「等 N」
    expect(miniBarToolLine(rows, 4, fmtTokens)).toEqual({
      text: "Claude 124.66万 · Codex 46.08万 · ZCode 23.46万 · WorkBuddy 1.23万",
      moreCount: 0,
    });
    // 零请求行在**过滤**阶段就被丢掉：既不成行、也不进「等 N」（与 Task 2 的 distributionRows 同口径）
    const withZero = [toolRow("claude", 1000), toolRow("codex", 0), toolRow("zcode", 5_000)];
    expect(miniBarToolLine(withZero, 1, fmtTokens)).toEqual({
      text: "ZCode 5000",
      moreCount: 1,
    });
  });

  it("7. miniBarToolLine 空态：全零 / 空输入 / 空值 → 空串（调用方渲染空态，绝不显示 0）", () => {
    const empty = { text: "", moreCount: 0 };
    expect(miniBarToolLine([], MINI_BAR_TOOL_LIMIT, fmtTokens)).toEqual(empty);
    expect(miniBarToolLine(null, MINI_BAR_TOOL_LIMIT, fmtTokens)).toEqual(empty);
    expect(miniBarToolLine(undefined, MINI_BAR_TOOL_LIMIT, fmtTokens)).toEqual(empty);
    // 全零：过滤后为空 ⇒ 空串（**不是**「0」、也不是「等 3」）
    expect(
      miniBarToolLine([toolRow("claude", 0), toolRow("codex", 0)], MINI_BAR_TOOL_LIMIT, fmtTokens)
    ).toEqual(empty);
    // limit = 0（设置越界）：一条都不出，全部折进 moreCount，不抛不崩
    expect(miniBarToolLine([toolRow("claude", 1000)], 0, fmtTokens)).toEqual({
      text: "",
      moreCount: 1,
    });
    // 未知 toolId：显示名走 usageAgentLabel → UNKNOWN_TOKEN（「对象名未知」，与 EM_DASH 不是一回事）
    expect(miniBarToolLine([toolRow("no-such-tool", 12_000)], 3, fmtTokens).text).toBe(
      `${UNKNOWN_TOKEN} 1.20万`
    );
  });

  it("8. miniBarToolRowText：口径标签在**行首**、列表居中、「等 N」在行尾；空态不得只留标签（P8）", () => {
    // 标签文案由调用方给（i18n 键 `usage.mini.toolsBasis`，**值自带分隔符**——zh 是全角冒号）；
    // 本层不加分隔符、不猜口径，只负责拼装顺序（用户 2026-10-06 裁决：数值口径不变，但要看得出口径）。
    const label = "按工具 请求输入：";
    const more = (n: number) => `等 ${n}`;
    const four = miniBarToolLine(
      [
        toolRow("codex", 460_789),
        toolRow("claude", 1_246_567),
        toolRow("workbuddy", 12_345),
        toolRow("zcode", 234_567),
      ],
      MINI_BAR_TOOL_LIMIT,
      fmtTokens
    );
    // 有行 + 折了 1 条：标签 + 列表 + 「等 1」（尾分隔符只由 more 段带出）
    expect(miniBarToolRowText(four, label, more)).toBe(
      "按工具 请求输入：Claude 124.66万 · Codex 46.08万 · ZCode 23.46万 · 等 1"
    );
    // 无「等 N」：行尾**不得**留一个孤零零的分隔符
    expect(miniBarToolRowText({ text: "Claude 1.20万", moreCount: 0 }, label, more)).toBe(
      "按工具 请求输入：Claude 1.20万"
    );
    // **空态**：`text` 为空串 ⇒ 返回空串。**绝不允许只留一个标签**——那会渲染成一行光秃秃的
    // 「按工具 请求输入：」，既不是空态占位、也读不出「没有数据」（spec P8 空态条）。
    expect(miniBarToolRowText({ text: "", moreCount: 0 }, label, more)).toBe("");
    expect(miniBarToolRowText({ text: "", moreCount: 3 }, label, more)).toBe("");
  });
});
