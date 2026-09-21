// 移动端审批卡（M8 Task 12，红卡选项卡 UI）：挂在 SessionDetail 正文视图
// messageArea 上方（waiting 态；预览/分屏分支不挂——MessageComposer 同一挂载惯例）。
// - 选项可用性：挂载拉取一次 /session-approve-options；拉取失败 / 网络异常 →
//   静默自隐；available=false 且无 reason（非 Waiting / 无映射 / 未命中）→ 自隐
//   （SessionDetail 无需感知选项可用性，详情页正文照常——fetchSessionFiles 静默
//   降级同一惯例）；available=false 且带 reason（严格档）→ 渲染提示条见下；
// - 红卡视觉：红色边框卡 + 标题「等待批准」（红点呼吸对齐看板 waiting 状态点）+
//   选项按钮横排（label 渲染；响应载荷只含 id/label，键位是投递层机密不外泄 UI）；
// - drift=true → 提示条「映射待实测确认，若提示不符请用普通发送」；
// - 严格档（M9R）：available=false 且后端下发 reason（Task 10：键位未实测确认时
//   下发「键位待实测确认，请用普通发送」）→ 卡片只渲染提示条（reason 原文内联，
//   琥珀色弱化视觉、不带红卡脉冲）不渲染按键组——键位未实测防误发；
//   available=false 且无 reason（非 Waiting / 无映射 / 未命中）→ 卡自隐（原惯例）；
// - 应答：POST /session-approve——key_sent → 「已发送按键」终态（按钮禁用）；
//   failed{error} → 错误文案可重试（按钮保持可点，重按即重试）；ApiError（409/404
//   带 data.error）→ 分診中文文案：not_waiting→「会话不在等待状态」、no_mapping→
//   「该工具暂不支持审批应答，请用普通发送」、no_session→「会话已结束，请返回看板刷新」、
//   其余显示 message。
// 局限（本任务范围裁决）：mount 只拉一次选项，卡内不做轮询——红卡的出现/消失依赖
// 页面数据刷新（SSE 快照 → 详情页重挂/卸载）自然带动；SSE 驱动卡内 re-fetch 属
// Task 12 后优化，不在本任务范围。
import { useCallback, useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ApiError, fetchApproveOptions, sessionApprove, type ApproveOptionsView } from "./api";

interface ApproveCardProps {
  /** 会话（本组件只消费 id；结构化类型，完整 Session 可直接传入） */
  session: { id: string };
}

