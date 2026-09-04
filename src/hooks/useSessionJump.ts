// 会话跳转共享逻辑 — 主界面卡片与通知浮窗复用同一实现（含歧义候选结果）
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { invoke } from "@tauri-apps/api/core";

export interface JumpWindowCandidate {
  hwnd: number;
  title: string;
  process: string;
  score?: number;
  uiaPrefix?: number;
}

export interface JumpTarget {
  pid: number;
  id: string;
  agentType: string;
  projectName: string;
  lastMessage?: string;
  // 未读标记：歧义选择器点选跳转成功后回标已读用（spec W4 已读信号 1）；未知时省略 → 无条件回标（删除不存在的行是 no-op）
  unread?: boolean;
  // 进程形态：CLI 会话在 TTY 聚焦失败走 APP 级保底时给出 UX 提示（review M3）
  form?: "cli" | "app";
}

export function useSessionJump() {
  const { t } = useTranslation();
  const [candidates, setCandidates] = useState<JumpWindowCandidate[] | null>(null);
  // 歧义挂起的跳转目标：focus 弹出候选时暂存，focusHwnd 点选成功后用于回标已读
  const pendingTargetRef = useRef<JumpTarget | null>(null);

  const focus = async (target: JumpTarget): Promise<JumpWindowCandidate[] | null> => {
    const result = await invoke<{ type: string; via?: string; windows?: JumpWindowCandidate[] }>(
      "focus_session",
      {
        pid: target.pid,
        sessionId: target.id,
        agentType: target.agentType,
        projectName: target.projectName,
        lastMessage: target.lastMessage,
        // P1-1：进程形态/未读标记传入后端，Windows App 会话优先深度链接直达（T8）
        form: target.form,
        unread: target.unread,
      }
    );
    const ambiguous = result.type === "ambiguous" && result.windows ? result.windows : null;
    const isAmbiguous = ambiguous !== null && ambiguous.length > 0;
    // review M3：CLI 会话 TTY 聚焦失败 → 后端按 spec 走 APP 级保底激活宿主。
    // 设计内行为，但可能让 CLI 用户困惑（终端没被带起、弹的是宿主 APP）——给一次性提示
    if (!isAmbiguous && target.form === "cli" && result.via === "app-fallback") {
      toast.info(t("sessions.appFallbackHint"));
    }
    // 直达成功时后端已回标已读；仅歧义分支需在点选成功后由前端补标
    pendingTargetRef.current = isAmbiguous ? target : null;
    setCandidates(isAmbiguous ? ambiguous : null);
    return ambiguous;
  };

  const focusHwnd = async (hwnd: number) => {
    await invoke("focus_hwnd", { hwnd });
    // 歧义选择器点选跳转成功 → 仅标记发起跳转的那张卡已读（spec W4 已读信号 1，含 Windows 歧义分支）。
    // fire-and-forget：已读标记失败只记日志，绝不阻塞/破坏跳转 UX
    const target = pendingTargetRef.current;
    pendingTargetRef.current = null;
    if (target && target.unread !== false) {
      invoke("mark_session_read", { agentType: target.agentType, sessionId: target.id }).catch(
        (err) => console.error("mark_session_read failed:", err)
      );
    }
  };

  return { candidates, setCandidates, focus, focusHwnd };
}
