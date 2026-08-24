import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface NotificationPayload {
  agentType: string;
  projectName: string;
  statusColor: string;
  statusLabel: string;
  lastMessage: string;
  pid: number;
  sessionId: string;
}

// 自定义通知浮窗页面 — 由 Rust 侧 show_notification_window 创建的独立小窗加载，
// 收到 notification:new 事件后显示，6 秒无交互自动隐藏（悬停保留）
export default function NotificationPage() {
  const [payload, setPayload] = useState<NotificationPayload | null>(null);
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | null = null;
    // 自动隐藏计时器：每次新通知重置
    const armTimer = () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => win.hide(), 6000);
    };
    listen<NotificationPayload>("notification:new", (e) => {
      setPayload(e.payload);
      win.show();
      armTimer();
    }).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  if (!payload) return <div className="h-full w-full" />;

  // 点击通知卡 → 隐藏浮窗并跳转到对应会话终端
  const jump = async () => {
    getCurrentWindow().hide();
    try {
      await invoke("focus_session", {
        pid: payload.pid,
        sessionId: payload.sessionId,
        agentType: payload.agentType,
        projectName: payload.projectName,
      });
    } catch {
      // 跳转失败不弹新提示（通知窗口环境无 toast 容器）
    }
  };

  return (
    <div
      className="flex h-screen w-screen cursor-pointer items-center gap-3 rounded-lg border bg-card p-3 shadow-2xl"
      onMouseEnter={() => timerRef.current && window.clearTimeout(timerRef.current)}
      onMouseLeave={() => {
        if (timerRef.current) window.clearTimeout(timerRef.current);
        timerRef.current = window.setTimeout(() => getCurrentWindow().hide(), 3000);
      }}
      onClick={jump}
    >
      <span
        className="h-3 w-3 shrink-0 rounded-full"
        style={{ background: payload.statusColor }}
      />
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-semibold">
          {payload.agentType} · {payload.projectName} · {payload.statusLabel}
        </p>
        <p className="mt-1 line-clamp-2 text-[11px] opacity-70">{payload.lastMessage}</p>
      </div>
    </div>
  );
}
