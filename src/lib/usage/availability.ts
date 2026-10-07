// 可得性判据纯层（计划② Task 8）——「这个指标的这块数字能不能显示」的唯一出处。
// 消费方：工作小结区 `UsageWorkSummary`（Task 8）；记录页 / 浮窗将来遇 `perSource` 可复用。
//
// 四条口径（§3 第 5/9 条 + 契约 §2 `UsageAvailability`）：
//  ① **缺 `availability` 条目 ≠ 不可得**：后端 `availability_table()` 运行时只出 **8** 条
//     （`turn` / `errorModel` / `errorTurn` / `errorTool` / `interrupted` / `longestTurn` /
//     `toolCalls` / `userEst`），`sessions` / `toolAvgMs` / `topTool` / `topToolMs` **没有条目**
//     —— 这四个字段的空态只由**值本身为 `null`** 驱动。把「缺条目」当空态会把真值（如 `sessions`）
//     整行抹掉，故 `av === null` 一律按**可用**处理。
//  ② **指标级与逐源是两层**：`available === false` = 所有来源都不可得；`perSource[<源>] = false`
//     = 只有该源不可得（如 claude 无回合级失败字段、workbuddy 无回合概念）。两者不得互推 ——
//     `metricReason(t, av)` 管指标级、`toolReason(t, toolId, av)` 管逐源，**不要合并回一个带
//     `toolId` 的函数**（合并后指标级分支会退化成「某个源的不可得」）。
//  ③ 逐源原因的文案取**该源的回合口径** `usage.turnBasis.<源>`：D16/D19 的逐源不可得根因都是
//     「该源的回合 / 时长判据不可用」（workbuddy 无回合概念、claude 无回合级失败字段…），
//     故 perSource 这一层的解释文案就是该源口径；指标级不可得则用 `usage.work.unavailableAll`。
//     ⚠️ 后端 `reason`（中文散文，如 `部分覆盖` / `mock-empty`）**不直渲**：它是给日志的原文、
//     不随语言切换，直渲会在英文界面里印中文；用户可见的原因一律走本层的 i18n 文案。
//  ④ **登记（2026-10-06 用户裁决）：dsh 的「部分覆盖」不在界面上表达。** 理由有两条：
//     (a) 可得性模型是**布尔两态**（`available` + `perSource`），**没有「部分」这一档**，
//         加一档要动 `UsageAvailability` 的 wire 形状 —— 那是冻结契约，不在本阶段范围内；
//     (b) 本层**不直渲**后端的自由文本 `reason`（见 ③），界面也不该为它另造一套文案。
//     ⇒ 某源只覆盖一部分会话时，界面照常显示它给得出的数字，给不出的位次走 `EM_DASH`
//     （用户裁决原文：「DSH 能提取哪些数据，能显示哪些就显示哪些。不能显示的话，就用斜杠或
//     横杠表示没有这个数据」）。覆盖率本身是可变的运维事实（随机器的会话清理策略变化），
//     不该由界面承担 —— 它留在后端 `reason` 与 §9.5 的审计记录里。
//
// 零 DOM API、零新增依赖（纯函数用例可不进 jsdom，§3 第 34 条）。
import type { TFn } from "@/lib/usage/range";
import type { UsageAvailability, UsageAvailabilityMetric } from "@/types/usage";

/** 取某指标的可得性条目；**没有条目 → `null`**（不等于「不可得」，见文件头 ①） */
export function availabilityOf(
  list: readonly UsageAvailability[] | null | undefined,
  metric: UsageAvailabilityMetric
): UsageAvailability | null {
  return list?.find((a) => a.metric === metric) ?? null;
}

/** 指标级判据：`null`（无条目）按可用处理；只有 `available === false` 才是「所有源都不可得」 */
export function isMetricAvailable(av: UsageAvailability | null): boolean {
  return av === null || av.available;
}

/** 逐源判据：指标级不可得 → 所有源都不可得；否则只看该源的 `perSource` 条目（缺条目 = 可用） */
export function isToolAvailable(av: UsageAvailability | null, toolId: string): boolean {
  if (av === null) return true;
  if (!av.available) return false;
  return av.perSource?.[toolId] ?? true;
}

/**
 * **指标级**空态原因：`available === false` → `usage.work.unavailableAll`（所有来源均不可得）。
 * ⚠️ 本函数**不接 `toolId`**（指标级没有「某个源」这一层，见文件头 ②）。
 * 指标可用时不该调用本函数（空态由「值本身为 `null`」驱动）；真调到时给中性原因
 * `usage.work.unavailable` —— 绝不回 `unavailableAll`（那会把「这个指标只有某几源不可得」
 * 谎报成「所有源都不可得」）。
 */
export function metricReason(t: TFn, av: UsageAvailability | null): string {
  return av !== null && !av.available
    ? t("usage.work.unavailableAll")
    : t("usage.work.unavailable");
}

/**
 * **逐源**空态原因（见文件头 ③）：指标级不可得 → `usage.work.unavailableAll`；
 * 该源 `perSource === false` → 该源回合口径 `usage.turnBasis.<toolId>`；
 * 其余（无条目 / 该源可用）→ 中性 `usage.work.unavailable`。
 */
export function toolReason(t: TFn, toolId: string, av: UsageAvailability | null): string {
  if (av !== null && !av.available) return t("usage.work.unavailableAll");
  if (av?.perSource?.[toolId] === false) return t(`usage.turnBasis.${toolId}`);
  return t("usage.work.unavailable");
}
