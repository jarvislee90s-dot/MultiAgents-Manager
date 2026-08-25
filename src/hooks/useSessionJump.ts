// 会话跳转共享逻辑 — 主界面卡片与通知浮窗复用同一实现（含歧义候选结果）
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface JumpWindowCandidate {
  hwnd: number;
  title: string;
  process: string;
  score?: number;
}

export interface JumpTarget {
  pid: number;
  id: string;
  agentType: string;
  projectName: string;
}

export function useSessionJump() {
  const [candidates, setCandidates] = useState<JumpWindowCandidate[] | null>(null);

  const focus = async (target: JumpTarget): Promise<JumpWindowCandidate[] | null> => {
    const result = await invoke<{ type: string; windows?: JumpWindowCandidate[] }>(
      "focus_session",
      {
        pid: target.pid,
        sessionId: target.id,
        agentType: target.agentType,
        projectName: target.projectName,
      }
    );
    const ambiguous = result.type === "ambiguous" && result.windows ? result.windows : null;
    setCandidates(ambiguous && ambiguous.length > 0 ? ambiguous : null);
    return ambiguous;
  };

  const focusHwnd = async (hwnd: number) => {
    await invoke("focus_hwnd", { hwnd });
  };

  return { candidates, setCandidates, focus, focusHwnd };
}
