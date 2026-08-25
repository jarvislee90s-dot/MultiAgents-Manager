import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  sendNotification,
  isPermissionGranted,
  requestPermission,
  onAction,
  registerActionTypes,
} from "@tauri-apps/plugin-notification";
import { useSessionStore } from "@/stores/sessionStore";
import { playCompletionSound } from "@/lib/audio";
import { addHistory } from "@/lib/notificationHistory";

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
    case "waiting":
      return "red";
    case "processing":
    case "thinking":
    case "compacting":
      return "yellow";
    case "idle":
    case "finished":
      return "green";
    default:
      return "gray";
  }
}

export function useNotification() {
  const sessions = useSessionStore((s) => s.sessions);
  const prevStatuses = useRef<Map<string, { status: string; color: string; at: number }>>(
    new Map()
  );
  // 上次实际通知记录：同会话 5 秒内重复翻转到同一目标颜色只弹一次（兜底状态抖动）
  const lastNotified = useRef<Map<string, { color: string; at: number }>>(new Map());
  const permissionGranted = useRef(false);
  const notificationsEnabled = useRef(true);

  // 初始化：请求通知权限 + 读取设置
  useEffect(() => {
    const init = async () => {
      // 渠道统一（spec 014）：清理旧系统通知开关的遗留键
      localStorage.removeItem("mam.useSystemNotification");
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
        const enabled = await invoke<string | null>("get_setting", {
          key: "notifications_enabled",
        });
        notificationsEnabled.current = enabled !== "false";
      } catch {
        notificationsEnabled.current = true;
      }

      // 注册"查看会话"通知 action + 监听点击（满足 FR-2 #12）
      try {
        await registerActionTypes([
          {
            id: "focus-session",
            actions: [{ id: "focus", title: "查看会话" }],
          },
        ]);
        await onAction(async (notification) => {
          if (notification.actionTypeId !== "focus-session") return;
          const pid = (notification.extra?.pid as number) ?? 0;
          if (pid > 0) {
            try {
              await invoke("focus_session", {
                pid,
                sessionId: (notification.extra?.sessionId as string) ?? undefined,
                agentType: (notification.extra?.agentType as string) ?? undefined,
                projectName: (notification.extra?.projectName as string) ?? undefined,
                lastMessage: (notification.extra?.lastMessage as string) ?? undefined,
              });
            } catch (e) {
              console.error("focus_session failed:", e);
            }
          }
        });
      } catch (e) {
        console.error("register action types failed:", e);
      }
    };
    init();
  }, []);

  useEffect(() => {
    (async () => {
      for (const session of sessions) {
        const prev = prevStatuses.current.get(session.id);

        // 首次加载不通知
        if (!prev) {
          prevStatuses.current.set(session.id, {
            status: session.status,
            color: statusToColor(session.status),
            at: Date.now(),
          });
          continue;
        }

        const currColor = statusToColor(session.status);
        prevStatuses.current.set(session.id, {
          status: session.status,
          color: currColor,
          at: Date.now(),
        });

        // 颜色未变 → 不通知（即使状态变了，如 Processing → Thinking 都是黄色）
        if (prev.color === currColor) continue;

        // 时间去重：5 秒内同目标颜色不重复弹（兜底状态抖动）
        const notified = lastNotified.current.get(session.id);
        if (notified && notified.color === currColor && Date.now() - notified.at < 5000) {
          continue;
        }

        // 通知
        // 每次轮询时刷新通知开关设置（支持运行时切换）
        try {
          const val = await invoke<string | null>("get_setting", { key: "notifications_enabled" });
          notificationsEnabled.current = val !== "false";
        } catch {
          // 忽略错误
        }
        if (!notificationsEnabled.current) continue;

        // 颜色变化时通知（红→黄→绿 任意切换）；记录本次通知用于时间去重

        // 记录到通知历史（spec 014）
        addHistory({
          agentType: session.agentType,
          projectName: session.projectName,
          status: session.status,
          lastMessage: session.lastMessage ?? "",
          pid: session.pid,
          sessionId: session.id,
          at: Date.now(),
        });

        // 方向过滤：仅变为绿（任务完成）时按工具播放提示音
        if (currColor === "green") playCompletionSound(session.agentType);
        lastNotified.current.set(session.id, { color: currColor, at: Date.now() });

        // 发送通知：应用内浮窗为唯一主路径（spec 014 渠道统一），失败降级系统 toast
        {
          const toolLabel = AGENT_LABELS[session.agentType] ?? session.agentType;
          const statusLabel = STATUS_LABELS[session.status] ?? session.status;
          const formTag = session.form === "app" ? " (APP)" : "";
          try {
            await invoke("show_notification_window", {
              payload: {
                agentType: session.agentType,
                agentLabel: toolLabel,
                projectName: session.projectName,
                statusColor: statusToColor(session.status),
                status: session.status,
                lastMessage: session.lastMessage ?? "",
                pid: session.pid,
                sessionId: session.id,
              },
            });
          } catch (e) {
            // 浮窗失败降级系统 toast，保证通知不丢（spec 008 错误处理要求）
            console.error("show_notification_window failed:", e);
            if (permissionGranted.current) {
              sendNotification({
                title: `${toolLabel}${formTag} — ${session.projectName}`,
                body: `${statusLabel}${session.lastMessage ? ": " + session.lastMessage.slice(0, 80) : ""}`,
                actionTypeId: "focus-session",
                extra: {
                  pid: session.pid,
                  sessionId: session.id,
                  agentType: session.agentType,
                  projectName: session.projectName,
                  lastMessage: session.lastMessage ?? "",
                },
              });
            }
          }
        }
      }

      // 清理已消失的会话
      const activeIds = new Set(sessions.map((s) => s.id));
      for (const id of prevStatuses.current.keys()) {
        if (!activeIds.has(id)) {
          prevStatuses.current.delete(id);
          lastNotified.current.delete(id);
        }
      }
    })();
  }, [sessions]);
}
