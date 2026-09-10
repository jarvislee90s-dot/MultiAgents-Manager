import type { Session } from "@/types/session";

/**
 * 会话标题空值归一（issue #45）：`Session.title` 是 `string | null`，各跳转入口
 * 此前按各自数据来源即兴选择 `??` / `||` / 原样，语义发散。统一收敛：null / 空串 /
 * 全空白 → undefined（后端 `title_keys` 亦过滤空键，此处归一使 TS 层口径一致）。
 * 非 Session 来源不走此函数：通知浮窗 payload 的 title 必填 string（`|| undefined`
 * 语义本就不同）、Rust `NotificationPayload.title: String` 必填（`?? ""` 兜底）
 */
export function sessionTitleOrUndefined(session: Pick<Session, "title">): string | undefined {
  const t = session.title;
  return t && t.trim() !== "" ? t : undefined;
}
