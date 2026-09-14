import { useCallback, useEffect, useState } from "react";
import { fetchSessions } from "./api";
import type { SessionsResponse } from "@/types/session";
import PairPage from "./PairPage";

// 配对状态机：sessions=null 即未配对（含 403 / 网络异常）→ 出配对页；
// 探测完成前由 PairPage 承载（探测与配对共用同一入口，首帧即出配对页不闪白）。
// 已配对态为极简占位，看板 Board 由 Task 6 接管。
export default function App() {
  const [sessions, setSessions] = useState<SessionsResponse | null>(null);

  const probe = useCallback(async () => {
    const s = await fetchSessions<SessionsResponse>().catch(() => null);
    setSessions(s);
  }, []);

  useEffect(() => {
    void probe();
  }, [probe]);

  if (sessions === null) {
    return <PairPage onPaired={() => void probe()} />;
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-slate-950 text-slate-200">
      {/* Task 6 占位：Board 将替换此文案 */}
      <p>已接入 · 看板加载中…（{sessions.totalCount} 个会话）</p>
    </div>
  );
}
