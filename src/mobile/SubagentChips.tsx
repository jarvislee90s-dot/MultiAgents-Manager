import { useEffect, useState } from "react";
import { fmtTokens } from "@/lib/usage/format";
import type { Session } from "@/types/session";
import { fetchSessionSubagents, type SubagentView } from "./api";

/** 运行中子 agent chip 区（spec §6）：ModeBar 兄弟节点，同一行 flex-wrap。
 *
 * - 拉取随 SessionDetail 的 refreshTick（DETAIL_REFRESH_MS 轮询——免费继承
 *   hidden 暂停/恢复补刷）独立拉取；空列表 = 端点权威，不渲染；
 * - 时长自走：tick 状态收在本组件（1s interval），不拖整页每秒重渲染；
 *   下轮拉取用服务端 spawnTs 校正；
 * - 失败静默**保留上一份成功数据**（不闪断、不报错——「静默不渲染」指无错误
 *   UI；首拉失败 → null 不渲染）；成功载荷（含空）即覆盖；
 * - 会话 finished 不拉取（挂载门在 SessionDetail；idle 不拦——claude 后台
 *   agent 可在主会话 idle 时仍在跑）。 */
export default function SubagentChips({
  session,
  refreshTick,
}: {
  session: Session;
  refreshTick: number;
}) {
  const [list, setList] = useState<SubagentView[] | null>(null);
  // 时长自走 tick（仅本组件重渲染）。now 收在 state 里由 effect/interval 更新——
  // render 期不得调 Date.now()（仓内 eslint react-hooks/purity 红线）；挂载即对齐，
  // 首帧前 now=null 不显时长（一帧，无感）
  const [now, setNow] = useState<number | null>(null);

  useEffect(() => {
    if (session.status === "finished") return;
    let alive = true;
    fetchSessionSubagents(session.agentType, session.id)
      .then((v) => {
        if (alive) setList(v);
      })
      .catch(() => {
        /* 静默：保留上一份（不闪断）；首拉失败维持 null 不渲染 */
      });
    return () => {
      alive = false;
    };
  }, [session.agentType, session.id, session.status, refreshTick]);

  useEffect(() => {
    const update = () => setNow(Date.now());
    update();
    const t = setInterval(update, 1000);
    return () => clearInterval(t);
  }, []);

  if (list === null || list.length === 0) return null;
  return (
    <div data-testid="subagent-chips" className="flex flex-wrap items-center gap-x-2 gap-y-1">
      {list.map((s) => {
        const elapsed =
          now !== null && s.spawnTs !== null && !Number.isNaN(Date.parse(s.spawnTs))
            ? formatElapsed(now - Date.parse(s.spawnTs))
            : null;
        return (
          <span
            key={s.id}
            data-testid={`subagent-chip-${s.id}`}
            title={s.description ?? undefined}
            className="rounded-full bg-[var(--cb)]/60 px-2 py-0.5 text-[11px] text-[var(--tx)]"
          >
            ◉ {s.name}
            {elapsed !== null ? ` ${elapsed}` : ""} · {chipTokenText(s.tokens)}
          </span>
        );
      })}
    </div>
  );
}

/** 运行时长格式（纯函数）：M:SS；≥1h → H:MM:SS */
export function formatElapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
}

/** chip 的 token 文案：四桶求和走既有 fmtTokens 口径（reasoning 类桶已在后端排除） */
export function chipTokenText(t: SubagentView["tokens"]): string {
  return fmtTokens(t.input + t.cacheRead + t.cacheCreation + t.output);
}
