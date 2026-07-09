import { invoke } from "@tauri-apps/api/core";
export async function getAllSessions() { return await invoke("get_all_sessions"); }
export async function focusSession(pid: number) { return await invoke("focus_session", { pid }); }
export async function killSession(pid: number) { return await invoke("kill_session", { pid }); }
