// 用量域前端错误码白名单——与 Rust 的 USAGE_CODES（services/usage/error.rs）及
// locales 的 usage.rpc.* **三处一一对应**（同 pet 的 KNOWN_RPC_CODES 纪律）。
export const KNOWN_USAGE_CODES = [
  "usage-db-failed",
  "usage-source-io",
  "usage-source-db-open",
  "usage-range-invalid",
  "usage-groupby-invalid",
  "usage-settings-invalid",
  "usage-disabled",
  "usage-internal",
] as const;

export interface UsageRpcErrorLike {
  code: string;
  detail?: string;
}

export function isUsageRpcError(e: unknown): e is UsageRpcErrorLike {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    (e as { code: string }).code !== ""
  );
}

/** UsageError → t("usage.rpc.<code>")；未知码收敛 internal（不依赖 i18next 缺键回键名） */
export function usageErrMsg(
  e: unknown,
  t: (k: string, p?: Record<string, unknown>) => string
): string {
  if (isUsageRpcError(e)) {
    if (!(KNOWN_USAGE_CODES as readonly string[]).includes(e.code)) {
      return t("usage.rpc.usage-internal", { err: e.detail ?? "" });
    }
    return t(`usage.rpc.${e.code}`, { err: e.detail ?? "" });
  }
  if (e instanceof Error && e.message) return e.message;
  return t("usage.rpc.usage-internal", { err: "" });
}
