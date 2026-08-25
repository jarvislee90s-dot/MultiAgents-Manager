import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";

interface NotificationPayload {
  agentType: string;
  agentLabel: string;
  projectName: string;
  statusColor: string;
  status: string;
  lastMessage: string;
  pid: number;
  sessionId: string;
}

interface WindowCandidate {
  hwnd: number;
  title: string;
  process: string;
  score?: number;
}

// 自定义通知浮窗页面 — 由 Rust 侧 show_notification_window 创建的独立小窗加载，
// 收到 notification:new 事件后显示，6 秒无交互自动隐藏（悬停保留）
export default function NotificationPage() {
  const [payload, setPayload] = useState<NotificationPayload | null>(null);
  const [candidates, setCandidates] = useState<WindowCandidate[] | null>(null);
  const { t } = useTranslation();
  const timerRef = useRef<number | null>(null);

  // 自动隐藏计时器（组件级，参数化时长）：通知卡片与候选列表复用
  const armTimer = (ms: number) => {
    if (timerRef.current) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => getCurrentWindow().hide(), ms);
  };

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | null = null;
    // 监听必须限定本窗口：不指定 target 时监听器目标为 Any，会收到所有槽位窗口的
    // notification:new（emit_to 定向发送被 Any 监听器全收），导致每个浮窗都显示最后一条内容
    listen<NotificationPayload>(
      "notification:new",
      (e) => {
        setPayload(e.payload);
        // 新通知到达时清掉旧候选列表，避免与卡片同时出现
        setCandidates(null);
        win.show();
        armTimer(6000);
      },
      { target: win.label }
    ).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  if (!payload) return <div className="h-full w-full" />;

  // 点击通知卡 → 隐藏浮窗并跳转到对应会话终端
  const jump = async () => {
    getCurrentWindow().hide();
    try {
      const result = await invoke<{ type: string; windows?: WindowCandidate[] }>("focus_session", {
        pid: payload.pid,
        sessionId: payload.sessionId,
        agentType: payload.agentType,
        projectName: payload.projectName,
      });
      // 多窗口歧义：在通知窗内联渲染候选，避免静默失败
      if (result.type === "ambiguous" && result.windows && result.windows.length > 0) {
        setCandidates(result.windows);
        getCurrentWindow().show();
        // 候选列表不操作 15 秒自动隐藏，避免无限驻留
        armTimer(15000);
      }
    } catch {
      // 跳转失败不弹新提示（通知窗口环境无 toast 容器）
    }
  };

  return (
    <>
      {payload && !candidates && (
        <div
          className="bg-card flex h-screen w-screen cursor-pointer items-center gap-3 rounded-lg border p-3 shadow-2xl"
          onMouseEnter={() => timerRef.current && window.clearTimeout(timerRef.current)}
          onMouseLeave={() => armTimer(3000)}
          onClick={jump}
        >
          <span
            className="h-3 w-3 shrink-0 rounded-full"
            style={{ background: payload.statusColor }}
          />
          <div className="min-w-0 flex-1">
            <p className="truncate text-xs font-semibold">
              {payload.agentLabel} · {payload.projectName} ·{" "}
              {t(`sessions.statusLabels.${payload.status}`, payload.status)}
            </p>
            <p className="mt-1 line-clamp-2 text-[11px] opacity-70">{payload.lastMessage}</p>
          </div>
        </div>
      )}
      {candidates && (
        <div
          className="bg-card flex h-screen w-screen flex-col gap-1 rounded-lg border p-3 shadow-2xl"
          onMouseEnter={() => timerRef.current && window.clearTimeout(timerRef.current)}
          onMouseLeave={() => armTimer(5000)}
        >
          <p className="text-xs font-semibold">{t("sessions.pickWindow")}</p>
          {candidates.map((w) => (
            <button
              key={w.hwnd}
              className="hover:bg-accent truncate rounded border px-2 py-1.5 text-left text-[11px]"
              title={w.title}
              onClick={() => {
                setCandidates(null);
                getCurrentWindow().hide();
                invoke("focus_hwnd", { hwnd: w.hwnd }).catch(() => {});
              }}
            >
              {w.title || t("sessions.untitledWindow")} — {w.process}
            </button>
          ))}
        </div>
      )}
    </>
  );
}
