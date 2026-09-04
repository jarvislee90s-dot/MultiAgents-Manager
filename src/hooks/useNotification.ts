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
import { getAgentLabel } from "@/lib/agentBadge";
import { addHistory } from "@/lib/notificationHistory";
import { petSoundTakeover, petSuppressPopup } from "@/components/pet/petConfig";
import type { Session } from "@/types/session";

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

// 首见未读卡补发通知的新鲜度门控（review F5）：重启/补偿场景下，转绿时间
// （lastActivityAt）距今超过 2 分钟的老卡静默显示，不重放历史通知。
// 时间无法解析（空串/异常）时保守静默
export const FIRST_SEEN_UNREAD_FRESH_MS = 2 * 60 * 1000;

export function isFreshFirstSeenUnread(
  session: Pick<Session, "unread" | "status" | "lastActivityAt">,
  nowMs: number = Date.now()
): boolean {
  if (!session.unread || statusToColor(session.status) !== "green") {
    return false;
  }
  const greenAt = Date.parse(session.lastActivityAt);
  if (Number.isNaN(greenAt)) {
    return false;
  }
  return nowMs - greenAt <= FIRST_SEEN_UNREAD_FRESH_MS;
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
      // 统一通知流：开关刷新 → 历史记录 → 提示音 → 浮窗/系统降级
      // 供两处调用：常规颜色变化 + 首见未读绿卡补偿通知
      const notifyCompletion = async (session: Session) => {
        // 每次通知前刷新通知开关设置（支持运行时切换）
        try {
          const val = await invoke<string | null>("get_setting", { key: "notifications_enabled" });
          notificationsEnabled.current = val !== "false";
        } catch {
          // 忽略错误
        }
        if (!notificationsEnabled.current) return;

        const currColor = statusToColor(session.status);

        // 记录到通知历史（spec 014）
        addHistory({
          agentType: session.agentType,
          form: session.form,
          projectName: session.projectName,
          status: session.status,
          lastMessage: session.lastMessage ?? "",
          pid: session.pid,
          sessionId: session.id,
          at: Date.now(),
        });

        // 方向过滤：仅变为绿（任务完成）时按工具播放提示音
        // 宠物开启即接管完成提示音（静音则整体静默，spec D3）
        if (currColor === "green" && !petSoundTakeover()) playCompletionSound(session.agentType);
        // 记录本次通知用于时间去重
        lastNotified.current.set(session.id, { color: currColor, at: Date.now() });

        // 发送通知：应用内浮窗为主路径，失败降级系统 toast（两者都在宠物压制守卫内）
        // 宠物可见时全部静默：头顶气泡是唯一通知面（spec W1）
        if (!petSuppressPopup()) {
          const toolLabel = getAgentLabel(session.agentType, session.form);
          const statusLabel = STATUS_LABELS[session.status] ?? session.status;
          try {
            await invoke("show_notification_window", {
              payload: {
                agentType: session.agentType,
                agentLabel: toolLabel,
                projectName: session.projectName,
                statusColor: currColor,
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
                title: `${toolLabel} — ${session.projectName}`,
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
      };

      for (const session of sessions) {
        const prev = prevStatuses.current.get(session.id);

        // 首次加载不通知——除非是「未读绿卡」：补偿/重启场景下它从未被观测过转绿，
        // 需补一次完成通知（spec W4；5 秒同色去重防双弹）
        if (!prev) {
          prevStatuses.current.set(session.id, {
            status: session.status,
            color: statusToColor(session.status),
            at: Date.now(),
          });
          // review F5：新鲜度门控——老未读卡（转绿 > 2 分钟）静默显示，不重放通知
          if (isFreshFirstSeenUnread(session)) {
            const notified = lastNotified.current.get(session.id);
            if (!notified || notified.color !== "green" || Date.now() - notified.at >= 5000) {
              // 走统一通知流（与常规颜色变化同一路径）
              await notifyCompletion(session);
            }
          }
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

        // 颜色变化 → 走统一通知流
        await notifyCompletion(session);
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
