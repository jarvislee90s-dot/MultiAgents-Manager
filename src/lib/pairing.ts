import type { Session } from "@/types/session";

/**
 * 配对不确定判定（issue #48）：同工具同项目（目录名，忽略大小写——Windows 路径
 * 不区分大小写）≥2 个会话同时在跑时，卡片 pid 与会话的启发式绑定（kimi=wire
 * mtime 最新 / opencode=time_updated DESC）可能互换，status / lastMessage 可能
 * 串卡。跳转侧已由 Rust `require_positive_evidence` 门控（禁演绎锁定、只认正向
 * 证据）；此处仅供卡片角标提示，口径与 commands/session.rs 的判定一致。
 * 空项目名不参与（无区分意义）
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
