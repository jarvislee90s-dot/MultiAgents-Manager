import { invoke } from "@tauri-apps/api/core";
import type { AgentType, SessionsResponse } from "@/types/session";

export async function getAllSessions(): Promise<SessionsResponse> {
  return await invoke<SessionsResponse>("get_all_sessions");
}
export async function focusSession(pid: number): Promise<void> {
  return await invoke("focus_session", { pid });
}
export async function killSession(pid: number): Promise<void> {
  return await invoke("kill_session", { pid });
}

// ==== M6R–M9R Task 11：R5 一键 resume（在电脑上打开）====

/** 支持「在电脑上打开」的工具镜像表：SSOT = src-tauri/src/inject/resume.rs 的
 *  RESUME_TABLE（Step 1 实测取证），后端查证新工具回填后**两处必须同步**——
 *  前端镜像仅驱动会话卡按钮禁用态（无映射工具不出键，原因见 resume.noCmd） */
export const RESUME_CAPABLE_TOOLS: ReadonlySet<AgentType> = new Set([
  "claude",
  "codex",
  "kimi",
  "opencode",
]);

/** 一键 resume：本机自动打开终端 + 进入项目目录 + 恢复该会话并聚焦。
 *  后端哨兵错误（no_resume_command / no_cwd）已在命令侧转译为中文文案，直接 toast 展示 */
export async function sessionOpen(sessionId: string): Promise<void> {
  return await invoke("session_open", { sessionId });
}
