// 宠物系统前端错误码 — 错误码化 + i18n 映射（P3-6 债务清理）。
// PetError 携带结构化 code/params，展示层经 petErrMsg 按当前语言翻译；
// 非 PetError 的普通 Error（含 Rust 侧透传消息）保持 message 原样。
// audio-* 三个错误码当前经 pet.import.problems.no-duration 路径等效展示，键位为前瞻保留。
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

/** PetError → t("pet.err.<code>", params)；普通 Error → message 透传；非对象 → scan-fail 兜底 */
export function petErrMsg(
  e: unknown,
  t: (k: string, p?: Record<string, unknown>) => string
): string {
  if (e instanceof PetError) return t(`pet.err.${e.code}`, e.params);
  if (e instanceof Error && e.message) return e.message;
  return t("pet.err.scan-fail");
}
