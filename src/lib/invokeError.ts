// invoke 错误的本地化渲染（issue #36-3）：后端守卫（services/tool_settings.rs
// ensure_tool_enabled_conn）返回结构化错误码 `W5_TOOL_DISABLED:<tool_id>`，
// 此处映射为 i18n 文案；其余错误原样透出（后端既有中文文案错误逐批迁移，
// 未迁移前英文用户仍会看到原文）。

type TranslateFn = (key: string, options?: Record<string, unknown>) => string;

const TOOL_DISABLED_CODE = /^W5_TOOL_DISABLED:(.+)$/;
const APPLY_IN_PROGRESS_CODE = /^W5_APPLY_IN_PROGRESS/;
const APPLY_TASK_FAILED_CODE = /^W5_APPLY_TASK_FAILED/;

export function formatInvokeError(e: unknown, t: TranslateFn): string {
  const raw = typeof e === "string" ? e : String(e);
  const m = raw.match(TOOL_DISABLED_CODE);
  if (m) return t("errors.toolDisabled", { tools: m[1] });
  if (APPLY_IN_PROGRESS_CODE.test(raw)) return t("errors.applyInProgress");
  if (APPLY_TASK_FAILED_CODE.test(raw)) return t("errors.applyTaskFailed");
  return raw;
}
