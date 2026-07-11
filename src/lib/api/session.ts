import { invoke } from "@tauri-apps/api/core";
import type { SessionsResponse } from "@/types/session";

export async function getAllSessions(): Promise<SessionsResponse> {
  return await invoke<SessionsResponse>("get_all_sessions");
}
export async function focusSession(pid: number): Promise<void> {
  return await invoke("focus_session", { pid });
}
export async function killSession(pid: number): Promise<void> {
  return await invoke("kill_session", { pid });
}
