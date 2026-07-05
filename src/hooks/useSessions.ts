import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SessionsResponse } from "@/types/session";
import { useSessionStore } from "@/stores/sessionStore";

// 轮询间隔：活跃会话 1s，空闲会话 3s
const POLL_INTERVAL = 1500;

export function useSessions() {
  const setSessions = useSessionStore((s) => s.setSessions);

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      try {
        const response = await invoke<SessionsResponse>("get_all_sessions");
        if (!cancelled) setSessions(response);
      } catch (e) {
        console.error("Failed to get sessions:", e);
      }
    };

    poll();
    const interval = setInterval(poll, POLL_INTERVAL);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [setSessions]);
}
