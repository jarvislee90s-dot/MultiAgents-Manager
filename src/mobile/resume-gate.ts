// 「在桌面端打开」可用性门（R5 Task 11 原生于 SessionDetail，2026-09-20 归档区
// 复用迁出为独立模块）。SSOT = src-tauri/src/inject/resume.rs 的 RESUME_TABLE——
// 后端查证新工具回填后**两处必须同步**（本镜像仅驱动按钮禁用态）。
export const RESUME_SUPPORTED_TOOLS: ReadonlySet<string> = new Set([
  "claude",
  "codex",
  "kimi",
  "opencode",
]);

/** 禁用原因（中文内联，移动端无 i18n 契约）：null = 可用 */
export function resumeUnavailableReason(session: {
  projectPath?: string | null;
  agentType: string;
}): string | null {
  if (!session.projectPath || !session.projectPath.trim()) {
    return "该会话没有项目目录信息，无法在电脑上打开";
  }
  if (!RESUME_SUPPORTED_TOOLS.has(session.agentType)) {
    return "该工具 resume 命令待查证，暂不支持一键打开";
  }
  return null;
}
