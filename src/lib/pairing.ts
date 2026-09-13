import type { Session } from "@/types/session";

/**
 * 配对不确定判定（issue #48）：同工具同项目（目录名，忽略大小写——Windows 路径
 * 不区分大小写）≥2 张会话卡时，pid 与会话的启发式绑定（kimi=wire mtime 最新 /
 * opencode=time_updated DESC）可能互换，status / lastMessage 可能串卡。
 * 空项目名不参与（无区分意义）。
 *
 * 口径说明：这是按【会话卡】计数的平台无关近似口径，与 Rust 跳转门控
 * `require_positive_evidence`（commands/session.rs：按【进程】计数、仅 5 个工具、
 * 仅 Windows 生效）存在已知偏差——本侧包含数据驱动滞留绿卡与 pid=0 未读卡，
 * zcode/workbuddy 双开也会出角标而 Rust 不会为其门控；反方向（门控开而角标无）
 * 则源于两侧数据源天然不同（渲染时会话列表 vs 跳转时进程快照）。角标只承诺
 * 「卡片信息可能互换」的提示，不承诺跳转行为；跳转安全由 Rust 门控独立保证。
 */
export function pairingAmbiguitySet(sessions: Session[]): Set<string> {
  const counts = new Map<string, number>();
  for (const s of sessions) {
    if (!s.projectName) continue;
    const key = pairingKey(s);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  const ambiguous = new Set<string>();
  for (const [key, n] of counts) {
    if (n >= 2) ambiguous.add(key);
  }
  return ambiguous;
}

export function pairingKey(s: Pick<Session, "agentType" | "projectName">): string {
  return `${s.agentType}|${s.projectName.toLowerCase()}`;
}
