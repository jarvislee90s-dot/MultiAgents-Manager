// 宠物系统前端错误码 — 错误码化 + i18n 映射（P3-6 债务清理 + 第六轮 RPC 错误接线）。
// PetError 携带结构化 code/params，展示层经 petErrMsg 按当前语言翻译；
// 非 PetError 的普通 Error（含 Rust 侧透传消息）保持 message 原样。
// audio-* 三个错误码当前经 pet.import.problems.no-duration 路径等效展示，键位为前瞻保留。
// PetRpcErrorLike 对应 Rust 侧 PetRpcError 的 serde 形状（camelCase），code 映射 pet.rpc.<code>。
export type PetErrCode =
  | "sheet-missing"
  | "sheet-bad-size"
  | "sheet-load-fail"
  | "audio-timeout"
  | "audio-bad-duration"
  | "audio-load-fail"
  | "scan-fail";

export class PetError extends Error {
  constructor(
    public code: PetErrCode,
    public params?: Record<string, string | number>
  ) {
    super(code);
  }
}

/** Rust 侧 PetRpcError 经 Tauri reject 到前端的形状（camelCase serde） */
export interface PetRpcErrorLike {
  code: string;
  params?: Record<string, string>;
  detail?: string;
}

/** 已知 RpcError 码白名单（与 locales 的 pet.rpc.* 键一一对应，防止两处漂移） */
export const KNOWN_RPC_CODES = [
  "audio-format-unsupported",
  "audio-not-found",
  "audio-relpath-invalid",
  "copy-failed",
  "delete-failed",
  "download-failed",
  "download-status",
  "download-url-invalid",
  "finalize-move-failed",
  "finalize-scan-failed",
  "group-invalid",
  "host-forbidden",
  "internal",
  "manifest-backup-failed",
  "manifest-parse-failed",
  "manifest-request-failed",
  "manifest-status",
  "manifest-write-failed",
  "pet-dir-missing",
  "pet-exists",
  "pet-name-dot-prefix",
  "pet-name-empty",
  "pet-name-illegal",
  "pet-name-reserved",
  "pet-not-found",
  "pet-not-on-petdex",
  "petdex-no-zip",
  "redirect-forbidden",
  "redirect-too-many",
  "rename-failed",
  "reveal-failed",
  "sheet-not-found",
  "slug-parse-failed",
  "source-not-folder",
  "staging-create-failed",
  "staging-id-invalid",
  "staging-missing-sheet",
  "staging-not-found",
  "tmp-write-failed",
  "zip-entry-illegal-path",
  "zip-open-failed",
  "zip-read-failed",
  "zip-too-many-entries",
  "zip-total-over-limit",
] as const;

export function isPetRpcError(e: unknown): e is PetRpcErrorLike {
  // PetError 也携带 string 类型的 code 属性，必须在开头显式排除：
  // 否则 fatal 分流会把图集类 PetError 误判为 rpc 走 fatalScan 前缀（第八轮）
  if (e instanceof PetError) return false;
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { code?: unknown }).code === "string" &&
    !((e as { code?: unknown }).code === "")
  );
}

/** PetError → t("pet.err.<code>", params)；RpcError → t("pet.rpc.<code>", params)；
 *  普通 Error → message 透传；非对象/空 message → scan-fail 兜底 */
export function petErrMsg(
  e: unknown,
  t: (k: string, p?: Record<string, unknown>) => string
): string {
  if (e instanceof PetError) return t(`pet.err.${e.code}`, e.params);
  if (isPetRpcError(e)) {
    // 白名单判定（第七轮）：不依赖 i18next 缺键回键名的默认行为，未知码显式收敛 internal
    if (!(KNOWN_RPC_CODES as readonly string[]).includes(e.code)) {
      return t("pet.rpc.internal", { err: e.detail ?? "" });
    }
    return t(`pet.rpc.${e.code}`, e.params ?? {});
  }
  if (e instanceof Error && e.message) return e.message;
  return t("pet.err.scan-fail");
}
