import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { sendNotification, isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";
import { useSessionStore } from "@/stores/sessionStore";
import { playSoundForStatus } from "@/lib/audio";

const AGENT_LABELS: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex CLI",
  opencode: "OpenCode",
};

const STATUS_LABELS: Record<string, string> = {
  waiting: "等待操作",
  processing: "运行中",
  thinking: "思考中",
  compacting: "压缩中",
  idle: "空闲",
  finished: "已结束",
};

// 通知去重：同一会话同一状态 5 秒内不重复
// 状态 → 颜色映射（三色：红/黄/绿）
function statusToColor(status: string): string {
  switch (status) {
    case "waiting": return "red";
    case "processing":
    case "thinking":
    case "compacting": return "yellow";
    case "idle":
    case "finished": return "green";
    default: return "gray";
  }
}

export function useNotification() {
  const sessions = useSessionStore((s) => s.sessions);
  const prevStatuses = useRef<Map<string, string>>(new Map());
  const permissionGranted = useRef(false);
  const notificationsEnabled = useRef(true);

  // 初始化：请求通知权限 + 读取设置
  useEffect(() => {
    const init = async () => {
      try {
        let granted = await isPermissionGranted();
        if (!granted) {
          const permission = await requestPermission();
          granted = permission === "granted";
        }
        permissionGranted.current = granted;
      } catch (e) {
        console.error("Notification permission error:", e);
      }



      // 读取通知开关设置
      try {
        const enabled = await invoke<string | null>("get_setting", { key: "notifications_enabled" });
        notificationsEnabled.current = enabled !== "false";
      } catch {
        notificationsEnabled.current = true;
      }
    };
    init();
  }, []);

  useEffect(() => {
    (async () => {
    for (const session of sessions) {
      const prevStatus = prevStatuses.current.get(session.id);

      // 首次加载不通知
      if (!prevStatus) {
        prevStatuses.current.set(session.id, session.status);
        continue;
      }

      // 比较颜色变化（非状态变化）
      const prevColor = statusToColor(prevStatus);
      const currColor = statusToColor(session.status);

      // 颜色未变 → 不通知（即使状态变了，如 Processing → Thinking 都是黄色）
      if (prevColor === currColor) {
        prevStatuses.current.set(session.id, session.status);
        continue;
      }

      // 更新上一个状态
      prevStatuses.current.set(session.id, session.status);

      // 通知
      // 每次轮询时刷新通知开关设置（支持运行时切换）
      try {
        const val = await invoke<string | null>("get_setting", { key: "notifications_enabled" });
        notificationsEnabled.current = val !== "false";
      } catch {}
      if (!notificationsEnabled.current) continue;

      // 颜色变化时通知（红→黄→绿 任意切换）

      // 播放提示音
      playSoundForStatus(session.status);

      // 发送桌面通知
      if (permissionGranted.current) {
        const toolLabel = AGENT_LABELS[session.agentType] ?? session.agentType;
        const statusLabel = STATUS_LABELS[session.status] ?? session.status;
        const formTag = session.form === "app" ? " (APP)" : "";

        sendNotification({
          title: `${toolLabel}${formTag} — ${session.projectName}`,
          body: `${statusLabel}${session.lastMessage ? ": " + session.lastMessage.slice(0, 80) : ""}`,
        });
      }
    }

    // 清理已消失的会话
    const activeIds = new Set(sessions.map((s) => s.id));
    for (const id of prevStatuses.current.keys()) {
      if (!activeIds.has(id)) {
        prevStatuses.current.delete(id);
      }
    }
    })();
  }, [sessions]);
}