export default function ApproveCard({ session }: ApproveCardProps) {
  // 选项可用性：ready=false（加载中 / 拉取失败）→ 不渲染（available/reason 分诊在渲染侧）
  const [options, setOptions] = useState<ApproveOptionsView | null>(null);
  const [ready, setReady] = useState(false);
  // 应答进行中（防连点）
  const [busy, setBusy] = useState(false);
  // 终态：按键已投递（key_sent）——按钮禁用 +「已发送按键」
  const [sent, setSent] = useState(false);
  // 失败文案（failed{error} 回执 / ApiError 分診）——非 null 展示，按钮保持可点（可重试）
  const [error, setError] = useState<string | null>(null);

  // 挂载拉取一次选项可用性；拉取失败 → 静默保持隐藏。
  // ready 只表示「载荷已落地」（available/reason 的分诊移到渲染侧——严格档
  // available=false + reason 也要渲染提示条，不能拿 available 当 ready）
  useEffect(() => {
    let alive = true;
    setReady(false);
    fetchApproveOptions(session.id)
      .then((v) => {
        if (!alive) return;
        setOptions(v);
        setReady(true);
      })
      .catch(() => {
        if (alive) setReady(false);
      });
    return () => {
      alive = false;
    };
  }, [session.id]);

  const handleAnswer = useCallback(
    async (optionId: string) => {
      if (busy || sent) return;
      setBusy(true);
      setError(null);
      try {
        const res = await sessionApprove(session.id, optionId);
        if (res.status === "key_sent") {
          setSent(true);
        } else {
          setError(res.error);
        }
      } catch (e) {
        if (e instanceof ApiError) {
          const code = typeof e.data?.error === "string" ? e.data.error : null;
          if (code === "not_waiting") {
            setError("会话不在等待状态");
          } else if (code === "no_mapping") {
            setError("该工具暂不支持审批应答，请用普通发送");
          } else if (code === "no_session") {
            setError("会话已结束，请返回看板刷新");
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
    [busy, sent, session.id]
  );

  // 加载中 / 拉取失败 / options 未落地：不渲染
  if (!ready || options === null) return null;
  // 严格档（M9R）：available=false 且后端给出 reason → 只渲染提示条不渲染按键；
  // available=false 且无 reason（非 Waiting / 无映射 / 未命中）：卡自身自隐（原惯例）
  const hintOnly =
    !options.available && typeof options.reason === "string" && options.reason.trim() !== "";
  if (!options.available && !hintOnly) return null;

  // 严格档提示条模式：reason 原文内联（后端中文），琥珀色弱化（非红卡脉冲——
  // 无可操作按键，避免误导），不渲染按键组
  if (hintOnly) {
    return (
      <div
        data-testid="approve-card"
        data-mode="hint"
        className="shrink-0 rounded-xl border border-amber-500/50 bg-amber-500/5 px-3 py-2 dark:border-amber-400/50 dark:bg-amber-400/5"
      >
        <p data-testid="approve-hint" className="text-xs text-amber-700 dark:text-amber-400">
          {options.reason}
        </p>
      </div>
    );
  }

  return (
    <div
      data-testid="approve-card"
      data-mode={options.dialog ? "dialog" : "binary"}
      className="shrink-0 rounded-xl border-2 border-rose-500/60 bg-rose-500/5 px-3 py-2 dark:border-rose-400/60 dark:bg-rose-400/5"
    >
      <div className="flex items-center gap-1.5">
        <span className="inline-block h-2 w-2 animate-pulse rounded-full bg-rose-500" />
        <span className="text-sm font-semibold text-rose-700 dark:text-rose-400">
          {options.dialog ? "等待批准（终端对话框）" : "等待批准"}
        </span>
      </div>
      {options.dialog && (
        <p
          data-testid="approve-dialog-label"
          className="mt-1 text-xs text-rose-700/80 dark:text-rose-400/80"
        >
          以下选项读自终端对话框，点按即代你按对应数字键
        </p>
      )}
      {/* T8：审批点 plan 聚合——计划确认类审批卡主体即见计划全文（不再要用户去
          消息流翻）。markdown 直出（claude/codex 的 kind="plan"）；kimi 的
          kind="plan-file" 是**文件路径**，此处以路径提示呈现（全文走文件预览，
          与消息流口径一致，不重复读文件正文）。无计划（plan null）→ 不渲染 */}
      {options.plan != null && options.plan.content.trim() !== "" && (
        <div
          data-testid="approve-plan"
          data-plan-file={options.plan.isFile ? "true" : "false"}
          className="mt-2 max-h-64 overflow-y-auto rounded-lg border border-rose-500/30 bg-white/60 p-2 text-xs text-slate-800 dark:border-rose-400/30 dark:bg-slate-900/60 dark:text-slate-200"
        >
          <p className="mb-1 text-[11px] font-medium tracking-wide text-rose-700/80 uppercase dark:text-rose-400/80">
            {options.plan.isFile ? "计划文件" : "计划内容"}
          </p>
          {options.plan.isFile ? (
            <p data-testid="approve-plan-file" className="font-mono break-all">
              {options.plan.content}
            </p>
          ) : (
            <div className="prose-sm max-w-none">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>{options.plan.content}</ReactMarkdown>
            </div>
          )}
        </div>
      )}
      {options.drift && (
        <p data-testid="approve-drift" className="mt-1 text-xs text-amber-700 dark:text-amber-400">
          映射待实测确认，若提示不符请用普通发送
        </p>
      )}
      {error !== null && (
        <p data-testid="approve-error" className="mt-1 text-xs text-rose-600 dark:text-rose-400">
          {error}
        </p>
      )}
      {sent && (
        <p
          data-testid="approve-sent"
          className="mt-1.5 text-xs font-medium text-emerald-600 dark:text-emerald-400"
        >
          已发送按键
        </p>
      )}
      <div className={options.dialog ? "mt-2 space-y-1" : "mt-2 flex gap-2"}>
        {/* T5：对话框选项 → 纵向编号列表（真实选项文本较长，纵向排布可读；
            编号徽标 = 将注入的数字键，用户所见即所按）；二元项维持既有横排 */}
        {options.dialog
          ? options.options.map((o, i) => (
              <button
                key={o.id}
                type="button"
                data-testid={`approve-option-${o.id}`}
                disabled={busy || sent}
                onClick={() => handleAnswer(o.id)}
                className="flex w-full items-start gap-2 rounded-lg bg-rose-500/10 px-2 py-1.5 text-left text-xs text-rose-700 disabled:opacity-40 dark:bg-rose-400/10 dark:text-rose-300"
              >
                <span className="shrink-0 rounded bg-rose-500/20 px-1.5 py-0.5 font-mono text-[10px] font-semibold dark:bg-rose-400/20">
                  {i + 1}
                </span>
                <span className="min-w-0 flex-1 break-words">{o.label}</span>
              </button>
            ))
          : options.options.map((o) => (
              <button
                key={o.id}
                type="button"
                data-testid={`approve-option-${o.id}`}
                disabled={busy || sent}
                onClick={() => handleAnswer(o.id)}
                className="flex-1 rounded-full bg-rose-500/10 px-3 py-1.5 text-sm text-rose-700 disabled:opacity-40 dark:bg-rose-400/10 dark:text-rose-300"
              >
                {o.label}
              </button>
            ))}
      </div>
    </div>
  );
}
