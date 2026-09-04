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

export function isPetRpcError(e: unknown): e is PetRpcErrorLike {
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
    const key = `pet.rpc.${e.code}`;
    const msg = t(key, e.params ?? {});
    // 未知 code（i18n 键缺失时 i18next 返回键名）：回退 internal 并保留开发者原文
    if (msg === key) return t("pet.rpc.internal", { err: e.detail ?? "" });
    return msg;
  }
  if (e instanceof Error && e.message) return e.message;
  return t("pet.err.scan-fail");
}