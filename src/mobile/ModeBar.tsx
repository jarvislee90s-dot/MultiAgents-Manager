import { useCallback, useEffect, useState } from "react";
import { toneTokens } from "./InteractiveCard";
import {
  ApiError,
  fetchSessionMode,
  MAM_MODE_LABELS,
  sessionModeSwitch,
  type MamMode,
  type SessionModeView,
} from "./api";

/** 模式栏（批次丙 T6）：会话头部显示当前模式 + 一键切档。
 *
 * 数据源 GET /session-mode（尽力而为：档位靠屏读回显，只有 opencode 有实测支撑）。
 *
 * **降级语义（T6 红线 4：不假装成功）**：
 * - `switchKind === "unsupported"` → 不渲染（该工具无实测切换机制）；
 * - `current === null` → 显示「模式未知」+「请人工核对终端」（屏读失败/不支持回显）；
 * - 切档回执 `verified === false` → 显示「已发送切换，请人工核对」（不声称已切到目标档）。
 *
 * 切档按钮：
 * - `shiftTab`（claude/opencode/kimi）→ 「切换模式」单钮（shift+tab 循环切一档；
 *   各家档位环序未实测，不提供「直达某档」的按钮——未验不出手）；
 * - `slashCommand`（codex）→ 逐档按钮（/plan、/permissions 有实测命令证据的档）；
 *   无命令证据的档位不渲染（target 传入也会被后端 409 拒绝）。
 */
export default function ModeBar({ session }: { session: { id: string } }) {
  const [view, setView] = useState<SessionModeView | null>(null);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 切档回执提示（成功后展示；verified=false 时是人工核对提示） */
  const [receipt, setReceipt] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    fetchSessionMode(session.id)
      .then((v) => {
        if (alive) setView(v);
      })
      .catch(() => {
        /* 拉取失败静默自隐（approve/question 同惯例） */
      })
      .finally(() => {
        if (alive) setReady(true);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  const handleSwitch = useCallback(
    async (target: MamMode) => {
      setBusy(true);
      setError(null);
      setReceipt(null);
      try {
        const res = await sessionModeSwitch(session.id, target);
        if (res.status === "key_sent") {
          // verified=false → 人工核对提示（红线 4）；true → 回读一次确认新档
          if (res.verified) {
            setReceipt("已切换");
            try {
              setView(await fetchSessionMode(session.id));
            } catch {
              /* 回读失败不清回执（切换本身已投递） */
            }
          } else {
            setReceipt(res.hint ?? "已发送切换，请人工核对终端模式");
          }
        } else {
          setError(res.error);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          if (e.status === 404 && e.data?.error === "no_session") {
            setError("会话已结束，请返回看板刷新");
          } else if (e.status === 409 && e.data?.error === "no_mechanism") {
            setError("该工具的模式切换未实测");
          } else if (e.status === 409 && e.data?.error === "blocked_by_dialog") {
            // 丁T3 §2.7 对话框在场红线（裁8/9，问题 5）：控制类注入被拒——后端
            // 屏读见编号选项对话框，切换键/斜杠命令会落进对话框（实机连点 17 次
            // 全变「选第一项」）。文案用后端下发的 reason（单一来源），缺省给同义兜底
            setError(
              typeof e.data?.reason === "string" ? e.data.reason : "终端有待决对话框，请先处理"
            );
          } else {
            setError(e.message);
          }
        } else {
          setError(String(e));
        }
      } finally {
        setBusy(false);
      }
    },
    [session.id]
  );

  // 未就绪 / 拉取失败 / 无实测机制：不渲染
  if (!ready || view === null || view.switchKind === "unsupported") return null;

  const currentText = view.currentLabel ?? "模式未知";

  // T10：本组件是**状态条**（非「等待用户输入」交互卡，任务书 §2.2 的容器契约
  // 针对 ApproveCard/QuestionCard 两类交互卡），故保留自身的横向布局；但**取色
  // 走统一 token**（InteractiveCard 的 mode 档）——四套界面同一套设计语言
  const t = toneTokens("mode");
  return (
    <div
      data-testid="mode-bar"
      data-mode={view.current ?? "unknown"}
      data-tone="mode"
      className={`flex flex-wrap items-center gap-2 px-2 py-1 ${t.box}`}
    >
      <span className="text-xs text-slate-500 dark:text-slate-400">模式</span>
      <span
        data-testid="mode-current"
        className={`text-xs font-semibold ${
          view.current === null
            ? "text-amber-700 dark:text-amber-400"
            : "text-slate-800 dark:text-slate-200"
        }`}
      >
        {currentText}
      </span>
      {view.current === null && (
        <span
          data-testid="mode-unknown-hint"
          className="text-[11px] text-amber-700 dark:text-amber-400"
        >
          请人工核对终端当前模式
        </span>
      )}
      {view.switchKind === "shiftTab" ? (
        // 循环切换：单钮（shift+tab 切一档；目标档由循环决定，未实测不出手直达）
        <button
          type="button"
          data-testid="mode-switch-next"
          disabled={busy}
          onClick={() => handleSwitch("default")}
          className="rounded-full bg-slate-500/15 px-2 py-0.5 text-[11px] text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
        >
          切换模式
        </button>
      ) : (
        // codex：逐档按钮（只渲染有实测命令证据的档：/plan 与 /permissions）
        <span className="flex gap-1">
          {(["plan", "bypass"] as MamMode[]).map((m) => (
            <button
              key={m}
              type="button"
              data-testid={`mode-switch-${m}`}
              disabled={busy}
              onClick={() => handleSwitch(m)}
              className="rounded-full bg-slate-500/15 px-2 py-0.5 text-[11px] text-slate-700 disabled:opacity-40 dark:bg-slate-400/15 dark:text-slate-300"
            >
              {MAM_MODE_LABELS[m]}
            </button>
          ))}
        </span>
      )}
      {error !== null && (
        <span data-testid="mode-error" className="text-[11px] text-rose-600 dark:text-rose-400">
          {error}
        </span>
      )}
      {receipt !== null && (
        <span
          data-testid="mode-receipt"
          className="text-[11px] text-emerald-600 dark:text-emerald-400"
        >
          {receipt}
        </span>
      )}
    </div>
  );
}
