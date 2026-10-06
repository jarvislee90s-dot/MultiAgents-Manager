// 评语池语言判定（计划② Task 12 步骤 3）——`i18n.language` → 池语言的**唯一**出处。
// 消费方 = `useExportShare.ts`（`pickQuote(agg, getLang(), …)`）；导出层不引 React，故这里只依赖
// i18n 单例本身（`src/i18n/index.ts`，`fallbackLng: "en"`）。
//
// 为什么要有这一层：评语池是**文案资产**（不进 i18n 键表，见 `quotePool.ts` 文件头），
// 它必须自己知道当前说什么语言；`i18n.language` 形如 `zh` / `zh-CN` / `en-US`，与池的两个桶对不齐。
import i18n from "@/i18n";

/** 评语池的两个桶（与 `i18n` 的 `resources` key 同名同义） */
export type QuoteLang = "zh" | "en";

/**
 * 当前界面语言 → 评语池语言：`zh*` 一律归 `zh`（`zh-CN` / `zh-TW` 都是中文），其余一律 `en`
 * （与 `fallbackLng: "en"` 同口径，绝不因第三种语言而掉进「没有池」的空档）。
 */
export function getLang(): QuoteLang {
  return (i18n.language ?? "").toLowerCase().startsWith("zh") ? "zh" : "en";
}
